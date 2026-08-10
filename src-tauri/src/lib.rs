//! Tauri shell. The Rust surface here is small on purpose: every command
//! delegates to `gate-connect-core` so the CLI and the GUI exercise the
//! same code path. Beyond commands, this file sets up the menu-bar / tray
//! presentation: no dock icon, hidden window on launch, click the tray
//! to toggle a popover-style window anchored under the icon.

use gate_connect_core::{account, registry, ConnectInput, Status, ToolId};

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, PhysicalPosition, WindowEvent,
};
#[cfg(not(target_os = "linux"))]
use tauri::{Position, Size};
// Used by the startup auto-enable to nudge the popover to re-read state, and
// by `report_backend_error` to nudge a drain.
use tauri::Emitter;

/// Shape-check a user-supplied key coming over the JS-to-Rust boundary.
/// Refuses empty input, control chars, lengths > 512 bytes, and a missing
/// prefix when one is required. The keychain layer treats keys as opaque
/// bytes - this check exists to fail fast before we persist nonsense.
fn validate_api_key(key: &str, required_prefix: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("API key is empty".into());
    }
    if key.len() > 512 {
        return Err("API key is unexpectedly long (>512 bytes)".into());
    }
    if key.chars().any(|c| c.is_control()) {
        return Err("API key contains control characters".into());
    }
    if !required_prefix.is_empty() && !key.starts_with(required_prefix) {
        return Err(format!("API key must start with {required_prefix:?}"));
    }
    Ok(())
}

#[derive(Serialize)]
struct ToolDto {
    slug: String,
    name: String,
    upstream_provider_name: String,
    default_upstream_url: String,
    requires_upstream_credential: bool,
    status: StatusDto,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StatusDto {
    NotInstalled,
    Detected,
    Connected,
    Drifted { reason: String },
    Error { message: String },
}

impl From<Status> for StatusDto {
    fn from(s: Status) -> Self {
        match s {
            Status::NotInstalled => StatusDto::NotInstalled,
            Status::Detected => StatusDto::Detected,
            Status::Connected => StatusDto::Connected,
            Status::Drifted(reason) => StatusDto::Drifted { reason },
        }
    }
}

fn status_for(integ: &dyn gate_connect_core::Integration) -> StatusDto {
    match integ.status() {
        Ok(s) => s.into(),
        Err(e) => StatusDto::Error {
            // `{e:#}` prints the whole anyhow context chain; bare Display
            // would stop at the outermost context and drop the root cause.
            message: format!("{e:#}"),
        },
    }
}

#[tauri::command]
fn list_tools() -> Vec<ToolDto> {
    // The UI boundary is where hiding happens. The registry itself keeps every
    // integration so the sweep, restore and sign-out paths still clean up a
    // tool someone connected with an earlier build.
    registry::registry()
        .iter()
        .filter(|integ| !integ.hidden_in_ui())
        .map(|integ| ToolDto {
            slug: integ.id().to_string(),
            name: integ.display_name().to_string(),
            upstream_provider_name: integ.upstream_provider_name().to_string(),
            default_upstream_url: integ.default_upstream_url().to_string(),
            requires_upstream_credential: integ.requires_upstream_credential(),
            status: status_for(integ.as_ref()),
        })
        .collect()
}

#[tauri::command]
fn tool_status(slug: String) -> Result<StatusDto, String> {
    let id = ToolId::from_slug(&slug).ok_or_else(|| format!("unknown tool {slug:?}"))?;
    let integ =
        registry::find(id).ok_or_else(|| "integration missing from registry".to_string())?;
    Ok(status_for(integ.as_ref()))
}

#[tauri::command]
async fn connect_tool(slug: String, upstream_url: String) -> Result<StatusDto, String> {
    // Off the main thread: connect does config-file I/O that shouldn't
    // block the UI thread.
    tauri::async_runtime::spawn_blocking(move || {
        let integ = resolve_integration(&slug)?;
        let account = account::load()
            .map_err(|e| format!("{e:#}"))?
            .ok_or_else(|| "Sign in to Gate AI first".to_string())?;
        if integ.requires_upstream_credential()
            && !integ
                .has_upstream_credential()
                .map_err(|e| format!("{e:#}"))?
        {
            return Err(
                "No upstream Anthropic credential saved. Add one before connecting.".into(),
            );
        }
        // Auto-enable the proxy so the reverse-proxy relay is live: relay-routed
        // tool configs point at the loopback relay, which only exists while the
        // engine runs. Idempotent if the proxy is already on.
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            gate_connect_core::proxy::manager()
                .enable()
                .map_err(|e| format!("{e:#}"))?;
            mark_routing_enabled();
        }
        let input = ConnectInput {
            gateway_base_url: account.gateway_base_url,
            upstream_url,
            relay_base_url: gate_connect_core::proxy::relay_base_url(),
            engine_proxy_url: gate_connect_core::proxy::engine_proxy_url(),
        };
        integ.connect(&input).map_err(|e| format!("{e:#}"))?;
        Ok(status_for(integ.as_ref()))
    })
    .await
    .map_err(|e| format!("connect join error: {e}"))?
}

#[tauri::command]
async fn disconnect_tool(slug: String) -> Result<StatusDto, String> {
    // Off the main thread: disconnect does config-file I/O that shouldn't
    // block the UI thread.
    tauri::async_runtime::spawn_blocking(move || {
        let integ = resolve_integration(&slug)?;
        integ.disconnect().map_err(|e| format!("{e:#}"))?;
        Ok(status_for(integ.as_ref()))
    })
    .await
    .map_err(|e| format!("disconnect join error: {e}"))?
}

#[tauri::command]
fn has_upstream_credential(slug: String) -> Result<bool, String> {
    let integ = resolve_integration(&slug)?;
    integ
        .has_upstream_credential()
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn save_upstream_api_key(slug: String, api_key: String) -> Result<(), String> {
    let integ = resolve_integration(&slug)?;
    // Enforce the integration's expected credential prefix in addition to
    // the empty/control-char/oversize checks, so a compromised renderer
    // can't write arbitrary bytes to a tool's keychain entry under a
    // mismatched slug.
    validate_api_key(api_key.trim(), integ.upstream_credential_prefix())?;
    integ
        .save_upstream_credential(&api_key)
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn clear_upstream_credential(slug: String) -> Result<(), String> {
    let integ = resolve_integration(&slug)?;
    integ
        .clear_upstream_credential()
        .map_err(|e| format!("{e:#}"))
}

fn resolve_integration(slug: &str) -> Result<Box<dyn gate_connect_core::Integration>, String> {
    let id = ToolId::from_slug(slug).ok_or_else(|| format!("unknown tool {slug:?}"))?;
    registry::find(id).ok_or_else(|| "integration missing from registry".to_string())
}

#[derive(Serialize)]
struct AccountDto {
    gateway_base_url: String,
    has_api_key: bool,
    /// Which credential the account authenticates with, so the UI can route
    /// an OAuth account that isn't signed in to the sign-in screen and show
    /// the legacy key controls only in API-key mode. Serialized snake_case
    /// (`"api_key"` / `"oauth"`).
    auth_mode: gate_connect_core::account::AuthMode,
    /// Selected org (OAuth mode), so the UI can show it and route to the picker
    /// when an OAuth session has no org yet. Both `None` until the user picks.
    org_id: Option<String>,
    org_name: Option<String>,
}

/// Whether an `account::reconcile` failure was already forwarded to the
/// analytics seam this run. The reconcile runs on every popover interaction
/// (each `get_account`), so a persistently failing one would otherwise emit
/// an `error_shown` per open; one event per run carries the same signal.
static ACCOUNT_RECONCILE_REPORTED: AtomicBool = AtomicBool::new(false);

#[tauri::command]
fn get_account() -> Result<Option<AccountDto>, String> {
    // Reconcile the stored account against its on-disk anchor before reading it,
    // so the first-run-vs-home decision this call drives always sees a
    // consistent view. An uninstall that removed Gate Connect's files but left
    // its OS keychain entry behind (macOS drag-to-trash, or a deep uninstaller
    // that purges Application Support but can't touch the keychain). Dropping
    // that orphaned key here, rather than on a startup thread that races this
    // read, means it can't briefly route the user to a half-signed-in home. A
    // key-less account.json (URL but no key) is left intact - it's a pending-key
    // state and the read below reports has_api_key=false, so the UI routes to
    // key entry. Best-effort: a reconcile hiccup must not flip a signed-in user
    // to first-run, so we log and fall through to the read below.
    if let Err(e) = account::reconcile() {
        eprintln!("account reconcile failed: {e}");
        if !ACCOUNT_RECONCILE_REPORTED.swap(true, Ordering::AcqRel) {
            report_backend_error("account_reconcile", format!("{e:#}"));
        }
    }
    let Some(gateway_base_url) = account::load_base_url().map_err(|e| format!("{e:#}"))? else {
        return Ok(None);
    };
    let has_api_key = account::has_api_key().map_err(|e| format!("{e:#}"))?;
    let auth_mode = account::auth_mode().map_err(|e| format!("{e:#}"))?;
    let (org_id, org_name) = match account::selected_org().map_err(|e| format!("{e:#}"))? {
        Some((id, name)) => (Some(id), Some(name)),
        None => (None, None),
    };
    Ok(Some(AccountDto {
        gateway_base_url,
        has_api_key,
        auth_mode,
        org_id,
        org_name,
    }))
}

/// Leading characters of the stored Gate key, for the "show which key" reveal
/// in Settings. Reads the prefix recorded in `account.json`, so it never
/// touches the keychain; the UI still calls it only when the user taps to
/// reveal, to keep even the prefix out of view until asked.
#[tauri::command]
fn get_account_key_prefix() -> Result<Option<String>, String> {
    account::api_key_prefix().map_err(|e| format!("{e:#}"))
}

/// Fallback for accounts saved before the prefix was recorded on disk: read the
/// key from the keychain (may prompt), backfill the prefix into `account.json`,
/// and return it. The UI calls this only after the user confirms the reveal,
/// since it touches the keychain.
#[tauri::command]
fn backfill_account_key_prefix() -> Result<Option<String>, String> {
    account::backfill_api_key_prefix().map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn save_account(base_url: String, api_key: Option<String>) -> Result<(), String> {
    if base_url.len() > 2048 {
        return Err("base url is unexpectedly long (>2048 bytes)".into());
    }
    let parsed = url::Url::parse(&base_url).map_err(|e| format!("invalid base url: {e}"))?;
    if parsed.scheme() != "https" {
        return Err("base url must use https".into());
    }
    if parsed.host_str().is_none() {
        return Err("base url is missing a host".into());
    }
    let key = api_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if let Some(k) = key.as_deref() {
        validate_api_key(k, "sk-")?;
    }
    // Off the main thread: keychain write plus up to three tool-config
    // rewrites, none of which should block the UI thread.
    tauri::async_runtime::spawn_blocking(move || {
        account::save(&base_url, key.as_deref()).map_err(|e| format!("{e:#}"))?;
        // A rotated key is hot-swapped into the running proxy engine below;
        // the relay injects it live per request, so no tool config embeds it.
        if let Some(k) = key.as_deref() {
            // Pasting a key selects the legacy path explicitly.
            account::set_auth_mode(account::AuthMode::ApiKey).map_err(|e| format!("{e:#}"))?;
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            {
                let manager = gate_connect_core::proxy::manager();
                // Drop any live OAuth bearer from the running engine now that
                // we're in ApiKey mode. The background refresh loop is gated on
                // OAuth mode, so it won't clear it; and a still-valid Cognito
                // session would otherwise keep overriding the pasted key until
                // the token expired. No-op when routing is off.
                manager.refresh_token("");
                manager.refresh_api_key(k);
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("save join error: {e}"))?
}

#[tauri::command]
async fn clear_account() -> Result<(), String> {
    // Off the main thread: per-tool config I/O that shouldn't freeze the UI.
    tauri::async_runtime::spawn_blocking(|| {
        // Disconnect managed tools first: clearing the account while their
        // configs still embed the key would leave them routing to the gateway
        // with a dead credential on disk. A failure aborts the sign-out.
        registry::disconnect_all_managed().map_err(|e| format!("{e:#}"))?;
        account::clear().map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| format!("sign-out join error: {e}"))?
}

/// Dev-mode gateway switch: repoint the account at another environment and
/// forget the current Gate key, so the UI can prompt for an
/// environment-appropriate one. Managed tools are disconnected first - their
/// config embeds the old gateway+key, and a later key rotation would push the
/// new key into configs still pointing at the old gateway. Mirrors the URL
/// validation in `save_account` and the disconnect-first order in
/// `clear_account`.
#[tauri::command]
async fn switch_gateway(base_url: String) -> Result<(), String> {
    if base_url.len() > 2048 {
        return Err("base url is unexpectedly long (>2048 bytes)".into());
    }
    let parsed = url::Url::parse(&base_url).map_err(|e| format!("invalid base url: {e}"))?;
    if parsed.scheme() != "https" {
        return Err("base url must use https".into());
    }
    if parsed.host_str().is_none() {
        return Err("base url is missing a host".into());
    }
    // Off the main thread: per-tool config I/O plus keychain delete.
    tauri::async_runtime::spawn_blocking(move || {
        registry::disconnect_all_managed().map_err(|e| format!("{e:#}"))?;
        account::switch_gateway(&base_url).map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| format!("switch join error: {e}"))?
}

// ---- OAuth (Cognito) ----
//
// Gate Connect's own gateway auth via a Cognito access token, the successor
// to the pasted API key. `oauth_begin_login` runs the full interactive flow
// (open the Hosted UI, catch the loopback redirect, exchange the code) off
// the main thread; `oauth_status` / `oauth_sign_out` are cheap keychain
// reads/writes.

#[derive(Serialize)]
struct OAuthStatusDto {
    signed_in: bool,
    email: Option<String>,
    /// Access-token expiry as a Unix timestamp; 0 when signed out.
    expires_at_unix: i64,
}

impl From<&gate_connect_core::oauth::OAuthTokens> for OAuthStatusDto {
    fn from(t: &gate_connect_core::oauth::OAuthTokens) -> Self {
        Self {
            signed_in: true,
            email: t.email(),
            expires_at_unix: t.expires_at_unix,
        }
    }
}

fn oauth_status_now() -> Result<OAuthStatusDto, String> {
    // Share the injector's source of truth (`live_session`): refresh a stale
    // access token so status reflects a live session, and report signed-out when
    // there's no usable session - never signed in, signed out, or the refresh
    // token is dead / unreachable. Reporting signed-out here is what routes the
    // UI back to the sign-in prompt instead of showing a signed-in home that's
    // actually riding the legacy API-key fallback. Keeping a running engine's
    // token fresh is the background refresh loop's job (see `run()`), not this
    // read's, so status stays a read that never mutates engine state.
    Ok(match gate_connect_core::oauth::live_session() {
        Some(t) => OAuthStatusDto::from(&t),
        None => OAuthStatusDto {
            signed_in: false,
            email: None,
            expires_at_unix: 0,
        },
    })
}

/// Run one interactive Cognito login: open the Hosted UI in the browser and
/// capture the redirect on a loopback listener. Blocks (off the main thread)
/// until the user finishes signing in or the flow times out.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
async fn oauth_begin_login(app: tauri::AppHandle) -> Result<OAuthStatusDto, String> {
    use tauri_plugin_opener::OpenerExt;

    let cfg = gate_connect_core::oauth::OAuthConfig::from_build_env()
        .ok_or_else(|| "OAuth is not configured in this build".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let tokens = gate_connect_core::oauth::login(
            &cfg,
            gate_connect_core::oauth::REDIRECT_PORTS,
            |url| {
                app.opener()
                    .open_url(url.to_string(), None::<String>)
                    .map_err(|e| anyhow::anyhow!("opening the sign-in page: {e}"))
            },
        )
        .map_err(|e| format!("{e:#}"))?;
        // Record that this account authenticates via OAuth so load() stops
        // requiring a pasted key, and push the fresh token into a running
        // engine so routing switches to it without waiting for a restart.
        gate_connect_core::account::set_auth_mode(gate_connect_core::account::AuthMode::OAuth)
            .map_err(|e| format!("{e:#}"))?;
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        gate_connect_core::proxy::manager().refresh_token(&tokens.access_token);
        Ok(OAuthStatusDto::from(&tokens))
    })
    .await
    .map_err(|e| format!("login join error: {e}"))?
}

/// Current OAuth sign-in status (signed in, email, expiry).
#[tauri::command]
async fn oauth_status() -> Result<OAuthStatusDto, String> {
    tauri::async_runtime::spawn_blocking(oauth_status_now)
        .await
        .map_err(|e| format!("oauth status join error: {e}"))?
}

/// Forget the stored OAuth tokens (sign out). Leaves `auth_mode` at `OAuth`
/// so the popover shows the sign-in prompt again rather than the legacy
/// key-entry form; choosing the legacy path is an explicit key save.
#[tauri::command]
async fn oauth_sign_out() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        gate_connect_core::oauth::clear().map_err(|e| format!("{e:#}"))?;
        // Revert a running engine to the legacy header immediately (empty
        // token == fall back to the API key, if one is present).
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        gate_connect_core::proxy::manager().refresh_token("");
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("oauth sign-out join error: {e}"))?
}

/// Explicitly set the auth mode. Used when a user chooses the legacy pasted-key
/// path from the sign-in screen; OAuth sign-in sets it implicitly.
#[tauri::command]
async fn set_auth_mode(oauth: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mode = if oauth {
            gate_connect_core::account::AuthMode::OAuth
        } else {
            gate_connect_core::account::AuthMode::ApiKey
        };
        gate_connect_core::account::set_auth_mode(mode).map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| format!("set auth mode join error: {e}"))?
}

/// List the orgs the signed-in user may act on (for the org picker). Reads the
/// current gateway + stored OAuth token and calls the gateway's `/v1/me/orgs`.
#[tauri::command]
async fn oauth_list_orgs() -> Result<Vec<gate_connect_core::org::Org>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        gate_connect_core::org::list_current().map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| format!("list orgs join error: {e}"))?
}

/// Persist the selected org and push it into a running engine/relay so
/// `X-Gate-Org-Id` takes effect live (no restart).
#[tauri::command]
async fn set_org(org_id: String, org_name: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        gate_connect_core::account::set_org(&org_id, &org_name).map_err(|e| format!("{e:#}"))?;
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        gate_connect_core::proxy::manager().refresh_org(&org_id);
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("set org join error: {e}"))?
}

/// OS identifier ("macos" / "windows" / "linux") so the UI can tailor
/// copy: keychain vs Credential Manager, plist vs registry, whether a
/// password prompt appears, etc.
#[tauri::command]
fn app_platform() -> &'static str {
    std::env::consts::OS
}

