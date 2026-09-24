import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type {
  Account,
  ProxyState,
  Tool,
  Verdict,
} from "./lib/api";
import {
  getAccount,
  getAccountKeyPrefix,
  listTools,
  proxyStatus,
  routingStartupPending,
  pinPopover,
  requestQuit,
  requestSwitchOrg,
  revealMainWindow,
  unpinPopover,
  routingVerdicts,
} from "./lib/api";
import { useRouting } from "./lib/useRouting";
import { useRunningApps } from "./lib/useRunningApps";
import { allSettled, allVerified, REOPEN_IDLE_WATCH_MS } from "./lib/reopen";
import { classifyError } from "./lib/errors";
import { forwardBackendErrors } from "./lib/backendErrors";
import type { ClassifiedError, ErrorContext } from "./lib/errors";
import {
  BAND_LABELS,
  buildGroups,
  hintForMember,
  isSettingsManaged,
  sectionHint,
  sectionMemberKeys,
} from "./lib/groups";
import type { Band, Group } from "./lib/groups";
import { useSectionRouting } from "./lib/useSectionRouting";
import { sectionStatus, verdictStatus, verdictsBySlug } from "./lib/verdict";
import { openExternal } from "./lib/openExternal";
import { GATE_DOCS_URL } from "./lib/config";
import { NO_DASHBOARD, dashboardLinks } from "./lib/dashboard";
import { trustPromptHint, usePlatform } from "./lib/platform";
import { useSecurityFeed } from "./lib/securityFeed";
import { useInstallations } from "./lib/activity";
import { useToolMessages } from "./lib/toolMessages";
import { orgLabel } from "./lib/orgLabel";
import type { ToolMessagesView } from "./lib/toolMessages";
import { Tray } from "./components/gc/Tray";
import type { MenuAction } from "./components/gc/OverflowMenu";
import type { SidebarApp, SidebarGroup } from "./components/gc/Sidebar";
import { brandMarkFor, brandMarkForSection } from "./components/gc/BrandMark";
import { ErrorBanner } from "./components/gc/banners";
import { Modal } from "./components/gc/Modal";
import {
  reopenSubjects,
  ApplyChangesDialog,
  ChangeReadyDialog,
  CloseAppsDialog,
  OpenCodeEnvDialog,
  ReviewConfigDialog,
} from "./components/gc/dialogs";

/** A whole reading, compared by value: every read builds fresh objects. */
const detectionSignature = (reading: unknown): string => JSON.stringify(reading);

/**
 * One row's messages figure, or nothing.
 *
 * Three states and no fourth. A reading - including a measured zero, which the
 * row says in words - carries the age the gateway computed it at, so a held
 * number can disclose that it is held. A first read still in flight holds a
 * place. Anything else draws nothing at all: no account, an unattributed machine,
 * a gateway that refused, or a row whose traffic the gateway cannot attribute.
 * A `0` for any of those would be a claim about this person's traffic that
 * nothing measured.
 */
function messageFigure(
  byTool: ToolMessagesView["byTool"],
  pending: ToolMessagesView["pending"],
  slug: string,
): SidebarApp["messages"] {
  const held = byTool.get(slug);
  if (held) {
    return { kind: "count", count: held.messages, measuredAt: held.measuredAt };
  }
  return pending.has(slug) ? { kind: "pending" } : undefined;
}

/**
 * The tray popover's shell (window label `tray`): the quick-status surface the
 * tray icon toggles, drawn by `components/gc/Tray`. It owns the same slice of
 * state the window shell reads - tools, providers, proxy, verdicts, account -
 * and dispatches through the same `useRouting`, so a switch here and the same
 * switch in the window cannot disagree about what a toggle does.
 *
 * What it deliberately does not own: setup (a signed-out tray hands over to
 * the main window, where the panes live), the quit dialog (`request_quit`
 * reveals the main window and defers there, same as the tray menu's Quit),
 * and the per-app panes (nothing here navigates).
 */
