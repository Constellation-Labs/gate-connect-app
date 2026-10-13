//! The part of the outside world a command reaches through: the app handle it
//! emits on, the OS it reports, and the store it writes. Behind a trait so the
//! command bodies in `commands.rs` can run in a unit test against
//! [`fake::FakePlatform`] instead of a running app (AG-903).
//!
//! Kept to what the extracted commands use today. A batch that moves another
//! command adds the methods that command needs, and nothing ahead of it.
//!
//! Split into sub-traits once it passed ~25 methods: [`Platform`] keeps what
//! has no better home, and requires [`Prefs`], [`Proxy`], [`Windows`],
//! [`Processes`], [`Account`], [`Session`] and [`Gateway`]. A command still
//! takes `&impl Platform` and reaches all of them through it.

use gate_connect_core::account::{AuthMode, BillingMode};
use gate_connect_core::activity::Failure;
use gate_connect_core::analytics::{Identity, SaveOutcome};
use gate_connect_core::logging::Level;
use gate_connect_core::oauth::SessionReading;
use gate_connect_core::org::Org;
use gate_connect_core::preferences::{GateModelMeta, ModelSource};
use gate_connect_core::security_feed::{FeedState, SecurityEvent};
use gate_connect_core::tool_models::ToolModelView;
use gate_connect_core::{Integration, ToolId};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) trait Platform:
    Prefs + Proxy + Windows + Processes + Account + Session + Gateway
{
    /// The OS identifier the UI tailors its copy on ("macos" / "windows" /
    /// "linux").
    fn os(&self) -> &'static str;

    /// The OS marketing name and version ("macOS 15.3 (24D60)").
    fn os_name(&self) -> String;

    /// The backend half of the diagnostics report. Shells out on macOS.
    fn diagnostics(&self) -> gate_connect_core::diagnostics::Diagnostics;

    /// This install's stable id, minted on first read.
    fn install_id(&self) -> anyhow::Result<String>;

    /// Write one line to the shared log, when logging is on.
    fn log(&self, level: Level, message: &str);

    /// Broadcast an event to every window. Fire and forget, like every emit in
    /// the shell: a window that is not listening is not a failure.
    fn emit(&self, event: &str, payload: Value);

    // ---- Tools ----

    /// Every integration the core knows, hidden ones included.
    fn integrations(&self) -> Vec<Box<dyn Integration>>;

    /// Find a tool's binary on this machine.
    fn resolve_binary(&self, well_known: &[&str], names: &[&str]) -> Option<PathBuf>;

    /// Ask a binary for its version. Spawns a process on a cold cache.
    fn version_at(&self, path: &Path) -> Option<String>;

    /// One row per model provider.
    fn providers(&self) -> Vec<gate_connect_core::provider::ProviderState>;

    /// Take Gate out of every tool it manages now. One error for every tool
    /// that would not let go, so the caller can abort.
    fn disconnect_all_managed(&self) -> anyhow::Result<()>;

    /// What each Gate-model tool's own config says, with drift folded back
    /// into the stored choice first.
    fn tool_model_states(&self) -> BTreeMap<&'static str, ToolModelView>;

    /// Store one tool's model choice and write it into the tool's config.
    /// True when the config changed.
    fn choose_tool_model(
        &self,
        tool: ToolId,
        source: ModelSource,
        model_ids: Vec<String>,
        acknowledge_paid_use: bool,
        meta: Vec<(String, GateModelMeta)>,
    ) -> anyhow::Result<bool>;

    // ---- Backend errors ----

    /// Queue a failure for the windows' analytics seam and nudge them to
    /// drain it.
    fn report_backend_error(&self, context: &'static str, message: String);

    // ---- Security feed ----

    /// The process's one live security-event feed.
    fn security_feed(&self) -> &dyn SecurityFeed;

    // ---- Updater ----

    /// Mark (or unmark) the next exit as an updater-driven relaunch.
    fn set_updater_relaunching(&self, relaunching: bool);

    // ---- Analytics ----

    /// Claim a once-per-install milestone. True exactly once per name.
    fn claim_milestone(&self, name: &str) -> anyhow::Result<bool>;

    /// Why local Cowork cannot run here, from Claude Desktop's settings.
    fn cowork_setting_missing(&self) -> Option<&'static str>;
}

/// The preferences file.
pub(crate) trait Prefs {
    /// The Settings choices, or the defaults when the file is missing or bad.
    fn preferences(&self) -> gate_connect_core::preferences::Preferences;

    /// What to call this machine.
    fn device_name(&self) -> String;

    /// Persist a name for this machine, or clear it.
    fn set_device_name(&self, name: &str) -> anyhow::Result<()>;

    /// Persist the native-notifications switch.
    fn set_notifications(&self, enabled: bool) -> anyhow::Result<()>;

    /// Persist the sound switch for security notifications.
    fn set_security_notification_sound(&self, enabled: bool) -> anyhow::Result<()>;

    /// Persist the answer to "may Gate Connect send diagnostic data".
    fn set_share_diagnostics(&self, enabled: bool) -> anyhow::Result<()>;

    /// Persist the provider domains Gate switched on for one tool.
    fn record_auto_enabled_domains(&self, tool: &str, domains: Vec<String>) -> anyhow::Result<()>;

    /// The provider domains Gate switched on for one tool.
    fn read_auto_enabled_domains(&self, tool: &str) -> Vec<String>;

    /// Record whether the last sign-out was the user's own.
    fn set_signed_out_deliberately(&self, deliberate: bool) -> anyhow::Result<()>;
}

/// The routing engine, as the shell sees it.
pub(crate) trait Proxy {
    /// Whether the startup enable moved the engine to a port that running
    /// clients may not know about.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn routed_clients_may_be_stale(&self) -> bool;

    /// Whether the startup enable has yet to settle.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn startup_enable_pending(&self) -> bool;

    /// The relay's loopback base URL, from the persisted port. `None` before
    /// one was ever bound.
    fn relay_base_url(&self) -> Option<String>;

    /// The proxy URL a tool being connected is pointed at. May start the
    /// environment forwarder.
    fn tool_proxy_url(&self) -> Option<String>;

    /// The engine's own proxy URL, while it is up.
    fn engine_proxy_url(&self) -> Option<String>;

    /// Retire the environment forwarder.
    fn stop_forwarder(&self);

    /// Persist whether routing is on.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn set_routing_intent(&self, enabled: bool) -> anyhow::Result<()>;

    /// Start the engine. Idempotent when it is already up.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn enable_engine(&self) -> anyhow::Result<()>;

    /// Stop the engine, so it cannot outlive the account it started from.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn shutdown_engine(&self) -> anyhow::Result<()>;

    /// Hand a running engine a new OAuth token (`""` takes it out).
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn refresh_engine_token(&self, token: &str);

    /// Hand a running engine a new Gate key (`""` takes it out).
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn refresh_engine_api_key(&self, api_key: &str);

    /// Have a running engine reread the billing mode.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn refresh_engine_mode(&self);

    /// Hand a running engine the selected org.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn refresh_engine_org(&self, org_id: &str);
}

/// The app's windows.
pub(crate) trait Windows {
    /// Pin the popover open, or let it dismiss on blur again.
    fn set_popover_pinned(&self, pinned: bool);

    /// Take one window's buffered backend failures, by label.
    fn drain_backend_errors(&self, label: &str) -> Vec<crate::BackendError>;
}

/// The AI tools' running processes.
pub(crate) trait Processes {
    /// Whether a tool's running process predates the config it should be
    /// reading, so it still routes the old way until reopened.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn reopen_pending(&self, slug: &str) -> bool;

    /// Restart Codex's app-server daemon once no Codex session is open, if it
    /// predates the config. Returns at once; the wait is on its own thread.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn refresh_codex_daemon_when_idle(&self, why: &'static str);
}

/// The account file, and the Gate key in the OS secret store.
pub(crate) trait Account {
    /// Drop a secret-store key the account file no longer vouches for.
    fn reconcile_account(&self) -> anyhow::Result<()>;

    /// True the first time it is asked in this run, false after. A reconcile
    /// failure is reported once per run, not once per read.
    fn first_reconcile_failure(&self) -> bool;

    /// The whole account, key included. `None` when there is none.
    fn account(&self) -> anyhow::Result<Option<gate_connect_core::account::Account>>;

    /// The gateway the account points at, without touching the secret store.
    fn account_base_url(&self) -> anyhow::Result<Option<String>>;

    /// Whether a Gate key is stored.
    fn has_api_key(&self) -> anyhow::Result<bool>;

    /// Which credential the account authenticates with.
    fn auth_mode(&self) -> anyhow::Result<AuthMode>;

    /// Who pays the upstream provider.
    fn billing_mode(&self) -> anyhow::Result<BillingMode>;

    /// The selected org, as id and name.
    fn selected_org(&self) -> anyhow::Result<Option<(String, String)>>;

    /// The recorded leading characters of the Gate key.
    fn api_key_prefix(&self) -> anyhow::Result<Option<String>>;

    /// Store the gateway, and the key when one is given.
    fn save_account(&self, base_url: &str, api_key: Option<&str>) -> anyhow::Result<()>;

    /// Persist which credential the account authenticates with.
    fn set_auth_mode(&self, mode: AuthMode) -> anyhow::Result<()>;

    /// Persist who pays the upstream provider.
    fn set_billing_mode(&self, mode: BillingMode) -> anyhow::Result<()>;

    /// Persist the selected org.
    fn set_org(&self, org_id: &str, org_name: &str) -> anyhow::Result<()>;

    /// Forget the account, its key and its OAuth session.
    fn clear_account(&self) -> anyhow::Result<()>;

    /// Point the account at another gateway and forget the key.
    fn switch_gateway(&self, base_url: &str) -> anyhow::Result<()>;
}

/// The OAuth session, and the analytics identity tied to it.
pub(crate) trait Session {
    /// The session as the injector sees it, refreshed when stale.
    fn session_reading(&self) -> SessionReading;

    /// Stop an interactive login that is still waiting for the browser.
    fn cancel_login(&self);

    /// Forget the stored OAuth tokens, and the analytics identity with them.
    fn clear_oauth_session(&self) -> anyhow::Result<()>;

    /// Whether the gateway refused the session, as opposed to it only being
    /// unreachable.
    fn session_rejected(&self) -> bool;

    /// Raise the dead-session signal: tray, windows, notification. Once per
    /// death.
    fn signal_session_dead(&self);

    /// Finish a forget a sign-out could not land. True when it forgot.
    fn forget_identity_if_signed_out(&self) -> anyhow::Result<bool>;

    /// What this install is identified as in analytics.
    fn analytics_identity(&self) -> Identity;

    /// Record a change of analytics identity. `on_stored` runs inside the
    /// core's lock with what was stored.
    fn save_identity(
        &self,
        identity: Identity,
        on_stored: &mut dyn FnMut(&Identity),
    ) -> anyhow::Result<SaveOutcome>;
}

/// Reads from the gateway, and the cache that holds them.
pub(crate) trait Gateway {
    /// The orgs the signed-in user may act on.
    fn list_orgs(&self) -> anyhow::Result<Vec<Org>>;

    /// The 24-hour activity overview.
    fn activity_overview(
        &self,
        install_id: Option<&str>,
        clients: &[&str],
    ) -> Result<String, Failure>;

    /// The last overview that landed for this scope.
    fn cached_activity_overview(
        &self,
        install_id: Option<&str>,
        clients: &[&str],
    ) -> Option<String>;

    /// Every held per-tool reading for this installation scope, by slug.
    fn cached_tool_overviews(&self, install_id: Option<&str>) -> BTreeMap<String, String>;

    /// The first page of the named senders' recent requests.
    fn activity_tool_events(
        &self,
        install_id: Option<&str>,
        clients: &[&str],
    ) -> Result<String, Failure>;

    /// The installations this account has sent traffic from.
    fn activity_installations(&self) -> Result<String, Failure>;

    /// Drop every held activity reading.
    fn clear_activity_cache(&self) -> anyhow::Result<()>;

    /// The models this gateway offers.
    fn gate_model_catalogue(&self) -> Result<String, Failure>;

    /// This organization's Gate credit balance and plan.
    fn gate_credits(&self) -> Result<String, Failure>;
}

/// The live security-event feed (AG-578). A handle rather than a set of
/// methods on [`Platform`], because the real one is a process-wide object.
pub(crate) trait SecurityFeed {
    /// `live`, `reconnecting` or `offline`.
    fn state(&self) -> FeedState;

    /// Whether the events from before this connection could be fetched.
    fn history_ok(&self) -> bool;

    /// The buffered events, oldest first.
    fn recent(&self) -> Vec<SecurityEvent>;

    /// Wake the connection loop out of its backoff.
    fn retry_now(&self);

    /// Drop everything buffered for the account or org being left.
    fn reset_for_account_change(&self);
}

impl SecurityFeed for gate_connect_core::security_feed::client::Feed {
    fn state(&self) -> FeedState {
        gate_connect_core::security_feed::client::Feed::state(self)
    }

    fn history_ok(&self) -> bool {
        gate_connect_core::security_feed::client::Feed::history_ok(self)
    }

    fn recent(&self) -> Vec<SecurityEvent> {
        gate_connect_core::security_feed::client::Feed::recent(self)
    }

    fn retry_now(&self) {
        gate_connect_core::security_feed::client::Feed::retry_now(self)
    }

    fn reset_for_account_change(&self) {
        gate_connect_core::security_feed::client::Feed::reset_for_account_change(self)
    }
}

/// The running app: the process-wide handle from `setup`, and the core.
pub(crate) struct TauriPlatform;

impl Platform for TauriPlatform {
    fn os(&self) -> &'static str {
        std::env::consts::OS
    }

    fn os_name(&self) -> String {
        gate_connect_core::diagnostics::os_name()
    }

    fn diagnostics(&self) -> gate_connect_core::diagnostics::Diagnostics {
        gate_connect_core::diagnostics::collect()
    }

    fn install_id(&self) -> anyhow::Result<String> {
        gate_connect_core::primitives::install_id()
    }

    fn log(&self, level: Level, message: &str) {
        gate_connect_core::logging::log(level, message);
    }

    fn emit(&self, event: &str, payload: Value) {
        use tauri::Emitter;
        // Unset before `setup` runs, as at every other emit site here.
        if let Some(handle) = crate::APP_HANDLE.get() {
            let _ = handle.emit(event, payload);
        }
    }

    fn integrations(&self) -> Vec<Box<dyn Integration>> {
        gate_connect_core::registry::registry()
    }

    fn resolve_binary(&self, well_known: &[&str], names: &[&str]) -> Option<PathBuf> {
        gate_connect_core::integrations::binaries::resolve_binary(well_known, names)
    }

    fn version_at(&self, path: &Path) -> Option<String> {
        gate_connect_core::integrations::binaries::version_at(path)
    }

    fn providers(&self) -> Vec<gate_connect_core::provider::ProviderState> {
        gate_connect_core::provider::list()
    }

    fn disconnect_all_managed(&self) -> anyhow::Result<()> {
        gate_connect_core::registry::disconnect_all_managed()
    }

    fn tool_model_states(&self) -> BTreeMap<&'static str, ToolModelView> {
        gate_connect_core::tool_models::states()
    }

    fn choose_tool_model(
        &self,
        tool: ToolId,
        source: ModelSource,
        model_ids: Vec<String>,
        acknowledge_paid_use: bool,
        meta: Vec<(String, GateModelMeta)>,
    ) -> anyhow::Result<bool> {
        gate_connect_core::tool_models::choose(tool, source, model_ids, acknowledge_paid_use, meta)
    }

    fn report_backend_error(&self, context: &'static str, message: String) {
        crate::report_backend_error(context, message);
    }

    fn security_feed(&self) -> &dyn SecurityFeed {
        crate::security_feed().as_ref()
    }

    fn set_updater_relaunching(&self, relaunching: bool) {
        crate::UPDATER_RELAUNCHING.store(relaunching, std::sync::atomic::Ordering::Release);
    }

    fn claim_milestone(&self, name: &str) -> anyhow::Result<bool> {
        gate_connect_core::analytics::claim(name)
    }

    fn cowork_setting_missing(&self) -> Option<&'static str> {
        gate_connect_core::analytics::cowork_setting_missing()
    }
}