/// Backend half of the diagnostics report: the facts about this install the
/// webview has no other way to see (OS build, data dir, persisted ports, and
/// the live OS-side readback of both proxy channels). Never fails - an
/// unresolvable field comes back null, because the machines that need this
/// report are the ones where probes fail. Carries no credential; see
/// `gate_connect_core::diagnostics`.
///
/// On macOS this shells out to `networksetup` once per active network
/// service, so it is bound to an explicit user action rather than any poll.
#[tauri::command]
fn diagnostics() -> gate_connect_core::diagnostics::Diagnostics {
    gate_connect_core::diagnostics::collect()
}

// ---- Providers ----
//
// One user-facing switch per model provider. Orchestrates the config
// integrations (cross-platform) and, on macOS when the proxy is already
// running, the matching proxy domains - so the UI shows a single toggle
// instead of exposing the proxy-vs-config split. Delegates to
// `gate_connect_core::provider`.

#[tauri::command]
fn list_providers() -> Vec<gate_connect_core::provider::ProviderState> {
    gate_connect_core::provider::list()
}

#[tauri::command]
fn provider_enable(slug: String) -> Result<gate_connect_core::provider::ProviderState, String> {
    gate_connect_core::provider::enable(&slug).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn provider_disable(slug: String) -> Result<gate_connect_core::provider::ProviderState, String> {
    gate_connect_core::provider::disable(&slug).map_err(|e| format!("{e:#}"))
}

// ---- Built-in MITM proxy (macOS + Windows + Linux) ----
//
// These delegate to the process-global `proxy::manager()`. They're gated to
// the platforms where CA trust + system-proxy wiring is implemented (macOS via
// `security`/`networksetup`, Windows via `certutil`/WinINET, Linux via the
// system trust store + `/etc/environment`) and registered in each platform's
// handler block below. Each build refreshes the tray status dot from
// enable/disable so the routing state shows in the menu bar / taskbar.

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn proxy_status() -> Result<gate_connect_core::proxy::ProxyState, String> {
    gate_connect_core::proxy::manager()
        .status()
        .map_err(|e| format!("{e:#}"))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn proxy_list_domains() -> Result<Vec<gate_connect_core::proxy::ProxyDomain>, String> {
    gate_connect_core::proxy::manager()
        .list_domains()
        .map_err(|e| format!("{e:#}"))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
async fn proxy_enable(
    app: tauri::AppHandle,
) -> Result<gate_connect_core::proxy::ProxyState, String> {
    // Off the main thread: enable can block on the CA-trust admin prompt
    // and waits up to 10s for engine readiness.
    let state = tauri::async_runtime::spawn_blocking(|| {
        // Global ON: restore every provider that was on when routing was last
        // turned off - *before* enabling - so the engine comes back up routing
        // the user's prior selection rather than bare. A no-op (no snapshot) on
        // a first enable, where the engine simply starts with zero domains and
        // passes through until a provider is enabled. Best-effort so a restore
        // hiccup never blocks the proxy from coming up.
        if let Err(e) = gate_connect_core::provider::restore_all() {
            eprintln!("[gate] restoring providers on proxy enable failed: {e}");
            report_backend_error("provider_restore", format!("{e:#}"));
        }
        gate_connect_core::proxy::manager()
            .enable()
            .map_err(|e| format!("{e:#}"))?;
        mark_routing_enabled();
        // Second restore pass now that the proxy is up: domain-only providers
        // (no installed tool) have nothing to configure before the engine is
        // running, so the pre-enable pass leaves them in the snapshot.
        if let Err(e) = gate_connect_core::provider::restore_all() {
            eprintln!("[gate] restoring providers after proxy enable failed: {e}");
            report_backend_error("provider_restore", format!("{e:#}"));
        }
        gate_connect_core::proxy::manager()
            .status()
            .map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| format!("proxy enable join error: {e}"))??;
    // Refresh the tray for the new routing state on every platform: retint the
    // mark, recolor the status dot (green on / gray off), and update the
    // tooltip where supported (macOS + Windows).
    update_tray_status(&app, state.running);
    // Persist the user's intent so the startup auto-enable in `setup` re-routes
    // after a restart. Whether the app actually relaunches at boot is governed
    // separately by the "Launch at login" setting (see `set_launch_at_login`).
    // Best-effort: routing is already on, so a persistence hiccup must not fail
    // the command.
    if let Err(e) = gate_connect_core::proxy::intent::set_intent(true) {
        eprintln!("[gate] persisting routing intent failed: {e}");
        report_backend_error("routing_intent", format!("{e:#}"));
    }
    // Crash safety net: routing is now on, but with no login item registered a
    // crash would strand the system proxy at a dead port with nothing
    // relaunching at boot to run the startup self-heal. Register the login
    // item and arm the pending-disable marker (the deferred-opt-out mechanism,
    // see autostart_optout's module docs), so every existing safe point
    // deregisters it and the Settings toggle keeps reporting the user's
    // choice. Skipped when the user opted in themselves - arming would make
    // the status lie and a later safe point would remove a registration they
    // want. Marker before registration: a crash between the two steps must
    // not leave a registration that reads as the user's choice. Best-effort:
    // routing is already on, so failures only lose the net, not the command.
    //
    // Known (accepted) race: this read-then-write pair and the one in
    // `set_launch_at_login` share no lock, so a Settings toggle landing
    // between this `is_enabled()` check and the arm+enable below can end up
    // marked pending (a fresh opt-in reported as off) until the next safe
    // point clears it. The window is milliseconds wide and both sites are
    // driven by one user in one popover; not worth a lock.
    {
        use gate_connect_core::proxy::autostart_optout;
        use tauri_plugin_autostart::ManagerExt;
        let mgr = app.autolaunch();
        match mgr.is_enabled() {
            Ok(false) => {
                match autostart_optout::record_safety_net_registration() {
                    Ok(()) => {
                        if let Err(e) = mgr.enable() {
                            eprintln!("[gate] registering launch-at-login safety net failed: {e}");
                            report_backend_error("launch_at_login", format!("{e:#}"));
                            // Nothing got registered, so there is nothing for the
                            // marker to defer; leaving it armed would only make a
                            // real opt-in later read as pending.
                            if let Err(e) = autostart_optout::set_pending(false) {
                                eprintln!("[gate] clearing safety-net marker failed: {e}");
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[gate] arming launch-at-login safety-net marker failed: {e}");
                        report_backend_error("launch_at_login", format!("{e:#}"));
                    }
                }
            }
            // Already registered (the user's own opt-in, or a still-pending
            // marker from an earlier session): nothing to arm.
            Ok(true) => {}
            // Can't tell whether a login item exists: don't risk arming the
            // marker over a real opt-in. Losing the net is the lesser harm,
            // but it should be visible.
            Err(e) => {
                eprintln!("[gate] probing launch-at-login for the safety net failed: {e}");
                report_backend_error("launch_at_login", format!("{e:#}"));
            }
        }
    }
    Ok(state)
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
async fn proxy_disable(
    app: tauri::AppHandle,
) -> Result<gate_connect_core::proxy::ProxyState, String> {
    // Off the main thread: disable runs system-proxy subprocesses and joins
    // the engine thread.
    let state = tauri::async_runtime::spawn_blocking(|| {
        // Global OFF: snapshot + disconnect everything managed BEFORE the
        // proxy stops, so config-based tools (Codex) also stop and their
        // domains are still flippable. The full sweep, not the provider-only
        // pass: the catalog maps no provider to OpenCode and friends, and
        // leaving them pointed at the relay we are about to kill would strand
        // them while the UI reports "not routing". Best-effort so it never
        // blocks the kill switch.
        if let Err(e) = gate_connect_core::provider::snapshot_and_disable_everything() {
            eprintln!("[gate] disabling providers on proxy disable failed: {e}");
            report_backend_error("provider_disable", format!("{e:#}"));
        }
        gate_connect_core::proxy::manager()
            .disable()
            .map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| format!("proxy disable join error: {e}"))??;
    // Refresh the tray for the new routing state on every platform: retint the
    // mark, recolor the status dot (green on / gray off), and update the
    // tooltip where supported (macOS + Windows).
    update_tray_status(&app, state.running);
    // Explicit "off" is sticky across restarts: clear the routing intent so the
    // startup auto-enable in `setup` leaves the app in passthrough. Whether the
    // app relaunches at boot is governed separately by the "Launch at login"
    // setting. Best-effort - routing is already off, so a cleanup failure
    // doesn't matter to this command's result.
    if let Err(e) = gate_connect_core::proxy::intent::set_intent(false) {
        eprintln!("[gate] clearing routing intent failed: {e}");
        report_backend_error("routing_intent", format!("{e:#}"));
    }
    // A deferred launch-at-login opt-out can complete now: with routing off,
    // deregistering can no longer strand the system proxy across a restart.
    complete_pending_autostart_disable(&app);
    Ok(state)
}

// Launch at login. A standalone user setting (Settings screen) that owns the
// login item directly - it is no longer armed/disarmed by the routing toggle.
//
// Disabling it while routing is on is two-step: the opt-out marker and the
// defer-vs-deregister decision live in `gate_connect_core::proxy::
// autostart_optout` (see its module docs for the full rationale), and only
// the OS login-item calls live here.

/// Finish a deferred launch-at-login opt-out: deregister the login item and
/// clear the marker. Call only when the system proxy is known to be safe
/// (routing off or already reverted). On failure the marker is kept so a
/// later safe point retries.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn complete_pending_autostart_disable(app: &tauri::AppHandle) {
    use gate_connect_core::proxy::autostart_optout;
    use tauri_plugin_autostart::ManagerExt;
    if !autostart_optout::pending() {
        return;
    }
    if let Err(e) = app.autolaunch().disable() {
        eprintln!("[gate] completing deferred launch-at-login opt-out failed: {e}");
        report_backend_error("launch_at_login", format!("{e:#}"));
        // If the item somehow reads as still registered, keep the marker and
        // retry at the next safe point; if it's gone despite the error, the
        // opt-out is done and the marker can drop.
        if app.autolaunch().is_enabled().unwrap_or(true) {
            return;
        }
    }
    if let Err(e) = autostart_optout::set_pending(false) {
        eprintln!("[gate] clearing launch-at-login opt-out marker failed: {e}");
    }
}

/// Frontend-facing launch-at-login state. `enabled` is the user's choice
/// (what the Settings toggle shows); `pending_disable` reports a deferred
/// opt-out whose deregistration hasn't completed yet - the OS login-items
/// list still shows the app during that window, and Settings explains why.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[derive(serde::Serialize)]
struct LaunchAtLoginStatus {
    enabled: bool,
    pending_disable: bool,
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn launch_at_login_status(app: tauri::AppHandle) -> Result<LaunchAtLoginStatus, String> {
    use tauri_plugin_autostart::ManagerExt;
    let registered = app
        .autolaunch()
        .is_enabled()
        .map_err(|e| format!("{e:#}"))?;
    let pending = gate_connect_core::proxy::autostart_optout::pending();
    Ok(LaunchAtLoginStatus {
        enabled: registered && !pending,
        pending_disable: pending,
    })
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn set_launch_at_login(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    use gate_connect_core::proxy::autostart_optout;
    use tauri_plugin_autostart::ManagerExt;
    // This marker/login-item read-then-write and the safety-net block in
    // `proxy_enable` share no lock; see the accepted-race note there.
    let mgr = app.autolaunch();
    if enabled {
        autostart_optout::set_pending(false).map_err(|e| format!("{e:#}"))?;
        mgr.enable().map_err(|e| format!("{e:#}"))
    } else if autostart_optout::record_disable().map_err(|e| format!("{e:#}"))? {
        mgr.disable().map_err(|e| format!("{e:#}"))
    } else {
        // Routing is on: the opt-out is deferred and the login item stays
        // registered until the next safe point.
        Ok(())
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn proxy_set_domain(
    slug: String,
    enabled: bool,
) -> Result<gate_connect_core::proxy::ProxyState, String> {
    gate_connect_core::proxy::manager()
        .set_domain(&slug, enabled)
        .map_err(|e| format!("{e:#}"))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
async fn proxy_set_env_export(
    enabled: bool,
) -> Result<gate_connect_core::proxy::ProxyState, String> {
    // Off the main thread: this shells out to `launchctl` on macOS and
    // broadcasts a settings change to every top-level window on Windows.
    tauri::async_runtime::spawn_blocking(move || {
        gate_connect_core::proxy::set_env_export(enabled).map_err(|e| format!("{e:#}"))?;
        gate_connect_core::proxy::manager()
            .status()
            .map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| format!("env export join error: {e}"))?
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
async fn proxy_trust_ca() -> Result<gate_connect_core::proxy::ProxyState, String> {
    // Off the main thread: trusting the CA pops an interactive prompt.
    tauri::async_runtime::spawn_blocking(|| {
        gate_connect_core::proxy::manager()
            .trust_ca()
            .map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| format!("trust join error: {e}"))?
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
async fn proxy_untrust_ca() -> Result<gate_connect_core::proxy::ProxyState, String> {
    // Off the main thread: untrusting the CA can pop an interactive prompt.
    tauri::async_runtime::spawn_blocking(|| {
        gate_connect_core::proxy::manager()
            .untrust_ca()
            .map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| format!("untrust join error: {e}"))?
}

/// Bytes of `src-tauri/icons/tray.png` compiled in so the menu bar gets
/// the right asset on first paint without a filesystem lookup at runtime.
const TRAY_ICON_PNG: &[u8] = include_bytes!("../icons/tray.png");

/// When the startup popover was shown. Used to ignore the spurious focus-loss
/// an Accessory (menu-bar) app can emit before it becomes frontmost, which
/// would otherwise immediately hide the popover we open on launch.
/// While true, the popover ignores focus-loss instead of hiding. Set at
/// macOS launch so the keychain-password dialog (and the first-run screen)
/// can't make the window vanish before the user has even seen it - a focus
/// steal by that system dialog would otherwise trip the dismiss-on-blur
/// handler. Cleared by [`unpin_popover`] once the user interacts, restoring
/// normal click-away dismissal. Defaults off so non-macOS behavior is
/// unchanged (only the macOS startup path pins it).
static POPOVER_PINNED: AtomicBool = AtomicBool::new(false);

/// Whether the popover is currently shown. Tracks the hidden→visible edge so
/// the focus hook reconciles once per open, not on every `Focused(true)`: a
/// refocus of an already-visible window (returning from a system dialog, or a
/// pinned-startup blur that never hides) leaves this `true` and is skipped.
/// Set true when the window gains focus; cleared at each real hide/minimize
/// site so the next open reconciles again. Starts false (window not yet shown).
static POPOVER_VISIBLE: AtomicBool = AtomicBool::new(false);

/// Whether the coming exit is an updater-driven relaunch rather than a user
/// quit. The exit handler clears the routing intent on a plain quit when
/// launch-at-login is off (no login item means nothing would re-route after a
/// reboot), but an update install relaunches us immediately - clearing the
/// intent there would leave routing off after every upgrade. Set by the
/// frontend after the update download completes, right before it kicks off
/// the install (not around the whole download: a quit while the download is
/// still running is a genuine user exit and must keep the exit-time intent
/// clear and deferred opt-out completion); reset if the install fails. If the
/// relaunch itself fails after a successful install the flag stays set, which
/// at worst preserves an intent that matched the pre-update state anyway.
static UPDATER_RELAUNCHING: AtomicBool = AtomicBool::new(false);

/// Whether the startup auto-enable brought the engine back on a *different*
/// loopback port than the previous session persisted (including "nothing
/// persisted" - the first launch of a port-persisting build, i.e. an upgrade
/// from an older version). Clients that resolved the proxy at their own
/// launch keep dialing the dead old port until relaunched, so the popover
/// shows a "restart your AI apps" notice while this is set. One-shot per app
/// run: once the port persists, later restarts reuse it and this stays false.
static ROUTED_CLIENTS_MAY_BE_STALE: AtomicBool = AtomicBool::new(false);

/// Whether already-running routed clients may be pointing at a dead port
/// (see [`ROUTED_CLIENTS_MAY_BE_STALE`]). Read-only; the frontend keeps its
/// own dismissed state for the webview's lifetime.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn routed_clients_stale() -> bool {
    ROUTED_CLIENTS_MAY_BE_STALE.load(Ordering::Acquire)
}

/// A backend failure worth surfacing in analytics. The frontend owns the
/// PostHog client, so failures are buffered here until it drains them: the
/// buffer covers the pre-webview window (startup auto-enable runs before the
/// popover mounts), and the nudge event covers failures while it's mounted.
/// `message` stays on this machine - the frontend classifies it and sends
/// only the classified title over the wire, same as invoke rejections.
#[derive(Clone, Serialize)]
struct BackendError {
    context: &'static str,
    message: String,
}

static PENDING_BACKEND_ERRORS: Mutex<Vec<BackendError>> = Mutex::new(Vec::new());
/// Set once in `setup`; lets failure sites without an AppHandle (threads,
/// spawn_blocking closures) nudge the popover.
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

/// Queue a failure for the frontend analytics seam and nudge a mounted
/// popover to drain it. Capped so a repeating failure can't grow unbounded;
/// oldest entries drop first.
fn report_backend_error(context: &'static str, message: String) {
    if let Ok(mut pending) = PENDING_BACKEND_ERRORS.lock() {
        if pending.len() >= 32 {
            pending.remove(0);
        }
        pending.push(BackendError { context, message });
    }
    if let Some(handle) = APP_HANDLE.get() {
        let _ = handle.emit("backend-error-pending", ());
    }
}

/// Hand the buffered backend failures to the frontend and clear the buffer.
#[tauri::command]
fn drain_backend_errors() -> Vec<BackendError> {
    PENDING_BACKEND_ERRORS
        .lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default()
}

/// Process names of the AI tools we're willing to close - deliberately both
/// the agent CLIs *and* the desktop apps that share the binary name: on macOS
/// Claude Desktop / Cowork's main process is literally `Claude`, and it is a
/// routed tool that resolves the proxy at its own launch, so closing it is
/// the point. A subset of the registry tools: `hermes` and `openclaw` are
/// excluded - their names are too generic / their processes shouldn't be
/// killed from here. (An unrelated user binary that happens to be named
/// `claude`/`codex`/`opencode` is accepted collateral; the action sits behind
/// an explicit in-popover confirm.) Matched against the process name with any
/// `.exe` suffix stripped, so one list serves all three desktop OSes.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
const AGENT_PROCESS_NAMES: [&str; 3] = ["claude", "codex", "opencode"];

/// Unix seconds of the most recent successful engine enable in this process
/// (user toggle, startup restore, or a connect_tool auto-enable). 0 = never;
/// `stale_agents_count` then falls back to our own process start, the
/// conservative bound for the Linux case where the detached engine outlived
/// a previous GUI session. Lets the frontend show its "restart your tools"
/// startup hint only for processes that genuinely predate routing, instead
/// of nagging every launch.
static ROUTING_ENABLED_AT_UNIX: AtomicU64 = AtomicU64::new(0);

fn mark_routing_enabled() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    ROUTING_ENABLED_AT_UNIX.store(now, Ordering::Release);
}

/// Visit every running agent process (see [`AGENT_PROCESS_NAMES`]), skipping
/// our own pid. Shared by the close command and the count probe so both match
/// the exact same process set.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn for_each_agent_process(mut f: impl FnMut(&sysinfo::Process)) {
    use sysinfo::{ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let own_pid = sysinfo::get_current_pid().ok();
    for (pid, process) in sys.processes() {
        if Some(*pid) == own_pid {
            continue;
        }
        let name = process.name().to_string_lossy().to_lowercase();
        let name = name.strip_suffix(".exe").unwrap_or(&name);
        if AGENT_PROCESS_NAMES.contains(&name) {
            f(process);
        }
    }
}

/// Count running agent processes without touching them. Lets the frontend
/// skip the "close running agents" routing takeover when there is nothing to
/// close.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn running_agents_count() -> u32 {
    let mut count = 0u32;
    for_each_agent_process(|_| count += 1);
    count
}

/// The Unix second before which a running process counts as pre-routing: the
/// last in-process enable, falling back to our own process start when routing
/// was already up before we launched (detached Linux engine). `None` when
/// neither is available, which callers degrade on rather than guessing.
///
/// Extracted so the count probe and the diagnostics listing cannot answer
/// "does this process predate routing" two different ways.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn routing_bound_unix() -> Option<u64> {
    use sysinfo::{ProcessesToUpdate, System};
    let enabled_at = ROUTING_ENABLED_AT_UNIX.load(Ordering::Acquire);
    if enabled_at != 0 {
        return Some(enabled_at);
    }
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    sysinfo::get_current_pid()
        .ok()
        .and_then(|pid| sys.process(pid))
        .map(|p| p.start_time())
        .filter(|start| *start != 0)
}

/// Count running agent processes that were started *before* routing last came
/// up, i.e. the ones that resolved their connection pre-Gate and genuinely
/// need a restart to route. Same process set as `running_agents_count`; the
/// bound is the last in-process enable, falling back to our own process start
/// when routing was already up before we launched (detached Linux engine).
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn stale_agents_count() -> u32 {
    let Some(bound) = routing_bound_unix() else {
        // No usable bound: degrade to "every running agent counts", the
        // pre-timestamp behavior, rather than silently claiming freshness.
        return running_agents_count();
    };
    let mut count = 0u32;
    for_each_agent_process(|process| {
        if process.start_time() < bound {
            count += 1;
        }
    });
    count
}

/// One running AI tool, as the diagnostics report lists it.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[derive(Serialize)]
struct RunningAgent {
    /// Process name as the OS spells it, original case - "Claude" is the
    /// desktop app, "claude" the CLI, and which one is running matters.
    name: String,
    pid: u32,
    /// Process start, Unix seconds. 0 when the platform wouldn't say.
    started_at_unix: u64,
    /// Started before routing last came up, so it resolved its connection
    /// pre-Gate and needs a restart to route. Same rule as
    /// [`stale_agents_count`], via [`routing_bound_unix`].
    predates_routing: bool,
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[derive(Serialize)]
struct RunningAgentsDto {
    /// The names this scan looks for ([`AGENT_PROCESS_NAMES`]). Reported so an
    /// empty list reads as "none of these three were running" rather than "no
    /// AI tools are running" - the scan does not cover Hermes or OpenClaw, and
    /// a report that hid that would be read as evidence they were stopped.
    scanned_names: Vec<String>,
    agents: Vec<RunningAgent>,
}

/// The running agent processes themselves, not just how many: name, pid, when
/// each started, and whether it predates routing. Same process set and the
/// same staleness rule as the two count probes, so the diagnostics report and
/// the routing takeover can never disagree about what is running.
///
/// Deliberately carries no command line: argv on these tools routinely holds
/// prompts, file paths and occasionally a key, and this list is built to be
/// pasted into a support thread.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn running_agents() -> RunningAgentsDto {
    let bound = routing_bound_unix();
    let mut agents = Vec::new();
    for_each_agent_process(|process| {
        let started_at_unix = process.start_time();
        agents.push(RunningAgent {
            name: process.name().to_string_lossy().to_string(),
            pid: process.pid().as_u32(),
            started_at_unix,
            // No usable bound degrades to "everything predates routing", the
            // same conservative direction `stale_agents_count` takes.
            predates_routing: bound.map(|b| started_at_unix < b).unwrap_or(true),
        });
    });
    // Oldest first: the ones that predate routing are the ones being looked
    // for, and a stable order keeps two reports from the same machine
    // diffable.
    agents.sort_by_key(|agent| agent.started_at_unix);
    RunningAgentsDto {
        scanned_names: AGENT_PROCESS_NAMES.iter().map(|n| n.to_string()).collect(),
        agents,
    }
}

/// Terminate running agent processes (CLIs and desktop apps, see
/// [`AGENT_PROCESS_NAMES`]) so their next launch picks up the routing change.
/// Graceful where the platform allows it (SIGTERM on
/// macOS/Linux, so agents can flush state; Windows only has TerminateProcess).
/// Returns how many processes were signalled - 0 means none were running.
/// Best-effort: processes we can't signal (another user's, already gone) are
/// skipped, not errors.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn close_running_agents() -> u32 {
    use sysinfo::Signal;
    let mut closed = 0u32;
    for_each_agent_process(|process| {
        // kill_with(Term) is None on platforms without signal support
        // (Windows); fall back to the hard kill there.
        let signalled = process
            .kill_with(Signal::Term)
            .unwrap_or_else(|| process.kill());
        if signalled {
            closed += 1;
        }
    });
    closed
}

/// Mark (or unmark) the next exit as an updater-driven relaunch. Called by the
/// frontend after the update download completes and before `install()` -
/// before, because on Windows the installer exits the app from inside that
/// call; after the download, so quitting mid-download still counts as a
/// genuine user exit.
#[tauri::command]
fn set_updater_relaunching(relaunching: bool) {
    UPDATER_RELAUNCHING.store(relaunching, Ordering::Release);
}
/// Whether the OAuth session has died and the user must sign in again. Set by
/// the background refresh loop on the signed-in→dead edge (a `live_session()`
/// that can no longer refresh) and read by the tray-drawing functions to raise
/// an attention signal - a red dot on the glyph and a "sign in required"
/// tooltip - that outranks the routing-on/off color. Relaxed ordering: it only
/// gates a cosmetic redraw. Starts false (assume signed in until proven dead).
static SESSION_NEEDS_SIGNIN: AtomicBool = AtomicBool::new(false);

/// Stop pinning the popover open. The frontend calls this on the user's
/// first interaction with the first-launch window, switching the popover
/// back to normal click-outside-to-dismiss behavior.
#[tauri::command]
fn unpin_popover() {
    POPOVER_PINNED.store(false, Ordering::Release);
}

/// Pin the popover open for the duration of a call that raises a system trust
/// dialog. Without this, `proxy_trust_ca` is the one action in the app that
/// hides the window it was clicked in: the OS dialog takes focus, the
/// `Focused(false)` handler hides the popover, and the copy telling the user
/// what to click goes with it. The frontend pins before the call and unpins
/// in its `finally`, so a cancelled dialog restores click-away dismissal too.
#[tauri::command]
fn pin_popover() {
    POPOVER_PINNED.store(true, Ordering::Release);
}

/// Open (or refocus) the full-size onboarding window. `source` rides along as
/// a query param so the flow can report whether it was a first launch or a
/// replay from Settings.
#[tauri::command]
async fn open_onboarding_window(app: tauri::AppHandle, source: String) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("onboarding") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    // The frontend only ever sends the two known values; normalize anyway so
    // nothing arbitrary is spliced into the webview URL.
    let source = if source == "settings" {
        "settings"
    } else {
        "firstrun"
    };
    // Pass `source` as a URL hash, not a query string: `WebviewUrl::App`
    // with a query string can fail to resolve the page on Windows (blank
    // window), whereas a hash is a client-side fragment the asset resolver
    // ignores. The frontend reads `window.location.hash`.
    let url = tauri::WebviewUrl::App(format!("index.html#{source}").into());
    let builder = tauri::WebviewWindowBuilder::new(&app, "onboarding", url)
        .title("Gate Connect")
        .inner_size(1080.0, 720.0)
        .min_inner_size(760.0, 560.0)
        .center();
    // Overlay title bar: the traffic lights float over the white surface so
    // the window reads as one chrome-less card, per the onboarding design.
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);
    let window = builder.build().map_err(|e| e.to_string())?;
    let _ = window.set_focus();
    Ok(())
}

/// Bring the tray popover back on screen, anchored under the tray icon where
/// the platform can report one. The onboarding flow calls this from its
/// "locate Gate Connect" button and on close, so the handoff always ends at
/// the popover.
fn reveal_popover_window(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    #[cfg(not(target_os = "linux"))]
    if let Some(rect) = app.tray_by_id("main").and_then(|t| t.rect().ok().flatten()) {
        anchor_under_tray(&window, rect.position, rect.size);
    }
    #[cfg(target_os = "linux")]
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    #[cfg(target_os = "macos")]
    order_front_regardless(&window);
}

#[tauri::command]
fn reveal_popover(app: tauri::AppHandle) {
    reveal_popover_window(&app);
}

/// Tray "Quit": exit immediately unless config-routed CLI tools are still
/// managed (Connected, or Drifted - either way their configs point at the
/// loopback relay). macOS / Windows only: there the relay lives in this
/// process and dies with it, so those tools hard-fail until Gate Connect runs
/// again. On Linux the engine lives in a detached helper daemon that outlives
/// the GUI (see core's `manager_linux`), so the relay port keeps serving
/// after a quit and there is nothing to warn about - quit plainly. (In OAuth
/// mode the daemon serves the last-pushed access token, so routing degrades
/// once it expires; still not the dead-port failure the warning describes.)
/// Instead of quitting blind, reveal the popover and let the frontend ask
/// whether to turn the integrations off first; it finishes the quit via
/// [`quit_app`]. The tool names are buffered (not just emitted): a Quit
/// clicked before the webview's listener is up would otherwise be silently
/// swallowed, so the frontend sweeps [`pending_quit_tools`] at mount and on
/// each `quit-requested` nudge (mirrors the backend-error seam).
fn request_quit(app: &tauri::AppHandle) {
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    app.exit(0);
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let app = app.clone();
        // Off the main thread: the status probe reads tool config files.
        tauri::async_runtime::spawn_blocking(move || {
            let connected: Vec<String> = registry::registry()
                .iter()
                .filter(|integ| {
                    matches!(integ.status(), Ok(Status::Connected | Status::Drifted(_)))
                })
                .map(|integ| integ.display_name().to_string())
                .collect();
            if connected.is_empty() {
                app.exit(0);
                return;
            }
            if let Ok(mut pending) = PENDING_QUIT_TOOLS.lock() {
                *pending = Some(connected);
            }
            reveal_popover_window(&app);
            let _ = app.emit("quit-requested", ());
        });
    }
}

/// Quit request buffered by [`request_quit`] for the frontend to sweep; the
/// connected tool names to show in the quit takeover. `None` when no quit is
/// pending.
static PENDING_QUIT_TOOLS: Mutex<Option<Vec<String>>> = Mutex::new(None);

/// Hand the buffered quit request (connected tool names) to the frontend and
/// clear it.
#[tauri::command]
fn pending_quit_tools() -> Option<Vec<String>> {
    PENDING_QUIT_TOOLS.lock().ok().and_then(|mut p| p.take())
}

/// Finish a quit that [`request_quit`] deferred to the popover. Plain exit;
/// the `RunEvent::Exit` handler still reverts the system proxy.
#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

/// Quit-time integration teardown (the "turn off integrations" quit choice):
/// snapshot + disconnect every enabled provider AND the managed standalone
/// tools no provider maps, so all the CLI tools fall back to their original
/// settings while Gate Connect is closed, WITHOUT touching the routing
/// intent - the startup restore reapplies both snapshots the next time the
/// app runs with routing intended on. Fires a system notification (the
/// popover dies with the process) telling the user to restart running CLI
/// agents, which keep the relay address they resolved at their own launch.
#[tauri::command]
async fn disconnect_tools_for_quit(app: tauri::AppHandle) -> Result<(), String> {
    // Off the main thread: disconnect does config-file I/O.
    tauri::async_runtime::spawn_blocking(|| {
        gate_connect_core::provider::snapshot_and_disable_everything().map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| format!("disconnect join error: {e}"))??;
    {
        use tauri_plugin_notification::NotificationExt;
        let _ = app
            .notification()
            .builder()
            .title("Gate Connect")
            .body(
                // "Your tools", not "Integrations": that word reaches the user
                // nowhere else in the product, and this notification arrives
                // seconds after a panel that called them tools. The rest of the
                // wording is shared with QuitConfirm on purpose.
                "Your tools are back on their own settings while Gate Connect is \
                 closed. Restart any running CLI agents; everything reconnects \
                 when Gate Connect starts again.",
            )
            .show();
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // A second launch landed here, in the already-running instance.
            // Reveal the popover, mirroring the "show" tray-menu handler.
            if let Some(window) = app.get_webview_window("main") {
                #[cfg(target_os = "linux")]
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        // Desktop notifications. Registered on all desktop platforms (harmless);
        // fired on macOS + Linux when a dead OAuth session is detected (Windows
        // relies on the tray tooltip). See the refresh loop in `setup`.
        .plugin(tauri_plugin_notification::init())
        // Login item, controlled by the standalone "Launch at login" setting
        // (see `set_launch_at_login`). It is no longer armed/disarmed by the
        // routing toggle; turning it on is what lets the app relaunch and
        // re-route after a restart. The `--silent` arg lets `setup` tell a login
        // launch from a manual one so the popover doesn't flash in the user's
        // face at every boot.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--silent"]),
        ))
        .invoke_handler({
            // The proxy subsystem (and its commands) only exists on the three
            // desktop OSes; the handler forks on that single axis. Forking the
            // whole generate_handler! invocation (rather than per-item cfg)
            // preserves Tauri's compile-time arg/return type-checking.
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            {
                tauri::generate_handler![
                    list_tools,
                    tool_status,
                    connect_tool,
                    disconnect_tool,
                    has_upstream_credential,
                    save_upstream_api_key,
                    clear_upstream_credential,
                    get_account,
                    get_account_key_prefix,
                    backfill_account_key_prefix,
                    save_account,
                    clear_account,
                    switch_gateway,
                    oauth_begin_login,
                    oauth_status,
                    oauth_sign_out,
                    set_auth_mode,
                    oauth_list_orgs,
                    set_org,
                    app_platform,
                    diagnostics,
                    unpin_popover,
                    pin_popover,
                    open_onboarding_window,
                    reveal_popover,
                    quit_app,
                    pending_quit_tools,
                    disconnect_tools_for_quit,
                    list_providers,
                    provider_enable,
                    provider_disable,
                    proxy_status,
                    proxy_list_domains,
                    proxy_enable,
                    proxy_disable,
                    proxy_set_domain,
                    proxy_set_env_export,
                    proxy_trust_ca,
                    proxy_untrust_ca,
                    launch_at_login_status,
                    set_launch_at_login,
                    set_updater_relaunching,
                    routed_clients_stale,
                    running_agents_count,
                    stale_agents_count,
                    running_agents,
                    close_running_agents,
                    drain_backend_errors,
                ]
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
            {
                tauri::generate_handler![
                    list_tools,
                    tool_status,
                    connect_tool,
                    disconnect_tool,
                    has_upstream_credential,
                    save_upstream_api_key,
                    clear_upstream_credential,
                    get_account,
                    get_account_key_prefix,
                    backfill_account_key_prefix,
                    save_account,
                    clear_account,
                    switch_gateway,
                    oauth_status,
                    oauth_sign_out,
                    set_auth_mode,
                    oauth_list_orgs,
                    set_org,
                    app_platform,
                    diagnostics,
                    unpin_popover,
                    pin_popover,
                    open_onboarding_window,
                    reveal_popover,
                    quit_app,
                    pending_quit_tools,
                    disconnect_tools_for_quit,
                    list_providers,
                    provider_enable,
                    provider_disable,
                    set_updater_relaunching,
                    drain_backend_errors,
                ]
            }
        })
        .on_window_event(|window, event| {
            // A system Light/Dark switch must re-tint the tray mark at once:
            // the routing-status refresh only fires on proxy changes, so
            // without this the glyph would keep its old (possibly invisible)
            // tone until the next toggle.
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            if let WindowEvent::ThemeChanged(_) = event {
                if let Ok(st) = gate_connect_core::proxy::manager().status() {
                    update_tray_status(window.app_handle(), st.running);
                }
            }
            // The onboarding window is a regular window: closing it really
            // closes it, and losing focus must not dismiss it. Hand the user
            // back to the popover so "Get started" (and an early close) both
            // land in the app.
            if window.label() == "onboarding" {
                if let WindowEvent::CloseRequested { .. } = event {
                    reveal_popover_window(window.app_handle());
                }
                return;
            }
            // X-button on the popover should hide it, not quit the app.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                POPOVER_VISIBLE.store(false, Ordering::Release);
            }
            // Opening the popover (the hidden→visible edge) is our cue to pick
            // up any tool installed since launch - e.g. Claude Code installed
            // after Gate Connect - and wire it up without a relaunch. Guarded by
            // POPOVER_VISIBLE so a refocus of an already-open window doesn't
            // re-run it; the flag is cleared at each hide site below. This is the
            // config route, so it runs on every platform (unlike the
            // Focused(false) dismiss below). Off-thread + best-effort so it never
            // blocks the event loop; reconcile_enabled is idempotent and only
            // writes when a tool is newly installed.
            if let WindowEvent::Focused(true) = event {
                if !POPOVER_VISIBLE.swap(true, Ordering::AcqRel) {
                    std::thread::spawn(|| {
                        if let Err(e) = gate_connect_core::provider::reconcile_enabled() {
                            eprintln!("[gate] provider config reconcile on focus failed: {e}");
                            report_backend_error("provider_reconcile", format!("{e:#}"));
                        }
                    });
                }
            }
            // Click outside the popover → dismiss. Linux is excluded: there the
            // window is a normal decorated, taskbar-visible window (see setup),
            // and a dismiss-on-blur reflex fights its own title bar - grabbing
            // the CSD title bar or close button momentarily blurs the GTK
            // toplevel, so minimizing here would yank drag/close out from under
            // the user. Linux dismisses via its native controls instead.
            #[cfg(not(target_os = "linux"))]
            if let WindowEvent::Focused(false) = event {
                // While pinned (first launch, through the keychain-password
                // dialog and first-run, until the user engages), a focus loss
                // - the keychain dialog stealing focus, or the spurious blur an
                // Accessory app emits right after the startup show - must not
                // hide the popover, or the user never sees the window.
                if POPOVER_PINNED.load(Ordering::Acquire) {
                    return;
                }
                let _ = window.hide();
                POPOVER_VISIBLE.store(false, Ordering::Release);
            }
        })
        .setup(|app| {
            // Lets failure sites without a handle of their own nudge the
            // popover to drain buffered analytics errors.
            let _ = APP_HANDLE.set(app.handle().clone());

            // Hide the dock icon - Gate Connect lives in the menu bar.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // Launch at login defaults ON: arm the login item the first time
            // we run so routing persists across a restart out of the box. A
            // one-shot marker file records that the default has been applied,
            // so a later user opt-out in Settings sticks - the OS login-item
            // flag alone can't tell "never configured" from "user turned it
            // off", since both read as disabled.
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            {
                use tauri_plugin_autostart::ManagerExt;
                if let Ok(dir) = gate_connect_core::env::app_support_dir() {
                    let marker = dir.join("autostart-defaulted");
                    if !marker.exists() {
                        if let Err(e) = app.autolaunch().enable() {
                            eprintln!("[gate] enabling launch-at-login default failed: {e}");
                            report_backend_error("launch_at_login", format!("{e}"));
                        }
                        let _ = std::fs::create_dir_all(&dir);
                        let _ = std::fs::write(&marker, b"1");
                    }
                }
            }

            // Apply gateway config to any tool installed *after* its provider
            // was enabled (e.g. Claude Code installed after Gate Connect). This
            // is the config route, independent of the proxy/routing intent, so
            // it runs on every launch regardless of whether the proxy comes up
            // below. Off-thread + best-effort: a slow or failing tool can't
            // stall the tray or block the startup proxy work.
            std::thread::spawn(|| {
                if let Err(e) = gate_connect_core::provider::reconcile_enabled() {
                    eprintln!("[gate] provider config reconcile on startup failed: {e}");
                    report_backend_error("provider_reconcile", format!("{e:#}"));
                }
            });

            // Startup proxy work runs off-thread so neither step stalls the
            // tray: reconcile can block on a rare admin prompt, and the
            // auto-enable below waits on engine readiness. `--silent` marks a
            // login-item launch (see the autostart plugin registration) so we
            // can re-route after a reboot without flashing the popover in the
            // user's face at every boot.
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            let silent_launch = std::env::args().any(|a| a == "--silent");
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    // OAuth: refresh a stale Cognito access token before the
                    // engine seeds itself below, so re-honor / auto-enable
                    // inject a live token (enable() re-reads the stored token
                    // via access_token_for_injection). Never opens the browser
                    // - a failed refresh just leaves the popover in the "sign
                    // in" state the UI derives from oauth_status. Best-effort.
                    if gate_connect_core::account::auth_mode().unwrap_or_default()
                        == gate_connect_core::account::AuthMode::OAuth
                    {
                        if let Some(cfg) = gate_connect_core::oauth::OAuthConfig::from_build_env() {
                        // Seed the tray attention flag from the result so the
                        // first tray paint in the auto-enable below is already
                        // correct, instead of showing a misleading routing dot
                        // for up to one refresh interval. Only a refresh
                        // *failure* (Err: a stored session that can't refresh)
                        // is an alarm; `Ok(None)` is a signed-out / never
                        // signed-in state and must stay quiet.
                        match gate_connect_core::oauth::ensure_fresh(&cfg) {
                            Ok(session) => {
                                SESSION_NEEDS_SIGNIN.store(false, Ordering::Relaxed);
                                // A locally-fresh session can still be dead at
                                // the gateway (upgrade / server-side drift:
                                // revoked user, reseeded org data) - the token
                                // looks fine here, so only the gateway can say.
                                // Ask it once, before the engine below seeds
                                // itself from this session. Offline starts get
                                // no verdict and change nothing.
                                if let (Some(tokens), Ok(Some(gateway))) = (
                                    session,
                                    gate_connect_core::account::load_base_url(),
                                ) {
                                    use gate_connect_core::org::SessionProbe;
                                    match gate_connect_core::org::probe_session(
                                        &gateway,
                                        &tokens.access_token,
                                    ) {
                                        SessionProbe::Rejected => {
                                            eprintln!(
                                                "[gate] gateway rejected the stored OAuth session; prompting sign-in"
                                            );
                                            gate_connect_core::oauth::mark_session_rejected();
                                            SESSION_NEEDS_SIGNIN.store(true, Ordering::Relaxed);
                                        }
                                        SessionProbe::Accepted(orgs) => {
                                            // Session is live, but a stored org that
                                            // dropped out of the membership list would
                                            // doom every request; clear it so the UI
                                            // routes to the org picker (`needsOrg`).
                                            let stale = gate_connect_core::account::selected_org()
                                                .ok()
                                                .flatten()
                                                .is_some_and(|(id, _)| {
                                                    !orgs.iter().any(|o| o.org_id == id)
                                                });
                                            if stale {
                                                eprintln!(
                                                    "[gate] stored org is no longer a membership; clearing it for re-pick"
                                                );
                                                if let Err(e) =
                                                    gate_connect_core::account::clear_org()
                                                {
                                                    eprintln!(
                                                        "[gate] clearing the stale org failed: {e:#}"
                                                    );
                                                }
                                            }
                                        }
                                        SessionProbe::Unavailable => {}
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("[gate] startup OAuth token refresh failed: {e}");
                                SESSION_NEEDS_SIGNIN.store(true, Ordering::Relaxed);
                            }
                        }
                    }
                    }

                    // If a previous session left the system proxy on (unclean
                    // quit / crash), revert it first so HTTPS isn't routed at a
                    // dead loopback port. A clean disable leaves nothing to do.
                    if let Err(e) = gate_connect_core::proxy::manager().reconcile_on_startup() {
                        eprintln!("proxy startup reconcile failed: {e}");
                        report_backend_error("restore_routing", format!("{e:#}"));
                    }

                    // A deferred launch-at-login opt-out reaching a login-item
                    // launch means the previous session never hit a clean stop
                    // (crash / hard restart; on Linux even a clean quit, since
                    // the exit handler below is macOS/Windows-only). Make sure
                    // routing is actually off, then finish the job: deregister,
                    // drop the routing intent so a later manual launch stays
                    // passthrough, and exit - the user asked us not to run at
                    // startup. On macOS/Windows the reconcile above has already
                    // *reverted* any stranded system proxy; on Linux it does
                    // the opposite - it re-honors (re-enables) a leftover
                    // snapshot - so without an explicit disable here the daemon
                    // would keep intercepting headless after we exit, and the
                    // re-written snapshot would make every later manual launch
                    // re-honor routing again with the intent cleared. A manual
                    // or updater-driven launch (not --silent) skips this and
                    // restores routing as the user left it; the still-pending
                    // opt-out completes at the next safe point instead.
                    if silent_launch && gate_connect_core::proxy::autostart_optout::pending() {
                        // Linux-only: macOS/Windows reconciled to "off" above,
                        // and running disable there would force_off proxy
                        // settings the reconcile just restored (e.g. a
                        // corporate proxy). Best-effort: even a failed
                        // disable_quiet has dropped the daemon to pass-through
                        // and cleared the snapshot, so nothing is stranded.
                        #[cfg(target_os = "linux")]
                        if let Err(e) = gate_connect_core::proxy::manager().disable_quiet() {
                            eprintln!(
                                "[gate] disabling re-honored routing for the deferred opt-out failed: {e}"
                            );
                            report_backend_error("restore_routing", format!("{e:#}"));
                        }
                        complete_pending_autostart_disable(&handle);
                        if let Err(e) = gate_connect_core::proxy::intent::set_intent(false) {
                            eprintln!("[gate] clearing routing intent failed: {e}");
                            report_backend_error("routing_intent", format!("{e:#}"));
                        }
                        handle.exit(0);
                        return;
                    }

                    // Restart persistence: bring routing back if the user last
                    // left it on. The exit-time disable reverts the *system
                    // proxy* only and keeps the routing intent when "Launch at
                    // login" is on, so this is what re-routes after a reboot.
                    // (With launch-at-login off, exit clears the intent, so we
                    // stay passthrough.) No intent recorded means first run, or
                    // the user left routing off - stay passthrough.
                    if !gate_connect_core::proxy::intent::load_intent() {
                        return;
                    }
                    // Mirror proxy_enable: restore any snapshotted providers
                    // before the engine comes up so it routes the prior
                    // selection. A no-op in the common restart case (routed
                    // domains persist in config and the engine reloads them on
                    // enable); harmless and idempotent otherwise.
                    if let Err(e) = gate_connect_core::provider::restore_all() {
                        eprintln!("[gate] restoring providers on startup auto-enable failed: {e}");
                        report_backend_error("provider_restore", format!("{e:#}"));
                    }
                    // Snapshot the persisted ports before enable overwrites
                    // them; comparing them against the state enable returns
                    // tells us whether the engine (and PAC listener) came back
                    // on the previous session's address. The returned state is
                    // the authority for the new ports - the post-enable
                    // persistence is best-effort, so re-reading the files here
                    // could compare enable's own input back against itself.
                    let prior_port = gate_connect_core::proxy::system_proxy::load_port();
                    #[cfg(any(target_os = "macos", target_os = "windows"))]
                    let prior_pac_port = gate_connect_core::proxy::system_proxy::load_pac_port();
                    match gate_connect_core::proxy::manager().enable() {
                        Ok(state) => {
                            mark_routing_enabled();
                            // Engine port changed (or none was persisted - the
                            // first launch after upgrading from a build without
                            // port persistence): clients that resolved the
                            // proxy at their own launch are now dialing a dead
                            // port. A changed PAC port breaks them more
                            // quietly: the AutoConfigURL they captured stops
                            // serving and they silently fall back to DIRECT,
                            // bypassing Gate. Either way, surface a "restart
                            // your AI apps" notice in the popover. An
                            // unreadable port file reads as "unknown", not
                            // "changed" - the engine may well have come back on
                            // the same port (its own load can succeed where
                            // this one failed), and a false notice nags the
                            // user for nothing.
                            let engine_moved =
                                prior_port.map(|p| p != state.port).unwrap_or(false);
                            #[cfg(any(target_os = "macos", target_os = "windows"))]
                            let pac_moved =
                                prior_pac_port.map(|p| p != state.pac_port).unwrap_or(false);
                            #[cfg(target_os = "linux")]
                            let pac_moved = false;
                            if engine_moved || pac_moved {
                                ROUTED_CLIENTS_MAY_BE_STALE.store(true, Ordering::Release);
                            }
                            // Second restore pass for domain-only providers the
                            // pre-enable pass left in the snapshot (nothing to
                            // configure until the proxy is running).
                            if let Err(e) = gate_connect_core::provider::restore_all() {
                                eprintln!(
                                    "[gate] restoring providers after startup auto-enable failed: {e}"
                                );
                                report_backend_error("provider_restore", format!("{e:#}"));
                            }
                            // Reflect the auto-enabled routing in the tray:
                            // retint the mark, turn the status dot green, and
                            // set the tooltip where supported (macOS + Windows).
                            update_tray_status(&handle, state.running);
                            // Nudge an already-mounted popover to re-read: its
                            // status poll is idle while routing last read as
                            // off, so it won't notice the flip on its own. The
                            // new state rides along as the payload.
                            let _ = handle.emit("proxy-state-changed", &state);
                        }
                        Err(e) => {
                            // Never surface a stray dialog at login. If the
                            // enable can't complete unattended (no Gate account,
                            // a prompt we won't raise), drop the auto-route; on a
                            // silent launch, open the popover so the user can
                            // finish it. A visible launch already shows it below.
                            eprintln!("[gate] startup auto-enable failed: {e}");
                            report_backend_error("restore_routing", format!("{e:#}"));
                            if silent_launch {
                                if let Some(window) = handle.get_webview_window("main") {
                                    POPOVER_PINNED.store(true, Ordering::Release);
                                    let _ = window.show();
                                    let _ = window.set_focus();
                                }
                            }
                        }
                    }
                });
            }

            // Keep the Cognito access token fresh for the whole session. The
            // engine (and its embedded relay) seed the token once at enable()
            // and only re-read it on login / sign-out, so without this a
            // long-lived session would keep injecting the access token past its
            // ~1h expiry and the gateway would start rejecting traffic until the
            // next launch. Mirror the standalone CLI relay's silent-refresh
            // loop: every 30s, refresh if near expiry and push the live token
            // into a running engine (a no-op when routing is off). Never opens
            // the browser - a failed refresh just lets the token lapse to the
            // "sign in" state the UI derives from oauth_status. Best-effort, off
            // the tray thread.
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            {
                let refresh_handle = app.handle().clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(
                        gate_connect_core::oauth::REFRESH_INTERVAL_SECS,
                    ));
                    if gate_connect_core::account::auth_mode().unwrap_or_default()
                        != gate_connect_core::account::AuthMode::OAuth
                    {
                        // Not an OAuth account (e.g. the user switched a dead
                        // session to a pasted key): clear any stale attention
                        // signal so the tray doesn't strand a red dot, then idle.
                        if SESSION_NEEDS_SIGNIN.swap(false, Ordering::Relaxed) {
                            let running = gate_connect_core::proxy::manager()
                                .status()
                                .map(|s| s.running)
                                .unwrap_or(false);
                            update_tray_status(&refresh_handle, running);
                        }
                        continue;
                    }
                    // `live_session` silently refreshes a stale token (persisting
                    // it) and yields None when the session is dead; push the
                    // result into the running engine (a no-op when routing is
                    // off). "" reverts to the API-key fallback, matching the
                    // signed-out state the UI derives from oauth_status.
                    let token = gate_connect_core::oauth::live_session()
                        .map(|t| t.access_token)
                        .unwrap_or_default();
                    gate_connect_core::proxy::manager().refresh_token(&token);

                    // Raise (or clear) the tray attention signal on the
                    // signed-in↔dead edge. "Dead" means a stored session exists
                    // but can no longer refresh (expired / revoked) - NOT a
                    // deliberate sign-out, which clears the stored tokens
                    // (`oauth::clear`) and so must stay quiet even though
                    // auth_mode is still OAuth. Redraw only on a change so the
                    // tray isn't rewritten every 30s.
                    let dead = token.is_empty()
                        && gate_connect_core::oauth::current()
                            .ok()
                            .flatten()
                            .is_some();
                    if SESSION_NEEDS_SIGNIN.swap(dead, Ordering::Relaxed) != dead {
                        let running = gate_connect_core::proxy::manager()
                            .status()
                            .map(|s| s.running)
                            .unwrap_or(false);
                        update_tray_status(&refresh_handle, running);
                        // First tick that finds the session dead: nudge the user
                        // with a system notification on macOS + Linux, so the
                        // dead session is noticed even when the popover is closed
                        // and the menu-bar/tray dot is out of the user's eyeline.
                        // Fired once per death by the edge guard above.
                        #[cfg(any(target_os = "macos", target_os = "linux"))]
                        if dead {
                            use tauri_plugin_notification::NotificationExt;
                            let _ = refresh_handle
                                .notification()
                                .builder()
                                .title("Gate Connect")
                                .body("Your session expired. Open Gate Connect to sign in again and keep routing.")
                                .show();
                        }
                    }
                });
            }

            // Linux dismisses the popover by minimizing (see the Focused(false)
            // handler), so it needs a taskbar/dock entry to restore from -
            // override the config's skipTaskbar:true here. macOS hides from the
            // dock via the Accessory activation policy instead, and Windows
            // keeps its hide-to-tray behavior, so neither wants this.
            #[cfg(target_os = "linux")]
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_skip_taskbar(false);
                // Stay borderless (config `decorations: false`): the native CSD
                // title-bar buttons don't reliably bind on Wayland/GNOME, so the
                // frontend draws its own chrome (`LinuxTitleBar`) and calls the
                // Tauri window APIs directly. A native title bar here would just
                // compete with that custom strip.
                //
                // Drop the config's alwaysOnTop on Linux: with no dismiss-on-blur
                // here (see the Focused handler), an always-on-top window would
                // float over everything with no way to recede when the user
                // switches apps. As a normal window it sinks behind on focus
                // loss and is re-summoned from the taskbar or tray.
                let _ = window.set_always_on_top(false);
            }

            // Round the NSWindow content view's CALayer so the transparent
            // window itself has rounded corners - without this, CSS-rounded
            // corners on the popover expose the dark window behind them.
            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                promote_to_nonactivating_panel(&window);
                apply_window_corner_radius(&window, 12.0);
                apply_popover_space_behavior(&window);
                install_click_outside_dismiss(app.handle());
            }
            #[cfg(target_os = "macos")]
            watch_menu_bar_appearance(app.handle());

            let tray_icon = Image::from_bytes(TRAY_ICON_PNG)?;

            let show_item = MenuItemBuilder::with_id("show", "Open Gate Connect").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "Quit Gate Connect").build(app)?;
            let menu = MenuBuilder::new(app)
                .items(&[&show_item, &quit_item])
                .build()?;

            TrayIconBuilder::with_id("main")
                .icon(tray_icon)
                .icon_as_template(true) // stand-in until update_tray_status paints the real icon
                .tooltip("Gate Connect") // baseline; macOS refines it to the routing state
                .menu(&menu)
                .show_menu_on_left_click(false) // left-click toggles window; right-click shows menu
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            // On Linux the SNI/AppIndicator tray doesn't hand us
                            // a click rect, and on GNOME the left-click path
                            // often never fires - users reach this code path via
                            // the right-click menu. Anchor at the current cursor.
                            #[cfg(target_os = "linux")]
                            if let Ok(cursor) = app.cursor_position() {
                                anchor_at_cursor(&window, cursor);
                            }
                            #[cfg(target_os = "linux")]
                            let _ = window.unminimize();
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => request_quit(app),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        rect,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            // is_focused() is unreliable for a non-activating panel.
                            // On Linux the popover is dismissed by minimizing, so a
                            // minimized window counts as "away" and should restore,
                            // not toggle off.
                            let is_visible = window.is_visible().unwrap_or(false);
                            let is_minimized = window.is_minimized().unwrap_or(false);
                            if is_visible && !is_minimized {
                                #[cfg(target_os = "linux")]
                                let _ = window.minimize();
                                #[cfg(not(target_os = "linux"))]
                                let _ = window.hide();
                                POPOVER_VISIBLE.store(false, Ordering::Release);
                            } else {
                                // Linux trays don't report a usable rect; fall
                                // back to the cursor position so the popover lands
                                // near the user's click.
                                #[cfg(target_os = "linux")]
                                {
                                    let _ = &rect;
                                    if let Ok(cursor) = app.cursor_position() {
                                        anchor_at_cursor(&window, cursor);
                                    }
                                }
                                #[cfg(not(target_os = "linux"))]
                                anchor_under_tray(&window, rect.position, rect.size);
                                #[cfg(target_os = "linux")]
                                let _ = window.unminimize();
                                let _ = window.show();
                                let _ = window.set_focus();
                                #[cfg(target_os = "macos")]
                                order_front_regardless(&window);
                            }
                        }
                    }
                })
                .build(app)?;

            // First impression: a hidden menu-bar / tray app looks broken on
            // launch, so surface the popover once at startup on every desktop
            // OS - but only on a visible, user-initiated launch. A login-item
            // launch (`--silent`) stays in the tray: the background thread
            // above re-routes quietly, and flashing the popover at every boot
            // would be hostile. Subsequent opens go through the tray click.
            //
            // Pin it open first: the frontend's initial load reads the OS
            // credential store, and the unlock dialog that can trigger (the
            // macOS keychain prompt, the GNOME keyring) would otherwise blur the
            // popover and dismiss it before the user sees anything. The window
            // stays put until the user interacts (`unpin_popover`).
            //
            // Show it here, from Rust: a hidden WKWebView reports visibility
            // "hidden" and WebKit suspends requestAnimationFrame, so revealing
            // from the frontend never fires and the popover never opens on
            // launch. The blank-window flash that a synchronous show otherwise
            // causes (the window appears before WKWebView's first paint) is
            // handled by painting the rounded content layer white up front (see
            // `apply_window_corner_radius`), so the first frame is the splash's
            // white card rather than a transparent flash.
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            if let Some(window) = app.get_webview_window("main").filter(|_| !silent_launch) {
                POPOVER_PINNED.store(true, Ordering::Release);

                // macOS: the tray icon has no laid-out rect yet at setup, and the
                // stale value it reports lands the popover in the wrong corner -
                // so place it deterministically under the menu bar instead.
                #[cfg(target_os = "macos")]
                position_startup(&window);
                // Windows: the tray rect is reliable here; anchor under it when
                // present, otherwise keep the configured default position.
                #[cfg(target_os = "windows")]
                {
                    let _ = app
                        .tray_by_id("main")
                        .and_then(|t| t.rect().ok().flatten())
                        .map(|rect| anchor_under_tray(&window, rect.position, rect.size));
                }
                let _ = window.show();
                let _ = window.set_focus();
            }

            // Reflect the current proxy state in the tray at launch: tint the
            // mark for the menu-bar / taskbar appearance, add the status dot,
            // and refresh the tooltip (macOS + Windows).
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            {
                let running = gate_connect_core::proxy::manager()
                    .status()
                    .map(|s| s.running)
                    .unwrap_or(false);
                update_tray_status(app.handle(), running);
            }
            // Linux has no tooltip, but the mark still needs tinting for the
            // panel's light/dark theme plus the status dot so it stays visible.
            #[cfg(target_os = "linux")]
            if let Ok(st) = gate_connect_core::proxy::manager().status() {
                update_tray_status(app.handle(), st.running);
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Gate Connect")
        .run(|app_handle, event| {
            // On app exit, revert the system proxy so traffic is never stranded
            // at the now-dead engine port. The engine lives in a process-global
            // static whose Drop is bypassed at normal exit, so without this the
            // system proxy stays pointed at a dead listener and kills
            // connectivity until the next launch's self-heal. disable_quiet()
            // is promptless and leaves the CA trusted.
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            if let tauri::RunEvent::Exit = &event {
                if let Err(e) = gate_connect_core::proxy::manager().disable_quiet() {
                    eprintln!("[gate] reverting proxy on exit failed: {e}");
                }
                // The login item is now a standalone "Launch at login" setting,
                // decoupled from routing. A deferred opt-out (toggled off while
                // routing was on) completes here: disable_quiet() above has
                // reverted the system proxy, so deregistering can no longer
                // strand it. An updater-driven relaunch is exempt - the app
                // comes right back, so the pending opt-out stays armed and
                // routing is restored exactly as the user left it.
                use tauri_plugin_autostart::ManagerExt;
                if !UPDATER_RELAUNCHING.load(Ordering::Acquire) {
                    complete_pending_autostart_disable(app_handle);
                }
                // If the user hasn't asked Gate to launch at login, it won't
                // relaunch to re-route after a restart - so clear the routing
                // intent too, otherwise a later manual launch would silently
                // re-enable routing. Launch-at-login on keeps the intent, so
                // opting in is what persists routing across a restart. A
                // still-pending opt-out counts as off: it's the user's choice,
                // even if the deregistration above couldn't complete.
                if !UPDATER_RELAUNCHING.load(Ordering::Acquire)
                    && (!app_handle.autolaunch().is_enabled().unwrap_or(false)
                        || gate_connect_core::proxy::autostart_optout::pending())
                {
                    if let Err(e) = gate_connect_core::proxy::intent::set_intent(false) {
                        eprintln!("[gate] clearing routing intent on exit failed: {e}");
                    }
                }
            }
            let _ = &event;
            let _ = &app_handle;
        });
}

/// Position the popover window centered horizontally on the tray icon
/// and just above or below it, whichever side has room on the current
/// monitor. macOS's menu bar lives at the top so the popover anchors
/// below the icon; Windows' taskbar is typically at the bottom and
/// anchoring below would push the popover off-screen - so we flip it
/// above the icon. X is clamped to the monitor bounds so a tray icon
/// near a screen edge doesn't push the popover past it.
/// Pick the monitor whose physical bounds contain point (x, y) - used to
/// place the popover on the same display as the tray icon that was clicked,
/// not whichever monitor the window happened to be on last. Falls back to the
/// window's current monitor, then the primary, so we always have somewhere to
/// show. Coordinates are physical pixels in the virtual-desktop space, matching
/// `Monitor::position()`/`size()`.
#[cfg(not(target_os = "linux"))]
fn monitor_at(window: &tauri::WebviewWindow, x: f64, y: f64) -> Option<tauri::Monitor> {
    let contains = |m: &tauri::Monitor| {
        let p = m.position();
        let s = m.size();
        x >= p.x as f64
            && x < p.x as f64 + s.width as f64
            && y >= p.y as f64
            && y < p.y as f64 + s.height as f64
    };
    window
        .available_monitors()
        .ok()
        .and_then(|ms| ms.into_iter().find(contains))
        .or_else(|| window.current_monitor().ok().flatten())
        .or_else(|| window.primary_monitor().ok().flatten())
}

/// On launch, place the popover at the top-right of the main display - just
/// under the macOS menu bar, where the tray icon lives - so first-time users
/// who double-click the app from Applications actually see the UI instead of a
/// seemingly-dead menu-bar icon. Subsequent opens reposition under the clicked
/// tray icon via `anchor_under_tray`.
#[cfg(target_os = "macos")]
fn position_startup(window: &tauri::WebviewWindow) {
    let Some(m) = window
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| window.current_monitor().ok().flatten())
    else {
        return;
    };
    let scale = m.scale_factor();
    let mp = m.position();
    let ms = m.size();
    let win_w = window
        .outer_size()
        .map(|s| s.width as f64)
        .unwrap_or(380.0 * scale);
    let margin = 8.0 * scale;
    let menubar = 28.0 * scale; // clear the macOS menu bar
    let x = (mp.x as f64 + ms.width as f64 - win_w - margin).round() as i32;
    let y = (mp.y as f64 + menubar).round() as i32;
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

/// Build the tray image, recoloring the hex mark to a high-contrast tone for
/// the current menu-bar / taskbar appearance (light vs dark) so it stays
/// visible on any backdrop, then compositing a colored status dot on top for
/// all platforms. A dead OAuth session (`needs_signin`) draws a red "sign in
/// required" dot; otherwise the routine routing dot is green when the proxy is
/// routing, gray when off.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn tray_image(proxy_on: bool, needs_signin: bool, dark_menubar: bool) -> Option<Image<'static>> {
    let base = Image::from_bytes(TRAY_ICON_PNG).ok()?;
    let w = base.width();
    let h = base.height();
    let mut rgba = base.rgba().to_vec();

    // Recolor the silhouette (preserve its alpha) for menu-bar contrast.
    let (mr, mg, mb): (u8, u8, u8) = if dark_menubar {
        (0xE6, 0xE8, 0xEE) // near-white on a dark menu bar
    } else {
        (0x3A, 0x3D, 0x4D) // dark navy-gray on a light menu bar
    };
    for px in rgba.chunks_exact_mut(4) {
        if px[3] > 0 {
            px[0] = mr;
            px[1] = mg;
            px[2] = mb;
        }
    }

    // Composite the status dot, bottom-right: the one colored element. A dead
    // OAuth session (`needs_signin`) shows a red "sign in required" dot;
    // otherwise it tracks routing - green when the proxy is routing, gray when
    // off. Rendered on every platform - macOS composites it over the
    // (temporarily non-template) mark, and the Windows/Linux trays carry the
    // full-color icon directly.
    {
        let (dr, dg, db): (u8, u8, u8) = if needs_signin {
            (0xE5, 0x48, 0x4D) // red - sign in required
        } else if proxy_on {
            (0x2E, 0xCC, 0x71) // green - routing
        } else {
            (0x8A, 0x8F, 0x9A) // gray - off
        };
        let radius = (w as f32 * 0.20).round() as i32;
        let cx = w as i32 - radius - 2;
        let cy = h as i32 - radius - 2;
        for y in (cy - radius)..=(cy + radius) {
            for x in (cx - radius)..=(cx + radius) {
                if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                    continue;
                }
                let dx = x - cx;
                let dy = y - cy;
                if dx * dx + dy * dy <= radius * radius {
                    let idx = ((y as u32 * w + x as u32) * 4) as usize;
                    rgba[idx] = dr;
                    rgba[idx + 1] = dg;
                    rgba[idx + 2] = db;
                    rgba[idx + 3] = 0xFF;
                }
            }
        }
    }

    Some(Image::new_owned(rgba, w, h))
}

