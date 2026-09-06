import { useCallback, useRef, useState } from "react";
import {
  getAccount,
  oauthBeginLogin,
  oauthListOrgs,
  oauthSignOut,
  oauthStatus,
  proxyEnable,
  saveAccount,
  setDeviceName,
  setOrg,
} from "./api";
import type { Account, OAuthStatus, Org, ProxyState } from "./api";
import { DEFAULT_GATEWAY_BASE_URL } from "./config";
import { isSignedIn, needsOrg } from "./session";
import { markOAuthOfferSeen } from "./oauthOffer";
import { track, trackError } from "./analytics";

/**
 * First run for the new window UI: sign in, pick an organization, confirm.
 *
 * The stage is **derived from account and OAuth state**, not stored, with one
 * exception (`acknowledged` below). That is what makes reset work: clearing the
 * account is enough to send the window back to sign-in, with no separate
 * "now show first run" flag that could disagree with what is on disk.
 *
 * `screens/FirstRun.tsx` and `screens/OrgPicker.tsx` are the popover's versions.
 * The call sequences here match theirs deliberately - saving the gateway before
 * `oauthBeginLogin` so the account exists on disk, marking the OAuth offer seen
 * when the user picks the key path - because those orderings encode decisions
 * that are not obvious from the API surface.
 */

export type SetupStage =
  | { kind: "loading" }
  /** No usable credential. `reauth` when an account exists but its session died. */
  | { kind: "welcome"; reauth: boolean }
  /** The key route, entered from the welcome pane on purpose. */
  | { kind: "api-key" }
  | { kind: "org-picker" }
  /** Signed in with an organization, and this machine has never been named.
   * Drawn between connecting and choosing apps, and both routes pass through it. */
  | { kind: "name-device" }
  /** Signed in, and the confirmation has not been dismissed yet. */
  | { kind: "connected" }
  /** Signed in and confirmed, but the diagnostic-data question has never been
   * answered. Derived from the stored preference, so it survives a restart
   * mid-setup and cannot be skipped by reloading. */
  | { kind: "diagnostics" }
  /** Signed in and done: the shell shows the app. */
  | { kind: "ready" };

export interface Setup {
  stage: SetupStage;
  busy: boolean;
  error: unknown;
  /** Organizations for the picker; null until they have been read. */
  orgs: Org[] | null;
  selectedOrgId: string | undefined;
  apiKey: string;
  gateway: string;
  /** The draft in the naming pane's field. */
  deviceNameDraft: string;

  signIn: () => Promise<void>;
  openApiKey: () => void;
  closeApiKey: () => void;
  setApiKey: (next: string) => void;
  setGateway: (next: string) => void;
  connectWithApiKey: () => Promise<void>;
  loadOrgs: () => Promise<void>;
  selectOrg: (id: string) => void;
  confirmOrg: () => Promise<void>;
  setDeviceNameDraft: (next: string) => void;
  nameDevice: () => Promise<void>;
  skipNaming: () => void;
  /** Drops the session and returns to sign-in. The Auth flow's "Use a different
   *  account", and the only way out of an account with no organization. */
  signOut: () => Promise<void>;
  turnOnRouting: () => Promise<void>;
  finish: () => void;
}

