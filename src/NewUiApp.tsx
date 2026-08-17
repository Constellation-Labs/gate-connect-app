import { useCallback, useEffect, useMemo, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize } from "@tauri-apps/api/dpi";
import type { Account, ProxyState, ProviderState, Tool } from "./lib/api";
import {
  getAccount,
  launchAtLoginStatus,
  listProviders,
  listTools,
  proxyStatus,
} from "./lib/api";
import { buildGroups } from "./lib/groups";
import type { Group, GroupMember } from "./lib/groups";
import { openExternal } from "./lib/openExternal";
import { GATE_DASHBOARD_URL } from "./lib/config";
import { AppShell } from "./components/gc/AppShell";
import { FamiliesPane } from "./components/gc/FamiliesPane";
import type { Family } from "./components/gc/FamiliesPane";
import { AppPane } from "./components/gc/AppPane";
import { Overview } from "./components/gc/Overview";
import { SettingsPane, buildSettingsSections } from "./components/gc/SettingsPane";
import { DiagnosticsDialog } from "./components/gc/dialogs";
import type { AppStatus, SidebarApp, SidebarView } from "./components/gc/Sidebar";
import type { TopnavAction } from "./components/gc/Topbar";
import { buildDiagnosticsReport } from "./lib/diagnosticsReport";
import { analyticsId } from "./lib/analytics";
import { usePlatform } from "./lib/platform";
import type { Platform } from "./lib/platform";

/**
 * The new shell against real data, behind the `gc.newUi` dev flag.
 *
 * **Read-only.** Every switch and action here is inert. The real toggles run
 * through drift review, certificate trust and the OAuth offer - the paths that
 * make routing safe to change - and none of that is wired yet. A preview that
 * silently rewrote a tool's config without the review dialog the design puts in
 * front of it would be worse than one that does nothing.
 *
 * Which means: this is for seeing the new UI at the right size with the right
 * data, not for driving the app. `App.tsx` still owns the shipping path.
 */
export function NewUiApp() {
  const [tools, setTools] = useState<Tool[]>([]);
  const [providers, setProviders] = useState<ProviderState[]>([]);
  const [proxy, setProxy] = useState<ProxyState | null>(null);
  const [account, setAccount] = useState<Account | null>(null);
  const [version, setVersion] = useState("");
  const [launchAtLogin, setLaunchAtLogin] = useState(false);
  const [view, setView] = useState<SidebarView>({ kind: "overview" });
  const [menuOpen, setMenuOpen] = useState(false);
  const [diagnosticsOpen, setDiagnosticsOpen] = useState(false);
  const platform = usePlatform();

  // The popover ships at 380x620 with no decorations. Resize here rather than
  // in tauri.conf.json so the shipping default stays the popover for anyone
  // without the flag set.
  useEffect(() => {
    void (async () => {
      try {
        // Inside the try: `getCurrentWindow` throws outside Tauri, which is
        // exactly the plain-browser case this preview is most useful in.
        const win = getCurrentWindow();
        await win.setResizable(true);
        await win.setSize(new LogicalSize(1024, 720));
        await win.setDecorations(true);
        await win.setAlwaysOnTop(false);
        await win.setSkipTaskbar(false);
        await win.center();
      } catch {
        // A webview that refuses to resize still renders; the layout just gets
        // clipped. Not worth blocking the preview over.
      }
    })();
  }, []);

  useEffect(() => {
    void (async () => {
      const [t, p, px, acct, v, launch] = await Promise.all([
        listTools().catch(() => [] as Tool[]),
        listProviders().catch(() => [] as ProviderState[]),
        proxyStatus().catch(() => null),
        getAccount().catch(() => null),
        getVersion().catch(() => ""),
        launchAtLoginStatus().catch(() => null),
      ]);
      setTools(t);
      setProviders(p);
      setProxy(px);
      setAccount(acct);
      setVersion(v);
      setLaunchAtLogin(launch?.enabled ?? false);
    })();
  }, []);

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
          status: toolStatus(t),
          // Intent, not observation: a drifted tool is still one the user asked
          // to route. See the note on SidebarApp.
          on: t.status.kind === "connected" || t.status.kind === "drifted",
        })),
    [tools],
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

  const settingsSections = useMemo(
    () =>
      buildSettingsSections({
        // Device name, install ID and plan have no backend yet, so they read as
        // unknown rather than as invented values.
        deviceName: "-",
        installId: "-",
        loginId: account?.org_name ?? "-",
        plan: "-",
        gateway: account?.gateway_base_url ?? "-",
        apiKeyMasked: account?.has_api_key ? `sk-gw${"*".repeat(20)}` : "Not set",
        launchAtLogin,
        notifications: false,
        version: version ? `v${version}` : "-",
        onRenameDevice: noop,
        onCopyInstallId: noop,
        onUpgradePlan: noop,
        onReplaceKey: noop,
        onDisconnect: noop,
        onToggleLaunchAtLogin: noop,
        onToggleNotifications: noop,
        onReplayTutorial: noop,
        onCheckForUpdates: noop,
        onViewDiagnostics: () => setDiagnosticsOpen(true),
        onReviewReset: noop,
      }),
    [account, launchAtLogin, version, noop],
  );

  const protectedCount = apps.filter((a) => a.status.kind === "protected").length;

  const onMenuSelect = useCallback((action: TopnavAction) => {
    setMenuOpen(false);
    if (action === "dashboard") void openExternal(GATE_DASHBOARD_URL);
  }, []);

  return (
    <AppShell
      menuOpen={menuOpen}
      onMenuToggle={() => setMenuOpen((v) => !v)}
      onMenuSelect={onMenuSelect}
      routing={{ protectedCount, totalCount: apps.length }}
      orgName={account?.org_name ?? "No organization"}
      onSwitchOrg={noop}
      view={view}
      onNavigate={setView}
      apps={apps}
      onSelectApp={(slug) => setView({ kind: "app", slug })}
      onToggleApp={noop}
      dialog={
        diagnosticsOpen ? (
          <DiagnosticsDialog
            report={previewDiagnostics({
              now: new Date(),
              version,
              platform,
              account,
              proxy,
              providers,
              tools,
            })}
            onCopy={noop}
            onClose={() => setDiagnosticsOpen(false)}
          />
        ) : undefined
      }
    >
      {view.kind === "settings" ? (
        <SettingsPane sections={settingsSections} />
      ) : view.kind === "families" ? (
        <FamiliesPane families={families} onToggleFamily={noop} onToggleMember={noop} />
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
        />
      ) : (
        <Overview
          // The 24-hour endpoint is still being built. These are zeros rather
          // than plausible-looking numbers: a preview that invents traffic is
          // one somebody eventually screenshots as real.
          stats={EMPTY_STATS}
          buckets={[]}
          policies={[]}
          savings={[]}
          onManagePolicies={noop}
          onManageSavings={noop}
          period="Awaiting the 24-hour backend"
        />
      )}
    </AppShell>
  );
}

/** The 24-hour endpoint is still being built. Zeros rather than plausible
 *  numbers: a preview that invents traffic is one somebody screenshots as
 *  real. */
const EMPTY_STATS = {
  messages: 0,
  blockedFlagged: 0,
  tokensSavedPercent: 0,
  tokensSavedAmount: "+$0.00",
};

function appFor(apps: SidebarApp[], slug: string): SidebarApp | undefined {
  return apps.find((a) => a.slug === slug);
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