/// Refresh the tray icon for the current appearance: tint the mark against the
/// menu bar / taskbar it's sitting on, and overlay the status dot (routing, or
/// the red sign-in-required dot when the OAuth session is dead - see
/// `tray_image`). Also refreshes the tooltip on macOS + Windows.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn update_tray_status(app: &tauri::AppHandle, proxy_on: bool) {
    use tauri::Manager;
    let needs_signin = SESSION_NEEDS_SIGNIN.load(Ordering::Relaxed);
    let system_dark = || {
        app.get_webview_window("main")
            .and_then(|win| win.theme().ok())
            .map(|t| t == tauri::Theme::Dark)
            .unwrap_or(false)
    };

    // macOS: the system theme is the wrong question (see `menu_bar_is_dark`),
    // so ask the menu bar and keep the sampled value for the watcher to diff
    // against - otherwise it would re-apply the icon on its next tick.
    #[cfg(target_os = "macos")]
    let dark = {
        let dark = menu_bar_is_dark().unwrap_or_else(system_dark);
        MENU_BAR_DARK.store(i8::from(dark), Ordering::Release);
        dark
    };
    #[cfg(not(target_os = "macos"))]
    let dark = system_dark();

    if let Some(tray) = app.tray_by_id("main") {
        // The colored dot requires non-template rendering, which forfeits the
        // automatic macOS tinting - `menu_bar_is_dark` above is what stands in
        // for it. Windows/Linux icons are never templates and already carry
        // full color.
        #[cfg(target_os = "macos")]
        let _ = tray.set_icon_as_template(false);
        if let Some(img) = tray_image(proxy_on, needs_signin, dark) {
            let _ = tray.set_icon(Some(img));
        }
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    update_tray_tooltip(app, proxy_on);
}

