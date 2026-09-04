import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { SecurityEventDialog, SecurityPane } from "./components/gc/SecurityPane";
import { useSecurityFeed } from "./lib/securityFeed";
import type { SecurityEvent } from "./lib/api";
import type {
  Account,
  OAuthStatus,
  Org,
  Preferences,
  ProxyState,
  ProviderState,
  PendingRestore,
  RecoverySummary,
  TeardownReport,
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
  proxyEnable,
  proxyStatus,
  proxyTrustCa,
  routedClientsStale,
  routingVerdicts,
  runningAgents as fetchRunningAgents,
  pendingQuitTools,
  disconnectToolsForQuit,
  quitApp,
  pendingRestore,
  resumeRestore,
  recoverySummary,
  retryRestoreEntry,
  teardownReport,
  getPreferences,
  setBlockedEventNotifications,
  setFlaggedEventNotifications,
  setSecurityNotificationSound,
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
import { MULTI_PROVIDER_ID, buildGroups } from "./lib/groups";
import { proxyMemberStatus, verdictStatus, verdictsBySlug } from "./lib/verdict";
import { recoveryRows, unresolved } from "./lib/recovery";
import type { Group } from "./lib/groups";
import { openExternal } from "./lib/openExternal";
import {
  GATEWAY_SERVERS,
  GATE_API_KEYS_URL,
  GATE_DASHBOARD_URL,
  GATE_DOCS_URL,
  GATE_POLICIES_URL,
  GATE_SAVINGS_URL,
  GATE_SUPPORT_URL,
} from "./lib/config";
import { hasSeenTour, markTourSeen } from "./lib/tour";
import { hasSeenOAuthOffer, markOAuthOfferSeen } from "./lib/oauthOffer";
import { TOUR_SEEN_EVENT } from "./screens/Onboarding";
import { AppShell } from "./components/gc/AppShell";
import { brandMarkFor } from "./components/gc/BrandMark";
import { AppPane } from "./components/gc/AppPane";
import type { ModelChoice } from "./components/gc/AppPane";
import { Overview } from "./components/gc/Overview";
import type { UsageStats } from "./components/gc/metrics";
import { InstallationPicker } from "./components/gc/InstallationPicker";
import { useActivity, useInstallations } from "./lib/activity";
import { formatCredits, useCredits, useGateModels, useToolModels } from "./lib/toolModels";
import { modelAttention } from "./lib/modelAttention";
import { useToolEvents } from "./lib/toolEvents";
import { buildNotices } from "./lib/notices";
import type { NoticeAction } from "./lib/notices";
import type { ActivityFailure, ActivityView } from "./lib/activity";
import { failureNotice, mergeNotices, sectionNotice } from "./lib/activityGaps";
import type { GapActionKind } from "./lib/activityGaps";
import {
  SettingsPane,
  buildSettingsSections,
} from "./components/gc/SettingsPane";
import type { DialogOrganization } from "./components/gc/dialogs";
import {
  ApplyChangesDialog,
  ChangeReadyDialog,
  CloseAppsDialog,
  ModelPickerDialog,
  QuitDialog,
  QuitLeftBehindDialog,
  QuitSafeToCloseDialog,
  UseGateModelDialog,
} from "./components/gc/dialogs";
import type { QuitChoice } from "./components/gc/dialogs";
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
  TeardownReportDialog,
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
import {
  AlertBanner,
  ErrorBanner,
  ErrorDetails,
  RecoveryBanner,
  ReopenAlert,
} from "./components/gc/banners";
import { Modal } from "./components/gc/Modal";
import type {
  InventoryState,
  SidebarApp,
  SidebarGroup,
  SidebarView,
} from "./components/gc/Sidebar";
import type { TopnavAction } from "./components/gc/Topbar";
import { buildDiagnosticsReport } from "./lib/diagnosticsReport";
import {
  analyticsId,
  setAnalyticsConsent,
  track,
  trackError,
} from "./lib/analytics";
import { secretStoreName, trustPromptHint, usePlatform } from "./lib/platform";

/** A whole reading, for deciding whether a re-read changed anything. Compared by
 * value because the identity never matches: every read builds fresh objects. */
function detectionSignature(reading: unknown): string {
  return JSON.stringify(reading);
}

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
  const [launchAtLoginUnavailable, setLaunchAtLoginUnavailable] =
    useState(false);
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
  const [scan, setScan] = useState<
    { kind: "ok"; at: Date } | { kind: "failed" } | null
  >(null);
  /**
   * What detection last put on screen, so a re-read can tell a changed machine
   * from an unchanged one and leave the unchanged one entirely alone.
   *
   * Written from an effect on the state itself rather than by `redetect`, because
   * that is not the only writer: a toggle re-reads both through `useRouting`,
   * and a ref updated in only one of those places would report a change that had
   * already been drawn.
   */
  const rendered = useRef({ tools: "", proxy: "" });
  /**
   * Whether a re-read is still in flight, so the next one drops itself rather
   * than stacking on the one before it.
   *
   * `tools-changed` fires when the filesystem moves, not when the previous read
   * finished - a burst of writes is debounced but not serialised against us - and
   * `redetect` is not always cheap: on Windows `proxy_status` shells out to
   * `certutil` for the CA-trust reading. On a host where that call hangs until
   * it is killed, every tick used to add another one, so the machine ended up
   * hosting a growing pile of doomed children. Dropping the tick is right rather
   * than queueing it: the next one is only seconds away, and it wants a fresh
   * reading, not this stale one's turn.
   */
  const redetecting = useRef(false);
  /** The read-only "what is collected" list. Separate from the report dialog:
   * that one shows this install's values, this one shows what leaves the device. */
  const [collectedDataOpen, setCollectedDataOpen] = useState(false);
  // Held as text rather than a boolean: the report is a snapshot, and the copy
  // button has to hand over exactly what the dialog showed.
  const [diagnosticsReport, setDiagnosticsReport] = useState<string | null>(
    null,
  );
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
   * The two overlays that change an app's model, and what each is for.
   *
   * The choice itself is no longer here: it lives in the org's preferences on the
   * gateway (AG-588), read by `useToolModels` below. What is local is only which
   * dialog is on screen and why it was opened.
   *
   * The picker carries `then`, because "choose a model" means two different
   * things depending on how it was reached. Opened from the Gate model radio it
   * is a step in switching, and the choice has to continue into the billing
   * confirmation; opened from Change model while on App default it is a browse,
   * and the choice is remembered without spending anything. Collapsing them
   * would either bill a user who only looked, or silently drop the switch they
   * asked for.
   */
  const [modelOverlay, setModelOverlay] = useState<
    | { kind: "picker"; then: "activate" | "remember" }
    | { kind: "confirm-gate"; modelIds: string[] }
    | null
  >(null);
  /** A model write is in flight. Keyed by nothing: only one pane is open. */
  const [modelBusy, setModelBusy] = useState(false);
  /** The last model write that failed, in the gateway's own words.
   *
   *  Its message rather than a code: the refusals on this path are written to be
   *  read by a person ("Your role can view this organization's model settings but
   *  not change them"), and no code we could branch on carries which rule
   *  refused. */
  const [modelError, setModelError] = useState<string | null>(null);
  /**
   * A quit the tray deferred to this window, and where it has got to.
   *
   * One union rather than three flags, because the stages are mutually
   * exclusive and the drawn flow moves between them: `choose` offers the two
   * outcomes (`quit-requested` carries the config-routed tools still pointed at
   * Gate), `confirm` reports what the chosen one did and holds the button that
   * actually exits, and `left-behind` is the failure branch AG-596 requires -
   * tools a teardown could not put back, named rather than quietly exited past.
   *
   * The names are swept from a backend buffer (at mount, then on each nudge)
   * rather than carried on the event, so a Quit clicked before this listener
   * registered is not lost - the same reasoning as `App.tsx`.
   */
  const [quit, setQuit] = useState<
    | { kind: "choose"; tools: string[]; choice: QuitChoice }
    | { kind: "confirm"; disconnected: boolean }
    | { kind: "left-behind"; tools: string[] }
    | null
  >(null);
  const [quitBusy, setQuitBusy] = useState(false);
  const platform = usePlatform();
  // Which installation the Overview's *filter* covers; `null` is the whole org,
  // and stays the default because traffic sent before attribution existed has no
  // installation at all. Selecting one refetches - the gateway narrows every
  // section server-side, so there is nothing to slice here.
  //
  // Named for the filter rather than the id on purpose: `installId` above is this
  // machine's own identity, which is a different fact. The two were briefly the
  // same name and the compiler caught it.
  const [installFilter, setInstallFilter] = useState<string | null>(null);
  // Which account the reading belongs to. Changing it refetches: numbers read for
  // one org must not sit on screen under another org's name, and an OAuth account
  // can switch org without the window remounting.
  //
  // The key prefix is in here for the api-key case, where `org_id` is always
  // absent: the org is whatever the gateway resolves the key to, so replacing a
  // key with one for a different org changed nothing in this string and nothing
  // refetched. Org A's figures and org name stayed on screen under org B's
  // credential until the window was reopened - and reopening painted them again
  // off disk, because the cache scope had the same gap. `keyPrefix` is read back
  // from the account file after every save, so it changes exactly when the key
  // does. Mirrors the scope in `activity_cache.rs`; the two must agree.
  const credential = account
    ? `${account.auth_mode}|${account.gateway_base_url}|${account.org_id ?? ""}|${keyPrefix ?? ""}`
    : "";
  // The live security-event feed (AG-578). Keyed on `credential` for the reason
  // every other reading here is: events belonging to the org the user just left
  // must not stay on screen under the new org's name.
  //
  // Not gated on routing. The events come from the gateway, not from local
  // traffic, and AC4 requires the feed's state to be independent of routing's -
  // a feed that only ran while routing was on would be reporting routing.
  const securityFeed = useSecurityFeed(Boolean(credential), credential);

  /** The event whose summary is open, or null.
   *
   * Held here rather than in the pane because AC7 turns on it *surviving* the
   * click that opens the dashboard: the summary stays up until the browser has
   * it, so a failed open leaves the user looking at the event rather than at
   * nothing. */
  const [openEvent, setOpenEvent] = useState<SecurityEvent | null>(null);

  // One fetch per account, plus the pane's own refresh. Not polled: the endpoint's
  // throttle bucket is keyed on the source address, so a timer here would spend
  // a budget shared with every other Gate Connect user on the same network.
  //
  // Held until the first account read lands and finds a credential. Before that
  // there is nothing to authenticate with, so a fetch could only fail, and the
  // pane would open on a "signed out" banner that is about to be wrong.
  const canRead = loaded && account !== null;
  const activity = useActivity(canRead, installFilter, credential);
  const {
    installations,
    current: currentInstallId,
    resolved: installsResolved,
  } = useInstallations(canRead, credential);
  /** The open pane belongs to a proxy domain rather than a config tool. The
   *  gateway attributes requests to config tools only - `client_tool` is
   *  derived from each tool's own user agent, and traffic from these surfaces
   *  arrives unattributed on purpose, because a guessed slug would file one
   *  app's traffic under another's name. So the per-tool reads below must not
   *  fire for a domain: filtering by its slug would return an empty reading,
   *  and the pane would report a quiet day over traffic it cannot see. A slug
   *  carried by an installed tool stays a tool. */
  const openDomain =
    view.kind === "app" &&
    !tools.some((t) => t.slug === view.slug) &&
    (proxy?.domains.some((d) => d.slug === view.slug) ?? false);
  /** The tool whose pane is open, or null on any other view. Drives both per-tool
   *  reads below, and gating on it keeps them from firing for a pane nobody is
   *  looking at - this endpoint shares an address-keyed rate limit with every
   *  other control-plane route. */
  const openTool = view.kind === "app" && !openDomain ? view.slug : null;
  /**
   * Whether the gateway has told us which installation this machine is.
   *
   * The app pane is scoped to *this machine*, and `installId: null` does not mean
   * that - it means org-wide, which drops the query parameter entirely. So a null
   * id must stop the read rather than widen it: otherwise the pane paints the
   * whole org's traffic under a heading that says one machine, and does it exactly
   * when this machine is unattributed, which is the case the pane exists to
   * explain. It is self-concealing too - the org-wide read succeeds, so nothing
   * is pending and no gap notice fires - so `unattributedMachine` below says it
   * out loud instead.
   */
  const machineKnown = installsResolved && currentInstallId !== null;
  /** The gateway answered and does not recognise this machine. Distinct from "not
   *  asked yet", which is why `useInstallations` reports `resolved`. */
  const unattributedMachine = installsResolved && currentInstallId === null;
  const toolActivity = useActivity(
    canRead && openTool !== null && machineKnown,
    currentInstallId,
    credential,
    openTool ?? undefined,
  );
  /** The org's per-tool model preferences (AG-588). One read for the whole
   *  sidebar: the preference is org-wide, so asking per pane would repeat the
   *  same question. */
  const toolModels = useToolModels(canRead, credential);
  /** This app's stored choice, or undefined when it has never been set - which
   *  is not a gap but the true default: the tool picks its own model. */
  const openPref = openTool ? toolModels.view?.byTool.get(openTool) : undefined;
  /**
   * The catalogue, read when the picker needs it OR when a tool is running on a
   * Gate model.
   *
   * The second case is AG-592: the catalogue is the definition of "available",
   * so checking whether a chosen model still exists means having it. It is a few
   * hundred rows, which is why this is not read on every pane - only where a
   * selection could have gone stale.
   */
  const gateModels = useGateModels(
    modelOverlay?.kind === "picker" || (canRead && openPref?.source === "gate"),
  );
  /** The org's Gate credit balance, for the card and the billing confirmation.
   *  Read whenever an app pane is open - it is what a switch to a Gate model
   *  starts spending. */
  const credits = useCredits(canRead && openTool !== null, credential);

  /**
   * What the open app is set to, or null when we do not know.
   *
   * Null only while the local read is in flight or after it failed. Rare, since
   * it is a file read - but defaulting to "app" on a failure would be the
   * principle 2 bug: an install that had switched to a Gate model would show App
   * default selected, and clicking Gate model would read as a change when it was
   * a no-op.
   */
  const openModelChoice: ModelChoice | null =
    toolModels.view === null
      ? null
      : openPref?.source === "gate"
        ? "gate"
        : "app";
  /**
   * The remembered models, active or not.
   *
   * A list because AG-590 enables a set. The pane's "Current Gate model" row
   * shows the first and says how many more there are, which keeps the card the
   * height the Figma draws whether one model is enabled or six.
   */
  const openModelIds = openPref?.modelIds ?? [];
  /** The primary - what a single-model reading of the same state would show. */
  const openModelId = openModelIds[0] ?? null;

  const toolEvents = useToolEvents(
    canRead && openTool !== null && machineKnown,
    openTool,
    currentInstallId,
    credential,
  );
  // A write failure belongs to the pane it happened on. Without this, refusing a
  // change on Codex would keep saying so over Claude Code's pane, blaming the
  // wrong app for a refusal that had nothing to do with it.
  useEffect(() => {
    setModelError(null);
  }, [openTool, credential]);

  /**
   * Write one model choice, and surface anything that goes wrong in its own
   * words.
   *
   * A local file write, so the failures are things like a read-only home rather
   * than a policy refusal - and no code distinguishes them. The message is what
   * tells the reader whether to retry or to look at their disk.
   */
  const saveModel = useCallback(
    async (
      source: "tool" | "gate",
      modelIds: string[],
      acknowledgePaidUse = false,
    ) => {
      if (!openTool) return;
      setModelBusy(true);
      setModelError(null);
      const failure = await toolModels.save(
        openTool,
        source,
        modelIds,
        acknowledgePaidUse,
      );
      setModelBusy(false);
      if (failure) setModelError(failure.message);
    },
    [openTool, toolModels],
  );

  /**
   * Hand routing to Gate for a set of models, asking about billing first if this
   * install has never been asked.
   *
   * Per install now that the choice is local - the trade recorded in
   * `preferences.rs`. Empty sets are refused here rather than written: Gate
   * cannot serve a model nobody enabled, and AG-590 makes that a rule rather
   * than an accident.
   */
  const activateGateModel = useCallback(
    (modelIds: string[]) => {
      if (modelIds.length === 0) return;
      if (toolModels.view?.paidAckUnix) {
        void saveModel("gate", modelIds);
      } else {
        setModelOverlay({ kind: "confirm-gate", modelIds });
      }
    },
    [saveModel, toolModels.view?.paidAckUnix],
  );

  /**
   * The feed's rows, each with somewhere to go.
   *
   * `onView` is attached here rather than in the adapter because the adapter has
   * no business knowing where a request can be looked at - and for a whole round
   * it could not have known, since `dashboard-web` had no route to send anyone to.
   * It has one: `/messages/:requestId` opens the request's own detail, and
   * `ActivityEntry.id` *is* the request id.
   */
  const toolEventRows = useMemo(
    () =>
      (toolEvents.view?.entries ?? []).map((e) => ({
        ...e,
        onView: () =>
          openLink(`${GATE_DASHBOARD_URL}messages/${encodeURIComponent(e.id)}`),
      })),
    [toolEvents.view],
  );

  // A machine id belongs to the org it sent traffic to, so a filter selected
  // before an org switch cannot be honoured after it.
  useEffect(() => {
    setInstallFilter(null);
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
    const [p, s] = await Promise.all([
      pendingRestore().catch(() => null),
      recoverySummary().catch(() => null),
    ]);
    if (p) setPending(p);
    // Read alongside the pending state, not lazily on click, for two reasons: the
    // notice decides whether to offer Review details at all, and it can only do
    // that if it knows whether there is anything to review; and its per-tool rows
    // are part of the notice itself, so fetching them on expand would leave the
    // Show tools control claiming a count it has not read.
    setSummary(s);
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
    // for something else to ask.
    void refreshVerdicts();
    // A master-on runs `restore_all`, which is what clears or shortens the
    // snapshots - so the notice has to be re-read on the same event that
    // repaints the switches, or it lingers after the work finished.
    void loadPending();
  }, [refreshVerdicts, loadPending]);

  /**
   * Re-read what is installed, without the routing sweep.
   *
   * The cheap half of {@link refresh}, and what `tools-changed` and the
   * visibility edge both call. `list_tools` walks config files and `proxy_status`
   * reads state already in memory; the sweep is left out because it probes the
   * relay and the gateway, which is not something a filesystem event should be
   * able to trigger at whatever rate a package manager writes.
   *
   * A reading that matches what is on screen is dropped rather than re-set. Both
   * of these feed every memo below, and committing an equal-but-new object would
   * rebuild the families, the settings sections and the routing callbacks for no
   * change at all.
   */
  const redetect = useCallback(async () => {
    if (redetecting.current) return;
    redetecting.current = true;
    try {
      const [t, px] = await Promise.all([
        listTools().catch(() => null),
        proxyStatus().catch(() => null),
      ]);
      // Written on every read, not only on a change: this timestamp is the empty
      // card's evidence that something is still looking, and letting it go stale
      // while the reads continued would misdate a scan that did happen.
      setScan(t ? { kind: "ok", at: new Date() } : { kind: "failed" });
      let changed = false;
      // A failed read commits nothing, so it also reports nothing as changed -
      // the last good list stays on screen and the card says the scan failed.
      if (t && detectionSignature(t) !== rendered.current.tools) {
        setTools(t);
        changed = true;
      }
      if (px && detectionSignature(px) !== rendered.current.proxy) {
        setProxy(px);
        changed = true;
      }
      // Either one moving invalidates every verdict: a tool that just appeared
      // has none yet, and the engine coming up or going down changes all of
      // them, because the relay health check behind them is shared.
      if (changed) void refreshVerdicts();
    } finally {
      redetecting.current = false;
    }
  }, [refreshVerdicts]);

  /** Re-run detection because the user asked - the inventory card's control, for
   * a scan that failed and may not fail again. Same reads as the event-driven
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
    rendered.current = {
      tools: detectionSignature(tools),
      proxy: detectionSignature(proxy),
    };
  }, [tools, proxy]);

  // Detection used to be the one reading the window could not be told about, so
  // this polled `list_tools` every five seconds. It is told now: the backend
  // watches the tool config files and binaries and emits `tools-changed` when
  // one moves (`core/src/tool_watch.rs`), which is the same shape as
  // `proxy-state-changed` below and costs nothing while nothing happens.
  //
  // The visibility read stays, and is not a poll. It covers what a filesystem
  // watch cannot see - a launcher installed somewhere on `$PATH` has no
  // directory to arm - and what a watch that failed to start would miss
  // entirely.
  useEffect(() => {
    const unlisten = listen("tools-changed", () => {
      void redetect();
    });
    const onVisible = () => {
      if (!document.hidden) void redetect();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      void unlisten.then((off) => off()).catch(() => {});
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [redetect]);

  useEffect(() => {
    const sweep = () => {
      pendingQuitTools()
        .then((tools) => {
          // Only ever opens the chooser: a sweep landing mid-flow must not
          // throw the user back to step one of a quit they are already past.
          if (tools && tools.length > 0)
            setQuit((q) =>
              q ?? { kind: "choose", tools, choice: "disconnect" },
            );
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
      // Swallowed on purpose. Tauri's unregisterListener throws if the listener
      // is already gone, which happens routinely on teardown, and an uncaught
      // rejection here is reported by the global handler as an app ERROR in the
      // diagnostic log - a failure that never happened, sitting where someone
      // debugging a real one will read it first.
      void unlisten.then((off) => off()).catch(() => {});
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
  /** The whole state of the interrupted operation: what the write reached per
   * tool, what the last checks saw, and what each one still needs. Null when
   * there is nothing to recover, which is the normal case. */
  const [summary, setSummary] = useState<RecoverySummary | null>(null);
  const [detailsOpen, setDetailsOpen] = useState(false);
  /**
   * Which entry a resume is on, and what it has attempted this pass.
   *
   * The resume drives the entries one at a time (see {@link resumeNow}) so it can
   * answer "which tool is it working on", which a single spinner over the whole
   * set cannot. Cleared when the pass ends: the recorded stages take over from
   * there, and leaving "just attempted" on a row would make the next render claim
   * an attempt that belongs to a previous resume.
   */
  const [resumeProgress, setResumeProgress] = useState<{
    active: string | null;
    done: string[];
  } | null>(null);
  /** Where the tools stand after a teardown - routing off, sign-out or reset.
   * Null unless one just ran and left something outstanding. */
  const [teardown, setTeardown] = useState<TeardownReport | null>(null);

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

  /**
   * Put the tools back, and move to whichever step the result earns: the
   * confirmation when the teardown was clean, the left-behind dialog when it
   * was not. Shared by the chooser's primary and that dialog's Try again, which
   * are the same operation reached from two places.
   *
   * Quitting on a partial teardown would strand a config pointing at a relay
   * that dies with this process, and reporting "their previous settings are
   * restored" over it would be the claim AG-596 forbids.
   */
  const runDisconnect = useCallback(async () => {
    setQuitBusy(true);
    setActionError(null);
    try {
      const failed = await disconnectToolsForQuit();
      setQuit(
        failed.length > 0
          ? { kind: "left-behind", tools: failed }
          : { kind: "confirm", disconnected: true },
      );
    } catch (e) {
      setActionError(classifyError(e, "quit_disable"));
    } finally {
      setQuitBusy(false);
    }
  }, []);

  /**
   * Carry out the chosen way to quit, then report it - the drawn flow's step
   * one to step two. Neither branch exits here: the confirmation's own button
   * does that, which is what lets it speak in the past tense.
   */
  const continueQuit = useCallback(() => {
    if (quit?.kind !== "choose") return;
    if (quit.choice === "leave") {
      // Nothing to carry out - the choice is to touch nothing - so this is a
      // step rather than an operation.
      setQuit({ kind: "confirm", disconnected: false });
      return;
    }
    void runDisconnect();
  }, [quit, runDisconnect]);

  const finishQuit = useCallback(async () => {
    setQuitBusy(true);
    await quitApp().catch(() => {});
  }, []);

  const cancelQuit = useCallback(() => {
    setQuit(null);
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
      const engineContexts: ErrorContext[] = [
        "proxy_toggle",
        "env_export",
        "untrust_ca",
      ];
      const ctx = engineContexts.find((c) => c === context) ?? "connect";
      const classified = classifyError(e, ctx);
      setActionError(
        // Unreachable from this shell since the family switches came off the
        // rail on 2026-08-27: `setFamilyRouted` is the only thing that throws
        // this, and nothing here calls it any more. Kept because the popover
        // still cascades and this shell is the one that will get a family
        // control back if the design ever draws one.
        e instanceof FamilyCascadeError
          ? { ...classified, title: cascadeTitle(e) }
          : classified,
      );
    },
  });
  const routingBusy = routing.busy;

  /** Where the tools stand after a teardown, raised only when something is
   * actually outstanding: a clean routing-off has nothing to report, and a
   * dialog saying so would be a dialog about nothing. */
  const reportTeardown = useCallback(async () => {
    const report = await teardownReport().catch(() => null);
    if (!report) return;
    const outstanding =
      report.still_gate.length + report.awaiting_reopen.length + report.failed.length;
    if (outstanding > 0) setTeardown(report);
  }, []);

  const runningApps = useRunningApps({
    onError: (e) => setActionError(classifyError(e, "close_agents")),
  });

  /**
   * Open a link, and show it in the existing error banner if it fails.
   *
   * Ten call sites open external URLs, and a rejected `openUrl` used to be
   * invisible - an opener-ACL miss looked exactly like a dead button. Routed
   * into `actionError` rather than a new surface: that banner is dismissible and
   * navigates nowhere, which is what AG-598 asks for (the failure must not
   * discard what the user was doing).
   */
  const openLink = useCallback((url: string) => {
    void openExternal(url).then((err) => {
      if (err) setActionError(err);
    });
  }, []);

  /**
   * A tool's config was rewritten. If that app is open it is still on its old
   * route until it restarts, so offer to close it - but only when something was
   * actually written, which is why `setAppRouted` reports back.
   *
   * Scoped to `slug`, and it has to be: the probe used to ask about every tool,
   * so flipping Codex offered to close a running `claude` that nothing had
   * reconfigured, and the confirm behind that offer would have killed it.
   */
  const routeApp = useCallback(
    async (slug: string, next: boolean, force = false) => {
      setActionError(null);
      if (await routing.setAppRouted(slug, next, force)) {
        // Only this tool's processes: its route is the only one that moved.
        await runningApps.offerAfterChange([slug]);
      }
    },
    [routing, runningApps],
  );

  /**
   * Turn all routing on or off.
   *
   * Same follow-up as a config write: every routed tool is on its old route until
   * it restarts, so a master toggle that actually moved offers to close them.
   *
   * The one caller that genuinely means every tool, so it passes no filter.
   */
  const toggleMaster = useCallback(
    async (next: boolean) => {
      setActionError(null);
      if (await routing.setMasterRouted(next)) {
        await runningApps.offerAfterChange();
        // Routing off is a teardown: it sweeps every tool back to its own
        // settings and the sweep is best-effort per tool. AG-570 requires the
        // result to name what it could not put back, so the configs are read
        // back and anything outstanding is reported.
        if (!next) await reportTeardown();
      }
    },
    [routing, runningApps, reportTeardown],
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

  /**
   * The tools with no single model family, taken from `buildGroups`' own
   * "Other tools" membership rather than a slug list of our own - so the pane
   * and the rail can never disagree about which tools those are.
   *
   * Today that is OpenCode, OpenClaw and Hermes. They get no model card; see
   * `AppPane`'s `modelChoice`.
   */
  const multiProviderSlugs = useMemo(
    () =>
      new Set(
        groups
          .filter((g) => g.id === MULTI_PROVIDER_ID)
          .flatMap((g) => g.members.map((m) => m.key)),
      ),
    [groups],
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
  const notice =
    notices.length > 0
      ? notices[Math.min(noticePage, notices.length - 1)]
      : null;

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
        // A switch with an exhaustiveness check, not an `else` fallthrough. The
        // notice vocabulary is expected to grow with AG-576, and a fourth kind
        // arriving on the old shape would silently have trusted the certificate
        // instead of doing its own work. Now it is a build failure.
        switch (action.kind) {
          case "reconnect":
            await routeApp(action.slug, true);
            break;
          case "enable-routing":
            await proxyEnable();
            break;
          case "trust-certificate":
            await proxyTrustCa();
            break;
          default: {
            const unhandled: never = action;
            throw new Error(
              `unhandled notice action ${JSON.stringify(unhandled)}`,
            );
          }
        }
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
          logo: brandMarkFor(t.slug),
          busy: routingBusy,
        })),
    [tools, verdicts, routing.writeFailures, routingBusy],
  );

  /**
   * The rail's rows under their family eyebrows (`Components / Sidenav`, read
   * 2026-08-23). Labels are the drawn vendor captions - "Anthropic" over the
   * Claude apps, "OpenAI" over Codex - taken from each family's
   * `upstream_provider_name`, falling back to the family's own name for a
   * family with no config tool to read it from. The multi-provider tools sit
   * under one "Other tools" eyebrow, as drawn - the 2026-08-21 read had each
   * under its own name, and the Sidenav page reversed that. Proxy members
   * (the chat domains and a family's app surfaces) are rows too now: no
   * verdict covers them - the sweep is per tool - so their status derives
   * from the domain's own state. Before the catalog loads, one unlabelled
   * group keeps the rows on screen rather than blanking the rail on a
   * grouping that is not yet known.
   */
  const sidebarGroups = useMemo<SidebarGroup[]>(() => {
    if (groups.length === 0) {
      return apps.length > 0 ? [{ id: "all", label: "", apps }] : [];
    }
    const bySlug = new Map(apps.map((a) => [a.slug, a]));
    const grouped: SidebarGroup[] = [];
    for (const g of groups) {
      const members: SidebarApp[] = [];
      let vendor: string | null = null;
      for (const m of g.members) {
        if (m.kind === "config" && m.tool) {
          const app = bySlug.get(m.key);
          if (!app) continue;
          bySlug.delete(m.key);
          vendor ??= m.tool.upstream_provider_name;
          members.push(app);
        } else if (m.kind === "proxy") {
          members.push({
            slug: m.key,
            name: m.name,
            status: proxyMemberStatus(m),
            // Intent, same as the tools: the switch says what the user asked
            // for, the status line says what is happening.
            on: m.desired,
            logo: brandMarkFor(m.key),
            busy: routingBusy,
          });
        }
      }
      if (members.length === 0) continue;
      grouped.push({
        // "Other tools" names itself; its members' vendor field is a sentence
        // fragment ("your existing providers"), not a caption.
        id: g.id,
        label: g.id === MULTI_PROVIDER_ID ? g.name : (vendor ?? g.name),
        apps: members,
      });
    }
    // A row the catalog did not claim keeps its place rather than vanishing.
    // buildGroups sweeps leftovers into "Other tools", so this only catches a
    // tool list and a catalog momentarily out of step with each other.
    if (bySlug.size > 0) {
      grouped.push({ id: "unclaimed", label: "", apps: [...bySlug.values()] });
    }
    return grouped;
  }, [groups, apps, routingBusy]);

  /** Every rail row flat, tools and domains together, for the pane header's
   *  name and switch state - `apps` alone covers only the tools. */
  const railApps = useMemo(
    () => sidebarGroups.flatMap((g) => g.apps),
    [sidebarGroups],
  );

  /** Route or unroute one rail row. The rail mixes tools and proxy domains
   *  now: a domain routes through `setDomainRouted` - no config file, so no
   *  drift gate - the same dispatch the family panel's member switches use. */
  const toggleRailApp = useCallback(
    (slug: string, next: boolean) => {
      const member = groups
        .flatMap((g) => g.members)
        .find((m) => m.key === slug);
      void (member?.kind === "proxy"
        ? routing.setDomainRouted(slug, next)
        : routeApp(slug, next));
    },
    [groups, routing.setDomainRouted, routeApp],
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

  /**
   * Retry one recorded entry, leaving every other entry's recorded work alone.
   *
   * The failure is reported and the pending state is still taken from the reply:
   * a retry changes what is outstanding for the *other* entries too - one may
   * have completed in another window - so a caller that dropped the reply on
   * error would redraw the notice from a stale list.
   */
  const retryEntry = useCallback(async (slug: string): Promise<boolean> => {
    const { error, pending: left } = await retryRestoreEntry(slug);
    setPending(left);
    // Deliberately not raised as a banner. The error banner outranks the
    // recovery notice - a failure that just happened beats a recorded one - and
    // that ordering is right for a failure the notice cannot express, but wrong
    // for this one: it would replace the notice with a message, taking the retry
    // button away at the exact moment the user wants it again. The row reports
    // it instead, as a stage and a category, which is the vocabulary AG-570 asks
    // for anyway. The raw message still reaches the backend-error buffer, so it
    // is not lost to telemetry or to the next drain.
    return error === null;
  }, []);

  /**
   * Finish what the interrupted operation left, one entry at a time.
   *
   * Entry by entry rather than through `resume_restore`, and the reason is the
   * progress: AG-570 asks a resume to show progress for each tool, and a single
   * batch call can only report what is left when the whole thing is over.
   * `restore_one` has the batch's own semantics narrowed to one slug - a slug
   * leaves its snapshot only once it is actually back - so this repeats no
   * completed write and reopens no verified tool, exactly as before.
   *
   * The close-and-reopen offer follows, scoped to the entries that came back:
   * their configs were just rewritten, and a tool that was running through all
   * of it is still on the route it started with. Scoped, not global, for the
   * reason `runningAgents`'s own docs give - offering to close Claude when the
   * resume touched Codex names processes the change never went near.
   */
  const resumeNow = useCallback(async () => {
    const queue = summary ? unresolved(summary) : [];
    // Nothing to drive: fall back to the batch, which is also what a summary
    // that could not be read leaves us with.
    if (queue.length === 0) {
      setResuming(true);
      setActionError(null);
      try {
        setPending(await resumeRestore());
        await refresh();
      } catch (e) {
        setActionError(classifyError(e, "provider_restore"));
      } finally {
        setResuming(false);
      }
      return;
    }
    setResuming(true);
    setActionError(null);
    const done: string[] = [];
    const restored: string[] = [];
    try {
      for (const entry of queue) {
        // Only an unfinished write is this button's business. A stale process or
        // a missing account is a real answer with its own action on the row, and
        // retrying the write would not move either.
        if (entry.next_step !== "retry") continue;
        setResumeProgress({ active: entry.slug, done: [...done] });
        // A rejected command - not a recorded failure - is the case the notice
        // genuinely cannot express, so that one does reach the banner.
        const ok = await retryEntry(entry.slug).catch((e) => {
          setActionError(classifyError(e, "provider_restore"));
          return false;
        });
        done.push(entry.slug);
        if (ok) restored.push(entry.slug);
      }
      setResumeProgress({ active: null, done });
      await refresh();
      if (restored.length > 0) await runningApps.offerAfterChange(restored);
    } finally {
      setResuming(false);
      setResumeProgress(null);
    }
  }, [summary, retryEntry, refresh, runningApps]);

  /** One row's Retry, outside a whole-pass resume. */
  const retryOne = useCallback(
    async (slug: string) => {
      setResuming(true);
      setResumeProgress({ active: slug, done: [] });
      setActionError(null);
      try {
        const ok = await retryEntry(slug).catch((e) => {
          setActionError(classifyError(e, "provider_restore"));
          return false;
        });
        setResumeProgress({ active: null, done: [slug] });
        await refresh();
        if (ok) await runningApps.offerAfterChange([slug]);
      } finally {
        setResuming(false);
        setResumeProgress(null);
      }
    },
    [retryEntry, refresh, runningApps],
  );


  /**
   * What is still outstanding, providers and tools together: the user does not
   * care which snapshot an entry came from.
   *
   * The snapshots are not the whole answer. A tool whose write finished but whose
   * process predates it has left the snapshot and is *not* routing, and AG-570 is
   * explicit that the notice goes away only once each affected tool reaches a
   * verified result - so the summary's own unresolved set is unioned in. Names,
   * deduplicated: the two sources overlap by design.
   */
  const recoveryNames = useMemo(() => {
    const names = [
      ...(pending?.providers ?? []),
      ...(pending?.tools ?? []),
    ].map((e) => e.name);
    for (const tool of summary ? unresolved(summary) : []) {
      if (!names.includes(tool.name)) names.push(tool.name);
    }
    return names;
  }, [pending, summary]);

  /**
   * The notice's per-tool rows, against one clock.
   *
   * Recomputed when the summary changes rather than on a timer: the ages on these
   * rows are read while the user is looking at a notice they just opened, and a
   * ticking "4m ago" would be redrawing the whole list to keep a number honest
   * that nobody is watching. The review dialog takes its own `new Date()` for the
   * same reason - it is read once, on open.
   */
  const recoveryRowList = useMemo(
    () => (summary ? recoveryRows(summary, new Date()) : undefined),
    [summary],
  );

  const noop = useCallback(() => {}, []);

  const onSession = useCallback(
    ({
      account: a,
      oauth: o,
    }: {
      account: Account | null;
      oauth: OAuthStatus | null;
    }) => {
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
    // Sign-out and reset are the other two teardowns. Reset puts the tools back
    // and can fail doing it; sign-out deliberately leaves the configs alone,
    // which is exactly the case worth reporting - the session those configs
    // authenticate with has just ended.
    onTeardown: () => void reportTeardown(),
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

  /**
   * Settings' "Use a Gate account".
   *
   * `upgradeToOAuth` throws on a browser flow that fails, times out or is
   * abandoned, and the row called it through `void` - so the rejection became an
   * unhandled promise, the busy flag cleared in `finally`, and the pane went
   * quiet. That is indistinguishable from a button that does nothing, which is
   * exactly how it was reported. The offer dialog beside it has always caught;
   * this now does too, onto the shell's own error banner.
   */
  const switchToGateAccount = useCallback(async () => {
    setActionError(null);
    try {
      await settings.upgradeToOAuth();
    } catch (e) {
      trackError(e, "sign_in");
      setActionError(classifyError(e, "sign_in"));
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
        // Device name and plan have no backend yet, so they read as unknown
        // rather than as invented values.
        //
        // The install id stays the *local* one, not the gateway's echo. The echo
        // is only populated once this machine has sent attributed traffic, so a
        // fresh install, or a key with no user (where the installations route is
        // refused outright), would blank the row - and it is the row a support
        // request asks for, at the moment the user is most likely to be filing
        // one. The local id always exists. `x-gate-install-id` sends this same
        // value, so the two never disagree.
        deviceName: device ?? "Unavailable",
        deviceNamed: prefs ? prefs.device_name !== null : undefined,
        onRenameDevice: device
          ? () => settings.openRenameDevice(device)
          : undefined,
        installId: installId ?? "Unavailable",
        loginId: account?.org_name ?? "-",
        plan: "-",
        gateway: account?.gateway_base_url ?? "-",
        apiKeyMasked: maskedKey(keyPrefix, account?.has_api_key ?? false),
        // Decides whether the key row is drawn at all: an upgraded account still
        // has its old key in the keychain, so `has_api_key` alone would keep
        // naming a credential that no longer authenticates anything.
        authMode: account?.auth_mode,
        launchAtLogin,
        launchAtLoginUnavailable,
        routingHealthNotifications: prefs?.routing_health_notifications,
        blockedEventNotifications: prefs?.blocked_event_notifications,
        flaggedEventNotifications: prefs?.flagged_event_notifications,
        securityNotificationSound: prefs?.security_notification_sound,
        shareDiagnostics: prefs?.share_diagnostics,
        preferencesUnavailable: prefsUnavailable,
        version: version ? `v${version}` : "-",
        updateNote: updateNoteFor(update),
        // Absent on a platform with no proxy subsystem: there is no engine, so
        // there is no certificate to describe.
        certificate: proxy
          ? proxy.ca_trusted
            ? "Trusted"
            : "Not trusted"
          : undefined,
        // Only while it is actually trusted. Removing a certificate that is not
        // there is a button that cannot do anything.
        onRemoveCertificate: proxy?.ca_trusted
          ? () => void routing.untrustCa()
          : undefined,
        onChangeGateway: settings.openSwitchGateway,
        onCopyInstallId: installId
          ? () => void settings.copyText(installId)
          : noop,
        // Only where there is a key to replace. On an OAuth account `saveAccount`
        // with a key would flip auth_mode to api_key, quietly converting the
        // account behind a button that says "replace".
        onReplaceKey:
          account?.auth_mode === "api_key"
            ? settings.openReplaceKey
            : undefined,
        // Same gate as Replace key, and the counterpart to it: a key account can
        // move to a Gate account whenever it likes, not only in the one-time
        // offer it may already have dismissed. An OAuth account is not offered
        // the reverse, matching the popover.
        onSwitchToGateAccount:
          account?.auth_mode === "api_key"
            ? () => void switchToGateAccount()
            : undefined,
        // `oauthBusy`, not `busy`: only the OAuth upgrade opens a browser.
        signInNote: settings.oauthBusy
          ? "Finish signing in on the page that opened in your browser."
          : undefined,
        // Only where there is a session to end. An API-key account never had one;
        // reset is its way out.
        onDisconnect:
          account?.auth_mode === "oauth" ? settings.openDisconnect : undefined,
        onReviewReset: settings.openReset,
        onToggleLaunchAtLogin: () => void settings.toggleLaunchAtLogin(),
        onRetryLaunchAtLogin: () => void loadLaunchAtLogin(),
        // Optimistic then re-read: the switch has to move on click, and the
        // re-read is what makes a failed write show up rather than leaving the
        // UI asserting a value the file does not hold.
        onToggleRoutingHealthNotifications: () => {
          const next = !(prefs?.routing_health_notifications ?? true);
          setPrefs((p) =>
            p ? { ...p, routing_health_notifications: next } : p,
          );
          void setRoutingHealthNotifications(next)
            .catch((e) => setActionError(classifyError(e, "generic")))
            .finally(() => void loadPreferences());
        },
        // Same optimistic-then-re-read shape as the routing switch above: the
        // switch has to move on click, and the re-read is what surfaces a failed
        // write instead of leaving the UI asserting a value the file lacks.
        onToggleBlockedEventNotifications: () => {
          const next = !(prefs?.blocked_event_notifications ?? true);
          setPrefs((p) => (p ? { ...p, blocked_event_notifications: next } : p));
          void setBlockedEventNotifications(next)
            .catch((e) => setActionError(classifyError(e, "generic")))
            .finally(() => void loadPreferences());
        },
        onToggleFlaggedEventNotifications: () => {
          const next = !(prefs?.flagged_event_notifications ?? true);
          setPrefs((p) => (p ? { ...p, flagged_event_notifications: next } : p));
          void setFlaggedEventNotifications(next)
            .catch((e) => setActionError(classifyError(e, "generic")))
            .finally(() => void loadPreferences());
        },
        onToggleSecurityNotificationSound: () => {
          const next = !(prefs?.security_notification_sound ?? true);
          setPrefs((p) => (p ? { ...p, security_notification_sound: next } : p));
          void setSecurityNotificationSound(next)
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
        onOpenDocs: () => openLink(GATE_DOCS_URL),
        // No `onContactSupport`, so the row is omitted - and this is NOT the
        // same call as the topnav's. No Settings frame draws a Support row, so
        // there is nothing to match here; the menu entry is drawn in two
        // places and now ships despite `GATE_SUPPORT_URL` 404ing (see
        // `TopnavAction`). Draw one and this gets it too.

        // The tutorial is its own window, already built and wired.
        onReplayTutorial: () => void openOnboardingWindow("settings"),
        // Explicit, so this one reports back: silence on a button the user just
        // pressed reads as broken.
        onCheckForUpdates: () => void update.checkNow(true),
        onViewCollectedData: () => setCollectedDataOpen(true),
        // The rendered report, not a fresh one: Overview's "something is missing"
        // banner open the same `openDiagnostics`, so the two can never disagree
        // about what the machine looked like. It runs the live probes rather than
        // rendering a preview, which is why both entrances share it.
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
      settings.copyText,
      settings.openRenameDevice,
      settings.openReplaceKey,
      // `busy` drives the sign-in row's waiting note, so the memo has to see it
      // change or the note never appears.
      settings.busy,
      switchToGateAccount,
      settings.openDisconnect,
      settings.openReset,
      settings.openSwitchGateway,
      settings.toggleLaunchAtLogin,
      routing.untrustCa,
      openDiagnostics,
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

  const protectedCount = apps.filter(
    (a) => a.status.kind === "protected",
  ).length;

  /**
   * The open app's own notice, for its own pane.
   *
   * This was a drift-only card built from `drifted[0]` - the first drifted
   * tool ANYWHERE - and handed to whichever pane happened to be open, so with
   * Codex drifted and Claude Desktop's pane open, Claude Desktop drew a card
   * whose body named Codex.
   *
   * It also only ever fired for drift, while `notices.ts` already had the copy
   * for master-off, needs-trust and error. The one cause `Flows / App` draws
   * (`116:30663`, "Claude Desktop isn't protected / Routing is set to off") is
   * master-off, and it never appeared on the pane that names the app it is
   * about - only on Overview. Both halves are the same mistake: the pane was
   * not asking about itself.
   *
   * No paging: this card is one app's, and the drawn chevrons belong to the
   * multiple-apps variant on Overview.
   */
  const paneNotice = useMemo(
    () =>
      view.kind === "app"
        ? (notices.find((n) => n.memberKey === view.slug) ?? null)
        : null,
    [notices, view],
  );
  /**
   * The open app's reopen card, when its verdict says a process is holding older
   * settings.
   *
   * Driven by the verdict rather than by the config, because that is the only
   * thing that knows: `Tool.status` says the file is right, and the file being
   * right is exactly the state this describes. The two routes come from the same
   * verdict, so the card cannot name a route the sweep did not establish.
   */
  const reopenAlert = useMemo(() => {
    if (view.kind !== "app") return undefined;
    const verdict = verdicts.get(view.slug);
    if (verdict?.reason !== "reopen_required") return undefined;
    const app = appFor(apps, view.slug);
    if (!app) return undefined;
    return (
      <ReopenAlert
        name={app.name}
        routeInUse={verdict.route_in_use}
        requestedRoute={verdict.requested_route}
        onReopen={() => void runningApps.offerAfterChange([view.slug])}
      />
    );
  }, [view, verdicts, apps, runningApps]);

  /**
   * The config-routed tools a quit would strand: connected or drifted, either
   * way their configs point at the loopback relay that dies with this process.
   *
   * The same set Rust's `request_quit` computes from the registry before it
   * defers to this window. Derived here rather than asked for, because the menu
   * entry raises the flow directly and the backend only buffers a list when the
   * *tray* asked.
   */
  const routedForQuit = useMemo(
    () =>
      tools
        .filter(
          (t) => t.status.kind === "connected" || t.status.kind === "drifted",
        )
        .map((t) => t.name),
    [tools],
  );

  const onMenuSelect = useCallback(
    (action: TopnavAction) => {
      setMenuOpen(false);
      if (action === "dashboard") openLink(GATE_DASHBOARD_URL);
      // Drawn in the menu and shipped even though the address 404s today; the
      // decision and the reasoning it overruled are on `TopnavAction`.
      else if (action === "support") openLink(GATE_SUPPORT_URL);
      // The docs entry was drawn, listed and dead: `GATE_DOCS_URL` is the same one
      // the Settings row opens.
      else if (action === "docs") openLink(GATE_DOCS_URL);
      else if (action === "quit") {
        // Nothing routed means nothing to put back, so there is no choice to
        // offer - which is the rule the tray's own Quit already follows, where
        // Rust exits outright on an empty list. Asking "how?" about a teardown
        // with no work in it would be a dialog for its own sake.
        if (routedForQuit.length === 0) void quitApp().catch(() => {});
        else
          setQuit({
            kind: "choose",
            tools: routedForQuit,
            choice: "disconnect",
          });
      }
    },
    [openLink, routedForQuit],
  );

  const setupError = setup.error ? classifyError(setup.error, "sign_in") : null;
  /** A failed diagnostics write. Its own state because the write is not the
   *  setup hook's, and because it used to go to `setActionError` - which
   *  renders in `AppShell`, and `AppShell` is not on screen during setup. A
   *  user whose preference write failed saw nothing happen, on either button,
   *  with no way forward and no help from restarting: the stage is derived
   *  from the stored flag, so it stayed on this step. */
  const [diagnosticsError, setDiagnosticsError] = useState<ClassifiedError | null>(
    null,
  );

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
    const gatewayPicker = !import.meta.env.DEV ? undefined : (
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
        // The stops the frames draw, measured off each pane's `prog-track` fill
        // against the 1024 track (2026-08-30): sign-in 256, API key 512, org
        // picker 512, name device 768, diagnostics 1024. Quarters, with the two
        // second-step panes sharing one - they are alternatives, not a sequence,
        // so the rail does not advance between them. These were six invented
        // fractions until that read.
        progress={
          stage.kind === "welcome"
            ? 0.25
            : stage.kind === "api-key"
              ? 0.5
              : stage.kind === "org-picker"
                ? 0.5
                : stage.kind === "name-device"
                  ? 0.75
                  : 1
        }
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
            error={diagnosticsError && <SetupNote error={diagnosticsError} />}
            onContinue={() => {
              // Records the *displayed* value, changed or not: leaving the default
              // in place is an answer, and treating it as unanswered would ask
              // again on the next launch. This is also what dismisses the step,
              // since the stage is derived from the stored flag.
              const share = prefs?.share_diagnostics ?? true;
              setDiagnosticsError(null);
              setAnalyticsConsent(share);
              void setShareDiagnostics(share)
                .catch((e) => setDiagnosticsError(classifyError(e, "generic")))
                .finally(() => void loadPreferences());
            }}
            onSkip={() => {
              // The drawn escape. Skipping consent is declining it: recorded as
              // off, not left unanswered, or the step would ask again next
              // launch and a skipped default-on would keep collecting.
              setDiagnosticsError(null);
              setAnalyticsConsent(false);
              void setShareDiagnostics(false)
                .catch((e) => setDiagnosticsError(classifyError(e, "generic")))
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
        {/* The teardown report outlives the screen change that produced it.
          * Disconnect and reset both end the session, so the shell drops to
          * these panes the moment they land - and the report is about the
          * operation the user just performed, not about the screen they are on.
          * Rendered inside the layout rather than beside it because `Modal`
          * positions itself over whatever is behind it. */}
        {teardown && (
          <TeardownReportDialog
            report={teardown}
            onClose={() => setTeardown(null)}
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
      appGroups={sidebarGroups}
      // The engine's own switch. Without it a window whose routing was off could
      // start it only by accident, through a config member's connect - and a
      // chat domain, which routes through the engine rather than the relay,
      // could not start it at all.
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
      onSelectApp={(slug) => setView({ kind: "app", slug })}
      onRefreshApps={() => void refreshNow()}
      refreshingApps={refreshing}
      inventory={inventory}
      notice={
        actionError ? (
          <ErrorBanner
            title={actionError.title}
            hint={actionError.hint}
            raw={actionError.raw}
            onDismiss={() => setActionError(null)}
          />
        ) : recoveryNames.length > 0 && !recoveryHidden ? (
          // Below the error banner: a failure that just happened outranks a
          // recorded one that can still be resumed.
          <RecoveryBanner
            names={recoveryNames}
            rows={recoveryRowList}
            progress={resumeProgress ?? undefined}
            busy={resuming}
            onResume={() => void resumeNow()}
            onAction={(slug, step) => {
              // Retry is this notice's own action; a reopen hands over to the
              // close-apps conversation, which is the only thing that can act on
              // a running process. Sign-in is neither - it is a different screen -
              // so the row names it and offers no control.
              if (step === "retry") void retryOne(slug);
              if (step === "reopen_tool") void runningApps.offerAfterChange([slug]);
            }}
            onReviewDetails={summary ? () => setDetailsOpen(true) : undefined}
            onFinishLater={() => setRecoveryHidden(true)}
          />
        ) : undefined
      }
      onToggleApp={toggleRailApp}
      dialog={
        // A pending quit decision outranks every other overlay: the user asked
        // to leave, and an update prompt or routing notice must not sit on top
        // of the question. Same precedence the popover gives it (TAKEOVER_Z.quit).
        quit?.kind === "left-behind" ? (
          <QuitLeftBehindDialog
            tools={quit.tools}
            busy={quitBusy}
            onRetry={() => void runDisconnect()}
            onQuitAnyway={() => void finishQuit()}
            onCancel={cancelQuit}
          />
        ) : quit?.kind === "confirm" ? (
          <QuitSafeToCloseDialog
            disconnected={quit.disconnected}
            busy={quitBusy}
            onClose={() => void finishQuit()}
            onCancel={cancelQuit}
          />
        ) : quit?.kind === "choose" ? (
          <QuitDialog
            tools={quit.tools}
            choice={quit.choice}
            onChoose={(choice) =>
              setQuit((q) => (q?.kind === "choose" ? { ...q, choice } : q))
            }
            busy={quitBusy}
            onContinue={continueQuit}
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
            secondary={{
              label: "Not now",
              onClick: () => routing.resolvePrompt(false),
            }}
            primary={{
              label: "Trust certificate",
              onClick: () => routing.resolvePrompt(true),
            }}
            onDismiss={() => routing.resolvePrompt(false)}
          >
            <p className="text-sm leading-5 text-neutral-600">
              The certificate stays on this machine, and you can remove it from
              Settings at any time.
            </p>
            {/* Naming the system dialog is the whole of AG-534, and "your
                operating system will ask for permission" is not that: on Windows
                what arrives is a red "Security Warning" quoting a certificate
                name, which reads as something having gone wrong. Same sentence
                the popover's `CertificateNotice` uses, from the same helper, so
                the two shells do not prepare the user two different ways. */}
            <p className="text-sm font-medium leading-5 text-base-foreground">
              {trustPromptHint(platform)}
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
            secondary={{
              label: "Keep it",
              onClick: () => routing.resolvePrompt(false),
            }}
            primary={{
              label: "Remove certificate",
              onClick: () => routing.resolvePrompt(true),
              destructive: true,
            }}
            onDismiss={() => routing.resolvePrompt(false)}
          >
            <p className="text-sm leading-5 text-neutral-600">
              Routing itself stays on, and your tools keep their configuration.
              Your operating system may ask for permission to remove it.
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
        ) : modelOverlay?.kind === "picker" ? (
          <ModelPickerDialog
            // A real catalogue now, read from the gateway. Still empty on a
            // deployment with no platform provider accounts, which the dialog
            // says in words - the design draws eleven `gate/...` ids, and
            // shipping those as though they were real would put a fabricated
            // catalogue in front of the user.
            appName={
              appFor(apps, view.kind === "app" ? view.slug : "")?.name ??
              "This app"
            }
            // The slug, not the display name: compatibility is keyed on the tool
            // the preferences use, and two apps can share a name.
            appSlug={view.kind === "app" ? view.slug : null}
            models={gateModels.models ?? []}
            loading={gateModels.loading && gateModels.models === null}
            failure={gateModels.failure?.message ?? null}
            selectedIds={openModelIds}
            onSave={(ids) => {
              const then = modelOverlay.then;
              setModelOverlay(null);
              if (then === "activate") activateGateModel(ids);
              else void saveModel("tool", ids);
            }}
            onDismiss={() => setModelOverlay(null)}
          />
        ) : modelOverlay?.kind === "confirm-gate" ? (
          <UseGateModelDialog
            app={{
              name:
                appFor(apps, view.kind === "app" ? view.slug : "")?.name ??
                "this app",
            }}
            // Only meaningful when there is one model to attribute; the dialog
            // drops it for a set.
            vendor={modelOverlay.modelIds[0].split("/")[0]}
            // The whole set, so the dialog can list what the charge covers.
            // AG-590 requires the enabled models be stated before it is accepted.
            modelIds={modelOverlay.modelIds}
            // The real balance now. This dialog is where someone agrees to spend
            // it, so showing what there is to spend belongs here more than
            // anywhere.
            credits={formatCredits(credits.credits) ?? "N/A"}
            onKeepAppDefault={() => {
              const ids = modelOverlay.modelIds;
              setModelOverlay(null);
              // They declined the billing, not the models. Keeping the picks
              // means the picker does not have to be walked again to change their
              // mind, and under App default they are remembered rather than
              // served.
              if (ids.join() !== openModelIds.join())
                void saveModel("tool", ids);
            }}
            onUseGateCredits={() => {
              const ids = modelOverlay.modelIds;
              setModelOverlay(null);
              void saveModel("gate", ids, true);
            }}
          />
        ) : detailsOpen && summary ? (
          <RestoreDetailsDialog
            summary={summary}
            now={new Date()}
            onClose={() => setDetailsOpen(false)}
          />
        ) : teardown ? (
          // After the review, before the incidental dialogs: a teardown that
          // left tools behind is the newest thing that happened, and the user
          // asked for the operation that produced it.
          <TeardownReportDialog
            report={teardown}
            onClose={() => setTeardown(null)}
          />
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
            currentKeyMasked={maskedKey(
              keyPrefix,
              account?.has_api_key ?? false,
            )}
            newKey={settings.newKey}
            onNewKeyChange={settings.setNewKey}
            onCancel={settings.dismissPrompt}
            onReplace={() => void settings.replaceKey()}
          />
        ) : settings.prompt?.kind === "switch-org" ? (
          <SwitchOrganizationDialog
            organizations={settings.prompt.orgs.map(toDialogOrg)}
            selectedId={settings.prompt.selectedId}
            currentId={account?.org_id ?? undefined}
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
        ) : openEvent ? (
          // Above the offer, below everything the user is in the middle of: they
          // clicked this row, so it outranks anything unsolicited.
          <SecurityEventDialog
            event={openEvent}
            onClose={() => setOpenEvent(null)}
            onOpenDashboard={() => {
              const requestId = openEvent.requestId;
              void openExternal(
                `${GATE_DASHBOARD_URL}messages/${encodeURIComponent(requestId)}`,
              ).then((err) => {
                if (err) {
                  // The browser never opened, so AC7's "until the matching
                  // dashboard detail opens" has not been met: leave the summary
                  // up and report the failure in the banner stack.
                  setActionError(err);
                  return;
                }
                setOpenEvent(null);
              });
            }}
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
                  className="rounded-md border border-red-200 bg-red-50 p-3 text-sm leading-5 text-red-900"
                >
                  <span className="font-medium">{offerError.title}</span>{" "}
                  {offerError.hint}
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
      ) : view.kind === "security" ? (
        <SecurityPane
          events={securityFeed.events}
          state={securityFeed.state}
          loading={securityFeed.loading}
          unavailable={securityFeed.unavailable}
          onRetry={securityFeed.retry}
          onOpenEvent={setOpenEvent}
        />
      ) : view.kind === "app" ? (
        <AppPane
          name={appFor(railApps, view.slug)?.name ?? view.slug}
          logo={brandMarkFor(view.slug)}
          // Intent, not the verdict: a drifted app is still one the user asked to
          // route, and driving this switch from the observed status is the bug
          // `lib/groups.ts` documents - it renders off, and clicking it turns off
          // the setting the user was trying to turn on.
          isProtected={appFor(railApps, view.slug)?.on ?? false}
          // Observation, and the whole of it: the rail row prints the phrase
          // alone because its reason does not fit 250px, so this pane is where
          // "Not protected - Verification failed" is legible.
          status={appFor(railApps, view.slug)?.status}
          busy={routingBusy}
          onToggleProtected={() =>
            toggleRailApp(
              view.slug,
              !(appFor(railApps, view.slug)?.on ?? false),
            )
          }
          stats={toolActivity.view?.stats ?? EMPTY_STATS}
          buckets={toolActivity.view?.buckets ?? []}
          // Pending while the installation list is still open too: until it
          // answers we do not know which machine this is, so there is nothing to
          // read yet - and a skeleton is the honest account of that. A domain
          // pane is never pending: its read will not fire (see `openDomain`),
          // and a skeleton would promise an answer that is not coming.
          pending={
            !openDomain &&
            (!installsResolved ||
              (toolActivity.view === null && toolActivity.failure === null))
          }
          // A multi-provider tool gets no model card: see `AppPane`'s
          // `modelChoice`. `multiProviderSlugs` is `buildGroups`' own
          // membership, so this can never disagree with the rail about which
          // tools those are.
          {...(multiProviderSlugs.has(view.slug)
            ? {}
            : {
                modelChoice: openModelChoice,
                modelBusy: modelBusy,
                // The preference's own read, not the activity pane's: an
                // unattributed machine has nothing to say about a setting.
                modelPending:
                  toolModels.view === null && toolModels.failure === null,
                // AG-592. Null while anything it depends on is unread - an
                // unchecked model is not a healthy one, and saying nothing is
                // the honest state.
                modelAttention:
                  modelAttention({
                    choice: openPref,
                    catalogue: gateModels.models,
                    credits: credits.credits,
                    // The feed is the only thing that can see a Gate model the
                    // tool cannot actually be served with: the catalogue says it
                    // exists and the balance says it is affordable, and the
                    // requests fail anyway.
                    recent: toolEvents.view?.entries.slice(0, 5) ?? null,
                  })?.message ?? null,
                // Switching to a Gate model spends PAYG credits, so it is
                // confirmed rather than taken on a radio click. Switching back
                // is not - and it keeps the chosen model, which is the whole
                // reason a preference may name a model while its source is
                // "tool".
                onChooseModel: (choice: ModelChoice) => {
                  if (choice === "gate") {
                    if (openModelIds.length > 0)
                      activateGateModel(openModelIds);
                    // Nothing to switch *to* yet, so the picker comes first:
                    // Gate cannot serve a model nobody enabled.
                    else setModelOverlay({ kind: "picker", then: "activate" });
                  } else {
                    void saveModel("tool", openModelId ? [openModelId] : []);
                  }
                },
                gateModel: openModelId
                  ? // Vendor from the id's own namespace rather than from the
                    // catalogue: the catalogue is only loaded when the picker is
                    // open, and a card that showed a vendor only while a dialog
                    // was up would be stranger than one that reads it off the
                    // id. AG-592 is where a selected model gets looked up and
                    // told it is gone.
                    {
                      vendor: openModelId.split("/")[0],
                      // The whole set: the card lists it rather than naming the
                      // first and counting the rest in a heading nobody can
                      // expand.
                      ids: openModelIds,
                    }
                  : null,
                onChangeModel: () =>
                  setModelOverlay({
                    kind: "picker",
                    // Already on Gate: a different model is served immediately,
                    // and billing was accepted when the switch was made. On App
                    // default it is a browse, and picking must not start
                    // spending.
                    then: openModelChoice === "gate" ? "activate" : "remember",
                  }),
                // The real balance, from `GET /v1/me/credits`. Null when it
                // could not be read, which the card draws as N/A rather than as
                // a zero balance. See principle 6.
                credits: formatCredits(credits.credits),
                // Null when unread, which the row omits rather than guessing.
                plan: credits.credits?.plan ?? null,
                // No dedicated credits endpoint, but the row's own glyph
                // promises an external link, and the dashboard is where credits
                // are actually bought.
                onAddCredits: () => openLink(GATE_DASHBOARD_URL),
                // AG-729 gave this a destination. Undefined when the gateway
                // named none, which removes the control rather than drawing a
                // dead one - the state every gateway was in before that field
                // existed.
                onManageBilling: credits.credits?.billingUrl
                  ? () => openLink(credits.credits!.billingUrl!)
                  : undefined,
              })}
          activity={toolEventRows}
          eventsPending={
            !openDomain &&
            (!installsResolved ||
              (toolEvents.view === null && toolEvents.failure === null))
          }
          onLoadMore={
            toolEvents.view?.nextCursor ? toolEvents.loadMore : undefined
          }
          // Each half reports its own read. Deriving the feed's flag from the
          // overview's state let a feed that answered - and answered empty - be
          // reported as unreadable because the *chart* had not landed, which is
          // precisely the unread-versus-empty confusion these flags exist to
          // prevent. Two endpoints, two answers.
          unavailable={{
            chart:
              openDomain ||
              unattributedMachine ||
              (toolActivity.view ? toolActivity.view.missing.chart : true),
            events:
              openDomain || unattributedMachine || toolEvents.failure !== null,
          }}
          alert={
            <>
              {reopenAlert}
              {paneNotice && (
                <AlertBanner
                  key={paneNotice.id}
                  title={paneNotice.title}
                  body={paneNotice.body}
                  switchLabel={paneNotice.switchLabel}
                  on={false}
                  busy={noticeBusy || routingBusy}
                  onToggle={() => void runNoticeAction(paneNotice.action)}
                  onDismiss={() =>
                    setDismissedNotices((d) => [...d, paneNotice.id])
                  }
                />
              )}
              {modelError && (
                // The gateway's own sentence, not a code. A role refusal and a
                // dead network want different things from the reader, and on a
                // write the message is the only thing that tells them apart.
                <p className="text-base-xs text-red-700">
                  <span className="font-medium">Model not changed:</span>{" "}
                  {modelError}
                </p>
              )}
              {openDomain ? (
                // The gateway attributes requests to config tools by their own
                // user agents; traffic from these surfaces arrives unattributed,
                // on purpose - a guessed slug would file one app's traffic under
                // another's name. So there is no per-app reading to show here,
                // and saying so beats a zero.
                <p className="text-base-xs text-base-muted-foreground">
                  <span className="font-medium">This app:</span> its requests
                  aren&apos;t attributed to a single app yet, so its own
                  activity can&apos;t be shown. The Overview still covers your
                  whole organisation.
                </p>
              ) : unattributedMachine ? (
                // No numbers can be shown here, and the reason is not a failure:
                // the gateway answered and does not recognise this machine, so
                // there is no way to ask "what has this tool done *here*". Said
                // plainly rather than by showing the org's traffic under one
                // machine's heading, or zeros under a tool in daily use.
                <p className="text-base-xs text-base-muted-foreground">
                  <span className="font-medium">This machine:</span> the gateway
                  has no traffic attributed to it yet, so this app&apos;s own
                  activity cannot be shown. The Overview still covers your whole
                  organisation.
                </p>
              ) : (
                <>
                  {/* The same notices the Overview shows, from the same builder, so
                      the two panes cannot describe one gateway failure two
                      different ways. Both reads get one: the feed is its own
                      endpoint and its own failure, and routing it through here is
                      what gives it a cause and a retry instead of a bare
                      "couldn't be read". */}
                  <ActivityGaps
                    view={toolActivity.view}
                    failure={toolActivity.failure}
                    loading={toolActivity.loading}
                    onRetry={() => {
                      toolActivity.reload();
                      toolEvents.reload();
                    }}
                    onDiagnostics={() => void openDiagnostics()}
                    onOpenLink={openLink}
                  />
                  <ActivityGaps
                    view={null}
                    failure={toolEvents.failure}
                    loading={toolEvents.loading}
                    onRetry={toolEvents.reload}
                    onDiagnostics={() => void openDiagnostics()}
                    onOpenLink={openLink}
                    subject="Recent activity"
                  />
                </>
              )}
            </>
          }
        />
      ) : (
        <Overview
          stats={activity.view?.stats ?? EMPTY_STATS}
          buckets={activity.view?.buckets ?? []}
          policies={activity.view?.policies ?? []}
          savings={activity.view?.savings ?? []}
          onManagePolicies={() => openLink(GATE_POLICIES_URL)}
          onManageSavings={() => openLink(GATE_SAVINGS_URL)}
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
              // What the user asked for, not what the gateway echoed. This is a
              // control, and driving a control from observed state is the bug
              // CLAUDE.md's second principle documents: selecting an installation
              // clears the view, so the echo was `null` for the whole round trip
              // and the picker snapped back to "All installations" - contradicting
              // the click that caused it, for three seconds under
              // `gcSlowActivity(3000)`.
              //
              // The old comment argued the label had to agree with numbers that
              // were "still the previous scope's". They are not: the effect below
              // clears them, so it agreed with nothing. The figures are skeletons
              // while this is pending, which is what says the numbers are not the
              // new scope's yet.
              value={installFilter}
              onChange={setInstallFilter}
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
                  onDismiss={() =>
                    setDismissedNotices((d) => [...d, notice.id])
                  }
                  paging={
                    notices.length > 1
                      ? {
                          onPrev: () =>
                            setNoticePage(
                              (p) => (p - 1 + notices.length) % notices.length,
                            ),
                          onNext: () =>
                            setNoticePage((p) => (p + 1) % notices.length),
                        }
                      : undefined
                  }
                />
              )}
              <ActivityGaps
                view={activity.view}
                failure={activity.failure}
                loading={activity.loading}
                onOpenLink={openLink}
                onRetry={activity.reload}
                onDiagnostics={() => void openDiagnostics()}
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
/**
 * A classified failure on a setup pane.
 *
 * Carries `ErrorDetails` for the same reason `ErrorBanner` does: the fallback
 * hint promises "the details below", and this is the surface with no other way
 * to see them - a first-run or re-sign-in failure has no shell behind it. An
 * unclassified `oauth_begin_login` failure ("OAuth is not configured in this
 * build", a refused loopback port) landed here as a bare "Couldn't complete
 * sign-in" with nothing under it, which is a dead end rather than a report.
 */
function SetupNote({ error }: { error: ClassifiedError }) {
  return (
    <>
      <span className="font-medium">{error.title}</span> {error.hint}
      <ErrorDetails raw={error.raw} title={error.title} />
    </>
  );
}

function initialsOf(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  const letters =
    words.length > 1
      ? words[0][0] + words[1][0]
      : (words[0] ?? "?").slice(0, 2);
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
 * already folds in. `proxyMemberStatus` in `lib/verdict.ts` carries the
 * derivation, shared with the tray popover.
 */

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
  onOpenLink,
  subject,
}: {
  view: ActivityView | null;
  failure: ActivityFailure | null;
  loading: boolean;
  onRetry: () => void;
  onDiagnostics: () => void;
  /** Opens an external link and surfaces a failure. Passed in rather than
   *  imported: this component sits at module scope and cannot reach the shell's
   *  error banner, and a link that silently fails is what AG-598 is about. */
  onOpenLink: (url: string) => void;
  /** Overrides the notice's subject, for a caller that owns one read rather than
   *  the whole pane. The app pane mounts this twice - once for the counters and
   *  chart, once for the event feed - and two notices both headed "Activity"
   *  would leave the user unable to tell which retry belongs to which. */
  subject?: string;
}) {
  const run = (kind: GapActionKind) => {
    if (kind === "retry") onRetry();
    else if (kind === "diagnostics") onDiagnostics();
    else if (kind === "dashboard") onOpenLink(GATE_DASHBOARD_URL);
    else if (kind === "api-keys") onOpenLink(GATE_API_KEYS_URL);
    else onOpenLink(GATE_DOCS_URL);
  };

  // A failed fetch outranks per-section gaps: if nothing landed there is nothing
  // to itemise, and the sections listed in the held view describe the *previous*
  // reading, not this one.
  // Merged before the subject override: a caller that owns one read renames the
  // notice, and renaming five identical ones first would produce five rows all
  // headed the same thing with nothing to tell them apart.
  const notices = mergeNotices(
    failure
      ? [failureNotice(failure)]
      : (view?.gaps ?? []).map((g) => sectionNotice(g.section, g.reason)),
  ).map((n) => (subject ? { ...n, subject } : n));
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
        <p key={`${n.subject}|${n.cause}`} className="text-base-xs text-base-muted-foreground">
          <span className="font-medium">{n.subject}:</span> {n.cause}
          {n.actions.map((a) => (
            <button
              key={a.kind}
              type="button"
              onClick={() => run(a.kind)}
              disabled={a.kind === "retry" && loading}
              // The visible label is the design's, and it is deliberately short -
              // "Try again", not "Try again reading your activity". That reads well
              // inside a sentence and badly out of context, and out of context is
              // exactly how it reaches a screen reader, a button list, or a test.
              // Two of these notices can be on screen at once, each with its own
              // Retry, and the sidebar's failed-scan control is also called "Try
              // again": three buttons, one name, three different jobs. Naming the
              // subject fixes the ambiguity without touching the visible copy.
              aria-label={`${a.label}: ${n.subject}`}
              className="ml-2 rounded-sm font-medium text-base-primary underline decoration-transparent underline-offset-2 transition hover:decoration-inherit focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary disabled:text-base-muted-foreground"
            >
              {a.kind === "retry" && loading ? "Trying…" : a.label}
            </button>
          ))}
        </p>
      ))}
    </div>
  );
}
