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

/** Hold the popover open across a call that raises a system dialog: the dialog
 *  takes focus, and without the pin the dismiss-on-blur handler would hide the
 *  window along with the copy telling the user what to click. Always paired
 *  with `unpinPopover` in a `finally`. */
export const pinPopover = () => invoke<void>("pin_popover");

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
  /** Loopback base URL config-routed tools are pointed at; null before a relay
   * port has ever been bound. Non-secret - it is already written verbatim into
   * each tool's own config file. The drift review shows it, because approving an
   * overwrite means seeing what it writes. */
  relay_base_url: string | null;
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
  /** Slugs of this family's chat-protocol domains: listed under the family,
   * excluded from its switch. Kept apart from `domain_slugs` rather than
   * merged with a flag, because that field means "what the family switch
   * flips" everywhere it is read, and these must never be flipped by it -
   * they intercept a session-cookie surface, not a key-brokered one. */
  chat_domain_slugs: string[];
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

/** Why a tool is not verifiably routing. Closed set, mirroring
 * `routing_health::Reason` - a sixth value would need a next action and a
 * recovery path to go with it. */
export type VerdictReason =
  | "configuration_changed"
  | "reopen_required"
  | "connection_problem"
  | "access_problem"
  | "verification_failed";

/** The one action offered for a reason. One-to-one with {@link VerdictReason};
 * the backend derives it so the pair cannot drift apart. */
export type VerdictNextAction =
  | "apply_gate_configuration"
  | "reopen_tool"
  | "reconnect"
  | "sign_in"
  | "retry_check";

/** What one tool is *doing*, as opposed to what its config says
 * ({@link Tool.status}) or what the user asked for. `reason` and `next_action`
 * are set only when `state` is `needs_attention`. */
export interface Verdict {
  slug: string;
  state: "not_installed" | "on" | "off" | "needs_attention";
  reason: VerdictReason | null;
  next_action: VerdictNextAction | null;
}

/** What every config-routed tool is actually doing: config state, plus a
 * loopback check that the relay answers, plus whether the tool's process
 * predates the last routing change.
 *
 * Separate from {@link listTools} because this does network I/O and walks the
 * process table, so it must not sit on a render path. It does **not** prove the
 * tool sent traffic - nothing attributes requests to a tool today. */
export const routingVerdicts = () => invoke<Verdict[]>("routing_verdicts");

/** One running AI tool. No command line by design: argv on these routinely
 * holds prompts, paths and occasionally a key, and this list is built to be
 * pasted into a support thread. */
export interface RunningAgent {
  /** Process name as the OS spells it, original case. "Claude" is the desktop
   * app, "claude" the CLI. */
  name: string;
  pid: number;
  /** Process start, Unix seconds. 0 when the platform wouldn't say. */
  started_at_unix: number;
  /** Started before routing last came up, so it resolved its connection
   * pre-Gate and needs a restart to route. Same rule as
   * {@link staleAgentsCount}. */
  predates_routing: boolean;
}

export interface RunningAgents {
  /** The process names this scan looks for. Reported so an empty list reads
   * as "none of these were running" rather than "no AI tools are running" -
   * the scan doesn't cover every integration. */
  scanned_names: string[];
  /** Oldest first, so two reports from the same machine stay diffable. */
  agents: RunningAgent[];
}

/** The running agents themselves rather than a count: name, pid, start time,
 * and whether each predates routing. Same process set and staleness rule as
 * the two count probes above. */
export const runningAgents = () => invoke<RunningAgents>("running_agents");

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

/** One thing a routing restore recorded and has not finished. `name` falls back to
 * the slug when the provider or tool has left the registry since - naming it beats
 * dropping it from a list the user is being asked to act on. */
export interface PendingEntry {
  slug: string;
  name: string;
}

/** Routing work that was written down and did not complete. Empty in the normal
 * case. The snapshots have always recorded unfinished work - `restore_all` keeps
 * failures in the file and clears it only once everything is back - but nothing
 * read them for display, so a half-finished restore left some tools routing, some
 * not, and no statement anywhere that Gate knew. */
export interface PendingRestore {
  providers: PendingEntry[];
  tools: PendingEntry[];
}

/** What a restore still owes. Read-only and cheap: opens no config, starts
 * nothing, safe on a status refresh. */
export const pendingRestore = () => invoke<PendingRestore>("pending_restore");

