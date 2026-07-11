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
}

/** Cognito OAuth sign-in state, mirrored from the keychain token bundle. */
export interface OAuthStatus {
  signed_in: boolean;
  email: string | null;
  /** Access-token expiry as a Unix timestamp; 0 when signed out. */
  expires_at_unix: number;
}

export const listTools = () => invoke<Tool[]>("list_tools");

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
  ca_trusted: boolean;
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

export const launchAtLoginStatus = () => invoke<boolean>("launch_at_login_status");

export const setLaunchAtLogin = (enabled: boolean) =>
  invoke<void>("set_launch_at_login", { enabled });
