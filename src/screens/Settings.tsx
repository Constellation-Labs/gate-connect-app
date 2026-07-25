import { useEffect, useState } from "react";
import type { Account } from "../lib/api";
import { launchAtLoginStatus, setLaunchAtLogin, getAccountKeyPrefix, backfillAccountKeyPrefix } from "../lib/api";
import { track, trackError } from "../lib/analytics";
import { GATEWAY_SERVERS } from "../lib/config";
import { SubHeader, SectionLabel, ConnPill, Button, Input, Switch } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";
import { usePlatform } from "../lib/platform";

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/** Settings - workspace + Gate API key management. The key itself is held in
 *  the OS keychain and never returned to the UI, so it shows masked; Replace
 *  key calls save_account, Disconnect calls clear_account. */
export function Settings({
  account,
  onBack,
  onReplaceKey,
  onDisconnect,
  onSwitchGateway,
  onReplayTour,
  routingOn,
  caTrusted,
  proxyBusy,
  onUntrustCa,
}: {
  account: Account;
  onBack: () => void;
  onReplaceKey: (key: string) => Promise<void>;
  onDisconnect: () => Promise<void>;
  onSwitchGateway: (url: string) => Promise<void>;
  onReplayTour: () => void;
  routingOn: boolean;
  caTrusted: boolean;
  proxyBusy: boolean;
  onUntrustCa: () => void;
}) {
  const [replacing, setReplacing] = useState(false);
  const [devMode, setDevMode] = useState(false);
  const platform = usePlatform();
  const trustStore = platform === "windows" ? "certificate store" : "keychain";
  const [newKey, setNewKey] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [launchAtLogin, setLaunchAtLoginState] = useState(false);
  const [laLoaded, setLaLoaded] = useState(false);
  // The launch-at-login toggle is off but the OS login item is still
  // registered as a crash safety net: either the backend deferred an opt-out
  // (toggled off while routing was on) or it registered the item itself when
  // routing turned on. Surface a note explaining why the login-items list
  // still shows the app.
  const [laPendingDisable, setLaPendingDisable] = useState(false);
  // Auto-loaded from account.json on mount once a prefix has been recorded
  // there, so it stays visible without re-revealing each visit. null = not
  // yet loaded/stored (pre-prefix accounts fall back to a keychain reveal).
  const [revealedPrefix, setRevealedPrefix] = useState<string | null>(null);
  // Set when a pre-prefix account has no stored prefix, so revealing must fall
  // back to a keychain read. We ask first, since that read can prompt.
  const [confirmReveal, setConfirmReveal] = useState(false);

  async function revealKeyPrefix() {
    try {
      const prefix = await getAccountKeyPrefix();
      if (prefix) {
        setRevealedPrefix(prefix);
      } else {
        // No prefix on disk: an account saved before we recorded one. Offer the
        // keychain fallback rather than silently doing nothing.
        setConfirmReveal(true);
      }
    } catch (err) {
      trackError(err, "generic");
    }
  }

  async function revealFromKeychain() {
    setConfirmReveal(false);
    try {
      const prefix = await backfillAccountKeyPrefix();
      if (prefix) setRevealedPrefix(prefix);
    } catch (err) {
      trackError(err, "generic");
    }
  }

  useEffect(() => {
    let active = true;
    launchAtLoginStatus()
      .then((status) => {
        if (!active) return;
        setLaunchAtLoginState(status.enabled);
        setLaPendingDisable(status.pending_disable);
        setLaLoaded(true);
      })
      .catch((err) => trackError(err, "launch_at_login"));
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    if (!account.has_api_key) return;
    let active = true;
    getAccountKeyPrefix()
      .then((prefix) => {
        if (active && prefix) setRevealedPrefix(prefix);
      })
      .catch((err) => trackError(err, "generic"));
    return () => {
      active = false;
    };
  }, [account.has_api_key]);

  async function toggleLaunchAtLogin() {
    if (!laLoaded) return;
    // Turning this off while routing is on needs no warning: the backend
    // defers the actual deregistration until the system proxy is safe
    // (routing off, clean quit, or the next login launch), so a crash can
    // never strand traffic (see set_launch_at_login in lib.rs).
    await applyLaunchAtLogin(!launchAtLogin);
  }

  async function applyLaunchAtLogin(next: boolean) {
    setLaunchAtLoginState(next); // optimistic
    try {
      await setLaunchAtLogin(next);
    } catch (err) {
      setLaunchAtLoginState(!next); // revert on failure
      setError(err instanceof Error ? err.message : String(err));
      trackError(err, "launch_at_login");
      return;
    }
    track("launch_at_login_toggled", { enabled: next });
    // The backend decides whether an opt-out deregisters now or is deferred
    // (routing on), so re-read the status for the pending note. Best-effort
    // and separate from the toggle above: the toggle already succeeded, so a
    // failed re-read must not revert the switch to the opposite of the
    // actual state.
    try {
      const status = await launchAtLoginStatus();
      setLaunchAtLoginState(status.enabled);
      setLaPendingDisable(status.pending_disable);
    } catch (err) {
      trackError(err, "launch_at_login");
    }
  }

  async function saveKey() {
    if (newKey.trim().length === 0 || submitting) return;
    setError(null);
    setSubmitting(true);
    try {
      await onReplaceKey(newKey.trim());
      setReplacing(false);
      setNewKey("");
      setRevealedPrefix(null);
      setConfirmReveal(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      trackError(err, "save_api_key");
    } finally {
      setSubmitting(false);
    }
  }

  async function disconnect() {
    if (submitting) return;
    setError(null);
    setSubmitting(true);
    try {
      await onDisconnect();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      trackError(err, "disconnect");
    } finally {
      setSubmitting(false);
    }
  }

  async function selectServer(url: string) {
    if (url === account.gateway_base_url || submitting) return;
    setError(null);
    setSubmitting(true);
    try {
      await onSwitchGateway(url); // relaunches the app on success; nothing below runs
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      trackError(err, "generic");
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="flex grow flex-col">
      <SubHeader title="Settings" onBack={onBack} />

      <SectionLabel>Workspace</SectionLabel>
      <div className="flex items-center gap-3 px-3.5 py-2.5">
        <div className="flex h-[30px] w-[30px] shrink-0 items-center justify-center rounded-lg bg-gc-accent-wash text-gc-accent">
          <Icon name="cube" size={16} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[13px] font-medium text-gc-ink">
            {hostOf(account.gateway_base_url)}
          </div>
          <div className="truncate font-mono text-[10.5px] text-gc-ink-4">
            {account.gateway_base_url}
          </div>
        </div>
        <ConnPill state={account.has_api_key ? "connected" : "signedout"} />
      </div>

      <SectionLabel>Gate API Key</SectionLabel>
      <div className="px-3.5">
        {!replacing ? (
          <div className="flex h-9 items-center gap-2 rounded bg-gc-subtle px-3 text-gc-ink-3 shadow-border">
            <Icon name="key" size={14} className="text-gc-ink-4" />
            <span className="flex-1 font-mono text-[12px] tracking-wide">
              {account.has_api_key
                ? revealedPrefix
                  ? `${revealedPrefix}••••••••••`
                  : "sk-gw-••••••••••••••••"
                : "No key stored"}
            </span>
            {account.has_api_key && revealedPrefix === null && (
              <button
                type="button"
                onClick={revealKeyPrefix}
                aria-label="Show start of key"
                className="text-gc-ink-4 hover:text-gc-ink-2"
              >
                <Icon name="eye" size={14} />
              </button>
            )}
          </div>
        ) : (
          <div className="flex flex-col gap-2">
            <Input
              leadingIcon={<Icon name="key" size={14} />}
              placeholder="Enter new sk-gw-… key"
              value={newKey}
              autoFocus
              spellCheck={false}
              onChange={(e) => setNewKey(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") saveKey();
              }}
            />
            <div className="flex gap-2">
              <Button
                variant="accent"
                className="flex-1"
                disabled={newKey.trim().length === 0 || submitting}
                onClick={saveKey}
              >
                {submitting ? "Saving…" : "Save key"}
              </Button>
              <Button
                className="flex-1"
                onClick={() => {
                  setReplacing(false);
                  setNewKey("");
                  setError(null);
                }}
              >
                Cancel
              </Button>
            </div>
          </div>
        )}
        {error && <p className="mt-2 text-[11.5px] text-gc-error">{error}</p>}
        {account.has_api_key && !replacing && (
          <p className="mt-1.5 text-[11px] text-gc-ink-4">Stored in your keychain.</p>
        )}
        {confirmReveal && (
          <div className="mt-2 rounded bg-gc-subtle p-3 shadow-border">
            <div className="text-[11.5px] leading-snug text-gc-ink-2">
              Showing the start of your key reads it from your keychain, which
              may ask for permission.
            </div>
            <div className="mt-2.5 flex items-center gap-2">
              <button
                type="button"
                onClick={revealFromKeychain}
                className="text-[12.5px] font-medium text-gc-accent"
              >
                Show start of key
              </button>
              <button
                type="button"
                onClick={() => setConfirmReveal(false)}
                className="ml-auto text-[12.5px] font-medium text-gc-ink-3"
              >
                Cancel
              </button>
            </div>
          </div>
        )}
      </div>

      {!replacing && (
        <div className="mt-3 flex items-center gap-4 px-3.5 pb-1">
          <button
            type="button"
            onClick={() => setReplacing(true)}
            className="inline-flex items-center gap-1.5 text-[12.5px] font-medium text-gc-accent"
          >
            <Icon name="refresh" size={14} />
            Replace key
          </button>
          <button
            type="button"
            onClick={disconnect}
            disabled={submitting}
            className="inline-flex items-center gap-1.5 text-[12.5px] font-medium text-gc-error"
          >
            <Icon name="trash" size={14} />
            Disconnect
          </button>
        </div>
      )}

      <SectionLabel>Startup</SectionLabel>
      <div className="flex items-center gap-3 px-3.5 py-2.5">
        <div className="min-w-0 flex-1">
          <div className="text-[13px] font-medium text-gc-ink">Launch at login</div>
          <div className="mt-0.5 text-[11.5px] leading-snug text-gc-ink-3">
            Open Gate Connect automatically when you log in. Keeps routing on after a restart.
          </div>
        </div>
        <Switch on={launchAtLogin} disabled={!laLoaded} onClick={toggleLaunchAtLogin} />
      </div>
      {laPendingDisable && (
        <div className="mx-3.5 mb-1 flex items-start gap-2.5 rounded bg-gc-sunken px-3 py-2.5">
          <Icon name="info" size={15} className="mt-px shrink-0 text-gc-ink-3" />
          <div className="min-w-0 flex-1 text-[11.5px] leading-snug text-gc-ink-2">
            Gate Connect is still listed in your login items as a safety net,
            so an unexpected restart can't leave routing broken. It removes
            itself automatically once that's safe.
          </div>
        </div>
      )}
      {caTrusted && !routingOn && (
        <>
          <SectionLabel>Certificate</SectionLabel>
          <div className="flex items-center gap-3 px-3.5 py-2.5">
            <div className="min-w-0 flex-1">
              <div className="text-[13px] font-medium text-gc-ink">Gate certificate</div>
              <div className="mt-0.5 text-[11.5px] leading-snug text-gc-ink-3">
                Still trusted in your {trustStore}. Removing it clears the
                certificate and private key from this machine.
              </div>
            </div>
            <button
              type="button"
              onClick={onUntrustCa}
              disabled={proxyBusy}
              className="shrink-0 text-[12px] font-medium text-gc-accent disabled:opacity-40"
            >
              Remove
            </button>
          </div>
        </>
      )}

      <div className="px-3.5 pb-1 pt-1">
        <button
            type="button"
            onClick={() => setDevMode((v) => !v)}
            className="inline-flex items-center gap-1.5 text-[12.5px] font-medium text-gc-ink-3"
        >
          <Icon name="settings" size={14} />
          Dev mode
        </button>
      </div>

      {devMode && !replacing && (
          <>
            <SectionLabel>Gateway server</SectionLabel>
            <div className="flex flex-col gap-2 px-3.5 pb-1">
              {GATEWAY_SERVERS.map((server) => {
                const active = server.url === account.gateway_base_url;
                return (
                    <button
                        key={server.url}
                        type="button"
                        onClick={() => selectServer(server.url)}
                        disabled={active || submitting}
                        className="flex items-center gap-3 rounded bg-gc-surface px-3 py-2 text-left shadow-border transition hover:shadow-border-hover disabled:cursor-default disabled:hover:shadow-border"
                    >
                      <div className="min-w-0 flex-1">
                        <div className="text-[13px] font-medium text-gc-ink">{server.label}</div>
                        <div className="truncate font-mono text-[10.5px] text-gc-ink-4">
                          {hostOf(server.url)}
                        </div>
                      </div>
                      {active && <Icon name="check" size={15} className="shrink-0 text-gc-accent" />}
                    </button>
                );
              })}
            </div>
          </>
      )}

      <div className="mt-auto">
        <SectionLabel>Help</SectionLabel>
        <div className="px-3.5 pb-1">
          <button
            type="button"
            onClick={onReplayTour}
            className="inline-flex items-center gap-1.5 text-[12.5px] font-medium text-gc-ink-3 transition hover:text-gc-ink"
          >
            <Icon name="info" size={14} />
            Replay tour
          </button>
        </div>
      </div>
    </div>
  );
}