export function useSetup({
  loaded,
  account,
  oauth,
  onSession,
  onProxy,
  diagnosticsAnswered,
  deviceNamed,
}: {
  /** Whether the first read of account and OAuth state has come back. Without
   *  it, a null account on launch is indistinguishable from no account, and the
   *  window flashes the sign-in pane at every signed-in user. */
  loaded: boolean;
  account: Account | null;
  oauth: OAuthStatus | null;
  /** A fresh read of both, after anything that could change either. */
  onSession: (next: { account: Account | null; oauth: OAuthStatus | null }) => void;
  onProxy: (next: ProxyState) => void;
  /** Whether the diagnostic-data question has been answered. `undefined` while the
   * preference read is in flight - not "unanswered", which would flash the step at
   * someone who answered it months ago. */
  diagnosticsAnswered?: boolean;
  /** Whether this machine carries a name of its own. `undefined` while the
   * preference read is in flight - the same distinction `diagnosticsAnswered`
   * makes, and for the same reason: "not yet known" must not render as "never
   * named" and flash the pane at someone who named it months ago. */
  deviceNamed?: boolean;
}): Setup {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [orgs, setOrgs] = useState<Org[] | null>(null);
  const [selectedOrgId, setSelectedOrgId] = useState<string | undefined>(undefined);
  const [keyPaneOpen, setKeyPaneOpen] = useState(false);
  const [apiKey, setApiKeyValue] = useState("");
  const [deviceNameDraft, setDeviceNameDraft] = useState("");
  // The picker's explicit choice, null until the user makes one.
  //
  // The effective gateway is *derived* rather than seeded into state, because the
  // account lands a beat after mount and a `useState` initialiser would freeze
  // the build default before it arrives. That is how re-signing in came to
  // rewrite a staging or local install's account with the production URL: the
  // picker showed production, `signIn` persisted it, and `OAuthConfig::from_build_env`
  // then resolved its pool from the *new* host - so a build carrying only one
  // pool's credentials failed with "OAuth is not configured in this build"
  // before the browser ever opened. `useSettingsActions.upgradeToOAuth` avoids
  // `signIn` for this exact reason; the hazard belongs fixed here instead.
  const [gatewayChoice, setGatewayChoice] = useState<string | null>(null);
  const gateway = gatewayChoice ?? account?.gateway_base_url ?? DEFAULT_GATEWAY_BASE_URL;
  // The two pieces of stage that cannot be derived.
  //
  // `confirmationSeen`: "signed in" is true the moment the org lands, so without
  // it the confirmation pane would never show.
  //
  // `namingDone`: skipping leaves `device_name` null, which is exactly the state
  // that puts the pane on screen, so declining has to be remembered or the step
  // cannot be declined at all. A successful save sets it too, since the
  // preference re-read that flips `deviceNamed` lands a beat later and the pane
  // must not linger in between.
  const [confirmationSeen, setConfirmationSeen] = useState(false);
  const [namingDone, setNamingDone] = useState(false);

  // Whether a signed-out state has been observed in this session. Without it the
  // confirmation pane greets everyone who was *already* signed in at launch,
  // since "signed in" alone cannot tell a returning user from one who just
  // finished signing in. Also survives reset: the wipe puts the window back in a
  // signed-out state, so the next sign-in is confirmed again.
  const sawSignedOut = useRef(false);

  // Bumped by a new attempt so a stale sign-in cannot re-lock the screen or
  // surface its error after the user has moved on. There is no backend abort for
  // a pending browser flow; if the user finishes it anyway the account lands and
  // the derived stage picks it up, which is never wrong.
  const attempt = useRef(0);

  const signedIn = isSignedIn(account, oauth);
  // Derived from props during render rather than in an effect, so the first paint
  // is already right; idempotent, so a re-render cannot change the answer.
  if (loaded && !signedIn) sawSignedOut.current = true;

  const stage: SetupStage = ((): SetupStage => {
    if (!loaded) return { kind: "loading" };
    if (needsOrg(account, oauth)) return { kind: "org-picker" };
    if (!signedIn) {
      // Asked for the key route on purpose, so it outranks the sign-in choice.
      if (keyPaneOpen) return { kind: "api-key" };
      // Reauth only for an OAuth account, matching the popover: an API-key
      // account that lost its key never had a session to expire.
      return { kind: "welcome", reauth: account?.auth_mode === "oauth" };
    }
    // Name the machine before the apps are chosen, the order the key pane's own
    // copy promises. Guarded on `sawSignedOut` for the reason the confirmation
    // is: a returning user whose machine follows the hostname settled that.
    if (sawSignedOut.current && deviceNamed === false && !namingDone) {
      return { kind: "name-device" };
    }
    // Confirm a sign-in that happened here; never greet a returning user.
    if (sawSignedOut.current && !confirmationSeen) return { kind: "connected" };
    // Consent before Overview, and before collection: `lib/analytics.ts` starts at
    // launch, so the first thing this buys is a person who has been asked. Only
    // once the answer is known to be missing - `undefined` is the read still being
    // in flight, and treating that as unanswered would flash the step at someone
    // who answered months ago.
    if (diagnosticsAnswered === false) return { kind: "diagnostics" };
    return { kind: "ready" };
  })();

  const reread = useCallback(async () => {
    const [acct, oauthState] = await Promise.all([
      getAccount().catch(() => null),
      oauthStatus().catch(() => null),
    ]);
    onSession({ account: acct, oauth: oauthState });
    return { account: acct, oauth: oauthState };
  }, [onSession]);

  const signIn = useCallback(async () => {
    if (busy) return;
    setError(null);
    setBusy(true);
    const mine = ++attempt.current;
    try {
      // The gateway is persisted first, with no key, so the account exists on
      // disk before the browser flow records OAuth against it.
      await saveAccount(gateway, null);
      await oauthBeginLogin();
      const next = await reread();
      track("signed_in");
      // Fresh sign-in with no org yet: the picker is the next stage, and it
      // needs its list. Derived stage already moved; this fills it.
      if (needsOrg(next.account, next.oauth)) setOrgs(null);
    } catch (err) {
      trackError(err, "sign_in");
      if (mine === attempt.current) setError(err);
    } finally {
      if (mine === attempt.current) setBusy(false);
    }
  }, [busy, gateway, reread]);

  const connectWithApiKey = useCallback(async () => {
    const key = apiKey.trim();
    if (!key || busy) return;
    setError(null);
    setBusy(true);
    try {
      await saveAccount(gateway, key);
      // Choosing the key here answers the "would you rather sign in?" question.
      // Without this the one-time offer arrives on the next launch and reverses
      // a decision the user just made on purpose.
      markOAuthOfferSeen();
      await reread();
      track("signed_in");
      setApiKeyValue("");
      setKeyPaneOpen(false);
    } catch (err) {
      setError(err);
      trackError(err, "sign_in");
    } finally {
      setBusy(false);
    }
  }, [apiKey, gateway, busy, reread]);

  /**
   * Read the org list, and pick for the user when there is only one - the common
   * case, and a question with one answer.
   */
  const loadOrgs = useCallback(async () => {
    setError(null);
    try {
      const list = await oauthListOrgs();
      setOrgs(list);
      setSelectedOrgId(list[0]?.orgId);
      if (list.length === 1) {
        setBusy(true);
        try {
          await setOrg(list[0].orgId, list[0].name);
          await reread();
        } finally {
          setBusy(false);
        }
      }
    } catch (err) {
      setError(err);
      trackError(err, "generic");
    }
  }, [reread]);

  const confirmOrg = useCallback(async () => {
    const org = orgs?.find((o) => o.orgId === selectedOrgId);
    if (!org || busy) return;
    setError(null);
    setBusy(true);
    try {
      await setOrg(org.orgId, org.name);
      await reread();
    } catch (err) {
      setError(err);
      trackError(err, "generic");
    } finally {
      setBusy(false);
    }
  }, [orgs, selectedOrgId, busy, reread]);

  /** Save the name and leave the step. */
  const nameDevice = useCallback(async () => {
    const name = deviceNameDraft.trim();
    if (!name || busy) return;
    setError(null);
    setBusy(true);
    try {
      await setDeviceName(name);
      setDeviceNameDraft("");
      setNamingDone(true);
    } catch (err) {
      setError(err);
      trackError(err, "generic");
    } finally {
      setBusy(false);
    }
  }, [deviceNameDraft, busy]);

  const skipNaming = useCallback(() => setNamingDone(true), []);

  /**
   * "Use a different account", and the org dead end's only way out.
   *
   * `oauthSignOut` alone would leave an API-key account signed in, so the key is
   * dropped as well - by rewriting the account with the gateway and no key,
   * which is the call `signIn` already makes before the browser flow. Clearing
   * the account outright would take the chosen gateway with it and silently
   * repoint a staging install at production.
   */
  const signOut = useCallback(async () => {
    if (busy) return;
    setError(null);
    setBusy(true);
    try {
      await oauthSignOut().catch(() => {});
      await saveAccount(gateway, null);
      setOrgs(null);
      setSelectedOrgId(undefined);
      setApiKeyValue("");
      setDeviceNameDraft("");
      setKeyPaneOpen(false);
      setNamingDone(false);
      setConfirmationSeen(false);
      await reread();
    } catch (err) {
      setError(err);
      trackError(err, "sign_out");
    } finally {
      setBusy(false);
    }
  }, [busy, gateway, reread]);

  /**
   * Finish the job the confirmation copy promises. Routing off at this point
   * would mean a window that says "connected" over apps carrying no traffic.
   */
  const turnOnRouting = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    try {
      onProxy(await proxyEnable());
      setConfirmationSeen(true);
    } catch (err) {
      setError(err);
      trackError(err, "proxy_toggle");
    } finally {
      setBusy(false);
    }
  }, [busy, onProxy]);

  const finish = useCallback(() => setConfirmationSeen(true), []);

  return {
    stage,
    busy,
    error,
    orgs,
    selectedOrgId,
    apiKey,
    gateway,
    deviceNameDraft,
    signIn,
    openApiKey: () => setKeyPaneOpen(true),
    closeApiKey: () => setKeyPaneOpen(false),
    setApiKey: setApiKeyValue,
    setGateway: setGatewayChoice,
    connectWithApiKey,
    loadOrgs,
    selectOrg: setSelectedOrgId,
    confirmOrg,
    setDeviceNameDraft,
    nameDevice,
    skipNaming,
    signOut,
    turnOnRouting,
    finish,
  };
}