/// Set the tray hover tooltip. Cross-platform (macOS + Windows); Linux tray
/// backends (SNI/AppIndicator) don't support tooltips, so this is compiled out
/// there. A dead OAuth session takes priority over the routing state; the
/// attention flag is read from `SESSION_NEEDS_SIGNIN` so the routing call sites
/// don't have to thread it through. The macOS status dot is handled in
/// `update_tray_status`.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn update_tray_tooltip(app: &tauri::AppHandle, proxy_on: bool) {
    if let Some(tray) = app.tray_by_id("main") {
        let text = if SESSION_NEEDS_SIGNIN.load(Ordering::Relaxed) {
            "Gate Connect · sign in required"
        } else if proxy_on {
            "Gate Connect · routing on"
        } else {
            "Gate Connect · routing off"
        };
        let _ = tray.set_tooltip(Some(text.to_string()));
    }
}

#[cfg(not(target_os = "linux"))]
fn anchor_under_tray(window: &tauri::WebviewWindow, tray_pos: Position, tray_size: Size) {
    let scale = window.scale_factor().unwrap_or(1.0);
    let pos = tray_pos.to_physical::<f64>(scale);
    let size = tray_size.to_physical::<f64>(scale);

    let window_w_px = window
        .outer_size()
        .map(|s| s.width as f64)
        .unwrap_or(380.0 * scale);
    let window_h_px = window
        .outer_size()
        .map(|s| s.height as f64)
        .unwrap_or(720.0 * scale);

    let tray_center_x = pos.x + size.width / 2.0;
    let tray_top_y = pos.y;
    let tray_bottom_y = pos.y + size.height;
    let gap = 6.0 * scale;

    // Fall back to the unbounded below-icon placement if we can't read
    // the monitor - better than refusing to show the window.
    let (mon_x, mon_y, mon_w, mon_h) = match monitor_at(window, tray_center_x, tray_top_y) {
        Some(m) => {
            let p = m.position();
            let s = m.size();
            (p.x as f64, p.y as f64, s.width as f64, s.height as f64)
        }
        None => {
            let x = (tray_center_x - window_w_px / 2.0).round() as i32;
            let y = (tray_bottom_y + gap).round() as i32;
            let _ = window.set_position(PhysicalPosition::new(x, y));
            return;
        }
    };

    let space_below = (mon_y + mon_h) - tray_bottom_y;
    let y = if space_below >= window_h_px + gap {
        tray_bottom_y + gap
    } else {
        tray_top_y - window_h_px - gap
    };

    let x = (tray_center_x - window_w_px / 2.0)
        .max(mon_x + 4.0)
        .min(mon_x + mon_w - window_w_px - 4.0);

    let _ = window.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
}