export function TrayApp() {
  const [tools, setTools] = useState<Tool[]>([]);
  const [proxy, setProxy] = useState<ProxyState | null>(null);
  /** Whether the startup auto-enable is still in flight, which is the only
   * thing separating "starting" from "didn't start" on the routing card. Read
   * beside `proxy` on the first load and on every `proxy-state-changed`, which
   * the backend emits when the enable settles either way. A failed read is
   * "not starting": that prints "Didn't start" over a slow enable, which is
   * what the card said before this existed, rather than a "Starting" that
   * nothing would ever clear. */
  const [starting, setStarting] = useState(false);
  const [verdicts, setVerdicts] = useState<Map<string, Verdict>>(new Map());
  const [account, setAccount] = useState<Account | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  /** The account read FAILED, which is not the same as having no account.
   *  Collapsing the two drew "Sign in to get started" at a signed-in user
   *  whose keychain prompt was dismissed - the exact scenario CLAUDE.md
   *  documents - and that sentence is false. Principle 6, one level up from
   *  a figure. */
  const [accountUnread, setAccountUnread] = useState(false);
  /** The account's key prefix, which is what makes a replaced api key a different
   *  credential. Read back after every account read for the reason
   *  `activity_cache.rs` records: in api-key mode the org is whatever the gateway
   *  resolves the *key* to, so without this a key swap to another org on the same
   *  gateway leaves every scope string byte-identical and the previous org's
   *  figures stay on screen under the new org's name. */
  const [keyPrefix, setKeyPrefix] = useState<string | null>(null);
  const [actionError, setActionError] = useState<ClassifiedError | null>(null);
  /**
   * The buffered routing-down failure, held apart from `actionError`.
   *
   * Two different facts, so two different pieces of state. `actionError` is an
   * *event*: the user pressed something and it failed, one second ago, and it
   * stands until they dismiss it. This one is a *claim about routing state*
   * drained out of the Rust buffer - `restore_routing`, `provider_restore`,
   * `provider_reconcile`, all three of which describe how things ARE rather
   * than something that just happened.
   *
   * Keeping them in one slot is what made the tray latch. The popover's webview
   * is created hidden at launch and never destroyed, so it drains the startup
   * auto-enable's failures before the user has opened anything, and nothing
   * cleared them: a failure raised hours earlier, already reported and
   * dismissed in the main window, sat in a banner over the first popover the
   * user happened to open. A state claim has to be allowed to stop being true,
   * which an action's error does not - so this one clears itself and that one
   * does not.
   */
  const [routingError, setRoutingError] = useState<ClassifiedError | null>(null);
  /** One banner, two sources. The action the user just took outranks a standing
   *  claim about routing: it is newer, and it is the one they are waiting on. */
  const shownError = actionError ?? routingError;
  /**
   * What a page open across a section's flip cannot know yet, composed by
   * `lib/useSectionRouting.ts`.
   *
   * The tray draws the same section switches as the window and cascades through
   * the same hook, so a flip here routes claude.ai exactly as a flip there does.
   * A surface that acts and says nothing is the gap this whole change is about,
   * and it would be a strange fix that closed it in one shell only.
   */
   const platform = usePlatform();

  /** What the last read put on screen, so an unchanged reading is dropped
   * rather than re-rendering the whole popover for it. */
  const rendered = useRef({ tools: "", proxy: "" });
  const rereading = useRef(false);

  /** The popover's root, focused on every reveal. See the `visibilitychange`
   *  handler for why the shell owns this rather than `Tray`. */
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    rendered.current = {
      tools: detectionSignature(tools),
      proxy: detectionSignature(proxy),
    };
  }, [tools, proxy]);

  /** Whether the last sweep landed. A failed one used to be indistinguishable
   *  from a quiet machine: the map stayed empty, every row rendered
   *  `Not protected - Checking`, the routing card read `0 of N`, and the polled
   *  path only re-swept when the tools/proxy signature CHANGED - so on an idle
   *  machine one failure at first load persisted until something moved. This
   *  surface has no refresh affordance, so nothing the user could do fixed it. */
  const verdictsRead = useRef(false);

  const refreshVerdicts = useCallback(async () => {
    const v = await routingVerdicts().catch(() => null);
    verdictsRead.current = v !== null;
    if (v) setVerdicts(verdictsBySlug(v));
    // A fresh sweep that reaches the engine and finds nothing complaining about
    // the connection is evidence against a buffered "routing is down", so it
    // retires it. This is the half of the latch that matters: clearing on hide
    // alone would still show a stale failure once, on the first open, which is
    // the moment it is least explicable. `connection_problem` is the one
    // `VerdictReason` that means what the buffered contexts mean; the others
    // (drift, reopen, access) are per-tool and do not contradict the claim.
    //
    // Deliberately not touching `actionError`: a probe cannot tell the user
    // their own failed button did not happen.
    if (v && !v.some((verdict) => verdict.reason === "connection_problem")) {
      setRoutingError(null);
    }
  }, []);

  /**
   * Is any tool waiting to be reopened?
   *
   * A boolean rather than the map, so the effect below arms and disarms on the
   * fact changing and not on every sweep replacing the map that told it.
   */
  const reopenWaiting = useMemo(
    () => [...verdicts.values()].some((v) => v.reason === "reopen_required"),
    [verdicts],
  );

  /**
   * Keep sweeping while something is waiting to be reopened.
   *
   * The window shell's effect, for the window shell's reason: a tool picks up
   * its new route when the person opens a terminal, and no event says so -
   * `tool_watch.rs` watches config files and binaries, never the process table.
   * Both shells draw "Reopen to finish" on the row, so both have to notice, and on the same
   * cadence: two numbers here is how the tray comes to clear a status the window
   * is still showing.
   *
   * Armed on the condition rather than always, because the sweep probes the
   * relay and the account's session - and this surface is behind a tray icon
   * somebody opens and closes all day.
   */
  useEffect(() => {
    if (!reopenWaiting) return;
    const id = setInterval(() => void refreshVerdicts(), REOPEN_IDLE_WATCH_MS);
    return () => clearInterval(id);
  }, [reopenWaiting, refreshVerdicts]);

  /** The same flag where the reveal handler can read it.
   *
   * That effect is keyed on `redetect` alone and registers a Tauri listener, so
   * adding this to its dependencies would tear the listener down and build it
   * again every time a tool starts or stops waiting. Reading it through the
   * render instead is how the reveal sweep would have been frozen at whatever
   * the flag was on first paint - which is `false`, so it would never have
   * fired at all. */
  const reopenWaitingRef = useRef(reopenWaiting);
  reopenWaitingRef.current = reopenWaiting;

  /** Event-driven re-read: a write landed or the engine changed state, so
   * commit whatever comes back and re-sweep the verdicts. */
  const refresh = useCallback(async () => {
    const [t, px, pending] = await Promise.all([
      listTools().catch(() => null),
      proxyStatus().catch(() => null),
      routingStartupPending().catch(() => false),
    ]);
    if (t) setTools(t);
    if (px) setProxy(px);
    setStarting(pending);
    void refreshVerdicts();
  }, [refreshVerdicts]);

  /** The `tools-changed` variant: drop a re-read that is already in flight
   * rather than stack on it, and drop an unchanged reading rather than commit
   * it. */
  const redetect = useCallback(async () => {
    if (rereading.current) return;
    rereading.current = true;
    try {
      const [t, px] = await Promise.all([
        listTools().catch(() => null),
        proxyStatus().catch(() => null),
      ]);
      let changed = false;
      if (t && detectionSignature(t) !== rendered.current.tools) {
        setTools(t);
        changed = true;
      }
      if (px && detectionSignature(px) !== rendered.current.proxy) {
        setProxy(px);
        changed = true;
      }
      // Or when the last sweep never landed: an unchanged reading is a reason
      // to skip re-reading the tools, not a reason to leave every row saying
      // "Checking" forever.
      if (changed || !verdictsRead.current) void refreshVerdicts();
    } finally {
      rereading.current = false;
    }
  }, [refreshVerdicts]);

  useEffect(() => {
    void (async () => {
      // No `list_providers`: the tray draws sections from the ledger, which
      // `buildGroups` builds from tools and domains alone. The family catalog
      // reached this shell only through a dependency array.
      const [t, px, pending, acct, prefix] = await Promise.all([
        listTools().catch(() => null),
        proxyStatus().catch(() => null),
        routingStartupPending().catch(() => false),
        // The wrapper distinguishes "the read failed" from "there is no
        // account": both arrive as null otherwise, and only the first should
        // suppress the signed-out state.
        getAccount()
          .then((a) => ({ read: true, account: a }))
          .catch(() => ({ read: false, account: null as Account | null })),
        getAccountKeyPrefix().catch(() => null),
      ]);
      setTools(t ?? []);
      setProxy(px);
      setStarting(pending);
      setAccount(acct.account);
      setAccountUnread(!acct.read);
      setKeyPrefix(prefix);
      void refreshVerdicts();
      setLoaded(true);
    })();
  }, [refreshVerdicts]);

  /**
   * Drain the backend's buffered failures, exactly as the window shell does.
   *
   * The tray had no drain, which is the third time this gap has been shipped -
   * `backendErrors.ts` was lifted out of `App.tsx` precisely "so both shells
   * share one copy", and then only one shell called it. It bit hardest on
   * "Resume now": `resume_restore` swallows the restore's error on purpose
   * (`lib.rs`, "Best-effort, like every other caller of this") and still returns
   * `pending_restore()` as `Ok`, so the frontend `catch` never fires. A resume
   * that failed for the same reason it failed the first time therefore redrew
   * an identical card and said nothing - indistinguishable from a dead button,
   * which is how it was reported. `provider_restore` is already in
   * `ROUTING_DOWN_CONTEXTS`, so the failure was reaching the buffer all along
   * and only ever needed reading here.
   */
  useEffect(() => {
    const sweep = () =>
      // Display only. The window shell forwards the batch to analytics; both
      // shells are handed their own copy of every failure so they cannot race
      // over one take, and reporting from both would double every event with
      // one of the two coming from a webview where nothing was shown.
      void forwardBackendErrors({ reportToAnalytics: false }).then((e) => {
        // `forwardBackendErrors` only ever returns a routing-down context, so
        // everything it hands back belongs in the state-claim slot.
        if (e) setRoutingError(e);
      });
    sweep();
    const unlisten = listen("backend-error-pending", sweep);
    return () => {
      void unlisten.then((f) => f()).catch(() => {});
    };
  }, []);

  // Told rather than polled, same as the window shell: the backend watches the
  // tool config files and emits `tools-changed` (`core/src/tool_watch.rs`). This
  // mattered more here than there - a popover the tray icon opens and closes all
  // day was running a config-file walk every five seconds behind it.
  //
  // The visibility read stays and is not a poll: reopening reads immediately,
  // which also covers the installs no watch can see (a launcher on `$PATH`).
  useEffect(() => {
    const unlisten = listen("tools-changed", () => {
      void redetect();
    });
    const onVisible = () => {
      if (document.hidden) {
        // Hidden, not destroyed - so the menu would still be open over the
        // list on the next reveal, with rows clickable beside it.
        setMenuOpen(false);
        // Same reasoning, one surface up: the routing banner has had its
        // showing. It is a state claim, and the next reveal re-reads the state
        // it was claiming, so carrying it across is how a failure the user
        // already saw comes back undated over an unrelated visit.
        setRoutingError(null);
        return;
      }
      void redetect();
      // A reveal is the likeliest moment for a reopen to have happened: the
      // popover is what someone comes back to after opening their terminal.
      // `redetect` above sweeps only when the tool inventory moved, and a
      // reopened CLI does not move it. Gated for the reason the interval is.
      if (reopenWaitingRef.current) void refreshVerdicts();
      // Put focus in the popover, for the same reason: the window is shown
      // rather than created, so nothing moves focus on its own. Without this a
      // keyboard user opened the popover and their focus was still in whatever
      // they had been doing, so Tab walked that window's controls and the tray
      // could be read by assistive technology but not operated.
      //
      // The root, not the first control: landing on "Expand app" would announce
      // a button with no idea what surface it belongs to, and would make Expand
      // one Return press away from a window swap nobody asked for.
      root.current?.focus();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      void unlisten.then((off) => off()).catch(() => {});
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [redetect]);

  /**
   * Escape dismisses the popover, and closes the menu first if one is open.
   *
   * The platform convention for a surface anchored to a tray icon, and the other
   * half of the click-outside dismissal: a keyboard user could open this window
   * and had no way to close it, because the only exits were the tray icon and a
   * click elsewhere.
   *
   * **Bubble phase, deliberately.** `useFocusTrap` handles Escape in the capture
   * phase and calls `stopPropagation`, so a dialog over the popover consumes the
   * key before it reaches here - which is what should happen. Escape closes the
   * config-overwrite confirmation, not the window the user was answering it in.
   * Registering this in capture would race that trap and sometimes hide the
   * window out from under an unanswered question.
   */
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (menuOpen) {
        // The menu's own handler covers Escape while focus is inside it; this is
        // the case where focus has since moved elsewhere in the popover.
        setMenuOpen(false);
        return;
      }
      try {
        void getCurrentWindow().hide();
      } catch {
        /* plain-browser dev */
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [menuOpen]);

  /**
   * The session changed in the other window: an org switch, a replaced key, a
   * gateway switch, a sign-out, a disconnect.
   *
   * This popover renders one account's everything - the org name in the footer,
   * the per-row figures - and every one of those is
   * keyed on `credential`, which is derived from state read here. Without this
   * the tray kept the previous org's numbers until something unrelated woke it,
   * and after a sign-out it kept figures for an account that could no longer read
   * them. `signal_session_changed` in the Rust shell is the other half.
   *
   * Only the account is re-read: the tools, the proxy and the verdicts are not
   * the session's, and `proxy-state-changed` and `tools-changed` already cover
   * them.
   */
  useEffect(() => {
    const unlisten = listen("session-changed", () => {
      void getAccount()
        .then(setAccount)
        .catch(() => {});
      void getAccountKeyPrefix()
        .then(setKeyPrefix)
        .catch(() => {});
    });
    return () => {
      void unlisten.then((off) => off()).catch(() => {});
    };
  }, []);

  // The engine changes state without us asking - a CLI toggle, the other
  // window, the startup auto-enable - and the account changes under an org
  // switch made in the main window. Repaint from the event.
  useEffect(() => {
    const unlisten = listen("proxy-state-changed", () => {
      void refresh();
      void getAccountKeyPrefix()
        .then(setKeyPrefix)
        .catch(() => {});
      void getAccount()
        .then((a) => {
          setAccount(a);
          setAccountUnread(false);
        })
        .catch(() => setAccountUnread(true));
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [refresh]);

  const routing = useRouting({
    tools,
    proxy,
    onSnapshot: ({ tools: t, proxy: px }) => {
      setTools(t);
      setProxy(px);
      void refreshVerdicts();
    },
    onError: (e, context) => {
      const engineContexts: ErrorContext[] = ["proxy_toggle", "env_export", "untrust_ca"];
      const ctx = engineContexts.find((c) => c === context) ?? "connect";
      setActionError(classifyError(e, ctx));
    },
  });
  const routingBusy = routing.busy;

  const runningApps = useRunningApps({
    onError: (e) => setActionError(classifyError(e, "close_agents")),
    // Nothing running for a tool the sweep named means the sweep's reading is out
    // of date, not that the click had nothing to do. Same handling as the
    // window shell.
    onNothingRunning: () => void refreshVerdicts(),
    nameFor: (slug) => tools.find((t) => t.slug === slug)?.product_name,
  });

  /**
   * The reopen flow draws a dialog for every stage but one: `work` shows only
   * "Change is ready", once every tool verifies. Until then the stage runs with
   * nothing on screen, so the rail carries it, and a CLI waiting for its user to
   * reopen it can stay there indefinitely.
   */
  const reopenDialogShown =
    runningApps.stage !== null &&
    (runningApps.stage.kind !== "work" || allVerified(runningApps.stage.tools));
  /** Every tool is done and they did not all verify: nothing will be drawn, so
   * end the flow. */
  const reopenOutcomeUndrawn =
    runningApps.stage?.kind === "work" &&
    allSettled(runningApps.stage.tools) &&
    !allVerified(runningApps.stage.tools);
  const { dismiss: dismissRunningApps } = runningApps;
  useEffect(() => {
    if (reopenOutcomeUndrawn) dismissRunningApps();
  }, [reopenOutcomeUndrawn, dismissRunningApps]);

  /** Same follow-up as the window shell: a config write that actually landed
   * offers to close the app still running on its old route. */
  const routeApp = useCallback(
    async (slug: string, next: boolean) => {
      setActionError(null);
      if (await routing.setAppRouted(slug, next)) {
        await runningApps.offerAfterChange([slug]);
      }
    },
    [routing, runningApps],
  );

  const groups = useMemo<Group[]>(
    () =>
      proxy
        ? // Same filter the window applies: the channel has a status card of
          // its own above this list, so a row for it here would be the second
          // control AG-893 is about - and the tray offers no control for it.
          buildGroups(tools.filter((t) => !isSettingsManaged(t.slug)), proxy.domains, {
            proxyOn: proxy.running,
            caTrusted: proxy.ca_trusted,
            // The sweep, which a section's rendered state cannot do without.
            // A member's `routed` is what `sectionStatus` counts to decide
            // whether the app is protected, and without this every config
            // member falls back to the conservative "not verified" - so a
            // connected, swept tool read "Not routed - Blocked" for as long as
            // its row was a section. The popover passed it from the start
            // (`App.tsx`); this shell never needed it until a row stopped being
            // a tool.
            verdicts,
          })
        : [],
    // Not `providers`: `buildGroups` stopped taking them, and leaving the poll's
    // result in here rebuilt the whole ledger every time it landed.
    [tools, proxy, verdicts],
  );

  /** Whose readings these are. Identical in shape to `NewUiApp`'s, key prefix
   *  included: anything keyed on this must drop when the credential changes, and
   *  a replaced api key is a changed credential even when every other field is
   *  the same. */
  const credential = account
    ? `${account.auth_mode}|${account.gateway_base_url}|${account.org_id ?? ""}|${keyPrefix ?? ""}`
    : "";

  // The live security-event feed (AG-578). Keyed on the credential so a switch
  // does not leave the previous org's alert counts on screen, matching the window shell.
  const securityFeed = useSecurityFeed(account !== null, credential);

  /**
   * This machine, as the gateway names it.
   *
   * Read here for the same reason the window reads it: a null `installId` means
   * *org-wide*, not "this machine", so a figure fetched without one would put the
   * whole org's traffic on this machine's rows. `resolved` is what separates "not
   * asked yet" from "asked, and this machine is unattributed" - only the first is
   * worth waiting for.
   */
  const installs = useInstallations(account !== null, credential);
  const machineKnown = installs.resolved && installs.current !== null;

  /**
   * The messages figure per row, off the held readings and refreshed on each look.
   *
   * The rows are the config tools only. A chat domain's traffic arrives at the
   * gateway unattributed on purpose, so there is no per-tool reading to ask for -
   * the same reason its alert count is absent rather than zero.
   */
  const messageSlugs = useMemo(
    () => tools.filter((t) => t.status.kind !== "not_installed").map((t) => t.slug),
    [tools],
  );
  // Destructured, not held as the view object. The hook returns a fresh literal
  // every render, so depending on it made `apps` - and `trayGroups` below it -
  // recompute on every render. `alertCounts` above depends on `securityFeed`'s
  // fields for exactly this reason.
  const {
    byTool: messagesByTool,
    pending: messagesPending,
    orgName: readingOrgName,
  } = useToolMessages(
    account !== null && machineKnown,
    messageSlugs,
    installs.current,
    credential,
  );

  /**
   * Blocked and flagged requests per app, counted off the feed's own buffer -
   * the alert half of the drawn activity line.
   *
   * Keyed on the event's `tool`, the only attribution the feed carries, so an
   * unattributed event is counted against nobody rather than against a guessed
   * slug. The chat domains are therefore permanently without one, which is why
   * `alerts` is optional rather than defaulted: their rows keep the two-line
   * shape instead of claiming a quiet day over traffic Gate cannot see.
   *
   * Null is "no reading", and a row draws nothing for it. Four ways to get
   * there and they are one thing to the reader: the feed could not be read, it
   * has not answered yet, it is offline or reconnecting, or there is no account for it to run under - and in
   * that last case `loading` never clears, because the hook's seed returns
   * early, which is why the account is checked here too.
   *
   * The buffer is what this popover has seen (200 events), the same depth the
   * window's Security events section lists.
   */
  const alertCounts = useMemo<Map<string, number> | null>(() => {
    // Only a live feed is a reading. `offline` and `reconnecting` both leave
    // `unavailable` false, and counting their buffer would put a zero on every
    // row that nobody measured - the one card that said the feed was down is
    // gone from this surface, so the rows must not claim otherwise.
    if (
      account === null ||
      securityFeed.loading ||
      securityFeed.unavailable ||
      securityFeed.state !== "live"
    ) {
      return null;
    }
    const counts = new Map<string, number>();
    for (const e of securityFeed.events) {
      if (e.tool) counts.set(e.tool, (counts.get(e.tool) ?? 0) + 1);
    }
    return counts;
  }, [
    account,
    securityFeed.events,
    securityFeed.loading,
    securityFeed.unavailable,
    securityFeed.state,
  ]);

  /** A feed that is actually running has not answered yet, which is the one case
   *  worth holding a place for. */
  const alertsPending =
    account !== null && securityFeed.loading && !securityFeed.unavailable;

  const apps = useMemo<SidebarApp[]>(
    () =>
      tools
        .filter((t) => t.status.kind !== "not_installed" && !isSettingsManaged(t.slug))
        .map((t) => ({
          slug: t.slug,
          name: t.name,
          status: verdictStatus(verdicts.get(t.slug), {
            writeFailed: routing.writeFailures.has(t.slug),
            // Same reading the window's rail takes, so the two surfaces
            // cannot disagree about whether a row is inspected. AG-932.
            coverage: t.coverage,
          }),
          on:
            t.status.kind === "connected" ||
            t.status.kind === "drifted" ||
            t.status.kind === "overridden",
          logo: brandMarkFor(t.slug),
          busy: routingBusy,
          hint: hintForMember(t.slug),
          // A held figure outranks the pending state, so a look that re-reads
          // keeps the last number on the row instead of blanking it for the
          // length of a fetch. The skeleton is the first read only.
          messages: messageFigure(messagesByTool, messagesPending, t.slug),
          alerts: alertCounts
            ? { kind: "count", count: alertCounts.get(t.slug) ?? 0 }
            : alertsPending
              ? { kind: "pending" }
              : undefined,
        })),
    [
      tools,
      verdicts,
      routing.writeFailures,
      routingBusy,
      alertCounts,
      alertsPending,
      messagesByTool,
      messagesPending,
    ],
  );

  // The rail's grouping, verbatim from `NewUiApp.sidebarGroups`: one row per
  // section under a band eyebrow, with the section's status and its switch.
  // Less the rail's "Not installed" group, which the tray leaves out on
  // purpose - see `Tray`'s deviations.
  //
  // Matching matters more than it looks. A tray drawing per-surface rows would
  // put a switch on the session surfaces, which the window's app switch now
  // owns and gates behind a confirmation - so the tray would be the way to
  // route someone's signed-in session without ever being asked.
  const trayGroups = useMemo<SidebarGroup[]>(() => {
    if (groups.length === 0) {
      return apps.length > 0 ? [{ id: "all", label: "", apps }] : [];
    }
    const bySlug = new Map(apps.map((a) => [a.slug, a]));
    const grouped: SidebarGroup[] = [];
    let band: Band | null = null;
    for (const g of groups) {
      const status = sectionStatus(g, bySlug);
      if (!status) continue;
      // The section's config tool carries the figures: the gateway attributes
      // per tool, and a host surface has nothing of its own to report.
      const tool = g.members.find((m) => m.kind === "config");
      const figures = tool ? bySlug.get(tool.key) : undefined;
      for (const m of g.members) bySlug.delete(m.key);
      if (g.band !== band) {
        band = g.band;
        grouped.push({ id: `band:${band}`, label: BAND_LABELS[band], apps: [] });
      }
      grouped[grouped.length - 1].apps.push({
        slug: g.id,
        name: g.name,
        status,
        on: g.switchOn,
        logo: brandMarkForSection(g.id, sectionMemberKeys(g.id)),
        busy: routingBusy,
        hint: sectionHint(g),
        messages: figures?.messages,
        alerts: figures?.alerts,
      });
    }
    if (bySlug.size > 0) {
      grouped.push({ id: "unclaimed", label: "", apps: [...bySlug.values()] });
    }
    return grouped.filter((g) => g.apps.length > 0);
  }, [groups, apps, routingBusy]);

  /**
   * The app switch, the same one the rail draws.
   *
   * `lib/useSectionRouting.ts` owns the consent gate, the single certificate
   * gate and the cascade. The tray had its own copy of all three for about a
   * week and they had already drifted; the shells share `useRouting` and
   * `lib/groups.ts` for the same reason.
   */
  const section = useSectionRouting({
    groups,
    routing,
    runningApps,
    onBeforeRoute: () => {
      setActionError(null);
      // See the window: a routing click retires the previous click's advice,
      // which is what keeps the banner from outliving the claim it makes.
    },
    routeApp: (slug, next) => void routeApp(slug, next),
  });
  const toggleApp = section.toggle;

  /** Reveal the full window and step aside. `getCurrentWindow` throws outside
   * Tauri (plain-browser dev), where there is nothing to hide anyway. */
  const expand = useCallback(() => {
    void revealMainWindow().catch(() => {});
    try {
      void getCurrentWindow().hide();
    } catch {
      /* plain-browser dev */
    }
  }, []);

  /** The dashboard for the gateway this install talks to, or null when it has
   *  none. Same derivation as the window (`lib/dashboard.ts`): a constant here
   *  sent a staging install to the production dashboard.
   *
   *  Memoised at component level rather than computed inside `onMenuSelect`,
   *  because that callback holds no dependencies and would have closed over the
   *  `null` account from first render - which reads on screen as "this gateway
   *  has no dashboard" on every single click. */
  const dash = useMemo(
    () => dashboardLinks(account?.gateway_base_url),
    [account?.gateway_base_url],
  );

  /**
   * Hold the popover open across anything the user must not lose by looking
   * away, and let it dismiss on blur the rest of the time.
   *
   * The window dismisses itself when it loses focus (`Focused(false)` in
   * `lib.rs`), which is the convention for a tray-anchored surface. Three things
   * blur it without the user having chosen to leave:
   *
   * - **The first load.** Rust pins the popover before revealing it at startup,
   *   precisely because reading the OS credential store can raise a keychain or
   *   keyring unlock dialog, and it expects the frontend to release that pin
   *   once the read is done. Only `src/App.tsx` ever did, so this window would
   *   have stayed pinned for its whole life - blur-dismiss silently dead on the
   *   popover the user is most likely to have open.
   * - **A system dialog this surface raised.** Accepting the certificate prompt
   *   hands focus to the OS trust dialog, and dismissing then takes away the
   *   copy that says which button to press. `routingBusy` covers that interval,
   *   which is why it is in here rather than just the prompt.
   * - **An in-app decision in progress.** Losing a config-overwrite
   *   confirmation because another window was clicked would be the one dialog
   *   on this surface where the user approves a destructive write.
   */
  const popoverHeld =
    !loaded ||
    routing.prompt !== null ||
    // The third bullet above, reached by a different route: this dialog is
    // raised before any routing call, so `routing.prompt` is still null while
    // it is on screen. Without it a click anywhere else blur-dismisses the
    // popover mid-question, and the answer the next click gives is to a
    // question nobody is being shown. (The session-consent dialog was the
    // other such question and is gone - AG-934.)
    reopenDialogShown ||
    routingBusy;
  useEffect(() => {
    // Best-effort on both sides: a failed pin must not break the flow it was
    // protecting, and a failed unpin leaves a popover that needs the tray icon
    // to dismiss - annoying, not broken.
    if (popoverHeld) void pinPopover().catch(() => {});
    else void unpinPopover().catch(() => {});
  }, [popoverHeld]);

  const onMenuSelect = useCallback(
    (action: MenuAction) => {
      setMenuOpen(false);
      if (action === "dashboard") {
        // Null means this gateway has no dashboard, which is worth saying
        // rather than opening a guess.
        if (dash === null) setActionError(NO_DASHBOARD);
        else
          void openExternal(dash.root).then((err) => {
            if (err) setActionError(err);
          });
      }
      else if (action === "support") {
        // The dashboard's Overview page: support is a floating action button in
        // its corner, not a route of its own (AG-598, settled 2026-09-07).
        if (dash === null) setActionError(NO_DASHBOARD);
        else
          void openExternal(dash.support).then((err) => {
            if (err) setActionError(err);
          });
      }
      else if (action === "docs") void openExternal(GATE_DOCS_URL).then((err) => {
        if (err) setActionError(err);
      });
      else if (action === "quit") {
        // The same path as the tray menu's Quit: exits outright, or reveals
        // the main window and raises the three-way dialog there when
        // config-routed tools would be left pointing at a dead relay.
        void requestQuit().catch(() => {});
        try {
          void getCurrentWindow().hide();
        } catch {
          /* plain-browser dev */
        }
      }
    },
    [dash],
  );



  if (!loaded) {
    // A sub-frame gap before the first read lands, same call as the window
    // shell: painting the signed-out card and replacing it a frame later is
    // worse than painting nothing.
    return null;
  }

  return (
    <Tray
      engine={proxy ? { running: proxy.running, starting } : undefined}
      groups={trayGroups}
      // Reported, not offered. The window's Settings pane owns this control -
      // the tray reports what the window decides and introduces no concept of
      // its own, the same rule the routing card follows.
      cli={
        proxy?.env_export_separable ? { on: proxy.env_export_opted_in } : undefined
      }
      rootRef={root}
      // Same chain as the window's rail. `account.org_name` is OAuth-only, so
      // on an api-key account this footer had nothing to name and asserted the
      // user had no organization, on the one line they would check to see which
      // org their traffic bills to. The reading's name comes off the cache read
      // the rows already do.
      orgName={orgLabel(account, readingOrgName)}
      // Only for an account that HAS orgs to switch between. An API-key account
      // holds no org locally, so the selector it would open has nothing to
      // offer, and the footer stays the label the frame draws.
      onSwitchOrg={
        account?.auth_mode === "oauth"
          ? () => {
              setMenuOpen(false);
              void requestSwitchOrg().catch((e) =>
                setActionError(classifyError(e, "generic")),
              );
            }
          : undefined
      }
      signedOut={account === null && !accountUnread}
      accountUnread={accountUnread}
      onToggleApp={toggleApp}
      onExpand={expand}
      menuOpen={menuOpen}
      onMenuToggle={() => setMenuOpen((v) => !v)}
      onMenuSelect={onMenuSelect}
      dialog={
        <>
          {shownError ? (
            // The tray draws no notice slot; the banner sits over the list the
            // way the dialogs do, because a swallowed failure is worse than an
            // undrawn surface.
            <div className="absolute inset-x-4 top-20 z-20">
              <ErrorBanner
                title={shownError.title}
                hint={shownError.hint}
                raw={shownError.raw}
                onDismiss={() => {
                  setActionError(null);
                  setRoutingError(null);
                }}
              />
            </div>
          ) : null}
          {routing.prompt?.kind === "drift" ? (
            <ReviewConfigDialog
              app={{ name: routing.prompt.name }}
              existingConfig={routing.prompt.existingConfig}
              gateRoute={proxy?.relay_base_url}
              configLocation={
                tools.find(
                  (t) =>
                    routing.prompt?.kind === "drift" && t.slug === routing.prompt.slug,
                )?.config_location ?? null
              }
              onKeep={() => routing.resolvePrompt(false)}
              onReplace={() => routing.resolvePrompt(true)}
            />
          ) : routing.prompt?.kind === "opencode-env" ? (
            // The tray routes OpenCode through the same `setAppRouted` the
            // window does, and `useRouting` raises this prompt from inside it -
            // then awaits a promise only a rendered dialog resolves. This slot
            // drew drift and trust and nothing else, so flipping OpenCode on
            // here set the prompt, showed nothing, and left the switch spinning
            // until the popover unmounted. Same dialog the window draws, from
            // the same component, so the two cannot drift apart again.
            <OpenCodeEnvDialog
              onCancel={() => routing.resolvePrompt(false)}
              onConfirm={() => routing.resolvePrompt(true)}
            />
          ) : routing.prompt?.kind === "trust" ? (
            // Same dialog as the window shell, for the same reason: the OS
            // keychain prompt that follows reads as malware unprompted.
            <Modal
              tone="warning"
              icon="shieldCheck"
              title="Trust the Gate certificate?"
              subtitle="Gate inspects your AI traffic locally, which needs a certificate your system trusts."
              secondary={{ label: "Not now", onClick: () => routing.resolvePrompt(false) }}
              primary={{ label: "Trust certificate", onClick: () => routing.resolvePrompt(true) }}
              onDismiss={() => routing.resolvePrompt(false)}
            >
              <p className="text-sm leading-5 text-neutral-600">
                The certificate stays on this machine, and you can remove it from
                Settings at any time.
              </p>
              <p className="text-sm font-medium leading-5 text-base-foreground">
                {trustPromptHint(platform)}
              </p>
            </Modal>
          ) : runningApps.stage?.kind === "offer" ? (
            <ApplyChangesDialog
              tools={reopenSubjects(runningApps.stage.tools)}
              onCloseApps={runningApps.goToConfirm}
              onReopenLater={runningApps.dismiss}
            />
          ) : runningApps.stage?.kind === "confirm" ? (
            <CloseAppsDialog
              tools={reopenSubjects(runningApps.stage.tools)}
              onGoBack={runningApps.goBack}
              onCloseApps={() => void runningApps.closeApps()}
            />
          ) : runningApps.stage?.kind === "work" &&
            allVerified(runningApps.stage.tools) ? (
            // The all-clear as drawn. Anything else is left to the rail, and
            // `useRunningApps` ends the stage for it.
            <ChangeReadyDialog
              app={{
                name:
                  runningApps.stage.tools.length === 1
                    ? runningApps.stage.tools[0].name
                    : "The affected apps",
              }}
              plural={runningApps.stage.tools.length !== 1}
              onDone={runningApps.dismiss}
            />
          ) : null}
        </>
      }
    />
  );
}
