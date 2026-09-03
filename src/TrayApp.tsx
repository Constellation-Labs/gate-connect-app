import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type {
  Account,
  ProviderState,
  ProxyState,
  Tool,
  Verdict,
} from "./lib/api";
import {
  getAccount,
  listProviders,
  listTools,
  proxyStatus,
  requestQuit,
  revealMainWindow,
  routingVerdicts,
} from "./lib/api";
import { useRouting } from "./lib/useRouting";
import { useRunningApps } from "./lib/useRunningApps";
import { classifyError } from "./lib/errors";
import type { ClassifiedError, ErrorContext } from "./lib/errors";
import { MULTI_PROVIDER_ID, buildGroups } from "./lib/groups";
import type { Group } from "./lib/groups";
import { proxyMemberStatus, verdictStatus, verdictsBySlug } from "./lib/verdict";
import { openExternal } from "./lib/openExternal";
import { GATE_DASHBOARD_URL, GATE_DOCS_URL } from "./lib/config";
import { trustPromptHint, usePlatform } from "./lib/platform";
import { useSecurityFeed } from "./lib/securityFeed";
import { Tray } from "./components/gc/Tray";
import type { TrayMenuAction, TrayNotInstalledApp } from "./components/gc/Tray";
import type { SidebarApp, SidebarGroup } from "./components/gc/Sidebar";
import { brandMarkFor } from "./components/gc/BrandMark";
import { ErrorBanner } from "./components/gc/banners";
import { Modal } from "./components/gc/Modal";
import {
  ApplyChangesDialog,
  ChangeReadyDialog,
  CloseAppsDialog,
  ReviewConfigDialog,
} from "./components/gc/dialogs";

