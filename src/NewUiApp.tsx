import { useCallback, useEffect, useMemo, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import type {
  Account,
  OAuthStatus,
  Org,
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
  proxyStatus,
  routingVerdicts,
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
import { GATE_DASHBOARD_URL } from "./lib/config";
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
  const [view, setView] = useState<SidebarView>({ kind: "overview" });
  const [menuOpen, setMenuOpen] = useState(false);
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
      const [t, p, px, acct, oauthState, v, launch] = await Promise.all([
        listTools().catch(() => [] as Tool[]),
        listProviders().catch(() => [] as ProviderState[]),
        proxyStatus().catch(() => null),
        getAccount().catch(() => null),
        oauthStatus().catch(() => null),
        getVersion().catch(() => ""),
        launchAtLoginStatus().catch(() => null),
      ]);
      setTools(t);
      void refreshVerdicts();
      setProviders(p);
      setProxy(px);
      setAccount(acct);
      setOAuth(oauthState);
      setVersion(v);
      setLaunchAtLogin(launch?.enabled ?? false);
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

  const apps = useMemo<SidebarApp[]>(
    () =>
      tools
        .filter((t) => t.status.kind !== "not_installed")
        .map((t) => ({
          slug: t.slug,
          name: t.name,
          status: verdictStatus(verdicts.get(t.slug)),
          // Intent, not observation: a drifted tool is still one the user asked
          // to route. See the note on SidebarApp.
          on: t.status.kind === "connected" || t.status.kind === "drifted",
          busy: routingBusy,
        })),
    [tools, verdicts, routingBusy],
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
        members: g.members.map((m) => memberToFamilyMember(m, verdicts)),
      })),
    [groups, verdicts],
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

  // The PostHog distinct id: a per-install random device id, and the one string
  // that lines a pasted diagnostics report up against its event stream. The
  // closest thing to the design's install ID that actually exists.
  const installId = useMemo(() => {
    const id = analyticsId();
    return id.kind === "id" ? id.value : null;
  }, []);

  const settingsSections = useMemo(
    () =>
      buildSettingsSections({
        // Device name and plan have no backend, so they read as unknown rather
        // than as invented values, and their actions are omitted entirely.
        deviceName: "-",
        installId: installId ?? "Unavailable",
        loginId: account?.org_name ?? "-",
        plan: "-",
        gateway: account?.gateway_base_url ?? "-",
        apiKeyMasked: account?.has_api_key ? `sk-gw${"*".repeat(20)}` : "Not set",
        launchAtLogin,
        version: version ? `v${version}` : "-",
        updateNote: updateNoteFor(update),
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
        // The tutorial is its own window, already built and wired.
        onReplayTutorial: () => void openOnboardingWindow("settings"),
        // Explicit, so this one reports back: silence on a button the user just
        // pressed reads as broken.
        onCheckForUpdates: () => void update.checkNow(true),
        onViewDiagnostics: () =>
          setDiagnosticsReport(
            previewDiagnostics({
              now: new Date(),
              version,
              platform,
              account,
              proxy,
              providers,
              tools,
            }),
          ),
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
      version,
      installId,
      settings.copyText,
      settings.openReplaceKey,
      settings.openDisconnect,
      settings.openReset,
      settings.toggleLaunchAtLogin,
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
      orgName={account?.org_name ?? "No organization"}
      onSwitchOrg={() => {
        setActionError(null);
        void settings.openSwitchOrg();
      }}
      view={view}
      onNavigate={setView}
      apps={apps}
      onSelectApp={(slug) => setView({ kind: "app", slug })}
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
          alert={driftAlert}
        />
      ) : (
        <Overview
          stats={EMPTY_STATS}
          buckets={[]}
          policies={[]}
          savings={[]}
          onManagePolicies={noop}
          onManageSavings={noop}
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

const EMPTY_STATS = {
  messages: 0,
  blockedFlagged: 0,
  tokensSavedPercent: 0,
  tokensSavedAmount: "+$0.00",
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
): Family["members"][number] {
  const status: AppStatus =
    m.kind === "config"
      ? verdictStatus(verdicts.get(m.key))
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
