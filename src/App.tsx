import { useCallback, useEffect, useState } from "react";
import type { ReactNode } from "react";
import type { Account, OAuthStatus, ProxyState, ProviderState, Tool } from "./lib/api";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  getAccount,
  saveAccount,
  clearAccount,
  switchGateway,
  oauthStatus,
  oauthSignOut,
  oauthBeginLogin,
  proxyStatus,
  proxyEnable,
  proxyDisable,
  proxyTrustCa,
  proxyUntrustCa,
  listProviders,
  providerEnable,
  providerDisable,
  listTools,
  unpinPopover,
  openOnboardingWindow,
  routedClientsStale,
} from "./lib/api";
import { FirstRun } from "./screens/FirstRun";
import { OrgPicker } from "./screens/OrgPicker";
import { Home } from "./screens/Home";
import { ProxyScreen } from "./screens/ProxyScreen";
import { Settings } from "./screens/Settings";
import { Success } from "./screens/Success";
import { ComingSoon } from "./screens/ComingSoon";
import { UpdatePanel } from "./components/UpdatePanel";
import { StartupRoutingNotice } from "./components/StartupRoutingNotice";
import { LinuxTitleBar } from "./components/LinuxTitleBar";
import { ConstellationHexMark } from "./components/gc/ConstellationHexMark";
import { track, trackError } from "./lib/analytics";
import { hasSeenTour, markTourSeen } from "./lib/tour";
import { TOUR_SEEN_EVENT } from "./screens/Onboarding";
import { usePlatform } from "./lib/platform";

type Screen =
  | "loading"
  | "firstrun"
  | "orgpicker"
  | "home"
  | "proxy"
  | "settings"
  | "success"
  | "coming-soon";

// Providers hidden from the UI for now. Slugs match the backend provider list.
const HIDDEN_PROVIDER_SLUGS = new Set<string>([]);

