//! Tauri shell. The Rust surface here is small on purpose: every command
//! delegates to `gate-connect-core` so the CLI and the GUI exercise the
//! same code path. Beyond commands, this file sets up a normal 1280x800 window
//! plus a tray icon that toggles it, and a close button that hides rather than
//! quits so the tray always has something to bring back.
//!
//! It used to be a menu-bar popover, and three habits of that had to go. The
//! tray placed the window (under the icon, at the cursor, or under the menu bar
//! by platform), so it now only toggles visibility and placement is the
//! window's own: centred on first launch, untouched after. Focus loss dismissed
//! it, which for a window means vanishing whenever the user clicks another app.
//! And on macOS it was promoted to a non-activating NSPanel with a hand-rolled
//! corner radius, its own space behaviour and a click-outside watcher, none of
//! which a regular window wants. The dock icon is back with them gone.

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
// For anchoring the tray popover under the clicked tray icon; Linux trays
// (SNI/AppIndicator) report no icon rect, so it anchors at the cursor there.
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
    /// The file Gate rewrites for this tool, for the copy that says what is
    /// about to change. `None` where no single file names it.
    config_location: Option<String>,
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
            // The row label, not the product name: this feeds the ledger, whose
            // rows sit under a heading that names the vendor. Every other reader
            // of a tool's name - the CLI, the logs, the quit takeover - wants
            // `display_name`, which is why the two are separate.
            name: integ.row_label().to_string(),
            upstream_provider_name: integ.upstream_provider_name().to_string(),
            default_upstream_url: integ.default_upstream_url().to_string(),
            config_location: integ.config_location(),
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
        // Auto-enable the proxy so the reverse-proxy relay is live: relay-routed
        // tool configs point at the loopback relay, which only exists while the
        // engine runs. Idempotent if the proxy is already on.
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            // Persist the routing intent too, and before the engine comes up
            // (see `routing::enable` for the ordering): connecting a tool is
            // as deliberate a "route through Gate" as the master switch, and
            // this engine start must survive a restart - the tool's config
            // keeps pointing at the relay across a quit, so a launch that
            // doesn't bring the relay back leaves the tool dialing a dead
            // loopback port while its ledger row still reads connected.
            // Deliberately not the full `routing::enable` ceremony: a connect
            // is scoped to one tool, so it must not also restore providers a
            // master-off swept.
            if let Err(e) = gate_connect_core::proxy::intent::set_intent(true) {
                eprintln!("[gate] persisting routing intent on connect failed: {e}");
                report_backend_error("routing_intent", format!("{e:#}"));
            }
            gate_connect_core::proxy::manager()
                .enable()
                .map_err(|e| format!("{e:#}"))?;
            mark_routing_enabled();
        }
        let input = ConnectInput {
            gateway_base_url: account.gateway_base_url,
            upstream_url,
            billing_mode: account.billing_mode,
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

// The upstream-credential trait surface (`has_upstream_credential` /
// `save_upstream_credential` / `clear_upstream_credential`) is CLI-only
// (`gate-connect set-upstream` / `clear-upstream`): no shipped integration
// requires an upstream credential, so the renderer has no commands for it
// and no UI to collect one.

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
    /// Who pays the upstream provider, so a UI can show it once one is
    /// designed. Read-only here; `set_billing_mode` is the setter. Serialized
    /// lowercase (`"byok"` / `"payg"`).
    billing_mode: gate_connect_core::account::BillingMode,
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
    let billing_mode = account::billing_mode().map_err(|e| format!("{e:#}"))?;
    let (org_id, org_name) = match account::selected_org().map_err(|e| format!("{e:#}"))? {
        Some((id, name)) => (Some(id), Some(name)),
        None => (None, None),
    };
    Ok(Some(AccountDto {
        gateway_base_url,
        has_api_key,
        auth_mode,
        billing_mode,
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

/// Is this gateway base URL's scheme acceptable?
///
/// This is the IPC boundary, so the check is deliberately defensive: a
/// compromised renderer must not be able to repoint the app at a plaintext host
/// and harvest the key off the wire. Production rule is therefore `https` only,
/// and `crates/core`'s `account::save` enforces the same rule again underneath.
///
/// Debug builds also accept `http://localhost` and `http://127.0.0.1` so the app
/// can talk to a gateway running on this machine. Host-exact, so
/// `http://localhost.evil.test` is still refused, and `#[cfg(debug_assertions)]`
/// compiles it out of `tauri build` (which is `--release`). Mirrors the guard in
/// `account::is_acceptable_gateway_url`; both must agree or the UI and the core
/// disagree about what is valid.
fn base_url_scheme_ok(parsed: &url::Url) -> bool {
    if parsed.scheme() == "https" {
        return true;
    }
    #[cfg(debug_assertions)]
    if parsed.scheme() == "http" {
        return matches!(parsed.host_str(), Some("localhost") | Some("127.0.0.1"));
    }
    false
}

/// What [`base_url_scheme_ok`] actually enforces, in the words of the build it is
/// compiled into.
///
/// Beside the check rather than at the two call sites, which both used to say
/// "must use https" unconditionally. In a debug build that sends a developer who
/// typoed `http://localhos:3000`, or pointed at a LAN address, off to change the
/// scheme - which was already right. Mirrors the same fix in `account.rs`.
fn base_url_scheme_error() -> String {
    #[cfg(debug_assertions)]
    return "base url must use https, or http on localhost or 127.0.0.1 exactly".into();
    #[cfg(not(debug_assertions))]
    return "base url must use https".into();
}

#[tauri::command]
async fn save_account(base_url: String, api_key: Option<String>) -> Result<(), String> {
    if base_url.len() > 2048 {
        return Err("base url is unexpectedly long (>2048 bytes)".into());
    }
    let parsed = url::Url::parse(&base_url).map_err(|e| format!("invalid base url: {e}"))?;
    if !base_url_scheme_ok(&parsed) {
        return Err(base_url_scheme_error());
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
    let done = tauri::async_runtime::spawn_blocking(move || {
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
    .map_err(|e| format!("save join error: {e}"))?;
    // What changed: the key, the gateway, or both.
    if done.is_ok() {
        signal_session_changed();
    }
    done
}

#[tauri::command]
async fn clear_account() -> Result<(), String> {
    // Off the main thread: per-tool config I/O that shouldn't freeze the UI.
    let done = tauri::async_runtime::spawn_blocking(|| {
        // Disconnect managed tools first: clearing the account while their
        // configs still embed the key would leave them routing to the gateway
        // with a dead credential on disk. A failure aborts the sign-out.
        registry::disconnect_all_managed().map_err(|e| format!("{e:#}"))?;
        account::clear().map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| format!("sign-out join error: {e}"))?;
    // What changed: the account is gone.
    if done.is_ok() {
        signal_session_changed();
    }
    done
}

/// Dev-mode gateway switch: repoint the account at another environment and
/// forget the current Gate key, so the UI can prompt for an
/// environment-appropriate one. Managed tools are disconnected first - their
/// config embeds the old gateway+key, and a later key rotation would push the
/// new key into configs still pointing at the old gateway. The proxy engine is
/// stopped for the same reason: it pins the gateway URL at start (only the key,
/// token, org, and domains update live), so leaving it up would keep traffic
/// rewritten to the old environment while the new environment's token gets
/// pushed in - a 401 on every proxied call, with the org list, which goes
/// direct, still working. Mirrors the URL validation in `save_account` and the
/// disconnect-first order in `clear_account`.
#[tauri::command]
async fn switch_gateway(base_url: String) -> Result<(), String> {
    if base_url.len() > 2048 {
        return Err("base url is unexpectedly long (>2048 bytes)".into());
    }
    let parsed = url::Url::parse(&base_url).map_err(|e| format!("invalid base url: {e}"))?;
    if !base_url_scheme_ok(&parsed) {
        return Err(base_url_scheme_error());
    }
    if parsed.host_str().is_none() {
        return Err("base url is missing a host".into());
    }
    // Off the main thread: per-tool config I/O plus keychain delete.
    let done = tauri::async_runtime::spawn_blocking(move || {
        registry::disconnect_all_managed().map_err(|e| format!("{e:#}"))?;
        // Before the account moves, so the engine can never be up against an
        // account it wasn't started from. A failure aborts the switch: routing
        // to the old gateway with the new environment's credential is the state
        // this whole command exists to avoid.
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        gate_connect_core::proxy::manager()
            .shutdown_engine()
            .map_err(|e| format!("{e:#}"))?;
        account::switch_gateway(&base_url).map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| format!("switch join error: {e}"))?;
    // What changed: another gateway, and the key with it.
    if done.is_ok() {
        signal_session_changed();
    }
    done
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
    let done = tauri::async_runtime::spawn_blocking(|| {
        gate_connect_core::oauth::clear().map_err(|e| format!("{e:#}"))?;
        // The held activity readings belong to the org just signed out of, and
        // signing out is not a disconnect: `account.json` keeps the gateway and
        // the org, so `activity_cache`'s scope stays byte-identical and every
        // reading in it would still match and still be served. `account::clear`
        // covers the disconnect path; this covers the one that leaves the account
        // file in place. Best effort and after the credential, on the same
        // reasoning as there: a cache that will not delete must not be the reason
        // a sign-out reports failure.
        let _ = gate_connect_core::activity_cache::clear();
        // Revert a running engine to the legacy header immediately (empty
        // token == fall back to the API key, if one is present).
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        gate_connect_core::proxy::manager().refresh_token("");
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("oauth sign-out join error: {e}"))?;
    // What changed: the OAuth session is gone, and `account.json` is not.
    if done.is_ok() {
        signal_session_changed();
    }
    done
}

/// Explicitly set the auth mode. Used when a user chooses the legacy pasted-key
/// path from the sign-in screen; OAuth sign-in sets it implicitly.
#[tauri::command]
async fn set_auth_mode(oauth: bool) -> Result<(), String> {
    let done = tauri::async_runtime::spawn_blocking(move || {
        let mode = if oauth {
            gate_connect_core::account::AuthMode::OAuth
        } else {
            gate_connect_core::account::AuthMode::ApiKey
        };
        gate_connect_core::account::set_auth_mode(mode).map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| format!("set auth mode join error: {e}"))?;
    // What changed: which credential the gateway is given.
    if done.is_ok() {
        signal_session_changed();
    }
    done
}

/// Switch who pays the upstream provider.
///
/// The relay and the MITM engine read the mode per request, so `refresh_mode`
/// is all that in-flight routing needs. Codex is the exception: its provider
/// block encodes whether Codex authenticates at all, so a connected Codex is
/// re-applied here. Every other integration writes a base URL and no
/// credential, and needs nothing.
///
/// Re-applying Codex is best-effort: the mode is already persisted by then, and
/// failing the whole call would leave the UI unable to say what happened. A
/// Codex left on the old shape reports `Drifted` on the next status read, which
/// is the path back.
#[tauri::command]
async fn set_billing_mode(payg: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mode = if payg {
            gate_connect_core::account::BillingMode::Payg
        } else {
            gate_connect_core::account::BillingMode::Byok
        };
        gate_connect_core::account::set_billing_mode(mode).map_err(|e| format!("{e:#}"))?;
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        gate_connect_core::proxy::manager().refresh_mode();
        reapply_codex_for_mode(mode);
        Ok(())
    })
    .await
    .map_err(|e| format!("set billing mode join error: {e}"))?
}

/// Rewrite Codex's provider block for `mode`, if Codex is currently routed
/// through Gate. Silent when Codex isn't installed or isn't connected - there
/// is nothing to rewrite, and a mode switch is not the place to start routing a
/// tool the user never connected.
fn reapply_codex_for_mode(mode: gate_connect_core::account::BillingMode) {
    let Some(integ) = registry::find(ToolId::Codex) else {
        return;
    };
    if !matches!(integ.status(), Ok(gate_connect_core::Status::Connected)) {
        return;
    }
    let Ok(Some(account)) = account::load() else {
        return;
    };
    let input = ConnectInput {
        gateway_base_url: account.gateway_base_url,
        upstream_url: integ.default_upstream_url().to_string(),
        billing_mode: mode,
        relay_base_url: gate_connect_core::proxy::relay_base_url(),
        engine_proxy_url: gate_connect_core::proxy::engine_proxy_url(),
    };
    if let Err(e) = integ.connect(&input) {
        eprintln!("re-applying Codex for the new billing mode failed: {e:#}");
    }
}

/// Fetch the 24-hour activity overview for the Overview pane (AG-572).
///
/// `install_id` scopes the reading to one installation (AC 1); omitted, it is
/// org-wide, which stays the default because attribution only starts with the
/// gateway migration that added it - scoping by default would hide every
/// earlier request from a total the user could already see.
///
/// The payload stays raw JSON while the gateway contract moves; `lib/activity.ts`
/// is the only place that models it.
///
/// Failures cross as a JSON envelope, `{"code":…,"message":…}`, not as prose.
/// AG-576 requires the pane to name the cause and offer a matching action, and
/// the front end cannot pick between Retry and Sign in by reading an English
/// sentence. See `gate_connect_core::activity::FailureCode`.
#[tauri::command]
async fn activity_overview(
    install_id: Option<String>,
    tool: Option<String>,
) -> Result<String, String> {
    let tool = parse_tool(tool)?;
    tauri::async_runtime::spawn_blocking(move || {
        gate_connect_core::activity::overview_json(install_id.as_deref(), tool).map_err(envelope)
    })
    .await
    .map_err(|e| format!("activity overview join error: {e}"))?
}

/// Read a tool slug from the front end, or `None` for every tool.
///
/// An unrecognised non-empty slug is an error rather than a silent fall back to
/// org-wide. The two sides of this boundary share one registry, so a slug that
/// does not parse means they disagree about it - a bug worth surfacing, not a
/// request to widen the scope. Falling back would quietly relabel every tool's
/// traffic as the one the user selected.
fn parse_tool(tool: Option<String>) -> Result<Option<gate_connect_core::registry::ToolId>, String> {
    match tool.as_deref().filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(slug) => gate_connect_core::registry::ToolId::from_slug(slug)
            .map(Some)
            .ok_or_else(|| format!("unknown tool slug {slug:?}")),
    }
}

/// The last overview that landed for this scope, or `None`.
///
/// A file read, not a network call, so the pane can paint real numbers on the
/// frame it opens on instead of a skeleton that resolves a round trip later
/// (AG-576). Never an error: no cache and an unreadable cache mean the same
/// thing to the caller, which is that it waits for [`activity_overview`].
#[tauri::command]
async fn activity_cached_overview(
    install_id: Option<String>,
    tool: Option<String>,
) -> Result<Option<String>, String> {
    let tool = parse_tool(tool)?;
    tauri::async_runtime::spawn_blocking(move || {
        gate_connect_core::activity::cached_overview_json(install_id.as_deref(), tool)
    })
    .await
    .map_err(|e| format!("cached activity join error: {e}"))
}

/// Every held per-tool reading for this installation scope, keyed by slug.
///
/// The tray's quick status draws a figure on every app row, and `/v1/me/activity`
/// answers for one tool at a time - so the popover opens on what is already on
/// disk and refreshes only what has gone stale. One file read rather than one per
/// row, because the file holds them all.
///
/// Never an error, for the same reason [`activity_cached_overview`] is not: an
/// empty map covers no cache, an unreadable one, and a scope that holds nothing,
/// and all three mean the caller waits for the network.
#[tauri::command]
async fn activity_cached_tool_overviews(
    install_id: Option<String>,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        gate_connect_core::activity::cached_tool_overviews_json(install_id.as_deref())
    })
    .await
    .map_err(|e| format!("cached tool activity join error: {e}"))
}

/// One page of a tool's recent requests, for the app pane's feed (AG-574).
///
/// `tool` is required here, unlike on the overview: the feed is always about one
/// tool, and the gateway refuses a request that names none. Not cached - see
/// `activity::tool_events_json` for why the held reading stays with the overview.
#[tauri::command]
async fn activity_tool_events(
    install_id: Option<String>,
    tool: String,
    cursor: Option<String>,
) -> Result<String, String> {
    let Some(tool) = parse_tool(Some(tool))? else {
        return Err("a tool slug is required to read a tool's events".into());
    };
    tauri::async_runtime::spawn_blocking(move || {
        gate_connect_core::activity::tool_events_json(
            install_id.as_deref(),
            tool,
            cursor.as_deref(),
        )
        .map_err(envelope)
    })
    .await
    .map_err(|e| format!("activity tool events join error: {e}"))?
}

/// List the installations this account has sent traffic from, for the Overview's
/// installation picker. Same envelope and the same failure taxonomy as
/// [`activity_overview`].
#[tauri::command]
async fn activity_installations() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(gate_connect_core::activity::installations_json)
        .await
        .map_err(|e| format!("activity installations join error: {e}"))?
        .map_err(envelope)
}

/// This install's per-tool model choices (AG-588).
///
/// A local file read, not a network call: the choice lives in
/// `preferences.json` beside the other user choices. It was briefly a gateway
/// endpoint scoped to the organization; keeping it local means the machine whose
/// traffic it governs is the machine that holds it, and one developer's click no
/// longer changes what a colleague's requests are answered with.
///
/// Returns the whole map plus the acknowledgement stamp, so the pane can decide
/// whether the next switch to a Gate model needs its confirmation before any
/// choice exists.
#[tauri::command]
async fn tool_model_preferences() -> Result<ToolModelsDto, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let prefs = gate_connect_core::preferences::load();
        ToolModelsDto {
            tools: prefs
                .tool_models
                .into_iter()
                .map(|(slug, choice)| (slug, ToolModelChoiceDto::from(choice)))
                .collect(),
            paid_ack_unix: prefs.gate_model_paid_ack_unix,
        }
    })
    .await
    .map_err(|e| format!("tool model preferences join error: {e}"))
}

#[derive(Serialize)]
struct ToolModelsDto {
    /// Keyed by tool slug. A tool with no entry is on its own default, which is
    /// why an absent key is not the same as an error and needs no placeholder.
    tools: std::collections::BTreeMap<String, ToolModelChoiceDto>,
    /// Unix seconds, or null when this install has never accepted paid use.
    paid_ack_unix: Option<i64>,
}

#[derive(Serialize)]
struct ToolModelChoiceDto {
    /// `"tool"` or `"gate"`. Only this decides what would be served.
    source: String,
    /// Chosen models, which may be non-empty while `source` is `"tool"` - that is
    /// a remembered choice, not an active one.
    model_ids: Vec<String>,
}

impl From<gate_connect_core::preferences::ToolModelChoice> for ToolModelChoiceDto {
    fn from(c: gate_connect_core::preferences::ToolModelChoice) -> Self {
        use gate_connect_core::preferences::ModelSource;
        Self {
            source: match c.source {
                ModelSource::Tool => "tool".into(),
                ModelSource::Gate => "gate".into(),
            },
            model_ids: c.model_ids,
        }
    }
}

/// Choose the model one tool runs on.
///
/// `source` is `"tool"` (the tool picks) or `"gate"` (Gate serves `model_ids`).
/// An unrecognised value is an error rather than a default, for the reason
/// [`parse_tool`] gives: a silent fall back would store the opposite of what the
/// user clicked.
///
/// `acknowledge_paid_use` records that the person accepted billing, and is
/// honoured only when moving to `"gate"` - remembering a model under the tool's
/// own default spends nothing and must not record consent to spend.
#[tauri::command]
async fn set_tool_model(
    tool: String,
    source: String,
    model_ids: Vec<String>,
    acknowledge_paid_use: bool,
) -> Result<(), String> {
    // Parsed, not trusted: the slug has to be one this app actually configures,
    // or the pane would store a choice under a key nothing reads.
    let Some(tool) = parse_tool(Some(tool))? else {
        return Err("a tool slug is required to set a model preference".into());
    };
    let source = match source.as_str() {
        "tool" => gate_connect_core::preferences::ModelSource::Tool,
        "gate" => gate_connect_core::preferences::ModelSource::Gate,
        other => return Err(format!("unknown model source {other:?}")),
    };
    tauri::async_runtime::spawn_blocking(move || {
        gate_connect_core::preferences::set_tool_model(
            tool.slug(),
            source,
            model_ids,
            acknowledge_paid_use,
        )
        .map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| format!("set tool model join error: {e}"))?
}

/// The models this gateway offers, for the picker.
///
/// An empty catalogue is a successful answer, not a failure: it is built from
/// platform provider accounts, and a deployment with none has nothing to offer.
/// The picker says so in words rather than drawing an empty list.
#[tauri::command]
async fn gate_model_catalogue() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(gate_connect_core::gate_models::catalogue_json)
        .await
        .map_err(|e| format!("gate model catalogue join error: {e}"))?
        .map_err(envelope)
}

/// This organization's Gate credit balance and plan (AG-588/590/592).
///
/// Its own read rather than part of the catalogue: the balance is small and
/// changes as requests are served, the catalogue is large and does not. Same
/// envelope and failure taxonomy as [`activity_overview`].
#[tauri::command]
async fn gate_credits() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(gate_connect_core::gate_models::credits_json)
        .await
        .map_err(|e| format!("gate credits join error: {e}"))?
        .map_err(envelope)
}

/// Write one line to the diagnostic log from the front end.
///
/// The front end is where this app is least observable: its errors go to a
/// webview console that nobody can read after the fact, which is how a routing
/// switch that stopped responding stayed undiagnosable. This is the door to the
/// same file the Rust side writes.
///
/// Off in production - `logging::enabled` decides, and a call made when it is off
/// does nothing rather than failing. Callers must not pass credentials, prompts
/// or request bodies; see that module's note on what may be written.
#[tauri::command]
fn log_message(level: String, message: String) {
    gate_connect_core::logging::log(
        gate_connect_core::logging::Level::from_wire(&level),
        &message,
    );
}

/// Where the diagnostic log lives, or `None` when logging is off.
///
/// Lets Settings and the diagnostics report name the file to send instead of
/// asking someone to find it, and returns nothing in a production build so the
/// UI cannot offer a path to a file that is never written.
#[tauri::command]
fn log_file_path() -> Option<String> {
    gate_connect_core::logging::path_for_report().map(|p| p.to_string_lossy().into_owned())
}

/// Serialize an activity failure for the IPC boundary.
fn envelope(f: gate_connect_core::activity::Failure) -> String {
    serde_json::to_string(&f).unwrap_or_else(|_| {
        // Serializing two owned strings and a unit enum cannot fail, but the
        // fallback still has to produce *valid* JSON: `f.message` can carry an
        // upstream error body, quotes and backslashes included, and interpolating
        // it raw made a string that `toFailure` cannot parse - which discards the
        // code and reports every failure as generic, exactly what this envelope
        // exists to prevent. Serialize the message on its own so the escaping is
        // the library's problem, and only hand-write the part that is a constant.
        let message = serde_json::to_string(&f.message).unwrap_or_else(|_| "\"\"".into());
        format!(r#"{{"code":"unknown","message":{message}}}"#)
    })
}

/// List the orgs the signed-in user may act on (for the org picker). Reads the
/// current gateway + live OAuth session and calls the gateway's `/v1/me/orgs`.
///
/// `list_current` renews a refused token and retries before giving up, and
/// records the session as rejected when even the renewed one is refused. The
/// screen that made this call shows the error either way, but a rejection is
/// news for the whole app - so raise the same signal the refresh loop would
/// have raised on its next tick, rather than leaving the tray green for up to
/// 30s behind a popover that has already given up.
///
/// Gated on `session_rejected()`, the recorded gateway verdict, and not on a
/// `live_session()` of `None`: an offline user with a locally-expired token
/// reads as `None` too, and answering that with a red tray and an expired-
/// session notification would be the app crying wolf about a dropped Wi-Fi.
///
/// All of it inside the blocking closure. The signal does blocking
/// engine-status IPC, and this is an `async fn` command, so its body is on a
/// tokio worker rather than the blocking pool.
#[tauri::command]
async fn oauth_list_orgs() -> Result<Vec<gate_connect_core::org::Org>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let listed = gate_connect_core::org::list_current().map_err(|e| format!("{e:#}"));
        if listed.is_err() && gate_connect_core::oauth::session_rejected() {
            if let Some(app) = APP_HANDLE.get() {
                signal_session_dead(app);
            }
        }
        listed
    })
    .await
    .map_err(|e| format!("list orgs join error: {e}"))?
}