/** Finish an interrupted restore, and report what is still outstanding.
 * `restore_all`'s existing retry semantics mean this repeats no completed write.
 * Returns the remaining state rather than void, because a partial success is the
 * interesting case and must not read as done. */
export const resumeRestore = () => invoke<PendingRestore>("resume_restore");
/** Non-secret Settings choices. Every field defaults to `true`, and an absent
 * field in the stored file loads as `true` - so a switch reads On before anything
 * has ever been written, which is what lets Settings show a truthful default.
 *
 * Only the preferences that currently gate something are here. Per-category
 * security-event notifications and a sound toggle belong with the live event feed,
 * which does not exist yet; a switch that gates nothing would tell the user they
 * had turned something off. */
export interface Preferences {
  /** Native notifications about routing itself - an expired session, a quit that
   * could not put a tool back. The two the app actually fires. */
  routing_health_notifications: boolean;
  /** Whether Gate Connect may send diagnostic data. Onboarding records the first
   * answer; Settings changes it after. Nothing is uploaded by setting it. */
  share_diagnostics: boolean;
  /** Whether the person has ever *answered* the question, rather than having the
   * default applied for them. False on installs that predate the field, which is
   * why they see the onboarding step once - consent nobody was asked for is not
   * consent. `setShareDiagnostics` sets it from either caller. */
  share_diagnostics_recorded: boolean;
}

export const getPreferences = () => invoke<Preferences>("get_preferences");

export const setRoutingHealthNotifications = (enabled: boolean) =>
  invoke<void>("set_routing_health_notifications", { enabled });

export const setShareDiagnostics = (enabled: boolean) =>
  invoke<void>("set_share_diagnostics", { enabled });

/** What a restore did to one entry on its last attempt. Closed set, mirroring
 * `recovery::Outcome`, and every member comes from the restore's own control flow
 * rather than from matching an error message. */
export type RestoreOutcome =
  | "pending"
  | "restored"
  | "write_failed"
  | "not_installed"
  | "unknown"
  | "deferred_signed_out";

export interface RestoreRecord {
  slug: string;
  name: string;
  kind: "provider" | "tool";
  outcome: RestoreOutcome;
  /** Unix seconds; 0 when the clock could not be read, which the UI shows as
   * unknown rather than as 1970. */
  at_unix: number;
}

/** The last restore, entry by entry. Explanation only - the snapshots behind
 * {@link pendingRestore} are what a resume actually works from. */
export interface RestoreJournal {
  updated_unix: number;
  requested_routing_on: boolean;
  entries: RestoreRecord[];
}

/** What the last restore did, or null when there is nothing to explain: a restore
 * that completed clears its journal. */
export const restoreJournal = () => invoke<RestoreJournal | null>("restore_journal");

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

// ---- Diagnostics ----

/** The backend half of the copy-pasteable support report: facts about this
 * install the webview cannot see any other way. The rest of the report is
 * composed from state the popover already holds, so what gets pasted matches
 * what is on screen. Carries no credential by construction - see
 * `crates/core/src/diagnostics.rs`. */
export interface Diagnostics {
  /** OS marketing name and version ("Ubuntu 25.10", "macOS 15.3 (24D60)"). */
  os_name: string;
  /** Kernel release. Linux only; empty elsewhere. */
  os_kernel: string;
  arch: string;
  data_dir: string | null;
  ca_cert_path: string | null;
  /** Whether the CA's public cert is actually on disk. Trusted-but-absent is
   * a real state and otherwise invisible. */
  ca_cert_present: boolean;
  /** The persisted "routing should be on" intent, as opposed to whether it
   * is on now. The two disagreeing is the commonest report we get. */
  routing_intent: boolean;
  persisted_engine_proxy_url: string | null;
  relay_base_url: string | null;
  /** The proxy URL currently in the user's environment, read back from the OS
   * rather than from our own record. */
  exported_proxy_url: string | null;
  /** The OS proxy setting (PAC on macOS/Windows, the drop-in on Linux), read
   * back live: the channel that routes GUI apps rather than CLI tools. */
  system_proxy: string | null;
}

/** One snapshot for the diagnostics panel. Best-effort throughout: never
 * rejects for a field it could not resolve. On macOS it shells out per
 * network service, so call it on an explicit user action, never on a poll. */
export const diagnostics = () => invoke<Diagnostics>("diagnostics");