/// Position the popover above or below the cursor on Linux, where
/// the tray protocol (SNI/AppIndicator) doesn't expose a usable icon
/// rect - and on GNOME, where the left-click event often never fires
/// at all. The cursor is the only positioning hint we can trust. On
/// Wayland the compositor may ignore `set_position` outright; on X11
/// this lands the popover near where the user clicked.
#[cfg(target_os = "linux")]
fn anchor_at_cursor(window: &tauri::WebviewWindow, cursor: PhysicalPosition<f64>) {
    let scale = window.scale_factor().unwrap_or(1.0);

    let window_w_px = window
        .outer_size()
        .map(|s| s.width as f64)
        .unwrap_or(380.0 * scale);
    let window_h_px = window
        .outer_size()
        .map(|s| s.height as f64)
        .unwrap_or(720.0 * scale);

    let gap = 6.0 * scale;

    let (mon_x, mon_y, mon_w, mon_h) = match window.current_monitor().ok().flatten() {
        Some(m) => {
            let p = m.position();
            let s = m.size();
            (p.x as f64, p.y as f64, s.width as f64, s.height as f64)
        }
        None => {
            let x = (cursor.x - window_w_px / 2.0).round() as i32;
            let y = (cursor.y + gap).round() as i32;
            let _ = window.set_position(PhysicalPosition::new(x, y));
            return;
        }
    };

    let space_below = (mon_y + mon_h) - cursor.y;
    let y = if space_below >= window_h_px + gap {
        cursor.y + gap
    } else {
        cursor.y - window_h_px - gap
    };

    let x = (cursor.x - window_w_px / 2.0)
        .max(mon_x + 4.0)
        .min(mon_x + mon_w - window_w_px - 4.0);

    let _ = window.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
}