/// Persist the selected org and push it into a running engine/relay so
/// `X-Gate-Org-Id` takes effect live (no restart).
#[tauri::command]
async fn set_org(org_id: String, org_name: String) -> Result<(), String> {
    let done = tauri::async_runtime::spawn_blocking(move || {
        gate_connect_core::account::set_org(&org_id, &org_name).map_err(|e| format!("{e:#}"))?;
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        gate_connect_core::proxy::manager().refresh_org(&org_id);
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("set org join error: {e}"))?;
    // What changed: another org, so every figure on screen belongs to the previous one.
    if done.is_ok() {
        signal_session_changed();
    }
    done
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
///
/// `(async)` so the body runs on the blocking pool: a plain sync command runs
/// inline on the main thread, which on Linux is the GTK loop that also drives
/// the webview's IPC - every probe here would freeze the popover for its own
/// duration.
#[tauri::command(async)]
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

// No `provider_enable` / `provider_disable` commands: the popover drives
// per-tool routing through `connect_tool` / `disconnect_tool` and the master
// switch through `proxy_enable` / `proxy_disable`; the provider layer is
// CLI/core-only (`provider::enable` / `disable`), so the renderer gets no
// handle on it.

// ---- Built-in MITM proxy (macOS + Windows + Linux) ----
//
// These delegate to the process-global `proxy::manager()`. They're gated to
// the platforms where CA trust + system-proxy wiring is implemented (macOS via
// `security`/`networksetup`, Windows via `certutil`/WinINET, Linux via the
// system trust store + `/etc/environment`) and registered in each platform's
// handler block below. Each build refreshes the tray status dot from
// enable/disable so the routing state shows in the menu bar / taskbar.

/// Async so the body lands on the blocking pool rather than the main thread. On
/// Windows `status()` shells out to `certutil` for the CA-trust reading, and a
/// sync command runs on the thread driving the event loop - so a `certutil` that
/// hangs (a crash on the host keeps the process alive while Windows Error
/// Reporting collects its dump) froze the window and left the app unquittable.
/// `ca_windows::certutil_bounded` caps that wait; this keeps even the capped
/// wait off the UI thread.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
async fn proxy_status() -> Result<gate_connect_core::proxy::ProxyState, String> {
    tauri::async_runtime::spawn_blocking(|| {
        gate_connect_core::proxy::manager()
            .status()
            .map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| format!("proxy status join error: {e}"))?
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
async fn proxy_enable(
    app: tauri::AppHandle,
) -> Result<gate_connect_core::proxy::ProxyState, String> {
    // Off the main thread: enable can block on the CA-trust admin prompt
    // and waits up to 10s for engine readiness.
    let state = tauri::async_runtime::spawn_blocking(|| {
        // Master ON is one policy shared with the CLI and the startup
        // auto-enable: persist the intent (so the startup auto-enable
        // re-routes after a restart - whether the app relaunches at boot is
        // governed separately by "Launch at login"), restore the provider
        // selection around the engine start, and surface best-effort hiccups
        // without blocking the proxy from coming up.
        let (_, warnings) = gate_connect_core::routing::enable().map_err(|e| format!("{e:#}"))?;
        for w in warnings {
            eprintln!("[gate] proxy enable: {} failed: {:#}", w.component, w.error);
            report_backend_error(w.component, format!("{:#}", w.error));
        }
        mark_routing_enabled();
        // Status re-read rather than enable's own state: the post-enable
        // restore pass can flip domains, and the UI wants the settled set.
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
    // Crash safety net: routing is now on, but with no login item registered a
    // crash would strand the system proxy at a dead port with nothing
    // relaunching at boot to run the startup self-heal. (macOS/Windows only;
    // see the function's doc for why Linux is excluded.)
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    arm_crash_safety_net(&app);
    Ok(state)
}

/// Crash safety net for a session with routing on but no login item: register
/// launch-at-login and arm the pending-disable marker (the deferred-opt-out
/// mechanism, see autostart_optout's module docs), so every existing safe
/// point deregisters the item and the Settings toggle keeps reporting the
/// user's choice. Skipped when the user opted in themselves - arming would
/// make the status lie and a later safe point would remove a registration
/// they want. Marker before registration: a crash between the two steps must
/// not leave a registration that reads as the user's choice. Best-effort:
/// routing is already on when this runs, so failures only lose the net.
///
/// macOS/Windows only. On Linux the engine lives in a detached helper daemon
/// that owns the port and falls back to pass-through when the GUI dies, so a
/// crash cannot strand the system proxy at a dead port - the net has nothing
/// to heal. And Linux has no exit-time safe point (the RunEvent::Exit
/// handler is macOS/Windows-only), so an armed marker would survive every
/// clean quit and turn each boot into a silent teardown launch.
///
/// Accepted trade for launch-at-login decliners: with routing restored on
/// any launch, this arms once per routed session instead of once per manual
/// routing toggle. The registration cadence matches the old behavior - with
/// the intent cleared at exit, a decliner re-toggled routing (and re-armed
/// the net) every session anyway - and a clean quit still deregisters, so
/// their "off" keeps meaning the app does not run at boot.
///
/// Known (accepted) race: this read-then-write pair and the one in
/// `set_launch_at_login` share no lock, so a Settings toggle landing
/// between the `is_enabled()` check and the arm+enable below can end up
/// marked pending (a fresh opt-in reported as off) until the next safe
/// point clears it. The window is milliseconds wide and both sites are
/// driven by one user in one popover; not worth a lock.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn arm_crash_safety_net(app: &tauri::AppHandle) {
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

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
async fn proxy_disable(
    app: tauri::AppHandle,
) -> Result<gate_connect_core::proxy::ProxyState, String> {
    // Off the main thread: disable runs system-proxy subprocesses and joins
    // the engine thread.
    let state = tauri::async_runtime::spawn_blocking(|| {
        // Master OFF is one policy shared with the CLI: sweep + disconnect
        // everything managed before the proxy stops, then clear the routing
        // intent so explicit "off" is sticky across restarts (whether the app
        // relaunches at boot is governed separately by "Launch at login").
        // Best-effort hiccups surface as warnings and never block the kill
        // switch.
        let (state, warnings) =
            gate_connect_core::routing::disable().map_err(|e| format!("{e:#}"))?;
        for w in warnings {
            eprintln!(
                "[gate] proxy disable: {} failed: {:#}",
                w.component, w.error
            );
            report_backend_error(w.component, format!("{:#}", w.error));
        }
        Ok::<_, String>(state)
    })
    .await
    .map_err(|e| format!("proxy disable join error: {e}"))??;
    // Refresh the tray for the new routing state on every platform: retint the
    // mark, recolor the status dot (green on / gray off), and update the
    // tooltip where supported (macOS + Windows).
    update_tray_status(&app, state.running);
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
    // This marker/login-item read-then-write and `arm_crash_safety_net`
    // share no lock; see the accepted-race note on that function.
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
    let state = gate_connect_core::proxy::manager()
        .set_domain(&slug, enabled)
        .map_err(|e| format!("{e:#}"))?;

    // Audited here rather than inside `ProxyManager::set_domain`, because
    // `provider::enable` / `provider::disable` drive that method internally -
    // instrumenting it there would turn one operator action into N+1 events.
    // This command is the operator toggling one domain by hand.
    if let Ok(Some(base_url)) = gate_connect_core::account::load_base_url() {
        gate_connect_core::audit::domain_toggled(&base_url, None, &slug, enabled);
    }
    Ok(state)
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
/// Vestigial. This once suppressed dismiss-on-blur while the keychain dialog or
/// first-run screen held focus, so a focus steal could not make the popover
/// vanish before the user had seen it. Nothing dismisses on blur any more - a
/// window stays put when you click another app - so the flag is written by
/// [`pin_popover`] / [`unpin_popover`] and read nowhere.
///
/// Kept because `App.tsx`, still reachable as the popover fallback, invokes
/// both commands; removing them would make those calls fail. Delete all three
/// together when the popover screens go.
static POPOVER_PINNED: AtomicBool = AtomicBool::new(false);

/// Whether the popover is currently shown. Tracks the hidden→visible edge so
/// the focus hook reconciles once per open, not on every `Focused(true)`: a
/// refocus of an already-visible window (returning from a system dialog, or a
/// pinned-startup blur that never hides) leaves this `true` and is skipped.
/// Set true when the window gains focus; cleared at each real hide/minimize
/// site so the next open reconciles again. Starts false (window not yet shown).
static POPOVER_VISIBLE: AtomicBool = AtomicBool::new(false);

/// Set while a reveal is waiting for the compositor to acknowledge the state
/// change below, so the restore knows there is work to do.
#[cfg(target_os = "linux")]
static DECOR_RESTORE_PENDING: AtomicBool = AtomicBool::new(false);

/// The state the window should end up in: what it was in before the reveal.
#[cfg(target_os = "linux")]
static DECOR_WANT_MAXIMIZED: AtomicBool = AtomicBool::new(false);

/// The window's own size, captured before the state change so the restore can
/// put it back without trusting GTK to have remembered it. Physical pixels;
/// zero means nothing usable was captured. Only written while the window is
/// un-maximised, since a maximised `inner_size` is the screen.
#[cfg(target_os = "linux")]
static DECOR_SAVED_W: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
#[cfg(target_os = "linux")]
static DECOR_SAVED_H: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Give the window's first map a compositor configure, so its native title-bar
/// buttons work, then put it back in the state the user left it in.
///
/// **A window has to be born with negotiated geometry.** Anything whose first
/// map comes from `show()` on a hidden window comes up with dead decoration
/// input regions - close, minimise and maximise all swallow their clicks, while
/// a double-click on the bar still works because the WM handles that at frame
/// level. Four cases established it: `visible: false` plus `show()` is broken;
/// `visible: true` on a normal launch works; `visible: true` hidden before the
/// map and revealed from the tray is broken again; and the onboarding window,
/// built visible at runtime, needs none of this. The config flag was never the
/// variable - whether the first map carries a compositor configure is.
///
/// A maximise/un-maximise is how that configure gets forced: the compositor owns
/// the bounds of a maximised window, so a change either into or out of that
/// state must be configured. **Which direction depends on where the window
/// starts**, and that is the part this got wrong twice. Maximising an
/// already-maximised window is a no-op, so a window closed while maximised was
/// skipped entirely and reopened with dead buttons. So: apply the *opposite* of
/// the wanted state before the show, and the wanted state after it.
///
/// Mechanisms that do **not** work, recorded so they are not retried: a 1px
/// `set_size` (a client-side request GTK satisfies with no round trip, and a
/// delta small enough to coalesce away), and toggling `set_decorations`
/// (rebuilds the title-bar widgets, renegotiates no geometry).
///
/// Runs on every reveal of a hidden window, not once per process: every map of a
/// hidden window is broken, so a one-shot let the bug back on the second open.
///
/// The principled repair is still to build this window at runtime the way
/// `open_onboarding_window` does, so it is never shown-after-hidden and none of
/// this exists. That is larger because a `--silent` session keeps the hidden
/// webview alive on purpose - detection polling, the `quit-requested` listener
/// and the backend-error drain all live in it - so it cannot simply be created
/// on demand.
#[cfg(target_os = "linux")]
fn map_maximized_for_decorations(window: &tauri::WebviewWindow) {
    // A reveal of a window that is already up needs nothing, and toggling its
    // state would be destructive.
    if window.is_visible().unwrap_or(false) {
        return;
    }
    let want_maximized = window.is_maximized().unwrap_or(false);
    // Only meaningful un-maximised: maximised, `inner_size` is the screen.
    if !want_maximized {
        if let Ok(size) = window.inner_size() {
            if size.width > 0 && size.height > 0 {
                DECOR_SAVED_W.store(size.width, Ordering::Release);
                DECOR_SAVED_H.store(size.height, Ordering::Release);
            }
        }
    }
    DECOR_WANT_MAXIMIZED.store(want_maximized, Ordering::Release);
    DECOR_RESTORE_PENDING.store(true, Ordering::Release);
    // The opposite state, so the map itself has to be configured.
    let asked = if want_maximized {
        window.unmaximize()
    } else {
        window.maximize()
    };
    if asked.is_err() {
        DECOR_RESTORE_PENDING.store(false, Ordering::Release);
        return;
    }
    poll_restore_after_repair(&window.app_handle().clone());
}

/// Put the window back into the state [`map_maximized_for_decorations`] recorded,
/// once the opposite state is observably in effect.
///
/// **Gated on `is_maximized()`, not on which event arrived.** Three attempts
/// failed by trusting an event to mean "the configure landed": a queued
/// event-loop turn, then `Resized`, then `Resized`-or-`Focused(true)`. The last
/// lost because `Focused(true)` fires during `show()`, *before* the configure
/// comes back, so the flag was consumed early and the restore raced as before.
/// Window state cannot be fooled that way: until the pre-state is really in
/// effect the request stays armed and a later event tries again.
///
/// Backed by a short poll too, because if no further event arrives after the
/// configure there is nothing left to re-trigger this.
///
/// Queued rather than called inline so GTK is not re-entered mid dispatch.
/// Takes `&Window`, not `&WebviewWindow`: that is what `on_window_event` hands
/// out, and the state calls live on both.
#[cfg(target_os = "linux")]
fn restore_after_repair(window: &tauri::Window) {
    if !DECOR_RESTORE_PENDING.load(Ordering::Acquire) {
        return;
    }
    let want_maximized = DECOR_WANT_MAXIMIZED.load(Ordering::Acquire);
    // Wait until the opposite state is actually in effect.
    if window.is_maximized().unwrap_or(false) == want_maximized {
        return;
    }
    if !DECOR_RESTORE_PENDING.swap(false, Ordering::AcqRel) {
        return;
    }
    let w = window.clone();
    let _ = window.app_handle().clone().run_on_main_thread(move || {
        if want_maximized {
            let _ = w.maximize();
            return;
        }
        let _ = w.unmaximize();
        // Then set the size ourselves, on the next turn so the un-maximise has
        // been applied first. GTK's own restore geometry is not trustworthy: by
        // the second reveal it has been overwritten with the maximised bounds,
        // so `unmaximize` alone reopened the window at screen size.
        let width = DECOR_SAVED_W.load(Ordering::Acquire);
        let height = DECOR_SAVED_H.load(Ordering::Acquire);
        if width == 0 || height == 0 {
            return;
        }
        let w2 = w.clone();
        let _ = w.app_handle().clone().run_on_main_thread(move || {
            let _ = w2.set_size(tauri::PhysicalSize::new(width, height));
            let _ = w2.center();
        });
    });
}

/// The main window's floor, in logical pixels.
///
/// Duplicated from `tauri.conf.json`'s `minWidth`/`minHeight` on purpose: the
/// config is what macOS, Windows and X11 enforce, and this is the only thing a
/// Wayland session enforces. The two have to be changed together.
#[cfg(target_os = "linux")]
const MAIN_MIN_SIZE: (f64, f64) = (1024.0, 800.0);

/// Hold the main window at its configured minimum, because on Wayland nothing
/// else will.
///
/// tao asks for a minimum with `gtk_window_set_geometry_hints` and
/// `GDK_HINT_MIN_SIZE` (`tao-0.35.3/src/platform_impl/linux/util.rs:58`), which
/// is the X11 `WM_NORMAL_HINTS` mechanism. GTK never translates those hints into
/// the compositor's `xdg_toplevel.set_min_size`, so under Wayland the config's
/// floor is advisory and a drag goes straight through it - the window shrinks
/// until the 256px rail and the content pane are sharing 300px. X11, macOS and
/// Windows honour the config and never reach this function.
///
/// Converges rather than loops: the clamp only fires *below* the floor, and the
/// resize it asks for is *at* the floor, which does not fire it again. The
/// snap-back is visible mid-drag on Wayland, and that is the whole trade - the
/// compositor owns interactive resize, so the choice is a snap or no floor.
#[cfg(target_os = "linux")]
fn clamp_to_minimum(window: &tauri::Window) {
    // The decoration repair drives this window through a maximise deliberately
    // and restores a size of its own afterwards; clamping mid-repair would be
    // two things fighting over the same geometry.
    if DECOR_RESTORE_PENDING.load(Ordering::Acquire) {
        return;
    }
    // Maximised and fullscreen bounds belong to the compositor, and both are
    // larger than the floor anyway.
    if window.is_maximized().unwrap_or(false) || window.is_fullscreen().unwrap_or(false) {
        return;
    }
    let Ok(size) = window.inner_size() else {
        return;
    };
    let Ok(scale) = window.scale_factor() else {
        return;
    };
    if scale <= 0.0 {
        return;
    }
    let (width, height) = (size.width as f64 / scale, size.height as f64 / scale);
    let (min_width, min_height) = MAIN_MIN_SIZE;
    // A pixel of tolerance, because a logical size that has been through
    // physical pixels at a fractional scale factor comes back a hair under the
    // number it went in as, and clamping on that would resize a window nobody
    // touched.
    if width >= min_width - 1.0 && height >= min_height - 1.0 {
        return;
    }
    let _ = window.set_size(tauri::LogicalSize::new(
        width.max(min_width),
        height.max(min_height),
    ));
}

/// Re-check [`restore_after_repair`] on a timer, for the case where the
/// configure is the last event the window sees.
///
/// Off-thread sleeps, main-thread checks: window APIs are only touched inside
/// `run_on_main_thread`. Bounded, so a window whose state never changes stops
/// being polled rather than being watched forever.
#[cfg(target_os = "linux")]
fn poll_restore_after_repair(app: &tauri::AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || {
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if !DECOR_RESTORE_PENDING.load(Ordering::Acquire) {
                return;
            }
            let inner = handle.clone();
            let _ = handle.run_on_main_thread(move || {
                if let Some(w) = inner.get_webview_window("main") {
                    restore_after_repair(&w.as_ref().window_ref().clone());
                }
            });
        }
    });
}

/// Whether the coming exit is an updater-driven relaunch rather than a user
/// quit. The exit handler completes a pending launch-at-login opt-out on a
/// plain quit, but an update install relaunches us immediately, and the
/// relaunched session would just re-arm the safety net it lost - so the
/// pending marker and login item ride through the relaunch untouched. Set by
/// the frontend after the update download completes, right before it kicks
/// off the install (not around the whole download: a quit while the download
/// is still running is a genuine user exit and must complete the opt-out as
/// usual); reset if the install fails. If the relaunch itself fails after a
/// successful install the flag stays set, which at worst defers the opt-out
/// completion to the next safe point.
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

/// Tell every mounted window that the signed-in session moved.
///
/// The account is the scope of nearly everything on screen: the org name, the
/// activity figures, the security feed's buffer, the installation list. Each
/// shell keys its readings on a credential string built from the account, and
/// drops them when it changes - but only the window that *made* the change knew,
/// because none of the mutating commands emitted anything. So the tray kept the
/// previous org's counts and header until something unrelated woke it, and a
/// sign-out left figures on screen for an account that could no longer read them.
///
/// Emitted after the change has landed, never before: a window that re-read on
/// the way in would read the state being replaced.
///
/// Not to be confused with [`signal_session_dead`] and its
/// `session-signin-required`, which says the gateway *refused* a session the user
/// did not change. This one says the user changed it on purpose.
fn signal_session_changed() {
    if let Some(handle) = APP_HANDLE.get() {
        let _ = handle.emit("session-changed", ());
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

/// Process names of the AI tools we're willing to close, each paired with the
/// registry slug whose config rewrite makes that process stale - deliberately
/// both the agent CLIs *and* the desktop apps that share the binary name: on
/// macOS Claude Desktop / Cowork's main process is literally `Claude`, and it
/// is a routed tool that resolves the proxy at its own launch, so closing it is
/// the point. A subset of the registry tools: `hermes` and `openclaw` are
/// excluded - their names are too generic / their processes shouldn't be
/// killed from here. (An unrelated user binary that happens to be named
/// `claude`/`codex`/`opencode` is accepted collateral; the action sits behind
/// an explicit in-popover confirm.) Matched against the process name with any
/// `.exe` suffix stripped, so one list serves all three desktop OSes.
///
/// The slug is what makes the per-tool offer honest. Without it a Codex config
/// write offered to close - and then SIGTERMed - a running `claude`, because
/// the probe and the kill both walked the whole list. A tool that isn't here
/// contributes no names, so asking about it finds nothing rather than falling
/// back to everything.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
const AGENT_PROCESSES: [(&str, &str); 3] = [
    ("claude-code", "claude"),
    ("codex", "codex"),
    ("opencode", "opencode"),
];

/// The process names to scan for. `None` means every tool - the master toggle,
/// the popover's routing takeover and the diagnostics listing all genuinely
/// mean all of them. `Some(slugs)` narrows to the tools whose configs were
/// just rewritten; slugs with no process of their own (`hermes`, `openclaw`,
/// `env-proxy`, a proxy domain key) drop out, and `Some(&[])` scans nothing.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn agent_names_for(only: Option<&[String]>) -> Vec<&'static str> {
    AGENT_PROCESSES
        .iter()
        .filter(|(slug, _)| only.is_none_or(|slugs| slugs.iter().any(|s| s == slug)))
        .map(|(_, name)| *name)
        .collect()
}

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

/// Visit every running agent process whose name is in `names` (see
/// [`agent_names_for`]), skipping our own pid. Shared by the close command and
/// the count probes so all of them match the exact same process set.
///
/// Processes only, and only the fields `/proc/<pid>/stat` already carries.
/// sysinfo counts *threads* as processes and leaves that on by default -
/// `ProcessRefreshKind::nothing()` sets `tasks: true`, and the `refresh_processes`
/// convenience adds `.with_tasks()` on top - so the default walk descends into
/// every process's `task/` directory and runs a full read (`stat`, `statm`,
/// `io`, `cmdline`, `readlink exe`) per thread. On a 526-process desktop that
/// is 3.4k entries and ~140ms of procfs instead of 536 entries and ~10ms, and
/// it puts any thread whose `comm` matches an agent name in the list as if it
/// were a second copy of the tool. Name, pid and start time - all this needs -
/// come from `stat`, which is read either way.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn for_each_agent_process(names: &[&str], mut f: impl FnMut(&sysinfo::Process)) {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    if names.is_empty() {
        return;
    }
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().without_tasks(),
    );
    let own_pid = sysinfo::get_current_pid().ok();
    for (pid, process) in sys.processes() {
        if Some(*pid) == own_pid {
            continue;
        }
        if names.contains(&agent_name_of(process).as_str()) {
            f(process);
        }
    }
}

/// A process's name as [`AGENT_PROCESSES`] spells it: lowercased, with any
/// `.exe` stripped, so one list serves all three desktop OSes. Extracted so the
/// per-tool staleness check below matches on exactly the same normalisation the
/// walk itself filtered by, rather than a second copy that could drift from it.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn agent_name_of(process: &sysinfo::Process) -> String {
    let name = process.name().to_string_lossy().to_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_string()
}

/// Count running agent processes without touching them. Lets the frontend
/// skip the "close running agents" routing takeover when there is nothing to
/// close.
///
/// `(async)`, like every probe here that walks the process table: sync would
/// put the walk on the main thread, which on Linux is the GTK loop. This one
/// runs on the boot path, where a blocked loop is a window that looks like it
/// never opened.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command(async)]
fn running_agents_count() -> u32 {
    let mut count = 0u32;
    for_each_agent_process(&agent_names_for(None), |_| count += 1);
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
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let enabled_at = ROUTING_ENABLED_AT_UNIX.load(Ordering::Acquire);
    if enabled_at != 0 {
        return Some(enabled_at);
    }
    // Our own pid only. `All` would walk every process in the table to read
    // one field off exactly one of them, and this fallback is the *Linux* path
    // (the detached engine is what leaves `ROUTING_ENABLED_AT_UNIX` at 0), so
    // the listing below would otherwise scan the table twice per call.
    //
    // `without_tasks` matters even here: the pid filter is applied *after* the
    // walk, so with threads left on this still descends every `task/` dir to
    // then throw all of it away (~14ms, against ~0.5ms for the one process we
    // asked for). Same refresh kind as `for_each_agent_process`, whose comment
    // has the rest of the reasoning.
    let pid = sysinfo::get_current_pid().ok()?;
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        false,
        ProcessRefreshKind::nothing().without_tasks(),
    );
    sys.process(pid)
        .map(|p| p.start_time())
        .filter(|start| *start != 0)
}

/// Count running agent processes that were started *before* routing last came
/// up, i.e. the ones that resolved their connection pre-Gate and genuinely
/// need a restart to route. Same process set as `running_agents_count`; the
/// bound is the last in-process enable, falling back to our own process start
/// when routing was already up before we launched (detached Linux engine).
///
/// `(async)` for the reason on [`running_agents_count`], and doubly so here:
/// the fallback bound costs a second refresh, and this is the probe the boot
/// path and the `proxy-state-changed` handler both call.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command(async)]
fn stale_agents_count() -> u32 {
    let Some(bound) = routing_bound_unix() else {
        // No usable bound: degrade to "every running agent counts", the
        // pre-timestamp behavior, rather than silently claiming freshness.
        return running_agents_count();
    };
    let mut count = 0u32;
    for_each_agent_process(&agent_names_for(None), |process| {
        if process.start_time() < bound {
            count += 1;
        }
    });
    count
}

/// The user's Settings choices. Never fails: a missing or mangled file loads as
/// the documented defaults, because refusing to render Settings over a
/// preferences file is a worse failure than showing "everything on".
#[tauri::command]
fn get_preferences() -> gate_connect_core::preferences::Preferences {
    gate_connect_core::preferences::load()
}

/// This install's stable id, for the Settings row and for support threads.
///
/// Not the analytics distinct id, which Settings used to show: that one is absent
/// in a build with no PostHog key and absent again once somebody opts out of
/// diagnostics, so the row read Unavailable for reasons that had nothing to do
/// with the install. The diagnostics report still carries the analytics id under
/// its own name - they are two different facts.
#[tauri::command]
fn install_id() -> Result<String, String> {
    gate_connect_core::primitives::install_id().map_err(|e| format!("{e:#}"))
}

/// What to call this machine: the person's own name for it, or the hostname.
///
/// Resolved in core (`preferences::device_name`) so one place decides what an
/// absent override means. The stored value stays an `Option` (see
/// `preferences::device_name`), so clearing the name goes back to following the
/// hostname instead of freezing today's.
///
/// The *display* answer, and the hostname fallback is the reason it is not also
/// the wire answer: `preferences::device_label` sends nothing at all for a
/// device the user never named, so a person who skipped the naming step does not
/// have their hostname on every request. The window and the wire agree wherever
/// there is a name to agree on, and the Settings row says which case it is in.
#[tauri::command]
fn device_name() -> String {
    gate_connect_core::preferences::device_name()
}

/// Rename this device, or clear the name and follow the hostname again.
#[tauri::command]
fn set_device_name(name: String) -> Result<(), String> {
    gate_connect_core::preferences::set_device_name(&name).map_err(|e| format!("{e:#}"))
}

/// Turn routing-health notifications on or off. Gates the two notifications the
/// app actually fires: an expired session, and a quit that could not put a tool
/// back on its own settings.
#[tauri::command]
fn set_routing_health_notifications(enabled: bool) -> Result<(), String> {
    gate_connect_core::preferences::set_routing_health_notifications(enabled)
        .map_err(|e| format!("{e:#}"))
}

/// Turn blocked-request notifications on or off (AG-578).
///
/// Separate from the flagged switch because the two differ in weight: a block
/// stopped something the user was doing, a flag only noted it.
#[tauri::command]
fn set_blocked_event_notifications(enabled: bool) -> Result<(), String> {
    gate_connect_core::preferences::set_blocked_event_notifications(enabled)
        .map_err(|e| format!("{e:#}"))
}

/// Turn flagged-request notifications on or off (AG-578).
#[tauri::command]
fn set_flagged_event_notifications(enabled: bool) -> Result<(), String> {
    gate_connect_core::preferences::set_flagged_event_notifications(enabled)
        .map_err(|e| format!("{e:#}"))
}

/// Turn the sound on security notifications on or off (AG-578).
#[tauri::command]
fn set_security_notification_sound(enabled: bool) -> Result<(), String> {
    gate_connect_core::preferences::set_security_notification_sound(enabled)
        .map_err(|e| format!("{e:#}"))
}

/// The live security-event feed (AG-578), one per process.
///
/// A singleton because the connection is a shared resource, not a per-window
/// one: the main window and the tray popover both read it, and two windows
/// holding two streams would double the org's fan-out and show each of them half
/// the dedupe state.
fn security_feed() -> &'static std::sync::Arc<gate_connect_core::security_feed::client::Feed> {
    static FEED: std::sync::OnceLock<
        std::sync::Arc<gate_connect_core::security_feed::client::Feed>,
    > = std::sync::OnceLock::new();
    FEED.get_or_init(|| std::sync::Arc::new(gate_connect_core::security_feed::client::Feed::new()))
}

