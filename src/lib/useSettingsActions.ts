import { useCallback, useState } from "react";
import { relaunch } from "@tauri-apps/plugin-process";
import {
  clearAccount,
  deviceName as fetchDeviceName,
  getAccount,
  oauthBeginLogin,
  oauthListOrgs,
  oauthSignOut,
  oauthStatus,
  proxyDisable,
  proxyStatus,
  saveAccount,
  setDeviceName,
  setLaunchAtLogin,
  setOrg,
  switchGateway,
} from "./api";
import type { Account, OAuthStatus, Org, ProxyState } from "./api";
import { launchAtLoginStatus } from "./api";
import { track, trackError } from "./analytics";

/**
 * The Settings pane's side effects, for the new window UI.
 *
 * Split out of `NewUiApp` for the same reason as `useRouting`: these actions
 * hold multi-step state (a dialog, a draft value, the order two backend calls
 * run in) and none of it can be exercised through a component without a Tauri
 * runtime. A hook can be driven against a mocked `lib/api`.
 *
 * Not extracted from `screens/Settings.tsx`. That one keeps every flow inline
 * as a confirm *panel*, which is the popover's shape - 380px, one thing at a
 * time. The design uses centred dialogs, so the state a container has to hold
 * is different, and the popover's copy dies with the popover.
 */

/**
 * The dialog the pane is currently asking for. One value rather than a boolean
 * per dialog: the window shows at most one, and a single field cannot get into
 * the state where two are open at once.
 */
export type SettingsPrompt =
  | { kind: "replace-key" }
  /** Renaming this device. Carries the name being replaced so the dialog can
   * show it without the shell threading it back in. */
  | { kind: "rename-device"; currentName: string }
  | { kind: "switch-org"; orgs: Org[]; selectedId: string }
  | { kind: "org-switched"; name: string }
  | { kind: "disconnect" }
  /** The environment picker, for people working on Gate itself. Carries the
   * selection because switching is confirmed, not applied on the click. */
  | { kind: "switch-gateway"; selectedUrl: string }
  | { kind: "reset"; acknowledged: boolean };

export interface SettingsActions {
  prompt: SettingsPrompt | null;
  /** An action is in flight. Dialog primaries read this to avoid a double submit. */
  busy: boolean;
  /** The OAuth upgrade specifically, which is the only action that opens a
   *  browser. `busy` is shared by every action in this hook, so driving the
   *  "finish signing in in your browser" note from it told a user mid-rename,
   *  mid-reset or mid-disconnect to go and finish signing in. Worst on a
   *  single-org account, where `openSwitchOrg` sets `busy` with no dialog
   *  open at all and the sentence just flashes onto the pane. */
  oauthBusy: boolean;
  /** Draft value for the replace-key field, owned here so the dialog stays presentational. */
  newKey: string;
  setNewKey: (next: string) => void;
  /** Draft value for the rename-device field. Separate from `newKey`: one dialog
   * is open at a time, but a shared draft would carry a typed key into the next
   * dialog's field. */
  newDeviceName: string;
  setNewDeviceName: (next: string) => void;
  /** Whether the install ID was just copied, for the row's confirmation. */
  copied: boolean;

  dismissPrompt: () => void;
  toggleLaunchAtLogin: () => Promise<void>;
  copyText: (text: string) => Promise<void>;
  openReplaceKey: () => void;
  replaceKey: () => Promise<void>;
  openRenameDevice: (currentName: string) => void;
  renameDevice: () => Promise<void>;
  openSwitchOrg: () => Promise<void>;
  selectOrg: (id: string) => void;
  confirmSwitchOrg: () => Promise<void>;
  openDisconnect: () => void;
  confirmDisconnect: () => Promise<void>;
  /** Move a key-based account onto Constellation sign-in, from the one-time
   * offer. Resolves once the browser flow is over, so the offer can close. */
  upgradeToOAuth: () => Promise<void>;
  openSwitchGateway: () => void;
  selectGateway: (url: string) => void;
  confirmSwitchGateway: () => Promise<void>;
  openReset: () => void;
  acknowledgeReset: (next: boolean) => void;
  confirmReset: () => Promise<void>;
}

