import { useCallback, useEffect, useRef, useState } from "react";
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
  connectTool,
  disconnectTool,
  launchAtLoginStatus,
  unpinPopover,
  openOnboardingWindow,
  routedClientsStale,
  runningAgentsCount,
  drainBackendErrors,
  pendingQuitTools,
} from "./lib/api";
import { FirstRun } from "./screens/FirstRun";
import { OrgPicker } from "./screens/OrgPicker";
import { Home } from "./screens/Home";
import { ProxyScreen } from "./screens/ProxyScreen";
import { Settings } from "./screens/Settings";
import { Success } from "./screens/Success";
import { ToolDetail } from "./screens/ToolDetail";
import { UpdatePanel } from "./components/UpdatePanel";
import { StartupRoutingNotice } from "./components/StartupRoutingNotice";
import { QuitConfirm } from "./components/QuitConfirm";
import { LinuxTitleBar } from "./components/LinuxTitleBar";
import { ConstellationHexMark } from "./components/gc/ConstellationHexMark";
import { track, trackError } from "./lib/analytics";
import { backendErrorContext, classifyError, type ClassifiedError } from "./lib/errors";
import { hasSeenTour, markTourSeen } from "./lib/tour";
import { TOUR_SEEN_EVENT } from "./screens/Onboarding";
import { usePlatform } from "./lib/platform";
import { useWindowReopen } from "./lib/useWindowReopen";

type Screen =
  | "loading"
  | "firstrun"
  | "orgpicker"
  | "home"
  | "proxy"
  | "settings"
  | "success"
  | "tool";

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

/** Drain the backend's buffered failures into the analytics seam. The raw
 * message is classified frontend-side like any invoke rejection; only the
 * title goes over the wire. */