impl Prefs for TauriPlatform {
    fn preferences(&self) -> gate_connect_core::preferences::Preferences {
        gate_connect_core::preferences::load()
    }

    fn device_name(&self) -> String {
        gate_connect_core::preferences::device_name()
    }

    fn set_device_name(&self, name: &str) -> anyhow::Result<()> {
        gate_connect_core::preferences::set_device_name(name)
    }

    fn set_notifications(&self, enabled: bool) -> anyhow::Result<()> {
        gate_connect_core::preferences::set_notifications(enabled)
    }

    fn set_security_notification_sound(&self, enabled: bool) -> anyhow::Result<()> {
        gate_connect_core::preferences::set_security_notification_sound(enabled)
    }

    fn set_share_diagnostics(&self, enabled: bool) -> anyhow::Result<()> {
        gate_connect_core::preferences::set_share_diagnostics(enabled)
    }

    fn record_auto_enabled_domains(&self, tool: &str, domains: Vec<String>) -> anyhow::Result<()> {
        gate_connect_core::preferences::record_auto_enabled_domains(tool, domains)
    }

    fn read_auto_enabled_domains(&self, tool: &str) -> Vec<String> {
        gate_connect_core::preferences::read_auto_enabled_domains(tool)
    }

    fn set_signed_out_deliberately(&self, deliberate: bool) -> anyhow::Result<()> {
        gate_connect_core::preferences::set_signed_out_deliberately(deliberate)
    }
}