export function useSettingsActions({
  account,
  proxyRunning,
  launchAtLogin,
  onLaunchAtLogin,
  onAccount,
  onDeviceName,
  onSession,
  onProxy,
  onTeardown,
  onError,
}: {
  account: Account | null;
  /** Whether the engine is running, so reset knows to stop it first. */
  proxyRunning: boolean;
  launchAtLogin: boolean;
  /** Both the enabled flag and whether an opt-out is still pending. */
  onLaunchAtLogin: (state: { enabled: boolean; pendingDisable: boolean }) => void;
  onAccount: (account: Account | null) => void;
  /** The resolved device name after a rename - the stored one, or the hostname
   * again when the name was cleared. Re-read rather than echoed, since the
   * backend decides what an empty name means. */
  onDeviceName: (name: string) => void;
  /** Both credentials at once, for the two actions that end the session. */
  onSession: (next: { account: Account | null; oauth: OAuthStatus | null }) => void;
  onProxy: (next: ProxyState | null) => void;
  /**
   * A teardown just ran, so the caller can report where the tools stand.
   *
   * Fired by the two actions here that end a session - sign-out and reset - and
   * on reset's *failure* path as well as its success one: a reset that aborts
   * because a tool could not be disconnected is precisely the case AG-570 asks
   * to be listed, and reporting only on success would hide it.
   *
   * A callback rather than a value, because what to do about it is the shell's
   * decision: the report is read back from the configs, not returned by these
   * commands, so there is nothing here to hand over.
   */
  onTeardown?: () => void;
  onError: (err: unknown) => void;
}): SettingsActions {
  const [prompt, setPrompt] = useState<SettingsPrompt | null>(null);
  const [busy, setBusy] = useState(false);
  const [oauthBusy, setOauthBusy] = useState(false);
  const [newKey, setNewKey] = useState("");
  const [newDeviceName, setNewDeviceName] = useState("");
  const [copied, setCopied] = useState(false);

  const dismissPrompt = useCallback(() => {
    setPrompt(null);
    setNewKey("");
    setNewDeviceName("");
  }, []);

  /**
   * Optimistic, then reconciled. The switch has to move on click or it reads as
   * broken, but the backend decides whether an opt-out deregisters the login
   * item now or defers it until routing is off (see `set_launch_at_login` in
   * lib.rs), so the truth comes from a re-read.
   */
  const toggleLaunchAtLogin = useCallback(async () => {
    const next = !launchAtLogin;
    onLaunchAtLogin({ enabled: next, pendingDisable: false });
    try {
      await setLaunchAtLogin(next);
    } catch (err) {
      onLaunchAtLogin({ enabled: !next, pendingDisable: false });
      onError(err);
      trackError(err, "launch_at_login");
      return;
    }
    track("launch_at_login_toggled", { enabled: next });
    // Best-effort and deliberately separate from the write above: the toggle
    // already succeeded, so a failed re-read must not revert the switch to the
    // opposite of the actual state.
    try {
      const status = await launchAtLoginStatus();
      onLaunchAtLogin({ enabled: status.enabled, pendingDisable: status.pending_disable });
    } catch (err) {
      trackError(err, "launch_at_login");
    }
  }, [launchAtLogin, onLaunchAtLogin, onError]);

  /** Identifiers should never need retyping by hand. */
  const copyText = useCallback(
    async (text: string) => {
      try {
        await navigator.clipboard.writeText(text);
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      } catch (err) {
        onError(err);
        trackError(err, "generic");
      }
    },
    [onError],
  );

  const openReplaceKey = useCallback(() => {
    setNewKey("");
    setPrompt({ kind: "replace-key" });
  }, []);

  const replaceKey = useCallback(async () => {
    const base = account?.gateway_base_url;
    const key = newKey.trim();
    if (!base || !key || busy) return;
    setBusy(true);
    try {
      await saveAccount(base, key);
      // The account's `has_api_key` and auth mode both change, and the pane
      // renders the masked key from them.
      onAccount(await getAccount());
      track("key_replaced");
      setPrompt(null);
      setNewKey("");
    } catch (err) {
      // Saving a key is by definition the key path, so a rejection here means
      // this key is wrong, not that the session expired. The dialog stays open
      // so the user can correct it rather than reopening and retyping.
      onError(err);
      trackError(err, "save_api_key");
    } finally {
      setBusy(false);
    }
  }, [account, newKey, busy, onAccount, onError]);

  const openSwitchOrg = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    try {
      const orgs = await oauthListOrgs();
      // Nothing to choose between: opening a picker over a single row asks a
      // question with one answer.
      if (orgs.length < 2) return;
      setPrompt({
        kind: "switch-org",
        orgs,
        selectedId: account?.org_id ?? orgs[0].orgId,
      });
    } catch (err) {
      onError(err);
      trackError(err, "generic");
    } finally {
      setBusy(false);
    }
  }, [account, busy, onError]);

  const selectOrg = useCallback((id: string) => {
    setPrompt((p) => (p?.kind === "switch-org" ? { ...p, selectedId: id } : p));
  }, []);

  const confirmSwitchOrg = useCallback(async () => {
    if (prompt?.kind !== "switch-org" || busy) return;
    const org = prompt.orgs.find((o) => o.orgId === prompt.selectedId);
    if (!org) return;
    setBusy(true);
    try {
      await setOrg(org.orgId, org.name);
      onAccount(await getAccount());
      // Confirmation rather than a silent close: the org decides what gets
      // billed and what the gateway rejects requests without, so "did that
      // work?" is a question worth answering.
      setPrompt({ kind: "org-switched", name: org.name });
    } catch (err) {
      onError(err);
      trackError(err, "generic");
    } finally {
      setBusy(false);
    }
  }, [prompt, busy, onAccount, onError]);

  const openDisconnect = useCallback(() => setPrompt({ kind: "disconnect" }), []);

  /**
   * End the OAuth session, keeping the account and the tools' connections.
   *
   * Scoped to the session because that is the row it lives under, and because
   * removing the account is what Reset is for. The drawn dialog copy says the
   * API key is removed from the keychain, which describes Reset rather than
   * this; implemented as a sign-out and the copy corrected. Raised with the
   * designer.
   */
  const confirmDisconnect = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    try {
      await oauthSignOut();
      const [acct, oauth] = await Promise.all([
        getAccount().catch(() => null),
        oauthStatus().catch(() => null),
      ]);
      onSession({ account: acct, oauth });
      setPrompt(null);
      // The configs are kept on purpose here (this row ends the session, not the
      // account), so every connected tool is now pointing at Gate with no
      // session behind it. That is a state the user should be told about rather
      // than discover from a failing tool.
      onTeardown?.();
    } catch (err) {
      onError(err);
      trackError(err, "sign_out");
    } finally {
      setBusy(false);
    }
  }, [busy, onSession, onTeardown, onError]);

  /**
   * The OAuth offer's accept.
   *
   * Deliberately **not** `useSetup.signIn`: that one saves the account first,
   * with the *default* gateway and no key, which is right for a machine with no
   * account and wrong here - it would repoint a staging install at production
   * and drop the key the user still has. `oauth_begin_login` records OAuth
   * against the account that already exists, which is all this needs.
   */
  const upgradeToOAuth = useCallback(async () => {
    setBusy(true);
    setOauthBusy(true);
    try {
      await oauthBeginLogin();
      const [acct, oauth] = await Promise.all([
        getAccount().catch(() => null),
        oauthStatus().catch(() => null),
      ]);
      onSession({ account: acct, oauth });
    } finally {
      setBusy(false);
      setOauthBusy(false);
    }
  }, [onSession]);

  /** Prefilled with the current name: the field is an edit, not a blank form,
   *  and the commonest rename is a small change to what is already there. */
  const openRenameDevice = useCallback((currentName: string) => {
    setPrompt({ kind: "rename-device", currentName });
    setNewDeviceName(currentName);
  }, []);

  const renameDevice = useCallback(async () => {
    if (prompt?.kind !== "rename-device" || busy) return;
    const name = newDeviceName.trim();
    if (!name) return;
    setBusy(true);
    try {
      await setDeviceName(name);
      onDeviceName(await fetchDeviceName());
      setPrompt(null);
      setNewDeviceName("");
    } catch (err) {
      // The dialog stays open, like the key form: the name is still in the field
      // and retrying is one click, where a closed dialog loses what was typed.
      onError(err);
      trackError(err, "generic");
    } finally {
      setBusy(false);
    }
  }, [prompt, busy, newDeviceName, onDeviceName, onError]);

  const openSwitchGateway = useCallback(() => {
    setPrompt({ kind: "switch-gateway", selectedUrl: account?.gateway_base_url ?? "" });
  }, [account]);

  const selectGateway = useCallback((url: string) => {
    setPrompt((p) => (p?.kind === "switch-gateway" ? { ...p, selectedUrl: url } : p));
  }, []);

  /**
   * Repoint the account at another environment, then relaunch.
   *
   * Switching forgets the stored key, disconnects managed tools and stops the
   * engine, which pins the gateway URL when it starts. `App.tsx` relaunches for
   * that reason and this does too: patching the rest of the window live would
   * mean reconciling an account, a session and a routing table that all just
   * changed underneath it.
   */
  const confirmSwitchGateway = useCallback(async () => {
    if (prompt?.kind !== "switch-gateway" || busy) return;
    const url = prompt.selectedUrl;
    if (!url || url === account?.gateway_base_url) return;
    setBusy(true);
    try {
      await switchGateway(url);
      // Nothing below runs on success.
      await relaunch();
    } catch (err) {
      onError(err);
      trackError(err, "generic");
    } finally {
      setBusy(false);
    }
  }, [prompt, busy, account, onError]);

  const openReset = useCallback(() => setPrompt({ kind: "reset", acknowledged: false }), []);

  const acknowledgeReset = useCallback((next: boolean) => {
    setPrompt((p) => (p?.kind === "reset" ? { ...p, acknowledged: next } : p));
  }, []);

  /**
   * Turn routing off, then remove the account. The order is the point.
   *
   * A failed disable can leave system HTTPS pointed at an engine port that is
   * about to die, so this aborts rather than continuing to `clearAccount` and
   * stranding the machine's traffic on a dead proxy. `clearAccount` itself
   * disconnects managed tools before wiping anything, so a failure there leaves
   * the user still signed in - which is why the error surfaces instead of the
   * window dropping to sign-in over a half-reset state.
   */
  const confirmReset = useCallback(async () => {
    if (prompt?.kind !== "reset" || !prompt.acknowledged || busy) return;
    setBusy(true);
    try {
      if (proxyRunning) {
        try {
          onProxy(await proxyDisable());
        } catch (err) {
          // Re-read so the UI shows what the engine actually did, then abort.
          onProxy(await proxyStatus().catch(() => null));
          throw err;
        }
      }
      await clearAccount();
      track("workspace_forgotten");
      // Nothing to route and nobody to route for. The derived setup stage picks
      // this up and shows sign-in; there is no separate "go to first run" step
      // that could disagree with what is on disk.
      onSession({ account: null, oauth: null });
      setPrompt(null);
      onTeardown?.();
    } catch (err) {
      onError(err);
      trackError(err, "forget");
      // On the failure path too, and this is the important half: `clearAccount`
      // aborts when a tool cannot be disconnected, which leaves that tool
      // pointing at Gate with the account still in place. The error says the
      // reset stopped; the report says which tool stopped it.
      onTeardown?.();
    } finally {
      setBusy(false);
    }
  }, [prompt, busy, proxyRunning, onProxy, onSession, onTeardown, onError]);

  return {
    prompt,
    busy,
    oauthBusy,
    newKey,
    setNewKey,
    newDeviceName,
    setNewDeviceName,
    copied,
    dismissPrompt,
    openDisconnect,
    confirmDisconnect,
    upgradeToOAuth,
    openSwitchGateway,
    selectGateway,
    confirmSwitchGateway,
    openReset,
    acknowledgeReset,
    confirmReset,
    toggleLaunchAtLogin,
    copyText,
    openReplaceKey,
    replaceKey,
    openRenameDevice,
    renameDevice,
    openSwitchOrg,
    selectOrg,
    confirmSwitchOrg,
  };
}