/// Let the popover render over the active Space, including a full-screen app's.
#[cfg(target_os = "macos")]
fn apply_popover_space_behavior(window: &tauri::WebviewWindow) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;

    const CAN_JOIN_ALL_SPACES: u64 = 1 << 0;
    const FULL_SCREEN_AUXILIARY: u64 = 1 << 8;
    const NS_STATUS_WINDOW_LEVEL: i64 = 25;

    let Ok(ns_window_ptr) = window.ns_window() else {
        return;
    };
    if ns_window_ptr.is_null() {
        return;
    }

    unsafe {
        let ns_window: *mut AnyObject = ns_window_ptr.cast();
        let behavior: u64 = CAN_JOIN_ALL_SPACES | FULL_SCREEN_AUXILIARY;
        let () = msg_send![ns_window, setCollectionBehavior: behavior];
        let () = msg_send![ns_window, setLevel: NS_STATUS_WINDOW_LEVEL];
    }
}

/// NSPanel subclass whose canBecomeKey/MainWindow return YES - a borderless
/// window can't become key otherwise, which blocks the cursor and keyboard.
#[cfg(target_os = "macos")]
fn key_panel_class() -> &'static objc2::runtime::AnyClass {
    use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
    use objc2::{class, sel};
    use std::ffi::CStr;

    const NAME: &CStr = c"GateConnectKeyPanel";

    if let Some(cls) = AnyClass::get(NAME) {
        return cls;
    }

    // Raw-pointer receiver keeps the fn type non-higher-ranked for add_method.
    extern "C" fn yes(_this: *mut AnyObject, _sel: Sel) -> Bool {
        Bool::YES
    }

    let mut builder = ClassBuilder::new(NAME, class!(NSPanel))
        .expect("GateConnectKeyPanel: class name should be unique in-process");
    unsafe {
        builder.add_method(
            sel!(canBecomeKeyWindow),
            yes as extern "C" fn(*mut AnyObject, Sel) -> Bool,
        );
        builder.add_method(
            sel!(canBecomeMainWindow),
            yes as extern "C" fn(*mut AnyObject, Sel) -> Bool,
        );
    }
    builder.register()
}