function hostOf(url: string | undefined): string {
  if (!url) return "";
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/** Whether the account is fully usable right now: a stored key in legacy mode,
 *  or a live OAuth session *with an org selected* in OAuth mode (the gateway
 *  rejects OAuth requests that carry no org). Drives home-vs-sign-in/picker. */
function isSignedIn(account: Account | null, oauth: OAuthStatus | null): boolean {
  if (!account) return false;
  if (account.auth_mode === "oauth") return (oauth?.signed_in ?? false) && !!account.org_id;
  return account.has_api_key;
}

/** An OAuth session that's authenticated but hasn't picked an org yet - the
 *  one state that routes to the org picker rather than sign-in or home. */
function needsOrg(account: Account | null, oauth: OAuthStatus | null): boolean {
  return (
    account?.auth_mode === "oauth" && (oauth?.signed_in ?? false) && !account.org_id
  );
}

export function App() {
  const platform = usePlatform();
  const [screen, setScreen] = useState<Screen>("loading");
  const [account, setAccount] = useState<Account | null>(null);
  const [oauth, setOAuth] = useState<OAuthStatus | null>(null);
  // Where the org picker returns to when done: "home" (startup re-pick),
  // "success" (fresh sign-in), or "settings" (Switch organization).
  const [orgPickerReturn, setOrgPickerReturn] = useState<Screen>("home");
  const [proxy, setProxy] = useState<ProxyState | null>(null);
  const [proxyBusy, setProxyBusy] = useState(false);
  const [providers, setProviders] = useState<ProviderState[]>([]);
  const [providerError, setProviderError] = useState<string | null>(null);
  const [tools, setTools] = useState<Tool[]>([]);
  // Codex drift usually means a hand-written Gate setup (the manual PAYG
  // instructions); enabling its provider adopts it one-way, so the Routing
  // screen shows a heads-up while this is true.
  const codexDrifted = tools.some((t) => t.slug === "codex" && t.status.kind === "drifted");
  // Set after a successful routing change; the Routing screen shows a
  // "restart your agent" note that auto-dismisses (also cleared when the
  // user leaves the screen).
  const [restartHint, setRestartHint] = useState(false);
  useEffect(() => {
    if (!restartHint) return;
    const t = setTimeout(() => setRestartHint(false), 8000);
    return () => clearTimeout(t);
  }, [restartHint]);
  // Flashed briefly when routing is turned on; the Routing screen shows a
  // Linux-only "relaunch your already-open apps" note that auto-dismisses.
  const [relaunchHint, setRelaunchHint] = useState(false);
  useEffect(() => {
    if (!relaunchHint) return;
    const t = setTimeout(() => setRelaunchHint(false), 8000);
    return () => clearTimeout(t);
  }, [relaunchHint]);

  // Set when the startup auto-enable brought routing back on a different
  // local port than the previous session (first launch after upgrading from
  // a build without port persistence, or the persisted port was taken).
  // Already-running AI apps keep dialing the dead old port, so Home shows a
  // restart notice. Sticky until the user dismisses it - unlike the
  // auto-dismissing hints above, their tools stay broken until acted on.
  const [staleAgentsHint, setStaleAgentsHint] = useState(false);
  const [staleAgentsDismissed, setStaleAgentsDismissed] = useState(false);

  // Set when routing flips on/off in a way worth a full-popover takeover:
  // either routing is already on as the app comes up (the initial load reads
  // it running, or the backend's startup auto-enable lands and announces
  // itself via `proxy-state-changed`), or the user toggles the proxy from the
  // home screen. Holds the direction so the takeover can word on vs off;
  // shown until dismissed.
  const [routingNotice, setRoutingNotice] = useState<"on" | "off" | null>(null);

  // App version, stamped into the bundle at release time and shown quietly
  // in the footer. Best-effort: stays empty (footer hidden) if it can't load.
  const [version, setVersion] = useState("");
  useEffect(() => {
    getVersion().then(setVersion).catch(() => {});
  }, []);

  // Initial load: account decides first-run vs home; proxy status is
  // best-effort (the proxy commands exist on all three desktop OSes, but a
  // failure here just hides the proxy UI via `showProxy`).
  useEffect(() => {
    let alive = true;
    (async () => {
      const acct = await getAccount().catch(() => null);
      // Best-effort: on failure treat as signed out (routes to sign-in), never
      // crashes the launch.
      const oauthState = await oauthStatus().catch(() => null);
      let px: ProxyState | null;
      try {
        px = await proxyStatus();
      } catch {
        px = null;
      }
      const provs = await listProviders().catch(() => []);
      const toolList = await listTools().catch(() => []);
      const stale = await routedClientsStale().catch(() => false);
      if (!alive) return;
      setAccount(acct);
      setOAuth(oauthState);
      setProxy(px);
      setProviders(provs);
      setTools(toolList);
      if (stale) setStaleAgentsHint(true);
      if (px?.running) setRoutingNotice("on");
      let resolved: Screen;
      if (isSignedIn(acct, oauthState)) {
        resolved = "home";
      } else if (needsOrg(acct, oauthState)) {
        // Signed in via OAuth but no org picked yet - go straight to the picker
        // (returning home once chosen), not back through sign-in.
        setOrgPickerReturn("home");
        resolved = "orgpicker";
      } else {
        resolved = "firstrun";
      }
      setScreen(resolved);
      if (!hasSeenTour()) {
        // First launch ever: open the window-sized intro and step the popover
        // aside; the intro hands back to the popover when it closes (see the
        // onboarding CloseRequested handler in src-tauri).
        openOnboardingWindow("firstrun").catch(() => {});
        getCurrentWindow().hide().catch(() => {});
      }
      track("app_launched", { has_account: !!acct, proxy_available: px !== null });
    })();
    return () => {
      alive = false;
    };
  }, []);

  // The backend can flip routing on by itself at startup (restart
  // persistence: it re-enables what the user last left on). Our status poll
  // stays idle while routing reads as off, and nothing re-reads when the
  // popover is reopened from the tray, so the backend emits a nudge after a
  // background enable and we re-read proxy + provider state here.
  useEffect(() => {
    let alive = true;
    const unlisten = listen("proxy-state-changed", async () => {
      const px = await proxyStatus().catch(() => null);
      const provs = await listProviders().catch(() => []);
      const toolList = await listTools().catch(() => []);
      const stale = await routedClientsStale().catch(() => false);
      if (!alive) return;
      setProxy(px);
      setProviders(provs);
      setTools(toolList);
      if (stale) setStaleAgentsHint(true);
      if (px?.running) setRoutingNotice("on");
    });
    return () => {
      alive = false;
      void unlisten.then((f) => f());
    };
  }, []);

  // The onboarding window announces completion; record the seen-flag in this
  // webview's storage too so the intro doesn't re-gate the next launch on
  // platforms where the two webviews don't share localStorage.
  useEffect(() => {
    const unlisten = listen(TOUR_SEEN_EVENT, () => markTourSeen());
    return () => {
      void unlisten.then((f) => f());
    };
  }, []);

  // (triggered by the initial load reading the key) can't dismiss it before
  // it's seen. Once the user actually interacts, release the pin so normal
  // click-away dismissal resumes.
  useEffect(() => {
    const engage = () => {
      void unpinPopover().catch(() => {});
      window.removeEventListener("pointerdown", engage);
      window.removeEventListener("keydown", engage);
    };
    window.addEventListener("pointerdown", engage);
    window.addEventListener("keydown", engage);
    return () => {
      window.removeEventListener("pointerdown", engage);
      window.removeEventListener("keydown", engage);
    };
  }, []);

  const refreshAccount = useCallback(async () => {
    setAccount(await getAccount().catch(() => null));
    setOAuth(await oauthStatus().catch(() => null));
  }, []);

  // OAuth sign-out: forget the stored tokens but keep the account, so the
  // popover returns to the sign-in prompt (not first-run) and routing config /
  // tool connections stay put for the next sign-in. The backend reverts a
  // running engine to the legacy header.
  const signOut = useCallback(async () => {
    await oauthSignOut();
    setOAuth(await oauthStatus().catch(() => null));
    setScreen("firstrun");
  }, []);

  // Open the org picker from Settings; it returns to Settings when done.
  const switchOrg = useCallback(() => {
    setOrgPickerReturn("settings");
    setScreen("orgpicker");
  }, []);

  const onConnected = useCallback(async () => {
    // Read the fresh account directly (state setters are async) so we can route
    // an OAuth sign-in that still needs an org straight to the picker.
    const acct = await getAccount().catch(() => null);
    const oauthState = await oauthStatus().catch(() => null);
    setAccount(acct);
    setOAuth(oauthState);
    track("signed_in");
    if (needsOrg(acct, oauthState)) {
      setOrgPickerReturn("success");
      setScreen("orgpicker");
    } else {
      setScreen("success");
    }
  }, []);

  // Org chosen (or auto-selected): refresh account state and return where the
  // picker was entered from.
  const onOrgChosen = useCallback(async () => {
    await refreshAccount();
    setScreen(orgPickerReturn);
  }, [refreshAccount, orgPickerReturn]);

  // `takeover: true` (the home-screen toggle) surfaces the result as the
  // full-popover routing notice; the Routing screen's toggle keeps its
  // inline hints instead.
  const toggleProxy = useCallback(
    async (takeover: boolean) => {
      if (proxyBusy) return;
      setProxyBusy(true);
      setProviderError(null);
      try {
        const next = proxy?.running ? await proxyDisable() : await proxyEnable();
        setProxy(next);
        track(next.running ? "proxy_enabled" : "proxy_disabled");
        // The takeover and the inline hints say the same thing ("restart your
        // agents"), so show one or the other, never both.
        if (takeover) {
          setRoutingNotice(next.running ? "on" : "off");
        } else {
          setRestartHint(true);
          if (next.running) setRelaunchHint(true);
        }
        // The backend owns the provider set across a master toggle: turning off
        // snapshots the on-providers and disables all; turning on restores that
        // snapshot. Just reflect the result - don't force every available
        // provider on, which would clobber the ones the user deliberately left off.
        setProviders(await listProviders().catch(() => []));
      } catch (e) {
        trackError(e, "generic");
        // Surface why the toggle failed (e.g. on Linux the CA-trust admin step
        // or a missing network service) instead of silently reverting - a
        // swallowed error reads as "the toggle does nothing".
        setProviderError(typeof e === "string" ? e : String(e));
        // Re-sync to the true state after the failed toggle.
        try {
          setProxy(await proxyStatus());
        } catch {
          /* noop */
        }
        setProviders(await listProviders().catch(() => []));
      } finally {
        setProxyBusy(false);
      }
    },
    [proxy, proxyBusy],
  );

  const setProvider = useCallback(
    async (slug: string, enabled: boolean) => {
      if (proxyBusy) return;
      setProxyBusy(true);
      setProviderError(null);
      try {
        if (enabled) await providerEnable(slug);
        else await providerDisable(slug);
        track("provider_toggled", { provider: slug, enabled });
        setRestartHint(true);
        if (enabled) setRelaunchHint(true);
        // Refresh provider state and proxy (enabling may have flipped a domain).
        setProviders(await listProviders().catch(() => []));
        setTools(await listTools().catch(() => []));
        try {
          setProxy(await proxyStatus());
        } catch {
          /* non-macOS: no proxy subsystem */
        }
      } catch (e) {
        setProviderError(typeof e === "string" ? e : String(e));
        trackError(e, "generic");
        // Re-sync the switch to its true state after a failed toggle.
        setProviders(await listProviders().catch(() => []));
      } finally {
        setProxyBusy(false);
      }
    },
    [proxyBusy],
  );

  const trustCa = useCallback(async () => {
    if (proxyBusy) return;
    setProxyBusy(true);
    try {
      setProxy(await proxyTrustCa());
      track("ca_trusted");
    } catch {
      // a cancelled trust dialog rejects; re-sync instead of leaking an
      // unhandled rejection with the banner stuck in its old state
      try {
        setProxy(await proxyStatus());
      } catch {
        /* noop */
      }
    } finally {
      setProxyBusy(false);
    }
  }, [proxyBusy]);

  const untrustCa = useCallback(async () => {
    if (proxyBusy) return;
    setProxyBusy(true);
    try {
      setProxy(await proxyUntrustCa());
      track("ca_untrusted");
    } catch {
      // a cancelled removal dialog rejects; re-sync instead of leaking an
      // unhandled rejection with the banner stuck in its old state
      try {
        setProxy(await proxyStatus());
      } catch {
        /* noop */
      }
    } finally {
      setProxyBusy(false);
    }
  }, [proxyBusy]);

  // Legacy key accounts can switch to Constellation sign-in from Settings; the
  // OAuth flow flips auth_mode to OAuth on success, then onConnected routes to
  // the org picker.
  const upgradeToOAuth = useCallback(async () => {
    await oauthBeginLogin();
    await onConnected();
  }, [onConnected]);

  const replaceKey = useCallback(
    async (key: string) => {
      const base = account?.gateway_base_url;
      if (!base) return;
      await saveAccount(base, key);
      await refreshAccount();
      track("key_replaced");
    },
    [account, refreshAccount],
  );

  const switchGatewayServer = useCallback(async (url: string) => {
    await switchGateway(url);
    // Switching forgets the stored key, disconnects managed tools, and leaves
    // the running proxy/providers pointed at the old gateway. Rather than patch
    // all that live, relaunch into a clean session that re-reads the new
    // account and lets the user enter an environment-appropriate key.
    await relaunch();
  }, []);

  const disconnect = useCallback(async () => {
    if (proxy?.running) {
      // A failed disable can leave system HTTPS pointed at a dead engine
      // port - abort the sign-out and surface it instead of silently
      // proceeding to first-run with the machine's traffic stranded.
      try {
        setProxy(await proxyDisable());
      } catch (err) {
        try {
          setProxy(await proxyStatus());
        } catch {
          /* noop */
        }
        throw err;
      }
    }
    // clear_account disconnects managed tools before wiping the account;
    // if that fails we are still signed in, so let the rejection reach
    // Settings instead of showing first-run over a half-signed-out state.
    await clearAccount();
    track("disconnected");
    setAccount(null);
    setScreen("firstrun");
  }, [proxy]);

  const workspace = hostOf(account?.gateway_base_url);
  const proxyOn = proxy?.running ?? false;
  const providerCount = providers.filter(
    (p) => p.enabled && !HIDDEN_PROVIDER_SLUGS.has(p.slug),
  ).length;
  const showProxy = proxy !== null;

  let body: ReactNode;
  if (screen === "loading") {
    // Startup screen while we resolve account + proxy (and the macOS keychain
    // dialog reads the key) - show the brand lockup instead of a blank popover.
    body = (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 py-16">
        <ConstellationHexMark size={40} fill="#002a5f" />
        <span className="text-[15px] font-semibold tracking-[-0.02em] text-gc-navy">
          Gate <span className="text-gc-accent">Connect</span>
        </span>
      </div>
    );
  } else if (screen === "firstrun") {
    body = (
      <FirstRun
        onConnected={onConnected}
        initialGateway={account?.gateway_base_url}
        // An existing OAuth account here means a prior session that's no longer
        // signed in (silent refresh failed / explicit sign-out): show the
        // welcome-back re-auth copy rather than the first-run welcome.
        reauth={!!account && account.auth_mode === "oauth"}
      />
    );
  } else if (screen === "orgpicker") {
    body = (
      <OrgPicker
        onDone={onOrgChosen}
        onBack={orgPickerReturn === "settings" ? () => setScreen("settings") : undefined}
        onReauth={signOut}
      />
    );
  } else if (screen === "success") {
    body = (
      <Success
        workspace={workspace}
        proxyOn={proxyOn}
        onDone={() => setScreen("home")}
        onOpenSettings={() => setScreen("settings")}
      />
    );
  } else if (screen === "proxy" && proxy) {
    body = (
      <ProxyScreen
        proxy={proxy}
        providers={providers.filter((p) => !HIDDEN_PROVIDER_SLUGS.has(p.slug))}
        busy={proxyBusy}
        error={providerError}
        restartHint={restartHint}
        relaunchHint={relaunchHint}
        codexDrifted={codexDrifted}
        onBack={() => {
          setProviderError(null);
          // Keep restart/relaunch hints alive so a change made here still
          // reminds the user on the home screen; their timers auto-dismiss.
          setScreen("home");
        }}
        onToggleProxy={() => toggleProxy(false)}
        onSetProvider={setProvider}
        onTrustCa={trustCa}
        onUntrustCa={untrustCa}
      />
    );
  } else if (screen === "settings" && account) {
    body = (
      <Settings
        account={account}
        oauth={oauth}
        onBack={() => setScreen("home")}
        onReplaceKey={replaceKey}
        onUpgradeToOAuth={upgradeToOAuth}
        onDisconnect={disconnect}
        onSignOut={signOut}
        onSwitchOrg={switchOrg}
        onSwitchGateway={switchGatewayServer}
        onReplayTour={() => {
          openOnboardingWindow("settings").catch(() => {});
        }}
        routingOn={proxyOn}
      />
    );
  } else if (screen === "coming-soon") {
    body = <ComingSoon onBack={() => setScreen("home")} />;
  } else {
    // home (and any fallback once loaded)
    body = (
      <Home
        workspace={workspace}
        proxyOn={proxyOn}
        providerCount={providerCount}
        showProxy={showProxy}
        error={providerError}
        restartHint={restartHint}
        relaunchHint={relaunchHint}
        staleAgentsHint={staleAgentsHint && !staleAgentsDismissed}
        onDismissStaleAgents={() => setStaleAgentsDismissed(true)}
        onOpenProxy={() => setScreen("proxy")}
        onToggleProxy={() => toggleProxy(true)}
        onOpenDirectGateway={() => setScreen("coming-soon")}
        onOpenSettings={() => setScreen("settings")}
      />
    );
  }

  return (
    <div
      className={`relative flex h-full w-full flex-col overflow-y-auto overflow-x-hidden bg-gc-surface text-gc-ink${
        // Linux runs as a borderless, opaque window, so a rounded card just
        // exposes the square window corners behind it. macOS/Windows round the
        // window itself, so the card rounds to match.
        platform === "linux" ? "" : " rounded-gc-lg"
      }${
        // While a proxy toggle is in flight, show the OS busy cursor everywhere
        // (the `!` overrides children's cursor-pointer / cursor-not-allowed) so
        // the slow enable/disable reads as "working", not "did nothing".
        proxyBusy ? " cursor-wait [&_*]:!cursor-wait" : ""
      }`}
    >
      {platform === "linux" && <LinuxTitleBar />}
      {/* Renders the startup takeover as an absolute overlay, or (on reopen)
          the slim update banner in-flow at the top of the popover - hence its
          placement above the body. */}
      <UpdatePanel />
      {routingNotice !== null && screen !== "loading" && (
        <StartupRoutingNotice
          routingOn={routingNotice === "on"}
          onDismiss={() => setRoutingNotice(null)}
        />
      )}
      {body}
      {version && (
        <p className="mt-auto shrink-0 px-3.5 py-2 text-center font-mono text-[10.5px] text-gc-ink-5">
          v{version}
        </p>
      )}
    </div>
  );
}
