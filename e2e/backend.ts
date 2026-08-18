/**
 * The fake Rust side, and the state it serves.
 *
 * These e2e tests run the real frontend bundle in a real browser: real
 * `App.tsx` orchestration, real `src/lib/api.ts`, real CSS at 360px, real
 * focus and keyboard behaviour. The one thing that isn't real is the Tauri
 * process, and this file is what stands in for it - a stateful in-page
 * backend that answers the same command names over the same IPC entry point
 * (`window.__TAURI_INTERNALS__.invoke`).
 *
 * Stateful, not stubbed: `provider_enable` mutates the provider and its
 * tools, so the next `list_providers` reflects the click. That is the whole
 * point of the layer - the vitest suites already cover each screen against
 * fixed props, and what they cannot see is App's orchestration deciding what
 * to re-read and re-render after a command lands.
 *
 * The Rust behaviour itself is covered elsewhere and deliberately not here:
 * `crates/core/tests/*_e2e.rs` for the engine, `ci/e2e/run.sh` for the real
 * tools against a real relay.
 */

/** Mirrors `Status` in src/lib/api.ts. */
export type ToolStatus =
  | { kind: "not_installed" }
  | { kind: "detected" }
  | { kind: "connected" }
  | { kind: "drifted"; reason: string }
  | { kind: "error"; message: string };

export interface ToolFixture {
  slug: string;
  name: string;
  upstream_provider_name: string;
  default_upstream_url: string;
  requires_upstream_credential: boolean;
  status: ToolStatus;
}

export interface ProviderFixture {
  slug: string;
  display_name: string;
  subtitle: string;
  enabled: boolean;
  available: boolean;
  tool_slugs: string[];
  domain_slugs: string[];
  /** The family's chat-protocol domains: listed under it on the ledger, never
   *  flipped by its switch. Separate from `domain_slugs` for exactly that
   *  reason - see `ProviderState` in src/lib/api.ts. */
  chat_domain_slugs: string[];
}

export interface DomainFixture {
  slug: string;
  display_name: string;
  hosts: string[];
  upstream_url: string;
  rewrite_prefixes: string[];
  passthrough_prefixes: string[];
  enabled: boolean;
  supported: boolean;
}

export interface ProxyFixture {
  running: boolean;
  port: number | null;
  pac_port: number | null;
  ca_trusted: boolean;
  /** Loopback base URL config-routed tools point at. Null before a relay port
      has been bound, which is what the drift dialog omits its Gate-route row
      for. */
  relay_base_url: string | null;
  env_export_opted_in: boolean;
  env_export_separable: boolean;
  domains: DomainFixture[];
}

export interface AccountFixture {
  gateway_base_url: string;
  has_api_key: boolean;
  auth_mode: "api_key" | "oauth";
  org_id: string | null;
  org_name: string | null;
}

export interface OAuthFixture {
  signed_in: boolean;
  email: string | null;
  expires_at_unix: number;
}

export interface OrgFixture {
  orgId: string;
  name: string;
  slug: string;
  role: string;
}

/** Everything the fake backend serves. Plain data: it crosses into the page
 *  through `addInitScript`, so it must survive structured cloning. */