/// How often to retire closed notification-grouping windows.
///
/// Ten seconds against the grouper's 60s window: short enough that a trailing
/// summary reads as part of the same incident, long enough that an idle machine
/// is doing nothing measurable.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
const SECURITY_SWEEP_INTERVAL_SECS: u64 = 10;

/// Fire a desktop notification for one security event, if the user asked for one.
///
/// Everything interesting is in `security_feed::notify`: which switch gates this
/// action, and whether an identical event has already spoken inside the grouping
/// window. This function's only job is the side effect.
///
/// Best-effort throughout. A notification that cannot be shown must not affect the
/// feed, and the event is already on its way to the window regardless - the
/// in-app feed is the record, the notification is the interruption.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn notify_for_event(
    app: &tauri::AppHandle,
    grouper: &std::sync::Mutex<gate_connect_core::security_feed::notify::Grouper>,
    event: &gate_connect_core::security_feed::SecurityEvent,
) {
    let prefs = gate_connect_core::preferences::load();
    let due = {
        let Ok(mut g) = grouper.lock() else {
            // A poisoned grouper means an earlier panic while holding it. Saying
            // nothing is the safe direction: the alternative is un-grouped
            // notifications, which is the failure this exists to prevent.
            return;
        };
        g.admit(event, &prefs, std::time::Instant::now())
    };
    for notification in due {
        fire_notification(app, notification);
    }
}