/** A whole reading, compared by value: every read builds fresh objects. */
const detectionSignature = (reading: unknown): string => JSON.stringify(reading);

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
  const [providers, setProviders] = useState<ProviderState[]>([]);
  const [proxy, setProxy] = useState<ProxyState | null>(null);
  const [verdicts, setVerdicts] = useState<Map<string, Verdict>>(new Map());
  const [account, setAccount] = useState<Account | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const [notInstalledOpen, setNotInstalledOpen] = useState(false);
  const [actionError, setActionError] = useState<ClassifiedError | null>(null);
  const platform = usePlatform();

  /** What the last read put on screen, so an unchanged reading is dropped
   * rather than re-rendering the whole popover for it. */
  const rendered = useRef({ tools: "", proxy: "" });
  const rereading = useRef(false);

  useEffect(() => {
    rendered.current = {
      tools: detectionSignature(tools),
      proxy: detectionSignature(proxy),
    };
  }, [tools, proxy]);

  const refreshVerdicts = useCallback(async () => {
    const v = await routingVerdicts().catch(() => null);
    if (v) setVerdicts(verdictsBySlug(v));
  }, []);

  /** Event-driven re-read: a write landed or the engine changed state, so
   * commit whatever comes back and re-sweep the verdicts. */
  const refresh = useCallback(async () => {
    const [t, px] = await Promise.all([
      listTools().catch(() => null),
      proxyStatus().catch(() => null),
    ]);
    if (t) setTools(t);
    if (px) setProxy(px);
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
      if (changed) void refreshVerdicts();
    } finally {
      rereading.current = false;
    }
  }, [refreshVerdicts]);

  useEffect(() => {
    void (async () => {
      const [t, p, px, acct] = await Promise.all([
        listTools().catch(() => null),
        listProviders().catch(() => [] as ProviderState[]),
        proxyStatus().catch(() => null),
        getAccount().catch(() => null),
      ]);
      setTools(t ?? []);
      setProviders(p);
      setProxy(px);
      setAccount(acct);
      void refreshVerdicts();
      setLoaded(true);
    })();
  }, [refreshVerdicts]);

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
      if (!document.hidden) void redetect();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      void unlisten.then((off) => off()).catch(() => {});
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [redetect]);

  // The engine changes state without us asking - a CLI toggle, the other
  // window, the startup auto-enable - and the account changes under an org
  // switch made in the main window. Repaint from the event.
  useEffect(() => {
    const unlisten = listen("proxy-state-changed", () => {
      void refresh();
      void getAccount()
        .then(setAccount)
        .catch(() => {});
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
  });

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
        ? buildGroups(providers, tools, proxy.domains, {
            proxyOn: proxy.running,
            caTrusted: proxy.ca_trusted,
          })
        : [],
    [providers, tools, proxy],
  );

  // The live security-event feed (AG-578). Keyed on the org so a switch does not
  // leave the previous org's count on screen, matching the window shell.
  const securityFeed = useSecurityFeed(
    account !== null,
    account ? `${account.auth_mode}|${account.gateway_base_url}|${account.org_id ?? ""}` : "",
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
   * Null is "no reading", and a row draws nothing for it. Three ways to get
   * there and they are one thing to the reader: the feed could not be read, it
   * has not answered yet, or there is no account for it to run under - and in
   * that last case `loading` never clears, because the hook's seed returns
   * early, which is why the account is checked here too.
   *
   * The buffer is what this popover has seen (200 events), the same depth the
   * window's Security events pane lists.
   */
  const alertCounts = useMemo<Map<string, number> | null>(() => {
    if (account === null || securityFeed.loading || securityFeed.unavailable) {
      return null;
    }
    const counts = new Map<string, number>();
    for (const e of securityFeed.events) {
      if (e.tool) counts.set(e.tool, (counts.get(e.tool) ?? 0) + 1);
    }
    return counts;
  }, [account, securityFeed.events, securityFeed.loading, securityFeed.unavailable]);

  /** A feed that is actually running has not answered yet, which is the one case
   *  worth holding a place for. */
  const alertsPending =
    account !== null && securityFeed.loading && !securityFeed.unavailable;

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
          on: t.status.kind === "connected" || t.status.kind === "drifted",
          logo: brandMarkFor(t.slug),
          busy: routingBusy,
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
    ],
  );

  // The rail's grouping, verbatim from `NewUiApp.sidebarGroups`: vendor
  // captions from `upstream_provider_name`, proxy members as rows with the
  // shared status derivation, one unlabelled group before the catalog loads.
  const trayGroups = useMemo<SidebarGroup[]>(() => {
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
            on: m.desired,
            logo: brandMarkFor(m.key),
            busy: routingBusy,
          });
        }
      }
      if (members.length === 0) continue;
      grouped.push({
        id: g.id,
        label: g.id === MULTI_PROVIDER_ID ? g.name : (vendor ?? g.name),
        apps: members,
      });
    }
    if (bySlug.size > 0) {
      grouped.push({ id: "unclaimed", label: "", apps: [...bySlug.values()] });
    }
    return grouped;
  }, [groups, apps, routingBusy]);

  const notInstalled = useMemo<TrayNotInstalledApp[]>(
    () =>
      tools
        .filter((t) => t.status.kind === "not_installed")
        .map((t) => ({ slug: t.slug, name: t.name, logo: brandMarkFor(t.slug) })),
    [tools],
  );

  /** Route or unroute one row - a domain through `setDomainRouted`, a tool
   * through the drift-gated config write. Same dispatch as the rail. */
  const toggleApp = useCallback(
    (slug: string, next: boolean) => {
      const member = groups.flatMap((g) => g.members).find((m) => m.key === slug);
      void (member?.kind === "proxy"
        ? routing.setDomainRouted(slug, next)
        : routeApp(slug, next));
    },
    [groups, routing.setDomainRouted, routeApp],
  );

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

  const onMenuSelect = useCallback(
    (action: TrayMenuAction) => {
      setMenuOpen(false);
      if (action === "dashboard") void openExternal(GATE_DASHBOARD_URL).then((err) => {
        if (err) setActionError(err);
      });
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
    [],
  );

  if (!loaded) {
    // A sub-frame gap before the first read lands, same call as the window
    // shell: painting the signed-out card and replacing it a frame later is
    // worse than painting nothing.
    return null;
  }

  return (
    <Tray
      master={proxy ? { on: proxy.running } : undefined}
      groups={trayGroups}
      notInstalled={notInstalled}
      notInstalledOpen={notInstalledOpen}
      onToggleNotInstalled={() => setNotInstalledOpen((v) => !v)}
      cli={
        proxy?.env_export_separable
          ? {
              on: proxy.env_export_opted_in,
              busy: routingBusy,
              onToggle: (next) => {
                setActionError(null);
                void routing.setEnvExport(next);
              },
            }
          : undefined
      }
      orgName={account?.org_name ?? "No organization"}
      signedOut={account === null}
      onToggleApp={toggleApp}
      onExpand={expand}
      security={
        securityFeed.loading
          ? undefined
          : {
              state: securityFeed.state,
              count: securityFeed.events.length,
              // The popover has room for a count, not a feed. Expanding is the
              // whole affordance: the pane it opens has the detail this card
              // deliberately does not try to fit.
              onOpen: expand,
            }
      }
      menuOpen={menuOpen}
      onMenuToggle={() => setMenuOpen((v) => !v)}
      onMenuSelect={onMenuSelect}
      dialog={
        <>
          {actionError && (
            // The tray draws no notice slot; the banner sits over the list the
            // way the dialogs do, because a swallowed failure is worse than an
            // undrawn surface.
            <div className="absolute inset-x-4 top-20 z-20">
              <ErrorBanner
                title={actionError.title}
                hint={actionError.hint}
                raw={actionError.raw}
                onDismiss={() => setActionError(null)}
              />
            </div>
          )}
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
              app={{
                name:
                  runningApps.stage.apps.length === 1
                    ? runningApps.stage.apps[0]
                    : "The affected apps",
              }}
              onDone={runningApps.dismiss}
            />
          ) : null}
        </>
      }
    />
  );
}