export interface BackendState {
  platform: "macos" | "windows" | "linux";
  version: string;
  account: AccountFixture | null;
  oauth: OAuthFixture;
  orgs: OrgFixture[];
  accountKeyPrefix: string | null;
  proxy: ProxyFixture;
  tools: ToolFixture[];
  providers: ProviderFixture[];
  launchAtLogin: { enabled: boolean; pending_disable: boolean };
  /** Settings preferences, as `preferences.json` holds them. Both default on,
      which is what lets a switch read On before anything has been written. */
  preferences: {
    routing_health_notifications: boolean;
    share_diagnostics: boolean;
    /** Whether the diagnostic-data question has been ANSWERED, as opposed to
        defaulted. False sends first run through the diagnostics step; the
        default here is true so the existing specs reach the app shell. */
    share_diagnostics_recorded: boolean;
  };
  routedClientsStale: boolean;
  runningAgents: number;
  /** Process names the agent scan reports as running. `runningAgents` is the
      count the older probes return; this is what `running_agents` lists, and a
      spec that cares about the close-apps sequence sets it. */
  runningAgentNames: string[];
  staleAgents: number;
  pendingQuitTools: string[] | null;
  /** Failures the Rust side has buffered for the frontend to drain. Emptied by
      `drain_backend_errors`, like the real buffer. A spec sets this to check that
      a failure predating the webview - the startup auto-enable runs before either
      shell mounts - reaches the screen. */
  backendErrors: { context: string; message: string }[];
  /** Routing work a restore recorded and did not finish, as the provider snapshots
      hold it. A spec sets this to produce the recovery notice; `resume_restore`
      empties it, or leaves `pendingResumeKeeps` behind to model a partial retry. */
  pendingRestore: {
    providers: { slug: string; name: string }[];
    tools: { slug: string; name: string }[];
  };
  /** Slugs `resume_restore` should FAIL to finish, so they stay outstanding. */
  pendingResumeKeeps: string[];
  /** The read-only account of the last restore, or null when there is nothing to
      explain. Drives whether the recovery notice offers Review details. */
  restoreJournal: {
    updated_unix: number;
    requested_routing_on: boolean;
    entries: {
      slug: string;
      name: string;
      kind: "provider" | "tool";
      outcome:
        | "pending"
        | "restored"
        | "write_failed"
        | "not_installed"
        | "unknown"
        | "deferred_signed_out";
      at_unix: number;
    }[];
  } | null;
  /** Commands that should reject, keyed by command name. The value is the
   *  error string the backend "returns" - App classifies it exactly as it
   *  would a real Tauri rejection. */
  failures: Record<string, string>;
  /** Seeded into localStorage before the app boots. `gc.tour.v3.seen` is set
   *  by default, because an unseen tour hides the popover and opens the
   *  onboarding window - the first-launch path, which one spec asks for
   *  explicitly. */
  localStorage: Record<string, string>;
}

const CLAUDE_CODE: ToolFixture = {
  slug: "claude-code",
  name: "Claude Code",
  upstream_provider_name: "Anthropic",
  default_upstream_url: "https://api.anthropic.com",
  requires_upstream_credential: false,
  status: { kind: "detected" },
};

const CODEX: ToolFixture = {
  slug: "codex",
  name: "Codex",
  upstream_provider_name: "OpenAI",
  default_upstream_url: "https://api.openai.com/v1",
  requires_upstream_credential: false,
  status: { kind: "detected" },
};

const OPENCODE: ToolFixture = {
  slug: "opencode",
  name: "OpenCode",
  upstream_provider_name: "your existing providers",
  default_upstream_url: "https://api.anthropic.com",
  requires_upstream_credential: false,
  status: { kind: "detected" },
};

const ANTHROPIC_DOMAIN: DomainFixture = {
  slug: "anthropic",
  display_name: "Claude apps",
  hosts: ["api.anthropic.com"],
  upstream_url: "https://gateway.constellationgate.ai",
  rewrite_prefixes: ["/v1"],
  passthrough_prefixes: [],
  enabled: false,
  supported: true,
};

/** Claude Desktop's chat surface. Off, supported, and reached only through its
 *  own row: it carries the user's claude.ai session cookie rather than a
 *  brokered key. In `defaultState`'s domain list because the shipped catalog
 *  always carries it: the row exists on every ledger, switched off. */
export const CLAUDE_WEB_DOMAIN: DomainFixture = {
  slug: "claude-web",
  display_name: "Claude Desktop chat",
  hosts: ["claude.ai"],
  upstream_url: "https://claude.ai/api",
  rewrite_prefixes: ["/organizations/"],
  passthrough_prefixes: [],
  enabled: false,
  supported: true,
};

const OPENAI_DOMAIN: DomainFixture = {
  slug: "openai",
  display_name: "OpenAI apps",
  hosts: ["api.openai.com"],
  upstream_url: "https://gateway.constellationgate.ai",
  rewrite_prefixes: ["/v1"],
  passthrough_prefixes: [],
  enabled: false,
  supported: true,
};

/** The ChatGPT-subscription Responses endpoint: off, supported, and reached only
 *  through its own row, because what it carries is the user's subscription
 *  bearer rather than a brokered key. Also the switch OpenClaw's subscription
 *  model calls need, which its `connect` used to flip unasked. */
