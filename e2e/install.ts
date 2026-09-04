/**
 * Installs the fake Tauri IPC into the page before any app code runs.
 *
 * `installFakeTauri` is serialized by Playwright and evaluated in the page, so
 * its body must be entirely self-contained - no imports, no references to
 * anything in module scope. Everything it needs arrives in `state`.
 *
 * What it provides, matching what `@tauri-apps/api@2` actually calls:
 *   - `window.__TAURI_INTERNALS__.invoke(cmd, args)` - the command table.
 *   - `.transformCallback(fn)` - how `listen` hands a handler to the backend.
 *   - `.metadata.currentWindow.label` - `getCurrentWindow()`, which `main.tsx`
 *     reads to choose the popover over the onboarding window.
 *   - `window.__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener` - called by
 *     the unlisten path on every effect cleanup.
 *
 * And for the tests: `window.__GATE_E2E__` with the live state, the recorded
 * call log, and `emit`, which delivers a backend event the way the Rust side
 * does (`quit-requested`, `proxy-state-changed`, `backend-error-pending`).
 */
import type { BackendState } from "./backend";

/** The handle each spec drives the fake backend through, from inside the page. */
export interface E2EHandle {
  state: BackendState;
  calls: { cmd: string; args: Record<string, unknown> }[];
  emit: (event: string, payload?: unknown) => void;
}

declare global {
  interface Window {
    __GATE_E2E__: E2EHandle;
  }
}

