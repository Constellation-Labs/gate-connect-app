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
  status: Status;
}

export type AuthMode = "api_key" | "oauth";

export interface Account {
  gateway_base_url: string;
  has_api_key: boolean;
  /** Which credential the account authenticates with. Drives sign-in routing
   * and whether the legacy key controls show in Settings. */
  auth_mode: AuthMode;
  /** Selected org (OAuth mode). Both null until the user picks one; an OAuth
   * account with no org routes to the picker. */
  org_id: string | null;
  org_name: string | null;
}

/** One organization the signed-in user may act on, from GET /v1/me/orgs. */
export interface Org {
  /** Org UUID - sent back as X-Gate-Org-Id (not the slug). */
  orgId: string;
  name: string;
  slug: string;
  role: string;
}

/** Cognito OAuth sign-in state, mirrored from the keychain token bundle. */
export interface OAuthStatus {
  signed_in: boolean;
  email: string | null;
  /** Access-token expiry as a Unix timestamp; 0 when signed out. */
  expires_at_unix: number;
}

export const listTools = () => invoke<Tool[]>("list_tools");

/** Point one tool's own config at Gate. Auto-enables the proxy engine when a
 * relay-routed config needs it (idempotent if already running). */
export const connectTool = (slug: string, upstreamUrl: string) =>
  invoke<Status>("connect_tool", { slug, upstreamUrl });

/** Revert one tool's config to its pre-Gate state. */
export const disconnectTool = (slug: string) => invoke<Status>("disconnect_tool", { slug });

export const hasUpstreamCredential = (slug: string) => invoke<boolean>("has_upstream_credential", { slug });

export const saveUpstreamApiKey = (slug: string, apiKey: string) =>
  invoke<void>("save_upstream_api_key", { slug, apiKey });

export const clearUpstreamCredential = (slug: string) => invoke<void>("clear_upstream_credential", { slug });

export const getAccount = () => invoke<Account | null>("get_account");

/** Leading characters of the stored Gate key, for the reveal control in
 * Settings. Reads the prefix recorded in the account config, not the keychain.
 * Returns null when no key is stored. */
export const getAccountKeyPrefix = () => invoke<string | null>("get_account_key_prefix");

/** Fallback reveal for accounts saved before the prefix was recorded on disk:
 * reads the key from the keychain (may prompt), backfills the prefix into the
 * config, and returns it. Call only after the user confirms the reveal. */
export const backfillAccountKeyPrefix = () => invoke<string | null>("backfill_account_key_prefix");

export const saveAccount = (baseUrl: string, apiKey: string | null) =>
  invoke<void>("save_account", { baseUrl, apiKey });

export const clearAccount = () => invoke<void>("clear_account");

// ---- OAuth (Cognito) ----
//
// The successor to the pasted API key: sign in through the Cognito Hosted UI in
// the browser, capture the redirect on a loopback listener, and store the token
// bundle in the keychain. The relay / MITM engine inject it live, so no
// credential is written to any config file.

/** Run one interactive sign-in: opens the Hosted UI in the browser and blocks
 * (off the main thread) until the redirect is captured and tokens are stored,
 * or the flow times out. Resolves with the resulting status. */
export const oauthBeginLogin = () => invoke<OAuthStatus>("oauth_begin_login");

/** Current sign-in status (signed in, email, expiry). Cheap keychain read. */
export const oauthStatus = () => invoke<OAuthStatus>("oauth_status");

/** Forget the stored OAuth tokens. Leaves auth mode at OAuth so the popover
 * shows the sign-in prompt again rather than the legacy key form. */
export const oauthSignOut = () => invoke<void>("oauth_sign_out");

/** Set the auth mode explicitly. Used when choosing the legacy pasted-key path
 * from the sign-in screen; OAuth sign-in sets it implicitly. */
export const setAuthMode = (oauth: boolean) => invoke<void>("set_auth_mode", { oauth });

/** List the orgs the signed-in user may act on, for the picker. */
export const oauthListOrgs = () => invoke<Org[]>("oauth_list_orgs");

