import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type {
  Account,
  PendingRestore,
  ProviderState,
  ProxyState,
  Tool,
  Verdict,
} from "./lib/api";
import {
  getAccount,
  getAccountKeyPrefix,
  listProviders,
  listTools,
  pendingRestore,
  proxyStatus,
  requestQuit,
  resumeRestore,
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
import { useInstallations } from "./lib/activity";
import { useToolMessages } from "./lib/toolMessages";
import type { ToolMessagesView } from "./lib/toolMessages";
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
  const [providers, setProviders] = useState<ProviderState[]>([]);
  const [proxy, setProxy] = useState<ProxyState | null>(null);
  const [verdicts, setVerdicts] = useState<Map<string, Verdict>>(new Map());
  /** What an interrupted restore still owes. Empty in the normal case, and the
   * card is omitted with it. */
  const [pending, setPending] = useState<PendingRestore | null>(null);
  const [resuming, setResuming] = useState(false);
  const [account, setAccount] = useState<Account | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  /** The account read FAILED, which is not the same as having no account.
   *  Collapsing the two drew "Sign in to get started" at a signed-in user
   *  whose keychain prompt was dismissed - the exact scenario CLAUDE.md
   *  documents - and that sentence is false. Principle 6, one level up from
   *  a figure. */
  const [accountUnread, setAccountUnread] = useState(false);
  const [notInstalledOpen, setNotInstalledOpen] = useState(false);
  /** The account's key prefix, which is what makes a replaced api key a different
   *  credential. Read back after every account read for the reason
   *  `activity_cache.rs` records: in api-key mode the org is whatever the gateway
   *  resolves the *key* to, so without this a key swap to another org on the same
   *  gateway leaves every scope string byte-identical and the previous org's
   *  figures stay on screen under the new org's name. */
  const [keyPrefix, setKeyPrefix] = useState<string | null>(null);
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

  /** Whether the last sweep landed. A failed one used to be indistinguishable
   *  from a quiet machine: the map stayed empty, every row rendered
   *  `Not protected - Checking`, the master card read `0 of N`, and the polled
   *  path only re-swept when the tools/proxy signature CHANGED - so on an idle
   *  machine one failure at first load persisted until something moved. This
   *  surface has no refresh affordance, so nothing the user could do fixed it. */
  const verdictsRead = useRef(false);

  const refreshVerdicts = useCallback(async () => {
    const v = await routingVerdicts().catch(() => null);
    verdictsRead.current = v !== null;
    if (v) setVerdicts(verdictsBySlug(v));
  }, []);

  /**
   * What an interrupted routing operation still owes (AG-570).
   *
   * Read here as well as in the window because the popover is a surface the
   * recovery has to stay reachable from, and the two shells do not share state -
   * a tray that waited for the window to tell it would say nothing on the many
   * machines where the window is never opened.
   */
  const loadRecovery = useCallback(async () => {
    const p = await pendingRestore().catch(() => null);
    if (p) setPending(p);
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
    // A master-on runs the restore, which is what shortens the snapshots - so
    // the card is re-read on the same event that repaints the switches, or it
    // lingers after the work finished.
    void loadRecovery();
  }, [refreshVerdicts, loadRecovery]);

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
      const [t, p, px, acct, prefix] = await Promise.all([
        listTools().catch(() => null),
        listProviders().catch(() => [] as ProviderState[]),
        proxyStatus().catch(() => null),
        // The wrapper distinguishes "the read failed" from "there is no
        // account": both arrive as null otherwise, and only the first should
        // suppress the signed-out state.
        getAccount()
          .then((a) => ({ read: true, account: a }))
          .catch(() => ({ read: false, account: null as Account | null })),
        getAccountKeyPrefix().catch(() => null),
      ]);
      setTools(t ?? []);
      setProviders(p);
      setProxy(px);
      setAccount(acct.account);
      setAccountUnread(!acct.read);
      setKeyPrefix(prefix);
      void refreshVerdicts();
      void loadRecovery();
      setLoaded(true);
    })();
  }, [refreshVerdicts, loadRecovery]);

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
        return;
      }
      void redetect();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      void unlisten.then((off) => off()).catch(() => {});
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [redetect]);

  /**
   * The session changed in the other window: an org switch, a replaced key, a
   * gateway switch, a sign-out, a disconnect.
   *
   * This popover renders one account's everything - the org name in the footer,
   * the security card's count, the per-row figures - and every one of those is
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

  /** Whose readings these are. Identical in shape to `NewUiApp`'s, key prefix
   *  included: anything keyed on this must drop when the credential changes, and
   *  a replaced api key is a changed credential even when every other field is
   *  the same. */
  const credential = account
    ? `${account.auth_mode}|${account.gateway_base_url}|${account.org_id ?? ""}|${keyPrefix ?? ""}`
    : "";

  // The live security-event feed (AG-578). Keyed on the credential so a switch
  // does not leave the previous org's count on screen, matching the window shell.
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
  const { byTool: messagesByTool, pending: messagesPending } = useToolMessages(
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

  /** What is still outstanding, providers and tools together: the user does not
   * care which snapshot an entry came from. */
  const recoveryNames = useMemo(
    () =>
      [...(pending?.providers ?? []), ...(pending?.tools ?? [])].map((e) => e.name),
    [pending],
  );

  /**
   * Finish what the interrupted restore left, as one call.
   *
   * The batch, not the window's per-entry walk: the progress the walk exists to
   * show has nowhere to go in a 400px card, and driving entries one at a time
   * from a popover that closes when it loses focus would leave a pass half done
   * with nothing on screen having said so. `restore_all`'s own retry semantics
   * mean this repeats no completed write either way.
   */
  const resumeNow = useCallback(async () => {
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
  }, [refresh]);

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
      signedOut={account === null && !accountUnread}
      accountUnread={accountUnread}
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
      recovery={
        recoveryNames.length > 0
          ? {
              names: recoveryNames,
              busy: resuming,
              onResume: () => void resumeNow(),
              // The per-tool account lives in the window, so this reveals it
              // rather than drawing a second, shorter version of the same
              // operation at 400px.
              onReview: expand,
            }
          : undefined
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