/// Make the popover a non-activating NSPanel: it can take key focus over a
/// full-screen app without activating us, which would leave that Space.
#[cfg(target_os = "macos")]
fn promote_to_nonactivating_panel(window: &tauri::WebviewWindow) {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    const NONACTIVATING_PANEL: u64 = 1 << 7;

    let Ok(ns_window_ptr) = window.ns_window() else {
        return;
    };
    if ns_window_ptr.is_null() {
        return;
    }

    unsafe {
        let ns_window: *mut AnyObject = ns_window_ptr.cast();
        let panel_cls: &AnyClass = key_panel_class();
        objc2::ffi::object_setClass(ns_window, panel_cls);
        let mask: u64 = msg_send![ns_window, styleMask];
        let () = msg_send![ns_window, setStyleMask: mask | NONACTIVATING_PANEL];
        let () = msg_send![ns_window, setBecomesKeyOnlyIfNeeded: false];
        let () = msg_send![ns_window, setHidesOnDeactivate: false];
    }
}

/// Dismiss the popover on click-outside; a non-activating panel emits no blur
/// event, so we watch for mouse-downs delivered to other apps instead.
#[cfg(target_os = "macos")]
fn install_click_outside_dismiss(app: &tauri::AppHandle) {
    use block2::RcBlock;
    use objc2_app_kit::{NSEvent, NSEventMask};

    let handle = app.clone();
    let block = RcBlock::new(move |_event: core::ptr::NonNull<NSEvent>| {
        if let Some(window) = handle.get_webview_window("main") {
            if window.is_visible().unwrap_or(false) {
                let _ = window.hide();
                POPOVER_VISIBLE.store(false, Ordering::Release);
            }
        }
    });
    let mask = NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown;
    if let Some(token) = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &block) {
        std::mem::forget(token); // dropping the token removes the monitor
    }
}

