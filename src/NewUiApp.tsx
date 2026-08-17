import { useCallback, useEffect, useMemo, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import type { Account, OAuthStatus, Org, ProxyState, ProviderState, Tool } from "./lib/api";
import {
  connectTool,
  getAccount,
  launchAtLoginStatus,
  listProviders,
  listTools,
  oauthStatus,
  openOnboardingWindow,
  proxyEnable,
  proxyStatus,
  proxyTrustCa,
} from "./lib/api";
import { useRouting } from "./lib/useRouting";
import { useSettingsActions } from "./lib/useSettingsActions";
import { useSetup } from "./lib/useSetup";
import { classifyError } from "./lib/errors";
import type { ClassifiedError } from "./lib/errors";
import { buildGroups } from "./lib/groups";
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
import { Overview } from "./components/gc/Overview";
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
 * Settings and org switching go through `useSettingsActions`. Rows with no
 * backend behind them are passed no handler, which omits the control rather than
 * leaving a dead one on screen.
 *
 * Still inert: the family master switch (hidden for the same reason), the
 * running-apps sequence, and the per-app model picker. Disconnect and reset wait
 * on a first-run screen to return to. The Overview and per-app metrics await the
 * 24-hour endpoint.
 */
export function NewUiApp() {
  const [tools, setTools] = useState<Tool[]>([]);
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
  const platform = usePlatform();
  // Which installation the Overview covers; `null` is the whole org, and stays
  // the default because traffic sent before attribution existed has no
  // installation at all. Selecting one refetches - the gateway narrows every
  // section server-side, so there is nothing to slice here.
  const [installId, setInstallId] = useState<string | null>(null);
  // One fetch per mount, plus the pane's own refresh. Not polled: the endpoint
  // shares the gateway's per-minute throttle bucket with the user's own traffic.
  const activity = useActivity(true, installId);
  const { installations, current: currentInstallId } = useInstallations(true);

  const refresh = useCallback(async () => {
    const [t, px] = await Promise.all([
      listTools().catch(() => null),
      proxyStatus().catch(() => null),
    ]);
    if (t) setTools(t);
    if (px) setProxy(px);
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
      setProviders(p);
      setProxy(px);
      setAccount(acct);
      setOAuth(oauthState);
      setVersion(v);
      setLaunchAtLogin(launch?.enabled ?? false);
      setLoaded(true);
    })();
  }, []);

  const [actionError, setActionError] = useState<ClassifiedError | null>(null);

  const routing = useRouting({
    tools,
    proxy,
    onSnapshot: ({ tools: t, proxy: px }) => {
      setTools(t);
      setProxy(px);
    },
    onError: (e) => {
      // `connect` covers both directions: the remedy copy is the same either way
      // for a failed tool write.
      setActionError(classifyError(e, "connect"));
    },
  });
  const routingBusy = routing.busy;

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

  /** Perform a notice's action, then re-read state so it clears itself. */
  const runNoticeAction = useCallback(
    async (action: NoticeAction) => {
      if (noticeBusy) return;
      setNoticeBusy(true);
      try {
        if (action.kind === "enable-routing") await proxyEnable();
        else if (action.kind === "trust-certificate") await proxyTrustCa();
        else await connectTool(action.slug, action.upstreamUrl);
      } catch {
        // Swallowed on purpose for now: the shell has nowhere to render a
        // failure yet, and the notice staying put is itself the signal that
        // nothing changed. Wire this to the error surface when one exists.
      } finally {
        await refreshRouting();
        setNoticeBusy(false);
      }
    },
    [noticeBusy, refreshRouting],
  );

  const apps = useMemo<SidebarApp[]>(
    () =>
      tools
        .filter((t) => t.status.kind !== "not_installed")
        .map((t) => ({
          slug: t.slug,
          name: t.name,
          status: toolStatus(t),
          // Intent, not observation: a drifted tool is still one the user asked
          // to route. See the note on SidebarApp.
          on: t.status.kind === "connected" || t.status.kind === "drifted",
          busy: routingBusy,
        })),
    [tools, routingBusy],
  );

  const families = useMemo<Family[]>(
    () =>
      groups.map((g) => ({
        id: g.id,
        name: g.name,
        on: g.desired > 0,
        members: g.members.map(memberToFamilyMember),
      })),
    [groups],
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
        version: version ? `v${version}` : "-",
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
        // The tutorial is its own window, already built and wired.
        onReplayTutorial: () => void openOnboardingWindow("settings"),
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
      version,
      currentInstallId,
      showDiagnostics,
      settings.copyText,
      settings.openReplaceKey,
      settings.openDisconnect,
      settings.openReset,
      settings.toggleLaunchAtLogin,
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
      onToggle={() => {
        setActionError(null);
        void routing.setAppRouted(drifted[0].slug, true);
      }}
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
      notice={
        actionError ? (
          <ErrorBanner
            title={actionError.title}
            hint={actionError.hint}
            onDismiss={() => setActionError(null)}
          />
        ) : undefined
      }
      onToggleApp={(slug, next) => {
        setActionError(null);
        void routing.setAppRouted(slug, next);
      }}
      dialog={
        routing.prompt?.kind === "drift" ? (
          <ReviewConfigDialog
            app={{ name: routing.prompt.name }}
            existingConfig={routing.prompt.existingConfig}
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
          onToggleMember={(familyId, key, next) => {
            setActionError(null);
            const member = families
              .find((f) => f.id === familyId)
              ?.members.find((m) => m.key === key);
            if (!member) return;
            void (member.kind === "proxy"
              ? routing.setDomainRouted(key, next)
              : routing.setAppRouted(key, next));
          }}
        />
      ) : view.kind === "app" ? (
        <AppPane
          name={appFor(apps, view.slug)?.name ?? view.slug}
          isProtected={appFor(apps, view.slug)?.status.kind === "protected"}
          onToggleProtected={noop}
          stats={EMPTY_STATS}
          buckets={[]}
          modelChoice="app"
          onChooseModel={noop}
          gateModel={{ vendor: "-", id: "-" }}
          onChangeModel={noop}
          credits="-"
          onAddCredits={noop}
          activity={[]}
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
          // Dashes rather than zeros until the first load lands: a zero is a
          // real reading and would claim the user had no traffic.
          pending={activity.view === null && activity.failure === null}
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
              {notices.length > 0 && (
                <AlertBanner
                  // Keyed so switching pages remounts rather than animating one
                  // card's text into another's.
                  key={notices[Math.min(noticePage, notices.length - 1)].id}
                  title={notices[Math.min(noticePage, notices.length - 1)].title}
                  body={notices[Math.min(noticePage, notices.length - 1)].body}
                  switchLabel={notices[Math.min(noticePage, notices.length - 1)].switchLabel}
                  // The switch reflects the state being fixed, which is always
                  // "not routing". Toggling it performs the action.
                  on={false}
                  onToggle={() =>
                    void runNoticeAction(
                      notices[Math.min(noticePage, notices.length - 1)].action,
                    )
                  }
                  onDismiss={() =>
                    setDismissedNotices((d) => [
                      ...d,
                      notices[Math.min(noticePage, notices.length - 1)].id,
                    ])
                  }
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

/** Shown before the first load lands, and for any counter the endpoint declined.
 *  Zeros rather than plausible numbers: a preview that invents traffic is one
 *  somebody screenshots as real. `ActivityGaps` says which numbers are missing,
 *  so a zero here is never silently mistaken for a real reading. */
const EMPTY_STATS = {
  messages: 0,
  blockedFlagged: 0,
  tokensSavedPercent: 0,
  tokensSavedAmount: "+$0.00",
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

function toolStatus(tool: Tool): AppStatus {
  switch (tool.status.kind) {
    case "connected":
      return { kind: "protected" };
    case "drifted":
      return { kind: "drifted" };
    default:
      // `detected` and `error` both mean "installed, not carrying traffic".
      // The error message has nowhere to go in the sidebar row; the per-app
      // pane is where it belongs once that is wired.
      return { kind: "not-protected" };
  }
}

function memberToFamilyMember(m: GroupMember): Family["members"][number] {
  const status: AppStatus = m.routed
    ? { kind: "protected" }
    : m.attention === "drifted"
      ? { kind: "drifted" }
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
      {/* Said before the cause, because "what you are looking at is old" is the
          more urgent fact: the numbers on screen are still readable and a user
          who misses this will read them as current. A clock time rather than an
          age, for the reason `ActivityView.takenAt` gives. */}
      {failure && view && (
        <p className="text-base-xs text-base-muted-foreground">
          <span className="font-medium">Stale reading.</span> These numbers are from{" "}
          {view.takenAt} and have not been refreshed since.
        </p>
      )}
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