/** Persist the selected org and push X-Gate-Org-Id into a running engine/relay
 * live (no restart). */
export const setOrg = (orgId: string, orgName: string) =>
  invoke<void>("set_org", { orgId, orgName });

/** Dev-mode gateway switch: repoint the account at another environment and
 *  forget the stored Gate key, so the UI can prompt for a new one. */
export const switchGateway = (baseUrl: string) =>
  invoke<void>("switch_gateway", { baseUrl });

/** Release the first-launch pin so the popover resumes click-away dismissal.
 *  Called once the user interacts with the startup window. */
export const unpinPopover = () => invoke<void>("unpin_popover");

/** Open (or refocus) the full-size onboarding window. `source` tags the
 *  analytics events with how the intro was reached. */
export const openOnboardingWindow = (source: "firstrun" | "settings") =>
  invoke<void>("open_onboarding_window", { source });

// ---- built-in MITM proxy (macOS, Windows, Linux) ----

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
  /** Loopback port serving the PAC script (macOS/Windows; null on Linux). */
  pac_port: number | null;
  ca_trusted: boolean;
  /** Whether Gate puts its proxy in the shell environment - the channel that
   * routes command-line tools, as opposed to the OS setting that routes GUI
   * apps. A separate choice because those variables are machine-wide. */
  env_export_opted_in: boolean;
  /** Whether that choice can be offered at all. False on Linux, where the
   * environment variables *are* the system proxy and cannot be declined
   * without turning routing off - so the switch must not render there. */
  env_export_separable: boolean;
  domains: ProxyDomain[];
}

export const proxyStatus = () => invoke<ProxyState>("proxy_status");

export const proxyListDomains = () => invoke<ProxyDomain[]>("proxy_list_domains");

/** Turn the proxy on: starts the loopback engine, trusts the CA (the one
 * step that prompts, and only when not already trusted), and points the
 * system proxy at it. */
export const proxyEnable = () => invoke<ProxyState>("proxy_enable");

/** Turn the proxy off: stops the engine and restores the prior system
 * proxy, promptless. The CA stays trusted so re-enabling is promptless;
 * removing it is the separate, explicit proxyUntrustCa. */
export const proxyDisable = () => invoke<ProxyState>("proxy_disable");

/** Toggle a provider. Applied live when the engine is running - no restart,
 * no prompt. */
export const proxySetDomain = (slug: string, enabled: boolean) =>
  invoke<ProxyState>("proxy_set_domain", { slug, enabled });

export const proxyTrustCa = () => invoke<ProxyState>("proxy_trust_ca");

/** Turn the shell-environment channel on or off. Applies immediately rather
 * than at the next routing toggle, and the choice persists across restarts. */
export const proxySetEnvExport = (enabled: boolean) =>
  invoke<ProxyState>("proxy_set_env_export", { enabled });
export const proxyUntrustCa = () => invoke<ProxyState>("proxy_untrust_ca");

// ---- Providers (one switch per model provider) ----
//
// A provider orchestrates the config integration(s) and, when the proxy is
// already running (macOS / Windows / Linux), the matching proxy domain(s) - so
// the UI shows a single toggle without exposing the proxy-vs-config split. A
// provider with no CLI integration (OpenRouter) is proxy-only.

export interface ProviderState {
  slug: string;
  display_name: string;
  subtitle: string;
  /** Headline on/off: at least one of the provider's tools is routed. */
  enabled: boolean;
  /** Whether the switch can act now (a tool is installed or the proxy is
   * running). When false the UI should render the switch disabled. */
  available: boolean;
  /** Slugs of the config-file tools this provider's switch governs, so the
   * UI can show the coupling with the per-tool switches. */
  tool_slugs: string[];
  /** Slugs of the proxy domains this provider covers. With `tool_slugs`,
   * a family's whole membership - what Home's ledger groups by. */
  domain_slugs: string[];
}

export const listProviders = () => invoke<ProviderState[]>("list_providers");

/** Turn a provider on: configures installed tools and, if the proxy is already
 * running, enables its proxy domain(s) (macOS / Windows / Linux). Never
 * triggers an admin prompt. */
