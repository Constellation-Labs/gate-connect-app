import { useEffect, useRef, useState } from "react";
import type { Account, OAuthStatus } from "../lib/api";
import { launchAtLoginStatus, setLaunchAtLogin, getAccountKeyPrefix, backfillAccountKeyPrefix } from "../lib/api";
import { track, trackError } from "../lib/analytics";
import { classifyError, type ClassifiedError } from "../lib/errors";
import { GATEWAY_SERVERS } from "../lib/config";
import { SubHeader, SectionLabel, ConnPill, Button, Input, Switch, ErrorNote, IconButton } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";
import { usePlatform } from "../lib/platform";

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/** Inline confirm step for the destructive actions (the popover never stacks
 * dialogs): names exactly what is about to be lost, then a confirm/cancel
 * pair. Same pattern as the key-reveal confirm below. */
function ConfirmPanel({
  message,
  confirmLabel,
  busy,
  onConfirm,
  onCancel,
}: {
  message: string;
  confirmLabel: string;
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="mx-3.5 mt-2 rounded bg-gc-subtle p-3 shadow-border">
      <div className="text-[11.5px] leading-snug text-gc-ink-2">{message}</div>
      <div className="mt-2.5 flex items-center gap-2">
        {/* Same destructive grammar as the routing takeover: the action that
            destroys something is never the encouraged indigo, and Cancel is
            its visual equal rather than an afterthought. */}
        <Button variant="danger" size="sm" disabled={busy} onClick={onConfirm}>
          {confirmLabel}
        </Button>
        <Button variant="secondary" size="sm" disabled={busy} onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

/** Settings - workspace + Gate API key management. The key itself is held in
 *  the OS keychain and never returned to the UI, so it shows masked; Replace
 *  key calls save_account, Forget calls clear_account. */
export function Settings({
  account,
  oauth,
  onBack,
  onReplaceKey,
  onUpgradeToOAuth,
  onForget,
  onSignOut,
  onSwitchOrg,
  onSwitchGateway,
  onReplayTour,
  version,
  routingOn,
  caTrusted,
  proxyBusy,
  onUntrustCa,
}: {
  account: Account;
  oauth: OAuthStatus | null;
  onBack: () => void;
  onReplaceKey: (key: string) => Promise<void>;
  onUpgradeToOAuth: () => Promise<void>;
  onForget: () => Promise<void>;
  onSignOut: () => Promise<void>;
  onSwitchOrg: () => void;
  onSwitchGateway: (url: string) => Promise<void>;
  onReplayTour: () => void;
  /** Shown under Help; the popover footer has no room for it. */
  version: string;
  routingOn: boolean;
  caTrusted: boolean;
  proxyBusy: boolean;
  onUntrustCa: () => void;
}) {
  const isOAuth = account.auth_mode === "oauth";
  const connected = isOAuth ? (oauth?.signed_in ?? false) : account.has_api_key;
  const [replacing, setReplacing] = useState(false);
  const [devMode, setDevMode] = useState(false);
  const platform = usePlatform();
  const trustStore = platform === "windows" ? "certificate store" : "keychain";
  const [newKey, setNewKey] = useState("");
  const [submitting, setSubmitting] = useState(false);
  // Where a failure belongs on screen. One shared slot rendered under
  // "Account" meant a launch-at-login failure printed its reason ~250px above
  // the switch that reverted, usually outside the 487px viewport.
  type ErrorSlot = "account" | "machine" | "server" | "reset";
  const [error, setError] = useState<{ slot: ErrorSlot; error: ClassifiedError } | null>(null);
  const errorFor = (slot: ErrorSlot) =>
    error?.slot === slot ? <ErrorNote error={error.error} className="mx-3.5 mt-2" /> : null;
  const [upgrading, setUpgrading] = useState(false);
  // Armed by the Reset buttons; the destructive clear only runs from the
  // inline confirm panel.
  const [confirmingReset, setConfirmingReset] = useState(false);
  // Armed by picking a different Dev-mode gateway server; holds the choice
  // until the confirm panel approves the forget-key-and-relaunch.
  const [confirmingServer, setConfirmingServer] = useState<{ url: string; label: string } | null>(
    null,
  );
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
  // Armed by the certificate Remove button; the untrust only runs from the
  // inline confirm.
  const [confirmingUntrust, setConfirmingUntrust] = useState(false);
  // Whether the certificate explanation is open. Collapsed on entry.
  const [certExplain, setCertExplain] = useState(false);
  // Copy-to-clipboard feedback on the gateway URL (flashes the icon to a
  // check). Identifiers should never need retyping by hand.
  const [copiedUrl, setCopiedUrl] = useState(false);

  async function copyGatewayUrl() {
    try {
      await navigator.clipboard.writeText(account.gateway_base_url);
      setCopiedUrl(true);
      setTimeout(() => setCopiedUrl(false), 1500);
    } catch (err) {
      trackError(err, "generic");
    }
  }

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
      setError({ slot: "machine", error: classifyError(err, "launch_at_login") });
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
      setError({ slot: "account", error: classifyError(err, "save_api_key") });
      trackError(err, "save_api_key");
    } finally {
      setSubmitting(false);
    }
  }

  async function forget() {
    if (submitting) return;
    setError(null);
    setSubmitting(true);
    try {
      await onForget();
    } catch (err) {
      setError({ slot: "reset", error: classifyError(err, "forget") });
      trackError(err, "forget");
    } finally {
      setSubmitting(false);
      setConfirmingReset(false);
    }
  }

  async function signOut() {
    if (submitting) return;
    setError(null);
    setSubmitting(true);
    try {
      await onSignOut();
    } catch (err) {
      setError({ slot: "account", error: classifyError(err, "sign_out") });
      trackError(err, "sign_out");
    } finally {
      setSubmitting(false);
    }
  }

  async function switchServer(url: string) {
    if (url === account.gateway_base_url || submitting) return;
    setError(null);
    setSubmitting(true);
    try {
      await onSwitchGateway(url); // relaunches the app on success; nothing below runs
    } catch (err) {
      setError({ slot: "server", error: classifyError(err, "generic") });
      trackError(err, "generic");
    } finally {
      setSubmitting(false);
      setConfirmingServer(null);
    }
  }

  // Same stale-attempt guard as FirstRun: Cancel releases the button; a
  // browser flow the user finishes anyway still lands via onUpgradeToOAuth.
  const upgradeAttempt = useRef(0);

  async function handleUpgrade() {
    setError(null);
    setUpgrading(true);
    const attempt = ++upgradeAttempt.current;
    try {
      await onUpgradeToOAuth();
    } catch (err) {
      trackError(err, "sign_in");
      if (attempt === upgradeAttempt.current) {
        setError({ slot: "account", error: classifyError(err, "sign_in") });
        setUpgrading(false);
      }
    }
  }

  function cancelUpgrade() {
    upgradeAttempt.current++;
    setUpgrading(false);
  }

  return (
    <div className="flex grow flex-col">
      <SubHeader title="Settings" onBack={onBack} />

      {/* Three sections, not six: Workspace / Signed in / Gate API Key were
          all "my account", and Startup / Certificate were both "this machine".
          Same controls, half the chunking. */}
      <SectionLabel>Account</SectionLabel>
      <div className="flex items-center gap-3 px-3.5 py-2.5">
        <div className="flex h-[30px] w-[30px] shrink-0 items-center justify-center rounded-[9px] bg-gc-accent-wash text-gc-accent">
          <Icon name="cube" size={16} />
        </div>
        <div className="min-w-0 flex-1">
          {/* Host only. The full URL sat right under it and both truncated
              ("gateway.constellationgat…" over "https://gateway.constell…"),
              so the second line spent a row saying less exactly the same
              thing. The copy button is what guarantees exactness. */}
          {/* 10.5px mono on its own line, matching the dev-mode server cards
              which fit the full host in the same 360px. At 13px sans beside a
              pill and a copy button this truncated to
              "gateway.constellationga…", and DESIGN.md's mono rule exists
              because identity and precision matter for exactly this string. */}
          <div className="text-[11px] font-medium text-gc-ink">Gateway</div>
          <div className="truncate font-mono text-[10.5px] text-gc-ink-3">
            {hostOf(account.gateway_base_url)}
          </div>
        </div>
        <ConnPill
          state={connected ? "connected" : "signedout"}
          // A stored key is not a session. Only OAuth is "Signed in".
          label={isOAuth ? (connected ? "Signed in" : "Signed out") : connected ? "Key stored" : "No key"}
        />
        <IconButton
          icon={copiedUrl ? "check" : "copy"}
          size={14}
          onClick={() => void copyGatewayUrl()}
          aria-label="Copy gateway URL"
        />
      </div>

      {isOAuth && (
        <>
          <div className="flex items-center gap-3 px-3.5 py-2.5">
            <div className="flex h-[30px] w-[30px] shrink-0 items-center justify-center rounded-[9px] bg-gc-accent-wash text-gc-accent">
              <Icon name="shieldCheck" size={16} />
            </div>
            <div className="min-w-0 flex-1">
              <div className="truncate text-[13px] font-medium text-gc-ink">
                {oauth?.email ?? (connected ? "Signed in" : "Session expired")}
              </div>
              <div className="truncate text-[11.5px] text-gc-ink-3">
                {account.org_name ?? "No organization selected"}
              </div>
            </div>
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-x-4 gap-y-2 px-3.5 pb-1">
            <button
              type="button"
              onClick={onSwitchOrg}
              disabled={submitting}
              className="inline-flex items-center gap-1.5 text-[12.5px] font-medium text-gc-accent"
            >
              <Icon name="refresh" size={14} />
              Switch organization
            </button>
            <button
              type="button"
              onClick={signOut}
              disabled={submitting}
              className="inline-flex items-center gap-1.5 text-[12.5px] font-medium text-gc-ink-3 transition hover:text-gc-ink"
            >
              <Icon name="logOut" size={14} />
              Sign out
            </button>
          </div>
          {errorFor("account")}
        </>
      )}

      {!isOAuth && (
        <>
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
        {errorFor("account")}
        {account.has_api_key && !replacing && (
          <p className="mt-1.5 text-[11px] text-gc-ink-3">Stored in your keychain.</p>
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
        </div>
      )}

      {/* A quiet row, not a full-width accent button. Indigo is affordance
          and live state; this is a conversion prompt, and it was the loudest
          thing on a screen an API-key user opens to check their key - sitting
          above the key itself. */}
      <div className="mb-3 px-3.5">
        <button
          type="button"
          disabled={upgrading}
          onClick={handleUpgrade}
          className="inline-flex items-center gap-1.5 text-[12.5px] font-medium text-gc-accent transition hover:text-gc-accent-ink disabled:opacity-50"
        >
          <Icon name="shieldCheck" size={14} />
          {upgrading ? "Waiting for browser…" : "Switch to Constellation sign-in"}
        </button>
        <p className="mt-1 text-[11px] leading-snug text-gc-ink-3">
          Nothing to paste or rotate; your session lives in the keychain and
          refreshes on its own. You can switch back anytime.
        </p>
        {upgrading && (
          <div className="mt-2 flex items-center justify-between gap-2">
            <p className="text-[11px] leading-snug text-gc-ink-3">
              Finish signing in on the page that opened in your browser.
            </p>
            <button
              type="button"
              onClick={cancelUpgrade}
              className="shrink-0 text-[12px] font-medium text-gc-ink-3 transition hover:text-gc-ink"
            >
              Cancel
            </button>
          </div>
        )}
      </div>
        </>
      )}

      <SectionLabel>This machine</SectionLabel>
      <div className="flex items-center gap-3 px-3.5 py-2.5">
        <div className="min-w-0 flex-1">
          <div className="text-[13px] font-medium text-gc-ink">Launch at login</div>
          <div className="mt-0.5 text-[11.5px] leading-snug text-gc-ink-3">
            Open Gate Connect automatically when you log in. Keeps routing on after a restart.
          </div>
        </div>
        <Switch
          on={launchAtLogin}
          label="Launch at login"
          disabled={!laLoaded}
          onClick={toggleLaunchAtLogin}
        />
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
      {/* Gated on trust alone, not on routing being off. Home promises "You
          can remove it anytime in Settings under Certificate" while routing is
          on, which is exactly the state the user reads it in; hiding the
          section behind !routingOn broke that promise at the moment they acted
          on it, for a root CA. The consequence goes in the copy instead. */}
      {errorFor("machine")}

      {caTrusted && (
        <>
          <div className="flex items-start gap-3 px-3.5 py-2.5">
            <div className="min-w-0 flex-1">
              <div className="text-[13px] font-medium text-gc-ink">Gate certificate</div>
              {/* Collapsed by default, same disclosure Home's certificate card
                  uses. The one line a user scanning Settings needs is whether
                  it is trusted and whether they can remove it; the consequence
                  of removing it is four lines they only need once, and it was
                  taking that room on every visit. */}
              <div className="mt-0.5 text-[11.5px] leading-snug text-gc-ink-3">
                Still trusted in your {trustStore}.
                {routingOn && " Turn routing off to remove it."}{" "}
                <button
                  type="button"
                  onClick={() => setCertExplain((v) => !v)}
                  aria-expanded={certExplain}
                  className="font-medium text-gc-ink-3 underline decoration-gc-line-strong underline-offset-2 transition hover:text-gc-ink"
                >
                  What&rsquo;s this?
                </button>
              </div>
            </div>
            {!routingOn && (
            <button
              type="button"
              onClick={() => setConfirmingUntrust(true)}
              disabled={proxyBusy}
              // error-deep like every other destructive entry point. It was
              // ink-3 with no underline or border - a label, not an action -
              // on the one control that deletes a private key.
              // Neutral: removing trust is reversible maintenance, not an
              // encouraged action. It still gets a confirm, because by its own
              // copy it deletes a private key and can stop apps routing - it
              // was the only state-destroying action in the app without one.
              className="shrink-0 text-[12px] font-medium text-gc-error-deep transition hover:brightness-90 disabled:opacity-40"
            >
              Remove
            </button>
            )}
          </div>
          {/* Below the row, not inside it: in the flex row the paragraph was
              squeezed into the column Remove left over, and Remove floated
              vertically centred against five lines of text, detached from the
              heading it belongs to. */}
          {certExplain && (
            <p className="px-3.5 pb-1 text-[11.5px] leading-snug text-gc-ink-2">
              {routingOn
                ? `Gate created this certificate on this machine so apps with no gateway setting of their own can route through the local proxy. Pulling it while routing is on stops every one of them, so removal waits until routing is off. The private key never leaves this machine.`
                : `Gate created this certificate on this machine so apps with no gateway setting of their own can route through the local proxy. Removing it deletes the certificate and its private key from this machine; you can trust a new one anytime.`}
            </p>
          )}
          {confirmingUntrust && (
            <ConfirmPanel
              message="Remove the Gate certificate? This deletes it and its private key from this machine. You can trust a new one anytime."
              confirmLabel="Remove certificate"
              busy={proxyBusy}
              onConfirm={() => {
                setConfirmingUntrust(false);
                onUntrustCa();
              }}
              onCancel={() => setConfirmingUntrust(false)}
            />
          )}
        </>
      )}

      <div className="mt-auto">
        <SectionLabel>Help</SectionLabel>
        {/* Dev mode lives here now. It used to float between the certificate
            and Help with no label, the only unlabelled control on a screen
            built entirely of labelled sections. */}
        <div className="flex flex-wrap items-center gap-x-4 gap-y-2 px-3.5 pb-1">
          <button
            type="button"
            onClick={onReplayTour}
            className="inline-flex items-center gap-1.5 text-[12.5px] font-medium text-gc-ink-3 transition hover:text-gc-ink"
          >
            <Icon name="info" size={14} />
            Replay tour
          </button>
          <button
            type="button"
            onClick={() => setDevMode((v) => !v)}
            aria-expanded={devMode}
            className="inline-flex items-center gap-1.5 text-[12.5px] font-medium text-gc-ink-3 transition hover:text-gc-ink"
          >
            <Icon name="settings" size={14} />
            Dev mode
          </button>
        </div>
        {version && (
          <p className="px-3.5 pt-1.5 font-mono text-[10.5px] text-gc-ink-3">v{version}</p>
        )}
        {devMode && !replacing && (
            <>
              {/* Not a SectionLabel: this is a sub-panel of Help, and an h2
                  here made the document outline gain and lose a top-level
                  section every time Dev mode toggled. */}
              <div className="px-3.5 pb-1.5 pt-1 font-mono text-[10.5px] font-medium uppercase tracking-[0.08em] text-gc-ink-3">
                Gateway server
              </div>
              <div className="flex flex-col gap-2 px-3.5 pb-1">
                {GATEWAY_SERVERS.map((server) => {
                  const active = server.url === account.gateway_base_url;
                  return (
                      <button
                          key={server.url}
                          type="button"
                          onClick={() => setConfirmingServer({ url: server.url, label: server.label })}
                          disabled={active || submitting}
                          className="flex items-center gap-3 rounded bg-gc-surface px-3 py-2 text-left shadow-border transition hover:shadow-border-hover disabled:cursor-default disabled:hover:shadow-border"
                      >
                        <div className="min-w-0 flex-1">
                          <div className="text-[13px] font-medium text-gc-ink">{server.label}</div>
                          <div className="truncate font-mono text-[10.5px] text-gc-ink-3">
                            {hostOf(server.url)}
                          </div>
                        </div>
                        {active && <Icon name="check" size={15} className="shrink-0 text-gc-accent" />}
                      </button>
                  );
                })}
              </div>
              {errorFor("server")}
              {confirmingServer && (
                <ConfirmPanel
                  message={`Switch to ${confirmingServer.label}? This forgets your stored key, disconnects your tools, and relaunches Gate Connect against the new server.`}
                  confirmLabel={submitting ? "Switching…" : "Switch and relaunch"}
                  busy={submitting}
                  onConfirm={() => void switchServer(confirmingServer.url)}
                  onCancel={() => setConfirmingServer(null)}
                />
              )}
            </>
        )}
        {/* Last, and under its own heading. Reset used to appear twice - once
            in each auth branch - sitting beside routine actions like Replace
            key, separated from them only by colour. */}
        <SectionLabel>Reset</SectionLabel>
        <div className="px-3.5 pb-1">
          <button
            type="button"
            onClick={() => setConfirmingReset(true)}
            disabled={submitting}
            className="inline-flex items-center gap-1.5 text-[12.5px] font-medium text-gc-error-deep disabled:opacity-50"
          >
            <Icon name="trash" size={14} />
            Reset Gate Connect
          </button>
        </div>
        {errorFor("reset")}
        {confirmingReset && (
          <ConfirmPanel
            message={`Reset Gate Connect? This turns routing off, disconnects your tools, and ${isOAuth ? "forgets this account" : "removes your key from the keychain"}. You'll start over from sign-in.`}
            confirmLabel={submitting ? "Resetting…" : "Reset everything"}
            busy={submitting}
            onConfirm={() => void forget()}
            onCancel={() => setConfirmingReset(false)}
          />
        )}
      </div>
    </div>
  );
}
