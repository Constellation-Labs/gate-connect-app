import { useCallback, useEffect, useMemo, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
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
import { GATE_DASHBOARD_URL, GATE_POLICIES_URL, GATE_SAVINGS_URL } from "./lib/config";
import { AppShell } from "./components/gc/AppShell";
import { FamiliesPane } from "./components/gc/FamiliesPane";
import type { Family } from "./components/gc/FamiliesPane";
import { AppPane } from "./components/gc/AppPane";
import { Overview } from "./components/gc/Overview";
import { useActivity } from "./lib/activity";
import type { ActivityView } from "./lib/activity";
import { SettingsPane, buildSettingsSections } from "./components/gc/SettingsPane";
import { DiagnosticsDialog } from "./components/gc/dialogs";
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
 * **Routing actions are still inert.** Every switch here is a no-op, because the
 * real toggles run through drift review, certificate trust and the OAuth offer,
 * and none of that is wired. Rewriting a tool's config without the review
 * dialog the design puts in front of it would be worse than doing nothing, so
 * they do nothing.
 *
 * That makes the popover the only surface that can currently change what is
 * routed, which is why it is kept rather than deleted. This branch must not
 * merge until the switches work.
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
  // One fetch per mount, plus the pane's own refresh. Not polled: the endpoint
  // shares the gateway's per-minute throttle bucket with the user's own traffic.
  const activity = useActivity(true);

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
      // An API-key account holds no org locally, so the gateway's answer is the
      // only name it can show. Account first: it is what the user picked.
      orgName={account?.org_name ?? activity.view?.orgName ?? "No organization"}
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
          stats={activity.view?.stats ?? EMPTY_STATS}
          buckets={activity.view?.buckets ?? []}
          policies={activity.view?.policies ?? []}
          savings={activity.view?.savings ?? []}
          onManagePolicies={() => void openExternal(GATE_POLICIES_URL)}
          onManageSavings={() => void openExternal(GATE_SAVINGS_URL)}
          // Dashes rather than zeros until the first load lands: a zero is a
          // real reading and would claim the user had no traffic.
          pending={activity.view === null && activity.error === null}
          period={activity.view?.period ?? "Last 24 hours"}
          alert={<ActivityGaps view={activity.view} error={activity.error} />}
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
 * action rather than blanking the surface. The endpoint sends the cause; this is
 * the copy for it.
 *
 * Deliberately plain text in the pane's alert slot rather than a designed
 * component: the visual treatment for this state is AG-575's job and does not
 * exist yet, and inventing one would be the "dressing scaffolding up as product"
 * mistake. What matters now is that a zero is never mistaken for a real reading. */
const GAP_COPY: Record<string, string> = {
  connectivity: "could not be reached - try again",
  access: "is not visible to your role",
  attribution: "needs a signed-in user; this credential has none",
  not_configured: "has nothing set up yet",
  definition_pending: "is not defined yet",
};

function ActivityGaps({
  view,
  error,
}: {
  view: ActivityView | null;
  error: string | null;
}) {
  // A failed fetch outranks per-section gaps: if nothing loaded there is nothing
  // to itemise, and the last good view (if any) is still on screen behind this.
  if (error) {
    return (
      <p className="text-base-xs text-base-muted-foreground">
        Activity could not be refreshed. Showing the last reading.
      </p>
    );
  }
  if (!view || view.gaps.length === 0) return null;
  return (
    <ul className="flex flex-col gap-1">
      {view.gaps.map((g) => (
        <li key={g.section} className="text-base-xs text-base-muted-foreground">
          <span className="font-medium">{g.section}</span>{" "}
          {GAP_COPY[g.reason] ?? "is unavailable"}
        </li>
      ))}
    </ul>
  );
}
