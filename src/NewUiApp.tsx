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
  Tool,
  Verdict,
} from "./lib/api";
import {
  getAccount,
  launchAtLoginStatus,
  listProviders,
  listTools,
  oauthStatus,
  openOnboardingWindow,
  proxyEnable,
  proxyStatus,
  proxyTrustCa,
  routingVerdicts,
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
import type { ClassifiedError } from "./lib/errors";
import { buildGroups } from "./lib/groups";
import { verdictStatus, verdictsBySlug } from "./lib/verdict";
import type { Group, GroupMember } from "./lib/groups";
import { openExternal } from "./lib/openExternal";
import {
  GATE_API_KEYS_URL,
  GATE_DASHBOARD_URL,
  GATE_DOCS_URL,
  GATE_POLICIES_URL,
  GATE_SAVINGS_URL,
  GATE_SUPPORT_URL,
} from "./lib/config";
import { AppShell } from "./components/gc/AppShell";
import { FamiliesPane } from "./components/gc/FamiliesPane";
import type { Family } from "./components/gc/FamiliesPane";
import { AppPane } from "./components/gc/AppPane";
import type { ModelChoice } from "./components/gc/AppPane";
import { Overview } from "./components/gc/Overview";
import type { UsageStats } from "./components/gc/metrics";
import { InstallationPicker } from "./components/gc/InstallationPicker";
import { useActivity, useInstallations } from "./lib/activity";
import { buildNotices } from "./lib/notices";
import type { NoticeAction } from "./lib/notices";
import type { ActivityFailure, ActivityView } from "./lib/activity";
import { failureNotice, sectionNotice } from "./lib/activityGaps";
import type { GapActionKind } from "./lib/activityGaps";
import { SettingsPane, buildSettingsSections } from "./components/gc/SettingsPane";
import type { DialogOrganization } from "./components/gc/dialogs";
import {
  ApplyChangesDialog,
  ChangeReadyDialog,
  CloseAppsDialog,
  ModelPickerDialog,
  UseGateModelDialog,
} from "./components/gc/dialogs";
import type { GateModelOption } from "./components/gc/dialogs";
import {
  ConnectedPane,
  OrgPickerPane,
  SetupLayout,
  WelcomePane,
} from "./components/gc/setup";
import type { SetupOrganization } from "./components/gc/setup";
import {
  DiagnosticsDialog,
  DisconnectGateDialog,
  OrganizationSwitchedDialog,
  ReplaceApiKeyDialog,
  ResetGateConnectDialog,
  ReviewConfigDialog,
  SwitchOrganizationDialog,
} from "./components/gc/dialogs";
import { AlertBanner, ErrorBanner } from "./components/gc/banners";
import { Modal } from "./components/gc/Modal";
import type { AppStatus, SidebarApp, SidebarView } from "./components/gc/Sidebar";
import type { TopnavAction } from "./components/gc/Topbar";
import { buildDiagnosticsReport } from "./lib/diagnosticsReport";
import { analyticsId } from "./lib/analytics";
import { usePlatform } from "./lib/platform";
import type { Platform } from "./lib/platform";

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
 * The Overview reads `GET /v1/me/activity` through `useActivity`, which serves
 * the previously held reading off disk while the network call is in flight, so
 * the pane opens on real numbers. Sections the gateway declines are named by
 * `ActivityGaps` rather than drawn as zeros, and a section still in flight draws
 * a skeleton rather than either (AG-576). It answers a different question from
 * the verdict sweep: the sweep establishes that a route is live, the endpoint
 * reports what was sent through it.
 *
 * Still inert: per-app metrics, whose own endpoint is AG-574's work, and the
 * Gate model catalogue, which the picker draws empty. Disconnect and reset wait
 * on a first-run screen to return to.
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
  // Held as text rather than a boolean: the report is a snapshot, and the copy
  // button has to hand over exactly what the dialog showed.
  const [diagnosticsReport, setDiagnosticsReport] = useState<string | null>(null);
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
  const platform = usePlatform();
  // Which installation the Overview covers; `null` is the whole org, and stays
  // the default because traffic sent before attribution existed has no
  // installation at all. Selecting one refetches - the gateway narrows every
  // section server-side, so there is nothing to slice here.
  const [installId, setInstallId] = useState<string | null>(null);
  // Which account the reading belongs to. Changing it refetches: numbers read for
  // one org must not sit on screen under another org's name, and an OAuth account
  // can switch org without the window remounting.
  const credential = account
    ? `${account.auth_mode}|${account.gateway_base_url}|${account.org_id ?? ""}`
    : "";
  // One fetch per account, plus the pane's own refresh. Not polled: the endpoint's
  // throttle bucket is keyed on the source address, so a timer here would spend
  // a budget shared with every other Gate Connect user on the same network.
  //
  // Held until the first account read lands and finds a credential. Before that
  // there is nothing to authenticate with, so a fetch could only fail, and the
  // pane would open on a "signed out" banner that is about to be wrong.
  const canRead = loaded && account !== null;
  const activity = useActivity(canRead, installId, credential);
  const { installations, current: currentInstallId } = useInstallations(canRead, credential);

  // A machine id belongs to the org it sent traffic to, so a scope selected
  // before an org switch cannot be honoured after it.
  useEffect(() => {
    setInstallId(null);
  }, [credential]);

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

  /** The routing sweep, kept separate from {@link refresh} because it is the one
   * probe that costs network I/O and a process walk. Callers that changed a
   * tool's config re-run it; callers that only repainted do not have to. */
  const refreshVerdicts = useCallback(async () => {
    const v = await routingVerdicts().catch(() => null);
    if (v) setVerdicts(verdictsBySlug(v));
  }, []);

  const refresh = useCallback(async () => {
    const [t, px] = await Promise.all([
      listTools().catch(() => null),
      proxyStatus().catch(() => null),
    ]);
    if (t) setTools(t);
    if (px) setProxy(px);
    // The engine coming up or going down changes every verdict, since the relay
    // health check is shared - so this follows the snapshot rather than waiting
    // for the next poll.
    void refreshVerdicts();
  }, [refreshVerdicts]);

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
        listTools().catch(() => [] as Tool[]),
        listProviders().catch(() => [] as ProviderState[]),
        proxyStatus().catch(() => null),
        getAccount().catch(() => null),
        oauthStatus().catch(() => null),
        getVersion().catch(() => ""),
      ]);
      void loadLaunchAtLogin();
      void loadPreferences();
      setTools(t);
      void refreshVerdicts();
      setProviders(p);
      setProxy(px);
      setAccount(acct);
      setOAuth(oauthState);
      setVersion(v);
      setLoaded(true);
    })();
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
    onError: (e) => {
      // `connect` covers both directions: the remedy copy is the same either way
      // for a failed tool write. A partial family failure keeps the remedy but
      // replaces the title, because naming who failed is the whole point.
      const classified = classifyError(e, "connect");
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

  // Re-read the routing facts the notices are built from. Their whole point is
  // that they disappear once acted on, which only works if the state behind them
  // is refetched rather than assumed.
  const refreshRouting = useCallback(async () => {
    const [t, px] = await Promise.all([
      listTools().catch(() => tools),
      proxyStatus().catch(() => proxy),
    ]);
    setTools(t);
    setProxy(px);
  }, [tools, proxy]);

  const [dismissedNotices, setDismissedNotices] = useState<string[]>([]);
  const [noticePage, setNoticePage] = useState(0);
  const [noticeBusy, setNoticeBusy] = useState(false);

  const notices = useMemo(
    () => buildNotices(groups).filter((n) => !dismissedNotices.includes(n.id)),
    [groups, dismissedNotices],
  );
  // Clamped rather than reset when the list shrinks: fixing the tool on the last
  // page removes its notice, and a page index left pointing past the end would
  // blank the banner while notices remain.
  const notice = notices.length > 0 ? notices[Math.min(noticePage, notices.length - 1)] : null;

  /** Perform a notice's action, then re-read state so it clears itself. */
  const runNoticeAction = useCallback(
    async (action: NoticeAction) => {
      if (noticeBusy) return;
      // Re-adopting goes through `routeApp`, not a bare `connectTool`: it
      // overwrites a config somebody hand-wrote, which the design gates behind
      // the review dialog, it may need the certificate first, and the app it
      // rewrote may be open on the old route. `useRouting` owns the first two
      // gates and `routeApp` adds the third, so a notice and a sidebar switch
      // leave the machine in the same state.
      setNoticeBusy(true);
      try {
        if (action.kind === "reconnect") await routeApp(action.slug, true);
        else if (action.kind === "enable-routing") await proxyEnable();
        else await proxyTrustCa();
      } catch {
        // Swallowed on purpose for now: the shell has nowhere to render a
        // failure yet, and the notice staying put is itself the signal that
        // nothing changed. Wire this to the error surface when one exists.
      } finally {
        await refreshRouting();
        setNoticeBusy(false);
      }
    },
    [noticeBusy, refreshRouting, routeApp],
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
  });

  const settings = useSettingsActions({
    account,
    proxyRunning: proxy?.running ?? false,
    launchAtLogin,
    onLaunchAtLogin: ({ enabled }) => setLaunchAtLogin(enabled),
    onAccount: setAccount,
    onSession,
    onProxy: setProxy,
    onError: (e) => setActionError(classifyError(e, "generic")),
  });

  // Diagnostics has two entrances - Settings, and the "something is missing"
  // banner on Overview - and both open the same rendered report rather than a
  // dialog that fetches its own, so the two can never disagree.
  const showDiagnostics = useCallback(() => {
    setDiagnosticsReport(
      previewDiagnostics({ now: new Date(), version, platform, account, proxy, providers, tools }),
    );
  }, [version, platform, account, proxy, providers, tools]);

  const settingsSections = useMemo(
    () =>
      buildSettingsSections({
        // Device name and plan have no backend yet, so they read as unknown
        // rather than as invented values. The install id now has one: it is the
        // id this app stamps on every routed request, reported back by the
        // gateway, so the row shows the identity the user's traffic actually
        // carries rather than a local guess at it.
        deviceName: "-",
        installId: currentInstallId ?? "-",
        loginId: account?.org_name ?? "-",
        plan: "-",
        gateway: account?.gateway_base_url ?? "-",
        apiKeyMasked: account?.has_api_key ? `sk-gw${"*".repeat(20)}` : "Not set",
        launchAtLogin,
        launchAtLoginUnavailable,
        routingHealthNotifications: prefs?.routing_health_notifications,
        shareDiagnostics: prefs?.share_diagnostics,
        preferencesUnavailable: prefsUnavailable,
        version: version ? `v${version}` : "-",
        updateNote: updateNoteFor(update),
        onCopyInstallId: currentInstallId ? () => void settings.copyText(currentInstallId) : noop,
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
        // The rendered report, not a fresh one: Overview's "something is missing"
        // banner opens the same `showDiagnostics`, and two builders could
        // disagree about what the machine looked like.
        onViewDiagnostics: showDiagnostics,
        // Deliberately absent, so the control is absent too: rename device,
        // notifications and plan upgrade have no backend command at all, update
        // checks belong with the update banner, and disconnect and reset both
        // end with no account - which needs a first-run screen to return to
        // that this shell does not have yet.
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
      currentInstallId,
      showDiagnostics,
      settings.copyText,
      settings.openReplaceKey,
      settings.openDisconnect,
      settings.openReset,
      settings.toggleLaunchAtLogin,
      update,
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
    else if (action === "docs") void openExternal(GATE_DOCS_URL);
    else void openExternal(GATE_SUPPORT_URL);
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
            apiKeyOpen={setup.apiKeyOpen}
            onToggleApiKey={setup.toggleApiKey}
            apiKey={setup.apiKey}
            onApiKeyChange={setup.setApiKey}
            onConnectWithApiKey={() => void setup.connectWithApiKey()}
            busy={setup.busy}
            error={setupError && <SetupNote error={setupError} />}
          />
        ) : stage.kind === "org-picker" ? (
          <OrgPickerPane
            organizations={(setup.orgs ?? []).map(toSetupOrg)}
            selectedId={setup.selectedOrgId}
            onSelect={setup.selectOrg}
            onContinue={() => void setup.confirmOrg()}
            onUseApiKey={setup.useApiKeyInstead}
            busy={setup.busy}
            error={setupError && <SetupNote error={setupError} />}
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
      // An API-key account holds no org locally, so the gateway's answer is the
      // only name it can show. Account first: it is what the user picked.
      orgName={account?.org_name ?? activity.view?.orgName ?? "No organization"}
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
      notice={
        actionError ? (
          <ErrorBanner
            title={actionError.title}
            hint={actionError.hint}
            onDismiss={() => setActionError(null)}
          />
        ) : undefined
      }
      onToggleApp={(slug, next) => void routeApp(slug, next)}
      dialog={
        routing.prompt?.kind === "drift" ? (
          <ReviewConfigDialog
            app={{ name: routing.prompt.name }}
            existingConfig={routing.prompt.existingConfig}
            // The relay is what a config-routed tool gets pointed at. Null
            // before a port has been bound, and the dialog omits the row rather
            // than inventing an address.
            gateRoute={proxy?.relay_base_url}
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
              machine and is removed when you reset Gate Connect.
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
        ) : diagnosticsReport !== null ? (
          <DiagnosticsDialog
            report={diagnosticsReport}
            copied={settings.copied}
            onCopy={() => void settings.copyText(diagnosticsReport)}
            onClose={() => setDiagnosticsReport(null)}
          />
        ) : settings.prompt?.kind === "replace-key" ? (
          <ReplaceApiKeyDialog
            currentKeyMasked={
              account?.has_api_key ? `sk-gw${"*".repeat(20)}` : "Not set"
            }
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
        ) : undefined
      }
    >
      {view.kind === "settings" ? (
        <SettingsPane sections={settingsSections} />
      ) : view.kind === "families" ? (
        <FamiliesPane
          families={families}
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
          isProtected={appFor(apps, view.slug)?.status.kind === "protected"}
          onToggleProtected={noop}
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
          onAddCredits={noop}
          activity={[]}
          // Not "this app sent nothing" - "nobody has asked". The per-app
          // reading is AG-574's endpoint and does not exist yet, so the cards
          // say they have no reading rather than reporting an app the user has
          // been working in all morning as idle.
          unavailable
          alert={driftAlert}
        />
      ) : (
        <Overview
          stats={activity.view?.stats ?? EMPTY_STATS}
          buckets={activity.view?.buckets ?? []}
          policies={activity.view?.policies ?? []}
          savings={activity.view?.savings ?? []}
          onManagePolicies={() => void openExternal(GATE_POLICIES_URL)}
          onManageSavings={() => void openExternal(GATE_SAVINGS_URL)}
          // Skeletons until there is something real to draw: a zero is a
          // reading and would claim the user had no traffic, and a dash says we
          // asked and were refused. Neither is true while the answer is on its
          // way. A held reading from the cache clears this on the first frame,
          // so the placeholders are only ever seen by an account that has none.
          pending={activity.view === null && activity.failure === null}
          // With no view at all - loading, or a failure with nothing held -
          // every section is unread, which is what the fallback says. Once
          // there is one, it names its own gaps.
          unavailable={activity.view?.missing ?? ALL_MISSING}
          period={activity.view?.period ?? "Last 24 hours"}
          scope={
            <InstallationPicker
              installations={installations}
              // The scope the gateway echoed, not the one we asked for: while a
              // refetch is in flight the numbers on screen are still the
              // previous scope's, and the label has to agree with them.
              value={activity.view?.installId ?? null}
              onChange={setInstallId}
            />
          }
          alert={
            <>
              {notice && (
                <AlertBanner
                  // Keyed so switching pages remounts rather than animating one
                  // card's text into another's.
                  key={notice.id}
                  title={notice.title}
                  body={notice.body}
                  switchLabel={notice.switchLabel}
                  // The switch reflects the state being fixed, which is always
                  // "not routing". Toggling it performs the action.
                  on={false}
                  // Both flags, because either path writes: `routingBusy` covers
                  // the reconnect that goes through `useRouting`, `noticeBusy`
                  // the two whole-machine actions that do not.
                  busy={noticeBusy || routingBusy}
                  onToggle={() => void runNoticeAction(notice.action)}
                  onDismiss={() => setDismissedNotices((d) => [...d, notice.id])}
                  paging={
                    notices.length > 1
                      ? {
                          onPrev: () =>
                            setNoticePage((p) => (p - 1 + notices.length) % notices.length),
                          onNext: () => setNoticePage((p) => (p + 1) % notices.length),
                        }
                      : undefined
                  }
                />
              )}
              <ActivityGaps
                view={activity.view}
                failure={activity.failure}
                loading={activity.loading}
                onRetry={activity.reload}
                onDiagnostics={showDiagnostics}
              />
            </>
          }
        />
      )}
    </AppShell>
  );
}

/** Before a reading lands, no section has one. Kept out of the render so the
 *  object identity is stable and the pane does not repaint for it. */
const ALL_MISSING = { chart: true, policies: true, savings: true };

/** No gateway endpoint reports the models on offer yet. See the picker. */
const GATE_MODELS: GateModelOption[] = [];

/** Shown before the first load lands, and on the per-app pane whose own reading
 *  is AG-574's work. All null rather than zeros: the tiles render a dash for
 *  null, and a zero here would be a claim about traffic nobody has measured yet.
 *  See `UsageStats`. */
const EMPTY_STATS: UsageStats = {
  messages: null,
  blockedFlagged: null,
  tokensSavedPercent: null,
  tokensSavedAmount: null,
};

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

/**
 * The same report the popover builds, minus the probes this preview does not
 * run: the backend snapshot, the OAuth bundle and the running-agent scan all
 * arrive as null, which `buildDiagnosticsReport` already renders as unknown.
 */
function previewDiagnostics(args: {
  now: Date;
  version: string;
  platform: Platform;
  account: Account | null;
  proxy: ProxyState | null;
  providers: ProviderState[];
  tools: Tool[];
}): string {
  return buildDiagnosticsReport({
    now: args.now,
    version: args.version,
    platform: args.platform,
    analyticsId: analyticsId(),
    backend: null,
    account: args.account,
    oauth: null,
    proxy: args.proxy,
    providers: args.providers,
    tools: args.tools,
    launchAtLogin: null,
    clientsStale: false,
    agents: null,
  });
}

/** What each `unavailable` cause means, and what the user can do about it.
 *
 * AG-576 asks an unavailable metric to name its cause and offer a matching
 * action rather than blanking the surface. The taxonomy and copy live in
 * `lib/activityGaps.ts`; this renders it and dispatches the actions.
 *
 * Deliberately plain text and text buttons in the pane's alert slot rather than a
 * designed component: the visual treatment for this state is AG-575's job and
 * still does not exist in the Figma (checked 2026-08-17 - neither the Overview
 * page nor the Components page has an unavailable, stale, empty or loading
 * state). Inventing one would be the "dressing scaffolding up as product"
 * mistake. What matters now is that a zero is never mistaken for a real reading,
 * and that every named cause comes with something the user can actually do. */
function ActivityGaps({
  view,
  failure,
  loading,
  onRetry,
  onDiagnostics,
}: {
  view: ActivityView | null;
  failure: ActivityFailure | null;
  loading: boolean;
  onRetry: () => void;
  onDiagnostics: () => void;
}) {
  const run = (kind: GapActionKind) => {
    if (kind === "retry") onRetry();
    else if (kind === "diagnostics") onDiagnostics();
    else if (kind === "dashboard") void openExternal(GATE_DASHBOARD_URL);
    else if (kind === "api-keys") void openExternal(GATE_API_KEYS_URL);
    else void openExternal(GATE_DOCS_URL);
  };

  // A failed fetch outranks per-section gaps: if nothing landed there is nothing
  // to itemise, and the sections listed in the held view describe the *previous*
  // reading, not this one.
  const notices = failure
    ? [failureNotice(failure)]
    : (view?.gaps ?? []).map((g) => sectionNotice(g.section, g.reason));
  if (notices.length === 0) return null;

  return (
    <div className="flex flex-col gap-2">
      {/* No separate staleness disclosure. It was a second sentence saying what
          the period label beside the header already says - "updated 14:03", with
          the date in front of it when the reading is not from today - and the
          product call (2026-08-18) was that a held reading is a feature rather
          than a warning: what the user wants on screen is the last thing that
          actually happened to their traffic. The notices below still name the
          cause and offer the action, which is the part that is actionable. */}
      {notices.map((n) => (
        <p key={n.subject} className="text-base-xs text-base-muted-foreground">
          <span className="font-medium">{n.subject}:</span> {n.cause}
          {n.actions.map((a) => (
            <button
              key={a.kind}
              type="button"
              onClick={() => run(a.kind)}
              disabled={a.kind === "retry" && loading}
              className="ml-2 rounded-base font-medium text-base-primary underline decoration-transparent underline-offset-2 transition hover:decoration-inherit focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary disabled:text-base-muted-foreground"
            >
              {a.kind === "retry" && loading ? "Trying…" : a.label}
            </button>
          ))}
        </p>
      ))}
    </div>
  );
}