export const CHATGPT_DOMAIN: DomainFixture = {
  slug: "chatgpt",
  display_name: "ChatGPT (Codex subscription)",
  hosts: ["chatgpt.com"],
  upstream_url: "https://chatgpt.com/backend-api",
  rewrite_prefixes: ["/codex/responses"],
  passthrough_prefixes: [],
  enabled: false,
  supported: true,
};

/** A signed-in OAuth account with an org picked, routing off, three installed
 *  tools. The state most specs start from; each one narrows it with `merge`. */
export function defaultState(): BackendState {
  return {
    platform: "macos",
    version: "1.4.0",
    account: {
      gateway_base_url: "https://gateway.constellationgate.ai",
      has_api_key: false,
      auth_mode: "oauth",
      org_id: "org-1",
      org_name: "Constellation Labs",
    },
    oauth: {
      signed_in: true,
      email: "dev@constellationnetwork.io",
      expires_at_unix: 4102444800,
    },
    orgs: [
      { orgId: "org-1", name: "Constellation Labs", slug: "constellation", role: "admin" },
      { orgId: "org-2", name: "Side Project", slug: "side-project", role: "member" },
    ],
    accountKeyPrefix: null,
    proxy: {
      running: false,
      port: null,
      pac_port: null,
      ca_trusted: false,
      relay_base_url: "http://127.0.0.1:45981",
      env_export_opted_in: false,
      env_export_separable: true,
      domains: [
        { ...ANTHROPIC_DOMAIN },
        { ...CLAUDE_WEB_DOMAIN },
        { ...OPENAI_DOMAIN },
        { ...CHATGPT_DOMAIN },
      ],
    },
    tools: [{ ...CLAUDE_CODE }, { ...CODEX }, { ...OPENCODE }],
    providers: [
      {
        slug: "anthropic",
        display_name: "Claude",
        subtitle: "Claude Code and the Claude apps",
        enabled: false,
        available: true,
        tool_slugs: ["claude-code"],
        domain_slugs: ["anthropic"],
        chat_domain_slugs: [],
      },
      {
        slug: "openai",
        display_name: "OpenAI",
        subtitle: "Codex and the OpenAI apps",
        enabled: false,
        available: true,
        tool_slugs: ["codex"],
        domain_slugs: ["openai"],
        chat_domain_slugs: ["chatgpt"],
      },
    ],
    launchAtLogin: { enabled: false, pending_disable: false },
    preferences: {
      routing_health_notifications: true,
      share_diagnostics: true,
      share_diagnostics_recorded: true,
    },
    routedClientsStale: false,
    runningAgents: 0,
    runningAgentNames: [],
    staleAgents: 0,
    pendingQuitTools: null,
    backendErrors: [],
    pendingRestore: { providers: [], tools: [] },
    pendingResumeKeeps: [],
    restoreJournal: null,
    failures: {},
    localStorage: { "gc.tour.v3.seen": "1", "gc.oauth-offer.v1.seen": "1" },
  };
}

/** Deep-ish merge for the one level of nesting the fixtures use, so a spec can
 *  say `{ proxy: { running: true } }` without restating the domain list. */
export function merge(base: BackendState, patch: DeepPartial<BackendState>): BackendState {
  const out = { ...base } as Record<string, unknown>;
  for (const [key, value] of Object.entries(patch)) {
    if (value === undefined) continue;
    const current = out[key];
    const mergeable =
      value !== null &&
      typeof value === "object" &&
      !Array.isArray(value) &&
      current !== null &&
      typeof current === "object" &&
      !Array.isArray(current);
    out[key] = mergeable
      ? { ...(current as object), ...(value as object) }
      : value;
  }
  return out as unknown as BackendState;
}

/** One level of optionality: `{ proxy: { running: true } }` keeps the rest of
 *  the proxy fixture, and a nullable object field can still be set to null
 *  (`{ account: null }` is how a spec asks for first run). */
type Patchable<V> = V extends unknown[] ? V : V extends object ? Partial<V> : V;

export type DeepPartial<T> = { [K in keyof T]?: Patchable<T[K]> };