impl Proxy for TauriPlatform {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn routed_clients_may_be_stale(&self) -> bool {
        crate::ROUTED_CLIENTS_MAY_BE_STALE.load(std::sync::atomic::Ordering::Acquire)
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn startup_enable_pending(&self) -> bool {
        crate::STARTUP_ENABLE_PENDING.load(std::sync::atomic::Ordering::Acquire)
    }

    fn relay_base_url(&self) -> Option<String> {
        gate_connect_core::proxy::relay_base_url()
    }

    fn tool_proxy_url(&self) -> Option<String> {
        gate_connect_core::proxy::tool_proxy_url()
    }

    fn engine_proxy_url(&self) -> Option<String> {
        gate_connect_core::proxy::engine_proxy_url()
    }

    fn stop_forwarder(&self) {
        gate_connect_core::proxy::forwarder::stop();
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn set_routing_intent(&self, enabled: bool) -> anyhow::Result<()> {
        gate_connect_core::proxy::intent::set_intent(enabled)
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn enable_engine(&self) -> anyhow::Result<()> {
        gate_connect_core::proxy::manager().enable().map(drop)
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn shutdown_engine(&self) -> anyhow::Result<()> {
        gate_connect_core::proxy::manager().shutdown_engine()
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn refresh_engine_token(&self, token: &str) {
        gate_connect_core::proxy::manager().refresh_token(token);
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn refresh_engine_api_key(&self, api_key: &str) {
        gate_connect_core::proxy::manager().refresh_api_key(api_key);
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn refresh_engine_mode(&self) {
        gate_connect_core::proxy::manager().refresh_mode();
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn refresh_engine_org(&self, org_id: &str) {
        gate_connect_core::proxy::manager().refresh_org(org_id);
    }
}

impl Windows for TauriPlatform {
    fn set_popover_pinned(&self, pinned: bool) {
        crate::POPOVER_PINNED.store(pinned, std::sync::atomic::Ordering::Release);
    }

    fn drain_backend_errors(&self, label: &str) -> Vec<crate::BackendError> {
        crate::drain_for_label(label)
    }
}

impl Processes for TauriPlatform {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn reopen_pending(&self, slug: &str) -> bool {
        crate::reopen_pending_for(slug)
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn refresh_codex_daemon_when_idle(&self, why: &'static str) {
        crate::refresh_codex_daemon_when_idle(why);
    }
}

impl Account for TauriPlatform {
    fn reconcile_account(&self) -> anyhow::Result<()> {
        gate_connect_core::account::reconcile()
    }

    fn first_reconcile_failure(&self) -> bool {
        !crate::ACCOUNT_RECONCILE_REPORTED.swap(true, std::sync::atomic::Ordering::AcqRel)
    }

    fn account(&self) -> anyhow::Result<Option<gate_connect_core::account::Account>> {
        gate_connect_core::account::load()
    }

    fn account_base_url(&self) -> anyhow::Result<Option<String>> {
        gate_connect_core::account::load_base_url()
    }

    fn has_api_key(&self) -> anyhow::Result<bool> {
        gate_connect_core::account::has_api_key()
    }

    fn auth_mode(&self) -> anyhow::Result<AuthMode> {
        gate_connect_core::account::auth_mode()
    }

    fn billing_mode(&self) -> anyhow::Result<BillingMode> {
        gate_connect_core::account::billing_mode()
    }

    fn selected_org(&self) -> anyhow::Result<Option<(String, String)>> {
        gate_connect_core::account::selected_org()
    }

    fn api_key_prefix(&self) -> anyhow::Result<Option<String>> {
        gate_connect_core::account::api_key_prefix()
    }

    fn save_account(&self, base_url: &str, api_key: Option<&str>) -> anyhow::Result<()> {
        gate_connect_core::account::save(base_url, api_key)
    }

    fn set_auth_mode(&self, mode: AuthMode) -> anyhow::Result<()> {
        gate_connect_core::account::set_auth_mode(mode)
    }

    fn set_billing_mode(&self, mode: BillingMode) -> anyhow::Result<()> {
        gate_connect_core::account::set_billing_mode(mode)
    }

    fn set_org(&self, org_id: &str, org_name: &str) -> anyhow::Result<()> {
        gate_connect_core::account::set_org(org_id, org_name)
    }

    fn clear_account(&self) -> anyhow::Result<()> {
        gate_connect_core::account::clear()
    }

    fn switch_gateway(&self, base_url: &str) -> anyhow::Result<()> {
        gate_connect_core::account::switch_gateway(base_url)
    }
}

impl Session for TauriPlatform {
    fn session_reading(&self) -> SessionReading {
        gate_connect_core::oauth::session_reading()
    }

    fn cancel_login(&self) {
        gate_connect_core::oauth::cancel_login();
    }

    fn clear_oauth_session(&self) -> anyhow::Result<()> {
        gate_connect_core::oauth::clear()
    }

    fn session_rejected(&self) -> bool {
        gate_connect_core::oauth::session_rejected()
    }

    fn signal_session_dead(&self) {
        // Unset before `setup` runs, as at every other use of the handle.
        if let Some(app) = crate::APP_HANDLE.get() {
            crate::signal_session_dead(app);
        }
    }

    fn forget_identity_if_signed_out(&self) -> anyhow::Result<bool> {
        gate_connect_core::analytics::forget_identity_if_signed_out()
    }

    fn analytics_identity(&self) -> Identity {
        gate_connect_core::analytics::load_identity()
    }

    fn save_identity(
        &self,
        identity: Identity,
        on_stored: &mut dyn FnMut(&Identity),
    ) -> anyhow::Result<SaveOutcome> {
        gate_connect_core::analytics::save_identity(identity, on_stored)
    }
}

impl Gateway for TauriPlatform {
    fn list_orgs(&self) -> anyhow::Result<Vec<Org>> {
        gate_connect_core::org::list_current()
    }

    fn activity_overview(
        &self,
        install_id: Option<&str>,
        clients: &[&str],
    ) -> Result<String, Failure> {
        gate_connect_core::activity::overview_json(install_id, clients)
    }

    fn cached_activity_overview(
        &self,
        install_id: Option<&str>,
        clients: &[&str],
    ) -> Option<String> {
        gate_connect_core::activity::cached_overview_json(install_id, clients)
    }

    fn cached_tool_overviews(&self, install_id: Option<&str>) -> BTreeMap<String, String> {
        gate_connect_core::activity::cached_tool_overviews_json(install_id)
    }

    fn activity_tool_events(
        &self,
        install_id: Option<&str>,
        clients: &[&str],
    ) -> Result<String, Failure> {
        gate_connect_core::activity::tool_events_json(install_id, clients)
    }

    fn activity_installations(&self) -> Result<String, Failure> {
        gate_connect_core::activity::installations_json()
    }

    fn clear_activity_cache(&self) -> anyhow::Result<()> {
        gate_connect_core::activity_cache::clear()
    }

    fn gate_model_catalogue(&self) -> Result<String, Failure> {
        gate_connect_core::gate_models::catalogue_json()
    }

    fn gate_credits(&self) -> Result<String, Failure> {
        gate_connect_core::gate_models::credits_json()
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::{
        Account, Gateway, Platform, Prefs, Processes, Proxy, SecurityFeed, Session, Windows,
    };
    use crate::BackendError;
    use gate_connect_core::account::{AuthMode, BillingMode};
    use gate_connect_core::activity::Failure;
    use gate_connect_core::analytics::{Identity, SaveOutcome};
    use gate_connect_core::logging::Level;
    use gate_connect_core::oauth::{OAuthTokens, SessionReading};
    use gate_connect_core::org::Org;
    use gate_connect_core::preferences::{GateModelMeta, ModelSource, Preferences};
    use gate_connect_core::registry::{Mechanism, Status, ToolId};
    use gate_connect_core::security_feed::{FeedState, SecurityEvent};
    use gate_connect_core::taxonomy::Client;
    use gate_connect_core::tool_models::ToolModelView;
    use gate_connect_core::{ConnectInput, Integration};
    use serde_json::Value;
    use std::collections::{BTreeMap, HashSet};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    /// One ordered record of every side effect, shared with the fake tools so
    /// a test can see a tool's disconnect land before the engine stops.
    pub(crate) type Journal = Arc<Mutex<Vec<String>>>;

    /// Records what a command did and answers what the test set up.
    ///
    /// A preference write either lands in its slot or fails with
    /// `fail_writes`, so a test can see both what was stored and what the
    /// caller was told.
    #[derive(Default)]
    pub(crate) struct FakePlatform {
        pub os: &'static str,
        pub os_name: String,
        pub install_id: Option<String>,
        pub fail_writes: Option<String>,
        pub tools: Vec<FakeTool>,
        /// Binary name to the version it reports (`None`: found, would not say).
        /// A name not listed is not installed.
        pub binaries: BTreeMap<&'static str, Option<String>>,
        pub cowork_missing: Option<&'static str>,
        pub feed: FakeFeed,
        pub routed_clients_stale: bool,
        pub startup_pending: bool,
        /// Slugs whose running process predates their config.
        pub reopen_pending: HashSet<&'static str>,
        /// Buffered backend failures, per window label.
        pub backend_errors: Mutex<BTreeMap<String, Vec<BackendError>>>,
        pub emitted: Mutex<Vec<(String, Value)>>,
        pub logged: Mutex<Vec<(Level, String)>>,
        pub notifications: Mutex<Option<bool>>,
        pub share_diagnostics: Mutex<Option<bool>>,
        pub security_sound: Mutex<Option<bool>>,
        pub device_name: Mutex<String>,
        pub auto_enabled: Mutex<BTreeMap<String, Vec<String>>>,
        pub milestones: Mutex<HashSet<String>>,
        pub updater_relaunching: Mutex<Option<bool>>,
        pub popover_pinned: Mutex<Option<bool>>,
        /// What `preferences()` answers.
        pub prefs: Preferences,
        pub signed_out_deliberately: Mutex<Option<bool>>,

        /// Every account, engine, session and tool side effect, and every
        /// emit, in the order they happened.
        pub calls: Journal,
        /// A call (by its name in `calls`) that fails, with the reason.
        pub fail: BTreeMap<&'static str, String>,

        // ---- Account ----
        /// The stored gateway. No account while `None`.
        pub account_url: Mutex<Option<String>>,
        pub api_key: Mutex<Option<String>>,
        pub auth_mode: Mutex<AuthMode>,
        pub billing_mode: Mutex<BillingMode>,
        pub org: Mutex<Option<(String, String)>>,
        pub key_prefix: Option<String>,
        /// Whether a reconcile failure was already reported this run.
        pub reconcile_reported: Mutex<bool>,

        // ---- Session ----
        /// The live session. Signed out while `None`.
        pub session: Option<OAuthTokens>,
        /// The session could not be read (offline, secret store silent).
        pub session_unavailable: bool,
        /// The gateway refused the session.
        pub session_rejected: bool,
        pub identity: Mutex<Identity>,
        /// A sign-out left an identity to forget.
        pub forget_pending: bool,
        /// What a save does. `None`: it is stored.
        pub save_outcome: Option<SaveOutcome>,

        // ---- Gateway ----
        pub orgs: Vec<Org>,
        /// Every gateway read fails with this.
        pub gateway_failure: Option<Failure>,
        pub relay_url: Option<String>,
        pub tool_proxy: Option<String>,
        pub engine_proxy: Option<String>,

        // ---- Tool models ----
        pub model_states: BTreeMap<&'static str, ToolModelView>,
        /// What a model choice answers: whether the tool's config changed.
        pub model_applied: bool,
    }

    impl FakePlatform {
        fn check_write(&self) -> anyhow::Result<()> {
            match &self.fail_writes {
                Some(err) => Err(anyhow::anyhow!("{err}").context("writing preferences")),
                None => Ok(()),
            }
        }

        fn write<T>(&self, slot: &Mutex<T>, value: T) -> anyhow::Result<()> {
            self.check_write()?;
            *slot.lock().unwrap() = value;
            Ok(())
        }

        pub fn emitted(&self) -> Vec<(String, Value)> {
            self.emitted.lock().unwrap().clone()
        }

        pub fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn record(&self, entry: impl Into<String>) {
            self.calls.lock().unwrap().push(entry.into());
        }

        /// Record `entry`, then fail if the test said `name` fails.
        fn attempt(&self, name: &'static str, entry: impl Into<String>) -> anyhow::Result<()> {
            self.record(entry);
            self.check(name)
        }

        fn check(&self, name: &'static str) -> anyhow::Result<()> {
            match self.fail.get(name) {
                Some(err) => Err(anyhow::anyhow!("{err}").context(name)),
                None => Ok(()),
            }
        }

        fn gateway<T>(&self, answer: T) -> Result<T, Failure> {
            match &self.gateway_failure {
                Some(f) => Err(f.clone()),
                None => Ok(answer),
            }
        }
    }

    impl Platform for FakePlatform {
        fn os(&self) -> &'static str {
            self.os
        }

        fn os_name(&self) -> String {
            self.os_name.clone()
        }

        fn diagnostics(&self) -> gate_connect_core::diagnostics::Diagnostics {
            unimplemented!("no test reads the diagnostics report through the fake")
        }

        fn install_id(&self) -> anyhow::Result<String> {
            self.install_id
                .clone()
                .ok_or_else(|| anyhow::anyhow!("no install id").context("reading install id"))
        }

        fn log(&self, level: Level, message: &str) {
            self.logged
                .lock()
                .unwrap()
                .push((level, message.to_string()));
        }

        fn emit(&self, event: &str, payload: Value) {
            self.record(format!("emit {event}"));
            self.emitted
                .lock()
                .unwrap()
                .push((event.to_string(), payload));
        }

        fn integrations(&self) -> Vec<Box<dyn Integration>> {
            self.tools
                .iter()
                .map(|t| {
                    let mut t = t.clone();
                    t.journal = self.calls.clone();
                    Box::new(t) as Box<dyn Integration>
                })
                .collect()
        }

        fn resolve_binary(&self, _well_known: &[&str], names: &[&str]) -> Option<PathBuf> {
            names
                .iter()
                .find(|n| self.binaries.contains_key(*n))
                .map(PathBuf::from)
        }

        fn version_at(&self, path: &Path) -> Option<String> {
            self.binaries.get(path.to_str()?).cloned().flatten()
        }

        fn providers(&self) -> Vec<gate_connect_core::provider::ProviderState> {
            Vec::new()
        }

        fn disconnect_all_managed(&self) -> anyhow::Result<()> {
            self.attempt("disconnect_all_managed", "disconnect_all_managed")
        }

        fn tool_model_states(&self) -> BTreeMap<&'static str, ToolModelView> {
            self.record("tool_model_states");
            self.model_states.clone()
        }

        fn choose_tool_model(
            &self,
            tool: ToolId,
            source: ModelSource,
            model_ids: Vec<String>,
            acknowledge_paid_use: bool,
            meta: Vec<(String, GateModelMeta)>,
        ) -> anyhow::Result<bool> {
            let meta: Vec<_> = meta.into_iter().map(|(id, _)| id).collect();
            self.attempt(
                "choose_tool_model",
                format!(
                    "choose_tool_model {tool} {source:?} {model_ids:?} ack={acknowledge_paid_use} meta={meta:?}"
                ),
            )?;
            Ok(self.model_applied)
        }

        fn report_backend_error(&self, context: &'static str, message: String) {
            self.record(format!("report_backend_error {context}: {message}"));
        }

        fn security_feed(&self) -> &dyn SecurityFeed {
            &self.feed
        }

        fn set_updater_relaunching(&self, relaunching: bool) {
            *self.updater_relaunching.lock().unwrap() = Some(relaunching);
        }

        fn claim_milestone(&self, name: &str) -> anyhow::Result<bool> {
            self.check_write()?;
            Ok(self.milestones.lock().unwrap().insert(name.to_string()))
        }

        fn cowork_setting_missing(&self) -> Option<&'static str> {
            self.cowork_missing
        }
    }

    impl Prefs for FakePlatform {
        fn preferences(&self) -> Preferences {
            self.prefs.clone()
        }

        fn device_name(&self) -> String {
            self.device_name.lock().unwrap().clone()
        }

        fn set_device_name(&self, name: &str) -> anyhow::Result<()> {
            self.write(&self.device_name, name.to_string())
        }

        fn set_notifications(&self, enabled: bool) -> anyhow::Result<()> {
            self.write(&self.notifications, Some(enabled))
        }

        fn set_security_notification_sound(&self, enabled: bool) -> anyhow::Result<()> {
            self.write(&self.security_sound, Some(enabled))
        }

        fn set_share_diagnostics(&self, enabled: bool) -> anyhow::Result<()> {
            self.write(&self.share_diagnostics, Some(enabled))
        }

        fn record_auto_enabled_domains(
            &self,
            tool: &str,
            domains: Vec<String>,
        ) -> anyhow::Result<()> {
            self.check_write()?;
            self.auto_enabled
                .lock()
                .unwrap()
                .insert(tool.to_string(), domains);
            Ok(())
        }

        fn read_auto_enabled_domains(&self, tool: &str) -> Vec<String> {
            self.auto_enabled
                .lock()
                .unwrap()
                .get(tool)
                .cloned()
                .unwrap_or_default()
        }

        fn set_signed_out_deliberately(&self, deliberate: bool) -> anyhow::Result<()> {
            self.record(format!("set_signed_out_deliberately {deliberate}"));
            self.write(&self.signed_out_deliberately, Some(deliberate))
        }
    }

    impl Proxy for FakePlatform {
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        fn routed_clients_may_be_stale(&self) -> bool {
            self.routed_clients_stale
        }

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        fn startup_enable_pending(&self) -> bool {
            self.startup_pending
        }

        fn relay_base_url(&self) -> Option<String> {
            self.relay_url.clone()
        }

        fn tool_proxy_url(&self) -> Option<String> {
            self.tool_proxy.clone()
        }

        fn engine_proxy_url(&self) -> Option<String> {
            self.engine_proxy.clone()
        }

        fn stop_forwarder(&self) {
            self.record("stop_forwarder");
        }

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        fn set_routing_intent(&self, enabled: bool) -> anyhow::Result<()> {
            self.attempt(
                "set_routing_intent",
                format!("set_routing_intent {enabled}"),
            )
        }

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        fn enable_engine(&self) -> anyhow::Result<()> {
            self.attempt("enable_engine", "enable_engine")
        }

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        fn shutdown_engine(&self) -> anyhow::Result<()> {
            self.attempt("shutdown_engine", "shutdown_engine")
        }

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        fn refresh_engine_token(&self, token: &str) {
            self.record(format!("refresh_engine_token {token:?}"));
        }

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        fn refresh_engine_api_key(&self, api_key: &str) {
            self.record(format!("refresh_engine_api_key {api_key:?}"));
        }

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        fn refresh_engine_mode(&self) {
            self.record("refresh_engine_mode");
        }

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        fn refresh_engine_org(&self, org_id: &str) {
            self.record(format!("refresh_engine_org {org_id}"));
        }
    }

    impl Windows for FakePlatform {
        fn set_popover_pinned(&self, pinned: bool) {
            *self.popover_pinned.lock().unwrap() = Some(pinned);
        }

        fn drain_backend_errors(&self, label: &str) -> Vec<BackendError> {
            self.backend_errors
                .lock()
                .unwrap()
                .get_mut(label)
                .map(std::mem::take)
                .unwrap_or_default()
        }
    }

    impl Processes for FakePlatform {
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        fn reopen_pending(&self, slug: &str) -> bool {
            self.reopen_pending.contains(slug)
        }

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        fn refresh_codex_daemon_when_idle(&self, why: &'static str) {
            self.record(format!("refresh_codex_daemon_when_idle {why}"));
        }
    }

    impl Account for FakePlatform {
        fn reconcile_account(&self) -> anyhow::Result<()> {
            self.attempt("reconcile_account", "reconcile_account")
        }

        fn first_reconcile_failure(&self) -> bool {
            !std::mem::replace(&mut *self.reconcile_reported.lock().unwrap(), true)
        }

        fn account(&self) -> anyhow::Result<Option<gate_connect_core::account::Account>> {
            self.check("read_account")?;
            Ok(self.account_url.lock().unwrap().clone().map(|url| {
                gate_connect_core::account::Account {
                    gateway_base_url: url,
                    api_key: self.api_key.lock().unwrap().clone().unwrap_or_default(),
                    auth_mode: *self.auth_mode.lock().unwrap(),
                    billing_mode: *self.billing_mode.lock().unwrap(),
                }
            }))
        }

        fn account_base_url(&self) -> anyhow::Result<Option<String>> {
            self.check("read_account")?;
            Ok(self.account_url.lock().unwrap().clone())
        }

        fn has_api_key(&self) -> anyhow::Result<bool> {
            self.check("read_account")?;
            Ok(self.api_key.lock().unwrap().is_some())
        }

        fn auth_mode(&self) -> anyhow::Result<AuthMode> {
            self.check("read_account")?;
            Ok(*self.auth_mode.lock().unwrap())
        }

        fn billing_mode(&self) -> anyhow::Result<BillingMode> {
            self.check("read_account")?;
            Ok(*self.billing_mode.lock().unwrap())
        }

        fn selected_org(&self) -> anyhow::Result<Option<(String, String)>> {
            self.check("read_account")?;
            Ok(self.org.lock().unwrap().clone())
        }

        fn api_key_prefix(&self) -> anyhow::Result<Option<String>> {
            self.check("read_account")?;
            Ok(self.key_prefix.clone())
        }

        fn save_account(&self, base_url: &str, api_key: Option<&str>) -> anyhow::Result<()> {
            self.attempt(
                "save_account",
                format!("save_account {base_url} {api_key:?}"),
            )?;
            *self.account_url.lock().unwrap() = Some(base_url.to_string());
            if let Some(k) = api_key {
                *self.api_key.lock().unwrap() = Some(k.to_string());
            }
            Ok(())
        }

        fn set_auth_mode(&self, mode: AuthMode) -> anyhow::Result<()> {
            self.attempt("set_auth_mode", format!("set_auth_mode {mode:?}"))?;
            *self.auth_mode.lock().unwrap() = mode;
            Ok(())
        }

        fn set_billing_mode(&self, mode: BillingMode) -> anyhow::Result<()> {
            self.attempt("set_billing_mode", format!("set_billing_mode {mode:?}"))?;
            *self.billing_mode.lock().unwrap() = mode;
            Ok(())
        }

        fn set_org(&self, org_id: &str, org_name: &str) -> anyhow::Result<()> {
            self.attempt("set_org", format!("set_org {org_id} {org_name}"))?;
            *self.org.lock().unwrap() = Some((org_id.to_string(), org_name.to_string()));
            Ok(())
        }

        fn clear_account(&self) -> anyhow::Result<()> {
            self.attempt("clear_account", "clear_account")?;
            *self.account_url.lock().unwrap() = None;
            *self.api_key.lock().unwrap() = None;
            *self.org.lock().unwrap() = None;
            Ok(())
        }

        fn switch_gateway(&self, base_url: &str) -> anyhow::Result<()> {
            self.attempt("switch_gateway", format!("switch_gateway {base_url}"))?;
            *self.account_url.lock().unwrap() = Some(base_url.to_string());
            *self.api_key.lock().unwrap() = None;
            Ok(())
        }
    }

    impl Session for FakePlatform {
        fn session_reading(&self) -> SessionReading {
            match (&self.session, self.session_unavailable) {
                (_, true) => SessionReading::Unavailable,
                (Some(t), false) => SessionReading::Live(t.clone()),
                (None, false) => SessionReading::SignedOut,
            }
        }

        fn cancel_login(&self) {
            self.record("cancel_login");
        }

        fn clear_oauth_session(&self) -> anyhow::Result<()> {
            self.attempt("clear_oauth_session", "clear_oauth_session")?;
            *self.identity.lock().unwrap() = Identity::default();
            Ok(())
        }

        fn session_rejected(&self) -> bool {
            self.session_rejected
        }

        fn signal_session_dead(&self) {
            self.record("signal_session_dead");
        }

        fn forget_identity_if_signed_out(&self) -> anyhow::Result<bool> {
            self.check("forget_identity_if_signed_out")?;
            if self.forget_pending {
                *self.identity.lock().unwrap() = Identity::default();
            }
            Ok(self.forget_pending)
        }

        fn analytics_identity(&self) -> Identity {
            self.identity.lock().unwrap().clone()
        }

        fn save_identity(
            &self,
            identity: Identity,
            on_stored: &mut dyn FnMut(&Identity),
        ) -> anyhow::Result<SaveOutcome> {
            self.check("save_identity")?;
            let outcome = self.save_outcome.clone().unwrap_or(SaveOutcome::Saved);
            match outcome {
                SaveOutcome::Saved => {
                    *self.identity.lock().unwrap() = identity;
                    on_stored(&self.identity.lock().unwrap().clone());
                }
                SaveOutcome::NotLive => on_stored(&self.identity.lock().unwrap().clone()),
                _ => {}
            }
            Ok(outcome)
        }
    }

    impl Gateway for FakePlatform {
        fn list_orgs(&self) -> anyhow::Result<Vec<Org>> {
            self.check("list_orgs")?;
            Ok(self.orgs.clone())
        }

        fn activity_overview(
            &self,
            install_id: Option<&str>,
            clients: &[&str],
        ) -> Result<String, Failure> {
            self.gateway(format!("overview {install_id:?} {clients:?}"))
        }

        fn cached_activity_overview(
            &self,
            install_id: Option<&str>,
            clients: &[&str],
        ) -> Option<String> {
            Some(format!("cached overview {install_id:?} {clients:?}"))
        }

        fn cached_tool_overviews(&self, install_id: Option<&str>) -> BTreeMap<String, String> {
            BTreeMap::from([("codex".to_string(), format!("cached {install_id:?}"))])
        }

        fn activity_tool_events(
            &self,
            install_id: Option<&str>,
            clients: &[&str],
        ) -> Result<String, Failure> {
            self.gateway(format!("events {install_id:?} {clients:?}"))
        }

        fn activity_installations(&self) -> Result<String, Failure> {
            self.gateway("installations".to_string())
        }

        fn clear_activity_cache(&self) -> anyhow::Result<()> {
            self.attempt("clear_activity_cache", "clear_activity_cache")
        }

        fn gate_model_catalogue(&self) -> Result<String, Failure> {
            self.record("gate_model_catalogue");
            self.gateway(
                r#"{"data":[{"id":"gate/one","name":"One","context_length":1000}]}"#.to_string(),
            )
        }

        fn gate_credits(&self) -> Result<String, Failure> {
            self.gateway("credits".to_string())
        }
    }

    /// A security feed that answers what the test set and counts retries.
    pub(crate) struct FakeFeed {
        pub state: FeedState,
        pub history_ok: bool,
        pub recent: Vec<SecurityEvent>,
        pub retries: Mutex<u32>,
        pub resets: Mutex<u32>,
    }

    impl Default for FakeFeed {
        /// The real feed's resting state: offline, nothing failed, nothing
        /// buffered.
        fn default() -> Self {
            Self {
                state: FeedState::Offline,
                history_ok: true,
                recent: Vec::new(),
                retries: Mutex::new(0),
                resets: Mutex::new(0),
            }
        }
    }

    impl SecurityFeed for FakeFeed {
        fn state(&self) -> FeedState {
            self.state
        }

        fn history_ok(&self) -> bool {
            self.history_ok
        }

        fn recent(&self) -> Vec<SecurityEvent> {
            self.recent.clone()
        }

        fn retry_now(&self) {
            *self.retries.lock().unwrap() += 1;
        }

        fn reset_for_account_change(&self) {
            *self.resets.lock().unwrap() += 1;
        }
    }

    /// A tool the registry would list. Only what the moved commands read is
    /// configurable; the rest answers fixed values or refuses.
    #[derive(Clone)]
    pub(crate) struct FakeTool {
        pub id: ToolId,
        pub hidden: bool,
        /// Binary names to look for. Empty: the tool has no binary.
        pub binaries: &'static [&'static str],
        /// `Err` carries the message `status()` fails with.
        pub status: Result<Status, String>,
        /// The message `connect` and `disconnect` fail with.
        pub write_fails: Option<String>,
        /// Where `connect` and `disconnect` are recorded. The fake platform
        /// hands each tool its own journal.
        pub journal: Journal,
    }

    impl FakeTool {
        pub fn new(id: ToolId) -> Self {
            Self {
                id,
                hidden: false,
                binaries: &[],
                status: Ok(Status::Detected),
                write_fails: None,
                journal: Journal::default(),
            }
        }
    }

    impl FakeTool {
        fn write_result(&self, what: &str) -> anyhow::Result<()> {
            match &self.write_fails {
                Some(err) => Err(anyhow::anyhow!("{err}").context(format!("{what} {}", self.id))),
                None => Ok(()),
            }
        }
    }

    impl Integration for FakeTool {
        fn id(&self) -> ToolId {
            self.id
        }
        fn display_name(&self) -> &'static str {
            "Fake Tool"
        }
        fn row_label(&self) -> &'static str {
            "Fake"
        }
        fn client(&self) -> Client {
            Client::ClaudeCode
        }
        fn binary(&self) -> (&'static [&'static str], &'static [&'static str]) {
            (&[], self.binaries)
        }
        fn upstream_provider_name(&self) -> &'static str {
            "Fake Provider"
        }
        fn default_upstream_url(&self) -> &'static str {
            "https://fake.example.com"
        }
        fn mechanism(&self) -> Mechanism {
            Mechanism::Relay
        }
        fn configured_addresses(&self) -> anyhow::Result<Vec<String>> {
            Ok(Vec::new())
        }
        fn watch_paths(&self) -> Vec<PathBuf> {
            Vec::new()
        }
        fn detect(&self) -> anyhow::Result<bool> {
            Ok(true)
        }
        fn status(&self) -> anyhow::Result<Status> {
            self.status
                .clone()
                .map_err(|e| anyhow::anyhow!("{e}").context("reading the config"))
        }
        fn connect(&self, input: &ConnectInput) -> anyhow::Result<()> {
            self.journal.lock().unwrap().push(format!(
                "connect {} {} {:?} relay={:?} proxy={:?}",
                self.id,
                input.gateway_base_url,
                input.billing_mode,
                input.relay_base_url,
                input.engine_proxy_url
            ));
            self.write_result("connect")
        }
        fn disconnect(&self) -> anyhow::Result<()> {
            self.journal
                .lock()
                .unwrap()
                .push(format!("disconnect {}", self.id));
            self.write_result("disconnect")
        }
        fn hidden_in_ui(&self) -> bool {
            self.hidden
        }
    }
}
