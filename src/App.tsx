import { useCallback, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import type { Account, OAuthStatus, ProviderState, ProxyState, Tool } from "./lib/api";
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
  proxySetEnvExport,
  proxyUntrustCa,
  proxySetDomain,
  listProviders,
  listTools,
  connectTool,
  disconnectTool,
  launchAtLoginStatus,
  unpinPopover,
  openOnboardingWindow,
  routedClientsStale,
  runningAgentsCount,
  staleAgentsCount,
  drainBackendErrors,
  pendingQuitTools,
} from "./lib/api";
import { FirstRun } from "./screens/FirstRun";
import { OrgPicker } from "./screens/OrgPicker";
import { Home } from "./screens/Home";
import { FamilyPanel } from "./screens/FamilyPanel";
import { Settings } from "./screens/Settings";
import { Success } from "./screens/Success";
import { UpdatePanel } from "./components/UpdatePanel";
import { RoutingChangeNotice } from "./components/RoutingChangeNotice";
import { QuitConfirm } from "./components/QuitConfirm";
import { OAuthOffer } from "./components/OAuthOffer";
import { LinuxTitleBar } from "./components/LinuxTitleBar";
import { ConstellationHexMark } from "./components/gc/ConstellationHexMark";
import { Icon } from "./components/gc/Icon";
import { track, trackError } from "./lib/analytics";
import { backendErrorContext, classifyError, type ClassifiedError } from "./lib/errors";
import { buildGroups } from "./lib/groups";
import { useTextScale } from "./lib/useTextScale";
import { hasSeenTour, markTourSeen } from "./lib/tour";
import { hasSeenOAuthOffer, markOAuthOfferSeen } from "./lib/oauthOffer";
import { TOUR_SEEN_EVENT } from "./screens/Onboarding";
import { secretStoreName, usePlatform } from "./lib/platform";
import { useWindowReopen } from "./lib/useWindowReopen";

type Screen =
  | "loading"
  | "firstrun"
  | "orgpicker"
  | "home"
  | "settings"
  | "success"
  | "family";

/** How deep each screen sits in the popover. In one 360px room, direction is
 *  the only navigational metaphor available: without it a push and a pop look
 *  identical and the user never builds a sense of where Settings is relative
 *  to Home. DESIGN.md has promised this grammar since the start; only the
 *  takeovers had it. `loading` is deliberately absent - the first paint has
 *  nothing to slide away from. */
const SCREEN_DEPTH: Record<Screen, number> = {
  loading: 0,
  firstrun: 0,
  orgpicker: 1,
  success: 2,
  home: 0,
  settings: 1,
  // A family has a screen of its own again, and it is the only thing on it. It
  // sits at depth 1 rather than 2 because it opens straight off Home: the
  // all-families ledger that used to be the level in between is gone, since
  // Home already lists every family and the panel only ever showed one.
  family: 1,
};

// Proxy domains hidden from the Apps ledger. "chatgpt" exists so the relay
// recognizes the Codex integration's upstream hint - it's plumbing for the
// Codex tool, not an app the user routes independently.
const HIDDEN_DOMAIN_SLUGS = new Set<string>(["chatgpt"]);

