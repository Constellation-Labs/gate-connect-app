import { useEffect, useState } from "react";
import type { Account } from "../lib/api";
import { launchAtLoginStatus, setLaunchAtLogin, getAccountKeyPrefix } from "../lib/api";
import { trackError } from "../lib/analytics";
import { GATEWAY_SERVERS } from "../lib/config";
import { SubHeader, SectionLabel, ConnPill, Button, Input, Switch } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";

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
}: {
  account: Account;
  onBack: () => void;
  onReplaceKey: (key: string) => Promise<void>;
  onDisconnect: () => Promise<void>;
  onSwitchGateway: (url: string) => Promise<void>;
  onReplayTour: () => void;
  routingOn: boolean;
}) {
  const [replacing, setReplacing] = useState(false);
  const [devMode, setDevMode] = useState(false);
  const [newKey, setNewKey] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [launchAtLogin, setLaunchAtLoginState] = useState(false);
  const [laLoaded, setLaLoaded] = useState(false);
  const [confirmDisableLaunch, setConfirmDisableLaunch] = useState(false);
  // Masked until the user taps to reveal - the fetch reads the keychain, so it
  // must stay opt-in rather than run on mount. null = masked, string = shown.
  const [revealedPrefix, setRevealedPrefix] = useState<string | null>(null);

  async function revealKeyPrefix() {
    if (revealedPrefix !== null) {
      setRevealedPrefix(null);
      return;
    }
    try {
      const prefix = await getAccountKeyPrefix();
      if (prefix) setRevealedPrefix(prefix);
    } catch (err) {
      trackError(err, "generic");
    }
  }

  useEffect(() => {
    let active = true;
    launchAtLoginStatus()
      .then((enabled) => {
        if (!active) return;
        setLaunchAtLoginState(enabled);
        setLaLoaded(true);
      })
      .catch((err) => trackError(err, "generic"));
    return () => {
      active = false;
    };
  }, []);

  async function toggleLaunchAtLogin() {
    if (!laLoaded) return;
    const next = !launchAtLogin;
    // Turning launch-at-login off while routing is on can strand the system
    // proxy after an unexpected restart: nothing relaunches to run the
    // startup self-heal, so other apps' traffic stays broken until the user
    // reopens Gate Connect. Confirm before disabling in that case.
    if (!next && routingOn) {
      setConfirmDisableLaunch(true);
      return;
    }
    await applyLaunchAtLogin(next);
  }

  async function applyLaunchAtLogin(next: boolean) {
    setConfirmDisableLaunch(false);
    setLaunchAtLoginState(next); // optimistic
    try {
      await setLaunchAtLogin(next);
    } catch (err) {
      setLaunchAtLoginState(!next); // revert on failure
      setError(err instanceof Error ? err.message : String(err));
      trackError(err, "generic");
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
            {account.has_api_key && (
              <button
                type="button"
                onClick={revealKeyPrefix}
                aria-label={revealedPrefix ? "Hide key" : "Show start of key"}
                className="text-gc-ink-4 hover:text-gc-ink-2"
              >
                <Icon name={revealedPrefix ? "eyeOff" : "eye"} size={14} />
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
      {confirmDisableLaunch && (
        <div className="mx-3.5 mb-1 rounded bg-gc-subtle p-3 shadow-border">
          <div className="text-[11.5px] leading-snug text-gc-ink-2">
            With this off, an unexpected restart can leave other apps unable to
            reach the internet until you reopen Gate Connect. Turn off routing
            first to avoid this.
          </div>
          <div className="mt-2.5 flex items-center gap-2">
            <button
              type="button"
              onClick={() => applyLaunchAtLogin(false)}
              className="text-[12.5px] font-medium text-gc-error"
            >
              Turn off anyway
            </button>
            <button
              type="button"
              onClick={() => setConfirmDisableLaunch(false)}
              className="ml-auto text-[12.5px] font-medium text-gc-ink-3"
            >
              Keep on
            </button>
          </div>
        </div>
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
