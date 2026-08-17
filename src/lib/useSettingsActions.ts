import { useCallback, useState } from "react";
import {
  clearAccount,
  getAccount,
  oauthListOrgs,
  oauthSignOut,
  oauthStatus,
  proxyDisable,
  proxyStatus,
  saveAccount,
  setLaunchAtLogin,
  setOrg,
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
  | { kind: "switch-org"; orgs: Org[]; selectedId: string }
  | { kind: "org-switched"; name: string }
  | { kind: "disconnect" }
  | { kind: "reset"; acknowledged: boolean };

export interface SettingsActions {
  prompt: SettingsPrompt | null;
  /** An action is in flight. Dialog primaries read this to avoid a double submit. */
  busy: boolean;
  /** Draft value for the replace-key field, owned here so the dialog stays presentational. */
  newKey: string;
  setNewKey: (next: string) => void;
  /** Whether the install ID was just copied, for the row's confirmation. */
  copied: boolean;

  dismissPrompt: () => void;
  toggleLaunchAtLogin: () => Promise<void>;
  copyText: (text: string) => Promise<void>;
  openReplaceKey: () => void;
  replaceKey: () => Promise<void>;
  openSwitchOrg: () => Promise<void>;
  selectOrg: (id: string) => void;
  confirmSwitchOrg: () => Promise<void>;
  openDisconnect: () => void;
  confirmDisconnect: () => Promise<void>;
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
  onSession,
  onProxy,
  onError,
}: {
  account: Account | null;
  /** Whether the engine is running, so reset knows to stop it first. */
  proxyRunning: boolean;
  launchAtLogin: boolean;
  /** Both the enabled flag and whether an opt-out is still pending. */
  onLaunchAtLogin: (state: { enabled: boolean; pendingDisable: boolean }) => void;
  onAccount: (account: Account | null) => void;
  /** Both credentials at once, for the two actions that end the session. */
  onSession: (next: { account: Account | null; oauth: OAuthStatus | null }) => void;
  onProxy: (next: ProxyState | null) => void;
  onError: (err: unknown) => void;
}): SettingsActions {
  const [prompt, setPrompt] = useState<SettingsPrompt | null>(null);
  const [busy, setBusy] = useState(false);
  const [newKey, setNewKey] = useState("");
  const [copied, setCopied] = useState(false);

  const dismissPrompt = useCallback(() => {
    setPrompt(null);
    setNewKey("");
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
    } catch (err) {
      onError(err);
      trackError(err, "sign_out");
    } finally {
      setBusy(false);
    }
  }, [busy, onSession, onError]);

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
    } catch (err) {
      onError(err);
      trackError(err, "forget");
    } finally {
      setBusy(false);
    }
  }, [prompt, busy, proxyRunning, onProxy, onSession, onError]);

  return {
    prompt,
    busy,
    newKey,
    setNewKey,
    copied,
    dismissPrompt,
    openDisconnect,
    confirmDisconnect,
    openReset,
    acknowledgeReset,
    confirmReset,
    toggleLaunchAtLogin,
    copyText,
    openReplaceKey,
    replaceKey,
    openSwitchOrg,
    selectOrg,
    confirmSwitchOrg,
  };
}