export const providerEnable = (slug: string) => invoke<ProviderState>("provider_enable", { slug });

/** Turn a provider off: reverts the tool config and disables its proxy
 * domain(s) if the proxy is running. */
export const providerDisable = (slug: string) => invoke<ProviderState>("provider_disable", { slug });

// ---- Launch at login ----
//
// Standalone user setting that owns the OS login item directly. Decoupled from
// the routing toggle: turning it on is what lets the app relaunch and re-route
// after a restart.

export interface LaunchAtLoginStatus {
  /** The user's choice, i.e. what the Settings toggle shows. */
  enabled: boolean;
  /** A deferred opt-out (toggled off while routing was on) whose OS
   * deregistration hasn't completed yet; the login-items list still shows
   * the app during this window. */
  pending_disable: boolean;
}

export const launchAtLoginStatus = () =>
  invoke<LaunchAtLoginStatus>("launch_at_login_status");

export const setLaunchAtLogin = (enabled: boolean) =>
  invoke<void>("set_launch_at_login", { enabled });

/** Mark (or unmark) the next exit as an updater-driven relaunch, so the exit
 * handler keeps the routing intent and the relaunched app restores routing.
 * Set after the update download completes, right before `install()` (Windows
 * exits from inside it); a quit while the download is still running is a
 * genuine user exit and must not carry the mark. Reset if the install fails. */
export const setUpdaterRelaunching = (relaunching: boolean) =>
  invoke<void>("set_updater_relaunching", { relaunching });

/** Whether the startup auto-enable brought routing back on a different local
 * port than the previous session (e.g. the first launch after upgrading from
 * a build without port persistence). Already-running AI apps may still point
 * at the dead old port, so the popover shows a restart notice. */
export const routedClientsStale = () => invoke<boolean>("routed_clients_stale");

/** Count running AI tools (same process set as {@link closeRunningAgents})
 * without touching them. Used to skip the routing-change takeover when there
 * is nothing to close. */
export const runningAgentsCount = () => invoke<number>("running_agents_count");

/** Running agent processes started *before* routing last came up - the ones
 * that genuinely need a restart to route. Drives the startup hint, so a
 * healthy restored session (agents launched after routing) stays quiet. */
export const staleAgentsCount = () => invoke<number>("stale_agents_count");

/** Terminate running AI tools (agent CLIs and the desktop apps sharing their
 * binary name, e.g. Claude Desktop's `Claude`) so their next launch picks up
 * the routing change. Resolves to how many processes were signalled; 0 means
 * none were running. */
export const closeRunningAgents = () => invoke<number>("close_running_agents");

/** Finish a quit the tray deferred to the popover: the backend buffers the
 * connected tool names and emits a `quit-requested` nudge instead of exiting
 * when config-routed tools would be left pointing at the dead relay. */
export const quitApp = () => invoke<void>("quit_app");

/** Hand over (and clear) the buffered quit request: the connected tool names
 * to show in the quit takeover, or null when no quit is pending. Swept once
 * at mount and again on each `quit-requested` nudge, so a Quit clicked
 * before the listener registered isn't lost. */
export const pendingQuitTools = () => invoke<string[] | null>("pending_quit_tools");

/** Quit-time teardown: snapshot + disconnect every enabled integration so the
 * CLI tools fall back to their original settings, leaving the routing intent
 * untouched so the next startup restore reapplies them. Fires the "restart
 * your CLI agents" system notification. */
export const disconnectToolsForQuit = () => invoke<void>("disconnect_tools_for_quit");

/** A backend failure buffered for the analytics seam. `context` names the
 * operation that failed (validated frontend-side against the known set);
 * `message` is the raw error chain - it stays on this machine, only the
 * classified title is sent. */
export interface BackendError {
  context: string;
  message: string;
}

/** Hand over (and clear) the backend's buffered analytics errors. Called once
 * at mount to sweep failures that predate the webview, then again on each
 * `backend-error-pending` nudge. */
export const drainBackendErrors = () => invoke<BackendError[]>("drain_backend_errors");