/// Show one notification the grouper decided on.
///
/// Split from the decision so the event path and the sweep timer cannot drift on
/// how a notification is presented.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn fire_notification(
    app: &tauri::AppHandle,
    notification: gate_connect_core::security_feed::notify::Notification,
) {
    use gate_connect_core::security_feed::notify::Notification;
    use tauri_plugin_notification::NotificationExt;

    let Notification::Fire { title, body, sound } = notification else {
        return;
    };
    let mut builder = app.notification().builder().title(title).body(body);
    if sound {
        // The platform default rather than a bundled asset: a sound the user
        // already recognises as "your computer wants you" beats one they have to
        // learn, and it follows their Do Not Disturb settings.
        //
        // `sound` is one of the four fields the plugin's desktop path actually
        // forwards (title, body, icon, sound) - `id`, `group` and `group_summary`
        // exist on the builder but are mobile-only and are dropped here without
        // error, which is why the trailing summary is a second notification
        // rather than an update to the first.
        builder = builder.sound("default");
    }
    let _ = builder.show();
}

/// What the feed is doing: `live`, `reconnecting` or `offline` (AC4).
///
/// Read on mount. Afterwards the window follows the `security-feed-state` event,
/// but a window that opened mid-session has missed every event so far and needs
/// somewhere to start.
#[tauri::command]
fn security_feed_state() -> gate_connect_core::security_feed::FeedState {
    security_feed().state()
}