export function installFakeTauri(state: BackendState): void {
  // ---- localStorage seed. Before the app reads the tour flag on first render.
  try {
    for (const [k, v] of Object.entries(state.localStorage)) {
      window.localStorage.setItem(k, v);
    }
  } catch {
    /* noop */
  }

  const calls: { cmd: string; args: Record<string, unknown> }[] = [];
  const callbacks = new Map<number, (payload: unknown) => void>();
  // event name -> handler ids. One name can have several (App and a child
  // component both listen to proxy-state-changed).
  const listeners = new Map<string, Set<number>>();
  let nextId = 1;

  function emit(event: string, payload?: unknown): void {
    const ids = listeners.get(event);
    if (!ids) return;
    for (const id of ids) {
      const cb = callbacks.get(id);
      if (cb) cb({ event, id, payload });
    }
  }

  // ---- Domain helpers over the mutable state.

  function tool(slug: string) {
    const t = state.tools.find((x) => x.slug === slug);
    if (!t) throw new Error(`fake backend: no such tool ${slug}`);
    return t;
  }

  function provider(slug: string) {
    const p = state.providers.find((x) => x.slug === slug);
    if (!p) throw new Error(`fake backend: no such provider ${slug}`);
    return p;
  }

  /** Recompute a provider's headline from its members, the way the Rust
   *  provider layer does: on when at least one member is routed. */
  function syncProvider(slug: string) {
    const p = provider(slug);
    const toolsOn = p.tool_slugs.some(
      (s) => state.tools.find((t) => t.slug === s)?.status.kind === "connected",
    );
    const domainsOn = p.domain_slugs.some(
      (s) => state.proxy.domains.find((d) => d.slug === s)?.enabled,
    );
    p.enabled = toolsOn || domainsOn;
    return p;
  }

  /** Rust's `AGENT_PROCESSES`: tool slug to the process name it runs under. */
  const AGENT_PROCESSES: [string, string][] = [
    ["claude-code", "claude"],
    ["codex", "codex"],
    ["opencode", "opencode"],
  ];

  /** Rust's `agent_names_for`: null/undefined asks about every tool, a list
      narrows to those slugs, and an unknown slug narrows to nothing. */
  function agentNamesFor(only: string[] | null | undefined): string[] {
    return AGENT_PROCESSES.filter(([slug]) => only == null || only.includes(slug)).map(
      ([, name]) => name,
    );
  }

  /** What the verdict layer calls the Gate route: the account's own gateway URL,
      or the same fallback the backend uses when it cannot read one. */
  function gateRoute(): string {
    return state.account?.gateway_base_url ?? "Constellation Gate";
  }

  /** Rust's `recovery_summary`, assembled from the same four sources: the
      journal (what the write reached), the snapshots (what is still owed), the
      verdict log (what the last check saw) and the process table.

      The verdict half is derived from `routing_verdicts` rather than stubbed per
      slug, so a spec cannot set up a summary the real backend could never
      produce - the same rule the verdict handler above follows. */
  function recoverySummary() {
    const journal = state.restoreJournal;
    const pending = [
      ...state.pendingRestore.providers.map((e) => ({ ...e, kind: "provider" as const })),
      ...state.pendingRestore.tools.map((e) => ({ ...e, kind: "tool" as const })),
    ];
    if (!journal && pending.length === 0) return null;
    const verdicts = commands.routing_verdicts({}) as {
      slug: string;
      state: string;
      reason: string | null;
    }[];
    const rows = [
      ...(journal?.entries.map((e) => ({
        slug: e.slug,
        name: e.name,
        kind: e.kind,
        stage: e.outcome,
        at: e.at_unix,
      })) ?? []),
      // Pending entries the journal never mentioned, seeded `pending` - what
      // `JournalWriter::reopen` does, for the same reason.
      ...pending
        .filter((p) => !journal?.entries.some((e) => e.slug === p.slug))
        .map((p) => ({
          slug: p.slug,
          name: p.name,
          kind: p.kind,
          stage: "pending" as const,
          at: 0,
        })),
    ];
    const complete = ["restored", "not_installed", "unknown"];
    return {
      operation: "restore",
      updated_unix: journal?.updated_unix ?? 0,
      requested_routing_on: journal?.requested_routing_on ?? true,
      tools: rows.map((row) => {
        const verdict = verdicts.find((v) => v.slug === row.slug) ?? null;
        const reopenPending = verdict?.reason === "reopen_required";
        const stageComplete = complete.includes(row.stage);
        return {
          slug: row.slug,
          name: row.name,
          kind: row.kind,
          stage: row.stage,
          stage_complete: stageComplete,
          error_category:
            row.stage === "write_failed"
              ? "write"
              : row.stage === "deferred_signed_out"
                ? "account"
                : row.stage === "not_installed"
                  ? "not_installed"
                  : row.stage === "unknown"
                    ? "unknown"
                    : "none",
          stage_at_unix: row.at,
          last_verified_state:
            verdict?.state === "on" || verdict?.state === "off" ? verdict.state : null,
          last_verified_unix:
            verdict?.state === "on" || verdict?.state === "off" ? state.nowUnix : 0,
          check_state: verdict?.state ?? null,
          check_reason: verdict?.reason ?? null,
          check_at_unix: verdict ? state.nowUnix : 0,
          running: state.runningAgentNames.length > 0,
          reopen_pending: reopenPending,
          // `recovery::next_step`, including its ordering: an unfinished write
          // outranks a stale process, because there is nothing on disk yet for
          // a reopen to pick up.
          next_step: !stageComplete
            ? row.stage === "deferred_signed_out"
              ? "sign_in"
              : "retry"
            : reopenPending
              ? "reopen_tool"
              : "none",
        };
      }),
    };
  }

  const commands: Record<string, (args: Record<string, any>) => unknown> = {
    // ---- platform / app
    app_platform: () => state.platform,
    "plugin:app|version": () => state.version,

    // ---- tools
    // Fill the field the real backend always sends, so a fixture that omits it
    // still produces a well-formed Tool rather than an undefined the UI has to
    // guess about.
    list_tools: () => state.tools.map((t) => ({ config_location: null, ...t })),
    connect_tool: ({ slug }) => {
      const t = tool(slug);
      t.status = { kind: "connected" };
      return t.status;
    },
    disconnect_tool: ({ slug }) => {
      const t = tool(slug);
      t.status = { kind: "detected" };
      return t.status;
    },
    has_upstream_credential: () => true,
    save_upstream_api_key: () => null,
    clear_upstream_credential: () => null,

    // ---- model selection (AG-588)
    // The choice is a local file, so these commands are file reads and writes,
    // not gateway calls. The fake enforces the one rule the real setter has -
    // consent is recorded only when moving to `gate` - because a mock that
    // accepted everything would let the confirmation flow rot unnoticed.
    tool_model_preferences: () => ({
      tools: state.toolModels.choices,
      paid_ack_unix: state.toolModels.paidAckUnix,
    }),
    set_tool_model: ({ tool, source, modelIds, acknowledgePaidUse }) => {
      const slug = String(tool);
      if (!state.tools.some((t) => t.slug === slug)) throw `unknown tool slug "${slug}"`;
      if (source !== "tool" && source !== "gate") throw `unknown model source "${String(source)}"`;
      if (source === "gate" && acknowledgePaidUse === true && state.toolModels.paidAckUnix === null) {
        state.toolModels.paidAckUnix = 1787740800;
      }
      state.toolModels.choices[slug] = {
        source,
        model_ids: (modelIds as string[]) ?? [],
      };
      return null;
    },
    gate_credits: () =>
      JSON.stringify({
        generatedAt: "2026-08-25T10:00:00.000Z",
        org: { orgId: "org-e2e", name: state.account?.org_name ?? null },
        ...state.toolModels.credits,
      }),
    gate_model_catalogue: () =>
      JSON.stringify({ object: "list", data: state.toolModels.catalogue }),

    // ---- account
    get_account: () => state.account,
    get_account_key_prefix: () => state.accountKeyPrefix,
    backfill_account_key_prefix: () => state.accountKeyPrefix,
    save_account: ({ baseUrl, apiKey }) => {
      state.account = {
        gateway_base_url: baseUrl,
        has_api_key: !!apiKey,
        // A pasted key sets the mode outright; saving without one only
        // creates the account, and the sign-in that follows records OAuth.
        auth_mode: apiKey ? "api_key" : (state.account?.auth_mode ?? "oauth"),
        org_id: state.account?.org_id ?? null,
        org_name: state.account?.org_name ?? null,
      };
      return null;
    },
    clear_account: () => {
      state.account = null;
      state.oauth = { signed_in: false, email: null, expires_at_unix: 0 };
      for (const t of state.tools) {
        if (t.status.kind === "connected") t.status = { kind: "detected" };
      }
      return null;
    },
    switch_gateway: ({ baseUrl }) => {
      if (state.account) {
        state.account.gateway_base_url = baseUrl;
        state.account.has_api_key = false;
      }
      return null;
    },

    // ---- OAuth
    oauth_status: () => state.oauth,
    oauth_begin_login: () => {
      state.oauth = {
        signed_in: true,
        email: state.oauth.email ?? "dev@constellationnetwork.io",
        expires_at_unix: 4102444800,
      };
      if (state.account) state.account.auth_mode = "oauth";
      return state.oauth;
    },
    oauth_sign_out: () => {
      state.oauth = { signed_in: false, email: null, expires_at_unix: 0 };
      return null;
    },
    set_auth_mode: ({ oauth }) => {
      if (state.account) state.account.auth_mode = oauth ? "oauth" : "api_key";
      return null;
    },
    oauth_list_orgs: () => state.orgs,
    set_org: ({ orgId, orgName }) => {
      if (state.account) {
        state.account.org_id = orgId;
        state.account.org_name = orgName;
      }
      return null;
    },

    // ---- window / tray plumbing
    pin_popover: () => null,
    unpin_popover: () => null,
    open_onboarding_window: () => null,

    // ---- proxy
    proxy_status: () => state.proxy,
    proxy_list_domains: () => state.proxy.domains,
    proxy_enable: () => {
      state.proxy.running = true;
      state.proxy.port = 8899;
      state.proxy.pac_port = state.platform === "linux" ? null : 8898;
      // Enabling trusts the CA - the one step that prompts, and only when it
      // isn't trusted already.
      state.proxy.ca_trusted = true;
      return state.proxy;
    },
    proxy_disable: () => {
      state.proxy.running = false;
      state.proxy.port = null;
      state.proxy.pac_port = null;
      // The CA stays trusted so re-enabling is promptless.
      return state.proxy;
    },
    proxy_set_domain: ({ slug, enabled }) => {
      const d = state.proxy.domains.find((x) => x.slug === slug);
      if (!d) throw new Error(`fake backend: no such domain ${slug}`);
      d.enabled = enabled;
      for (const p of state.providers) {
        if (p.domain_slugs.includes(slug)) syncProvider(p.slug);
      }
      return state.proxy;
    },
    proxy_trust_ca: () => {
      state.proxy.ca_trusted = true;
      return state.proxy;
    },
    proxy_untrust_ca: () => {
      state.proxy.ca_trusted = false;
      return state.proxy;
    },
    proxy_set_env_export: ({ enabled }) => {
      state.proxy.env_export_opted_in = enabled;
      return state.proxy;
    },

    // ---- providers
    list_providers: () => state.providers,
    provider_enable: ({ slug }) => {
      const p = provider(slug);
      for (const s of p.tool_slugs) {
        const t = state.tools.find((x) => x.slug === s);
        if (t && t.status.kind !== "not_installed") t.status = { kind: "connected" };
      }
      // Proxy domains only follow when the engine is already running -
      // enabling a provider never starts it.
      if (state.proxy.running) {
        for (const s of p.domain_slugs) {
          const d = state.proxy.domains.find((x) => x.slug === s);
          if (d) d.enabled = true;
        }
      }
      return syncProvider(slug);
    },
    provider_disable: ({ slug }) => {
      const p = provider(slug);
      for (const s of p.tool_slugs) {
        const t = state.tools.find((x) => x.slug === s);
        if (t && t.status.kind === "connected") t.status = { kind: "detected" };
      }
      if (state.proxy.running) {
        for (const s of p.domain_slugs) {
          const d = state.proxy.domains.find((x) => x.slug === s);
          if (d) d.enabled = false;
        }
      }
      return syncProvider(slug);
    },

    // ---- launch at login
    launch_at_login_status: () => state.launchAtLogin,
    set_launch_at_login: ({ enabled }) => {
      state.launchAtLogin = { enabled, pending_disable: false };
      return null;
    },
    set_updater_relaunching: () => null,

    // ---- preferences
    // A copy, not the live object. Real IPC serialises every response, so the
    // frontend always gets a fresh value; handing out the reference this stub
    // mutates in place meant `setPrefs` received an identical object, React
    // skipped the re-render, and a preference change was invisible to anything
    // derived from it.
    get_preferences: () => ({ ...state.preferences }),
    set_routing_health_notifications: ({ enabled }) => {
      state.preferences.routing_health_notifications = enabled as boolean;
      return null;
    },
    set_blocked_event_notifications: ({ enabled }) => {
      state.preferences.blocked_event_notifications = enabled as boolean;
      return null;
    },
    set_flagged_event_notifications: ({ enabled }) => {
      state.preferences.flagged_event_notifications = enabled as boolean;
      return null;
    },
    set_security_notification_sound: ({ enabled }) => {
      state.preferences.security_notification_sound = enabled as boolean;
      return null;
    },
    // The live security-event feed (AG-578). The real backend holds the
    // connection and pushes; here a spec pushes with `app.emit`, which is the
    // same thing from the window's side.
    security_feed_state: () => state.securityFeed.state,
    security_feed_recent: () => state.securityFeed.events.map((e) => ({ ...e })),
    security_feed_retry: () => null,
    set_share_diagnostics: ({ enabled }) => {
      state.preferences.share_diagnostics = enabled as boolean;
      // Answering is what the real command records too, and it is what dismisses
      // the onboarding step.
      state.preferences.share_diagnostics_recorded = true;
      return null;
    },

    // Derived from the proxy state the spec set rather than stubbed free-hand, so
    // a report cannot describe an install the rest of the fake backend is not
    // running. The window's diagnostics dialog reads this; before it did, four
    // sections of the report were hard-coded to unknown.
    diagnostics: () => ({
      os_name: "macOS 15.3 (24D60)",
      os_kernel: "",
      arch: "aarch64",
      data_dir: "/Users/e2e/Library/Application Support/gate-connect",
      ca_cert_path: "/Users/e2e/Library/Application Support/gate-connect/ca.crt",
      ca_cert_present: state.proxy.ca_trusted,
      routing_intent: state.proxy.running,
      persisted_engine_proxy_url: state.proxy.running
        ? `http://127.0.0.1:${state.proxy.port}`
        : null,
      relay_base_url: state.proxy.relay_base_url,
      exported_proxy_url: state.proxy.env_export_opted_in
        ? `http://127.0.0.1:${state.proxy.port}`
        : null,
      system_proxy: state.proxy.running ? "PAC http://127.0.0.1:8" : null,
    }),

    // ---- agents / quit
    install_id: () => state.installId,
    // Mirrors the Rust resolution: the stored override, or the hostname.
    device_name: () => state.preferences.device_name ?? state.hostName,
    set_device_name: ({ name }) => {
      const trimmed = (name as string).trim();
      state.preferences.device_name = trimmed === "" ? null : trimmed;
      return null;
    },
    routed_clients_stale: () => state.routedClientsStale,
    running_agents_count: () => state.runningAgents,
    // `only` is a list of tool slugs, or null for "every tool". Mirrors the
    // Rust `AGENT_PROCESSES` table: a slug with no process name of its own
    // (`hermes`, `openclaw`, `env-proxy`, a proxy domain key) matches nothing
    // rather than everything, which is the whole point of the filter.
    running_agents: ({ only }) => {
      const names = agentNamesFor(only as string[] | null | undefined);
      return {
        scanned_names: names,
        agents: state.runningAgentNames
          .map((name, i) => ({
            name,
            pid: 100 + i,
            started_at_unix: 1_700_000_000,
            predates_routing: true,
          }))
          .filter((a) => names.includes(a.name.toLowerCase())),
      };
    },
    stale_agents_count: () => state.staleAgents,
    // Mirrors `routing_health::verdict_for`'s precedence over the state a spec
    // can actually set. The relay is hosted by the engine, so `proxy.running`
    // stands in for relay reachability, and `staleAgents` for a process that
    // predates the last routing change. Derived rather than stubbed per-slug so
    // a spec cannot set up a verdict that the real backend could never produce.
    // The Gate route as the verdict names it: the account's own gateway URL, or
    // the fallback the backend uses when it cannot be read.
    routing_verdicts: () =>
      state.tools.map((t) => {
        const attention = (reason: string, next_action: string) => ({
          slug: t.slug,
          state: "needs_attention",
          reason,
          next_action,
          // Only the reopen verdict carries the pair, and which way round it is
          // depends on which change the process missed - a managed config means
          // the tool is still going direct. Mirrors `routing_verdicts_now`.
          route_in_use:
            reason === "reopen_required"
              ? t.status.kind === "connected"
                ? t.default_upstream_url
                : gateRoute()
              : null,
          requested_route:
            reason === "reopen_required"
              ? t.status.kind === "connected"
                ? gateRoute()
                : t.default_upstream_url
              : null,
        });
        const plain = (verdictState: string) => ({
          slug: t.slug,
          state: verdictState,
          reason: null,
          next_action: null,
          route_in_use: null,
          requested_route: null,
        });
        if (t.status.kind === "not_installed") return plain("not_installed");
        if (t.status.kind === "error")
          return attention("verification_failed", "retry_check");
        if (t.status.kind === "drifted")
          return attention("configuration_changed", "apply_gate_configuration");
        if (t.status.kind === "detected")
          return state.staleAgents > 0
            ? attention("reopen_required", "reopen_tool")
            : plain("off");
        if (!state.proxy.running) return attention("connection_problem", "reconnect");
        if (state.staleAgents > 0) return attention("reopen_required", "reopen_tool");
        return plain("on");
      }),
    close_running_agents: ({ only }) => {
      const names = agentNamesFor(only as string[] | null | undefined);
      const doomed = state.runningAgentNames.filter((n) => names.includes(n.toLowerCase()));
      state.runningAgentNames = state.runningAgentNames.filter(
        (n) => !names.includes(n.toLowerCase()),
      );
      // The unfiltered count probe has no per-name breakdown to subtract from,
      // so a scoped close leaves it alone; only a close-everything zeroes it.
      const n = doomed.length || (only == null ? state.runningAgents : 0);
      if (only == null) {
        state.runningAgents = 0;
        state.staleAgents = 0;
      }
      return n;
    },
    quit_app: () => null,
    // Window choreography the tray popover invokes: revealing the main window
    // and requesting the tray-menu quit are Rust-side effects with nothing to
    // model here - the call log is what a spec asserts on.
    reveal_popover: () => null,
    request_app_quit: () => null,
    pending_quit_tools: () => {
      const pending = state.pendingQuitTools;
      state.pendingQuitTools = null;
      return pending;
    },
    disconnect_tools_for_quit: () => {
      // A tool the teardown could not put back stays connected, which is what
      // leaves it pointing at a relay about to die.
      for (const t of state.tools) {
        if (t.status.kind === "connected" && !state.quitLeftBehind.includes(t.name)) {
          t.status = { kind: "detected" };
        }
      }
      return state.quitLeftBehind;
    },

    // ---- analytics seam
    // Drains, like the real buffer: a second call returns nothing.
    drain_backend_errors: () => state.backendErrors.splice(0),

    // ---- interrupted restore
    // Copies, for the reason `get_preferences` does: real IPC serialises every
    // response, and handing out the live object hides mutations from React.
    pending_restore: () => ({
      providers: [...state.pendingRestore.providers],
      tools: [...state.pendingRestore.tools],
    }),
    recovery_summary: () => recoverySummary(),
    retry_restore_entry: ({ slug }) => {
      state.retryCalls.push(slug as string);
      const stuck = state.pendingResumeKeeps.includes(slug as string);
      if (!stuck) {
        // Mirrors `restore_one`: the slug leaves its snapshot only once it is
        // actually back, and the journal records what happened to it.
        state.pendingRestore = {
          providers: state.pendingRestore.providers.filter((e) => e.slug !== slug),
          tools: state.pendingRestore.tools.filter((e) => e.slug !== slug),
        };
        if (state.restoreJournal) {
          for (const entry of state.restoreJournal.entries) {
            if (entry.slug === slug) entry.outcome = "restored";
          }
        }
      } else if (state.restoreJournal && state.retryErrors.includes(slug as string)) {
        for (const entry of state.restoreJournal.entries) {
          if (entry.slug === slug) entry.outcome = "write_failed";
        }
      }
      return {
        error: state.retryErrors.includes(slug as string)
          ? `writing ${slug} failed`
          : null,
        pending: {
          providers: [...state.pendingRestore.providers],
          tools: [...state.pendingRestore.tools],
        },
      };
    },
    // Read back from the tools' own state, never from what a teardown believed
    // it wrote - the whole point of the report. Same bucket rules as Rust:
    // managed means still on Gate's values, clean-plus-stale-process means
    // waiting for a reopen, an unreadable config is its own answer.
    teardown_report: () => {
      const bucket = (next_action: string) => (t: { slug: string; name: string }) => ({
        slug: t.slug,
        name: t.name,
        next_action,
      });
      const installed = state.tools.filter((t) => t.status.kind !== "not_installed");
      return {
        defaults: installed
          .filter((t) => t.status.kind === "detected" && state.staleAgents === 0)
          .map(bucket("none")),
        still_gate: installed
          .filter((t) => t.status.kind === "connected" || t.status.kind === "drifted")
          .map(bucket("retry_disconnect")),
        awaiting_reopen: installed
          .filter((t) => t.status.kind === "detected" && state.staleAgents > 0)
          .map(bucket("reopen_tool")),
        failed: installed.filter((t) => t.status.kind === "error").map(bucket("retry_check")),
      };
    },
    resume_restore: () => {
      // Mirrors restore_all: entries that fail stay recorded, the rest clear.
      const keep = (e: { slug: string }) => state.pendingResumeKeeps.includes(e.slug);
      state.pendingRestore = {
        providers: state.pendingRestore.providers.filter(keep),
        tools: state.pendingRestore.tools.filter(keep),
      };
      return {
        providers: [...state.pendingRestore.providers],
        tools: [...state.pendingRestore.tools],
      };
    },

    // ---- event plugin
    "plugin:event|listen": ({ event, handler }) => {
      let ids = listeners.get(event);
      if (!ids) {
        ids = new Set();
        listeners.set(event, ids);
      }
      ids.add(handler);
      // The real plugin returns an event id; the unlisten path hands it back.
      return handler;
    },
    "plugin:event|unlisten": ({ event, eventId }) => {
      listeners.get(event)?.delete(eventId);
      callbacks.delete(eventId);
      return null;
    },
    "plugin:event|emit": () => null,
    "plugin:event|emit_to": () => null,

    // ---- updater: no update available, so UpdatePanel stays out of the way.
    "plugin:updater|check": () => null,
  };

  const invoke = (cmd: string, args: Record<string, unknown> = {}) => {
    calls.push({ cmd, args: args ?? {} });
    const failure = state.failures[cmd];
    if (failure !== undefined) return Promise.reject(failure);
    const handler = commands[cmd];
    if (handler) {
      try {
        return Promise.resolve(handler(args ?? {}));
      } catch (err) {
        return Promise.reject(String(err));
      }
    }
    // Window/opener/process and any other plugin surface the app touches
    // incidentally: record it and succeed. An unknown *app* command is a typo
    // or a command the fake backend hasn't learned yet, and must be loud.
    if (cmd.startsWith("plugin:")) return Promise.resolve(null);
    return Promise.reject(`fake backend: unhandled command ${cmd}`);
  };

  (window as any).__TAURI_INTERNALS__ = {
    invoke,
    transformCallback(callback: (payload: unknown) => void) {
      const id = nextId++;
      callbacks.set(id, callback);
      return id;
    },
    unregisterCallback(id: number) {
      callbacks.delete(id);
    },
    convertFileSrc: (path: string) => path,
    metadata: {
      currentWindow: { label: state.windowLabel },
      currentWebview: { windowLabel: state.windowLabel, label: state.windowLabel },
    },
  };

  (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener(event: string, eventId: number) {
      listeners.get(event)?.delete(eventId);
      callbacks.delete(eventId);
    },
  };

  (window as any).__GATE_E2E__ = { state, calls, emit } satisfies E2EHandle;
}
