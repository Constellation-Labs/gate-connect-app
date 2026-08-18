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

  const commands: Record<string, (args: Record<string, any>) => unknown> = {
    // ---- platform / app
    app_platform: () => state.platform,
    "plugin:app|version": () => state.version,

    // ---- tools
    list_tools: () => state.tools,
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

    // ---- agents / quit
    routed_clients_stale: () => state.routedClientsStale,
    running_agents_count: () => state.runningAgents,
    running_agents: () => ({
      scanned_names: ["claude", "codex", "opencode"],
      agents: state.runningAgentNames.map((name, i) => ({
        name,
        pid: 100 + i,
        started_at_unix: 1_700_000_000,
        predates_routing: true,
      })),
    }),
    stale_agents_count: () => state.staleAgents,
    // Mirrors `routing_health::verdict_for`'s precedence over the state a spec
    // can actually set. The relay is hosted by the engine, so `proxy.running`
    // stands in for relay reachability, and `staleAgents` for a process that
    // predates the last routing change. Derived rather than stubbed per-slug so
    // a spec cannot set up a verdict that the real backend could never produce.
    routing_verdicts: () =>
      state.tools.map((t) => {
        const attention = (reason: string, next_action: string) => ({
          slug: t.slug,
          state: "needs_attention",
          reason,
          next_action,
        });
        if (t.status.kind === "not_installed")
          return { slug: t.slug, state: "not_installed", reason: null, next_action: null };
        if (t.status.kind === "error")
          return attention("verification_failed", "retry_check");
        if (t.status.kind === "drifted")
          return attention("configuration_changed", "apply_gate_configuration");
        if (t.status.kind === "detected")
          return state.staleAgents > 0
            ? attention("reopen_required", "reopen_tool")
            : { slug: t.slug, state: "off", reason: null, next_action: null };
        if (!state.proxy.running) return attention("connection_problem", "reconnect");
        if (state.staleAgents > 0) return attention("reopen_required", "reopen_tool");
        return { slug: t.slug, state: "on", reason: null, next_action: null };
      }),
    close_running_agents: () => {
      const n = state.runningAgents || state.runningAgentNames.length;
      state.runningAgents = 0;
      state.staleAgents = 0;
      state.runningAgentNames = [];
      return n;
    },
    quit_app: () => null,
    pending_quit_tools: () => {
      const pending = state.pendingQuitTools;
      state.pendingQuitTools = null;
      return pending;
    },
    disconnect_tools_for_quit: () => {
      for (const t of state.tools) {
        if (t.status.kind === "connected") t.status = { kind: "detected" };
      }
      return null;
    },

    // ---- analytics seam
    drain_backend_errors: () => [],

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
      currentWindow: { label: "main" },
      currentWebview: { windowLabel: "main", label: "main" },
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