/// The events the feed has buffered, oldest first.
///
/// Tauri events only reach a window that is already listening, and the tray
/// window is created and destroyed on demand - so without this a popover opened
/// after ten blocked requests would show an empty feed and call it "no security
/// events", which is a different claim entirely.
#[tauri::command]
fn security_feed_recent() -> Vec<gate_connect_core::security_feed::SecurityEvent> {
    security_feed().recent()
}

/// AC6's recovery action: the "Try again" behind an Unavailable feed.
///
/// Wakes the connection loop out of whatever backoff it is sitting in, so a user
/// who clicks Retry sees something happen instead of waiting out a 60s sleep.
#[tauri::command]
fn security_feed_retry() {
    security_feed().retry_now();
}

/// Record whether Gate Connect may send diagnostic data. Onboarding records the
/// first answer; this is Settings changing it. Nothing is uploaded here - the
/// send path is its own story.
#[tauri::command]
fn set_share_diagnostics(enabled: bool) -> Result<(), String> {
    gate_connect_core::preferences::set_share_diagnostics(enabled).map_err(|e| format!("{e:#}"))
}

/// The process name to look for on behalf of one tool.
///
/// `None` for the tools Gate has no way to recognise in the process table:
/// OpenClaw and Hermes ship no fixed process name, and `env-proxy` is not a
/// process at all. For those, staleness is *unobservable* rather than false -
/// see [`reopen_pending_for`], which says what that costs.
///
/// Reads [`AGENT_PROCESSES`] rather than repeating it: the per-tool verdict and
/// the per-tool close offer have to name the same process for a slug, or one of
/// them is talking about a tool the other is not.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn agent_process_name(slug: &str) -> Option<&'static str> {
    AGENT_PROCESSES
        .iter()
        .find(|(s, _)| *s == slug)
        .map(|(_, name)| *name)
}

/// Is a process for this one tool running that predates the last routing change,
/// and is therefore still using whatever settings it loaded then?
///
/// This is `stale_agents_count` narrowed to one tool, which is what a per-tool
/// verdict needs: the count answers "does anything need restarting", and cannot
/// say *which* row to mark.
///
/// Two honest limits, both deliberate:
///
/// - **A tool with no known process name returns `false`**, so it can still read
///   `On`. The alternative - reporting every such tool unverifiable forever -
///   would bury a real signal (the route probe) under a permanent warning. The
///   cost is that OpenClaw and Hermes will not be told to reopen.
/// - **No usable bound degrades to "running means stale"**, matching
///   `stale_agents_count` rather than claiming freshness we cannot support.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn reopen_pending_for(slug: &str) -> bool {
    let Some(wanted) = agent_process_name(slug) else {
        return false;
    };
    let bound = routing_bound_unix();
    let mut pending = false;
    for_each_agent_process(&[wanted], |process| match bound {
        Some(bound) => {
            if process.start_time() < bound {
                pending = true;
            }
        }
        None => pending = true,
    });
    pending
}

/// One tool's routing verdict, flattened for the frontend.
///
/// `state` / `reason` / `next_action` are strings rather than a tagged union
/// because the pairing is fixed in
/// [`gate_connect_core::routing_health::Reason::next_action`] and the UI only
/// ever renders them. `reason` and `next_action` are both `None` unless `state`
/// is `needs_attention`.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[derive(Serialize)]
struct VerdictDto {
    slug: String,
    state: &'static str,
    reason: Option<&'static str>,
    next_action: Option<&'static str>,
}

/// What every config-routed tool is actually doing.
///
/// Deliberately a separate command from `list_tools`: this one does network I/O
/// (a loopback health check, and one gateway call when the account is OAuth) and
/// walks the process table, none of which belongs on the path the popover calls
/// on every render.
///
/// The two probes run **once** for the whole sweep, not once per tool. They ask
/// about shared infrastructure - the relay port and the account's session - so
/// per-tool calls would be the same answer at N times the cost, and would let
/// two rows in one refresh disagree about whether the session is alive.
///
/// Off the main thread for the reason on [`running_agents_count`] - but as a
/// real `async fn` handing the work to `spawn_blocking`, not as
/// `#[tauri::command(async)]` on a sync fn. That attribute does not move a sync
/// body to the blocking pool: the macro inlines it into `async_runtime::spawn`,
/// so it runs on a tokio *worker*, with the runtime entered. Both probes below
/// are blocking HTTP, and reqwest's blocking client asserts against being
/// called from an entered worker in debug builds - it builds and immediately
/// drops a throwaway runtime purely to detect the case. The resulting panic
/// took the task with it, so the webview's `invoke` promise never settled and
/// the refresh hung.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
async fn routing_verdicts() -> Vec<VerdictDto> {
    // A join error means the probe itself panicked. An empty sweep is what this
    // command already returns when it can tell nothing about any tool.
    tauri::async_runtime::spawn_blocking(routing_verdicts_now)
        .await
        .unwrap_or_default()
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn routing_verdicts_now() -> Vec<VerdictDto> {
    use gate_connect_core::routing_health::{self, ConfigState, Evidence};

    let route = gate_connect_core::proxy::probe_relay_route();
    let session = probe_session_health();

    registry::registry()
        .iter()
        .filter(|integ| !integ.hidden_in_ui())
        .map(|integ| {
            let status = integ.status();
            let installed = !matches!(status, Ok(gate_connect_core::Status::NotInstalled));
            let slug = integ.id().to_string();
            let verdict = routing_health::verdict_for(&Evidence {
                installed,
                config: ConfigState::from_status(&status),
                route,
                session,
                reopen_pending: reopen_pending_for(&slug),
            });
            let reason = verdict.reason();
            VerdictDto {
                slug,
                state: verdict.as_str(),
                reason: reason.map(|r| r.as_str()),
                next_action: reason.map(|r| r.next_action().as_str()),
            }
        })
        .collect()
}

/// Ask the gateway whether the stored session still works, mapped onto the
/// verdict layer's vocabulary.
///
/// An API-key account reports `Valid`: there is no session to probe, and the key
/// is validated when it is saved. Reporting `Unknown` instead would park every
/// key-based install on "Verification failed" permanently.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn probe_session_health() -> gate_connect_core::routing_health::SessionHealth {
    use gate_connect_core::org::SessionProbe;
    use gate_connect_core::routing_health::SessionHealth;

    if account::auth_mode().unwrap_or_default() != account::AuthMode::OAuth {
        return SessionHealth::Valid;
    }
    let Some(tokens) = gate_connect_core::oauth::live_session() else {
        // No live session in OAuth mode is a definite negative: the refresh loop
        // either has no tokens or the gateway already rejected them.
        return SessionHealth::Rejected;
    };
    let Ok(Some(gateway)) = account::load_base_url() else {
        return SessionHealth::Unknown;
    };
    match gate_connect_core::org::probe_session(&gateway, &tokens.access_token) {
        SessionProbe::Accepted(_) => SessionHealth::Valid,
        SessionProbe::Rejected => SessionHealth::Rejected,
        // Offline or a non-auth error. Never evidence against the credential -
        // `SessionProbe::Unavailable`'s own docs are explicit about this, and
        // the verdict layer turns it into "Verification failed", not "Access
        // problem".
        SessionProbe::Unavailable => SessionHealth::Unknown,
    }
}

/// What a routing restore still owes, from the snapshots on disk.
///
/// Read-only and cheap: it opens no tool config and starts nothing, so the UI can
/// call it on a status refresh. Empty in the normal case.
#[tauri::command]
fn pending_restore() -> Result<gate_connect_core::provider::PendingRestore, String> {
    gate_connect_core::provider::pending_restore().map_err(|e| format!("{e:#}"))
}