function hostOf(url: string | undefined): string {
  if (!url) return "";
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/** Contexts whose failure means traffic is not flowing. These are the startup
 *  restore paths, which is exactly why the buffer exists: they run before the
 *  webview does, so the user has no other way to learn the engine never came
 *  up. A failure here is a total routing outage that the popover would
 *  otherwise render as a healthy screen. */
const ROUTING_DOWN_CONTEXTS = new Set(["restore_routing", "provider_restore", "provider_reconcile"]);

/** Drain the backend's buffered failures into the analytics seam, and hand back
 *  the first one that means routing is down so a human sees it too. The raw
 *  message is classified frontend-side like any invoke rejection; only the
 *  title goes over the wire.
 *
 *  Telemetry was the only consumer. A buffered "failed to bind loopback port
 *  8317: address in use" produced zero pixels of UI, in the one app whose
 *  first principle is reassurance through transparency, on the one error class
 *  the user cannot discover any other way. */
async function forwardBackendErrors(): Promise<ClassifiedError | null> {
  const errs = await drainBackendErrors().catch(() => []);
  let surfaced: ClassifiedError | null = null;
  for (const e of errs) {
    const context = backendErrorContext(e.context);
    trackError(e.message, context);
    if (!surfaced && ROUTING_DOWN_CONTEXTS.has(e.context)) {
      surfaced = classifyError(e.message, context);
    }
  }
  return surfaced;
}

// The routing takeover teaches its lesson once per install; after the first
// acknowledgment (persisted like the tour flag), later toggles fall back to
// the inline restart hint that carries the same advice, so the daily user
// isn't re-interrupted every session. Storage failures degrade to "already
// seen" - never trap the user in a recurring takeover.
const ROUTING_TAKEOVER_SEEN_KEY = "gc.routing-takeover.v1.seen";
function hasSeenRoutingTakeover(): boolean {
  try {
    return localStorage.getItem(ROUTING_TAKEOVER_SEEN_KEY) === "1";
  } catch {
    return true;
  }
}
function markRoutingTakeoverSeen(): void {
  try {
    localStorage.setItem(ROUTING_TAKEOVER_SEEN_KEY, "1");
  } catch {
    /* noop */
  }
}

/** What the last routing change resulted in. "started" means the change
 * turned the master on as a side effect; "pending" means it did not and
 * nothing is routing. */
export type ChangeNotice = "on" | "off" | "started" | "pending" | null;

/** Which change notice a member/group toggle earned, from the engine state
 * that actually resulted rather than from the direction of the click.
 *
 * `on`/`off` both mean traffic is flowing, so the close-your-apps advice
 * applies and only the wording differs. `pending` is the case the old
 * two-value notice had no room for: the user switched something on, the engine
 * is down, nothing routes, and telling them to close their apps would be
 * false. Turning something off while the engine is already down changed
 * nothing observable, so it earns no notice at all. */
function noticeFor(
  turnedOn: boolean,
  engineRunning: boolean,
  engineWasRunning: boolean,
): ChangeNotice {
  // The master is a control, and a family switch is allowed to turn it on:
  // connecting a config tool has to, because the tool's file points at the
  // loopback relay and the relay only exists while the engine runs. The rule
  // is do it and say so, so a side-effect start gets its own notice rather
  // than hiding inside the generic "Routing is on".
  if (engineRunning && !engineWasRunning && turnedOn) return "started";
  if (engineRunning) return turnedOn ? "on" : "off";
  return turnedOn ? "pending" : null;
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
  // Text scaling owns the rem root and the Cmd/Ctrl +/-/0 accelerators. Mounted
  // here rather than per screen so the shortcuts work on every surface,
  // including the takeovers.
  const { scale: textScale, setScale: setTextScale } = useTextScale();
  const [screen, setScreen] = useState<Screen>("loading");
  // Direction of the current screen change, derived during render (the React
  // pattern for adjusting state when an input changes) rather than in an
  // effect, so the panel slides in on its first paint instead of a frame
  // late. Nothing animates away from `loading`: the first screen has no
  // predecessor to have come from.
  const [motionFrom, setMotionFrom] = useState<Screen>("loading");
  const [screenMotion, setScreenMotion] = useState("");
  if (motionFrom !== screen) {
    setScreenMotion(
      motionFrom === "loading"
        ? ""
        : SCREEN_DEPTH[screen] > SCREEN_DEPTH[motionFrom]
          ? "ob-slide-in-fwd"
          : SCREEN_DEPTH[screen] < SCREEN_DEPTH[motionFrom]
            ? "ob-slide-in-back"
            : "",
    );
    setMotionFrom(screen);
  }
  const [account, setAccount] = useState<Account | null>(null);
  const [oauth, setOAuth] = useState<OAuthStatus | null>(null);
  // Where the org picker returns to when done: "home" (startup re-pick),
  // "success" (fresh sign-in), or "settings" (Switch organization).
  const [orgPickerReturn, setOrgPickerReturn] = useState<Screen>("home");
  // Set when the org picker hands a user to the API-key fallback, so FirstRun
  // opens on the key form instead of making them find the disclosure again.
  const [startOnKey, setStartOnKey] = useState(false);
  // Which family the ledger panel should open with, set by the Home row that
  // opened it. Null when the user arrived from the heading rather than a row.
  const [openFamily, setOpenFamily] = useState<string | null>(null);
  const [proxy, setProxy] = useState<ProxyState | null>(null);
  const [proxyBusy, setProxyBusy] = useState(false);
  const [providerError, setProviderError] = useState<ClassifiedError | null>(null);
  const [tools, setTools] = useState<Tool[]>([]);
  // The provider catalog is the grouping contract for Home's ledger
  // (tool_slugs + domain_slugs), not just an analytics dimension.
  const [providers, setProviders] = useState<ProviderState[]>([]);
  // Set after any routing change, and by the startup auto-enable. One state,
  // not one per flavour of advice: three independent booleans meant a fast
  // on/off flip left the "on" notice standing over the "off" one, telling the
  // user the opposite of what just happened. Holds the direction of the last
  // change so the banner's wording follows the switch. The advice holds until
  // they actually close their tools (which we can't observe), so it stays
  // until dismissed rather than vanishing on a timer.
  // "pending" is the state the old two-value notice could not express: the
  // user switched something on and the engine is not running, so nothing
  // routes and the close-your-apps advice is simply wrong. Deriving the
  // notice from the toggle's direction produced "Routing is on" over a card
  // reading "Off · not routing", because proxy_set_domain never starts the
  // engine while connect_tool does.
  const [changeNotice, setChangeNotice] = useState<ChangeNotice>(null);

  // Set when the startup auto-enable brought routing back on a different
  // local port than the previous session (first launch after upgrading from
  // a build without port persistence, or the persisted port was taken).
  // Already-running AI apps keep dialing the dead old port, so Home shows a
  // restart notice. Sticky until the user dismisses it - unlike the
  // auto-dismissing hints above, their tools stay broken until acted on.
  const [staleAgentsHint, setStaleAgentsHint] = useState(false);
  const [staleAgentsDismissed, setStaleAgentsDismissed] = useState(false);

  // Set when routing flips on/off in a way worth a full-popover takeover: the
  // user toggled the proxy from the home screen while tools were running, or
  // asked for it from the Home banner's "Close them…" action (which opens
  // straight on the confirm step - the banner click already declared the
  // intent). Shown until dismissed. Routing that comes up on its own at
  // startup gets the calm inline `changeNotice` banner on Home instead (and
  // only when something predates it; see stale_agents_count).
  const [routingNotice, setRoutingNotice] = useState<{
    dir: "on" | "off";
    confirming: boolean;
  } | null>(null);
  // Whether the update panel's startup takeover is currently mounted, so the
  // background can go aria-hidden while any takeover is up.
  const [updateTakeoverVisible, setUpdateTakeoverVisible] = useState(false);
  // Whether the one-time OAuth offer is up. Armed on load, never re-armed.
  const [oauthOffer, setOAuthOffer] = useState(false);

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

  // App version, stamped into the bundle at release time. Shown in Settings
  // under Help rather than the popover footer: the footer strip carries two
  // items at 360px and the credential line and the dashboard link both earn
  // their place there more than a build number does. Best-effort.
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
  // with a single `proxy-state-changed` nudge. Reopening the popover from the
  // tray also re-reads, via `refreshState` below - it used to do neither, and
  // this comment used to claim a status poll that had never existed.
  // Registering the listener *before* the first
  // status read (and awaiting the registration) closes the gap where the
  // enable lands after the read but before the listener is live - that
  // one-shot nudge would otherwise be missed for the webview's lifetime.
  // Re-read everything the ledger renders. Nothing else in the app does this:
  // the mount effect reads once, and the mutation callbacks only re-read what
  // they just changed. So without a caller here, a webview that has been alive
  // since morning renders morning's truth - a tool installed since launch never
  // appears, a config edited by hand still reads Routed, and a dead helper
  // daemon still shows "Routing on".
  //
  // `announce` separates the two callers: the backend's startup nudge is a
  // state *change* worth a banner and an analytics event, whereas reopening the
  // popover is just the user looking, and must stay silent.
  const refreshState = useCallback(async (announce: boolean) => {
    const px = await proxyStatus().catch(() => null);
    const toolList = await listTools().catch(() => []);
    const provs = await listProviders().catch(() => []);
    const stale = await routedClientsStale().catch(() => false);
    // Only agents that predate routing genuinely need a restart; a healthy
    // restored session (agents launched after routing came up) stays
    // quiet. A failed probe defaults to showing.
    const agents = px?.running && announce ? await staleAgentsCount().catch(() => 1) : 0;
    setProxy(px);
    setTools(toolList);
    setProviders(provs);
    if (stale) setStaleAgentsHint(true);
    if (announce && px?.running) {
      if (agents > 0) setChangeNotice("on");
      // The backend only emits this nudge after its startup auto-enable, so
      // routing coming up here is a restored session, not a user toggle.
      track("proxy_enabled", { source: "restored" });
    }
  }, []);

  useEffect(() => {
    let alive = true;
    const unlisten = listen("proxy-state-changed", async () => {
      if (!alive) return;
      await refreshState(true);
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
      // Analytics-only: the provider layer still exists in core/CLI, and
      // provider_count keeps the app_launched dimension comparable across
      // releases even though the UI no longer shows providers.
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
      // Same gate as the proxy-state-changed listener: only agents that
      // predate routing warrant the hint; a failed probe defaults to showing.
      const agents = px?.running ? await staleAgentsCount().catch(() => 1) : 0;
      if (!alive) return;
      setAccount(acct);
      setOAuth(oauthState);
      setProxy(px);
      setTools(toolList);
      setProviders(provs);
      if (stale) setStaleAgentsHint(true);
      if (px?.running && agents > 0) setChangeNotice("on");
      let resolved: Screen;
      if (isSignedIn(acct, oauthState)) {
        resolved = "home";
        // Offer OAuth once to a working key-based account. Gated on being
        // signed in so it never lands over first-run.
        //
        // This used to also test hasSeenTour(), with a comment claiming it
        // spared someone who had just chosen the key on FirstRun. It did not:
        // the tour window opens and completes before FirstRun renders, so the
        // flag is already set by the time they paste a key, and they got the
        // offer on their next launch anyway. FirstRun stamps the seen flag
        // itself now, which is the only place that knows the choice was
        // deliberate.
        if (acct?.auth_mode === "api_key" && acct.has_api_key && !hasSeenOAuthOffer()) {
          setOAuthOffer(true);
        }
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
  // Reopening from the tray is the one moment the user is asking what is true
  // now, so it is the right refresh trigger. Deliberately not a timer: reading
  // status walks every integration's config on disk, and a popover that is
  // closed most of the time has nothing to keep warm.
  useWindowReopen(() => {
    track("popover_opened");
    void refreshState(false);
  });

  // Backend failures buffer Rust-side because they can predate this webview
  // (the startup auto-enable runs before the popover mounts). Sweep the
  // buffer once at mount, then drain again on each nudge. A failure that means
  // routing is down also lands on Home, where blockers render.
  useEffect(() => {
    const sweep = () =>
      void forwardBackendErrors().then((e) => {
        if (e) setProviderError(e);
      });
    sweep();
    const unlisten = listen("backend-error-pending", sweep);
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
    if (oauthOffer) track("oauth_offer_shown");
  }, [oauthOffer]);
  useEffect(() => {
    if (changeNotice) {
      track("routing_notice_shown", { enabled: changeNotice === "on", inline: true });
    }
  }, [changeNotice]);
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
      if (screen === "settings" || screen === "family") {
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
            if (!hasSeenRoutingTakeover()) {
              markRoutingTakeoverSeen();
              setRoutingNotice({ dir: next.running ? "on" : "off", confirming: false });
            } else {
              setChangeNotice(next.running ? "on" : "off");
            }
          }
        } else {
          setChangeNotice(next.running ? "on" : "off");
        }
        // The backend owns the routed set across a master toggle: turning off
        // snapshots what was on and disables all; turning on restores that
        // snapshot. Just reflect the result (the returned ProxyState already
        // carries the restored domains) and refresh the tool ledger.
        setTools(await listTools().catch(() => []));
      } catch (e) {
        trackError(e, "proxy_toggle");
        // Surface why the toggle failed (e.g. on Linux the CA-trust admin step
        // or a missing network service) instead of silently reverting - a
        // swallowed error reads as "the toggle does nothing".
        setProviderError(classifyError(e, "proxy_toggle", account?.auth_mode));
        // Re-sync to the true state after the failed toggle.
        try {
          setProxy(await proxyStatus());
        } catch {
          /* noop */
        }
      } finally {
        setProxyBusy(false);
      }
    },
    [proxy, proxyBusy],
  );

  // Toggle one proxy domain (an "Apps" ledger row). Applied live by the
  // engine - no agent restart involved, so no hint.
  const setDomain = useCallback(
    async (slug: string, enabled: boolean) => {
      if (proxyBusy) return;
      setProxyBusy(true);
      setProviderError(null);
      try {
        setProxy(await proxySetDomain(slug, enabled));
        track("domain_toggled", { domain: slug, enabled });
      } catch (e) {
        trackError(e, "provider_toggle", { domain: slug, enabled });
        // Re-sync the switch to its true state after a failed toggle.
        try {
          setProxy(await proxyStatus());
        } catch {
          /* noop */
        }
        // Rethrow: the group row that asked for this shows the failure next
        // to the member it belongs to, rather than in a screen-level note.
        throw e;
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
      const wasRunning = proxy?.running ?? false;
      setProxyBusy(true);
      try {
        const tool = tools.find((t) => t.slug === slug);
        if (routed) await connectTool(slug, tool?.default_upstream_url ?? "");
        else await disconnectTool(slug);
        track("tool_toggled", { tool: slug, routed });
      } catch (e) {
        trackError(e, "connect", { tool: slug, routed });
        throw e;
      } finally {
        setTools(await listTools().catch(() => tools));
        let running = false;
        try {
          const fresh = await proxyStatus();
          setProxy(fresh);
          running = fresh.running;
        } catch {
          /* non-macOS: no proxy subsystem */
        }
        setChangeNotice(noticeFor(routed, running, wasRunning));
        setProxyBusy(false);
      }
    },
    [proxyBusy, tools, proxy],
  );

  // Route (or unroute) a whole model family from one switch. Runs the same
  // per-member commands the detail screen uses rather than the backend's
  // provider_enable, because the provider catalog only maps Claude Code to
  // Anthropic - OpenCode and friends would be silently left behind.
  //
  // Turning a family on deliberately skips members with a hand-written Gate
  // setup: adopting one replaces someone's config, which stays an explicit
  // act on the tool's own screen. Turning off touches only what is actually
  // routing.
  const setGroupRouted = useCallback(
    async (id: string, on: boolean) => {
      if (proxyBusy) return;
      const group = buildGroups(providers, tools, proxy?.domains ?? [], {
        proxyOn: proxy?.running ?? false,
        caTrusted: proxy?.ca_trusted ?? false,
      }).find((g) => g.id === id);
      if (!group) return;
      const wasRunning = proxy?.running ?? false;
      setProxyBusy(true);
      setProviderError(null);
      // One member failing used to abort the loop and surface "Couldn't
      // connect this tool", naming nobody, reporting no partial success, and
      // pushing the culprit row below the fold. Now every member is attempted
      // and the failures are named.
      const failed: string[] = [];
      let lastError: unknown = null;
      for (const member of group.members) {
        try {
          if (member.kind === "config" && member.tool) {
            if (on && !member.desired && member.attention !== "drifted") {
              await connectTool(member.key, member.tool.default_upstream_url);
            } else if (!on && member.desired) {
              await disconnectTool(member.key);
            }
          } else if (member.domain) {
            if (on && !member.domain.enabled) await proxySetDomain(member.key, true);
            else if (!on && member.domain.enabled) await proxySetDomain(member.key, false);
          }
        } catch (e) {
          failed.push(member.name);
          lastError = e;
          trackError(e, "connect", { provider: id, enabled: on, tool: member.key });
        }
      }
      track("group_toggled", { provider: id, enabled: on });
      if (lastError !== null) {
        const classified = classifyError(lastError, "connect", account?.auth_mode);
        setProviderError({
          ...classified,
          title:
            failed.length === 1
              ? `Couldn’t ${on ? "connect" : "disconnect"} ${failed[0]}`
              : `Couldn’t ${on ? "connect" : "disconnect"} ${failed.length} of ${group.members.length}: ${failed.join(", ")}`,
        });
      }
      setTools(await listTools().catch(() => tools));
      let running = false;
      try {
        const fresh = await proxyStatus();
        setProxy(fresh);
        running = fresh.running;
      } catch {
        /* non-macOS: no proxy subsystem */
      }
      setChangeNotice(noticeFor(on, running, wasRunning));
      setProxyBusy(false);
    },
    [proxyBusy, providers, tools, proxy],
  );

  /** Toggle the shell-environment channel, the master's sub-setting. Its own
   * handler rather than a branch of `toggleProxy`: this one never starts or
   * stops the engine, it only decides whether the proxy is also put in the
   * user's environment - a machine-wide change that reaches git and curl, not
   * just the AI tools. */
  const toggleEnvExport = useCallback(async () => {
    if (proxyBusy) return;
    setProviderError(null);
    setProxyBusy(true);
    try {
      const next = !proxy?.env_export_opted_in;
      setProxy(await proxySetEnvExport(next));
      track(next ? "env_export_enabled" : "env_export_disabled");
    } catch (err) {
      trackError(err, "env_export");
      setProviderError(classifyError(err, "env_export"));
      try {
        setProxy(await proxyStatus());
      } catch {
        /* noop */
      }
    } finally {
      setProxyBusy(false);
    }
  }, [proxyBusy, proxy]);

  const trustCa = useCallback(async () => {
    if (proxyBusy) return;
    setProviderError(null);
    setProxyBusy(true);
    try {
      setProxy(await proxyTrustCa());
      track("ca_trusted");
    } catch (err) {
      // Surface it, don't just log it. This used to call trackError alone, so
      // a cancelled admin prompt - the likeliest failure in the app - produced
      // a screen that appeared to do nothing, while classifyError's carefully
      // worded trust_ca branch went to PostHog and nowhere else. Rethrown so
      // the member-level button in GroupMembers can show it in place.
      trackError(err, "trust_ca");
      setProviderError(classifyError(err, "trust_ca"));
      try {
        setProxy(await proxyStatus());
      } catch {
        /* noop */
      }
      throw err;
    } finally {
      setProxyBusy(false);
    }
  }, [proxyBusy]);

  const untrustCa = useCallback(async () => {
    if (proxyBusy) return;
    setProviderError(null);
    setProxyBusy(true);
    try {
      setProxy(await proxyUntrustCa());
      track("ca_untrusted");
    } catch (err) {
      // Same as trustCa: the classified untrust_ca string reached no screen.
      trackError(err, "untrust_ca");
      setProviderError(classifyError(err, "untrust_ca"));
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

  const gatewayHost = hostOf(account?.gateway_base_url);
  // The header's mono sub-label answers "who am I here?", and the gateway host
  // cannot: it is byte-identical for every customer of a given deployment. The
  // org is what gets billed and what the gateway rejects requests without (see
  // `isSignedIn`), and it used to appear on no screen the user operates from -
  // only in Settings, two navigations away.
  //
  // Home takes the org alone and shows nothing when there isn't one, because
  // Home prints the host on its own line and a header repeating it read as two
  // facts where there was one. Success has no such line, so it falls back.
  const orgName = account?.org_name ?? null;
  const workspace = orgName ?? gatewayHost;
  const proxyOn = proxy?.running ?? false;
  const showProxy = proxy !== null;
  // Proxy domains minus internal plumbing entries.
  const visibleDomains = (proxy?.domains ?? []).filter(
    (d) => !HIDDEN_DOMAIN_SLUGS.has(d.slug),
  );
  // The ledger, grouped by model family; the group screen reads the same
  // shape Home renders so both stay in step after a toggle.
  const groups = buildGroups(providers, tools, visibleDomains, {
    proxyOn,
    caTrusted: proxy?.ca_trusted ?? false,
  });
  // The family whose panel is open, re-resolved from the ledger on every render
  // so a toggle inside the panel repaints it. `undefined` when the family
  // stopped existing while its panel was up - a tool uninstalled between
  // popover opens - and the render below falls through to Home rather than
  // holding a panel about nothing. The old all-families panel handled the same
  // case with a "nothing is installed to route" sentence; a panel titled with a
  // family that no longer exists cannot say that about itself.
  const openGroup = groups.find((g) => g.id === openFamily);

  let body: ReactNode;
  if (screen === "loading") {
    // Startup screen while we resolve account + proxy (and the macOS keychain
    // dialog reads the key) - show the brand lockup instead of a blank popover.
    body = (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 py-16">
        <ConstellationHexMark size={40} />
        <span className="text-gc-title font-semibold tracking-[-0.02em] text-gc-navy">
          Gate <span className="text-gc-accent">Connect</span>
        </span>
      </div>
    );
  } else if (screen === "firstrun") {
    body = (
      <FirstRun
        onConnected={onConnected}
        initialGateway={account?.gateway_base_url}
        startOnKey={startOnKey}
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
        onUseApiKey={() => {
          setStartOnKey(true);
          void signOut();
        }}
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
        textScale={textScale}
        onSetTextScale={setTextScale}
      />
    );
  } else if (screen === "family" && openGroup) {
    body = (
      <FamilyPanel
        group={openGroup}
        busy={proxyBusy}
        onBack={() => setScreen("home")}
        onToggleGroup={(id, on) => void setGroupRouted(id, on)}
        onToggleTool={setToolRouted}
        onSetDomain={setDomain}
        onTrustCa={trustCa}
        proxyOn={proxy?.running ?? false}
        onEnableRouting={() => void toggleProxy(false)}
        authMode={account?.auth_mode}
      />
    );
  } else {
    // home (and any fallback once loaded)
    body = (
      <Home
        workspace={orgName ?? ""}
        gatewayHost={gatewayHost}
        proxyOn={proxyOn}
        // `?? false`, matching the other three call sites. An unresolved
        // proxy state is not evidence that the CA is trusted, and defaulting
        // a security fact to the reassuring answer is the wrong direction
        // even where nothing visible currently depends on it.
        caTrusted={proxy?.ca_trusted ?? false}
        showProxy={showProxy}
        providers={providers}
        tools={tools}
        domains={visibleDomains}
        busy={proxyBusy}
        error={providerError}
        changeNotice={changeNotice}
        onDismissChangeNotice={() => setChangeNotice(null)}
        // User-initiated, so the full takeover is earned here even though
        // startup itself no longer opens it - and since the banner click
        // already declared the intent, land directly on the confirm step.
        // Carries the banner's direction so the takeover doesn't announce
        // "Routing is on" over a switch the user just turned off.
        // "pending" can never reach here: that banner offers Turn on routing,
        // not Close them, because nothing is running to close.
        onCloseAgents={() =>
          setRoutingNotice({
            dir: changeNotice === "off" ? "off" : "on",
            confirming: true,
          })
        }
        onEnableRouting={() => void toggleProxy(false)}
        staleAgentsHint={staleAgentsHint && !staleAgentsDismissed}
        onDismissStaleAgents={() => setStaleAgentsDismissed(true)}
        onToggleProxy={() => toggleProxy(true)}
        onTrustCa={trustCa}
        onOpenFamily={(groupId) => {
          setOpenFamily(groupId);
          setScreen("family");
        }}
        onOpenSettings={() => setScreen("settings")}
        envExportSeparable={proxy?.env_export_separable ?? false}
        envExportOn={proxy?.env_export_opted_in ?? false}
        onToggleEnvExport={() => void toggleEnvExport()}
      />
    );
  }

  // Named per platform, the way `trustStoreName` is elsewhere: the
  // reassurance is worthless if it names the wrong vault.
  const hasCredential = !!account && (account.has_api_key || (oauth?.signed_in ?? false));
  // First run has no credential yet, but it is the screen where the user is
  // about to hand one over, so the promise is worth more here than anywhere
  // else. Stated in the general voice ("credentials live in X") because at
  // this point we do not know whether it will be a session or a key.
  const credentialStore =
    hasCredential || screen === "firstrun" ? secretStoreName(platform) : null;

  // Any full-popover takeover is up, so the room behind it is not the user's
  // to read or click.
  const obscured =
    quitTools !== null || routingNotice !== null || updateTakeoverVisible || oauthOffer;

  // Whether the body has content below the fold, so the scroll region can fade
  // its bottom edge instead of letting the footer's hairline cut a row in half.
  // Recomputed on scroll and whenever the content resizes: a banner mounting, a
  // member row expanding and a screen change all move the fold without any
  // scrolling happening.
  const scrollRef = useRef<HTMLDivElement>(null);
  const [moreBelow, setMoreBelow] = useState(false);
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const sync = () => setMoreBelow(el.scrollHeight - el.scrollTop - el.clientHeight > 1);
    sync();
    el.addEventListener("scroll", sync, { passive: true });
    const ro = new ResizeObserver(sync);
    ro.observe(el);
    // The keyed child is what actually grows; the container's own box only
    // changes when the window does.
    if (el.firstElementChild) ro.observe(el.firstElementChild);
    return () => {
      el.removeEventListener("scroll", sync);
      ro.disconnect();
    };
  }, [screen]);

  return (
    <div
      className={`relative flex h-full w-full flex-col overflow-hidden bg-gc-surface text-gc-ink${
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
        suppressTakeover={quitTools !== null || routingNotice !== null || oauthOffer}
        onTakeoverVisibleChange={setUpdateTakeoverVisible}
      />
      {routingNotice !== null && screen !== "loading" && (
        <RoutingChangeNotice
          routingOn={routingNotice.dir === "on"}
          startConfirming={routingNotice.confirming}
          onDismiss={() => setRoutingNotice(null)}
          onAgentsClosed={() => setChangeNotice(null)}
        />
      )}
      {quitTools !== null && (
        <QuitConfirm tools={quitTools} onCancel={() => setQuitTools(null)} />
      )}
      {/* Lowest-priority takeover: anything the user just did, or a pending
          update, outranks an offer they did not ask for. Dismissing marks it
          seen whichever way they leave, so it never returns. */}
      {oauthOffer &&
        screen === "home" &&
        quitTools === null &&
        routingNotice === null &&
        !updateTakeoverVisible && (
          <OAuthOffer
            onUpgrade={upgradeToOAuth}
            onDismiss={() => {
              markOAuthOfferSeen();
              setOAuthOffer(false);
            }}
          />
        )}
      {/* While any takeover is up the obscured content goes aria-hidden, so a
          screen reader's virtual cursor can't wander under the dialog, and
          pointer-events-none, so the mouse can't either. aria-hidden alone
          left the gear and the row switches clickable underneath the panel:
          the focus traps cover Tab, but a click still reached a control the
          user could not see. (`inert` would do both in one attribute; React
          18's DOM typings don't carry it yet.) */}
      <div
        className={`flex min-h-0 grow flex-col${obscured ? " pointer-events-none" : ""}`}
        aria-hidden={obscured ? true : undefined}
      >
        {/* Only the body scrolls. The version line used to live inside the
            scroll container, so it drifted with the content - PRODUCT.md says
            header and footer never scroll, and on Home it meant the footer
            and the dashboard link both sat below an invisible fold. */}
        <div
          ref={scrollRef}
          className={`gc-scroll min-h-0 grow overflow-y-auto overflow-x-hidden${
            moreBelow ? " gc-scroll-more" : ""
          }`}
        >
          {/* Keyed by screen so the slide replays on every navigation and not
              on ordinary re-renders. The reduced-motion guard in index.css
              collapses both classes to instant. */}
          <div key={screen} className={screenMotion}>
            {body}
          </div>
        </div>
        {/* PRODUCT.md's first principle is that every screen should make the
            user feel where the key lives. Home never said "keychain" once -
            the claim lived in the tour, one line of Settings, and two taps
            into a group detail, i.e. everywhere except the screen people
            actually look at. Now it is pinned, so it is on every screen and
            costs the scroll budget nothing. */}
        {/* `flex-wrap` on the strip: at 200% the promise and the version cannot
            share one line, and truncating the product's core reassurance to
            "Session in your ke…" is the loss of content SC 1.4.4 forbids.
            Wrapped, the version takes its own line and the sentence keeps all of
            itself. */}
        {credentialStore && (
          <div className="flex shrink-0 flex-wrap items-center gap-x-2 gap-y-0.5 border-t border-gc-line px-3.5 py-2">
            <span className="flex min-w-0 flex-1 basis-[12em] items-center gap-1.5 text-gc-label text-gc-ink-3">
              <Icon name="key" size={11} className="shrink-0" />
              {/* This slot has to hold "Session in Credential Manager" (146px)
                  and, during the first async tick before the platform
                  resolves, "Key in your system’s secure store" (159px). With
                  the dashboard button also on the strip it was 136-142px, so
                  the product's core promise truncated on Windows and briefly
                  everywhere. The button moved into Home's ledger heading; the
                  strip is back to a label and a version, which is what it was
                  measured for.

                  Wraps rather than truncates. `flex-wrap` on the strip gave
                  this line the full width, but `truncate` then cut the sentence
                  anyway: at 200% it needs 360px and the widest a 380px window
                  can offer it is 324px, so it rendered "Session in your
                  system's secu…". That is loss of content under SC 1.4.4, on
                  the one sentence carrying PRODUCT.md's first principle - where
                  the key lives - at exactly the text size a low-vision user
                  asked for. Two lines in a fixed footer costs 14px once; the
                  truncation cost the promise. Nothing changes at 100%, where it
                  has always fit on one line. */}
              <span>
                {!hasCredential
                  ? `Credentials live in ${credentialStore}`
                  : `${account?.auth_mode === "oauth" ? "Session" : "Key"} in ${credentialStore}`}
              </span>
            </span>
            {/* Suppressed before there is an account: on first run the strip
                carries the promise and nothing else. */}
            {hasCredential && version && (
              <span className="ml-auto shrink-0 font-mono text-gc-label text-gc-ink-3">
                v{version}
              </span>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
