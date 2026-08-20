import { useCallback, useEffect, useMemo, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import type {
  Account,
  OAuthStatus,
  Org,
  Preferences,
  ProxyState,
  ProviderState,
  PendingRestore,
  RestoreJournal,
  Tool,
  Verdict,
} from "./lib/api";
import {
  deviceName as fetchDeviceName,
  diagnostics as fetchDiagnostics,
  getAccount,
  getAccountKeyPrefix,
  installId as fetchInstallId,
  launchAtLoginStatus,
  listProviders,
  listTools,
  oauthStatus,
  openOnboardingWindow,
  proxyStatus,
  routedClientsStale,
  routingVerdicts,
  runningAgents as fetchRunningAgents,
  pendingQuitTools,
  disconnectToolsForQuit,
  quitApp,
  pendingRestore,
  resumeRestore,
  restoreJournal,
  getPreferences,
  setRoutingHealthNotifications,
  setShareDiagnostics,
} from "./lib/api";
import { useRouting, FamilyCascadeError } from "./lib/useRouting";
import { useSettingsActions } from "./lib/useSettingsActions";
import { useSetup } from "./lib/useSetup";
import { useRunningApps } from "./lib/useRunningApps";
import { useUpdate } from "./lib/useUpdate";
import type { UpdateState } from "./lib/useUpdate";
import { useWindowReopen } from "./lib/useWindowReopen";
import { classifyError } from "./lib/errors";
import type { ErrorContext } from "./lib/errors";
import { forwardBackendErrors } from "./lib/backendErrors";
import type { ClassifiedError } from "./lib/errors";
import { buildGroups } from "./lib/groups";
import { verdictStatus, verdictsBySlug } from "./lib/verdict";
import type { Group, GroupMember } from "./lib/groups";
import { openExternal } from "./lib/openExternal";
import { GATEWAY_SERVERS, GATE_DASHBOARD_URL, GATE_DOCS_URL } from "./lib/config";
import { hasSeenTour, markTourSeen } from "./lib/tour";
import { hasSeenOAuthOffer, markOAuthOfferSeen } from "./lib/oauthOffer";
import { TOUR_SEEN_EVENT } from "./screens/Onboarding";
import { AppShell } from "./components/gc/AppShell";
import { FamiliesPane } from "./components/gc/FamiliesPane";
import type { Family } from "./components/gc/FamiliesPane";
import { AppPane } from "./components/gc/AppPane";
import type { ModelChoice } from "./components/gc/AppPane";
import { Overview } from "./components/gc/Overview";
import { SettingsPane, buildSettingsSections } from "./components/gc/SettingsPane";
import type { DialogOrganization } from "./components/gc/dialogs";
import {
  ApplyChangesDialog,
  ChangeReadyDialog,
  CloseAppsDialog,
  ModelPickerDialog,
  QuitDialog,
  QuitLeftBehindDialog,
  UseGateModelDialog,
} from "./components/gc/dialogs";
import type { GateModelOption } from "./components/gc/dialogs";
import {
  ApiKeyPane,
  ConnectedPane,
  DiagnosticsPane,
  GatewayPicker,
  NameDevicePane,
  OrgPickerPane,
  SetupLayout,
  WelcomePane,
} from "./components/gc/setup";
import type { SetupOrganization } from "./components/gc/setup";
import {
  CollectedDataDialog,
  DiagnosticsDialog,
  RestoreDetailsDialog,
  DisconnectGateDialog,
  OAuthOfferDialog,
  RenameDeviceDialog,
  OrganizationSwitchedDialog,
  ReplaceApiKeyDialog,
  ResetGateConnectDialog,
  ReviewConfigDialog,
  SwitchGatewayDialog,
  SwitchOrganizationDialog,
} from "./components/gc/dialogs";
import { AlertBanner, ErrorBanner, RecoveryBanner } from "./components/gc/banners";
import { Modal } from "./components/gc/Modal";
import type {
  AppStatus,
  InventoryState,
  SidebarApp,
  SidebarView,
} from "./components/gc/Sidebar";
import type { TopnavAction } from "./components/gc/Topbar";
import { buildDiagnosticsReport } from "./lib/diagnosticsReport";
import { analyticsId, setAnalyticsConsent, track } from "./lib/analytics";
import { secretStoreName, usePlatform } from "./lib/platform";

/**
 * The new window UI, and the default surface as of 2026-08-17. `App.tsx` and the
 * popover are still reachable via `gcNewUi(false)`.
 *
 * Routing is wired: app and family-member switches go through `useRouting`,
 * which gates a drifted config behind the review dialog and the certificate
 * behind a prompt, then re-reads backend truth. Backend state pushes here too,
 * so a change made elsewhere repaints this window.
 *
 * Status lines come from `routing_verdicts`, not from `Tool.status`: a config
 * file Gate wrote is not evidence that anything is using it, so the sidebar asks
 * whether the relay answers and whether the tool's process predates the last
 * write. Switches still read intent off `Tool.status`, which is the split
 * `lib/groups.ts` documents. A row with no verdict yet says it is checking
 * rather than borrowing the config's answer.
 *
 * Settings and org switching go through `useSettingsActions`. Rows with no
 * backend behind them are passed no handler, which omits the control rather than
 * leaving a dead one on screen.
 *
 * Still awaiting a backend: the per-app model picker (session state only) and
 * the Overview and per-app metrics (the 24-hour endpoint). The verdict sweep
 * establishes that a route is live, not that the tool sent traffic through it -
 * nothing attributes requests to a tool yet.
 */
export function NewUiApp() {
  const [tools, setTools] = useState<Tool[]>([]);
  /** Observed routing, by slug. Separate from `tools` because it answers a
   * different question: `tools` is what the config says, this is what the
   * config is actually doing. An absent entry means the sweep has not
   * answered yet, which `verdictStatus` renders as "checking" rather than
   * guessing from the config. */
  const [verdicts, setVerdicts] = useState<Map<string, Verdict>>(new Map());
  const [providers, setProviders] = useState<ProviderState[]>([]);
  const [proxy, setProxy] = useState<ProxyState | null>(null);
  const [account, setAccount] = useState<Account | null>(null);
  const [oauth, setOAuth] = useState<OAuthStatus | null>(null);
  // Whether the first read has landed. A null account before it does is not the
  // same as no account, and treating them alike flashes sign-in at every user.
  const [loaded, setLoaded] = useState(false);
  const [version, setVersion] = useState("");
  const [launchAtLogin, setLaunchAtLogin] = useState(false);
  /**
   * Whether each read *failed*, kept apart from the value it failed to produce.
   *
   * `launch?.enabled ?? false` used to collapse "off" and "could not be read"
   * into one Off switch, which is a claim about the user's setting they cannot
   * distinguish from one they made. Settings now renders Unavailable + Retry for
   * these instead. Same rule as the routing verdict and the zeroed metrics: an
   * unknown is never rendered as a value.
   */
  const [launchAtLoginUnavailable, setLaunchAtLoginUnavailable] = useState(false);
  const [prefs, setPrefs] = useState<Preferences | null>(null);
  const [prefsUnavailable, setPrefsUnavailable] = useState(false);
  const [view, setView] = useState<SidebarView>({ kind: "overview" });
  const [menuOpen, setMenuOpen] = useState(false);
  /** A manual scan is in flight. Separate from `routingBusy`, which is about a
   * write: refusing to re-read while a toggle is mid-flight would be the wrong
   * coupling, and a scan changes nothing on disk. */
  const [refreshing, setRefreshing] = useState(false);
  /**
   * What the last detection scan established, as opposed to how many rows it
   * produced. `null` before the first one lands - which is not "nothing found",
   * and must not render as it.
   */
  const [scan, setScan] = useState<{ kind: "ok"; at: Date } | { kind: "failed" } | null>(null);
  // Held as text rather than a boolean: the report is a snapshot, and the copy
  // button has to hand over exactly what the dialog showed.
  const [diagnosticsReport, setDiagnosticsReport] = useState<string | null>(null);
  /** The read-only "what is collected" list. Separate from the report dialog:
   * that one shows this install's values, this one shows what leaves the device. */
  const [collectedDataOpen, setCollectedDataOpen] = useState(false);
  /**
   * Leading characters of the stored Gate key, as recorded in the account config.
   *
   * Null when there is no key, and also when the account predates the prefix
   * being written down - which is not the same as knowing it, so the row says the
   * key is in the keychain rather than drawing a fabricated `sk-gw` and twenty
   * asterisks, which is what it used to do.
   *
   * `backfill_account_key_prefix` could recover it from the keychain and is
   * deliberately not called: it can raise an OS prompt, and this row is a passive
   * mask nobody asked to reveal.
   */
  const [keyPrefix, setKeyPrefix] = useState<string | null>(null);
  /**
   * This install's id and this machine's name, both read from the backend.
   *
   * Null while the read is in flight or after it failed, which the rows render as
   * Unavailable rather than as a value. The install id used to be the PostHog
   * distinct id, which is absent in a build with no project key and absent again
   * once diagnostics are switched off - so the row read Unavailable on a perfectly
   * ordinary dev build, and on any install that opted out. The analytics id is
   * still in the diagnostics report, under its own name.
   */
  const [installId, setInstallId] = useState<string | null>(null);
  const [device, setDevice] = useState<string | null>(null);
  /** The environment picker under the sign-in card, collapsed until asked for. */
  const [gatewayOpen, setGatewayOpen] = useState(false);
  /**
   * The one-time offer to move a pasted key onto Constellation sign-in, with its
   * own busy and error state: the browser flow can fail, and the offer is what
   * the user is looking at when it does.
   */
  const [offerOpen, setOfferOpen] = useState(false);
  const [offerBusy, setOfferBusy] = useState(false);
  const [offerError, setOfferError] = useState<ClassifiedError | null>(null);
  // Dismissal is per-session and per-surface: the banner going away should not
  // stop the next launch offering the same update.
  const [updateDismissed, setUpdateDismissed] = useState(false);
  /**
   * Which model an app runs on, and the two overlays that change it.
   *
   * Session-only, and deliberately so: there is no backend for model selection
   * at all - no command, no Rust - so nothing here survives a reload. The design
   * is built and the wiring is in place; what is missing is somewhere to put the
   * answer. See plans/new-app-ui-figma.md.
   */
  const [modelChoice, setModelChoice] = useState<Record<string, ModelChoice>>({});
  const [modelOverlay, setModelOverlay] = useState<"picker" | "confirm-gate" | null>(null);
  /**
   * A quit the tray deferred to this window, and its aftermath.
   *
   * `quitTools` holds the config-routed tools still pointed at Gate; non-null
   * raises the dialog. `quitLeftBehind` holds the ones a teardown could not put
   * back, which AG-596 requires be named rather than quietly exited past.
   *
   * The names are swept from a backend buffer (at mount, then on each nudge)
   * rather than carried on the event, so a Quit clicked before this listener
   * registered is not lost - the same reasoning as `App.tsx`.
   */
  const [quitTools, setQuitTools] = useState<string[] | null>(null);
  const [quitLeftBehind, setQuitLeftBehind] = useState<string[] | null>(null);
  const [quitBusy, setQuitBusy] = useState(false);
  const platform = usePlatform();

  const loadLaunchAtLogin = useCallback(async () => {
    const launch = await launchAtLoginStatus().catch(() => null);
    setLaunchAtLoginUnavailable(launch === null);
    if (launch) setLaunchAtLogin(launch.enabled);
  }, []);

  const loadPreferences = useCallback(async () => {
    const p = await getPreferences().catch(() => null);
    setPrefsUnavailable(p === null);
    if (p) setPrefs(p);
  }, []);

  const loadKeyPrefix = useCallback(async () => {
    setKeyPrefix(await getAccountKeyPrefix().catch(() => null));
  }, []);

  const loadIdentity = useCallback(async () => {
    const [id, name] = await Promise.all([
      fetchInstallId().catch(() => null),
      fetchDeviceName().catch(() => null),
    ]);
    setInstallId(id);
    setDevice(name);
  }, []);

  /** The routing sweep, kept separate from {@link refresh} because it is the one
   * probe that costs network I/O and a process walk. Callers that changed a
   * tool's config re-run it; callers that only repainted do not have to. */
  const refreshVerdicts = useCallback(async () => {
    const v = await routingVerdicts().catch(() => null);
    if (v) setVerdicts(verdictsBySlug(v));
  }, []);

  const loadPending = useCallback(async () => {
    const [p, j] = await Promise.all([
      pendingRestore().catch(() => null),
      restoreJournal().catch(() => null),
    ]);
    if (p) setPending(p);
    // Read alongside the pending state, not lazily on click: the banner decides
    // whether to offer Review details at all, and it can only do that if it knows
    // whether a journal exists.
    setJournal(j);
  }, []);

  const refresh = useCallback(async () => {
    const [t, px] = await Promise.all([
      listTools().catch(() => null),
      proxyStatus().catch(() => null),
    ]);
    // A failed scan is not an empty machine. `catch(() => [])` used to collapse
    // the two, so a device Gate could not read rendered as a device with no AI
    // apps on it - the exact confusion AG-560 exists to remove.
    setScan(t ? { kind: "ok", at: new Date() } : { kind: "failed" });
    if (t) setTools(t);
    if (px) setProxy(px);
    // The engine coming up or going down changes every verdict, since the relay
    // health check is shared - so this follows the snapshot rather than waiting
    // for the next poll.
    void refreshVerdicts();
    // A master-on runs `restore_all`, which is what clears or shortens the
    // snapshots - so the notice has to be re-read on the same event that
    // repaints the switches, or it lingers after the work finished.
    void loadPending();
  }, [refreshVerdicts, loadPending]);

  /** Re-run detection because the user asked. Same reads as the event-driven
   * `refresh`, plus a flag so the control can refuse a second click. */
  const refreshNow = useCallback(async () => {
    setRefreshing(true);
    try {
      await refresh();
    } finally {
      setRefreshing(false);
    }
  }, [refresh]);

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

  // The engine changes state without us asking: a CLI toggle, the startup
  // auto-enable, another window. Repaint from the event rather than leaving a
  // stale switch on screen until the next click.
  useEffect(() => {
    const unlisten = listen("proxy-state-changed", () => {
      void refresh();
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [refresh]);

  useEffect(() => {
    void (async () => {
      const [t, p, px, acct, oauthState, v] = await Promise.all([
        listTools().catch(() => null),
        listProviders().catch(() => [] as ProviderState[]),
        proxyStatus().catch(() => null),
        getAccount().catch(() => null),
        oauthStatus().catch(() => null),
        getVersion().catch(() => ""),
      ]);
      void loadLaunchAtLogin();
      void loadPreferences();
      void loadIdentity();
      setTools(t ?? []);
      setScan(t ? { kind: "ok", at: new Date() } : { kind: "failed" });
      void refreshVerdicts();
      void loadPending();
      setProviders(p);
      setProxy(px);
      setAccount(acct);
      setOAuth(oauthState);
      setVersion(v);
      setLoaded(true);
    })();
  }, []);

  // Re-read on every account change rather than once: replacing the key writes a
  // new prefix, and an org switch or a sign-out re-reads the account anyway. The
  // account only changes on a user action, so this is not a poll.
  useEffect(() => {
    if (account?.has_api_key) void loadKeyPrefix();
    else setKeyPrefix(null);
  }, [account, loadKeyPrefix]);

  /**
   * First launch ever: open the tutorial window.
   *
   * The popover has done this since the intro moved into its own window, and this
   * shell only offered Replay tutorial in Settings - so a new install on what is
   * now the default surface replayed something it had never been shown.
   *
   * Unlike the popover this does **not** hide the main window. Stepping a 360px
   * panel aside is housekeeping; a 1024x720 window the user just opened
   * disappearing reads as a crash, and the onboarding window's close handler
   * reveals this one either way.
   */
  useEffect(() => {
    if (hasSeenTour()) return;
    void openOnboardingWindow("firstrun").catch(() => {});
  }, []);

  // The tutorial announces completion from its own webview; record the flag in
  // this one too, since the two do not share localStorage on every platform.
  useEffect(() => {
    const unlisten = listen(TOUR_SEEN_EVENT, () => markTourSeen());
    return () => {
      void unlisten.then((f) => f()).catch(() => {});
    };
  }, []);

  const update = useUpdate();
  const checkForUpdates = update.checkNow;

  // Silent at startup: offline, or an unreachable endpoint, is not worth
  // interrupting anyone about.
  useEffect(() => {
    void checkForUpdates();
  }, [checkForUpdates]);

  // A window left open for days would otherwise never see a release. Re-checking
  // when it is focused again costs one request and keeps the banner honest.
  useWindowReopen(() => {
    void checkForUpdates();
  });

  const [actionError, setActionError] = useState<ClassifiedError | null>(null);
  /**
   * Routing work that was recorded and did not finish, read from the provider
   * snapshots. Null until the first read; empty lists mean nothing outstanding,
   * which is the normal case.
   */
  const [pending, setPending] = useState<PendingRestore | null>(null);
  const [resuming, setResuming] = useState(false);
  /** Dismissed for this session only. The pending state lives on disk, so the
   * notice returns on the next launch until the work actually finishes - which is
   * the persistence the recovery action is supposed to have. */
  const [recoveryHidden, setRecoveryHidden] = useState(false);
  /** The read-only account of the last restore. Null when there is nothing to
   * explain; a restore that completed clears it. */
  const [journal, setJournal] = useState<RestoreJournal | null>(null);
  const [journalOpen, setJournalOpen] = useState(false);

  /**
   * Backend failures buffer Rust-side because they can predate this webview - the
   * startup auto-enable runs before either shell mounts. Sweep once at mount, then
   * on each nudge.
   *
   * The window shell had no drain at all, so a failed restore went to telemetry
   * and nowhere else: `report_backend_error("provider_restore", ...)` fires on both
   * restore passes in `proxy_enable`, and this window showed nothing. That is the
   * bug the popover's version was written to fix, reintroduced here.
   */
  useEffect(() => {
    const sweep = () =>
      void forwardBackendErrors().then((e) => {
        if (e) setActionError(e);
      });
    sweep();
    const unlisten = listen("backend-error-pending", sweep);
    return () => {
      void unlisten.then((f) => f()).catch(() => {});
    };
  }, []);


  /** Put the tools back, then quit - unless something stayed on Gate, in which
   * case name it and stay open. Quitting there would strand a config pointing at
   * a relay that dies with this process. */
  const disconnectAndQuit = useCallback(async () => {
    setQuitBusy(true);
    setActionError(null);
    try {
      const failed = await disconnectToolsForQuit();
      if (failed.length > 0) {
        setQuitLeftBehind(failed);
        setQuitTools(null);
        setQuitBusy(false);
        return;
      }
      await quitApp();
    } catch (e) {
      setActionError(classifyError(e, "quit_disable"));
      setQuitBusy(false);
    }
  }, []);

  const quitAnyway = useCallback(async () => {
    setQuitBusy(true);
    await quitApp().catch(() => {});
  }, []);

  const cancelQuit = useCallback(() => {
    setQuitTools(null);
    setQuitLeftBehind(null);
    setQuitBusy(false);
  }, []);


  const routing = useRouting({
    tools,
    proxy,
    onSnapshot: ({ tools: t, proxy: px }) => {
      setTools(t);
      setProxy(px);
      // A write landed, so the verdicts are stale: the tool may now need a
      // reopen, and the relay may have been auto-enabled by the connect.
      void refreshVerdicts();
    },
    onError: (e, context) => {
      // `connect` covers both directions of a tool write: the remedy copy is the
      // same either way. The engine-level actions are the ones whose remedy
      // genuinely differs - a cancelled admin prompt on the master toggle has
      // nothing to do with a config file - so those report their own context.
      const engineContexts: ErrorContext[] = ["proxy_toggle", "env_export", "untrust_ca"];
      const ctx = engineContexts.find((c) => c === context) ?? "connect";
      const classified = classifyError(e, ctx);
      setActionError(
        e instanceof FamilyCascadeError
          ? { ...classified, title: cascadeTitle(e) }
          : classified,
      );
    },
  });
  const routingBusy = routing.busy;

  const runningApps = useRunningApps({
    onError: (e) => setActionError(classifyError(e, "close_agents")),
  });

  /** Same follow-up as a single app: a family cascade rewrites configs too. */
  const routeFamily = useCallback(
    async (group: Group, next: boolean) => {
      setActionError(null);
      if (await routing.setFamilyRouted(group, next)) {
        await runningApps.offerAfterChange();
      }
    },
    [routing, runningApps],
  );

  /**
   * A tool's config was rewritten. If that app is open it is still on its old
   * route until it restarts, so offer to close it - but only when something was
   * actually written, which is why `setAppRouted` reports back.
   */
  const routeApp = useCallback(
    async (slug: string, next: boolean, force = false) => {
      setActionError(null);
      if (await routing.setAppRouted(slug, next, force)) {
        await runningApps.offerAfterChange();
      }
    },
    [routing, runningApps],
  );

  /**
   * Turn all routing on or off.
   *
   * Same follow-up as a config write: every routed tool is on its old route until
   * it restarts, so a master toggle that actually moved offers to close them.
   */
  const toggleMaster = useCallback(
    async (next: boolean) => {
      setActionError(null);
      if (await routing.setMasterRouted(next)) {
        await runningApps.offerAfterChange();
      }
    },
    [routing, runningApps],
  );

  const groups = useMemo<Group[]>(
    () =>
      proxy
        ? buildGroups(providers, tools, proxy.domains, {
            proxyOn: proxy.running,
            caTrusted: proxy.ca_trusted,
          })
        : [],
    [providers, tools, proxy],
  );

  const apps = useMemo<SidebarApp[]>(
    () =>
      tools
        .filter((t) => t.status.kind !== "not_installed")
        .map((t) => ({
          slug: t.slug,
          name: t.name,
          status: verdictStatus(verdicts.get(t.slug), {
            writeFailed: routing.writeFailures.has(t.slug),
          }),
          // Intent, not observation: a drifted tool is still one the user asked
          // to route. See the note on SidebarApp.
          on: t.status.kind === "connected" || t.status.kind === "drifted",
          busy: routingBusy,
        })),
    [tools, verdicts, routing.writeFailures, routingBusy],
  );

  /**
   * What the sidebar should say when the app list is empty. `ok` while there are
   * rows, because the rows speak for themselves; before the first scan lands the
   * state is unknown, and rendering "no apps detected" then would be a claim
   * nothing has checked.
   */
  const inventory = useMemo<InventoryState>(() => {
    if (apps.length > 0) return { kind: "ok" };
    if (scan === null) return { kind: "ok" };
    if (scan.kind === "failed") return { kind: "failed" };
    return { kind: "none", scannedAt: scan.at.toLocaleTimeString() };
  }, [apps.length, scan]);

  const families = useMemo<Family[]>(
    () =>
      groups.map((g) => ({
        id: g.id,
        name: g.name,
        // `cascadeDesired`, not `desired`: this switch governs only the members
        // it can flip. A chat member switched on alone would otherwise render
        // the family switch on while everything it governs is off, and clicking
        // it would ask to turn off a set already off - leaving it stuck on.
        on: g.cascadeDesired > 0,
        members: g.members.map((m) =>
          memberToFamilyMember(m, verdicts, routing.writeFailures),
        ),
      })),
    [groups, verdicts, routing.writeFailures],
  );

  /** Finish what the interrupted restore left. `restore_all` retries only the
   * recorded entries, so this repeats no completed write; the command hands back
   * what is still outstanding rather than a bare success. */
  const resumeNow = useCallback(async () => {
    setResuming(true);
    setActionError(null);
    try {
      setPending(await resumeRestore());
      // The retry may have changed what is routing, so re-read the rest too.
      await refresh();
    } catch (e) {
      setActionError(classifyError(e, "provider_restore"));
    } finally {
      setResuming(false);
    }
  }, [refresh]);

  /** What is still outstanding, providers and tools together: the user does not
   * care which snapshot an entry came from. */
  const recoveryNames = useMemo(
    () => [...(pending?.providers ?? []), ...(pending?.tools ?? [])].map((e) => e.name),
    [pending],
  );

  const noop = useCallback(() => {}, []);

  const onSession = useCallback(
    ({ account: a, oauth: o }: { account: Account | null; oauth: OAuthStatus | null }) => {
      setAccount(a);
      setOAuth(o);
    },
    [],
  );

  const setup = useSetup({
    loaded,
    account,
    oauth,
    onSession,
    onProxy: setProxy,
    // `undefined` while the preference read is in flight, which is not the same as
    // unanswered - see the note on the hook's argument.
    diagnosticsAnswered: prefs?.share_diagnostics_recorded,
    // Same `undefined` distinction: null means the name follows the hostname,
    // and no preferences at all means the read has not landed.
    deviceNamed: prefs ? prefs.device_name !== null : undefined,
  });

  const settings = useSettingsActions({
    account,
    proxyRunning: proxy?.running ?? false,
    launchAtLogin,
    onLaunchAtLogin: ({ enabled }) => setLaunchAtLogin(enabled),
    onAccount: setAccount,
    onDeviceName: setDevice,
    onSession,
    onProxy: setProxy,
    onError: (e) => setActionError(classifyError(e, "generic")),
  });

  /**
   * The one-time OAuth offer, for an account still on a pasted key.
   *
   * Raised here rather than in `useSetup`, and only from inside the app shell:
   * the setup panes are the sign-in decision itself, and an offer stacked over
   * them would be asking the same question twice.
   */
  useEffect(() => {
    if (!loaded) return;
    if (account?.auth_mode !== "api_key" || !account.has_api_key) return;
    if (hasSeenOAuthOffer()) return;
    setOfferOpen(true);
  }, [loaded, account]);

  const acceptOffer = useCallback(async () => {
    setOfferError(null);
    setOfferBusy(true);
    try {
      await settings.upgradeToOAuth();
      track("oauth_offer_accepted");
      // Seen whichever way the user leaves, so a completed upgrade cannot be
      // offered again on the next launch either.
      markOAuthOfferSeen();
      setOfferOpen(false);
    } catch (e) {
      setOfferError(classifyError(e, "sign_in"));
    } finally {
      setOfferBusy(false);
    }
  }, [settings]);

  const declineOffer = useCallback(() => {
    markOAuthOfferSeen();
    setOfferOpen(false);
  }, []);

  /**
   * Build the diagnostics report against live probes.
   *
   * This used to pass `backend: null`, `oauth: null`, `agents: null` and
   * `clientsStale: false` - four sections the popover fills in, and the last of
   * those is not "unknown" but a claim that routed clients are fine. Every probe
   * exists; the window simply never ran them.
   *
   * Sequential and best-effort, the same call `screens/Diagnostics.tsx` makes: on
   * macOS the backend snapshot shells out per network service, and these all touch
   * the same subsystems. A hole in the report is a finding; a report that took
   * thirty seconds is not.
   */
  const openDiagnostics = useCallback(async () => {
    // Something on screen while the probes run. A button that sits silent for a
    // couple of seconds reads as broken, which is the argument the version row's
    // update note makes.
    setDiagnosticsReport(COLLECTING_DIAGNOSTICS);
    const backend = await fetchDiagnostics().catch(() => null);
    const launch = await launchAtLoginStatus().catch(() => null);
    const clientsStale = await routedClientsStale().catch(() => false);
    // One process walk, raced against a timer: a process table that never answers
    // costs the scan and nothing else.
    let scanTimer: ReturnType<typeof setTimeout> | undefined;
    const agents = await Promise.race([
      fetchRunningAgents().catch(() => null),
      new Promise<null>((resolve) => {
        scanTimer = setTimeout(() => resolve(null), AGENT_SCAN_TIMEOUT_MS);
      }),
    ]);
    clearTimeout(scanTimer);
    setDiagnosticsReport(
      buildDiagnosticsReport({
        now: new Date(),
        version,
        platform,
        analyticsId: analyticsId(),
        backend,
        account,
        oauth,
        proxy,
        providers,
        tools,
        launchAtLogin: launch,
        clientsStale,
        agents,
      }),
    );
  }, [version, platform, account, oauth, proxy, providers, tools]);

  const settingsSections = useMemo(
    () =>
      buildSettingsSections({
        // Plan still has no backend, so it reads as unknown rather than as an
        // invented value, and its action is omitted entirely.
        deviceName: device ?? "Unavailable",
        onRenameDevice: device ? () => settings.openRenameDevice(device) : undefined,
        installId: installId ?? "Unavailable",
        loginId: account?.org_name ?? "-",
        plan: "-",
        gateway: account?.gateway_base_url ?? "-",
        apiKeyMasked: maskedKey(keyPrefix, account?.has_api_key ?? false),
        launchAtLogin,
        launchAtLoginUnavailable,
        routingHealthNotifications: prefs?.routing_health_notifications,
        shareDiagnostics: prefs?.share_diagnostics,
        preferencesUnavailable: prefsUnavailable,
        version: version ? `v${version}` : "-",
        updateNote: updateNoteFor(update),
        // Absent on a platform with no proxy subsystem: there is no engine, so
        // there is no certificate to describe.
        certificate: proxy ? (proxy.ca_trusted ? "Trusted" : "Not trusted") : undefined,
        // Only while it is actually trusted. Removing a certificate that is not
        // there is a button that cannot do anything.
        onRemoveCertificate: proxy?.ca_trusted ? () => void routing.untrustCa() : undefined,
        onChangeGateway: settings.openSwitchGateway,
        onCopyInstallId: installId ? () => void settings.copyText(installId) : noop,
        // Only where there is a key to replace. On an OAuth account `saveAccount`
        // with a key would flip auth_mode to api_key, quietly converting the
        // account behind a button that says "replace".
        onReplaceKey: account?.auth_mode === "api_key" ? settings.openReplaceKey : undefined,
        // Only where there is a session to end. An API-key account never had one;
        // reset is its way out.
        onDisconnect: account?.auth_mode === "oauth" ? settings.openDisconnect : undefined,
        onReviewReset: settings.openReset,
        onToggleLaunchAtLogin: () => void settings.toggleLaunchAtLogin(),
        onRetryLaunchAtLogin: () => void loadLaunchAtLogin(),
        // Optimistic then re-read: the switch has to move on click, and the
        // re-read is what makes a failed write show up rather than leaving the
        // UI asserting a value the file does not hold.
        onToggleRoutingHealthNotifications: () => {
          const next = !(prefs?.routing_health_notifications ?? true);
          setPrefs((p) => (p ? { ...p, routing_health_notifications: next } : p));
          void setRoutingHealthNotifications(next)
            .catch((e) => setActionError(classifyError(e, "generic")))
            .finally(() => void loadPreferences());
        },
        onToggleShareDiagnostics: () => {
          const next = !(prefs?.share_diagnostics ?? true);
          setPrefs((p) => (p ? { ...p, share_diagnostics: next } : p));
          // Stop (or resume) collection immediately, not on the next launch. An
          // opt-out that only takes effect after a restart is not an opt-out, and
          // this happens before the write so a failed write cannot leave the
          // client sending after the user said no.
          setAnalyticsConsent(next);
          void setShareDiagnostics(next)
            .catch((e) => setActionError(classifyError(e, "generic")))
            .finally(() => void loadPreferences());
        },
        onRetryPreferences: () => void loadPreferences(),
        onOpenDocs: () => void openExternal(GATE_DOCS_URL),
        // No `onContactSupport`, so the row is omitted: there is no support URL
        // anywhere in the app to open, and a button that opens an invented
        // address is worse than an absent one. The topnav's Contact support
        // entry is dead for the same reason.

        // The tutorial is its own window, already built and wired.
        onReplayTutorial: () => void openOnboardingWindow("settings"),
        // Explicit, so this one reports back: silence on a button the user just
        // pressed reads as broken.
        onCheckForUpdates: () => void update.checkNow(true),
        onViewCollectedData: () => setCollectedDataOpen(true),
        onViewDiagnostics: () => void openDiagnostics(),
        // Deliberately absent, so the control is absent too: plan upgrade has no
        // billing URL to open and Contact support has no address. Rename is
        // omitted only while the name has not been read - renaming to something
        // when Gate cannot say what it is now would be a blind edit.
      }),
    // The individual callbacks rather than `settings`: the hook returns a fresh
    // object each render, which would defeat the memo.
    [
      account,
      launchAtLogin,
      launchAtLoginUnavailable,
      prefs,
      prefsUnavailable,
      loadLaunchAtLogin,
      loadPreferences,
      version,
      installId,
      keyPrefix,
      device,
      installId,
      settings.copyText,
      settings.openRenameDevice,
      settings.openReplaceKey,
      settings.openDisconnect,
      settings.openReset,
      settings.openSwitchGateway,
      settings.toggleLaunchAtLogin,
      routing.untrustCa,
      openDiagnostics,
      update,
      platform,
      proxy,
      providers,
      tools,
      noop,
    ],
  );

  // The picker needs its list, and only it knows when it is on screen. Guarded on
  // `orgs === null` so a genuine empty list - the dead end the pane draws - does
  // not re-read forever.
  const setupStageKind = setup.stage.kind;
  const setupOrgs = setup.orgs;
  const loadOrgs = setup.loadOrgs;
  useEffect(() => {
    if (setupStageKind === "org-picker" && setupOrgs === null) void loadOrgs();
  }, [setupStageKind, setupOrgs, loadOrgs]);

  const protectedCount = apps.filter((a) => a.status.kind === "protected").length;

  // A drifted app's sidebar switch reads on - intent, and drift means the config
  // changed behind Gate rather than the user turning it off. So the sidebar can
  // only turn it off, and re-adopting is this card's job. Its switch reads off
  // because the app is not protected, and flipping it on is what reaches the
  // review gate.
  const drifted = useMemo(() => tools.filter((t) => t.status.kind === "drifted"), [tools]);
  const driftAlert = drifted.length ? (
    <AlertBanner
      title={`${drifted[0].name} isn't protected`}
      body="Its config changed outside Gate, so its traffic isn't routed. Reconnect to restore protection."
      on={false}
      switchLabel={drifted[0].name}
      onToggle={() => void routeApp(drifted[0].slug, true)}
      onDismiss={noop}
      paging={
        drifted.length > 1
          ? // Paging is drawn for the multiple-apps variant. Selecting which app
            // the card shows is not wired yet, so the controls stay inert rather
            // than pretending to page.
            { onPrev: noop, onNext: noop }
          : undefined
      }
    />
  ) : undefined;

  const onMenuSelect = useCallback((action: TopnavAction) => {
    setMenuOpen(false);
    if (action === "dashboard") void openExternal(GATE_DASHBOARD_URL);
    // The docs entry was drawn, listed and dead: `GATE_DOCS_URL` is the same one
    // the Settings row opens.
    else if (action === "docs") void openExternal(GATE_DOCS_URL);
  }, []);

  const setupError = setup.error ? classifyError(setup.error, "sign_in") : null;

  // Before there is a usable credential there is nothing to navigate, so the
  // window is chrome plus one centred card rather than the shell with an empty
  // sidebar. The stage is derived from what is on disk, so reset and a dead
  // session both land here without anything having to route them.
  if (setup.stage.kind === "loading") {
    // A sub-frame gap before the first read lands. Painting the sign-in card and
    // replacing it a frame later is worse than painting nothing.
    return null;
  }
  if (setup.stage.kind !== "ready") {
    const stage = setup.stage;
    const gatewayPicker = (
      <GatewayPicker
        value={setup.gateway}
        servers={GATEWAY_SERVERS}
        open={gatewayOpen}
        onOpenChange={setGatewayOpen}
        onSelect={setup.setGateway}
      />
    );
    return (
      <SetupLayout
        menuOpen={menuOpen}
        onMenuToggle={() => setMenuOpen((v) => !v)}
        onMenuSelect={onMenuSelect}
      >
        {stage.kind === "welcome" ? (
          <WelcomePane
            reauth={stage.reauth}
            onSignIn={() => void setup.signIn()}
            onUseApiKey={setup.openApiKey}
            // The card had no way to name the gateway it was about to sign in
            // against, so the new shell could only ever reach the build's
            // default - the popover has offered this since first run existed.
            gateway={gatewayPicker}
            busy={setup.busy}
            error={setupError && <SetupNote error={setupError} />}
          />
        ) : stage.kind === "api-key" ? (
          <ApiKeyPane
            apiKey={setup.apiKey}
            onApiKeyChange={setup.setApiKey}
            onConnect={() => void setup.connectWithApiKey()}
            onGoBack={setup.closeApiKey}
            gateway={gatewayPicker}
            busy={setup.busy}
            error={setupError && <SetupNote error={setupError} />}
          />
        ) : stage.kind === "org-picker" ? (
          <OrgPickerPane
            organizations={(setup.orgs ?? []).map(toSetupOrg)}
            selectedId={setup.selectedOrgId}
            onSelect={setup.selectOrg}
            onContinue={() => void setup.confirmOrg()}
            // The design draws both affordances on the dead end and both mean
            // the same thing here: the session is already spent, so the only
            // way back to the sign-in choice is to drop it.
            onGoBack={() => void setup.signOut()}
            onUseDifferentAccount={() => void setup.signOut()}
            busy={setup.busy}
            error={setupError && <SetupNote error={setupError} />}
          />
        ) : stage.kind === "name-device" ? (
          <NameDevicePane
            value={setup.deviceNameDraft}
            onChange={setup.setDeviceNameDraft}
            onContinue={() => void setup.nameDevice()}
            onSkip={setup.skipNaming}
            busy={setup.busy}
            error={setupError && <SetupNote error={setupError} />}
          />
        ) : stage.kind === "diagnostics" ? (
          <DiagnosticsPane
            share={prefs?.share_diagnostics ?? true}
            onToggleShare={() =>
              setPrefs((p) =>
                p ? { ...p, share_diagnostics: !p.share_diagnostics } : p,
              )
            }
            busy={setup.busy}
            onContinue={() => {
              // Records the *displayed* value, changed or not: leaving the default
              // in place is an answer, and treating it as unanswered would ask
              // again on the next launch. This is also what dismisses the step,
              // since the stage is derived from the stored flag.
              const share = prefs?.share_diagnostics ?? true;
              setAnalyticsConsent(share);
              void setShareDiagnostics(share)
                .catch((e) => setActionError(classifyError(e, "generic")))
                .finally(() => void loadPreferences());
            }}
          />
        ) : (
          <ConnectedPane
            workspace={account?.org_name ?? account?.gateway_base_url ?? "Gate"}
            offerRouting={!!proxy && !proxy.running}
            busy={setup.busy}
            onTurnOnRouting={() => void setup.turnOnRouting()}
            onDone={setup.finish}
          />
        )}
      </SetupLayout>
    );
  }

  return (
    <AppShell
      menuOpen={menuOpen}
      onMenuToggle={() => setMenuOpen((v) => !v)}
      onMenuSelect={onMenuSelect}
      update={
        update.available && !updateDismissed
          ? {
              version: `v${update.available.version}`,
              onUpdate: () => void update.install(),
              onDismiss: () => setUpdateDismissed(true),
            }
          : undefined
      }
      routing={{ protectedCount, totalCount: apps.length }}
      orgName={account?.org_name ?? "No organization"}
      onSwitchOrg={() => {
        setActionError(null);
        void settings.openSwitchOrg();
      }}
      view={view}
      onNavigate={setView}
      apps={apps}
      onSelectApp={(slug) => setView({ kind: "app", slug })}
      onRefreshApps={() => void refreshNow()}
      refreshingApps={refreshing}
      inventory={inventory}
      notice={
        actionError ? (
          <ErrorBanner
            title={actionError.title}
            hint={actionError.hint}
            onDismiss={() => setActionError(null)}
          />
        ) : recoveryNames.length > 0 && !recoveryHidden ? (
          // Below the error banner: a failure that just happened outranks a
          // recorded one that can still be resumed.
          <RecoveryBanner
            names={recoveryNames}
            busy={resuming}
            onResume={() => void resumeNow()}
            onReviewDetails={journal ? () => setJournalOpen(true) : undefined}
            onFinishLater={() => setRecoveryHidden(true)}
          />
        ) : undefined
      }
      onToggleApp={(slug, next) => void routeApp(slug, next)}
      dialog={
        // A pending quit decision outranks every other overlay: the user asked
        // to leave, and an update prompt or routing notice must not sit on top
        // of the question. Same precedence the popover gives it (TAKEOVER_Z.quit).
        quitLeftBehind !== null ? (
          <QuitLeftBehindDialog
            tools={quitLeftBehind}
            busy={quitBusy}
            onRetry={() => void disconnectAndQuit()}
            onQuitAnyway={() => void quitAnyway()}
            onCancel={cancelQuit}
          />
        ) : quitTools !== null ? (
          <QuitDialog
            tools={quitTools}
            busy={quitBusy}
            onDisconnectAndQuit={() => void disconnectAndQuit()}
            onQuitAnyway={() => void quitAnyway()}
            onCancel={cancelQuit}
          />
        ) : routing.prompt?.kind === "drift" ? (
          <ReviewConfigDialog
            app={{ name: routing.prompt.name }}
            existingConfig={routing.prompt.existingConfig}
            // The relay is what a config-routed tool gets pointed at. Null
            // before a port has been bound, and the dialog omits the row rather
            // than inventing an address.
            gateRoute={proxy?.relay_base_url}
            configLocation={configLocationFor(tools, routing.prompt.slug)}
            onKeep={() => routing.resolvePrompt(false)}
            onReplace={() => routing.resolvePrompt(true)}
          />
        ) : routing.prompt?.kind === "trust" ? (
          // Not in the Figma: the new design has no certificate surface, and
          // connecting cannot proceed without one. Asking first matters because
          // the OS keychain prompt that follows reads as malware unprompted.
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
              Your operating system will ask for permission. The certificate stays on this
              machine, and you can remove it from Settings at any time.
            </p>
          </Modal>
        ) : routing.prompt?.kind === "untrust" ? (
          // The other half of the certificate story, and the reason the trust copy
          // above no longer says reset is the only way out.
          <Modal
            tone="danger"
            icon="triangleAlert"
            title="Remove the Gate certificate?"
            subtitle="Sites and apps routed through the local proxy stop being inspected until it is trusted again."
            secondary={{ label: "Keep it", onClick: () => routing.resolvePrompt(false) }}
            primary={{
              label: "Remove certificate",
              onClick: () => routing.resolvePrompt(true),
              destructive: true,
            }}
            onDismiss={() => routing.resolvePrompt(false)}
          >
            <p className="text-sm leading-5 text-neutral-600">
              Routing itself stays on, and your tools keep their configuration. Your
              operating system may ask for permission to remove it.
            </p>
          </Modal>
        ) : runningApps.stage?.kind === "offer" ? (
          <ApplyChangesDialog
            apps={runningApps.stage.apps.map((name) => ({ name }))}
            onCloseApps={runningApps.goToConfirm}
            onReopenLater={runningApps.dismiss}
          />
        ) : runningApps.stage?.kind === "confirm" ? (
          <CloseAppsDialog
            apps={runningApps.stage.apps.map((name) => ({ name }))}
            onGoBack={runningApps.goBack}
            onCloseApps={() => void runningApps.closeApps()}
          />
        ) : runningApps.stage?.kind === "done" ? (
          <ChangeReadyDialog
            app={{ name: closedLabel(runningApps.stage.apps) }}
            onDone={runningApps.dismiss}
          />
        ) : modelOverlay === "picker" ? (
          <ModelPickerDialog
            // Empty until a gateway endpoint reports what it offers. The design
            // draws eleven `gate/...` ids; shipping those as though they were
            // real would put a fabricated model catalogue in front of the user,
            // which is the same argument the zeroed metrics make.
            models={GATE_MODELS}
            selectedId={undefined}
            onSelect={() => setModelOverlay(null)}
            onDismiss={() => setModelOverlay(null)}
          />
        ) : modelOverlay === "confirm-gate" ? (
          <UseGateModelDialog
            app={{ name: appFor(apps, view.kind === "app" ? view.slug : "")?.name ?? "this app" }}
            vendor="-"
            modelId="-"
            credits="-"
            onKeepAppDefault={() => setModelOverlay(null)}
            onUseGateCredits={() => {
              if (view.kind === "app") {
                setModelChoice((m) => ({ ...m, [view.slug]: "gate" }));
              }
              setModelOverlay(null);
            }}
          />
        ) : journalOpen && journal ? (
          <RestoreDetailsDialog journal={journal} onClose={() => setJournalOpen(false)} />
        ) : collectedDataOpen ? (
          <CollectedDataDialog onClose={() => setCollectedDataOpen(false)} />
        ) : diagnosticsReport !== null ? (
          <DiagnosticsDialog
            report={diagnosticsReport}
            copied={settings.copied}
            onCopy={() => void settings.copyText(diagnosticsReport)}
            onClose={() => setDiagnosticsReport(null)}
          />
        ) : settings.prompt?.kind === "replace-key" ? (
          <ReplaceApiKeyDialog
            currentKeyMasked={maskedKey(keyPrefix, account?.has_api_key ?? false)}
            newKey={settings.newKey}
            onNewKeyChange={settings.setNewKey}
            onCancel={settings.dismissPrompt}
            onReplace={() => void settings.replaceKey()}
          />
        ) : settings.prompt?.kind === "switch-org" ? (
          <SwitchOrganizationDialog
            organizations={settings.prompt.orgs.map(toDialogOrg)}
            selectedId={settings.prompt.selectedId}
            onSelect={settings.selectOrg}
            onCancel={settings.dismissPrompt}
            onConfirm={() => void settings.confirmSwitchOrg()}
          />
        ) : settings.prompt?.kind === "rename-device" ? (
          <RenameDeviceDialog
            currentName={settings.prompt.currentName}
            newName={settings.newDeviceName}
            onNewNameChange={settings.setNewDeviceName}
            onCancel={settings.dismissPrompt}
            onRename={() => void settings.renameDevice()}
          />
        ) : settings.prompt?.kind === "switch-gateway" ? (
          <SwitchGatewayDialog
            servers={GATEWAY_SERVERS}
            selectedUrl={settings.prompt.selectedUrl}
            currentUrl={account?.gateway_base_url ?? ""}
            busy={settings.busy}
            onSelect={settings.selectGateway}
            onCancel={settings.dismissPrompt}
            onConfirm={() => void settings.confirmSwitchGateway()}
          />
        ) : settings.prompt?.kind === "org-switched" ? (
          <OrganizationSwitchedDialog
            organizationName={settings.prompt.name}
            onDone={settings.dismissPrompt}
          />
        ) : settings.prompt?.kind === "disconnect" ? (
          <DisconnectGateDialog
            onCancel={settings.dismissPrompt}
            onDisconnect={() => void settings.confirmDisconnect()}
          />
        ) : settings.prompt?.kind === "reset" ? (
          <ResetGateConnectDialog
            acknowledged={settings.prompt.acknowledged}
            onAcknowledgedChange={settings.acknowledgeReset}
            onCancel={settings.dismissPrompt}
            onReset={() => void settings.confirmReset()}
          />
        ) : offerOpen ? (
          // Lowest precedence in the stack: anything the user just did, a pending
          // quit, or an update outranks an offer they did not ask for.
          <OAuthOfferDialog
            secretStore={secretStoreName(platform, "the")}
            busy={offerBusy}
            error={
              offerError && (
                <p
                  role="alert"
                  className="rounded-lg border border-red-200 bg-red-50 p-3 text-sm leading-5 text-red-900"
                >
                  <span className="font-medium">{offerError.title}</span> {offerError.hint}
                </p>
              )
            }
            onSignIn={() => void acceptOffer()}
            onKeepKey={declineOffer}
          />
        ) : undefined
      }
    >
      {view.kind === "settings" ? (
        <SettingsPane sections={settingsSections} />
      ) : view.kind === "families" ? (
        <FamiliesPane
          families={families}
          // The engine's own switch. Without it a window whose routing was off
          // could start it only by accident, through a config member's connect -
          // and a chat domain, which routes through the engine rather than the
          // relay, could not start it at all.
          master={
            proxy
              ? {
                  on: proxy.running,
                  busy: routingBusy,
                  caTrusted: proxy.ca_trusted,
                  onToggle: (next) => void toggleMaster(next),
                  // Absent on Linux, where these variables *are* the system proxy
                  // and cannot be declined without turning routing off.
                  envExport: proxy.env_export_separable
                    ? {
                        on: proxy.env_export_opted_in,
                        onToggle: (next) => {
                          setActionError(null);
                          void routing.setEnvExport(next);
                        },
                      }
                    : undefined,
                }
              : undefined
          }
          onToggleFamily={(id, next) => {
            const group = groups.find((g) => g.id === id);
            if (group) void routeFamily(group, next);
          }}
          onToggleMember={(familyId, key, next) => {
            setActionError(null);
            const member = families
              .find((f) => f.id === familyId)
              ?.members.find((m) => m.key === key);
            if (!member) return;
            void (member.kind === "proxy"
              ? routing.setDomainRouted(key, next)
              : routeApp(key, next));
          }}
        />
      ) : view.kind === "app" ? (
        <AppPane
          name={appFor(apps, view.slug)?.name ?? view.slug}
          // Intent, not the verdict: a drifted app is still one the user asked to
          // route, and driving this switch from the observed status is the bug
          // `lib/groups.ts` documents - it renders off, and clicking it turns off
          // the setting the user was trying to turn on.
          isProtected={appFor(apps, view.slug)?.on ?? false}
          busy={routingBusy}
          onToggleProtected={() =>
            void routeApp(view.slug, !(appFor(apps, view.slug)?.on ?? false))
          }
          stats={EMPTY_STATS}
          buckets={[]}
          modelChoice={modelChoice[view.slug] ?? "app"}
          // Switching to a Gate model spends PAYG credits, so it is confirmed
          // rather than taken on a radio click. Switching back is not.
          onChooseModel={(choice) => {
            if (choice === "gate") setModelOverlay("confirm-gate");
            else setModelChoice((m) => ({ ...m, [view.slug]: "app" }));
          }}
          gateModel={{ vendor: "-", id: "-" }}
          onChangeModel={() => setModelOverlay("picker")}
          credits="-"
          // No billing endpoint, but the row's own glyph promises an external
          // link, and the dashboard is where credits are actually bought.
          onAddCredits={() => void openExternal(GATE_DASHBOARD_URL)}
          activity={[]}
          alert={driftAlert}
        />
      ) : (
        <Overview
          stats={EMPTY_STATS}
          buckets={[]}
          policies={[]}
          savings={[]}
          // Neither policies nor savings has a local backend; both are configured
          // on the web. Dead buttons were worse: the user cannot tell "not built"
          // from "broken".
          onManagePolicies={() => void openExternal(GATE_DASHBOARD_URL)}
          onManageSavings={() => void openExternal(GATE_DASHBOARD_URL)}
          period="Awaiting the 24-hour backend"
          alert={driftAlert}
        />
      )}
    </AppShell>
  );
}

/** The 24-hour endpoint is still being built. Zeros rather than plausible
 *  numbers: a preview that invents traffic is one somebody screenshots as
 *  real. */
/** No gateway endpoint reports the models on offer yet. See the picker. */
const GATE_MODELS: GateModelOption[] = [];

/** What both panes show until the 24-hour endpoint exists. `null` rather than
 *  `0` for the saving: the design draws `N/A` for a period it has no figure for,
 *  and "0%" would be a claim about traffic that was never measured. */
const EMPTY_STATS = {
  messages: 0,
  blockedFlagged: 0,
  tokensSavedPercent: null,
  tokensSavedAmount: undefined,
};

/** The file Gate rewrites for one tool, for the drift review's copy. */
function configLocationFor(tools: Tool[], slug: string): string | null {
  return tools.find((t) => t.slug === slug)?.config_location ?? null;
}

function appFor(apps: SidebarApp[], slug: string): SidebarApp | undefined {
  return apps.find((a) => a.slug === slug);
}

/**
 * The design's org rows carry "12 members - Free plan", neither of which the
 * orgs endpoint returns. Slug and role are what it does return, joined the same
 * way `screens/OrgPicker.tsx` joins them so the two pickers read alike.
 */
function toDialogOrg(org: Org): DialogOrganization {
  return {
    id: org.orgId,
    name: org.name,
    initials: initialsOf(org.name),
    meta: [org.slug, org.role].filter(Boolean).join(" · "),
  };
}

/**
 * `ChangeReadyDialog` names one subject ("Codex closed successfully"), so naming
 * a single app when that is what was closed, and staying vague when it was
 * several, beats asserting something that was not true.
 */
/** "Couldn't connect Codex", or "Couldn't connect 2 of 4: Codex, OpenCode". */
function cascadeTitle(e: FamilyCascadeError): string {
  const verb = e.routed ? "connect" : "disconnect";
  return e.names.length === 1
    ? `Couldn't ${verb} ${e.names[0]}`
    : `Couldn't ${verb} ${e.names.length} of ${e.attempted}: ${e.names.join(", ")}`;
}

function closedLabel(apps: string[]): string {
  return apps.length === 1 ? apps[0] : "The affected apps";
}

/**
 * What the version row says after the user presses Check for updates. Nothing
 * until they do: a standing "you're up to date" is noise, and the banner already
 * speaks for a found update.
 */
function updateNoteFor(update: UpdateState): string | undefined {
  if (update.checking) return "Checking for updates...";
  if (update.failed) return "That update could not be installed. Try again.";
  switch (update.outcome) {
    case "up-to-date":
      return "You're on the latest version.";
    case "failed":
      return "Could not reach the update server.";
    default:
      return undefined;
  }
}

/** The setup panes take the same shape as the dialog's org rows. */
function toSetupOrg(org: Org): SetupOrganization {
  return toDialogOrg(org);
}

/** Title plus remedy, the same two lines the popover's `ErrorNote` shows. */
function SetupNote({ error }: { error: ClassifiedError }) {
  return (
    <>
      <span className="font-medium">{error.title}</span> {error.hint}
    </>
  );
}

function initialsOf(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  const letters =
    words.length > 1 ? words[0][0] + words[1][0] : (words[0] ?? "?").slice(0, 2);
  return letters.toUpperCase();
}

/**
 * AG-562 requires every surface to show the same status, so a config member here
 * reads the same verdict the sidebar row does rather than deriving its own line.
 *
 * Proxy members keep the `routed` derivation, and that is not a shortcut left
 * undone: `routing_verdicts` covers registry integrations, and a catalog domain
 * routes through the engine rather than the relay, so its observation is
 * certificate trust plus the master switch - which is exactly what `routed`
 * already folds in.
 */
function memberToFamilyMember(
  m: GroupMember,
  verdicts: Map<string, Verdict>,
  writeFailures: ReadonlySet<string>,
): Family["members"][number] {
  const status: AppStatus =
    m.kind === "config"
      ? verdictStatus(verdicts.get(m.key), { writeFailed: writeFailures.has(m.key) })
      : m.routed
        ? { kind: "protected" }
        : { kind: "not-routed", detail: m.desired ? "Blocked" : "Off" };
  return { key: m.key, name: m.name, kind: m.kind, status, on: m.desired };
}

/** Placeholder while the probes run, and what a copy taken mid-collection would
 *  hand over - so it says what it is rather than looking like a report. */
const COLLECTING_DIAGNOSTICS = "Collecting diagnostics...";

/** The process walk's budget, matching `screens/Diagnostics.tsx`: a report that
 *  is missing the agent section beats a dialog that never opens. */
const AGENT_SCAN_TIMEOUT_MS = 2000;

/**
 * The stored Gate key, masked.
 *
 * The prefix is the part that identifies *which* key is stored, which is the
 * whole reason to show a masked value at all. Accounts saved before the prefix
 * was recorded have none, and say so: an invented `sk-gw` is the same class of
 * mistake as the zeroed metrics.
 */
function maskedKey(prefix: string | null, hasKey: boolean): string {
  if (!hasKey) return "Not set";
  return prefix ? `${prefix}${"*".repeat(20)}` : "Stored in the keychain";
}