/// Finish an interrupted restore, and report what is still outstanding.
///
/// `restore_all` already has the retry semantics this needs: it re-attempts each
/// recorded entry, keeps failures in the snapshot, and clears the file only once
/// everything is back. So resuming repeats no completed write and reopens no
/// verified tool without any new bookkeeping.
///
/// Returns the remaining pending state rather than unit, so a caller does not have
/// to guess whether the resume finished the job - a partial success is the
/// interesting case and the one that must not read as done.
#[tauri::command]
async fn resume_restore() -> Result<gate_connect_core::provider::PendingRestore, String> {
    tauri::async_runtime::spawn_blocking(|| {
        // Best-effort, like every other caller of this: a failure leaves its
        // entries in the snapshot, which is exactly what the return value reports.
        if let Err(e) = gate_connect_core::provider::restore_all() {
            eprintln!("[gate] resuming an interrupted restore failed: {e}");
            report_backend_error("provider_restore", format!("{e:#}"));
        }
        gate_connect_core::provider::pending_restore().map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| format!("resume restore join error: {e}"))?
}

/// What the last routing restore did, entry by entry.
///
/// Read-only, and `None` when there is nothing to explain - a restore that
/// completed clears its journal. Never fails: a journal that cannot be read is an
/// explanation lost, not a recovery blocked, so an unreadable one reads as absent.
#[tauri::command]
fn restore_journal() -> Option<gate_connect_core::recovery::RestoreJournal> {
    gate_connect_core::recovery::load()
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
    /// The names this scan looked for (see [`agent_names_for`]). Reported so an
    /// empty list reads as "none of these were running" rather than "no AI
    /// tools are running" - the scan does not cover Hermes or OpenClaw, and a
    /// report that hid that would be read as evidence they were stopped. With
    /// an `only` filter it also names which tool the question was about, so a
    /// caller cannot mistake a narrow answer for a whole-machine one.
    scanned_names: Vec<String>,
    agents: Vec<RunningAgent>,
}

/// The running agent processes themselves, not just how many: name, pid, when
/// each started, and whether it predates routing. Same process set and the
/// same staleness rule as the two count probes, so the diagnostics report and
/// the routing takeover can never disagree about what is running.
///
/// `only` narrows the scan to the tools whose configs were just rewritten - a
/// per-app switch passes its own slug, a family cascade the slugs it touched.
/// `None` asks about every tool, which is what diagnostics and the master
/// toggle mean. Without it, flipping one tool listed the others: the offer
/// named a `claude` that nothing had reconfigured, and the confirm behind it
/// would have killed it.
///
/// Deliberately carries no command line: argv on these tools routinely holds
/// prompts, file paths and occasionally a key, and this list is built to be
/// pasted into a support thread.
///
/// `(async)` for the same reason as [`diagnostics`]: this walks the whole
/// process table, and a sync command would do that on the main thread - the
/// GTK loop on Linux - with the popover frozen until it returns.
/// `only` narrows the scan to the processes belonging to those tool slugs.
///
/// Omitted means every agent, which is right for a master toggle: it changed
/// the route for all of them, and all of them are stale until they restart. It
/// is wrong for a single tool - offering to close Claude because someone
/// switched Codex names processes the change did not touch, and asks to kill
/// work for no reason.
///
/// A slug with no process to look for contributes nothing rather than widening
/// the scan back to everything. `agent_process_name` covers the three tools this
/// scan knows; OpenClaw and Hermes have none, and a filter that silently fell
/// back to "all" for them would reintroduce exactly this bug for the tools it
/// least applies to.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command(async)]
fn running_agents(only: Option<Vec<String>>) -> RunningAgentsDto {
    let names = agent_names_for(only.as_deref());
    let bound = routing_bound_unix();
    let mut agents = Vec::new();
    for_each_agent_process(&names, |process| {
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
        scanned_names: names.iter().map(|n| n.to_string()).collect(),
        agents,
    }
}

/// Terminate running agent processes (CLIs and desktop apps, see
/// [`AGENT_PROCESSES`]) so their next launch picks up the routing change.
/// Graceful where the platform allows it (SIGTERM on
/// macOS/Linux, so agents can flush state; Windows only has TerminateProcess).
/// Returns how many processes were signalled - 0 means none were running.
/// Best-effort: processes we can't signal (another user's, already gone) are
/// skipped, not errors.
///
/// `only` carries the same tool-slug filter as [`running_agents`], and callers
/// are expected to pass back exactly what they offered: this signals processes,
/// so the set it kills must be the set the user was shown and agreed to.
///
/// `(async)` on top of the walk's own reason: this one also blocks on
/// signalling every match, and it runs from a button the user is watching.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command(async)]
fn close_running_agents(only: Option<Vec<String>>) -> u32 {
    use sysinfo::Signal;
    let mut closed = 0u32;
    for_each_agent_process(&agent_names_for(only.as_deref()), |process| {
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

/// How far the wall clock may drift from elapsed monotonic time across one
/// refresh tick before the background loop treats it as a jump rather than
/// drift. Ordinary NTP slew across 30s is milliseconds; a resume or a stepped
/// correction is minutes to hours. Wide enough that nothing renews for a
/// well-behaved clock, narrow enough to catch a sleep short of the token's own
/// lifetime.
const CLOCK_JUMP_TOLERANCE: std::time::Duration = std::time::Duration::from_secs(120);

/// Raise the dead-session signal from somewhere other than the refresh loop:
/// the gateway-auth observer, which learns from a refused request that the
/// session is gone without waiting up to 30s for the next tick.
///
/// Edge-guarded on the same flag the loop swaps, so the two can't both react
/// to one death - whichever gets there first does the work and the other sees
/// the flag already set. Repaints the tray, nudges a mounted popover (which
/// re-reads `oauth_status` and routes to sign-in; the frontend has no status
/// poll by design), and posts the same notification the refresh loop would
/// have, on the same platforms it notifies on - the tray dot alone is out of
/// the user's eyeline while they sit watching a tool fail.
/// Re-verify a session the gateway has just refused, and react to the verdict.
///
/// Shared by the two triggers, which differ only in how the refusal reaches us:
/// the in-process engine's observer on macOS/Windows, and the session loop's
/// poll of the helper daemon's refusal counter on Linux. The response to a
/// verdict is not platform-specific, so it lives in one place.
///
/// Says nothing itself about whether the session is dead - that is
/// [`gate_connect_core::startup::reverify_session`]'s call, made with our own
/// token against the gateway. This only carries out what it decided.
///
/// Does NOT take the [`gate_connect_core::proxy::GateAuthCheck`] debounce
/// guard: the engine-side latch belongs to whichever process saw the 401, which
/// on Linux is the daemon, and the observer path takes it at the top of its own
/// thread so a panic still releases it.
fn recheck_gate_session(app: &tauri::AppHandle) {
    match gate_connect_core::startup::reverify_session() {
        // The session was alive and the local clock was simply wrong about it.
        // The forced refresh minted a token that works; push it into the
        // running engine so in-flight tools recover without a restart, and take
        // back any dead-session signal.
        gate_connect_core::startup::Recheck::Recovered(token) => {
            gate_connect_core::proxy::manager().refresh_token(&token);
            if SESSION_NEEDS_SIGNIN.swap(false, Ordering::Relaxed) {
                let running = gate_connect_core::proxy::manager()
                    .status()
                    .map(|s| s.running)
                    .unwrap_or(false);
                update_tray_status(app, running);
            }
        }
        gate_connect_core::startup::Recheck::Dead => {
            signal_session_dead(app);
        }
        // No verdict (offline, or the 401 belonged to the client's own upstream
        // credential rather than to us): change nothing.
        gate_connect_core::startup::Recheck::Unchanged => {}
    }
}

fn signal_session_dead(app: &tauri::AppHandle) {
    if SESSION_NEEDS_SIGNIN.swap(true, Ordering::Relaxed) {
        return;
    }
    let running = gate_connect_core::proxy::manager()
        .status()
        .map(|s| s.running)
        .unwrap_or(false);
    update_tray_status(app, running);
    let _ = app.emit("session-signin-required", ());
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use tauri_plugin_notification::NotificationExt;
        let _ = app
            .notification()
            .builder()
            .title("Gate Connect")
            .body("Your session expired. Open Gate Connect to sign in again and keep routing.")
            .show();
    }
}

/// Stop pinning the popover open. The frontend calls this on the user's
/// first interaction with the first-launch window, switching the popover
/// back to normal click-outside-to-dismiss behavior.
#[tauri::command]
fn unpin_popover() {
    POPOVER_PINNED.store(false, Ordering::Release);
}

/// Pin the popover open for the duration of a call that raises a system trust
/// dialog. Without this, `proxy_trust_ca` is the one action in the app that
/// hides the window it was clicked in: the OS dialog takes focus, a
/// `Focused(false)` handler hides the popover, and the copy telling the user
/// what to click goes with it.
///
/// **That handler does not currently exist**, and neither does the dismissal
/// this guards against. There is no `Focused(false)` arm anywhere in this
/// file, so [`POPOVER_PINNED`] is written at four sites and read at none, and
/// the tray sits over every other window until the icon is clicked again. Kept
/// rather than deleted because the two commands are still called by the
/// retiring `src/App.tsx` shell and because the pin is what a blur-dismiss
/// would need on the day one lands. Whether it should is a product question,
/// raised in `docs/figma-questions-for-design.md`.
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

/// Label of the Cloudflare challenge-solve webview (see
/// [`open_cf_challenge_window`]). Named because the window-event handler must
/// recognise it too.
const CF_CHALLENGE_WINDOW: &str = "cf-challenge";

/// The last `cf_clearance` value fed into the engine. Lets the solve window
/// tell a pre-existing still-good cookie in the webview store (feed it
/// straight away with no user action - the engine value is memory-only, so
/// this is the normal case right after an app restart) from the very cookie
/// Cloudflare just challenged (wait for the solve to mint a new value;
/// re-feeding the stale one would loop: inject -> challenge -> reopen).
#[cfg(any(target_os = "macos", target_os = "windows"))]
static LAST_CF_CLEARANCE: Mutex<String> = Mutex::new(String::new());

/// Open (or refocus) the Cloudflare challenge-solve webview at the real
/// chatgpt.com and poll its cookie store for the `cf_clearance` a solved
/// interstitial mints. On capture: feed the cookie into the running engine,
/// close the window, and clear the solve latch. If the user closes the window
/// first, just clear the latch - the next challenged turn re-opens it.
///
/// The webview loads the live site the way the browser does (it honours the
/// system proxy, and `/` is not a rewritten path, so the interstitial itself
/// egresses from the user's IP) - the browser path is the one empirically
/// proven to mint a cookie Cloudflare then accepts from Gate's IP.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn open_cf_challenge_window(app: &tauri::AppHandle) {
    // A window from a previous attempt should not exist here - the observer
    // is latched, so only one attempt runs at a time, and every exit path
    // closes its window. If one survived anyway, destroy it and start over
    // rather than adopting it: the poll thread is what reveals the window,
    // captures the cookie, and releases the latch, so a window with no live
    // poll behind it is a wedge that suppresses every later challenge for
    // the rest of the session. Since the window is now built hidden, such an
    // orphan would also be invisible - the exact failure this avoids.
    // `destroy`, not `close`: close is asynchronous, so the build below
    // would race the teardown and fail on the still-taken label.
    if let Some(stale) = app.get_webview_window(CF_CHALLENGE_WINDOW) {
        eprintln!("[gate] challenge-solve: destroying an orphaned solve window");
        let _ = stale.destroy();
    }
    let url: tauri::Url = "https://chatgpt.com"
        .parse()
        // Infallible: static, pre-validated URL.
        .expect("static chatgpt.com URL parses");
    let builder = tauri::WebviewWindowBuilder::new(
        app,
        CF_CHALLENGE_WINDOW,
        tauri::WebviewUrl::External(url.clone()),
    )
    .title("Verify ChatGPT connection")
    .inner_size(480.0, 640.0)
    .center()
    // Starts hidden. A non-interactive managed challenge is pure JavaScript
    // and resolves on its own, so the common case should cost the user no
    // window at all; the poll below reveals it only when a cookie has not
    // appeared, which is the signature of the kind that wants a click.
    //
    // Hidden is not free: a window the platform considers invisible can have
    // its rendering and timers throttled, and the challenge leans on exactly
    // that work, so it may simply not complete while concealed. The reveal is
    // the safety net either way - the worst case is that the window appears a
    // few seconds later and behaves as it always did.
    .visible(false);
    // Wear the app's own user-agent: a stock webview is waved through without
    // a challenge, and `cf_clearance` only exists as the result of one, so
    // without this there is nothing to capture. See
    // `proxy::chatgpt_app_user_agent`. Absent until the engine has seen an app
    // request, in which case the platform default stands.
    let app_user_agent = gate_connect_core::proxy::chatgpt_app_user_agent();
    let builder = match app_user_agent.as_deref() {
        Some(ua) => builder.user_agent(ua),
        None => builder,
    };
    eprintln!(
        "[gate] challenge-solve window opening as {}",
        app_user_agent.as_deref().unwrap_or("<platform default UA>")
    );
    // Built for its side effect; the poll thread below re-resolves the window
    // by label, so there is nothing to hold on to here.
    if let Err(e) = builder.build() {
        eprintln!("[gate] opening the challenge-solve window failed: {e}");
        report_backend_error("cf_challenge_window", format!("{e}"));
        gate_connect_core::proxy::cf_challenge_solve_finished(false);
        return;
    }
    // Deliberately no `set_focus` here: the window is hidden, and the whole
    // point of starting it that way is to not interrupt someone mid-sentence
    // for a challenge that may well solve itself. Focus is taken at the
    // reveal below, where a human genuinely has to act.

    // Poll for the cookie rather than hooking navigation: the challenge
    // round-trips within one page, so the cookie can land without any
    // navigation event. A separate thread is also what the cookie API needs
    // on Windows - reading cookies from an event handler deadlocks WebView2.
    let app = app.clone();
    std::thread::spawn(move || {
        let challenged = LAST_CF_CLEARANCE
            .lock()
            .map(|v| v.clone())
            .unwrap_or_default();
        let mut last_names = String::new();
        let mut reported_unchanged = false;
        // Bounded, because the window can sit on a challenge it will never
        // clear: the poll would otherwise log a jar line every two seconds
        // for as long as the app runs. Generous enough for a person to read
        // an interstitial and click through it.
        let started = std::time::Instant::now();
        let deadline = started + std::time::Duration::from_secs(180);
        // How long a non-interactive challenge gets to resolve unseen before
        // we assume it wants a human. Short enough that an interactive one
        // does not feel stalled, long enough to cover a page load plus the
        // challenge round trip on a slow link.
        let reveal_at = started + std::time::Duration::from_secs(8);
        let mut revealed = false;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            // Window gone: the user gave up (or the capture below already
            // closed it). Release the latch, reporting no capture so the
            // cooldown keeps the next challenge from reopening immediately.
            let Some(window) = app.get_webview_window(CF_CHALLENGE_WINDOW) else {
                gate_connect_core::proxy::cf_challenge_solve_finished(false);
                return;
            };
            if std::time::Instant::now() >= deadline {
                eprintln!(
                    "[gate] challenge-solve: no cf_clearance after 3 minutes, giving up on this attempt"
                );
                let _ = window.close();
                gate_connect_core::proxy::cf_challenge_solve_finished(false);
                return;
            }
            // Nothing captured while hidden: either the challenge wants a
            // click, or being concealed stopped it running. Both are fixed by
            // putting it in front of the user, and this is the one moment in
            // the flow where taking focus is warranted.
            if !revealed && std::time::Instant::now() >= reveal_at {
                eprintln!("[gate] challenge-solve: not resolved on its own, showing the window");
                let _ = window.show();
                let _ = window.set_focus();
                revealed = true;
            }
            let cookies = window.cookies_for_url(url.clone()).unwrap_or_default();
            // Which cookies the jar holds, by NAME only - the values are
            // session credentials. Without this, the window's behaviour is
            // the only signal, and "Cloudflare never minted a cf_clearance"
            // looks identical to "capture is broken", which cost a build to
            // tell apart. Logged only when the set CHANGES: it is polled
            // every two seconds and the jar is usually static.
            let names = cookies
                .iter()
                .map(|c| c.name())
                .collect::<Vec<_>>()
                .join(",");
            if names != last_names {
                eprintln!("[gate] challenge-solve jar: [{names}]");
                last_names = names;
            }
            let captured = cookies
                .into_iter()
                .find(|c| c.name() == "cf_clearance")
                .map(|c| c.value().to_string());
            let Some(value) = captured else { continue };
            if value.is_empty() || value == challenged {
                // A cookie identical to the one just challenged is not a
                // solve; re-feeding it would loop the window open. Said once
                // - the poll re-reads the same jar until the deadline.
                if !reported_unchanged {
                    eprintln!(
                        "[gate] challenge-solve: cf_clearance present but unchanged, waiting"
                    );
                    reported_unchanged = true;
                }
                continue;
            }
            if let Ok(mut last) = LAST_CF_CLEARANCE.lock() {
                *last = value.clone();
            }
            gate_connect_core::proxy::manager().refresh_cf_clearance(&value);
            eprintln!("[gate] challenge-solve: captured cf_clearance, fed to the engine");
            let _ = window.close();
            gate_connect_core::proxy::cf_challenge_solve_finished(true);
            return;
        }
    });
}

/// Bring the main window back on screen, wherever the user left it. The
/// onboarding flow calls this from its "locate Gate Connect" button and on
/// close, so the handoff always ends at the app.
///
/// Deliberately does not reposition. This is a 1280x800 window, not a tray
/// popover: moving it out from under the user's cursor on every reveal is
/// exactly what a window must not do.
/// Also clears [`POPOVER_VISIBLE`]: handing over to the main window means the
/// tray is going away, and `TrayApp` hides itself with a frontend
/// `getCurrentWindow().hide()` that Rust never sees as a window event. Without
/// this the flag leaked `true` past every Expand-app and every Quit.
fn reveal_popover_window(app: &tauri::AppHandle) {
    POPOVER_VISIBLE.store(false, Ordering::Release);
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    #[cfg(target_os = "linux")]
    let _ = window.unminimize();
    // Before the show, for the reason in `map_maximized_for_decorations`.
    #[cfg(target_os = "linux")]
    map_maximized_for_decorations(&window);
    let _ = window.show();
    let _ = window.set_focus();
    #[cfg(target_os = "macos")]
    order_front_regardless(&window);
}

#[tauri::command]
fn reveal_popover(app: tauri::AppHandle) {
    reveal_popover_window(&app);
}

/// Show the tray popover (window label `tray`), anchored to the tray icon
/// where the platform reports a rect and at the cursor on Linux, where the
/// SNI/AppIndicator protocol does not. Placement is correct *here*, unlike
/// the main window's reveal, which deliberately stopped repositioning
/// (c63e1880): this window is a popover again, and a popover that opens
/// wherever it was last left reads as detached from the icon that summoned it.
fn reveal_tray_window(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("tray") else {
        return;
    };
    #[cfg(not(target_os = "linux"))]
    if let Some(rect) = app.tray_by_id("main").and_then(|t| t.rect().ok().flatten()) {
        anchor_under_tray(&window, rect.position, rect.size);
    }
    #[cfg(target_os = "linux")]
    if let Ok(cursor) = app.cursor_position() {
        anchor_at_cursor(&window, cursor);
    }
    let _ = window.show();
    let _ = window.set_focus();
    #[cfg(target_os = "macos")]
    order_front_regardless(&window);
}

/// Position the tray popover centered horizontally on the tray icon and just
/// above or below it, whichever side has room on the icon's monitor - macOS's
/// menu bar is at the top so the popover lands below, Windows' taskbar is
/// typically at the bottom so it flips above. X is clamped to the monitor so
/// an icon near an edge doesn't push the popover past it. Resurrected from
/// c63e1880, where it served the old popover, scoped to the tray window now.
#[cfg(not(target_os = "linux"))]
fn anchor_under_tray(window: &tauri::WebviewWindow, tray_pos: Position, tray_size: Size) {
    let scale = window.scale_factor().unwrap_or(1.0);
    let pos = tray_pos.to_physical::<f64>(scale);
    let size = tray_size.to_physical::<f64>(scale);

    let window_w_px = window
        .outer_size()
        .map(|s| s.width as f64)
        .unwrap_or(400.0 * scale);
    let window_h_px = window
        .outer_size()
        .map(|s| s.height as f64)
        .unwrap_or(700.0 * scale);

    let tray_center_x = pos.x + size.width / 2.0;
    let tray_top_y = pos.y;
    let tray_bottom_y = pos.y + size.height;
    let gap = 6.0 * scale;

    // Fall back to the unbounded below-icon placement if we can't read the
    // monitor - better than refusing to show the window.
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

/// Pick the monitor whose physical bounds contain point (x, y) - the display
/// the clicked tray icon is on, not whichever one the window was on last.
/// Falls back to the window's current monitor, then the primary, so there is
/// always somewhere to show. Resurrected from c63e1880 with `anchor_under_tray`.
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

/// Position the tray popover above or below the cursor on Linux, where the
/// tray protocol exposes no usable icon rect - and on GNOME, where the
/// left-click event often never fires, so the menu's Quick status entry is
/// how users get here. On Wayland the compositor may ignore `set_position`
/// outright; on X11 this lands the popover near the click. Resurrected from
/// c63e1880 with `anchor_under_tray`.
#[cfg(target_os = "linux")]
fn anchor_at_cursor(window: &tauri::WebviewWindow, cursor: PhysicalPosition<f64>) {
    let scale = window.scale_factor().unwrap_or(1.0);

    let window_w_px = window
        .outer_size()
        .map(|s| s.width as f64)
        .unwrap_or(400.0 * scale);
    let window_h_px = window
        .outer_size()
        .map(|s| s.height as f64)
        .unwrap_or(700.0 * scale);

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
    // **Linux exits outright, and that is correct.** It looks like the flow
    // being skipped on one platform, and it is not: there the engine is a
    // DETACHED helper daemon that outlives this process (see the note at
    // `LAUNCH_AT_LOGIN`-adjacent code, "on Linux the engine lives in a
    // detached helper daemon", and "Linux has no exit-time safe point - the
    // RunEvent::Exit handler is macOS/Windows-only"). Quitting the GUI there
    // stops nothing: routing continues, no config is left aimed at a dead
    // relay, and so there is no question to ask. Asking would be worse than
    // silent - the "disconnect and quit" branch would tear down routing the
    // user never needed to lose.
    //
    // I removed this gate once, reasoning from the docstring above and from
    // AG-596, and it was a regression. Do not remove it again.
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

/// The tray popover's own Quit entry: the same three-way quit the tray menu's
/// item runs, so both entrances exit outright or defer to the main window's
/// dialog when config-routed tools would be left pointing at a dead relay.
#[tauri::command]
fn request_app_quit(app: tauri::AppHandle) {
    request_quit(&app);
}

/// Quit-time integration teardown (the "turn off integrations" quit choice):
/// snapshot + disconnect every enabled provider AND the managed standalone
/// tools no provider maps, so all the CLI tools fall back to their original
/// settings while Gate Connect is closed, WITHOUT touching the routing
/// intent - the startup restore reapplies both snapshots the next time the
/// app runs with routing intended on. Fires a system notification (the
/// popover dies with the process) telling the user to restart running CLI
/// agents, which keep the relay address they resolved at their own launch.
///
/// Returns the display names of any tools it could **not** return to their own
/// settings. Empty means the teardown was clean. A non-empty list is not an
/// error - the rest of the sweep still ran - but the caller must not report the
/// quit as tidy, and must not quit without saying so.
#[tauri::command]
async fn disconnect_tools_for_quit(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    // Off the main thread: disconnect does config-file I/O.
    let failed: Vec<String> = tauri::async_runtime::spawn_blocking(|| {
        gate_connect_core::provider::snapshot_and_disable_everything().map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| format!("disconnect join error: {e}"))??;
    // Gated on the routing-health preference: this is a notification about
    // routing, and a switch the user turned off has to actually stop something or
    // it was never a switch. The list is still returned either way - suppressing
    // the notification must not suppress the *result*.
    if gate_connect_core::preferences::load().routing_health_notifications {
        use tauri_plugin_notification::NotificationExt;
        // The clean-teardown wording is only true when the teardown was clean.
        // With a tool left on Gate's settings, telling the user everything is
        // back is worse than saying nothing: their next request goes to a relay
        // that died with this process, and the notification told them it would
        // not.
        let body = if failed.is_empty() {
            // "Your tools", not "Integrations": that word reaches the user
            // nowhere else in the product, and this notification arrives
            // seconds after a panel that called them tools. The rest of the
            // wording is shared with QuitConfirm on purpose.
            "Your tools are back on their own settings while Gate Connect is \
             closed. Restart any running CLI agents; everything reconnects \
             when Gate Connect starts again."
                .to_string()
        } else {
            format!(
                "Gate Connect closed, but {} could not be put back on {} own \
                 settings. {} still point at Gate and will not reach a model until \
                 Gate Connect runs again.",
                failed.join(", "),
                if failed.len() == 1 { "its" } else { "their" },
                if failed.len() == 1 { "It" } else { "They" },
            )
        };
        let _ = app
            .notification()
            .builder()
            .title("Gate Connect")
            .body(body)
            .show();
    }
    Ok(failed)
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
                    set_billing_mode,
                    oauth_list_orgs,
                    activity_overview,
                    activity_installations,
                    activity_cached_overview,
                    activity_cached_tool_overviews,
                    activity_tool_events,
                    tool_model_preferences,
                    set_tool_model,
                    gate_model_catalogue,
                    gate_credits,
                    log_message,
                    log_file_path,
                    set_org,
                    app_platform,
                    diagnostics,
                    unpin_popover,
                    pin_popover,
                    open_onboarding_window,
                    reveal_popover,
                    quit_app,
                    pending_quit_tools,
                    request_app_quit,
                    disconnect_tools_for_quit,
                    list_providers,
                    proxy_status,
                    proxy_enable,
                    proxy_disable,
                    proxy_set_domain,
                    proxy_set_env_export,
                    proxy_trust_ca,
                    proxy_untrust_ca,
                    launch_at_login_status,
                    set_launch_at_login,
                    get_preferences,
                    set_routing_health_notifications,
                    set_share_diagnostics,
                    install_id,
                    device_name,
                    set_device_name,
                    set_updater_relaunching,
                    routed_clients_stale,
                    routing_verdicts,
                    pending_restore,
                    resume_restore,
                    restore_journal,
                    running_agents_count,
                    stale_agents_count,
                    running_agents,
                    close_running_agents,
                    drain_backend_errors,
                    security_feed_state,
                    security_feed_recent,
                    security_feed_retry,
                    set_blocked_event_notifications,
                    set_flagged_event_notifications,
                    set_security_notification_sound,
                ]
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
            {
                tauri::generate_handler![
                    list_tools,
                    tool_status,
                    connect_tool,
                    disconnect_tool,
                    get_account,
                    get_account_key_prefix,
                    backfill_account_key_prefix,
                    save_account,
                    clear_account,
                    switch_gateway,
                    oauth_status,
                    oauth_sign_out,
                    set_auth_mode,
                    set_billing_mode,
                    oauth_list_orgs,
                    activity_overview,
                    activity_installations,
                    activity_cached_overview,
                    activity_cached_tool_overviews,
                    activity_tool_events,
                    tool_model_preferences,
                    set_tool_model,
                    gate_model_catalogue,
                    gate_credits,
                    log_message,
                    log_file_path,
                    set_org,
                    app_platform,
                    diagnostics,
                    unpin_popover,
                    pin_popover,
                    open_onboarding_window,
                    reveal_popover,
                    quit_app,
                    pending_quit_tools,
                    request_app_quit,
                    disconnect_tools_for_quit,
                    list_providers,
                    set_updater_relaunching,
                    get_preferences,
                    set_routing_health_notifications,
                    set_share_diagnostics,
                    install_id,
                    device_name,
                    set_device_name,
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
            // The challenge-solve webview is a regular window too: closing it
            // really closes it (the capture thread notices and clears the
            // solve latch), and the popover's blur-dismiss below must not
            // hide it mid-solve.
            if window.label() == CF_CHALLENGE_WINDOW {
                return;
            }
            // First post-map event after a maximised map: geometry is settled,
            // so put the window back to its configured size. Main window only:
            // the repair's pending flags are the main reveal's, and the tray
            // popover - undecorated, so never repaired - must not consume them
            // and get resized to the main window's bounds.
            #[cfg(target_os = "linux")]
            if window.label() == "main" {
                match event {
                    WindowEvent::Resized(_) => {
                        restore_after_repair(window);
                        // After the repair, not before: it is the one thing
                        // allowed to move this window through odd geometry.
                        clamp_to_minimum(window);
                    }
                    WindowEvent::Focused(true) => {
                        restore_after_repair(window);
                    }
                    _ => {}
                }
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
            // config route, so it runs on every platform. Off-thread + best-effort so it never
            // blocks the event loop; reconcile_enabled is idempotent and only
            // writes when a tool is newly installed.
            if let WindowEvent::Focused(true) = event {
                // TRAY ONLY. This arm used to run for every window, and the
                // Linux `main` branch above it does not return, so focusing the
                // main window consumed the edge and the flag stayed true - after
                // which every tray open skipped the reconcile below until the
                // user happened to dismiss the tray with the icon. The flag is
                // cleared at the tray's hide sites, so it is the tray's flag.
                if window.label() != "tray" {
                    return;
                }
                if !POPOVER_VISIBLE.swap(true, Ordering::AcqRel) {
                    std::thread::spawn(|| {
                        if let Err(e) = gate_connect_core::provider::reconcile_enabled() {
                            eprintln!("[gate] provider config reconcile on focus failed: {e}");
                            report_backend_error("provider_reconcile", format!("{e:#}"));
                        }
                    });
                }
            }
        })
        .setup(|app| {
            // Lets failure sites without a handle of their own nudge the
            // popover to drain buffered analytics errors.
            let _ = APP_HANDLE.set(app.handle().clone());

            // Engine crash fail-safe UI: the manager reverts the system proxy
            // on its own, but it has no window handle - without this observer
            // the tray kept its green "routing on" dot and an open popover
            // kept rendering On until the user happened to reopen it, while
            // traffic already flowed direct. Repaint the tray and nudge the
            // popover with the post-crash state (mirrors the startup
            // auto-enable's emit; the frontend has no status poll by design).
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            {
                let crash_handle = app.handle().clone();
                gate_connect_core::proxy::set_engine_crash_observer(move || {
                    update_tray_status(&crash_handle, false);
                    match gate_connect_core::proxy::manager().status() {
                        Ok(state) => {
                            let _ = crash_handle.emit("proxy-state-changed", &state);
                        }
                        Err(e) => {
                            eprintln!("[gate] status after engine crash failed: {e}");
                            report_backend_error("restore_routing", format!("{e:#}"));
                        }
                    }
                });

                // ChatGPT app Cloudflare-challenge fix: the engine detects
                // `cf-mitigated: challenge` on a rewritten chatgpt.com app
                // turn (the app shell can't render the interstitial) and this
                // observer opens a one-time solve webview at the real host;
                // the captured `cf_clearance` is fed back into the engine and
                // merged into subsequent app turns. Off-thread because the
                // observer fires on the engine thread mid-response and must
                // not block it on window creation. The event mirrors
                // `proxy-state-changed` so a mounted popover can react;
                // opening the window does not depend on it.
                let challenge_handle = app.handle().clone();
                gate_connect_core::proxy::set_cf_challenge_observer(move || {
                    let _ = challenge_handle.emit("cf-challenge-required", ());
                    let handle = challenge_handle.clone();
                    std::thread::spawn(move || open_cf_challenge_window(&handle));
                });
            }

            // Dead-session detection on the data plane: the engine reports
            // a 401 the gateway gave a call we authenticated, and this
            // observer decides what it meant. Until now nothing did - the
            // 401 went to the tool that made the call and Gate kept
            // showing a green dot while every request failed, which is how
            // a resume-from-sleep clock jump (locally-fresh token, expired
            // as far as the gateway is concerned) could persist until a
            // restart.
            //
            // Off-thread: the observer fires on the engine thread
            // mid-response, and the re-verification makes its own blocking
            // HTTP calls. The `GateAuthCheck` guard releases the debounce
            // latch when the thread ends, panic included - a latch left set
            // would leave 401-driven recovery dead for the rest of the
            // process.
            //
            // Registered on every desktop OS, unlike the two observers above:
            // there is no window to create here, so nothing is
            // platform-specific, and keeping it off the cfg gate is what lets
            // it be type-checked on any of them. On Linux it simply never
            // fires - the engine lives in the helper daemon, so the notify
            // happens in a process that registered no observer.
            let auth_handle = app.handle().clone();
            gate_connect_core::proxy::set_gate_auth_observer(move || {
                let handle = auth_handle.clone();
                std::thread::spawn(move || {
                    let _release = gate_connect_core::proxy::GateAuthCheck;
                    recheck_gate_session(&handle);
                });
            });

            // Detection is the one reading a window cannot be told about. A tool
            // installed while the app is open happens entirely outside it, so
            // both shells used to poll `list_tools` every five seconds to
            // notice - twelve config-file walks a minute, forever, for an event
            // that happens a handful of times in a machine's life. The OS
            // already knows; `tool_watch` asks it and this emits.
            //
            // Registered on every desktop OS and not gated behind a window:
            // `emit` reaches whichever shells are mounted, and neither the
            // window nor the tray has to exist yet.
            //
            // A watch that cannot start is reported and then left alone. The
            // shells still re-read on their visibility edge, which is what
            // covers the paths a watch cannot see anyway - a `$PATH` install of
            // a launcher has no directory to arm.
            {
                let watch_handle = app.handle().clone();
                if let Err(e) = gate_connect_core::tool_watch::start(move || {
                    let _ = watch_handle.emit("tools-changed", ());
                }) {
                    eprintln!("[gate] tool config watch failed to start: {e:#}");
                    report_backend_error("tool_watch", format!("{e:#}"));
                }
            }

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
                    // OAuth: refresh a stale token and probe the session at
                    // the gateway before the engine seeds itself below (the
                    // policy lives in `gate_connect_core::startup`). Seed the
                    // tray attention flag from the verdict so the first tray
                    // paint in the auto-enable below is already correct,
                    // instead of showing a misleading routing dot for up to
                    // one refresh interval.
                    match gate_connect_core::startup::refresh_session() {
                        gate_connect_core::startup::SessionVerdict::Healthy => {
                            SESSION_NEEDS_SIGNIN.store(false, Ordering::Relaxed);
                        }
                        gate_connect_core::startup::SessionVerdict::NeedsSignIn => {
                            SESSION_NEEDS_SIGNIN.store(true, Ordering::Relaxed);
                        }
                        gate_connect_core::startup::SessionVerdict::NotOauth => {}
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
                    // routing is actually off, then finish the job: deregister
                    // and exit - the user asked us not to run at startup. The
                    // routing intent stays put: the opt-out governs autostart,
                    // not routing, so the next manual launch restores routing
                    // as the user left it. On macOS/Windows the reconcile
                    // above has already *reverted* any stranded system proxy;
                    // on Linux it does the opposite - it re-honors
                    // (re-enables) a leftover snapshot - so without an
                    // explicit disable here the daemon would keep intercepting
                    // headless after we exit. A manual or updater-driven
                    // launch (not --silent) skips this and restores routing as
                    // the user left it; the still-pending opt-out completes at
                    // the next safe point instead.
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
                        handle.exit(0);
                        return;
                    }

                    // Restart persistence: bring routing back if the user last
                    // left it on. The exit-time disable reverts the *system
                    // proxy* only and never touches the routing intent, so
                    // every launch - login item, manual, updater relaunch -
                    // restores routing as the user left it. No intent recorded
                    // means first run, or the user last turned routing off -
                    // stay passthrough.
                    if !gate_connect_core::proxy::intent::load_intent() {
                        return;
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
                    // The same master-ON ceremony as the routing toggle and the
                    // CLI (`routing::enable`): restore the provider selection
                    // around the engine start. Re-persisting the intent it just
                    // loaded is a harmless no-op.
                    match gate_connect_core::routing::enable() {
                        Ok((state, warnings)) => {
                            for w in warnings {
                                eprintln!(
                                    "[gate] startup auto-enable: {} failed: {:#}",
                                    w.component, w.error
                                );
                                report_backend_error(w.component, format!("{:#}", w.error));
                            }
                            mark_routing_enabled();
                            // Restore-on-any-launch means this can be the
                            // first thing to route on a machine with no login
                            // item, so it needs the same crash safety net as
                            // the routing toggle.
                            #[cfg(any(target_os = "macos", target_os = "windows"))]
                            arm_crash_safety_net(&handle);
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
            // The live security-event feed (AG-578). Spawned beside the token
            // refresh loop above because it depends on the same thing: a live
            // credential. It reads one per connect attempt through
            // `live_session`, so a token this loop replaces mid-stream is picked
            // up by the next reconnect with no coordination between the two.
            //
            // Deliberately started unconditionally, not gated on routing being
            // on. The events come from the gateway, not from local traffic, and
            // AC4 requires the feed's state to be independent of routing's - a
            // feed that only ran while routing was on would be reporting routing.
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            {
                let feed_handle = app.handle().clone();
                let feed = security_feed().clone();
                // One grouper for the process, so a burst of identical blocks
                // collapses into a single notification however many windows are
                // open. See `security_feed::notify` for why that is required
                // rather than polite.
                let grouper = std::sync::Arc::new(std::sync::Mutex::new(
                    gate_connect_core::security_feed::notify::Grouper::new(),
                ));

                // A storm ends by events *stopping*, so the moment worth
                // reporting - "and 29 more" - is precisely the moment nothing
                // arrives to drive a decision. Without this timer the count is
                // collected and never spoken, which is where this started: one
                // notification saying a request was blocked, for thirty.
                //
                // Every 10s against a 60s window, so a summary lands within ten
                // seconds of the window closing. Cheap: it walks a map that is
                // empty on an ordinary machine.
                let sweep_handle = app.handle().clone();
                let sweep_grouper = grouper.clone();
                // A plain thread, matching the token-refresh loop below rather
                // than the async runtime: `tokio` is not a direct dependency
                // here, and this wants a sleep, not a scheduler.
                std::thread::spawn(move || {
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(
                            SECURITY_SWEEP_INTERVAL_SECS,
                        ));
                        let prefs = gate_connect_core::preferences::load();
                        let due = {
                            let Ok(mut g) = sweep_grouper.lock() else {
                                // Poisoned by a panic elsewhere. Stop sweeping
                                // rather than spin: the feed itself is unaffected
                                // and the in-app pane still has every event.
                                return;
                            };
                            g.sweep(&prefs, std::time::Instant::now())
                        };
                        for notification in due {
                            fire_notification(&sweep_handle, notification);
                        }
                    }
                });
                tauri::async_runtime::spawn(async move {
                    gate_connect_core::security_feed::client::run(feed, move |update| {
                        use gate_connect_core::security_feed::Update;
                        // Failing to emit means no window is listening, which is
                        // ordinary: the feed keeps its own buffer and a window
                        // asks for it on mount.
                        match update {
                            Update::State(state) => {
                                let _ = feed_handle.emit("security-feed-state", state);
                            }
                            Update::Event(event) => {
                                let _ = feed_handle.emit("security-event", &*event);
                                notify_for_event(&feed_handle, &grouper, &event);
                            }
                        }
                    })
                    .await;
                });
            }

            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            {
                let refresh_handle = app.handle().clone();
                // Previous tick's paired clock readings, for the jump check
                // below. `None` until the first tick has one to compare with.
                let mut last_tick: Option<(std::time::Instant, std::time::SystemTime)> = None;
                // Previous reading of the helper daemon's gateway-refusal
                // counter (Linux only; see the poll at the end of the tick).
                // `None` until a tick has read one.
                #[cfg(target_os = "linux")]
                let mut last_refusals: Option<u64> = None;
                std::thread::spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(
                        gate_connect_core::oauth::REFRESH_INTERVAL_SECS,
                    ));
                    // Did the wall clock move by something other than the time
                    // that actually passed? `Instant` is monotonic and stops
                    // while the machine is suspended; `SystemTime` is the wall
                    // clock and can be stepped by the time service. When the
                    // two disagree across one tick, the machine resumed from
                    // sleep or the clock was corrected - and either way the
                    // token's `expires_at_unix`, stamped from a reading of the
                    // wall clock that no longer relates to this one, has
                    // stopped meaning anything. Renew now rather than let the
                    // gateway be the one to notice.
                    //
                    // This is the preventive half. It cannot catch a guest
                    // clock that froze *with* the machine (both readings stall
                    // together, so nothing here looks wrong while the token
                    // silently ages out); that case has no local signal at all
                    // and is what the gateway-auth observer recovers from
                    // after the fact.
                    let now_mono = std::time::Instant::now();
                    let now_wall = std::time::SystemTime::now();
                    let clock_jumped = last_tick.is_some_and(|(mono, wall)| {
                        let elapsed = now_mono.saturating_duration_since(mono);
                        now_wall.duration_since(wall).map_or(true, |moved| {
                            moved.max(elapsed) - moved.min(elapsed) > CLOCK_JUMP_TOLERANCE
                        })
                    });
                    last_tick = Some((now_mono, now_wall));
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
                    // A clock jump invalidates the local expiry test that
                    // `live_session` trusts, so renew unconditionally on the
                    // tick that saw one. Skipped once the session is known
                    // dead: a forced refresh stores whatever it gets and
                    // clears the gateway's rejection with it, which would
                    // report a refused session as signed in again.
                    if clock_jumped && !SESSION_NEEDS_SIGNIN.load(Ordering::Relaxed) {
                        eprintln!(
                            "[gate] the system clock moved out of step with elapsed time \
                             (sleep/resume or a time correction); renewing the access token"
                        );
                        if let Some(cfg) = gate_connect_core::oauth::OAuthConfig::from_build_env() {
                            if let Err(e) = gate_connect_core::oauth::force_refresh(&cfg) {
                                // Not a verdict on its own - the gateway has
                                // said nothing. Leave the session alone; the
                                // read below reports what it can.
                                eprintln!("[gate] renewing after a clock jump failed: {e:#}");
                            }
                        }
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
                        if dead
                            && gate_connect_core::preferences::load()
                                .routing_health_notifications
                        {
                            use tauri_plugin_notification::NotificationExt;
                            let _ = refresh_handle
                                .notification()
                                .builder()
                                .title("Gate Connect")
                                .body("Your session expired. Open Gate Connect to sign in again and keep routing.")
                                .show();
                        }
                    }

                    // Linux's substitute for the in-process 401 observer the
                    // other platforms register at setup. The engine lives in the
                    // helper daemon here, so a gateway refusal of our bearer is
                    // seen in another process; the daemon counts them and this
                    // asks for the count.
                    //
                    // Why this needs to exist at all: everything above is the
                    // *preventive* half, and it is blind to one case by
                    // construction. The clock-jump check compares a monotonic
                    // reading against a wall-clock one, and a guest clock that
                    // froze together with the machine stalls both, so nothing
                    // looks wrong locally while the token ages out. The
                    // gateway's refusal is the only signal that state produces.
                    //
                    // Acted on as an EDGE, not a level: the count only rises, so
                    // a move means at least one new refusal since the last look,
                    // and a tick that could not read it loses nothing. The first
                    // reading only seeds the baseline - a daemon can outlive
                    // several GUI runs, and launch-time `refresh_session` has
                    // already probed for a session that died while we were gone.
                    // A restarted daemon counts from zero again, which reads as
                    // no increase and simply re-seeds.
                    //
                    // `None` means nobody answered (routing off, so no control
                    // connection, or a failed round trip). It is not zero, and
                    // it must not overwrite the baseline with a value we never
                    // read.
                    //
                    // Runs even when the session is already known dead, matching
                    // the other platforms: the daemon's own cooldown throttles
                    // the counter to about one bump a minute, so this becomes a
                    // once-a-minute check for a session that came back some
                    // other way.
                    #[cfg(target_os = "linux")]
                    if let Some(refusals) = gate_connect_core::proxy::manager().gate_auth_refusals()
                    {
                        let refused_since_last_tick =
                            last_refusals.is_some_and(|seen| refusals > seen);
                        last_refusals = Some(refusals);
                        if refused_since_last_tick {
                            eprintln!(
                                "[gate] the helper daemon's engine reports the gateway refusing                                  our bearer; re-verifying the session"
                            );
                            recheck_gate_session(&refresh_handle);
                        }
                    }
                });
            }


            #[cfg(target_os = "macos")]
            watch_menu_bar_appearance(app.handle());

            let tray_icon = Image::from_bytes(TRAY_ICON_PNG)?;

            let tray_item = MenuItemBuilder::with_id("tray", "Quick status").build(app)?;
            let show_item = MenuItemBuilder::with_id("show", "Open Gate Connect").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "Quit Gate Connect").build(app)?;
            let menu = MenuBuilder::new(app)
                .items(&[&tray_item, &show_item, &quit_item])
                .build()?;

            TrayIconBuilder::with_id("main")
                .icon(tray_icon)
                .icon_as_template(true) // stand-in until update_tray_status paints the real icon
                .tooltip("Gate Connect") // baseline; macOS refines it to the routing state
                .menu(&menu)
                .show_menu_on_left_click(false) // left-click toggles window; right-click shows menu
                .on_menu_event(|app, event| match event.id().as_ref() {
                    // The compact popover, for platforms where the left-click
                    // path never fires: Linux trays (SNI/AppIndicator) only
                    // raise this menu, so without an entry the tray flow
                    // would be unreachable there. Onboarding calls the same
                    // surface "the compact popover for a quick status check".
                    "tray" => reveal_tray_window(app),
                    "show" => {
                        // On Linux the SNI/AppIndicator tray hands us no click
                        // rect and GNOME often never fires the left-click path,
                        // so the right-click menu is how users reach this.
                        // Either way it only reveals the window; it does not
                        // place it.
                        //
                        // Goes through `reveal_popover_window` rather than
                        // repeating show/focus inline. Three copies of this
                        // existed and only one of them carried the Linux
                        // decoration repair, so a `--silent` launch revealed
                        // from the tray came up with dead title-bar buttons.
                        reveal_popover_window(app);
                    }
                    "quit" => request_quit(app),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        // The click toggles the compact tray popover (Figma
                        // `Flows / Tray`), not the main window - the menu's
                        // "Open Gate Connect" still reveals that one. Plain
                        // hide() on every platform: the old Linux
                        // minimize-to-dismiss dance existed to keep window
                        // decorations alive across hide/show, and this window
                        // is undecorated.
                        if let Some(window) = app.get_webview_window("tray") {
                            let is_visible = window.is_visible().unwrap_or(false);
                            let is_minimized = window.is_minimized().unwrap_or(false);
                            if is_visible && !is_minimized {
                                let _ = window.hide();
                                POPOVER_VISIBLE.store(false, Ordering::Release);
                            } else {
                                reveal_tray_window(app);
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
            // launch. A synchronous show can flash an unpainted window before
            // WKWebView's first frame; the window is opaque now (config
            // `transparent: false`), so that flash is the window's own
            // background rather than a see-through hole.
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            if let Some(window) = app.get_webview_window("main").filter(|_| !silent_launch) {
                POPOVER_PINNED.store(true, Ordering::Release);

                // Centre on the primary display. Window position is not
                // persisted across launches, so every launch is a first launch
                // as far as placement goes; centring is the sane default for a
                // 1280x800 window. Within a session, hide/show keeps whatever
                // position the user chose.
                let _ = window.center();
                // Before the show, not after: the point is for the *first* map
                // to be the maximised one.
                #[cfg(target_os = "linux")]
                map_maximized_for_decorations(&window);
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
                if !UPDATER_RELAUNCHING.load(Ordering::Acquire) {
                    complete_pending_autostart_disable(app_handle);
                }
                // Exit reverts the *system proxy* only. The routing intent is
                // the user's last explicit toggle and survives every quit: the
                // next launch, however it happens, restores routing as it was
                // left. The only durable "off" is the routing switch itself
                // (proxy_disable clears the intent).
            }
            let _ = &event;
            let _ = &app_handle;
        });
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