async function forwardBackendErrors(): Promise<void> {
  const errs = await drainBackendErrors().catch(() => []);
  for (const e of errs) trackError(e.message, backendErrorContext(e.context));
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
  const [providerError, setProviderError] = useState<ClassifiedError | null>(null);
  const [tools, setTools] = useState<Tool[]>([]);
  // Which tool the "tool" screen shows; set by tapping a ledger row on Home.
  const [toolSlug, setToolSlug] = useState<string | null>(null);
  // Codex drift usually means a hand-written Gate setup (the manual PAYG
  // instructions); enabling its provider adopts it one-way, so the Routing
  // screen shows a heads-up while this is true.
  const codexDrifted = tools.some((t) => t.slug === "codex" && t.status.kind === "drifted");
  // Set after a successful routing change; Home and the Routing screen show a
  // "restart your agents" note. The advice holds until the agents actually
  // restart (which we can't observe), so the note stays until the user
  // dismisses it rather than vanishing on a timer.
  const [restartHint, setRestartHint] = useState(false);
  // Flashed when routing is turned on; the Linux-only "reopen your
  // already-open apps" note, dismissible like the restart hint.
  const [relaunchHint, setRelaunchHint] = useState(false);

  // Set when the startup auto-enable brought routing back on a different
  // local port than the previous session (first launch after upgrading from
  // a build without port persistence, or the persisted port was taken).
  // Already-running AI apps keep dialing the dead old port, so Home shows a
  // restart notice. Sticky until the user dismisses it - unlike the
  // auto-dismissing hints above, their tools stay broken until acted on.
  const [staleAgentsHint, setStaleAgentsHint] = useState(false);
  const [staleAgentsDismissed, setStaleAgentsDismissed] = useState(false);

  // Set when routing flips on/off in a way worth a full-popover takeover: the
  // user toggled the proxy from the home screen while agents were running, or
  // asked for it from the startup banner's "Close agents…" action (which
  // opens straight on the confirm step - the banner click already declared
  // the intent). Shown until dismissed. Routing that comes up on its own at
  // startup gets the calm inline `startupRoutingHint` on Home instead.
  const [routingNotice, setRoutingNotice] = useState<{
    dir: "on" | "off";
    confirming: boolean;
  } | null>(null);
  // The takeover teaches its lesson once per session; after the first
  // acknowledgment, later toggles fall back to the inline restart hint that
  // carries the same advice, so the daily user isn't re-interrupted.
  const routingNoticeSeen = useRef(false);
  // Whether the update panel's startup takeover is currently mounted, so the
  // background can go aria-hidden while any takeover is up.
  const [updateTakeoverVisible, setUpdateTakeoverVisible] = useState(false);

  // Routing came back on its own as the app started (initial load read it
  // running, or the backend's startup auto-enable announced itself) while
  // agents were running. Informational, not an alarm: surfaces as a
  // dismissible Home banner.
  const [startupRoutingHint, setStartupRoutingHint] = useState(false);

  // The tray Quit defers to the popover when config-routed CLI tools are
  // still managed (their configs point at the loopback relay, which dies
  // with the app). Holds the connected tool names; non-null shows the quit
  // takeover. The names are swept from a backend buffer (once at mount, then
  // on each nudge) rather than carried on the event, so a Quit clicked
  // before this listener registered isn't lost.
  const [quitTools, setQuitTools] = useState<string[] | null>(null);
  useEffect(() => {
    const sweep = () => {
      pendingQuitTools()
        .then((tools) => {
          if (tools && tools.length > 0) setQuitTools(tools);
        })
        .catch(() => {});
    };
    sweep();
    const unlisten = listen("quit-requested", sweep);
    return () => {
      void unlisten.then((f) => f()).catch(() => {});
    };
  }, []);

  // App version, stamped into the bundle at release time and shown quietly
  // in the footer. Best-effort: stays empty (footer hidden) if it can't load.
  const [version, setVersion] = useState("");
  useEffect(() => {
    getVersion().then(setVersion).catch(() => {});
  }, []);

  // Initial load: account decides first-run vs home; proxy status is
  // best-effort (the proxy commands exist on all three desktop OSes, but a
  // failure here just hides the proxy UI via `showProxy`).
  //
  // The backend can flip routing on by itself at startup (restart
  // persistence: it re-enables what the user last left on) and announces it
  // with a single `proxy-state-changed` nudge; our status poll stays idle
  // while routing reads as off, and nothing re-reads when the popover is
  // reopened from the tray. Registering the listener *before* the first
  // status read (and awaiting the registration) closes the gap where the
  // enable lands after the read but before the listener is live - that
  // one-shot nudge would otherwise be missed for the webview's lifetime.
  useEffect(() => {
    let alive = true;
    const unlisten = listen("proxy-state-changed", async () => {
      const px = await proxyStatus().catch(() => null);
      const provs = await listProviders().catch(() => []);
      const toolList = await listTools().catch(() => []);
      const stale = await routedClientsStale().catch(() => false);
      // The hint only says "restart your agents", so skip it when none are
      // running; a failed probe defaults to showing.
      const agents = px?.running ? await runningAgentsCount().catch(() => 1) : 0;
      if (!alive) return;
      setProxy(px);
      setProviders(provs);
      setTools(toolList);
      if (stale) setStaleAgentsHint(true);
      if (px?.running) {
        if (agents > 0) setStartupRoutingHint(true);
        // The backend only emits this nudge after its startup auto-enable, so
        // routing coming up here is a restored session, not a user toggle.
        track("proxy_enabled", { source: "restored" });
      }
    });
    (async () => {
      // A failed registration shouldn't block the popover from loading.
      await unlisten.catch(() => {});
      // Each load degrades to its empty default so the popover still opens;
      // the failure itself is tracked rather than swallowed.
      const acct = await getAccount().catch((err) => {
        trackError(err, "startup");
        return null;
      });
      // Best-effort: on failure treat as signed out (routes to sign-in), never
      // crashes the launch.
      const oauthState = await oauthStatus().catch(() => null);
      let px: ProxyState | null;
      try {
        px = await proxyStatus();
      } catch (err) {
        trackError(err, "startup");
        px = null;
      }
      const provs = await listProviders().catch((err) => {
        trackError(err, "startup");
        return [];
      });
      const toolList = await listTools().catch((err) => {
        trackError(err, "startup");
        return [];
      });
      const stale = await routedClientsStale().catch((err) => {
        trackError(err, "startup");
        return false;
      });
      // Analytics-only dimension on app_launched; omitted when unreadable.
      const lal = await launchAtLoginStatus().catch(() => null);
      // Same gate as the proxy-state-changed listener: no running agents,
      // nothing to restart, no hint; a failed probe defaults to showing.
      const agents = px?.running ? await runningAgentsCount().catch(() => 1) : 0;
      if (!alive) return;
      setAccount(acct);
      setOAuth(oauthState);
      setProxy(px);
      setProviders(provs);
      setTools(toolList);
      if (stale) setStaleAgentsHint(true);
      if (px?.running && agents > 0) setStartupRoutingHint(true);
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
      track("app_launched", {
        has_account: !!acct,
        proxy_available: px !== null,
        routing_on: px?.running ?? false,
        provider_count: provs.filter((p) => p.enabled).length,
        // Sizes the hand-written-Gate-setup population (see codexDrifted).
        codex_drifted: toolList.some((t) => t.slug === "codex" && t.status.kind === "drifted"),
        ...(lal === null ? {} : { launch_at_login: lal.enabled }),
      });
    })();
    return () => {
      alive = false;
      void unlisten.then((f) => f()).catch(() => {});
    };
  }, []);

  // Reopens from the tray: the launch itself is app_launched; only returns
  // count here.
  useWindowReopen(() => track("popover_opened"));

  // Backend failures buffer Rust-side because they can predate this webview
  // (the startup auto-enable runs before the popover mounts). Sweep the
  // buffer once at mount, then drain again on each nudge.
  useEffect(() => {
    void forwardBackendErrors();
    const unlisten = listen("backend-error-pending", () => void forwardBackendErrors());
    return () => {
      void unlisten.then((f) => f());
    };
  }, []);

  // Exposure events for the two "your tools need attention" surfaces, fired
  // on the state edge so the two set-sites (initial load and the backend
  // nudge) don't each need their own call.
  useEffect(() => {
    if (staleAgentsHint) track("stale_agents_shown");
  }, [staleAgentsHint]);
  useEffect(() => {
    if (startupRoutingHint) track("routing_notice_shown", { enabled: true, inline: true });
  }, [startupRoutingHint]);
  useEffect(() => {
    if (routingNotice !== null) {
      track("routing_notice_shown", { enabled: routingNotice.dir === "on" });
    }
  }, [routingNotice]);
  useEffect(() => {
    if (quitTools !== null) track("quit_warning_shown", { tool_count: quitTools.length });
  }, [quitTools]);

  // The popover webview persists across tray hide/show, so the initial-load
  // effect doesn't re-run when the user reopens the popover. Re-check the OAuth
  // session whenever the window regains focus: if the access token expired while
  // the popover was closed and couldn't be silently refreshed (refresh token
  // revoked / offline), drop to the sign-in prompt instead of leaving a
  // signed-in home up that's actually riding the legacy API-key fallback. Scoped
  // to OAuth accounts - a pasted-key account has no session to expire.
  useEffect(() => {
    if (account?.auth_mode !== "oauth") return;
    let alive = true;
    const unlisten = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (!focused) return;
      void (async () => {
        const oauthState = await oauthStatus().catch(() => null);
        if (!alive) return;
        setOAuth(oauthState);
        // Session died out from under us: prompt re-sign-in. Guard on needsOrg so
        // a signed-in-but-org-pending session isn't yanked off the picker.
        if (!isSignedIn(account, oauthState) && !needsOrg(account, oauthState)) {
          setScreen("firstrun");
        }
      })();
    });
    return () => {
      alive = false;
      void unlisten.then((f) => f());
    };
  }, [account]);

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

  // Move focus to the incoming panel's heading on every screen change, so
  // assistive tech announces where the user landed and Tab starts at the top
  // of the new panel instead of dying on the unmounted control they clicked.
  // Skips the initial resolve (loading -> first screen): stealing focus the
  // moment the popover opens would fight the OS.
  const prevScreen = useRef<Screen>("loading");
  useEffect(() => {
    const prev = prevScreen.current;
    prevScreen.current = screen;
    if (prev === "loading" || prev === screen || screen === "loading") return;
    requestAnimationFrame(() => {
      document.querySelector<HTMLElement>("[data-screen-focus]")?.focus();
    });
  }, [screen]);

  // Escape steps back out of the sub-screens (Settings, Routing), the same
  // exit the header back button offers. The takeovers own Escape themselves
  // via their focus traps, and text fields keep it (clearing/IME), so this
  // only fires when neither is in play.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      const target = e.target as HTMLElement | null;
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA")) return;
      if (quitTools !== null || routingNotice !== null) return;
      if (screen === "settings" || screen === "proxy" || screen === "tool") {
        setProviderError(null);
        setScreen("home");
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [screen, quitTools, routingNotice]);

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
        track(next.running ? "proxy_enabled" : "proxy_disabled", { source: "toggle" });
        // The takeover and the inline hints say the same thing ("restart your
        // agents"), so show one or the other, never both.
        if (takeover) {
          // Nothing running means nothing to close: skip the takeover
          // entirely; a failed probe defaults to showing. After the first
          // acknowledged takeover this session, degrade to the inline hint -
          // both carry the same "restart your agents" advice.
          const agents = await runningAgentsCount().catch(() => 1);
          if (agents > 0) {
            if (!routingNoticeSeen.current) {
              routingNoticeSeen.current = true;
              setRoutingNotice({ dir: next.running ? "on" : "off", confirming: false });
            } else {
              setRestartHint(true);
              if (next.running) setRelaunchHint(true);
            }
          }
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
        trackError(e, "proxy_toggle");
        // Surface why the toggle failed (e.g. on Linux the CA-trust admin step
        // or a missing network service) instead of silently reverting - a
        // swallowed error reads as "the toggle does nothing".
        setProviderError(classifyError(e, "proxy_toggle"));
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
        setProviderError(classifyError(e, "provider_toggle"));
        trackError(e, "provider_toggle", { provider: slug, enabled });
        // Re-sync the switch to its true state after a failed toggle.
        setProviders(await listProviders().catch(() => []));
      } finally {
        setProxyBusy(false);
      }
    },
    [proxyBusy],
  );

  // Connect or disconnect one tool from its detail screen. Rethrows so the
  // caller can classify and display the failure in place; either way the
  // finally block re-syncs to backend truth (the toggle can flip the
  // provider headline, and connect auto-enables the proxy engine).
  const setToolRouted = useCallback(
    async (slug: string, routed: boolean) => {
      if (proxyBusy) return;
      setProxyBusy(true);
      try {
        const tool = tools.find((t) => t.slug === slug);
        if (routed) await connectTool(slug, tool?.default_upstream_url ?? "");
        else await disconnectTool(slug);
        track("tool_toggled", { tool: slug, routed });
        setRestartHint(true);
      } catch (e) {
        trackError(e, "connect", { tool: slug, routed });
        throw e;
      } finally {
        setTools(await listTools().catch(() => tools));
        setProviders(await listProviders().catch(() => []));
        try {
          setProxy(await proxyStatus());
        } catch {
          /* non-macOS: no proxy subsystem */
        }
        setProxyBusy(false);
      }
    },
    [proxyBusy, tools],
  );

  const trustCa = useCallback(async () => {
    if (proxyBusy) return;
    setProxyBusy(true);
    try {
      setProxy(await proxyTrustCa());
      track("ca_trusted");
    } catch (err) {
      // a cancelled trust dialog rejects; re-sync instead of leaking an
      // unhandled rejection with the banner stuck in its old state. Tracked
      // so a genuine keychain failure is visible (cancels classify apart).
      trackError(err, "trust_ca");
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
    } catch (err) {
      // a cancelled removal dialog rejects; re-sync instead of leaking an
      // unhandled rejection with the banner stuck in its old state. Tracked
      // so a genuine keychain failure is visible (cancels classify apart).
      trackError(err, "untrust_ca");
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

  const forget = useCallback(async () => {
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
    track("workspace_forgotten");
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
        <span className="text-[14.5px] font-semibold tracking-[-0.02em] text-gc-navy">
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
        showProxy={showProxy}
        busy={proxyBusy}
        onTurnOnRouting={async () => {
          // Inline hints (not the takeover): the user is mid-flow and lands on
          // Home right after, where the restart hint carries the follow-up.
          await toggleProxy(false);
          setScreen("home");
        }}
        onDone={() => setScreen("home")}
        onOpenSettings={() => setScreen("settings")}
      />
    );
  } else if (screen === "proxy" && proxy) {
    body = (
      <ProxyScreen
        proxy={proxy}
        providers={providers.filter((p) => !HIDDEN_PROVIDER_SLUGS.has(p.slug))}
        tools={tools}
        busy={proxyBusy}
        error={providerError}
        restartHint={restartHint}
        onDismissRestartHint={() => setRestartHint(false)}
        relaunchHint={relaunchHint}
        onDismissRelaunchHint={() => setRelaunchHint(false)}
        codexDrifted={codexDrifted}
        onBack={() => {
          setProviderError(null);
          // Keep restart/relaunch hints alive so a change made here still
          // reminds the user on the home screen until dismissed.
          setScreen("home");
        }}
        onToggleProxy={() => toggleProxy(false)}
        onSetProvider={setProvider}
        onTrustCa={trustCa}
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
        onForget={forget}
        onSignOut={signOut}
        onSwitchOrg={switchOrg}
        onSwitchGateway={switchGatewayServer}
        onReplayTour={() => {
          openOnboardingWindow("settings").catch(() => {});
        }}
        routingOn={proxyOn}
        caTrusted={proxy?.ca_trusted ?? false}
        proxyBusy={proxyBusy}
        onUntrustCa={untrustCa}
      />
    );
  } else if (screen === "tool" && toolSlug && tools.some((t) => t.slug === toolSlug)) {
    body = (
      <ToolDetail
        tool={tools.find((t) => t.slug === toolSlug)!}
        providers={providers}
        busy={proxyBusy}
        onSetRouted={(routed) => setToolRouted(toolSlug, routed)}
        onBack={() => setScreen("home")}
      />
    );
  } else {
    // home (and any fallback once loaded)
    body = (
      <Home
        workspace={workspace}
        proxyOn={proxyOn}
        caTrusted={proxy?.ca_trusted ?? true}
        providerCount={providerCount}
        showProxy={showProxy}
        tools={tools}
        error={providerError}
        restartHint={restartHint}
        onDismissRestartHint={() => setRestartHint(false)}
        relaunchHint={relaunchHint}
        onDismissRelaunchHint={() => setRelaunchHint(false)}
        startupRoutingHint={startupRoutingHint}
        onDismissStartupRoutingHint={() => setStartupRoutingHint(false)}
        // User-initiated, so the full takeover is earned here even though
        // startup itself no longer opens it - and since the banner click
        // already declared the intent, land directly on the confirm step.
        onCloseAgents={() => setRoutingNotice({ dir: "on", confirming: true })}
        staleAgentsHint={staleAgentsHint && !staleAgentsDismissed}
        onDismissStaleAgents={() => setStaleAgentsDismissed(true)}
        onOpenProxy={() => setScreen("proxy")}
        onOpenTool={(slug) => {
          setToolSlug(slug);
          setScreen("tool");
        }}
        onToggleProxy={() => toggleProxy(true)}
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
          placement above the body. The takeover defers while the quit or
          routing takeover is up, so it can never mount under one (z-20 vs
          the quit takeover's z-30 / over the routing notice's z-10) and trap
          focus in a hidden panel. */}
      <UpdatePanel
        suppressTakeover={quitTools !== null || routingNotice !== null}
        onTakeoverVisibleChange={setUpdateTakeoverVisible}
      />
      {routingNotice !== null && screen !== "loading" && (
        <StartupRoutingNotice
          routingOn={routingNotice.dir === "on"}
          startConfirming={routingNotice.confirming}
          onDismiss={() => setRoutingNotice(null)}
          onAgentsClosed={() => setStartupRoutingHint(false)}
        />
      )}
      {quitTools !== null && (
        <QuitConfirm tools={quitTools} onCancel={() => setQuitTools(null)} />
      )}
      {/* While any takeover is up, the obscured content goes aria-hidden so
          a screen reader's virtual cursor can't wander under the dialog (the
          focus traps already handle Tab). */}
      <div
        className="flex grow flex-col"
        aria-hidden={
          quitTools !== null || routingNotice !== null || updateTakeoverVisible
            ? true
            : undefined
        }
      >
        {body}
        {version && (
          <p className="mt-auto shrink-0 px-3.5 py-2 text-center font-mono text-[10.5px] text-gc-ink-4">
            v{version}
          </p>
        )}
      </div>
    </div>
  );
}
