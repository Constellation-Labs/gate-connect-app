import { invoke } from "@tauri-apps/api/core";

export type Status =
  | { kind: "not_installed" }
  | { kind: "detected" }
  | { kind: "connected" }
  | { kind: "drifted"; reason: string }
  | { kind: "error"; message: string };

export interface Tool {
  slug: string;
  name: string;
  upstream_provider_name: string;
  default_upstream_url: string;
  requires_upstream_credential: boolean;
  supports_claude_oauth_delegation: boolean;
  supports_migrate: boolean;
  status: Status;
}

export interface Account {
  gateway_base_url: string;
  has_api_key: boolean;
}

export const listTools = () => invoke<Tool[]>("list_tools");

export const toolStatus = (slug: string) => invoke<Status>("tool_status", { slug });

export const connectTool = (slug: string, upstreamUrl: string) => invoke<Status>("connect_tool", { slug, upstreamUrl });

export const disconnectTool = (slug: string) => invoke<Status>("disconnect_tool", { slug });

export const hasUpstreamCredential = (slug: string) => invoke<boolean>("has_upstream_credential", { slug });

export const saveUpstreamApiKey = (slug: string, apiKey: string) =>
  invoke<void>("save_upstream_api_key", { slug, apiKey });

/**
 * Delegate to the user's live Claude Code session: stores a sentinel so the
 * credential helper reads Claude Code's current access token on every request
 * (refresh is handled by Claude Code itself). Requires Claude Code to be
 * signed in; fails fast if no valid session is found. macOS may prompt once
 * to authorize Gate Connect reading the Claude Code keychain entry.
 */
export const saveUpstreamViaClaudeOauth = (slug: string) => invoke<void>("save_upstream_via_claude_oauth", { slug });

/**
 * Probe whether Claude Code has a usable signed-in session right now. Drives
 * the credential-source picker (pre-select / label the delegation option).
 */
export const detectClaudeCodeSession = () => invoke<boolean>("detect_claude_code_session");

export const clearUpstreamCredential = (slug: string) => invoke<void>("clear_upstream_credential", { slug });

export const getAccount = () => invoke<Account | null>("get_account");

export const saveAccount = (baseUrl: string, apiKey: string | null) =>
  invoke<void>("save_account", { baseUrl, apiKey });

export const clearAccount = () => invoke<void>("clear_account");

// ---- migrate from Cowork standard mode ----

export type MigrateBlocker =
  | { kind: "source_missing" }
  | { kind: "dest_missing" }
  | { kind: "cowork_running" }
  | {
      kind: "insufficient_disk_space";
      needed_bytes: number;
      available_bytes: number;
    };

export interface MigrateRoots {
  source_account: string;
  source_org: string;
  source_dir: string;
  dest_account: string;
  dest_org: string;
  dest_dir: string;
}

export interface MigrateDiscover {
  roots: MigrateRoots | null;
  plugins: number;
  scheduled: number;
  conversations: number;
  has_memory: boolean;
  has_enabled_plugins: boolean;
  has_preferences: boolean;
  artifacts: number;
  bytes_estimated: number;
  ready: boolean;
  blockers: MigrateBlocker[];
}

export interface MigrateOptions {
  include_plugins: boolean;
  include_scheduled: boolean;
  include_conversations: boolean;
  include_memory: boolean;
  include_enabled_plugins: boolean;
  include_preferences: boolean;
  include_artifacts: boolean;
  dry_run: boolean;
}

export const allMigrateOptions = (): MigrateOptions => ({
  include_plugins: true,
  include_scheduled: true,
  include_conversations: true,
  include_memory: true,
  include_enabled_plugins: true,
  include_preferences: true,
  include_artifacts: true,
  dry_run: false,
});

export interface CategoryReport {
  copied: number;
  skipped: number;
  failed: number;
  errors: string[];
}

export interface MigrateReport {
  per_category: Record<string, CategoryReport>;
  dry_run: boolean;
}

export const migrateDiscover = (slug: string) => invoke<MigrateDiscover>("migrate_discover", { slug });

export const migratePreview = (slug: string, options: MigrateOptions) =>
  invoke<MigrateReport>("migrate_preview", { slug, options });

export const migrateExecute = (slug: string, options: MigrateOptions) =>
  invoke<MigrateReport>("migrate_execute", { slug, options });

// ---- built-in MITM proxy (macOS only) ----

export interface ProxyDomain {
  slug: string;
  display_name: string;
  hosts: string[];
  upstream_url: string;
  rewrite_prefixes: string[];
  passthrough_prefixes: string[];
  enabled: boolean;
  /** Whether Gate can upstream this provider yet. Unsupported domains
   * render as disabled rows and can't be turned on. */
  supported: boolean;
}

export interface ProxyState {
  running: boolean;
  port: number | null;
  ca_trusted: boolean;
  domains: ProxyDomain[];
}

export const proxyStatus = () => invoke<ProxyState>("proxy_status");

export const proxyListDomains = () => invoke<ProxyDomain[]>("proxy_list_domains");

/** Turn the proxy on: starts the loopback engine, trusts the CA, and points
 * the system proxy at it. Triggers a single macOS admin prompt. */
export const proxyEnable = () => invoke<ProxyState>("proxy_enable");

/** Turn the proxy off: restores the prior system proxy and untrusts the CA
 * in one admin prompt. */
export const proxyDisable = () => invoke<ProxyState>("proxy_disable");

/** Toggle a provider. Applied live when the engine is running — no restart,
 * no prompt. */
export const proxySetDomain = (slug: string, enabled: boolean) =>
  invoke<ProxyState>("proxy_set_domain", { slug, enabled });

export const proxyTrustCa = () => invoke<ProxyState>("proxy_trust_ca");

export const proxyUntrustCa = () => invoke<ProxyState>("proxy_untrust_ca");