/// Last menu-bar appearance the tray icon was painted for, so the watcher can
/// tell a real flip from a redundant sample: -1 unknown, 0 light, 1 dark.
#[cfg(target_os = "macos")]
static MENU_BAR_DARK: std::sync::atomic::AtomicI8 = std::sync::atomic::AtomicI8::new(-1);

/// Read the appearance the macOS menu bar is *actually* drawing with.
///
/// The system Light/Dark setting is the wrong question. Since Big Sur the menu
/// bar is translucent and picks its content color from the desktop picture
/// behind it, so a dark wallpaper turns every icon white while the system is
/// still in Light Mode. Template images follow that automatically; ours can't
/// be one, because templating discards color and the routing dot needs to stay
/// green. Hand-tinting from the system theme is what left the mark black among
/// white neighbors.
///
/// AppKit does expose the truth, on the status bar window's
/// `effectiveAppearance`. That window belongs to AppKit rather than to us, so
/// find it by class and only read from it - registering KVO on a window we
/// don't own risks the "deallocated while observers were still registered"
/// crash if AppKit tears it down.
///
/// Returns `None` when the status item isn't on screen yet or the class is
/// renamed out from under us; callers fall back to the system theme.
#[cfg(target_os = "macos")]
fn menu_bar_is_dark() -> Option<bool> {
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2::{class, msg_send};
    use std::ffi::CStr;
    use std::os::raw::c_char;

    // Reaching NSApp off the main thread is undefined behavior, and this is
    // called from command handlers and the startup thread as well as from the
    // main-thread watcher. Off-thread, answer from the watcher's last sample
    // rather than touching AppKit.
    if objc2::MainThreadMarker::new().is_none() {
        return match MENU_BAR_DARK.load(Ordering::Acquire) {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        };
    }

    let status_bar_cls = AnyClass::get(c"NSStatusBarWindow")?;

    // SAFETY: main-thread-only AppKit reads (callers hop via
    // `run_on_main_thread`). Every message is a documented public selector on
    // NSApplication / NSArray / NSWindow / NSAppearance / NSString, each
    // returns an autoreleased or long-lived object we only borrow, and every
    // pointer is null-checked before it is messaged again.
    unsafe {
        let ns_app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        if ns_app.is_null() {
            return None;
        }
        let windows: *mut AnyObject = msg_send![ns_app, windows];
        if windows.is_null() {
            return None;
        }
        let count: usize = msg_send![windows, count];
        for i in 0..count {
            let window: *mut AnyObject = msg_send![windows, objectAtIndex: i];
            if window.is_null() {
                continue;
            }
            let is_status_bar: Bool = msg_send![window, isKindOfClass: status_bar_cls];
            if !is_status_bar.as_bool() {
                continue;
            }
            let appearance: *mut AnyObject = msg_send![window, effectiveAppearance];
            if appearance.is_null() {
                return None;
            }
            let name: *mut AnyObject = msg_send![appearance, name];
            if name.is_null() {
                return None;
            }
            let utf8: *const c_char = msg_send![name, UTF8String];
            if utf8.is_null() {
                return None;
            }
            // Every dark NSAppearance name carries "Dark" - DarkAqua,
            // VibrantDark, AccessibilityHighContrastDarkAqua - so a substring
            // test covers the set without enumerating it.
            return Some(CStr::from_ptr(utf8).to_string_lossy().contains("Dark"));
        }
    }
    None
}

/// Keep the tray mark legible when the menu bar flips appearance under it.
///
/// Three things flip it: the system Light/Dark switch, a new desktop picture,
/// and moving to a Space that has a different one. Only the first reaches us,
/// as `WindowEvent::ThemeChanged`, which is why the icon could sit wrong until
/// the next routing toggle. `effectiveAppearance` is observable in principle,
/// but not safely on a window we don't own (see `menu_bar_is_dark`), so sample
/// it on a slow timer instead and repaint only on an actual flip. The work is a
/// short walk of `NSApp.windows`, hopped onto the main thread because AppKit
/// demands it.
#[cfg(target_os = "macos")]
fn watch_menu_bar_appearance(app: &tauri::AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || {
        // The status item may have no window yet when setup paints the launch
        // icon, so take one quick sample to correct that guess before settling
        // into the slow cadence.
        let mut delay = std::time::Duration::from_millis(300);
        loop {
            std::thread::sleep(delay);
            delay = std::time::Duration::from_secs(2);
            let inner = handle.clone();
            let _ = handle.run_on_main_thread(move || {
                let Some(dark) = menu_bar_is_dark() else {
                    return;
                };
                if MENU_BAR_DARK.load(Ordering::Acquire) == i8::from(dark) {
                    return;
                }
                // `update_tray_status` re-reads the appearance and stores it,
                // so this stays a no-op until the bar flips again.
                if let Ok(status) = gate_connect_core::proxy::manager().status() {
                    update_tray_status(&inner, status.running);
                }
            });
        }
    });
}

/// Raise and key the popover without activating the app - set_focus() alone
/// won't raise a background app's window.
#[cfg(target_os = "macos")]
fn order_front_regardless(window: &tauri::WebviewWindow) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;

    let Ok(ns_window_ptr) = window.ns_window() else {
        return;
    };
    if ns_window_ptr.is_null() {
        return;
    }

    unsafe {
        let ns_window: *mut AnyObject = ns_window_ptr.cast();
        let () = msg_send![ns_window, orderFrontRegardless];
        let () = msg_send![ns_window, makeKeyWindow];
    }
}

/// Round the corners of the underlying NSWindow on macOS.
///
/// The window is created with `transparent: false`, so it needs no macOS
/// private API. On its own that yields a square, opaque window. To get a
/// rounded shape we use only public AppKit/QuartzCore:
/// 1. make the NSWindow non-opaque with a clear background, so the corner
///    regions outside the radius render transparent rather than painting
///    the default window background;
/// 2. layer-back the content view and mask its CALayer to a corner radius.
///
/// The webview body stays opaque (white from CSS), so the corners read as
/// cleanly rounded against the desktop.
#[cfg(target_os = "macos")]
fn apply_window_corner_radius(window: &tauri::WebviewWindow, radius: f64) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};

    let Ok(ns_window_ptr) = window.ns_window() else {
        return;
    };
    if ns_window_ptr.is_null() {
        return;
    }

    // SAFETY: `ns_window()` hands us a borrowed NSWindow pointer that lives
    // as long as the Tauri window. We only call documented AppKit/QuartzCore
    // selectors on the main thread (Tauri's setup callback runs on main).
    unsafe {
        let ns_window: *mut AnyObject = ns_window_ptr.cast();
        // Drop Tauri's private-API transparency: make the NSWindow
        // non-opaque with a clear background using only public AppKit, so
        // the corner regions outside the rounded mask render transparent
        // instead of painting the default opaque window background.
        let clear_color: *mut AnyObject = msg_send![class!(NSColor), clearColor];
        let () = msg_send![ns_window, setOpaque: false];
        let () = msg_send![ns_window, setBackgroundColor: clear_color];
        let content_view: *mut AnyObject = msg_send![ns_window, contentView];
        if content_view.is_null() {
            return;
        }
        // Layer-back the content view so it actually has a CALayer to mask.
        let () = msg_send![content_view, setWantsLayer: true];
        let layer: *mut AnyObject = msg_send![content_view, layer];
        if layer.is_null() {
            return;
        }
        let () = msg_send![layer, setCornerRadius: radius];
        let () = msg_send![layer, setMasksToBounds: true];
        // Paint the masked layer white so the window's first on-screen frame is
        // a white rounded card - matching the splash background - instead of the
        // transparent/blank flash the clear window background shows before
        // WKWebView paints. The webview's opaque white body draws over this once
        // it renders, so the handoff is seamless. Clipped to the corner mask, so
        // the rounded corners stay transparent.
        let white: *mut AnyObject = msg_send![class!(NSColor), whiteColor];
        let white_cg: *mut AnyObject = msg_send![white, CGColor];
        let () = msg_send![layer, setBackgroundColor: white_cg];
    }
}
