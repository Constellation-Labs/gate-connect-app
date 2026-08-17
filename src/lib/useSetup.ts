import { useCallback, useRef, useState } from "react";
import {
  getAccount,
  oauthBeginLogin,
  oauthListOrgs,
  oauthStatus,
  proxyEnable,
  saveAccount,
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
  | { kind: "org-picker" }
  /** Signed in, and the confirmation has not been dismissed yet. */
  | { kind: "connected" }
  /** Signed in and done: the shell shows the app. */
  | { kind: "ready" };

export interface Setup {
  stage: SetupStage;
  busy: boolean;
  error: unknown;
  /** Organizations for the picker; null until they have been read. */
  orgs: Org[] | null;
  selectedOrgId: string | undefined;
  apiKeyOpen: boolean;
  apiKey: string;
  gateway: string;

  signIn: () => Promise<void>;
  toggleApiKey: () => void;
  setApiKey: (next: string) => void;
  setGateway: (next: string) => void;
  connectWithApiKey: () => Promise<void>;
  loadOrgs: () => Promise<void>;
  selectOrg: (id: string) => void;
  confirmOrg: () => Promise<void>;
  useApiKeyInstead: () => void;
  turnOnRouting: () => Promise<void>;
  finish: () => void;
}

export function useSetup({
  loaded,
  account,
  oauth,
  onSession,
  onProxy,
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
}): Setup {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [orgs, setOrgs] = useState<Org[] | null>(null);
  const [selectedOrgId, setSelectedOrgId] = useState<string | undefined>(undefined);
  const [apiKeyOpen, setApiKeyOpen] = useState(false);
  const [apiKey, setApiKeyValue] = useState("");
  const [gateway, setGatewayValue] = useState(DEFAULT_GATEWAY_BASE_URL);
  // The two pieces of stage that cannot be derived.
  //
  // `confirmationSeen`: "signed in" is true the moment the org lands, so without
  // it the confirmation pane would never show.
  //
  // `keyFormForced`: the org-picker dead end (signed in, no organization) hands
  // the user to the key form, and derivation alone would keep returning them to
  // the picker they were trying to leave.
  const [confirmationSeen, setConfirmationSeen] = useState(false);
  const [keyFormForced, setKeyFormForced] = useState(false);

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
    // Asked for the key form on purpose: they are signed in, so nothing has
    // expired and the reauth copy would be a lie.
    if (keyFormForced) return { kind: "welcome", reauth: false };
    if (needsOrg(account, oauth)) return { kind: "org-picker" };
    if (!signedIn) {
      // Reauth only for an OAuth account, matching the popover: an API-key
      // account that lost its key never had a session to expire.
      return { kind: "welcome", reauth: account?.auth_mode === "oauth" };
    }
    // Confirm a sign-in that happened here; never greet a returning user.
    return sawSignedOut.current && !confirmationSeen
      ? { kind: "connected" }
      : { kind: "ready" };
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
      setKeyFormForced(false);
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

  /** The org picker's dead end: no organization and no admin to ask. Hands the
   *  user straight to the key form rather than making them find it again. */
  const useApiKeyInstead = useCallback(() => {
    setApiKeyOpen(true);
    setKeyFormForced(true);
  }, []);

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
    apiKeyOpen,
    apiKey,
    gateway,
    signIn,
    toggleApiKey: () => setApiKeyOpen((v) => !v),
    setApiKey: setApiKeyValue,
    setGateway: setGatewayValue,
    connectWithApiKey,
    loadOrgs,
    selectOrg: setSelectedOrgId,
    confirmOrg,
    useApiKeyInstead,
    turnOnRouting,
    finish,
  };
}
