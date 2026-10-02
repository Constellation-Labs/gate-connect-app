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

/// How a challenge-solve attempt ended; reported to the engine so the cooldown
/// message can name the failure. Only the solve window uses it, hence the cfg.
#[cfg(any(target_os = "macos", target_os = "windows"))]
use gate_connect_core::proxy::SolveOutcome;

use std::collections::HashMap;
use std::path::PathBuf;
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
    /// The ledger row's label, under a heading that already names the vendor -
    /// one word, and two tools can share it.
    name: String,
    /// The product name, for a reader that is a flat list rather than a grouped
    /// ledger. Distinct across the registry, which `name` is not.
    product_name: String,
    upstream_provider_name: String,
    default_upstream_url: String,
    /// The file Gate rewrites for this tool, for the copy that says what is
    /// about to change. `None` where no single file names it.
    config_location: Option<String>,
    status: StatusDto,
    /// What Gate can and cannot see of this tool's upstream, or `None` when
    /// there is nothing to report - which is every tool but Hermes and
    /// OpenClaw, and those two whenever their provider is covered.
    ///
    /// Beside `status` rather than inside it, deliberately. Routed and
    /// inspected are two different questions, and folding this into the tool's
    /// `Status` would make it a fault the repair path tries to clear:
    /// `provider::reconcile_unmapped_tools` retries a drifted tool on every
    /// pass, and no re-connect can turn on a domain or invent a catalog entry.
    /// `openclaw.rs` records that trap from the last time it was walked into.
    ///
    /// Recomputed per poll, because both halves move: the user repoints the
    /// tool, or a domain is flipped elsewhere. AG-932.
    coverage: Option<gate_connect_core::coverage::UpstreamCoverage>,
    /// The program this row is aimed at: the ledger's grouping key, shared
    /// with `ProxyDomain::client` so a tool row and a domain row aimed at the
    /// same program land under one heading.
    client: gate_connect_core::taxonomy::Client,
    /// How much of the machine this row reaches. `Client` for every config
    /// tool; `Machine` for the environment channel, which is the one row here
    /// whose reach is wider than the program it names.
    scope: gate_connect_core::taxonomy::Scope,
    /// Whose credential rides the traffic. `Brokered` for every config tool -
    /// carried anyway so the UI reads one field whatever kind of row it has,
    /// rather than knowing that tool rows are brokered and domain rows may not
    /// be.
    credential: gate_connect_core::taxonomy::Credential,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StatusDto {
    NotInstalled,
    Detected,
    Connected,
    Drifted {
        reason: String,
    },
    /// Gate's values are on disk and something the tool ranks higher decides
    /// where its traffic goes. `source` names that layer, because a status line
    /// saying the route is not ours has to say where to go and look (AG-674).
    Overridden {
        source: String,
    },
    Error {
        message: String,
    },
}

impl From<Status> for StatusDto {
    fn from(s: Status) -> Self {
        match s {
            Status::NotInstalled => StatusDto::NotInstalled,
            Status::Detected => StatusDto::Detected,
            Status::Connected => StatusDto::Connected,
            Status::Drifted(reason) => StatusDto::Drifted { reason },
            Status::Overridden(source) => StatusDto::Overridden { source },
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
            // of a tool's name - the CLI, the logs, the quit notice - wants
            // `display_name`, which is why the two are separate.
            name: integ.row_label().to_string(),
            // And the product name beside it, for the readers that are a flat
            // list rather than a grouped ledger: the reopen flow's dialogs and
            // its banner. `registry` has a test asserting display
            // names are distinct precisely because such a list cannot tell two
            // "CLI"s apart.
            product_name: integ.display_name().to_string(),
            upstream_provider_name: integ.upstream_provider_name().to_string(),
            default_upstream_url: integ.default_upstream_url().to_string(),
            config_location: integ.config_location(),
            status: status_for(integ.as_ref()),
            coverage: integ.upstream_coverage(),
            client: integ.client(),
            scope: integ.scope(),
            credential: integ.credential(),
        })
        .collect()
}

/// Every visible tool's version, keyed by slug.
///
/// A separate command from [`list_tools`] on purpose, and the separation is the
/// design: `list_tools` feeds the sidebar and repaints constantly, while this
/// spawns one process per tool on a cold cache. Wiring versions into the DTO
/// would put five `--version` calls on every repaint.
///
/// Absent from the map where the binary could not be found; present-but-null
/// where it was found and would not say. The report prints those differently,
/// because "not installed here" and "installed, version unreadable" send a
/// support thread to different places.
#[tauri::command]
fn tool_versions() -> std::collections::HashMap<String, Option<String>> {
    registry::registry()
        .iter()
        .filter(|integ| !integ.hidden_in_ui())
        .filter_map(|integ| {
            let (well_known, names) = integ.binary();
            if names.is_empty() {
                return None;
            }
            // Resolved here rather than inside the probe so "no binary found"
            // can drop out of the map entirely, while "found, would not say"
            // stays as an explicit null.
            let path =
                gate_connect_core::integrations::binaries::resolve_binary(well_known, names)?;
            Some((
                integ.id().to_string(),
                gate_connect_core::integrations::binaries::version_at(&path),
            ))
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
async fn connect_tool(slug: String) -> Result<StatusDto, String> {
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
        }
        let input = ConnectInput {
            gateway_base_url: account.gateway_base_url,
            billing_mode: account.billing_mode,
            relay_base_url: gate_connect_core::proxy::relay_base_url(),
            engine_proxy_url: gate_connect_core::proxy::tool_proxy_url(),
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

/// Async with the work on the blocking pool: the reconcile can end in
/// `oauth::clear`, whose identity forget waits on the analytics identity lock,
/// so on the main thread a save holding it, or a CLI logout, would stall the UI.
#[tauri::command]
async fn get_account() -> Result<Option<AccountDto>, String> {
    tauri::async_runtime::spawn_blocking(read_account)
        .await
        .map_err(|e| format!("get_account join error: {e}"))?
}

fn read_account() -> Result<Option<AccountDto>, String> {
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
        // And stop the environment forwarder. It is deliberately left running
        // across a plain routing-off - that is exactly when the processes
        // holding our exported variables still need it - so this path and the
        // CA untrust are the only places it is retired. This is the one an
        // ordinary user reaches: it is what the Reset button runs
        // (`App.tsx`'s `forget`, which disables routing and then calls here).
        gate_connect_core::proxy::forwarder::stop();
        account::clear().map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| format!("sign-out join error: {e}"))?;
    // What changed: the account is gone.
    if done.is_ok() {
        // Same reason as the org switch: the events belonged to the account
        // being cleared, and the feed outlives it.
        security_feed().reset_for_account_change();
        signal_session_changed();
    }
    // `account::clear` forgot the analytics identity (through `oauth::clear`);
    // tell every window, so none keeps the account that went (AG-960).
    announce_stored_analytics_identity();
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
    /// The id token's Cognito `sub`, which the webview identifies its PostHog
    /// person with once signed in so the install funnel joins the dashboard's
    /// person (AG-960). Not shown anywhere.
    sub: Option<String>,
    /// `"live"`, `"signed_out"` (no session, or a refused one) or
    /// `"unavailable"` (the identity provider or the secret store did not
    /// answer). `signed_in` is false for both of the last two, as it always was;
    /// this is what lets the analytics seam tell a sign-out from a machine that
    /// is only offline (AG-960).
    session: &'static str,
    /// Access-token expiry as a Unix timestamp; 0 when signed out.
    expires_at_unix: i64,
}

impl From<&gate_connect_core::oauth::OAuthTokens> for OAuthStatusDto {
    fn from(t: &gate_connect_core::oauth::OAuthTokens) -> Self {
        Self {
            signed_in: true,
            email: t.email(),
            sub: t.sub(),
            session: "live",
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
    use gate_connect_core::oauth::SessionReading;
    Ok(match gate_connect_core::oauth::session_reading() {
        SessionReading::Live(t) => OAuthStatusDto::from(&t),
        other => OAuthStatusDto {
            signed_in: false,
            email: None,
            sub: None,
            session: if matches!(other, SessionReading::Unavailable) {
                "unavailable"
            } else {
                "signed_out"
            },
            expires_at_unix: 0,
        },
    })
}

/// Run one interactive Cognito login: open the Hosted UI in the browser and
/// capture the redirect on a loopback listener. Blocks (off the main thread)
/// until the user finishes signing in or the flow times out.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
async fn oauth_begin_login<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<OAuthStatusDto, String> {
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
        // The engine's key goes with it: an account that pasted one before
        // this sign-in would otherwise keep it there, and a later dead session
        // would be served under it rather than refused (`lacks_gate_credential`).
        gate_connect_core::account::set_auth_mode(gate_connect_core::account::AuthMode::OAuth)
            .map_err(|e| format!("{e:#}"))?;
        // Whatever ended the last session, this one is live - so the flag stops
        // describing anything. Best-effort: a preferences write that fails must
        // not fail a sign-in that succeeded, and the cost of it going unwritten
        // is one welcome pane that would have said the wrong word, on a screen
        // the user is no longer looking at.
        let _ = gate_connect_core::preferences::set_signed_out_deliberately(false);
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            gate_connect_core::proxy::manager().refresh_api_key("");
            gate_connect_core::proxy::manager().refresh_token(&tokens.access_token);
        }
        Ok(OAuthStatusDto::from(&tokens))
    })
    .await
    .map_err(|e| format!("login join error: {e}"))?
}

/// Stop an interactive login that is still waiting for the browser.
///
/// The offer dialog's own decline calls this: the flow waits five minutes for a
/// callback, and the usual reason it never comes is the sign-in page opening in
/// a browser profile the person is not signed into. Without this the dialog
/// could only stop *showing* the wait - the login would go on, and a late
/// success would upgrade an account the user had just declined to upgrade.
///
/// Not async and not blocking: it sets a flag the login's own poll loop reads.
#[tauri::command]
fn oauth_cancel_login() {
    gate_connect_core::oauth::cancel_login();
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
        // Say that this was asked for. The state left behind is identical to an
        // expired session - no token, `auth_mode` still OAuth by design two
        // lines up - so without this the welcome pane can only guess, and it
        // guessed "Session expired" over the user's own deliberate click.
        // Best-effort and after the credential, on the same reasoning as the
        // cache clear below: a preferences write must not be the reason a
        // sign-out reports failure.
        let _ = gate_connect_core::preferences::set_signed_out_deliberately(true);
        // The held activity readings belong to the org just signed out of, and
        // signing out is not a disconnect: `account.json` keeps the gateway and
        // the org, so `activity_cache`'s scope stays byte-identical and every
        // reading in it would still match and still be served. `account::clear`
        // covers the disconnect path; this covers the one that leaves the account
        // file in place. Best effort and after the credential, on the same
        // reasoning as there: a cache that will not delete must not be the reason
        // a sign-out reports failure.
        let _ = gate_connect_core::activity_cache::clear();
        // Take the token out of a running engine now. The account stays in
        // OAuth mode and holds no key, so routed requests are refused as
        // signed out rather than sent under anything.
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
    // `oauth::clear` forgot the analytics identity; tell every window (AG-960).
    announce_stored_analytics_identity();
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
        gate_connect_core::account::set_auth_mode(mode).map_err(|e| format!("{e:#}"))?;
        // An OAuth account holds no key (`account::load`), so neither may a
        // running engine: see `oauth_begin_login`.
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        if oauth {
            gate_connect_core::proxy::manager().refresh_api_key("");
        }
        Ok(())
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
        // Configs first: a tool moved off Gate models from inside itself is put
        // back on its own model here, and the stored choices read below must
        // already say so.
        let configured = gate_connect_core::tool_models::states();
        let prefs = gate_connect_core::preferences::load();
        ToolModelsDto {
            tools: prefs
                .tool_models
                .into_iter()
                .map(|(slug, choice)| (slug, ToolModelChoiceDto::from(choice)))
                .collect(),
            paid_ack_unix: prefs.gate_model_paid_ack_unix,
            configured: configured
                .into_iter()
                .map(|(slug, view)| (slug.to_string(), ConfiguredModelDto::from(view)))
                .collect(),
        }
    })
    .await
    .map_err(|e| format!("tool model preferences join error: {e}"))
}

/// What one tool's own config says about Gate models (R3: the config, not the
/// stored choice, is what the tool will run).
#[derive(Serialize)]
struct ConfiguredModelDto {
    /// `"applied"`, `"not_applied"` or `"drifted"`. Drift has normally been
    /// resolved by the time this is read, so `"drifted"` means the resolution
    /// itself failed.
    state: &'static str,
    /// The model the config starts the tool on, when it is on Gate models.
    model: Option<String>,
    /// True when this read found the tool moved off Gate models from inside the
    /// tool, and put it back on its own model. The window says so once.
    left_gate_models: bool,
    /// With `left_gate_models`: the model the tool's config names now, if any.
    left_to_model: Option<String>,
    /// Why the card cannot trust this reading: the config was unreadable, or
    /// the tool could not be put back on its own model. Null when all is well.
    problem: Option<String>,
}

impl From<gate_connect_core::tool_models::ToolModelView> for ConfiguredModelDto {
    fn from(v: gate_connect_core::tool_models::ToolModelView) -> Self {
        use gate_connect_core::registry::GateModelState;
        let (state, model) = match v.state {
            GateModelState::Applied { model } => ("applied", Some(model)),
            GateModelState::Drifted { model } => ("drifted", model),
            GateModelState::NotApplied | GateModelState::Unsupported => ("not_applied", None),
        };
        Self {
            state,
            model,
            left_gate_models: v.left_gate_models.is_some(),
            left_to_model: v.left_gate_models.flatten(),
            problem: v.problem,
        }
    }
}

#[derive(Serialize)]
struct ToolModelsDto {
    /// Keyed by tool slug. A tool with no entry is on its own default, which is
    /// why an absent key is not the same as an error and needs no placeholder.
    tools: std::collections::BTreeMap<String, ToolModelChoiceDto>,
    /// Unix seconds, or null when this install has never accepted paid use.
    paid_ack_unix: Option<i64>,
    /// Keyed by tool slug, for the tools that support Gate models.
    configured: std::collections::BTreeMap<String, ConfiguredModelDto>,
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
///
/// The choice is written into the tool's own config when Gate manages it, and
/// the answer says whether it was: `true` means the file changed and the tool
/// picks it up on its next session, which is the window's cue to offer the
/// restart notice.
#[tauri::command]
async fn set_tool_model(
    tool: String,
    source: String,
    model_ids: Vec<String>,
    acknowledge_paid_use: bool,
) -> Result<bool, String> {
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
        // The names and context windows the tool's own picker will show, read
        // now because the connect that writes them may run offline later. A
        // catalogue that cannot be read costs the picker its labels, not the
        // choice.
        let meta = match source {
            gate_connect_core::preferences::ModelSource::Gate => {
                gate_connect_core::gate_models::catalogue_json()
                    .map(|json| {
                        gate_connect_core::tool_models::meta_from_catalogue(&json, &model_ids)
                    })
                    .unwrap_or_default()
            }
            gate_connect_core::preferences::ModelSource::Tool => Vec::new(),
        };
        let applied = gate_connect_core::tool_models::choose(
            tool,
            source,
            model_ids,
            acknowledge_paid_use,
            meta,
        )
        .map_err(|e| format!("{e:#}"))?;
        // Codex's app-server daemon reads its config and model catalog only
        // when it starts; see `refresh_codex_daemon_when_idle`.
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        if applied && tool == gate_connect_core::registry::ToolId::Codex {
            refresh_codex_daemon_when_idle("set model");
        }
        Ok(applied)
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
        // The feed included. It is a process singleton, so without this its
        // buffer, its dedupe set and its catch-up verdict all survive the
        // switch - and `security_feed_recent` hands the previous org's events
        // straight back to a window that has just cleared its own copy. That is
        // the one thing the activity surfaces are not allowed to do: show one
        // org's traffic under another org's name.
        //
        // `reset_for_account_change` was written for exactly this and had no
        // production caller at all, only a test, so the guarantee its name makes
        // was never kept.
        security_feed().reset_for_account_change();
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

/// OS marketing name AND version ("Ubuntu 25.10", "macOS 15.3 (24D60)").
///
/// Separate from [`diagnostics`], which returns this among fifteen other
/// fields at a cost that module's own doc warns about. The analytics error
/// context reads this once at startup, where the distro or point release is
/// often the whole answer - the AppImage's Wayland loader problem is an
/// Ubuntu-version question, and trust-store behaviour moves between macOS
/// builds Apple ships under one marketing version.
#[tauri::command]
fn os_name() -> String {
    gate_connect_core::diagnostics::os_name()
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

/// Read the browser stores, once, for the note the window raises when
/// trust is granted somewhere this process could not see it happen.
///
/// Async for the same reason `proxy_status` is, and more so: this is the call
/// that deliberately shells out to `certutil`, once or twice per NSS database.
/// The window asks for it on a transition and never on a poll - see
/// `proxy::probe_browser_store`.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
async fn proxy_browser_store() -> Result<Option<gate_connect_core::proxy::NssTrust>, String> {
    tauri::async_runtime::spawn_blocking(gate_connect_core::proxy::probe_browser_store)
        .await
        .map_err(|e| format!("browser store probe join error: {e}"))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
async fn proxy_enable<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
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
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        refresh_codex_daemon_when_idle("routing on");
        for w in warnings {
            eprintln!("[gate] proxy enable: {} failed: {:#}", w.component, w.error);
            report_backend_failure(w.component, &w.error);
        }
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
fn arm_crash_safety_net<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
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
                    } else {
                        // A fresh LaunchAgent, so it carries none of our policy.
                        #[cfg(target_os = "macos")]
                        arm_crash_restart(app);
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
async fn proxy_disable<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
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
            report_backend_failure(w.component, &w.error);
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
fn complete_pending_autostart_disable<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
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
fn launch_at_login_status<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<LaunchAtLoginStatus, String> {
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

/// Re-apply the crash-restart policy to the LaunchAgent.
///
/// That file belongs to `tauri-plugin-autostart`, which writes it from a fixed
/// template and truncates whatever was there, so every enable erases the
/// policy. Hence this runs after each one and again at startup, which also
/// picks up an install that had launch-at-login on before this feature
/// existed. `arm` is idempotent and treats a missing plist as nothing to do,
/// so it is always safe to call.
///
/// Best-effort: failing to arm costs the next crash its relaunch, which is the
/// behaviour every build before this one had.
#[cfg(target_os = "macos")]
fn arm_crash_restart<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use gate_connect_core::crash_restart;
    let result = crash_restart::launch_agent_plist(&app.package_info().name)
        .and_then(|plist| crash_restart::arm(&plist));
    if let Err(e) = result {
        eprintln!("[gate] arming crash restart failed: {e:#}");
    }
}

/// Stop asking launchd to bring us back, leaving the rest of the LaunchAgent
/// alone so the user's launch-at-login choice survives. Called when the streak
/// of launches that never got off the ground hits its ceiling; a clean exit
/// clears that streak and the next startup arms again.
#[cfg(target_os = "macos")]
fn disarm_crash_restart(app: &tauri::AppHandle, streak: u32) {
    use gate_connect_core::crash_restart;
    let result = crash_restart::launch_agent_plist(&app.package_info().name)
        .and_then(|plist| crash_restart::disarm(&plist));
    match result {
        Ok(_) => report_backend_error(
            "crash_restart",
            format!("automatic restart after a crash is off: {streak} launches in a row ended before the app was up"),
        ),
        Err(e) => eprintln!("[gate] disarming crash restart failed: {e:#}"),
    }
}

/// Ask Windows Error Reporting to relaunch us after a crash or a hang.
///
/// The macOS half of this is a file; Windows has none, and this one call is
/// the whole mechanism. The flags are subtractive, and the two we pass narrow
/// it to the cases we want: an installer patch or a system reboot must not
/// bring the app back on its own, because neither is a crash and the user's
/// launch-at-login setting already answers whether it starts at boot.
///
/// `--silent` so the relaunch comes up in the tray the way a login launch
/// does, rather than throwing a window in front of whatever the user is doing.
#[cfg(target_os = "windows")]
fn register_application_restart() {
    // Declared rather than pulled from a crate: it is one function in
    // kernel32, which std already links on this target.
    extern "system" {
        fn RegisterApplicationRestart(pwz_commandline: *const u16, dw_flags: u32) -> i32;
    }
    /// Do not restart after an installer patch.
    const RESTART_NO_PATCH: u32 = 4;
    /// Do not restart after a system reboot.
    const RESTART_NO_REBOOT: u32 = 8;

    let mut command_line: Vec<u16> = "--silent".encode_utf16().collect();
    command_line.push(0);
    // SAFETY: a documented kernel32 entry point, passed a null-terminated
    // UTF-16 buffer that outlives the call and a flags word. It only records a
    // preference with the OS and touches nothing of ours.
    let hr = unsafe {
        RegisterApplicationRestart(command_line.as_ptr(), RESTART_NO_PATCH | RESTART_NO_REBOOT)
    };
    if hr != 0 {
        eprintln!("[gate] registering automatic restart failed (hresult {hr:#x})");
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn set_launch_at_login<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    enabled: bool,
) -> Result<(), String> {
    use gate_connect_core::proxy::autostart_optout;
    use tauri_plugin_autostart::ManagerExt;
    // This marker/login-item read-then-write and `arm_crash_safety_net`
    // share no lock; see the accepted-race note on that function.
    let mgr = app.autolaunch();
    if enabled {
        autostart_optout::set_pending(false).map_err(|e| format!("{e:#}"))?;
        mgr.enable().map_err(|e| format!("{e:#}"))?;
        // The plugin has just rewritten the LaunchAgent from its template,
        // taking the crash-restart policy with it. Put it back.
        #[cfg(target_os = "macos")]
        arm_crash_restart(&app);
        Ok(())
    } else if autostart_optout::record_disable().map_err(|e| format!("{e:#}"))? {
        mgr.disable().map_err(|e| format!("{e:#}"))
    } else {
        // Routing is on: the opt-out is deferred and the login item stays
        // registered until the next safe point.
        Ok(())
    }
}

/// What Gate would and would not see of this Hermes install right now.
///
/// Hermes is one of only two rows whose upstream is chosen by the user and
/// lives in a *different* section. `groups.ts` shows why: `claude` bundles
/// `["claude-code", "anthropic", "claude-web"]` and `chatgpt` bundles its
/// own provider rows, so one switch turns on the tool and intercepts what the
/// tool talks to. `hermes` is `["hermes"]`, and `openclaw` is `["openclaw"]`.
/// Turning Hermes on therefore routes Hermes and inspects nothing, and the app
/// reports Protected while every request tunnels past unseen.
///
/// `gate-connect connect hermes` has printed exactly this to stderr since the
/// integration was written. The window has never had it. This command is that
/// reading, and nothing else: it enables no domain and writes no file, so the
/// objection recorded on `integrations::hermes::Coverage` - that a tool must
/// not widen the machine's interception as a side effect - still holds. What
/// the caller does with the answer is ask.
///
/// Uncached deliberately. Both inputs move after the toggle: the person
/// repoints Hermes, or a domain flips elsewhere. Removing and re-trusting a
/// certificate reset `openrouter` to off on the machine that prompted this,
/// hours after Hermes had been connected.
///
/// The reading itself cannot fail - a missing or unparseable `config.yaml` is
/// reported as Hermes' default and marked `defaulted`, not returned as an
/// error - so the only `Err` here is the worker failing to join.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
async fn hermes_upstream_coverage(
) -> Result<gate_connect_core::integrations::hermes::Coverage, String> {
    // Off the main thread, like `connect_tool`: this reads `config.yaml` and the
    // domain catalog, and a sync command runs on the thread the webview waits on.
    tauri::async_runtime::spawn_blocking(gate_connect_core::integrations::hermes::upstream_coverage)
        .await
        .map_err(|e| format!("coverage join error: {e}"))
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

/// When the popover last dismissed itself on losing focus, as milliseconds
/// since the epoch (0 = never).
///
/// This exists for one race. Clicking the tray icon while the popover is open
/// blurs it first, so the blur-dismiss below has already hidden the window by
/// the time the icon's own click arrives - and that handler, seeing a hidden
/// window, would helpfully re-open it. The icon would then be unable to close
/// the popover at all. A click within [`BLUR_HIDE_GRACE_MS`] of a blur-dismiss
/// is therefore treated as the second half of that dismissal rather than as a
/// request to re-open.
///
/// Only stamped when the popover was still VISIBLE as focus left, which is what
/// keeps the invariant true: a hide performed on purpose (Escape, Expand app,
/// Switch organization, Quit) blurs the window too, and stamping there would
/// eat the next tray-icon click rather than the one that caused the blur.
///
/// Wall clock rather than `Instant` because it has to live in an atomic. The
/// comparison is a short grace window, so a clock step can only mean one click
/// opens when it would have closed.
static BLUR_HIDE_AT_MS: AtomicU64 = AtomicU64::new(0);

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
/// webview alive on purpose - detection polling and the backend-error drain
/// both live in it - so it cannot simply be created on demand.
#[cfg(target_os = "linux")]
fn map_maximized_for_decorations<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
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
fn restore_after_repair<R: tauri::Runtime>(window: &tauri::Window<R>) {
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
fn poll_restore_after_repair<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
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
/// user's quit, but an update install relaunches us immediately, and the
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
/// On macOS and Windows an engine move counts only when a tool's config still
/// names the old port; everything else names the forwarder, which follows it.
static ROUTED_CLIENTS_MAY_BE_STALE: AtomicBool = AtomicBool::new(false);

/// Whether already-running routed clients may be pointing at a dead port
/// (see [`ROUTED_CLIENTS_MAY_BE_STALE`]). Read-only; the frontend keeps its
/// own dismissed state for the webview's lifetime.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn routed_clients_stale() -> bool {
    ROUTED_CLIENTS_MAY_BE_STALE.load(Ordering::Acquire)
}

/// Whether the startup thread's auto-enable has yet to settle, one way or the
/// other. Lets the tray tell "still starting" from "didn't start": both read
/// `running: false`, and printing "Didn't start" over an enable that is merely
/// in flight reports a failure at the one moment the user is looking.
///
/// Starts `true` rather than being set before the spawn, because Tauri builds
/// the config windows before `setup` runs - a webview's first read can land
/// before any line of `setup` does. [`StartupEnableSettled`] is the only thing
/// that clears it, on every way out of the startup thread.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
static STARTUP_ENABLE_PENDING: AtomicBool = AtomicBool::new(true);

/// Whether the startup auto-enable is still in flight (see
/// [`STARTUP_ENABLE_PENDING`]).
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn routing_startup_pending() -> bool {
    STARTUP_ENABLE_PENDING.load(Ordering::Acquire)
}

/// Settles [`STARTUP_ENABLE_PENDING`] when the startup thread ends, whichever
/// arm it ends in - enabled, failed, no account, or a panic - and nudges both
/// shells to re-read. A drop guard because the failure arm emits nothing of its
/// own: without the nudge a tray that read "starting" would keep saying so.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
struct StartupEnableSettled<R: tauri::Runtime>(tauri::AppHandle<R>);

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
impl<R: tauri::Runtime> Drop for StartupEnableSettled<R> {
    fn drop(&mut self) {
        STARTUP_ENABLE_PENDING.store(false, Ordering::Release);
        let _ = self.0.emit("proxy-state-changed", ());
    }
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
    /// The connection-failure reason decided from the error's TYPE, where the
    /// failure site had the error itself rather than only its text
    /// (`gate_connect_core::analytics::failure_reason`). `None` leaves the
    /// webview to classify the message, as before.
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
}

/// Buffered failures, **per webview label**, because both shells drain.
///
/// This was one shared `Vec` and the drain took it (`std::mem::take`). That was
/// correct while exactly one shell drained; the tray gained a drain and the two
/// began racing over a single take, with the loser getting `[]`. Both webviews
/// are mounted from launch - `tauri.conf.json` declares `main` and `tray`
/// created hidden and never destroyed - so the race is live on every report, and
/// `report_backend_error` broadcasts the nudge to both.
///
/// Two ways that showed. A resume failure raised in the popover could be taken
/// by the hidden main window, leaving the tray to redraw an identical card and
/// say nothing, which is the dead button this was meant to fix. And a startup
/// failure could be taken by the hidden *tray*, where nothing cleared
/// `actionError`, so it surfaced later as an unexplained banner over a popover
/// the user had just opened while the foreground window showed nothing.
///
/// A copy per label fixes **the first**: each shell drains only what was queued
/// for it, and neither can consume the other's. Keyed by label rather than by a
/// cursor because the buffer evicts its oldest entry at the cap, and a per-shell
/// index into a shifting `Vec` is a second thing to get wrong.
///
/// It does not fix the second, and on its own it made it certain rather than
/// intermittent: the hidden tray is now queued a copy of *every* routing-down
/// failure by construction, where before it had to win a race for one. What
/// closes that is the popover clearing its own banner when it hides
/// (`TrayApp.tsx`, the `document.hidden` edge) - a display decision, made where
/// the display is. This buffer's job is only to stop the two shells fighting
/// over one `Vec`.
///
/// Analytics is deliberately NOT duplicated with the display copies. Both
/// shells drain, but only `main` forwards to the analytics seam
/// (`forwardBackendErrors`'s `reportToAnalytics`); a copy per label would
/// otherwise emit two `error_shown` events per failure, one of them from a
/// webview where nothing was shown.
static PENDING_BACKEND_ERRORS: Mutex<Option<HashMap<String, Vec<BackendError>>>> = Mutex::new(None);

/// The webviews that drain. A label not listed here queues nothing, which is
/// what keeps a transient window from accumulating a buffer nobody reads.
const ERROR_SINK_LABELS: [&str; 2] = ["main", "tray"];
/// Set once in `setup`; lets failure sites without an AppHandle (threads,
/// spawn_blocking closures) nudge the popover.
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

/// Queue a failure for the frontend analytics seam and nudge a mounted
/// popover to drain it. Capped so a repeating failure can't grow unbounded;
/// oldest entries drop first.
fn report_backend_error(context: &'static str, message: String) {
    queue_backend_error(context, message, None);
}

/// [`report_backend_error`] for a failure whose error value is in hand, so the
/// reason it is filed under in analytics comes from its type (an `AddrInUse`
/// anywhere in the chain) rather than from matching words in the message.
fn report_backend_failure(context: &'static str, err: &anyhow::Error) {
    queue_backend_error(
        context,
        format!("{err:#}"),
        gate_connect_core::analytics::failure_reason(err),
    );
}

fn queue_backend_error(context: &'static str, message: String, reason: Option<&'static str>) {
    if let Ok(mut guard) = PENDING_BACKEND_ERRORS.lock() {
        let per_label = guard.get_or_insert_with(HashMap::new);
        for label in ERROR_SINK_LABELS {
            let pending = per_label.entry(label.to_string()).or_default();
            if pending.len() >= 32 {
                pending.remove(0);
            }
            pending.push(BackendError {
                context,
                message: message.clone(),
                reason,
            });
        }
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

/// Take one label's buffered failures, leaving every other label's alone.
///
/// Split out of [`drain_backend_errors`] so the test can exercise this exact
/// code rather than a copy of it: a unit test cannot build a `tauri::Window`,
/// and a local reimplementation of the take would stay green through a
/// regression in the real command - a `mem::take` over the whole map, say.
fn drain_for_label(label: &str) -> Vec<BackendError> {
    PENDING_BACKEND_ERRORS
        .lock()
        .ok()
        .and_then(|mut guard| {
            guard
                .as_mut()
                .and_then(|per_label| per_label.get_mut(label).map(std::mem::take))
        })
        .unwrap_or_default()
}

/// Hand the calling window its buffered backend failures and clear ITS copy.
///
/// Scoped to `window.label()`: see [`PENDING_BACKEND_ERRORS`]. A drain from a
/// label that queues nothing returns empty rather than stealing another
/// shell's, which is the behaviour that matters if a third window ever calls
/// this. The label comes from the window tauri resolved for the invoke, not
/// from the payload, so one webview cannot name another's.
#[tauri::command]
fn drain_backend_errors<R: tauri::Runtime>(window: tauri::Window<R>) -> Vec<BackendError> {
    drain_for_label(window.label())
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
///
/// The third column is the tool's product name, and it is here because two of
/// these slugs are *not* registry tool ids. `list_tools` is where every surface
/// gets a product name from, and it has no `anthropic` or `chatgpt` row - so a
/// process scan that reported only the OS name left the reopen flow drawing
/// rows titled `Claude` or, where a caller fell back to the key, `anthropic`.
/// Naming them beside the process is the only place that cannot drift from the
/// row it names.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
const AGENT_PROCESSES: [(&str, &str, &str, Surface); 6] = [
    ("claude-code", "claude", "Claude Code", Surface::Cli),
    ("codex", "codex", "Codex", Surface::Cli),
    ("opencode", "opencode", "OpenCode", Surface::Cli),
    // Hermes is a Python program: its launcher execs the venv's `python` with
    // the `hermes` script, so no process is *named* `hermes`. `agent_name_of`
    // resolves it from the command line (see `is_hermes_command`). It needs a
    // row now that Gate writes its model into `config.yaml`, which Hermes reads
    // at startup - the restart notice after a model change has to find it.
    ("hermes", "hermes", "Hermes", Surface::Cli),
    // The desktop apps. Their slugs are proxy-domain keys rather than registry
    // tool ids, because that is what these are: Gate routes them through the
    // system proxy, not by rewriting a config file. `agent_names_for`'s doc
    // already anticipated being asked about a proxy domain key - it just had
    // nothing to answer with until now.
    //
    // Case matters and is not incidental: `Claude` here is the desktop app,
    // `claude` above is the CLI, and `agent_name_of` deliberately does not fold
    // them together. Confirmed with the product.
    ("anthropic", "Claude", "Claude Desktop", Surface::App),
    // **This row covers Cowork too, and the ChatGPT row below covers Work.**
    //
    // Cowork is a mode inside the Claude desktop app, not an app of its own -
    // same process, same host, same switch - and Work is the same thing inside
    // the ChatGPT app. So neither needs a row, and adding one would be adding a
    // name no process ever answers to.
    //
    // Spelled out because the tree has been wrong about this twice, in opposite
    // directions, and the second error is the one that looks correct:
    //
    // - A Cowork row was added here once, on the reading that it was a separate
    //   desktop app (a Windows spelling of Claude Desktop). It is not.
    // - It was then deleted on the reading that Cowork *is* the ChatGPT app,
    //   because `engine.rs` carried a captured turn to
    //   `chatgpt.com/backend-api/codex/responses` labelled "Cowork's". That
    //   capture is Work's, and Work belongs to ChatGPT - see `work_upgrade`,
    //   which used to be `cowork_upgrade` and is the whole origin of the
    //   confusion. So "there is no Cowork process" was right, and the reason
    //   given for it was wrong, and it pointed at the wrong row.
    //
    // The surviving consequence of that second error is worth knowing: it left
    // a note claiming `provider.rs` and `GroupMembers.tsx` might be wrong to
    // label the *anthropic* switch "Claude Desktop / Cowork". They are not.
    // That is this row, and Cowork rides it.
    //
    // `ChatGPT` on Windows too, where `.exe` is stripped before the match.
    // Confirmed with the product.
    ("chatgpt", "ChatGPT", "ChatGPT", Surface::App),
];

/// Whether Gate may relaunch a process it closed.
///
/// The distinction is not cosmetic and not about how the tool is routed - it is
/// about what "reopen" can honestly mean.
///
/// A [`Surface::Cli`] runs inside a shell session Gate does not own, with a
/// working directory, arguments and a conversation this process cannot see.
/// Spawning its binary again would not reopen it; it would start a different
/// one, somewhere else, detached from the terminal the user was working in - and
/// the session they agreed to close would be gone with nothing put back.
///
/// A [`Surface::App`] owns its own window and its own state. Launching it again
/// is exactly what the user would have done by hand, which is what makes closing
/// it defensible in the first place: Gate is not ending their session, it is
/// restarting an application so it picks up the route.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Surface {
    Cli,
    App,
}

/// The process names to scan for. `None` means every tool - the master toggle,
/// the popover's routing takeover and the diagnostics listing all genuinely
/// mean all of them. `Some(slugs)` narrows to the tools whose configs were
/// just rewritten; slugs with no process of their own (`openclaw`,
/// `env-proxy`, a proxy domain key) drop out, and `Some(&[])` scans nothing.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn agent_names_for(only: Option<&[String]>) -> Vec<&'static str> {
    AGENT_PROCESSES
        .iter()
        .filter(|(slug, _, _, _)| only.is_none_or(|slugs| slugs.iter().any(|s| s == slug)))
        .map(|(_, name, _, _)| *name)
        .collect()
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
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
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
    // Candidates by name alone, in any case. The row a process belongs to can
    // depend on its executable path (`agent_row_of`), and the walk above does
    // not read it: on Windows `sysinfo` fills `exe` only when asked, so
    // resolving rows here saw `None` for every process, and a Claude desktop
    // app on Windows - which reports itself as `claude.exe` - read as the CLI.
    // Case-folded so `Claude` and `claude` both survive to be told apart below.
    let candidates: Vec<sysinfo::Pid> = sys
        .processes()
        .iter()
        .filter(|(pid, process)| {
            let name = agent_name_of(process);
            // An interpreter is a candidate only when Hermes is asked about:
            // this pass has no command lines, and the one below reads them for
            // candidates alone, which is what tells Hermes from other Python.
            Some(**pid) != own_pid
                && (AGENT_PROCESSES
                    .iter()
                    .any(|(_, n, _, _)| n.eq_ignore_ascii_case(&name))
                    || (names.contains(&"hermes") && is_python_name(&name)))
        })
        .map(|(pid, _)| *pid)
        .collect();
    if candidates.is_empty() {
        return;
    }
    // Executable paths and command lines for the candidates only, so the full
    // walk above stays at `stat`. The path decides which row a process is
    // (AG-947, `claude_desktop_part`); the command line tells a Claude Code
    // session from the Chrome bridge that shares its binary
    // (`is_chrome_native_host`) and the desktop app from its Electron children.
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&candidates),
        false,
        ProcessRefreshKind::nothing()
            .without_tasks()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );
    for pid in candidates {
        let Some(process) = sys.process(pid) else {
            continue;
        };
        if walk_yields(&agent_name_of(process), process.exe(), process.cmd(), names) {
            f(process);
        }
    }
}

/// Whether [`for_each_agent_process`] yields a candidate, given the fields the
/// walk read for it. The half of the walk that is testable without a live
/// process table.
///
/// Resolved, not just normalised: a `codex` the ChatGPT app ships is that app,
/// so a walk for the CLI must not yield it and a walk for the app must
/// (AG-947). The Chrome bridge shares Claude Code's binary and is not a
/// session, and the desktop app's Electron children are the app, not rows of
/// their own.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn walk_yields(
    name: &str,
    exe: Option<&std::path::Path>,
    cmd: &[std::ffi::OsString],
    names: &[&str],
) -> bool {
    agent_row_for(name, exe).is_some_and(|row| {
        if !names.contains(&row.1) || is_chrome_native_host(cmd) {
            return false;
        }
        if row.1 == "Claude" && is_electron_child(cmd) {
            return false;
        }
        // Codex's app-server daemon outlives every session and is not one
        // (`codex::is_app_server_command`); counting it put "Close tool" on
        // screen with nothing open.
        !(row.1 == "codex" && gate_connect_core::integrations::codex::is_app_server_command(cmd))
    })
}

/// Is this `claude` the Claude in Chrome native-messaging host rather than a
/// Claude Code session?
///
/// Chrome launches `claude --chrome-native-host` as the bridge between the
/// extension and Claude Code, and it carries the CLI's process name, so the
/// name match counted it as a session. It is started and stopped by Chrome,
/// not by the user, which made it a Claude Code "reopen" nobody could clear by
/// reopening Claude Code - and put it in the set `close_running_agents` kills.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn is_chrome_native_host(cmd: &[std::ffi::OsString]) -> bool {
    cmd.iter().skip(1).any(|arg| arg == "--chrome-native-host")
}

/// A process's name as [`AGENT_PROCESSES`] spells it: any `.exe` stripped, so
/// one list serves all three desktop OSes. Extracted so the per-tool staleness
/// check below matches on exactly the same normalisation the walk itself
/// filtered by, rather than a second copy that could drift from it.
///
/// **Case is preserved, and that is the whole point.** This used to lowercase,
/// which folded together the two things [`RunningAgent::name`] says have to stay
/// apart: `Claude` is the desktop app and `claude` is the CLI. With Claude
/// Desktop open and no CLI running, the app matched the `claude-code` entry, so
/// the routing takeover offered to close the user's desktop app and the startup
/// hint nagged about a CLI that was not running. macOS and Windows both; Linux
/// has no desktop app to collide with.
///
/// The `.exe` strip is what the lowercasing was really for - a Windows suffix,
/// not a case difference - so it is now matched case-insensitively and the name
/// itself is left alone.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn agent_name_of(process: &sysinfo::Process) -> String {
    let name = normalise_agent_name(&process.name().to_string_lossy());
    if is_python_name(&name) && is_hermes_command(process.cmd()) {
        return "hermes".to_string();
    }
    name
}

/// Whether a process name is a Python interpreter (`python`, `python3.11`,
/// macOS's `Python`). Only these can be Hermes.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn is_python_name(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("python")
}

/// Whether an interpreter's command line runs Hermes: the script right after
/// the interpreter (or after one flag) is a file called `hermes` - the
/// launcher's `hermes-agent/hermes` and the venv's `bin/hermes` both are. The
/// basename rather than the path, since `HERMES_HOME` moves the install.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn is_hermes_command(cmd: &[std::ffi::OsString]) -> bool {
    cmd.iter().skip(1).take(2).any(|arg| {
        std::path::Path::new(arg)
            .file_name()
            .is_some_and(|n| n == "hermes")
    })
}

/// The half of [`agent_name_of`] that is testable without a live process table.
fn normalise_agent_name(raw: &str) -> String {
    let cut = raw.len().saturating_sub(4);
    // `is_char_boundary` guards the slice: a process name is practically always
    // ASCII, but it comes from the OS and nothing here should be the thing that
    // panics on a name we did not expect.
    if raw.is_char_boundary(cut) && raw[cut..].eq_ignore_ascii_case(".exe") {
        raw[..cut].to_string()
    } else {
        raw.to_string()
    }
}

/// Whether a process called `codex` is the one the ChatGPT desktop app ships,
/// rather than the standalone Codex CLI.
///
/// The ChatGPT app bundles the Codex binary and runs it as a helper -
/// `/Applications/ChatGPT.app/Contents/Resources/codex … app-server …` on
/// macOS - and it is named exactly `codex`, so [`AGENT_PROCESSES`] matched it
/// as the CLI. Opening the app and then routing the section is the ordinary
/// order, and it produced "Reopen CLI to finish" for a terminal the person
/// never opened, on a surface Gate would not offer to relaunch because
/// [`Surface::Cli`] forbids it (AG-947).
///
/// **Anchored to the bundle, not to the word.** The first version of this
/// matched any ancestor directory named `ChatGPT`, on any platform, which
/// claims `~/code/chatgpt/node_modules/.bin/codex` - somebody's project that
/// happens to be called that. Promoting a process to [`Surface::App`] hands it
/// to the kill-and-relaunch machinery, so a false positive there does not
/// merely mislabel: it offers to "reopen ChatGPT" and opens the user's own
/// script. The match is therefore a `ChatGPT.app` component **immediately
/// followed by `Contents`**, which is the bundle layout and not a folder name.
///
/// **macOS only, deliberately.** A Windows arm was written from a guess at the
/// install layout and is gone: the desktop app ships through the Microsoft
/// Store, which installs under `WindowsApps\OpenAI.ChatGPT-Desktop_<version>_…`
/// with no component named `ChatGPT`, so the arm matched no real install and
/// contributed only false positives. It needs a real `process.exe()` from a
/// Windows machine before it comes back - the diagnostics report's
/// running-agents list is one place to get one.
///
/// Off Windows and macOS there is no ChatGPT desktop app to find.
fn is_chatgpt_bundled_codex(exe: Option<&std::path::Path>) -> bool {
    let Some(exe) = exe else {
        // No path to judge by. The CLI reading is the safe one: it reports a
        // reopen the person can act on and never offers to relaunch something
        // Gate has not identified.
        return false;
    };
    // The binary itself, first. Without this the check claims every process in
    // the bundle - including the app's own executable, which then lost its
    // relaunch target. `agent_row_of` only asks about a process already named
    // `codex`, but `relaunch_target_for` asks about anything, and a predicate
    // called "is the bundled codex" should answer that question at either call
    // site rather than rely on the caller having asked it.
    let Some(file) = exe.file_name() else {
        return false;
    };
    if normalise_agent_name(&file.to_string_lossy()) != "codex" {
        return false;
    }
    let parts: Vec<_> = exe.components().collect();
    parts.windows(2).any(|pair| {
        let bundle = pair[0].as_os_str().to_string_lossy();
        let inner = pair[1].as_os_str().to_string_lossy();
        // `eq_ignore_ascii_case` on the bundle because HFS+/APFS are usually
        // case-insensitive and the name reaches us as the OS spelled it;
        // `Contents` is what makes it a bundle rather than a directory.
        bundle.eq_ignore_ascii_case("ChatGPT.app") && inner == "Contents"
    })
}

/// Which part of the Claude desktop app a process called `claude` is, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaudeDesktopPart {
    /// The app's own executable, from its MSIX package.
    App,
    /// The Claude Code binary the app downloads and runs for its Code tab.
    CodeTab,
}

/// Whether a process called `claude` belongs to the Claude desktop app, rather
/// than being a Claude Code CLI the person started in a terminal.
///
/// The name cannot tell them apart on Windows, where every one of them is
/// `claude.exe` in lowercase. Measured 2026-09-28 on Claude 2.9939.2.0 from the
/// Microsoft Store, through the `sysinfo` this crate links: the app's main
/// process and all its Electron children report `claude.exe` from
/// `C:\Program Files\WindowsApps\Claude_2.9939.2.0_x64__pzs8sxrjxfjjc\app\`,
/// so the `Claude` row never matched and the app read as the CLI - the same
/// failure AG-947 fixed for ChatGPT's `codex`, with the name wrong in the other
/// direction. The Code tab's sessions report `claude.exe` from
/// `...\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\claude-code\2.1.281\`
/// (WMI shows the same file un-virtualised, under `AppData\Roaming\Claude`),
/// and read as a terminal session Gate can never reopen, when quitting and
/// reopening the app is what restarts them.
///
/// **Anchored to the layout, not to the word**, for the reason
/// [`is_chatgpt_bundled_codex`] gives: promoting a process to [`Surface::App`]
/// hands it to the kill-and-relaunch machinery. So the app is a component
/// shaped like the package's full name (`Claude_<version>_<arch>__<publisher
/// hash>`) directly under `WindowsApps`, and the Code tab is exactly
/// `Roaming\Claude\claude-code\<version>\claude.exe`. A `claude` installed
/// with npm, WinGet or the native installer lands in none of these.
///
/// **Windows shapes only.** macOS spells the app `Claude`, which the table
/// already matches, and the Code tab's binary there has not been measured.
/// The installer downloaded from claude.ai is the same MSIX package, in the
/// same `WindowsApps\Claude_<version>_<arch>__pzs8sxrjxfjjc\` folder, so this
/// covers it too.
fn claude_desktop_part(exe: Option<&std::path::Path>) -> Option<ClaudeDesktopPart> {
    let exe = exe?;
    if !normalise_agent_name(&exe.file_name()?.to_string_lossy()).eq_ignore_ascii_case("claude") {
        return None;
    }
    let parts: Vec<String> = exe
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    // The Store's publisher hash for Anthropic, which is the half of the name
    // no other package can carry.
    let is_claude_package =
        |dir: &str| dir.starts_with("claude_") && dir.ends_with("__pzs8sxrjxfjjc");
    if parts
        .windows(2)
        .any(|pair| pair[0] == "windowsapps" && is_claude_package(&pair[1]))
    {
        return Some(ClaudeDesktopPart::App);
    }
    // `.../Roaming/Claude/claude-code/<version>/claude.exe`: the four
    // components above the file, read from the end.
    let n = parts.len();
    if n >= 5
        && parts[n - 5] == "roaming"
        && parts[n - 4] == "claude"
        && parts[n - 3] == "claude-code"
    {
        return Some(ClaudeDesktopPart::CodeTab);
    }
    None
}

/// Is this an Electron child process (renderer, GPU, utility) rather than the
/// app itself?
///
/// On Windows the Claude app's children share its executable and so its name,
/// and there were thirteen of them beside the one main process in the
/// measurement [`claude_desktop_part`] cites. Each would be a row, a staleness
/// count and a kill target of its own. Chromium marks every child with a
/// `--type=` switch and never the browser process, so that is the test.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn is_electron_child(cmd: &[std::ffi::OsString]) -> bool {
    cmd.iter()
        .skip(1)
        .any(|arg| arg.to_string_lossy().starts_with("--type="))
}

/// The [`AGENT_PROCESSES`] row a running process belongs to, by the same
/// normalisation the walk filtered on ([`agent_name_of`]).
///
/// `None` for a process no row claims, which cannot happen for a process the
/// walk yielded and is handled rather than asserted: the table is the only
/// thing keeping the two in step. (That paragraph was stranded above
/// `surface_of` before this function existed, and inserting one under it would
/// have glued it to the wrong thing - raised in review on #352.)
///
/// The one place the process table is consulted, so the walk's filter and every
/// per-process question below cannot disagree about what a process is - which
/// is exactly how the bundled `codex` came to be filtered in as a CLI and then
/// described as one.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn agent_row_of(
    process: &sysinfo::Process,
) -> Option<&'static (&'static str, &'static str, &'static str, Surface)> {
    agent_row_for(&agent_name_of(process), process.exe())
}

/// The half of [`agent_row_of`] that is testable without a live process table.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn agent_row_for(
    name: &str,
    exe: Option<&std::path::Path>,
) -> Option<&'static (&'static str, &'static str, &'static str, Surface)> {
    // Before the name lookup, because the name is the thing that is wrong here.
    let name = if name == "codex" && is_chatgpt_bundled_codex(exe) {
        "ChatGPT"
    } else if name.eq_ignore_ascii_case("claude") {
        match claude_desktop_part(exe) {
            Some(ClaudeDesktopPart::App) => "Claude",
            Some(ClaudeDesktopPart::CodeTab) => return Some(&CLAUDE_CODE_TAB),
            None => name,
        }
    } else {
        name
    };
    AGENT_PROCESSES.iter().find(|(_, n, _, _)| *n == name)
}

/// The row for a Claude Code session the desktop app's Code tab started.
///
/// **Still `claude-code`, and still named `claude`**, because it is Claude
/// Code: it reads the `~/.claude/settings.json` the claude-code integration
/// rewrites, so a scan asking which `claude-code` processes are stale must find
/// it, and it goes stale on that file's changes rather than on the desktop
/// app's (`config_changed_at_unix` is keyed by this slug). Filing it under
/// `anthropic` dropped it from every one of those scans, and Gate reported no
/// restart needed over a session still running without the route.
///
/// What differs from a terminal `claude` is the product name. The surface is
/// [`Surface::Cli`], because Gate may not relaunch it: the thing to relaunch is
/// the app, and the app's own row carries that.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
static CLAUDE_CODE_TAB: (&str, &str, &str, Surface) = (
    "claude-code",
    "claude",
    "Claude Code in Claude Desktop",
    Surface::Cli,
);

/// Which kind of surface a running process is, by the same normalisation the
/// walk filtered on. `None` for a process no row claims.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn surface_of(process: &sysinfo::Process) -> Option<Surface> {
    agent_row_of(process).map(|(_, _, _, surface)| *surface)
}

/// The product name of the tool a running process belongs to, by the same
/// normalisation the walk filtered on. `None` for a process no row claims.
///
/// The fallback every surface of the reopen flow needs when `list_tools` cannot
/// answer, which is the case for the two desktop-app rows - see
/// [`AGENT_PROCESSES`].
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn agent_product_name_of(process: &sysinfo::Process) -> Option<&'static str> {
    agent_row_of(process).map(|(_, _, product, _)| *product)
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn agent_slug_of(process: &sysinfo::Process) -> Option<&'static str> {
    agent_row_of(process).map(|(slug, _, _, _)| *slug)
}

/// Did this agent start before the last change to something it reads once at
/// launch, and so is still using what it loaded then?
///
/// The per-process form of [`reopen_pending_for`], and the same decision:
/// [`gate_connect_core::reopen::reopen_pending`] over this process's start, its
/// tool's configuration file and Gate's CA certificate. One rule, so the count,
/// the listing and the per-tool verdict cannot disagree about a process.
///
/// **The configuration's time is Gate's record, not its mtime.** The mtime
/// moved on the tool's own edits too - Claude Code rewrites `settings.json` on
/// every permission approval - and each one made every older process read as
/// stale with nothing Gate routes by changed.
/// [`gate_connect_core::config_changes`] is stamped only when Gate's write
/// actually changed the file. A hand edit to a routing value is what this
/// gives up, and the config status still reports that one.
///
/// **Not "started before routing came up".** That was the rule until the
/// forwarder fronted both the proxy and the relay, and it asked the wrong
/// question: a config naming the forwarder or the relay port reaches the engine
/// the moment the engine is up, so an agent that predates the enable is routed
/// without a restart. It also reset on every launch of Gate, so every agent
/// already running was reported stale each time the app opened.
///
/// A desktop app has no configuration file of its own, so only the certificate
/// can make it stale here; a process with no slug makes no claim.
///
/// `since` narrows the question to changes stamped at or after it. A routing
/// toggle asks only about what it changed itself: an agent that missed the
/// first connect stays behind that stamp for good, and without the bound
/// every later off-and-on repeated the notice for a change it did not make.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn agent_needs_reopen(process: &sysinfo::Process, since: Option<u64>) -> bool {
    agent_slug_of(process)
        .map(|slug| {
            gate_connect_core::reopen::reopen_pending(&gate_connect_core::reopen::ReopenEvidence {
                process_names_known: true,
                process_starts: &[process.start_time()],
                config_changed_at: at_or_after(config_changed_at_unix(slug), since),
                ca_cert_changed_at: at_or_after(ca_cert_changed_at_unix(), since),
            })
        })
        .unwrap_or(false)
}

/// A change stamp, kept only when it is at or after `since`, so the decision in
/// [`agent_needs_reopen`] sees only the changes a toggle made itself. All Unix
/// seconds.
///
/// `>=` because both sides are whole seconds: a change stamped in the same
/// second the toggle began was made by it.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn at_or_after(changed_at: Option<u64>, since: Option<u64>) -> Option<u64> {
    changed_at.filter(|&changed_at| since.is_none_or(|since| changed_at >= since))
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

/// Turn native notifications on or off. One switch, because Settings draws one
/// row: a request blocked or flagged by the security feed (AG-578); the notice
/// of a [`quit_app`] that put every tool back on its own settings; and the
/// session-expired notice from either of its two paths (the refresh loop and
/// [`signal_session_dead`]).
///
/// One exception: a quit's notice that a tool could *not* be put back is shown
/// regardless, because nothing else is left running to say it (see
/// `quit_notice_body`).
#[tauri::command]
fn set_notifications(enabled: bool) -> Result<(), String> {
    gate_connect_core::preferences::set_notifications(enabled).map_err(|e| format!("{e:#}"))
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

/// Whether the events from before this connection could be fetched.
///
/// Read on mount for the same reason `security_feed_state` is: the
/// `security-feed-history` event only reaches a window that was already
/// listening, and the backfill runs once per connection - so a window opened
/// after a failed catch-up would never hear about it and would render the gap as
/// an empty feed.
#[tauri::command]
fn security_feed_history_ok() -> bool {
    security_feed().history_ok()
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

/// Record the provider domains Gate switched on for one tool.
///
/// Recording only, like the preference it writes: the routing already happened
/// through `proxy_set_domain`. This is what lets the matching disconnect switch
/// off what Gate turned on and nothing else - see
/// `preferences::auto_enabled_domains`.
#[tauri::command]
fn record_auto_enabled_domains(tool: String, domains: Vec<String>) -> Result<(), String> {
    gate_connect_core::preferences::record_auto_enabled_domains(&tool, domains)
        .map_err(|e| format!("{e:#}"))
}

/// Read what Gate switched on for one tool, without clearing it.
///
/// The caller writes the record back once it knows what it actually managed to
/// undo - see `preferences::read_auto_enabled_domains`.
#[tauri::command]
fn read_auto_enabled_domains(tool: String) -> Vec<String> {
    gate_connect_core::preferences::read_auto_enabled_domains(&tool)
}

/// Claim a once-per-install analytics milestone (AG-960). True exactly once per
/// install across every window and process; see
/// `gate_connect_core::analytics`. Rejects for a name outside the closed set,
/// and the webview reads any rejection as "do not send". Off the main thread,
/// like every other command here that touches the disk.
#[tauri::command]
async fn analytics_milestone_claim(name: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        gate_connect_core::analytics::claim(&name).map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| format!("analytics claim join error: {e}"))?
}

/// Why local Cowork cannot run on this machine, from Claude Desktop's own
/// settings, or null when nothing readable says it is off. Read once, after the
/// Claude Desktop row is connected: a few small file reads (and the registry on
/// Windows), never on a timer, and off the main thread, because a redirected
/// `%APPDATA%` can be slow. See
/// `gate_connect_core::analytics::cowork_setting_missing`.
#[tauri::command]
async fn cowork_setting_check() -> Option<&'static str> {
    tauri::async_runtime::spawn_blocking(gate_connect_core::analytics::cowork_setting_missing)
        .await
        .ok()
        .flatten()
}

/// Record whether Gate Connect may send diagnostic data. Onboarding records the
/// first answer; this is Settings changing it. Nothing is uploaded here - the
/// send path is its own story.
#[tauri::command]
fn set_share_diagnostics(enabled: bool) -> Result<(), String> {
    gate_connect_core::preferences::set_share_diagnostics(enabled).map_err(|e| format!("{e:#}"))?;
    // Every window runs its own analytics client, and only the one the user
    // clicked in knows the answer changed. Broadcast it so the tray and the
    // intro stop (or start) too, instead of acting on the answer they read at
    // launch until the next one (AG-960).
    if let Some(handle) = APP_HANDLE.get() {
        let _ = handle.emit(
            ANALYTICS_CONSENT_EVENT,
            serde_json::json!({ "share_diagnostics": enabled, "recorded": true }),
        );
    }
    Ok(())
}

/// What this install is identified as in analytics; see
/// `gate_connect_core::analytics::Identity`. First finishes a forget a
/// sign-out could not land (`forget_identity_if_signed_out`), so a window never
/// starts as an account that has gone; when it forgot, the other windows are
/// told too. Best-effort: a failure there leaves the read as it was.
#[tauri::command]
async fn analytics_identity() -> Result<gate_connect_core::analytics::Identity, String> {
    tauri::async_runtime::spawn_blocking(|| {
        use gate_connect_core::analytics;
        let forgot = analytics::forget_identity_if_signed_out().unwrap_or_else(|e| {
            eprintln!("analytics identity: finishing a forget failed: {e:#}");
            false
        });
        let identity = analytics::load_identity();
        if forgot {
            announce_analytics_identity(identity.clone());
        }
        identity
    })
    .await
    .map_err(|e| format!("analytics identity join error: {e}"))
}

/// Record a change of analytics identity, and tell every window, so each one's
/// client follows the same person (AG-960). Called only by the window that
/// owns sign-in.
///
/// The record is announced from inside the core's lock (`on_stored`), so two
/// announcements reach the windows in the order of the writes. A save refused
/// because its sub is not the live session's is announced too: the window is
/// acting on a session that has ended, and the stored record is what moves it,
/// and every other window, back. A save whose session could not be READ (the
/// secret store did not answer) is announced to nobody and rejected with
/// [`ANALYTICS_IDENTITY_UNCONFIRMED`], which the webview reads as "keep what
/// you have and try again", not as a sign-out.
#[tauri::command]
async fn set_analytics_identity(
    identity: gate_connect_core::analytics::Identity,
) -> Result<(), String> {
    use gate_connect_core::analytics::SaveOutcome;
    tauri::async_runtime::spawn_blocking(move || match gate_connect_core::analytics::save_identity(
        identity,
        |stored| announce_analytics_identity(stored.clone()),
    ) {
        Ok(SaveOutcome::Saved) => Ok(()),
        Ok(SaveOutcome::NotLive) => Err(ANALYTICS_IDENTITY_NOT_LIVE.to_string()),
        Ok(SaveOutcome::Unconfirmed) => Err(ANALYTICS_IDENTITY_UNCONFIRMED.to_string()),
        Err(e) => Err(format!("{e:#}")),
    })
    .await
    .map_err(|e| format!("analytics identity join error: {e}"))?
}

/// The rejection of a save whose sub is not the live session's. Pinned with the
/// webview by `src/lib/analytics.contract.test.ts`.
const ANALYTICS_IDENTITY_NOT_LIVE: &str = "analytics-identity-not-live";
/// The rejection of a save whose session could not be read. Pinned likewise.
const ANALYTICS_IDENTITY_UNCONFIRMED: &str = "analytics-identity-unconfirmed";

/// The event every window's analytics seam listens on to follow a change of
/// analytics identity (`src/lib/analytics.ts`). Pinned on both sides by
/// `src/lib/analytics.contract.test.ts`.
const ANALYTICS_IDENTITY_EVENT: &str = "analytics-identity-changed";
/// The event every window listens on to follow a change of the diagnostics
/// answer made in another window.
const ANALYTICS_CONSENT_EVENT: &str = "analytics-consent-changed";

fn announce_analytics_identity(identity: gate_connect_core::analytics::Identity) {
    if let Some(handle) = APP_HANDLE.get() {
        let _ = handle.emit(ANALYTICS_IDENTITY_EVENT, identity);
    }
}

/// Tell every window what analytics identity is stored now. The core does the
/// forgetting (`oauth::clear`), so every caller of it, the CLI included,
/// changes the record; this is the shell's
/// half, because only the shell has windows to tell. Without it each window
/// kept the old account for the rest of the session.
fn announce_stored_analytics_identity() {
    announce_analytics_identity(gate_connect_core::analytics::load_identity());
}

/// The process name to look for on behalf of one tool.
///
/// `None` for the tools Gate has no way to recognise in the process table:
/// OpenClaw and Hermes ship no fixed process name, and `env-proxy` is not a
/// process at all. For those, staleness is *unobservable* rather than false -
/// see [`reopen_pending_for`], which says what that costs.
///
/// Reads [`AGENT_PROCESSES`] rather than repeating it: the per-tool verdict and
/// the per-tool close offer have to name the same processes for a slug, or one
/// of them is talking about a tool the other is not.
///
/// **All of them, not the first.** A slug can name more than one process -
/// `anthropic` covers Claude Desktop and Cowork - and a `find` here would have
/// checked one and quietly reported the other as not running.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn agent_process_names(slug: &str) -> Vec<&'static str> {
    AGENT_PROCESSES
        .iter()
        .filter(|(s, _, _, _)| *s == slug)
        .map(|(_, name, _, _)| *name)
        .collect()
}

/// When Gate last changed this tool's configuration file, in Unix seconds.
///
/// The durable half of the reopen decision: the file is what the tool reads at
/// startup, and the record survives restarts of Gate, reboots and reinstalls -
/// which is the whole point, because the timestamps this used to compare
/// against did not. See [`gate_connect_core::reopen`] for the defect this
/// replaces.
///
/// **Gate's record, not the file's mtime.** The mtime moved on the tool's own
/// edits too - Claude Code rewrites `settings.json` on every permission
/// approval - and each one made every older process read as stale with
/// nothing Gate routes by changed. [`gate_connect_core::config_changes`] is
/// stamped only when Gate's write actually changed the file.
///
/// `None` for a tool with no configuration file of its own (the environment
/// channel) and for one Gate has no recorded change to. Both mean the same
/// thing to the caller: no recorded change, so no claim.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn config_changed_at_unix(slug: &str) -> Option<u64> {
    let integration = gate_connect_core::registry::ToolId::from_slug(slug)
        .and_then(gate_connect_core::registry::find)?;
    let path = integration.config_location()?;
    gate_connect_core::config_changes::changed_at(std::path::Path::new(&path))
}

/// When Gate's CA certificate was last written, as Unix seconds.
///
/// The second file a routed tool reads once at startup, and not per tool -
/// every tool Gate points at it reads the same one. `None` when there is none
/// on disk, which is the ordinary state before routing has ever been on.
///
/// **`ca_cert_path`, not `ca_bundle::path`.** This read the bundle until
/// review on #329, and that was wrong twice over: the tools with process
/// names are pointed at the certificate through `NODE_EXTRA_CA_CERTS`, so the
/// bundle is not a file they read; and the bundle regenerates on every Hermes
/// connect, so it would have raised the reopen alert on healthy tools while
/// missing the re-mint it exists to catch - a re-mint rewrites the
/// certificate and leaves the bundle alone until Hermes next connects.
/// `ca-cert.pem` is written only by the generate path, so its mtime is the
/// re-mint moment and nothing else. `reopen_source_is_the_file_tools_read`
/// pins the choice.
/// The file whose mtime bounds the reopen decision.
///
/// Named and separate from the stat below so the CHOICE can be tested. The
/// `max` tests in `reopen.rs` pass whichever file is stat'd - they exercise
/// the comparison - and this PR's original mistake was the choice, not the
/// comparison. `reopen_source_is_the_file_tools_read` asserts this equals
/// what the tools are pointed at.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn reopen_cert_source() -> Option<std::path::PathBuf> {
    gate_connect_core::proxy::ca_cert_path().ok()
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn ca_cert_changed_at_unix() -> Option<u64> {
    let path = reopen_cert_source()?;
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// Is a process for this one tool running that predates the last change to that
/// tool's configuration, and is therefore still using whatever it loaded then?
///
/// Narrowed to one tool *and* given a durable bound, which is what a per-tool
/// verdict needs: it has to say which row to mark, and survive a restart of
/// Gate.
///
/// The decision itself is [`gate_connect_core::reopen::reopen_pending`], which
/// is pure and carries the reasoning. This function is only the three readings
/// it is made from: the process names for the slug, their start times, and the
/// configuration file's mtime.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn reopen_pending_for(slug: &str) -> bool {
    let wanted = agent_process_names(slug);
    if wanted.is_empty() {
        // No process table walk for a tool the walk could not recognise, and
        // no stat either. `process_names_known: false` below is the same
        // answer, stated where the decision lives.
        return false;
    }
    let mut starts = Vec::new();
    for_each_agent_process(&wanted, |process| starts.push(process.start_time()));
    gate_connect_core::reopen::reopen_pending(&gate_connect_core::reopen::ReopenEvidence {
        process_names_known: true,
        process_starts: &starts,
        config_changed_at: config_changed_at_unix(slug),
        ca_cert_changed_at: ca_cert_changed_at_unix(),
    })
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
    /// Where this tool's traffic is going *now*, and where the config on disk
    /// asks it to go. Both `None` unless the reason is `reopen_required`, which
    /// is the one verdict where those two answers can differ: the process
    /// resolved its route at launch and the file has changed under it since.
    ///
    /// `requested_route` is read from the file. `route_in_use` is **always
    /// `None`**: Gate cannot see inside another process, so there is no reading
    /// to report, and it used to be derived from the config state instead - an
    /// absent config was published as "the process is on the gateway". That is
    /// a claim about the user's traffic with nothing behind it, and it was
    /// false on the machine that reported it. See
    /// [`gate_connect_core::reopen::ReopenRoutes`], which is where the pair is
    /// built and where the reasoning lives.
    route_in_use: Option<String>,
    requested_route: Option<String>,
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
/// Off the main thread for the reason on [`running_agents`] - but as a
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
    // One read for the whole sweep, like the two probes: every row that needs to
    // name the Gate route names the same one.
    let gate_route = gate_connect_core::account::load_base_url()
        .ok()
        .flatten()
        .unwrap_or_else(|| "Constellation Gate".to_string());

    let mut recorded: Vec<(String, routing_health::RoutingVerdict)> = Vec::new();
    let verdicts: Vec<VerdictDto> = registry::registry()
        .iter()
        .filter(|integ| !integ.hidden_in_ui())
        .map(|integ| {
            let status = integ.status();
            let installed = !matches!(status, Ok(gate_connect_core::Status::NotInstalled));
            let slug = integ.id().to_string();
            let config = ConfigState::from_status(&status);
            let verdict = routing_health::verdict_for(&Evidence {
                installed,
                config,
                route,
                session,
                reopen_pending: reopen_pending_for(&slug),
            });
            let reason = verdict.reason();
            recorded.push((slug.clone(), verdict));
            // Only the half that was read off disk. The surfaces draw the pair
            // when both are present and omit it otherwise, so a reopen notice
            // now names the action rather than an endpoint nobody measured.
            let routes = if matches!(reason, Some(routing_health::Reason::ReopenRequired)) {
                gate_connect_core::reopen::reopen_routes(
                    config,
                    &gate_route,
                    integ.default_upstream_url(),
                )
            } else {
                gate_connect_core::reopen::ReopenRoutes {
                    in_use: None,
                    requested: None,
                }
            };
            VerdictDto {
                slug,
                state: verdict.as_str(),
                reason: reason.map(|r| r.as_str()),
                next_action: reason.map(|r| r.next_action().as_str()),
                route_in_use: routes.in_use,
                requested_route: routes.requested,
            }
        })
        .collect();
    // Persisted after the sweep, not during it: the log is what lets the recovery
    // summary report a check it did not take, and a half-written sweep would be a
    // worse record than the previous whole one.
    gate_connect_core::verdict_log::record_sweep(&recorded);
    verdicts
}

/// Ask the gateway whether the stored session still works, mapped onto the
/// verdict layer's vocabulary.
///
/// An API-key account reports `Valid`: there is no session to probe, and the key
/// is validated when it is saved. Reporting `Unknown` instead would park every
/// key-based install on "Verification failed" permanently.
///
/// `Rejected` is kept for what signing in actually fixes: a refusal from the
/// gateway or the identity provider, or no stored session at all. It used to be
/// whatever `live_session()` returned `None` for, which is also an unreachable
/// identity provider or an unreadable secret store, and those put "Access
/// problem / Sign in" on a machine that was only offline.
///
/// A gateway refusal goes to [`recheck_gate_session`], the forced refresh the
/// data-plane 401 path already uses. Expiry is stamped and checked against the
/// local clock, so a constant offset cancels out, but a clock that moved after
/// the token was stamped keeps a dead token looking fresh. The forced refresh
/// is what recovers that, here too, instead of waiting for traffic to fail.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn probe_session_health() -> gate_connect_core::routing_health::SessionHealth {
    use gate_connect_core::oauth;
    use gate_connect_core::org::{self, SessionProbe};
    use gate_connect_core::routing_health::SessionHealth;

    if account::auth_mode().unwrap_or_default() != account::AuthMode::OAuth {
        return SessionHealth::Valid;
    }
    // Set in `setup`, before any webview can ask for a sweep. Without it the
    // recheck could not push a recovered token or raise the tray signal, so
    // report the refusal as it stands. A check already in flight gets no
    // second one beside it: that one is forcing the same refresh, so this
    // sweep has no verdict of its own and reads "Verification failed" until
    // the next sweep sees what it decided.
    let recheck = || match APP_HANDLE.get() {
        Some(app) => gate_connect_core::proxy::try_begin_gate_auth_check()
            .map(|_check| recheck_gate_session(app))
            .unwrap_or(SessionHealth::Unknown),
        None => SessionHealth::Rejected,
    };
    let Ok(Some(gateway)) = account::load_base_url() else {
        return SessionHealth::Unknown;
    };
    if oauth::session_rejected() {
        // Startup, the data-plane recheck and the org list only record this
        // after a forced refresh was refused too, so a clock that moved has
        // already had its chance to recover there.
        return SessionHealth::Rejected;
    }
    let Some(cfg) = oauth::OAuthConfig::from_build_env() else {
        // An OAuth account in a build that cannot refresh it: nothing will
        // work until the user signs in to something this build can use.
        return SessionHealth::Rejected;
    };
    let tokens = match oauth::ensure_fresh_classified(&cfg) {
        Ok(Some(tokens)) => tokens,
        // OAuth mode with nothing stored: signed out.
        Ok(None) => return SessionHealth::Rejected,
        Err(e) if e.is_refusal() => return SessionHealth::Rejected,
        // Identity provider unreachable, or the secret store would not answer.
        // Neither is evidence against the credential.
        Err(_) => return SessionHealth::Unknown,
    };
    match org::probe_session(&gateway, &tokens.access_token) {
        SessionProbe::Accepted(_) => SessionHealth::Valid,
        SessionProbe::Rejected => recheck(),
        // Offline or a non-auth error. Never evidence against the credential -
        // `SessionProbe::Unavailable`'s own docs are explicit about this, and
        // the verdict layer turns it into "Verification failed", not "Access
        // problem".
        SessionProbe::Unavailable => SessionHealth::Unknown,
    }
}

/// One tool in a teardown report, and the one thing left to do about it.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[derive(Serialize)]
struct TeardownToolDto {
    slug: String,
    name: String,
    next_action: &'static str,
}

/// Where every installed tool stands after a teardown - routing off, disconnect,
/// sign-out or reset.
///
/// **Read back, not recorded.** The buckets come from re-reading each tool's own
/// config, never from what the teardown believed it wrote. That is the O1 rule
/// `docs/routing-architecture.md` states for the environment channel, applied
/// here: a sweep that returns success having written nothing is exactly the
/// failure this report exists to catch, and a report assembled from the sweep's
/// own return value would repeat its mistake.
///
/// A consequence worth stating: a tool the sweep *named* as failed can appear
/// under `defaults` if its config reads clean now. That is right. The user asked
/// where their tools point, and the file is the answer.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[derive(Serialize)]
struct TeardownReportDto {
    /// Back on their own settings, verified by reading the config.
    defaults: Vec<TeardownToolDto>,
    /// Still carrying Gate's values. The teardown did not put these back.
    still_gate: Vec<TeardownToolDto>,
    /// On their own settings on disk, but running a process that predates the
    /// change - so still routing through Gate until it is reopened.
    awaiting_reopen: Vec<TeardownToolDto>,
    /// Could not be read at all, so nothing about them is known. Not `defaults`:
    /// an unreadable config is ignorance, not a clean result - the same
    /// distinction `routing_health::ConfigState::Unreadable` draws.
    failed: Vec<TeardownToolDto>,
}

/// Where the tools stand after a teardown, so an operation that could not put
/// every tool back can say which ones.
///
/// Read-only and cheap enough for a dialog to call on open: one status read per
/// integration plus one process walk, the same work `routing_verdicts` does
/// without the network probes.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command(async)]
fn teardown_report() -> TeardownReportDto {
    let mut report = TeardownReportDto {
        defaults: Vec::new(),
        still_gate: Vec::new(),
        awaiting_reopen: Vec::new(),
        failed: Vec::new(),
    };
    for integ in registry::registry() {
        if integ.hidden_in_ui() {
            continue;
        }
        let slug = integ.id().to_string();
        let status = integ.status();
        // Not on the machine: it has no configuration to put back, and listing it
        // would pad a report the user reads as a to-do list.
        if matches!(status, Ok(gate_connect_core::Status::NotInstalled)) {
            continue;
        }
        let tool = |next_action: &'static str| TeardownToolDto {
            slug: slug.clone(),
            name: integ.display_name().to_string(),
            next_action,
        };
        match status {
            Ok(gate_connect_core::Status::Connected)
            | Ok(gate_connect_core::Status::Drifted(_))
            | Ok(gate_connect_core::Status::Overridden(_)) => {
                report.still_gate.push(tool("retry_disconnect"))
            }
            Ok(_) if reopen_pending_for(&slug) => report.awaiting_reopen.push(tool("reopen_tool")),
            Ok(_) => report.defaults.push(tool("none")),
            Err(_) => report.failed.push(tool("retry_check")),
        }
    }
    report
}

/// One running AI tool, as the diagnostics report lists it.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[derive(Serialize)]
struct RunningAgent {
    /// The tool this process belongs to, from [`AGENT_PROCESSES`]. Carried so a
    /// caller that offered to close a set of tools can tie each process back to
    /// the row it was offered under - the reopen flow tracks a tool through
    /// close, reopen and verification, and a process name is not a key: `claude`
    /// and `Claude` are one slug and two different programs.
    slug: String,
    /// Process name as the OS spells it, original case. On macOS "Claude" is
    /// the desktop app and "claude" the CLI; on Windows both are `claude.exe`
    /// and only the path tells them apart ([`claude_desktop_part`]), so `slug`
    /// is the key to read, not this.
    name: String,
    /// The tool's product name, from [`AGENT_PROCESSES`]. What a surface should
    /// draw when `list_tools` cannot name the slug, which is every scan that
    /// finds one of the two desktop apps: their slugs are proxy-domain keys, so
    /// the registry has no row to read a name off.
    product_name: String,
    /// Can the routing sweep produce a verdict for this tool at all?
    ///
    /// True exactly for the registry integrations. `routing_verdicts` walks
    /// [`registry::registry`], so the desktop-app rows - whose slugs are
    /// proxy-domain keys - never get an entry there, whatever they are doing.
    ///
    /// Reported rather than inferred because the reopen flow's verification step
    /// waits on that verdict. Without this the wait could only ever time out:
    /// the row spun in `Verifying` and then claimed verification had *failed*
    /// for a tool nothing was ever going to answer for - which on macOS is the
    /// one row Gate closes and reopens itself, so it is the row the user
    /// watches.
    verifiable: bool,
    /// Can Gate Connect launch this tool again itself, once it has been closed?
    ///
    /// **False for every tool in the registry**, and this is a fact about them
    /// rather than a feature nobody wrote yet. All six are terminal programs:
    /// they run inside a shell session Gate does not own, with a working
    /// directory, arguments and a conversation this process cannot see, and
    /// nothing that walks the process table can recover them. Spawning a fresh
    /// terminal would not be reopening the tool - it would be starting a
    /// different one, somewhere else, and dropping the session the user was
    /// asked to save.
    ///
    /// It is reported rather than assumed because the flow that reads it has to
    /// say which tools it will reopen and which the user must, and that sentence
    /// should come from the backend that knows. A GUI tool - one launchable by
    /// bundle id, shortcut or `.desktop` entry - is where this turns true.
    can_reopen: bool,
    pid: u32,
    /// Process start, Unix seconds. 0 when the platform wouldn't say.
    started_at_unix: u64,
    /// Started before the last change to its own configuration or to Gate's
    /// certificate, so it is still using what it loaded and needs a restart.
    /// Decided by [`agent_needs_reopen`].
    needs_reopen: bool,
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
/// each started, and whether it needs a reopen. Same process set and the
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
    let mut agents = Vec::new();
    for_each_agent_process(&names, |process| {
        let started_at_unix = process.start_time();
        let slug = agent_slug_of(process).unwrap_or_default();
        agents.push(RunningAgent {
            slug: slug.to_string(),
            name: process.name().to_string_lossy().to_string(),
            // The table's name, falling back to the OS's rather than to a blank:
            // a row with no title at all is worse than one titled `Claude`, and
            // a process the walk yielded always has a row to read.
            product_name: agent_product_name_of(process)
                .map(|n| n.to_string())
                .unwrap_or_else(|| process.name().to_string_lossy().to_string()),
            verifiable: ToolId::from_slug(slug).is_some(),
            // Derived, not assumed: true exactly when Gate resolved somewhere
            // to launch this back from. A CLI never resolves one - see
            // `Surface` - and an app whose executable path the OS would not
            // give us reports false rather than promising a reopen that would
            // then not happen.
            can_reopen: surface_of(process)
                .and_then(|s| relaunch_target(process, s))
                .is_some(),
            pid: process.pid().as_u32(),
            started_at_unix,
            needs_reopen: agent_needs_reopen(process, None),
        });
    });
    // Oldest first: the ones that need a reopen are the ones being looked
    // for, and a stable order keeps two reports from the same machine
    // diffable.
    //
    // **The tie-break is load-bearing, not tidiness.** `reopen.ts`'s
    // `reopenTools` keeps the FIRST row per slug, and since the ChatGPT app and
    // the `codex` it bundles resolve to one slug (AG-947), two rows now compete
    // to speak for that app. `start_time` is whole seconds and both processes
    // can land in the same one; the walk itself is a `HashMap` iteration, and
    // `sort_by_key` is stable, so a tie would hand the row to whichever the map
    // happened to yield first. When that is the helper the app reports
    // `can_reopen: false` and the dialog offers no way to reopen it.
    //
    // So: relaunchable first among equals, then pid, which is total. Before
    // the two shared a slug there was no contest and the plain sort was right.
    agents.sort_by_key(|agent| (agent.started_at_unix, !agent.can_reopen, agent.pid));
    RunningAgentsDto {
        scanned_names: names.iter().map(|n| n.to_string()).collect(),
        agents,
    }
}

/// How long an agent gets to quit on its own before Gate stops waiting. A
/// restart has to see the app gone before it relaunches it, or the launch just
/// brings the old one forward. Claude Desktop's quit cleanup, Cowork's VM
/// shutdown included, runs in about a second; the rest is margin for a machine
/// under load.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
const AGENT_CLOSE_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

/// One agent picked for closing, captured before it is asked to quit, because
/// afterwards there is no process left to ask where it came from.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
struct CloseTarget {
    pid: sysinfo::Pid,
    /// With the pid, because Windows reuses pids quickly and a wait of seconds
    /// must not end in killing or counting whatever took over the number.
    started: u64,
    /// The [`AGENT_PROCESSES`] row's slug, which the reopen queue is keyed by.
    slug: Option<&'static str>,
    /// The row's product name.
    name: String,
    surface: Option<Surface>,
    /// `None` for a CLI, which is the whole point of `Surface` - see
    /// [`relaunch_target`].
    relaunch: Option<Relaunch>,
}

/// Close running agents (CLIs and desktop apps, see [`AGENT_PROCESSES`]) and
/// wait for them to go, for [`close_running_agents`].
///
/// Asking first matters most on Windows, where the only step used to be the
/// hard kill. Claude Desktop never ran its quit cleanup, so Cowork's VM was
/// left running with nothing on the host end and the next launch could not
/// reach it. SIGTERM on macOS and Linux already is a quit request; on Windows
/// the request is `taskkill` without `/F`, which posts `WM_CLOSE` to the app's
/// windows. Only Windows kills what has not quit by [`AGENT_CLOSE_GRACE`], as
/// it always did; elsewhere a process that ignores SIGTERM is left running,
/// also as before, and reported in `still_running`.
///
/// An agent hosted by a desktop app that is being closed too (a Claude Code
/// session in Claude Desktop's Code tab) is left to that app: the app stops it
/// in its own cleanup and starts it again on relaunch, and killing it first is
/// the same damage. Any ancestor counts, not only the parent, because the app
/// may start it from one of its helpers or through a shell (see
/// [`hosted_by_agent`]). A session whose app is NOT in the set is closed like
/// any other, or a scoped close of Claude Code would leave it stale.
///
/// `only` carries the same tool-slug filter as [`running_agents`], and callers
/// are expected to pass back exactly what they offered: this closes processes,
/// so the set it closes must be the set the user was shown and agreed to.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn close_agents(only: Option<&[String]>) -> (Vec<CloseTarget>, Vec<String>) {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    // Pick first, signal after: a host can come after its child in the walk.
    let mut found: Vec<CloseTarget> = Vec::new();
    for_each_agent_process(&agent_names_for(only), |process| {
        let surface = surface_of(process);
        found.push(CloseTarget {
            pid: process.pid(),
            started: process.start_time(),
            slug: agent_slug_of(process),
            name: agent_product_name_of(process)
                .map(str::to_string)
                .unwrap_or_else(|| process.name().to_string_lossy().into_owned()),
            surface,
            relaunch: surface.and_then(|s| relaunch_target(process, s)),
        });
    });
    // The whole table, not the walk's filtered set: an app's helpers and the
    // shells between it and its child are what the ancestor walk climbs.
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().without_tasks(),
    );
    let closing_apps: std::collections::HashSet<Pid> = found
        .iter()
        .filter(|t| t.surface == Some(Surface::App))
        .map(|t| t.pid)
        .collect();
    let targets: Vec<CloseTarget> = found
        .into_iter()
        .filter(|target| {
            !hosted_by_agent(
                target.pid,
                |pid| sys.process(pid).and_then(|p| p.parent()),
                |pid| closing_apps.contains(&pid),
            )
        })
        .collect();

    let mut closed: Vec<CloseTarget> = Vec::new();
    let mut still_running: Vec<String> = Vec::new();
    let mut waiting: Vec<CloseTarget> = Vec::new();
    for target in targets {
        let Some(process) = sys
            .process(target.pid)
            .filter(|p| p.start_time() == target.started)
        else {
            continue; // quit on its own since the walk
        };
        match request_close(process) {
            // A terminal tool is waited on too: SIGTERM is a request, and one
            // that ignores it has not closed.
            CloseRequest::Asked => waiting.push(target),
            // On Windows a console process has no window to ask and `taskkill`
            // refuses it, so it is killed straight away, as it always was.
            // Elsewhere a failed SIGTERM means the kill fails too (another
            // user's process, or already gone).
            CloseRequest::NotAsked => {
                if process.kill() {
                    closed.push(target);
                }
            }
        }
    }

    let deadline = std::time::Instant::now() + AGENT_CLOSE_GRACE;
    loop {
        let pids: Vec<Pid> = waiting.iter().map(|t| t.pid).collect();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::nothing().without_tasks(),
        );
        let (running, exited): (Vec<_>, Vec<_>) = waiting.into_iter().partition(|t| {
            sys.process(t.pid)
                .is_some_and(|p| p.start_time() == t.started)
        });
        closed.extend(exited);
        waiting = running;
        if waiting.is_empty() || std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    for target in waiting {
        eprintln!(
            "[gate] close agents: {} (pid {}) did not quit within {AGENT_CLOSE_GRACE:?}",
            target.name, target.pid
        );
        let killed = cfg!(target_os = "windows")
            && sys
                .process(target.pid)
                .is_some_and(|process| process.kill());
        if killed {
            closed.push(target);
        } else {
            still_running.push(target.name);
        }
    }
    still_running.sort();
    still_running.dedup();
    (closed, still_running)
}

/// Refresh Codex's app-server daemon, if it is stale and no Codex session is
/// open. The one rule for it, shared by every path that changes what Codex
/// should load: a model save, the restart notice's Close, and routing coming
/// up (startup or the toggle), which reconnects Codex and can rewrite its
/// config (review on #382).
///
/// Stale is `codex::refresh_app_server_daemon`'s test: the daemon started
/// before Gate last changed Codex's config. An open session is left alone,
/// because the restart would end it; the restart notice asks the user to close
/// it, and its Close comes back here. A session that outlived that close still
/// counts as open, so it is not cut off either.
///
/// On a thread of its own: the restart can take seconds, and the callers are a
/// save the user is watching, a close, and the startup thread.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn refresh_codex_daemon_when_idle(why: &'static str) {
    std::thread::spawn(move || {
        let mut open_sessions = 0u32;
        for_each_agent_process(&["codex"], |_| open_sessions += 1);
        if open_sessions > 0 {
            return;
        }
        if let Err(e) = gate_connect_core::integrations::codex::refresh_app_server_daemon() {
            eprintln!("[gate] {why}: could not refresh the Codex app server: {e:#}");
        }
    });
}

/// Close running agents so their next launch picks up the routing change, and
/// queue the apps among them for [`reopen_running_agents`]. Returns how many
/// processes closed - 0 means none were running. One still running when Gate
/// stopped waiting is not counted. See [`close_agents`] for how each is asked.
///
/// `(async)` on top of the walk's own reason: this one blocks on the wait, and
/// it runs from a button the user is watching.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command(async)]
fn close_running_agents(only: Option<Vec<String>>) -> u32 {
    let (closed, _) = close_agents(only.as_deref());
    if only
        .as_deref()
        .is_none_or(|slugs| slugs.iter().any(|s| s == "codex"))
    {
        refresh_codex_daemon_when_idle("close agents");
    }
    let mut reopen: Vec<(String, Relaunch)> = Vec::new();
    for target in &closed {
        if let (Some(slug), Some(relaunch)) = (target.slug, &target.relaunch) {
            // One launch per app, however many of its processes were closed.
            if !reopen.iter().any(|(s, _)| s == slug) {
                reopen.push((slug.to_string(), relaunch.clone()));
            }
        }
    }
    if !reopen.is_empty() {
        let mut guard = PENDING_REOPEN.lock().unwrap_or_else(|e| e.into_inner());
        // Replace rather than append for the slugs in hand: a second close of
        // the same app must not queue a second launch of it.
        guard.retain(|(slug, _)| !reopen.iter().any(|(s, _)| s == slug));
        guard.extend(reopen);
    }
    closed.len() as u32
}

/// Was `pid` started under another agent, at any depth? `parent_of` and
/// `is_agent` read the process table; they are parameters so the walk can be
/// tested without one. Bounded, because a table read in pieces can hold a
/// parent loop, and pid 0/1 or a missing entry ends the chain.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn hosted_by_agent(
    pid: sysinfo::Pid,
    parent_of: impl Fn(sysinfo::Pid) -> Option<sysinfo::Pid>,
    is_agent: impl Fn(sysinfo::Pid) -> bool,
) -> bool {
    let mut current = pid;
    for _ in 0..64 {
        match parent_of(current) {
            Some(parent) if parent != current => {
                if is_agent(parent) {
                    return true;
                }
                current = parent;
            }
            _ => return false,
        }
    }
    false
}

/// How a quit request landed.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
enum CloseRequest {
    /// Delivered: SIGTERM, or on Windows a `WM_CLOSE` to the app's own
    /// windows.
    Asked,
    /// Not delivered. On Windows: no window of its own to ask, so a hard kill
    /// is the only way. Elsewhere: it could not be signalled at all.
    NotAsked,
}

/// Ask a process to quit: SIGTERM on macOS and Linux, and on Windows what its
/// own window's close button would send.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn request_close(process: &sysinfo::Process) -> CloseRequest {
    if process.kill_with(sysinfo::Signal::Term).unwrap_or(false) {
        CloseRequest::Asked
    } else {
        CloseRequest::NotAsked
    }
}

/// `taskkill` *without* `/F` posts `WM_CLOSE` to the process's top-level
/// windows and succeeds only if it had some. That is not the process agreeing
/// to exit - [`close_agents`] waits to find that out.
#[cfg(target_os = "windows")]
fn request_close(process: &sysinfo::Process) -> CloseRequest {
    use std::os::windows::process::CommandExt;
    let asked = std::process::Command::new("taskkill")
        .args(["/PID", &process.pid().to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if asked {
        CloseRequest::Asked
    } else {
        CloseRequest::NotAsked
    }
}

/// `CREATE_NO_WINDOW`: no console flash per `taskkill`, as with `certutil` in
/// `ca_windows`.
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How to open a desktop app again after it quit.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[derive(Debug, Clone, PartialEq)]
enum Relaunch {
    /// A macOS `.app` bundle, handed to `open`, or an executable elsewhere.
    /// See [`relaunch_target_for`].
    Path(PathBuf),
    /// A Windows Store app, by its application user model ID
    /// (`Claude_pzs8sxrjxfjjc!Claude`). See [`store_app_relaunch`].
    StoreApp(String),
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
impl Relaunch {
    fn spawn(&self) -> bool {
        match self {
            Relaunch::Path(path) => launch(path),
            // Through the shell, never as Gate's child: the app gets the
            // user's environment rather than Gate's, and outlives Gate.
            // `explorer.exe` exits 1 on success, so only the spawn is checked.
            Relaunch::StoreApp(aumid) => std::process::Command::new("explorer.exe")
                .arg(format!("shell:AppsFolder\\{aumid}"))
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .is_ok(),
        }
    }
}

/// The relaunch for a process whose executable is inside a Windows Store
/// package, by its app ID. A Store app cannot be started from its `.exe` under
/// `WindowsApps` (which is why [`relaunch_target_for`] refuses those paths);
/// the shell has to activate the package, and the ID it takes is read from the
/// package's own `AppxManifest.xml`.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn store_app_relaunch(exe: &str) -> Option<Relaunch> {
    let package = windows_store_package(exe)?;
    let manifest =
        std::fs::read_to_string(format!("{}\\AppxManifest.xml", package.install_dir)).ok()?;
    let executable = exe[package.install_dir.len()..].trim_start_matches('\\');
    let app_id = manifest_app_id(&manifest, executable)?;
    Some(Relaunch::StoreApp(format!("{}!{app_id}", package.family)))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[derive(Debug, PartialEq)]
struct StorePackage {
    install_dir: String,
    /// `Claude_pzs8sxrjxfjjc`: the package name plus its publisher ID.
    family: String,
    name: String,
}

/// Read a Store package out of an executable path under `WindowsApps`. The
/// folder is the package's full name, `Name_Version_Arch_ResourceId_PublisherId`:
/// five fields, the resource ID usually empty. A package name cannot hold an
/// underscore, so splitting on it is exact.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn windows_store_package(exe: &str) -> Option<StorePackage> {
    const MARKER: &str = "\\windowsapps\\";
    let start = exe.to_ascii_lowercase().find(MARKER)? + MARKER.len();
    let full_name = exe[start..].split('\\').next()?;
    let fields: Vec<&str> = full_name.split('_').collect();
    let [name, _version, _arch, _resource, publisher] = fields[..] else {
        return None;
    };
    if name.is_empty() || publisher.is_empty() {
        return None;
    }
    Some(StorePackage {
        install_dir: exe[..start + full_name.len()].to_string(),
        family: format!("{name}_{publisher}"),
        name: name.to_string(),
    })
}

/// The `Id` of the `<Application>` in an `AppxManifest.xml` whose `Executable`
/// is the one that was running (relative to the package, `app\Claude.exe`). By
/// executable, not first match: Claude's package declares a second
/// application, its SSH askpass helper, and a package is free to list them in
/// any order.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn manifest_app_id(manifest: &str, executable: &str) -> Option<String> {
    manifest
        .split("<Application")
        .skip(1)
        // `<Applications>` is the list around them, not one of them.
        .filter(|rest| rest.starts_with(char::is_whitespace))
        .filter_map(|rest| rest.split('>').next())
        .find(|tag| xml_attr(tag, "Executable").is_some_and(|e| e.eq_ignore_ascii_case(executable)))
        .and_then(|tag| xml_attr(tag, "Id"))
        .map(str::to_string)
}

/// An attribute's value inside one start tag, `name="value"`.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn xml_attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=\"");
    let mut search = tag;
    loop {
        let at = search.find(&needle)?;
        // Whole attribute names only: `Id` must not match inside `AppId`.
        let whole = search[..at].ends_with(char::is_whitespace);
        let value = &search[at + needle.len()..];
        if whole {
            return value.split('"').next();
        }
        search = value;
    }
}

/// What Gate closed and intends to put back, captured before the kill.
///
/// **Held in Rust, deliberately.** The obvious alternative is for
/// `close_running_agents` to return the paths and the frontend to hand them back
/// to the reopen call - and that would turn "reopen what you just closed" into
/// "launch whatever the webview names", which is a code-execution vector with a
/// friendly signature. The webview only ever says *reopen*; what that means was
/// decided here, from a process Gate itself found running.
///
/// Cleared as it is consumed, so a second reopen cannot launch a second copy.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
static PENDING_REOPEN: Mutex<Vec<(String, Relaunch)>> = Mutex::new(Vec::new());

/// How an app would be launched again, or `None` if Gate should not try.
///
/// `None` for a CLI even though its executable path is perfectly resolvable -
/// see [`Surface`]. Being able to spawn something is not the same as being able
/// to reopen it.
///
/// A Store app is reopened by its package identity ([`store_app_relaunch`]),
/// and anything else by path ([`relaunch_target_for`], which refuses
/// `WindowsApps` paths for exactly that reason). The bundled helper rule
/// applies to both: it is the app's process that gets relaunched.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn relaunch_target(process: &sysinfo::Process, surface: Surface) -> Option<Relaunch> {
    let exe = process.exe();
    if surface == Surface::App && !is_chatgpt_bundled_codex(exe) {
        if let Some(store) = exe.and_then(|e| store_app_relaunch(&e.to_string_lossy())) {
            return Some(store);
        }
    }
    relaunch_target_for(exe, surface).map(Relaunch::Path)
}

/// The half of [`relaunch_target`] that is testable without a live process
/// table, which is what the bundled-helper rule below is worth having.
fn relaunch_target_for(exe: Option<&std::path::Path>, surface: Surface) -> Option<PathBuf> {
    if surface != Surface::App {
        return None;
    }
    let exe = exe?;
    // The ChatGPT app's bundled `codex` reads as that app now
    // ([`is_chatgpt_bundled_codex`]), which hands it to the relaunch machinery
    // - and `close_running_agents` pushes one target per killed PROCESS, so
    // closing ChatGPT queued two `chatgpt` entries: the app and its helper.
    //
    // The helper is not a thing to launch. It cannot run without the app, and
    // the app's own process is killed and queued alongside it, so relaunching
    // that is what brings the helper back. Returning `None` here leaves exactly
    // one target for the slug and keeps `can_reopen` honest on the helper's
    // row.
    //
    // On macOS both walked up to the same `.app` and the cost was a duplicate
    // `open`. The rule is stated at the source rather than at that symptom,
    // because a platform whose layout has no bundle to walk up to would have
    // spawned the bare Codex binary as "reopen ChatGPT".
    if is_chatgpt_bundled_codex(Some(exe)) {
        return None;
    }
    // Nothing from an MSIX package is launched by path, which covers both the
    // Claude and ChatGPT Store apps. A Store app is started through its package
    // identity (`shell:AppsFolder\...`), and exec'ing the file inside
    // `WindowsApps` has not been shown to do that. Reporting `can_reopen:
    // false` asks the person to reopen it, which is honest; an unverified
    // launch that failed would claim a reopen that never happened. This became
    // reachable when the walk started reading `exe` on Windows: until then the
    // path was always `None` here.
    if exe
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case("WindowsApps"))
    {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        // `.../Claude.app/Contents/MacOS/Claude` -> `.../Claude.app`. Hand
        // LaunchServices the bundle rather than exec'ing the inner Mach-O: a
        // bare exec skips the single-instance handling that makes a second
        // launch focus the existing window, and an Electron app started that
        // way can come up without its own environment.
        if let Some(bundle) = exe
            .ancestors()
            .find(|a| a.extension().is_some_and(|e| e == "app"))
        {
            return Some(bundle.to_path_buf());
        }
    }
    Some(exe.to_path_buf())
}

/// Launch one captured target.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn launch(target: &std::path::Path) -> bool {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        // `open` asks LaunchServices, which is what makes this a reopen rather
        // than a second process: it honours the app's own single-instance rules.
        let mut c = std::process::Command::new("/usr/bin/open");
        c.arg(target);
        c
    };
    #[cfg(not(target_os = "macos"))]
    let mut cmd = std::process::Command::new(target);

    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    // Spawned, never waited on: this is a GUI application that will outlive the
    // call, and `open` on macOS returns immediately anyway. The child is
    // deliberately not reaped - see the caller, which does not block a button on
    // an app's startup time.
    cmd.spawn().is_ok()
}

/// Put back the apps [`close_running_agents`] closed.
///
/// Separate from the close so the two map onto the stages the frontend already
/// draws (`closing` -> `reopening`), and so a user who declines the reopen just
/// never calls this.
///
/// Waits for the processes to actually be gone first, bounded: the close waited
/// already, but one may have ignored the request. Relaunching an
/// app whose old instance is still shutting down is how you get the single-
/// instance logic to focus the dying window and then exit with it, which looks
/// exactly like "Gate closed my app and did not reopen it".
///
/// Returns how many were launched. Best-effort per app: one that will not start
/// does not stop the others, and the caller learns from the count rather than
/// from an error, because a partial reopen is a real outcome worth reporting.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command(async)]
fn reopen_running_agents(only: Option<Vec<String>>) -> u32 {
    let pending: Vec<(String, Relaunch)> = {
        let mut guard = PENDING_REOPEN.lock().unwrap_or_else(|e| e.into_inner());
        let (take, keep): (Vec<_>, Vec<_>) = guard.drain(..).partition(|(slug, _)| {
            only.as_deref()
                .is_none_or(|slugs| slugs.iter().any(|s| s == slug))
        });
        *guard = keep;
        take
    };
    if pending.is_empty() {
        return 0;
    }

    // Bounded wait for the old instances to go. 5s is long enough for an app
    // asked to quit gracefully and short enough that a user watching a button
    // does not conclude it is broken.
    let names: Vec<&str> = AGENT_PROCESSES
        .iter()
        .filter(|(slug, _, _, _)| pending.iter().any(|(s, _)| s == slug))
        .map(|(_, name, _, _)| *name)
        .collect();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let mut still_running = false;
        for_each_agent_process(&names, |_| still_running = true);
        if !still_running || std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }

    pending
        .iter()
        .filter(|(_, relaunch)| relaunch.spawn())
        .count() as u32
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
/// the background refresh loop on the signed-in→dead edge (a session the
/// identity provider or the gateway refused, never one it could not reach; see
/// [`session_dead_after_tick`]) and read by the tray-drawing functions to raise
/// an attention signal - a red dot on the glyph and a "sign in required"
/// tooltip - that outranks the routing-on/off color. Relaxed ordering: it only
/// gates a cosmetic redraw. Starts false (assume signed in until proven dead).
static SESSION_NEEDS_SIGNIN: AtomicBool = AtomicBool::new(false);

/// Whether the refresh loop should consider the session dead after a tick's
/// [`session_reading`](gate_connect_core::oauth::session_reading).
///
/// A refused or rejected session is dead only while a bundle is still stored:
/// a deliberate sign-out clears the bundle (`oauth::clear`) and must stay quiet
/// even though `auth_mode` is still OAuth. A bundle that no longer parses is
/// still stored ([`is_corrupt_bundle`](gate_connect_core::oauth::is_corrupt_bundle)). An unavailable reading - the
/// identity provider could not be reached, or the secret store could not be
/// read - is no verdict, so the tick keeps whatever it believed before. Calling
/// it dead put "session expired" on every machine that woke offline with an
/// expired access token.
fn session_dead_after_tick(
    reading: &gate_connect_core::oauth::SessionReading,
    has_stored_bundle: impl FnOnce() -> bool,
    was_dead: bool,
) -> bool {
    use gate_connect_core::oauth::SessionReading;
    match reading {
        SessionReading::Live(_) => false,
        SessionReading::SignedOut => has_stored_bundle(),
        SessionReading::Unavailable => was_dead,
    }
}

/// How far the wall clock may drift from elapsed monotonic time across one
/// refresh tick before the background loop treats it as a jump rather than
/// drift. Ordinary NTP slew across 30s is milliseconds; a resume or a stepped
/// correction is minutes to hours. Wide enough that nothing renews for a
/// well-behaved clock, narrow enough to catch a sleep short of the token's own
/// lifetime.
const CLOCK_JUMP_TOLERANCE: std::time::Duration = std::time::Duration::from_secs(120);

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
/// guard; every caller holds one. The observer path takes it at the top of its
/// own thread so a panic still releases it, and the routing sweep and the Linux
/// refusal counter take it with
/// [`gate_connect_core::proxy::try_begin_gate_auth_check`], so two triggers
/// never force two refreshes at once.
///
/// Returns the verdict in the routing sweep's vocabulary, for
/// [`probe_session_health`]; the refusal-driven callers ignore it.
fn recheck_gate_session(
    app: &tauri::AppHandle,
) -> gate_connect_core::routing_health::SessionHealth {
    use gate_connect_core::routing_health::SessionHealth;
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
            SessionHealth::Valid
        }
        gate_connect_core::startup::Recheck::Dead => {
            // A sign-in that finished since the verdict cleared it
            // (`oauth::store`) and pushed its own token, which this stale
            // verdict must not replace.
            if !gate_connect_core::oauth::session_rejected() {
                return SessionHealth::Unknown;
            }
            // Push the empty token now rather than on the next tick: the
            // engine then refuses routed requests as signed out at once, and
            // a relay request waiting on this verdict stops waiting.
            gate_connect_core::proxy::manager().refresh_token("");
            signal_session_dead(app);
            SessionHealth::Rejected
        }
        // No verdict (offline, or the 401 belonged to the client's own upstream
        // credential rather than to us): change nothing.
        gate_connect_core::startup::Recheck::Unchanged => SessionHealth::Unknown,
    }
}

/// Raise the dead-session signal from somewhere other than the refresh loop:
/// a real call the gateway has just refused - the `Dead` verdict in
/// [`recheck_gate_session`], or [`oauth_list_orgs`] - so the session is known
/// gone without waiting up to 30s for the next tick.
///
/// Edge-guarded on the same flag the loop swaps, so the two can't both react
/// to one death - whichever gets there first does the work and the other sees
/// the flag already set. Repaints the tray, nudges a mounted popover (which
/// re-reads `oauth_status` and routes to sign-in; the frontend has no status
/// poll by design), and posts the same notification the refresh loop would
/// have, on the same platforms and under the same notifications switch - the
/// tray dot alone is out of the user's eyeline while they sit watching a tool
/// fail.
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
    // Gated on the notifications preference like the refresh loop's copy of
    // this notice: whichever path consumes the `SESSION_NEEDS_SIGNIN` edge is
    // the only one that notifies, so both have to honour the switch. The tray
    // and the emit above are in-app state and stay unconditional.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if gate_connect_core::preferences::load().notifications {
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
/// hides the window it was clicked in: the OS dialog takes focus, the
/// `Focused(false)` handler hides the popover, and the copy telling the user
/// what to click goes with it.
///
/// **The handler exists now**, so this flag is finally read. It was written at
/// four sites and read at none for as long as blur-dismiss was deferred, and
/// this docstring said so; the arm in `on_window_event` landed on 2026-09-07
/// with the tray's click-outside dismissal, and `TrayApp` pins across the first
/// load, a system dialog it raised, and an in-app decision in progress. Note
/// what that means for the sequencing: a change here can hide a window
/// mid-trust-prompt, which is the failure this command was written for before
/// there was a dismissal to guard against.
#[tauri::command]
fn pin_popover() {
    POPOVER_PINNED.store(true, Ordering::Release);
}

/// Open (or refocus) the full-size onboarding window. `source` rides along as
/// a query param so the flow can report whether it was a first launch or a
/// replay from Settings.
#[tauri::command]
async fn open_onboarding_window<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    source: String,
) -> Result<(), String> {
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

/// The last `cf_clearance` value fed into the engine, so the solve window can
/// tell a freshly minted cookie from the very one Cloudflare just challenged
/// (wait for the solve to mint a new value; re-feeding the stale one would
/// loop: inject -> challenge -> reopen).
///
/// Now a backstop rather than the main event: the window is incognito, so its
/// jar starts empty and anything appearing in it is new by construction. It
/// still guards the case where Cloudflare hands back the same value it just
/// rejected, which would otherwise re-enter that loop.
#[cfg(any(target_os = "macos", target_os = "windows"))]
static LAST_CF_CLEARANCE: Mutex<String> = Mutex::new(String::new());

/// Open (or refocus) the Cloudflare challenge-solve webview at the real
/// chatgpt.com and poll its cookie store for the `cf_clearance` a solved
/// interstitial mints. On capture: feed the cookie into the running engine,
/// close the window, and clear the solve latch. If the user closes the window
/// first, just clear the latch - the next challenged turn re-opens it.
///
/// The webview loads the live site the way the browser does: it honours the
/// system proxy, and its navigation is not a rewritten path, so the
/// interstitial egresses from the user's own IP.
///
/// That is also this approach's ceiling: `cf_clearance` is bound to the
/// address it was issued to, and this window can only ever mint one for the
/// user's. Which bounds what the window is FOR rather than breaking it. The
/// app's passthrough traffic leaves from that same address and is fixed by
/// the cookie; the rewritten chat turn is a separate mechanism, challenged on
/// its user-agent rather than its address, and handled upstream of here.
///
/// The captures behind both halves live with the flag they justify,
/// `engine::website_shaped_rewritten_turns`. Read them there rather than
/// restating them: the copy that used to sit in this comment spent a while
/// claiming the opposite of what the captures actually show.
///
/// So this window is not the chat turn's fix and never was. Keep it anyway:
/// without it the app's warm-up sequence 403s across the board, which kills
/// the app before it can issue a chat turn at all.
///
/// What it does NOT do is render the interstitial the engine intercepted.
/// That response is Cloudflare's answer to a POST the app made, and its
/// challenge script only runs against the origin that issued it, so there is
/// nothing to replay: the window instead makes its own request to the same
/// host wearing the same user-agent, and lets Cloudflare challenge that.
/// Which means the load has to be one Cloudflare will actually adjudicate -
/// it goes to the challenged PATH for that reason (`proxy::cf_challenged_path`),
/// and starts from an empty jar (see the `incognito` note below). Treat any
/// change to either as load-bearing.
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
    // Every exit below reports through this. It holds the latch the notify
    // claimed on our behalf and releases it on drop, so a panic anywhere in
    // the poll thread cannot leave challenge detection silently dead for the
    // rest of the process.
    let solve = gate_connect_core::proxy::CfChallengeSolve::new();
    if let Some(stale) = app.get_webview_window(CF_CHALLENGE_WINDOW) {
        eprintln!("[gate] challenge-solve: destroying an orphaned solve window");
        let _ = stale.destroy();
    }
    // The origin, used to read the cookie jar. Host-scoped, so it finds
    // `cf_clearance` whatever path the window is actually sitting on.
    let url: tauri::Url = "https://chatgpt.com"
        .parse()
        // Infallible: static, pre-validated URL.
        .expect("static chatgpt.com URL parses");
    // What to LOAD: the path Cloudflare just challenged, not the host root.
    //
    // The rule here is scoped to a path rather than to the host - a capture
    // put the managed challenge on
    // `/backend-api/sentinel/chat-requirements/prepare` while `/` loaded
    // normally - so loading the root asked a question Cloudflare had no reason
    // to answer, and the window sat waiting for an interstitial that was never
    // coming. Following the challenged turn's own path is what puts the window
    // in front of the same rule the app hit.
    //
    // Falls back to the origin when nothing has been recorded yet, which
    // should not happen (the window only opens after a challenge names a
    // path) but is a sane thing to load rather than a reason to fail.
    //
    // Joined onto the parsed origin rather than concatenated into a string,
    // and checked afterwards: the path crosses a process boundary through a
    // global, and between the two no value it could hold moves this window
    // off chatgpt.com.
    let nav_url = gate_connect_core::proxy::cf_challenged_path()
        .and_then(|path| url.join(&path).ok())
        .filter(|candidate| candidate.host_str() == Some("chatgpt.com"))
        .unwrap_or_else(|| url.clone());
    // Behind the debug switch, unlike the rest of this flow's logging: a
    // challenged path can name a conversation or a resource of the user's,
    // where every other line here carries no user data at all.
    if gate_connect_core::proxy::engine::debug_log() {
        eprintln!("[gate] challenge-solve: loading {nav_url}");
    }
    let builder = tauri::WebviewWindowBuilder::new(
        app,
        CF_CHALLENGE_WINDOW,
        tauri::WebviewUrl::External(nav_url),
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
    .visible(false)
    // Empty jar, every time. This is what makes the window a CHALLENGE
    // surface rather than a chat window.
    //
    // Sharing the app's normal webview profile looked harmless and is the
    // whole bug: that profile already holds a live chatgpt.com session and a
    // `cf_clearance` from an earlier solve, so Cloudflare waves the load
    // through and the user is shown their own conversation list. Nothing is
    // challenged, nothing new is minted, and the poll below then rejects the
    // cookie it does find as unchanged (correctly - re-feeding the value that
    // was just challenged would loop) and hangs until its deadline, which
    // spends the long no-capture cooldown on an attempt that was never given
    // anything to solve.
    //
    // Incognito starts with no cookies at all, so the same load has to be
    // adjudicated from scratch: either Cloudflare issues the interstitial -
    // which is the thing we are here to capture the answer to - or it does
    // not, and the absence is then real evidence rather than an artifact of a
    // cookie we brought with us. Anything it mints is new by construction,
    // which is also what retires the unchanged-value stall.
    //
    // The cost is that this window is signed out. That is fine and slightly
    // desirable: `cf_clearance` is bot-management state, issued to any client
    // that passes the check, and has nothing to do with the account - so the
    // window never needs, and now never sees, the user's session.
    .incognito(true);
    // Wear the app's own user-agent: a stock webview is waved through without
    // a challenge, and `cf_clearance` only exists as the result of one, so
    // without this there is nothing to capture. See
    // `proxy::chatgpt_app_user_agent`.
    //
    // None recorded yet is a reason NOT to open a window. Wearing the
    // platform default, the load classifies as some client other than the
    // app, so the engine never records its challenge as ours: the reveal
    // never fires, the window is never shown, and the attempt closes at the
    // 20s deadline reporting "Cloudflare did not challenge Gate's page" while
    // an unseen interstitial sits on screen. Reporting plainly that no window
    // opened is the honest version of the same silence, and costs the same
    // cooldown.
    let Some(app_user_agent) = gate_connect_core::proxy::chatgpt_app_user_agent() else {
        eprintln!(
            "[gate] challenge-solve: no chatgpt.com app user-agent recorded yet, so a window \
             could not be challenged as the app - not opening one"
        );
        solve.finish(SolveOutcome::WindowFailed);
        return;
    };
    let builder = builder.user_agent(&app_user_agent);
    eprintln!("[gate] challenge-solve window opening as {app_user_agent}");
    // Sampled before the build, which is what kicks the navigation off:
    // everything the poll asks about that load is `*_since(started)`, so a
    // challenge recorded in the gap between the two would read as "never
    // challenged" and close the window unseen on a ten minute cooldown.
    let started = std::time::Instant::now();
    // Built for its side effect; the poll thread below re-resolves the window
    // by label, so there is nothing to hold on to here.
    if let Err(e) = builder.build() {
        eprintln!("[gate] opening the challenge-solve window failed: {e}");
        report_backend_error("cf_challenge_window", format!("{e}"));
        solve.finish(SolveOutcome::WindowFailed);
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
        let mut last_read_error = String::new();
        let mut reported_unchanged = false;
        // Bounded, because the window can sit on a challenge it will never
        // clear: the poll would otherwise log a jar line every two seconds
        // for as long as the app runs. Generous enough for a person to read
        // an interstitial and click through it. `started` comes from before
        // the build, so nothing this thread asks about the load has a blind
        // spot in front of it.
        let deadline = started + std::time::Duration::from_secs(180);
        // How long a non-interactive challenge gets to resolve unseen before
        // we assume it wants a human. Short enough that an interactive one
        // does not feel stalled, long enough to cover a page load plus the
        // challenge round trip on a slow link.
        let reveal_at = started + std::time::Duration::from_secs(8);
        // How long to wait for Cloudflare to challenge the window's own load
        // before concluding it never will. Past this with nothing challenged,
        // the window is sitting on an ordinary chatgpt.com page and there is
        // nothing for anyone to solve, so it closes without ever being shown -
        // the alternative is putting the ChatGPT site in front of someone
        // whose message just went through fine. Comfortably longer than
        // `reveal_at` so a slow load still gets its chance.
        let no_challenge_at = started + std::time::Duration::from_secs(20);
        // How long after Cloudflare lets the window's page through a fresh
        // cookie gets to show up in the jar before the capture is declared
        // failed. Counted from the LATEST let-through load, so a page that
        // keeps reloading restarts it. The cookie is set on the very response
        // that lets the page through, so this only has to cover a few poll
        // ticks.
        const CAPTURE_GRACE: std::time::Duration = std::time::Duration::from_secs(10);
        let mut revealed = false;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            // Window gone: the user closed it (or the capture below already
            // did). Release the latch, reporting no capture so the cooldown
            // keeps the next challenge from reopening immediately. Closed
            // AFTER Cloudflare let the page through, the user was looking at
            // the origin's error page with nothing left to solve, and closing
            // it is the obvious thing to do: that is a failed capture, not an
            // unsolved check, and "finish the check" would be the wrong advice.
            let Some(window) = app.get_webview_window(CF_CHALLENGE_WINDOW) else {
                let outcome =
                    if gate_connect_core::proxy::cf_navigation_passed_since(started).is_some() {
                        SolveOutcome::Uncaptured
                    } else {
                        SolveOutcome::Unsolved
                    };
                solve.finish(outcome);
                return;
            };
            if std::time::Instant::now() >= deadline {
                eprintln!(
                    "[gate] challenge-solve: no cf_clearance after 3 minutes, giving up on this attempt"
                );
                let _ = window.close();
                solve.finish(SolveOutcome::Unsolved);
                return;
            }
            let navigation_challenged =
                gate_connect_core::proxy::cf_navigation_challenged_since(started);
            // Never challenged, so there is nothing here to solve. Close
            // without ever showing it: the window would only display an
            // ordinary chatgpt.com page to someone who is not expecting one.
            // Reported as no capture, which is honest and also starts the
            // cooldown - if this host is not challenging our navigations, the
            // next app turn should not reopen the window to find out again.
            //
            // Which of the two silences this is decides what the user is told,
            // because they have different fixes: a page Cloudflare declined to
            // challenge means the window is asking the wrong question, while a
            // load the engine never saw means the webview is not going through
            // the proxy at all and no challenge could ever be observed.
            if !navigation_challenged && std::time::Instant::now() >= no_challenge_at {
                let outcome = if gate_connect_core::proxy::cf_navigation_seen_since(started) {
                    SolveOutcome::NotChallenged
                } else {
                    SolveOutcome::NotProxied
                };
                eprintln!(
                    "[gate] challenge-solve: nothing to solve ({outcome:?}) - closing without \
                     showing the window"
                );
                let _ = window.close();
                solve.finish(outcome);
                return;
            }
            // Nothing captured while hidden: either the challenge wants a
            // click, or being concealed stopped it running. Both are fixed by
            // putting it in front of the user, and this is the one moment in
            // the flow where taking focus is warranted.
            //
            // Gated on the window actually HAVING a challenge on screen. "No
            // cookie yet" was the old trigger and it cannot tell an unsolved
            // interstitial from a page that loaded perfectly well, so a turn
            // that succeeded could still be followed by the ChatGPT site
            // appearing over the user's work a few seconds later.
            //
            // `set_focus` alone does not always do it. Gate is a background
            // process at this point - the user is in the ChatGPT app, which is
            // what got challenged - and neither platform lets a background app
            // take the foreground on request: Windows refuses and flashes the
            // taskbar button instead, macOS needs `orderFrontRegardless` (see
            // the popover's own helper, written for this same reason).
            //
            // Kept to `show` + `set_focus` plus that macOS helper, which
            // touches AppKit directly rather than the runtime. Raising the
            // window topmost from here was tried and rejected: each such call
            // dispatches to the main thread and blocks, and this thread
            // exists precisely because `cookies_for_url` deadlocks on Windows
            // when the main thread is busy (wry#583). If a raise turns out to
            // be needed it belongs on the BUILDER, where it costs no runtime
            // dispatch.
            //
            // And gated on the challenge still BEING there. `navigation_challenged`
            // stays true once Cloudflare has let the page through, so a
            // challenge that cleared on its own while hidden would otherwise
            // be revealed anyway - with a notification asking the user to
            // accept a screen that is gone, over the origin's error page.
            // What happens after a pass is the capture check's business below.
            let passed = gate_connect_core::proxy::cf_navigation_passed_since(started).is_some();
            if !revealed
                && navigation_challenged
                && !passed
                && std::time::Instant::now() >= reveal_at
            {
                eprintln!("[gate] challenge-solve: not resolved on its own, showing the window");
                // Say why a Cloudflare page just appeared over the ChatGPT
                // app. A system notification rather than a note in the page:
                // anything injected into the window sits beside Cloudflare's
                // own script, and a window wearing the app's user-agent should
                // not carry a marker saying it is Gate.
                {
                    use tauri_plugin_notification::NotificationExt;
                    let _ = app
                        .notification()
                        .builder()
                        .title("Gate Connect")
                        .body("Accept the verification screen to continue with chat")
                        .show();
                }
                let _ = window.show();
                let _ = window.set_focus();
                #[cfg(target_os = "macos")]
                order_front_regardless(&window);
                revealed = true;
            }
            // A failed read is logged, not flattened into an empty jar: the two
            // look identical from the window's behaviour, and only one of them
            // means Cloudflare never minted anything. Said once per distinct
            // error, like the jar line below.
            let (cookies, read_ok) = match window.cookies_for_url(url.clone()) {
                Ok(cookies) => (cookies, true),
                Err(e) => {
                    let error = format!("{e}");
                    if error != last_read_error {
                        eprintln!("[gate] challenge-solve: reading the jar failed: {error}");
                        last_read_error = error;
                    }
                    (Vec::new(), false)
                }
            };
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
            // Not after a failed read: its empty list would print the same
            // `jar: []` as a jar Cloudflare minted nothing into.
            if read_ok && names != last_names {
                eprintln!("[gate] challenge-solve jar: [{names}]");
                last_names = names;
            }
            let captured = cookies
                .into_iter()
                .find(|c| c.name() == "cf_clearance")
                .map(|c| c.value().to_string())
                .filter(|value| !value.is_empty());
            let fresh = match captured {
                // A cookie identical to the one just challenged is not a
                // solve; re-feeding it would loop the window open. Said once
                // - the poll re-reads the same jar until the deadline.
                Some(value) if value == challenged => {
                    if !reported_unchanged {
                        eprintln!(
                            "[gate] challenge-solve: cf_clearance present but unchanged, waiting"
                        );
                        reported_unchanged = true;
                    }
                    None
                }
                other => other,
            };
            let Some(value) = fresh else {
                // Cloudflare has let the page through, so there is nothing on
                // screen left to solve - on a `/backend-api/...` path the window
                // is now showing the origin's `{"detail":"Unauthorized"}` to a
                // signed-out load. The cookie normally lands with that reload;
                // past `CAPTURE_GRACE` without one, the capture has failed and
                // the user cannot help it along, so close rather than leave an
                // error page up until the deadline. Checked after the jar read,
                // so the tick that trips this still had its chance to capture.
                if let Some(passed) = gate_connect_core::proxy::cf_navigation_passed_since(started)
                {
                    if passed.elapsed() >= CAPTURE_GRACE {
                        eprintln!(
                            "[gate] challenge-solve: Cloudflare let the window through but no new \
                             cf_clearance appeared - closing"
                        );
                        let _ = window.close();
                        solve.finish(SolveOutcome::Uncaptured);
                        return;
                    }
                }
                continue;
            };
            if let Ok(mut last) = LAST_CF_CLEARANCE.lock() {
                *last = value.clone();
            }
            gate_connect_core::proxy::manager().refresh_cf_clearance(&value);
            eprintln!("[gate] challenge-solve: captured cf_clearance, fed to the engine");
            let _ = window.close();
            solve.finish(SolveOutcome::Captured);
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
fn reveal_popover_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    POPOVER_VISIBLE.store(false, Ordering::Release);
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    // Same correction as the single-instance callback: this is the tray's
    // "Expand app" and the onboarding window's close handler, both of which
    // have to produce a window the user can see. A minimized one is not that,
    // and `show` alone does not un-iconify it.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
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
fn reveal_popover<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    reveal_popover_window(&app);
}

/// How long after a blur-dismiss a tray-icon click still counts as part of it.
///
/// Long enough to cover the blur-then-click ordering on a slow frame, short
/// enough that a deliberate re-open a moment later still opens.
const BLUR_HIDE_GRACE_MS: u64 = 400;

/// Milliseconds since the epoch, for the blur-dismiss race guard.
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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

/// Hand a "switch organization" request from the tray popover to the main
/// window, which owns the organization selector.
///
/// The popover shows which organization is selected (its footer names it) and
/// AG-582 asks it to open the selector - but the selector is a dialog over the
/// 1024px window, with the orgs read, an in-flight state and a failure path.
/// Rebuilding it at 400px would be a second surface over one setting, and two
/// selectors that could disagree about which org is active is the divergence
/// principle 2 warns about.
///
/// So the popover asks, and the window answers. Reveal first, then emit, the
/// same order `request_quit` uses: the main window's webview is created at
/// startup and its listener is already mounted, so the event cannot outrun it.
///
/// **This is a single-purpose stand-in for a general navigation intent.** The
/// tray also owes "open this tool's detail" and "open this alert" (AG-584), and
/// those want a payload rather than another bespoke command. When that lands,
/// this collapses into it.
#[tauri::command]
fn request_switch_org<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    reveal_popover_window(&app);
    let _ = app.emit("switch-org-requested", ());
    if let Some(tray) = app.get_webview_window("tray") {
        // Same hand-over as Expand app: the popover asked the window to take
        // over, so it gets out of the way.
        let _ = tray.hide();
        POPOVER_VISIBLE.store(false, Ordering::Release);
    }
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

/// Tray "Quit": the same quit as [`quit_app`], with no question in front of it.
///
/// There used to be one. On macOS and Windows this revealed the window and let
/// the frontend ask whether to disconnect the tools first; since the "quit
/// without disconnecting" row went, the question had one answer, and every
/// quit now gives it.
fn request_quit<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    tauri::async_runtime::spawn(quit_app(app.clone()));
}

/// Quit, putting every tool back on its own settings on the way out.
///
/// **Linux exits outright, and that is correct.** It looks like the teardown
/// being skipped on one platform, and it is not: there the engine is a
/// DETACHED helper daemon that outlives this process (see the note at
/// `LAUNCH_AT_LOGIN`-adjacent code, "on Linux the engine lives in a detached
/// helper daemon", and "Linux has no exit-time safe point - the RunEvent::Exit
/// handler is macOS/Windows-only"). Quitting the GUI there stops nothing:
/// routing continues and no config is left aimed at a dead relay. Tearing it
/// down would take away routing the user never asked to lose. A gate like this
/// was removed once, reasoning from AG-596, and it was a regression. Do not
/// remove it again.
///
/// On macOS and Windows the sweep runs here, before the exit, rather than only
/// in the `RunEvent::Exit` handler: this side can still fire a notification,
/// and a rewrite of somebody's config file is worth a sentence. The exit
/// handler runs the same sweep for the exits that never reach this command
/// (Cmd+Q, a logout, a shutdown), and skips it after this one.
///
/// **Once per process** ([`claim_quit_teardown`]). A second Quit while the
/// first is sweeping returns without doing anything, because the first is
/// about to exit. Running it anyway waited out the first one's guard, reported
/// "Failed to remove Gate" over a quit that was succeeding, and exited in the
/// middle of the first one's writes.
#[tauri::command]
async fn quit_app<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        use tauri_plugin_notification::NotificationExt;
        if !claim_quit_teardown() {
            return;
        }
        let outcome = tauri::async_runtime::spawn_blocking(disconnect_tools_for_quit)
            .await
            .unwrap_or_else(|e| Err(format!("join error: {e}")));
        if let Err(e) = &outcome {
            eprintln!("[gate] putting tools back for quit failed: {e}");
        }
        if let Some(body) = quit_notice_body(&outcome, || {
            gate_connect_core::preferences::load().notifications
        }) {
            let _ = app
                .notification()
                .builder()
                .title("Gate Connect")
                .body(&body)
                .show();
        }
    }
    app.exit(0);
}

/// Quit-time teardown: snapshot + disconnect every enabled provider AND the
/// managed standalone tools no provider maps, so all the CLI tools fall back
/// to their original settings while Gate Connect is closed, WITHOUT touching
/// the routing intent - the startup restore reapplies both snapshots the next
/// time the app runs.
///
/// Returns what it found and what it could **not** return to its own settings.
/// `Err` means it could not run at all (another routing operation held the
/// guard), so nothing was put back.
///
/// Shared with the `RunEvent::Exit` handler, so a quit means one thing however
/// it arrives. Callers go through [`claim_quit_teardown`] first.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn disconnect_tools_for_quit() -> Result<gate_connect_core::provider::QuitTeardown, String> {
    let teardown = gate_connect_core::provider::snapshot_and_disable_everything_for_exit()
        .map_err(|e| format!("{e:#}"))?;
    // This is a disconnect, not a routing-off: every exit takes Gate out of
    // the path, so nothing starts the passthrough listener again. It is
    // drained rather than stopped, because a tool already running still
    // holds its address and would otherwise fail until reopened - see
    // `proxy::forwarder::drain`.
    gate_connect_core::proxy::forwarder::drain();
    Ok(teardown)
}

/// Whether the quit teardown has been claimed by anyone in this process yet.
#[cfg(any(target_os = "macos", target_os = "windows"))]
static QUIT_TEARDOWN_CLAIMED: AtomicBool = AtomicBool::new(false);

/// Claim the quit teardown: `true` exactly once per process, for whichever of
/// [`quit_app`] and the `RunEvent::Exit` handler gets there first.
///
/// Once is not an optimisation. A second sweep after a partial first one is not
/// a no-op: the first leaves a provider it could not finish reading as enabled,
/// and the second then records the members the first just disconnected as
/// members that were already off, so the next launch leaves them off. It is
/// also a second round of trust probes on the logout path.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn claim_quit_teardown() -> bool {
    !QUIT_TEARDOWN_CLAIMED.swap(true, Ordering::AcqRel)
}

/// What [`quit_app`] says on its way out, if anything. Split out of the
/// command, which only runs on macOS and Windows, so the one rule that matters
/// here is pinned by a test on every platform.
///
/// A clean teardown is information, gated on the notifications preference: a
/// switch the user turned off has to actually stop something. A teardown that
/// found no tool naming Gate says nothing at all, because nothing was removed.
/// A teardown that left tools on Gate's settings is **not** gated. The window,
/// the tray and the process are all gone by the time it lands, so it is the only
/// way the user learns a tool still points at Gate. `notifications` is asked
/// only on the clean branch with something removed, so every other quit reads
/// no preferences file.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
fn quit_notice_body(
    outcome: &Result<gate_connect_core::provider::QuitTeardown, String>,
    notifications: impl FnOnce() -> bool,
) -> Option<String> {
    match outcome.as_ref().map(|t| (t.managed, t.failed.as_slice())) {
        Ok((0, [])) => None,
        Ok((_, [])) => notifications().then(|| "Gate removed from tool configs".to_string()),
        // Actionable over explanatory: the fix is the same whatever the tool
        // does next.
        Ok((_, [one])) => Some(format!(
            "Failed to remove Gate from the {one} config. Edit it by hand."
        )),
        Ok((_, failed)) => Some(format!(
            "Failed to remove Gate from the {} configs. Edit them by hand.",
            join_names(failed)
        )),
        Err(_) => Some("Failed to remove Gate from tool configs. Edit them by hand.".to_string()),
    }
}

/// "A", "A and B", "A, B and C" - the list is read by a person, and a bare
/// comma-join reads as a fragment at two items.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The command table, shared by the app and by `examples/ui-harness.rs`.
///
/// Generic over the runtime so the harness can register this identical list
/// on `tauri::test::MockRuntime`. A UI e2e that drove a hand-maintained copy
/// would be asserting against a backend the app does not have, and the copy
/// would rot from the first command added on either side.
pub fn invoke_handler<R: tauri::Runtime>(
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    // The proxy subsystem (and its commands) only exists on the three
    // desktop OSes; the handler forks on that single axis. Forking the
    // whole generate_handler! invocation (rather than per-item cfg)
    // preserves Tauri's compile-time arg/return type-checking.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        tauri::generate_handler![
            list_tools,
            tool_versions,
            tool_status,
            connect_tool,
            disconnect_tool,
            get_account,
            get_account_key_prefix,
            save_account,
            clear_account,
            switch_gateway,
            oauth_begin_login,
            oauth_cancel_login,
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
            set_org,
            app_platform,
            os_name,
            diagnostics,
            unpin_popover,
            pin_popover,
            open_onboarding_window,
            reveal_popover,
            request_switch_org,
            quit_app,
            list_providers,
            proxy_status,
            proxy_browser_store,
            proxy_enable,
            proxy_disable,
            proxy_set_domain,
            hermes_upstream_coverage,
            proxy_set_env_export,
            proxy_trust_ca,
            proxy_untrust_ca,
            launch_at_login_status,
            set_launch_at_login,
            get_preferences,
            set_notifications,
            set_share_diagnostics,
            analytics_milestone_claim,
            cowork_setting_check,
            analytics_identity,
            set_analytics_identity,
            record_auto_enabled_domains,
            read_auto_enabled_domains,
            install_id,
            device_name,
            set_device_name,
            set_updater_relaunching,
            routed_clients_stale,
            routing_startup_pending,
            routing_verdicts,
            teardown_report,
            running_agents,
            close_running_agents,
            reopen_running_agents,
            drain_backend_errors,
            security_feed_state,
            security_feed_history_ok,
            security_feed_recent,
            security_feed_retry,
            set_security_notification_sound,
        ]
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        tauri::generate_handler![
            list_tools,
            tool_versions,
            tool_status,
            connect_tool,
            disconnect_tool,
            get_account,
            get_account_key_prefix,
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
            set_org,
            app_platform,
            os_name,
            diagnostics,
            unpin_popover,
            pin_popover,
            open_onboarding_window,
            reveal_popover,
            request_switch_org,
            quit_app,
            list_providers,
            set_updater_relaunching,
            get_preferences,
            set_notifications,
            set_share_diagnostics,
            analytics_milestone_claim,
            cowork_setting_check,
            analytics_identity,
            set_analytics_identity,
            record_auto_enabled_domains,
            read_auto_enabled_domains,
            install_id,
            device_name,
            set_device_name,
            drain_backend_errors,
        ]
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // A second launch landed here, in the already-running instance.
            // Reveal the popover, mirroring the "show" tray-menu handler.
            if let Some(window) = app.get_webview_window("main") {
                // Every desktop platform, not Linux alone. `show` un-hides a
                // window; it does not un-iconify one, and `set_focus` on a
                // minimized window focuses it minimized. So on Windows a second
                // launch of the executable kept the single instance - correctly -
                // and left the window in the taskbar with nothing on screen,
                // which reads as a launch that did nothing at all. Read-only
                // `IsIconic` stayed true across both `WindowStyle Hidden` and
                // `WindowStyle Normal`; Alt+Tab or the tray's Expand app were the
                // only ways back.
                //
                // The Linux-only guard was written for Linux's own reason and
                // never revisited when this became the main window's re-entry
                // point. Harmless where a window is not minimized: `unminimize`
                // is a no-op then.
                #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
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
        // relies on the tray tooltip). See the refresh loop in `setup` and
        // `signal_session_dead`.
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
        .invoke_handler(invoke_handler())
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
            // Click-outside dismisses the popover, which is the convention for a
            // surface anchored to a tray icon and what this window is.
            //
            // TRAY ONLY, and after the guards above: the main and onboarding
            // windows are ordinary windows that must survive losing focus, and
            // the challenge-solve webview returns before reaching here.
            //
            // [`POPOVER_PINNED`] is what makes this safe, and it is why the pin
            // machinery was kept when it had no reader. Two things blur the
            // popover without the user having clicked away from it: a system
            // dialog the popover itself raised (the certificate trust prompt,
            // the keychain unlock on first load), and an in-app dialog the user
            // is mid-decision on. Dismissing then would take away the copy
            // telling them what to click, or lose a config-overwrite
            // confirmation because they glanced at another window. The frontend
            // pins across both.
            if let WindowEvent::Focused(false) = event {
                if window.label() == "tray" && !POPOVER_PINNED.load(Ordering::Acquire) {
                    // Whether the popover was still up when focus left decides
                    // whether this blur is a dismissal or the echo of one.
                    //
                    // Already hidden means something hid it ON PURPOSE and this
                    // blur is the consequence: Escape, Expand app, Switch
                    // organization, Quit. Stamping the race mark then would
                    // consume the user's next tray-icon click, so pressing
                    // Escape and reaching for the icon - the natural retry -
                    // would do nothing the first time. The mark exists for one
                    // race only, a click on the icon that blurs the popover
                    // before the click itself arrives, and that race can only
                    // happen while the window is visible.
                    let was_visible = window.is_visible().unwrap_or(false);
                    let _ = window.hide();
                    POPOVER_VISIBLE.store(false, Ordering::Release);
                    if was_visible {
                        BLUR_HIDE_AT_MS.store(now_millis(), Ordering::Release);
                    }
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
            // Before any window loads and before anything below writes to the
            // data dir, so the milestone store judges a fresh install fresh and
            // an upgraded one legacy (AG-960). Best-effort: a store that cannot
            // be created only means the webview's claims fail, which it reads
            // as "do not send".
            if let Err(e) = gate_connect_core::analytics::init() {
                eprintln!("[gate] analytics milestone store unavailable: {e:#}");
            }

            // Open the crash-restart session before anything that could itself
            // crash, and decide from the last one whether to keep asking the OS
            // to bring us back. macOS and Windows only: Linux has no exit
            // handler to close the session with, so every start would read as
            // unclean, and its engine is a detached daemon that a GUI crash
            // does not strand anyway.
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            match gate_connect_core::crash_restart::record_start() {
                Ok(verdict) => {
                    if verdict.previous_was_unclean {
                        eprintln!(
                            "[gate] previous session ended without a clean exit ({} in a row)",
                            verdict.unclean_streak
                        );
                    }
                    #[cfg(target_os = "macos")]
                    if verdict.exhausted {
                        disarm_crash_restart(app.handle(), verdict.unclean_streak);
                    } else {
                        arm_crash_restart(app.handle());
                    }
                    #[cfg(target_os = "windows")]
                    if verdict.exhausted {
                        eprintln!(
                            "[gate] not registering automatic restart: {} launches in a row ended before the app was up",
                            verdict.unclean_streak
                        );
                    } else {
                        register_application_restart();
                    }
                }
                // Losing the bookkeeping costs a relaunch after the next crash,
                // which is what every build before this one did.
                Err(e) => eprintln!("[gate] crash-restart bookkeeping failed: {e:#}"),
            }

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
                            report_backend_failure("restore_routing", &e);
                        }
                    }
                });

                // ChatGPT app Cloudflare-challenge fix: the engine detects a
                // challenge answering a chatgpt.com app turn - marked
                // `cf-mitigated: challenge`, or an unmarked 403/503 of HTML,
                // rewritten or passthrough alike - and, because the app shell
                // cannot render an interstitial, this observer opens a
                // one-time solve webview at the path that was challenged;
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
                // Both verdicts reach the token watch: `Recovered` pushes the
                // new token, `Dead` pushes the empty one. `Unchanged` pushes
                // nothing, and a relay request waiting on it stops when the
                // guard above drops.
                true
            });

            // Routed traffic left for the gateway, from these tools. The
            // window's activity reads refresh on this and on nothing else
            // periodic: the endpoint is throttled per source address, so a
            // poll would spend a budget shared with everyone behind the same
            // egress, while this fires only when a read is certain to find
            // something new. Already coalesced in the core - at most one
            // report per tool every 30s, and only once its burst has gone
            // quiet - so the emit is as-is. Payload: the tools' slugs, `null`
            // for a sender the relay could not name. Not gated behind a
            // window, like `tools-changed` below; on Linux it never fires,
            // for the reason the auth observer above never does.
            let traffic_handle = app.handle().clone();
            gate_connect_core::proxy::set_traffic_observer(move |tools| {
                let _ = traffic_handle.emit("traffic-observed", tools);
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
                        } else {
                            // Same rewrite as every other enable; see
                            // `arm_crash_restart`.
                            #[cfg(target_os = "macos")]
                            arm_crash_restart(app.handle());
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
                    let _settled = StartupEnableSettled(handle.clone());
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
                        gate_connect_core::startup::SessionVerdict::NotOauth
                        | gate_connect_core::startup::SessionVerdict::Unavailable => {}
                    }

                    // An `opencode.ai` domain an older build turned on, left
                    // behind on a machine OpenCode is not on. Before routing
                    // comes up, which reads the flag; `StartupEnableSettled`
                    // emits on every way out of this thread, so the rail
                    // redraws without the row.
                    if let Err(e) =
                        gate_connect_core::integrations::opencode::switch_off_orphaned_domain()
                    {
                        eprintln!("[gate] switching off the orphaned OpenCode domain failed: {e:#}");
                    }

                    // If a previous session left the system proxy on (unclean
                    // quit / crash), revert it first so HTTPS isn't routed at a
                    // dead loopback port. A clean disable leaves nothing to do.
                    if let Err(e) = gate_connect_core::proxy::manager().reconcile_on_startup() {
                        eprintln!("proxy startup reconcile failed: {e}");
                        report_backend_failure("restore_routing", &e);
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
                            report_backend_failure("restore_routing", &e);
                        }
                        complete_pending_autostart_disable(&handle);
                        handle.exit(0);
                        return;
                    }

                    // Routing follows the app: it is on for exactly as long
                    // as Gate Connect is running, so every launch - login
                    // item, manual, updater relaunch - enables it. There is no
                    // persisted choice to consult, because there is no longer
                    // a switch for the user to have made one with.
                    //
                    // This used to read the routing intent and return early
                    // when it was false. The intent file still exists and
                    // `routing::enable`/`disable` still keep it current, but it
                    // now records what is true *right now* rather than a choice
                    // to restore - `autostart_optout::record_disable` reads it
                    // as exactly that, and diagnostics reports it.
                    //
                    // The way off is the exit, which is where it already was:
                    // every quit takes Gate out of every tool's config
                    // (`disconnect_tools_for_quit`) and `RunEvent::Exit` then
                    // `disable_quiet`s the system proxy. This change is only about the way *on* no longer
                    // being a thing the user sets.
                    //
                    // A launch that cannot complete the enable unattended (a
                    // prompt we will not raise, an engine that cannot bind)
                    // still degrades quietly - see the error arm below. That is
                    // the one state where the app runs and routing does not,
                    // and the panes report it per tool as `not-routing`.
                    //
                    // No account is not that state, and is the one case still
                    // gated here. `routing::enable` needs a gateway to point
                    // the relay at, so on a machine that has never signed in
                    // it fails every time - and the error arm reports a
                    // backend failure, which would put "Couldn't restore
                    // routing at startup" in front of a first-run user who has
                    // not been asked for an account yet. The setup flow's own
                    // "Turn on routing" step is what enables it the first
                    // time; every launch after that comes through here.
                    if gate_connect_core::account::load_base_url()
                        .ok()
                        .flatten()
                        .is_none()
                    {
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
                            // Reconnecting can rewrite Codex's config; the
                            // daemon picks it up only if refreshed.
                            refresh_codex_daemon_when_idle("startup");
                            for w in warnings {
                                eprintln!(
                                    "[gate] startup auto-enable: {} failed: {:#}",
                                    w.component, w.error
                                );
                                report_backend_failure(w.component, &w.error);
                            }
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
                                prior_port.as_ref().map(|p| *p != state.port).unwrap_or(false);
                            // On macOS and Windows a moved engine usually strands
                            // nothing: tool configs, the export and the PAC name the
                            // forwarder, which reads the engine's port per
                            // connection. Only a config that still names the old
                            // engine port is left dialing it, so only that one is
                            // worth the notice. An unknown prior port keeps the
                            // old answer, since there is nothing to compare.
                            #[cfg(any(target_os = "macos", target_os = "windows"))]
                            let engine_moved = engine_moved
                                && prior_port
                                    .as_ref()
                                    .ok()
                                    .copied()
                                    .flatten()
                                    .map(gate_connect_core::provider::managed_tool_names_port)
                                    .unwrap_or(true);
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
                            report_backend_failure("restore_routing", &e);
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
                            // Independent of the connection state on purpose:
                            // the stream can be Live with its history missing,
                            // which is the case that used to render as an empty
                            // feed. See `Update::History`.
                            Update::History { ok } => {
                                let _ = feed_handle.emit("security-feed-history", ok);
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
                    // `session_reading` silently refreshes a stale token
                    // (persisting it) and is `Live` only for a usable session;
                    // push its token into the running engine (a no-op when
                    // routing is off). "" means no usable session: the engine
                    // then refuses routed requests as signed out - an OAuth
                    // account holds no key to fall back to - matching the
                    // signed-out state the UI derives from oauth_status.
                    let reading = gate_connect_core::oauth::session_reading();
                    let token = match &reading {
                        gate_connect_core::oauth::SessionReading::Live(t) => {
                            t.access_token.clone()
                        }
                        _ => String::new(),
                    };
                    gate_connect_core::proxy::manager().refresh_token(&token);

                    // Raise (or clear) the tray attention signal on the
                    // signed-in↔dead edge. "Dead" means a stored session was
                    // refused (revoked, expired refresh token, rejected by the
                    // gateway) - NOT a deliberate sign-out, and not a refresh
                    // that got no answer; see `session_dead_after_tick`.
                    // Redraw only on a change so the tray isn't rewritten
                    // every 30s.
                    let dead = session_dead_after_tick(
                        &reading,
                        || {
                            gate_connect_core::oauth::current().map_or_else(
                                |e| gate_connect_core::oauth::is_corrupt_bundle(&e),
                                |bundle| bundle.is_some(),
                            )
                        },
                        SESSION_NEEDS_SIGNIN.load(Ordering::Relaxed),
                    );
                    if SESSION_NEEDS_SIGNIN.swap(dead, Ordering::Relaxed) != dead {
                        let running = gate_connect_core::proxy::manager()
                            .status()
                            .map(|s| s.running)
                            .unwrap_or(false);
                        update_tray_status(&refresh_handle, running);
                        // Tell a mounted window too, as `signal_session_dead`
                        // does: otherwise one that stays focused never re-reads
                        // the session and keeps its old screen up.
                        if dead {
                            let _ = refresh_handle.emit("session-signin-required", ());
                        }
                        // First tick that finds the session dead: nudge the user
                        // with a system notification on macOS + Linux, so the
                        // dead session is noticed even when the popover is closed
                        // and the menu-bar/tray dot is out of the user's eyeline.
                        // Fired once per death by the edge guard above - or not
                        // here at all, when `signal_session_dead` took the edge
                        // first after a refused call; it notifies under the same
                        // switch.
                        #[cfg(any(target_os = "macos", target_os = "linux"))]
                        if dead
                            && gate_connect_core::preferences::load().notifications
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
                    // Acted on as an EDGE, not a level, by
                    // `proxy::refused_since_last_look`, which carries the rules:
                    // a rise is a new refusal, the first reading only seeds, and a
                    // reading below the baseline is a restarted daemon counting
                    // from zero, whose every refusal is new. A tick that could
                    // not read the counter loses nothing.
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
                            gate_connect_core::proxy::refused_since_last_look(
                                last_refusals,
                                refusals,
                            );
                        last_refusals = Some(refusals);
                        if refused_since_last_tick {
                            eprintln!(
                                "[gate] the helper daemon's engine reports the gateway refusing \
                                 our bearer; re-verifying the session"
                            );
                            // Skipped when a routing sweep is already re-checking:
                            // it is forcing the same refresh this would.
                            if let Some(_check) = gate_connect_core::proxy::try_begin_gate_auth_check() {
                                recheck_gate_session(&refresh_handle);
                            }
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
                            } else if now_millis().saturating_sub(
                                BLUR_HIDE_AT_MS.load(Ordering::Acquire),
                            ) < BLUR_HIDE_GRACE_MS
                            {
                                // The blur-dismiss just hid it, and that blur was
                                // this very click landing on the tray icon. Re-
                                // opening here is how the icon would lose the
                                // ability to close the popover at all. Consume the
                                // click and clear the mark, so an immediate second
                                // click still opens.
                                BLUR_HIDE_AT_MS.store(0, Ordering::Release);
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
                // Put every tool back on its own settings on *every* exit, not
                // only the one that goes through `quit_app`: the same teardown
                // (`disconnect_tools_for_quit`), so a quit means one thing
                // however it arrives.
                //
                // `quit_app` is reached by the tray's Quit, the window menu's
                // and the crash screen's, and by nothing else: macOS Cmd+Q
                // comes from Tauri's default app menu, and a logout or a
                // shutdown comes from the OS, and both land here having touched
                // none of our own code. This used to revert only the tools
                // whose address dies with this process, leaving the forwarder's
                // tools pointed at Gate and working with nothing reading their
                // traffic. After `quit_app` it is skipped: the teardown runs
                // once per process (`claim_quit_teardown`), because a second
                // pass after a partial first one is not a no-op.
                //
                // Before `disable_quiet` below, deliberately: the provider half
                // turns domains off in the engine, which has to still be up.
                //
                // Not on an updater relaunch: the app is coming straight back,
                // so the sweep would rewrite every tool's config and the
                // restore would undo it, costing each of them a restart for
                // nothing.
                //
                // Deliberately does not *veto* the exit. `ExitRequested` can be
                // prevented - `code` is `None` exactly when something outside
                // our own code asked to quit, so Cmd+Q could be routed into a
                // dialog. It is not, because that event also carries a logout
                // and a shutdown, and an app that puts a dialog in front of
                // those is an app that hangs the user's logout. For the same
                // reason the sweep gives up rather than waiting on another
                // routing operation.
                //
                // Unwound rather than trusted not to panic: everything below it
                // has to run, `disable_quiet` above all, and the sweep is the
                // largest block on this path.
                //
                // No notification either. `quit_app` can fire one because it
                // runs before the exit; by the time this runs the process is
                // going away and a notification would be a promise we cannot
                // keep.
                if !UPDATER_RELAUNCHING.load(Ordering::Acquire) && claim_quit_teardown() {
                    match std::panic::catch_unwind(disconnect_tools_for_quit) {
                        Ok(Ok(teardown)) if teardown.failed.is_empty() => {}
                        Ok(Ok(teardown)) => eprintln!(
                            "[gate] {} could not be put back on their own settings on exit",
                            join_names(&teardown.failed)
                        ),
                        Ok(Err(e)) => {
                            eprintln!("[gate] putting tools back on exit failed: {e}")
                        }
                        Err(_) => eprintln!("[gate] putting tools back on exit panicked"),
                    }
                }
                // Reaching this event at all is what
                // makes the exit clean. Recording it clears the unclean streak,
                // so a user who quits normally after a crash gets the restart
                // policy back rather than carrying the streak forever.
                if let Err(e) = gate_connect_core::crash_restart::record_clean_exit() {
                    eprintln!("[gate] recording clean exit failed: {e:#}");
                }
                if let Err(e) = gate_connect_core::proxy::manager().disable_quiet() {
                    eprintln!("[gate] reverting proxy on exit failed: {e}");
                }
                // The engine's relay goes with this process, parked or not, so
                // the forwarder should stop asking its port whether it is up.
                gate_connect_core::proxy::forget_engine_relay_port();
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
fn update_tray_status<R: tauri::Runtime>(app: &tauri::AppHandle<R>, proxy_on: bool) {
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
fn update_tray_tooltip<R: tauri::Runtime>(app: &tauri::AppHandle<R>, proxy_on: bool) {
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
///
/// Main thread only, and it puts itself there. Unlike the Tauri calls beside
/// it (`show`, `set_focus`, `unminimize`), which post to the event loop from
/// any thread, the two messages below go straight to the NSWindow on the
/// calling thread, and AppKit traps window ordering off the main thread
/// ("Must only be used from the main thread", SIGILL). `request_quit` hit
/// exactly that when it still raised a quit dialog: it probed tool configs on a
/// blocking thread and revealed the dialog from the same thread, so quitting
/// with a connected tool crashed the app before `RunEvent::Exit` could revert
/// the system proxy. The hop is a
/// post rather than a wait, so it never blocks the thread that called it, and
/// it can only fail once the event loop has shut down - which is why the `Err`
/// is dropped: by then there is no window left to raise.
#[cfg(target_os = "macos")]
fn order_front_regardless<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;

    if objc2::MainThreadMarker::new().is_none() {
        let window = window.clone();
        let _ = window
            .app_handle()
            .clone()
            .run_on_main_thread(move || order_front_regardless(&window));
        return;
    }

    let Ok(ns_window_ptr) = window.ns_window() else {
        return;
    };
    if ns_window_ptr.is_null() {
        return;
    }

    // SAFETY: on the main thread, by the guard above. Both messages are
    // documented public NSWindow selectors taking no arguments and returning
    // void; the pointer comes from this window's own `ns_window()`, which
    // errors rather than answering for a window that is gone, and is
    // null-checked before it is messaged.
    unsafe {
        let ns_window: *mut AnyObject = ns_window_ptr.cast();
        let () = msg_send![ns_window, orderFrontRegardless];
        let () = msg_send![ns_window, makeKeyWindow];
    }
}

#[cfg(test)]
mod tests {

    /// An offline tick is no verdict: it keeps the flag where it was, so a
    /// machine that wakes without a network is not told its session expired,
    /// and one already known dead stays dead until a real answer comes.
    #[test]
    fn a_refused_session_is_dead_and_no_answer_keeps_the_flag() {
        use super::session_dead_after_tick;
        use gate_connect_core::oauth::{OAuthTokens, SessionReading};
        let live = SessionReading::Live(OAuthTokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            id_token: None,
            expires_at_unix: 0,
            client_id: String::new(),
        });
        assert!(!session_dead_after_tick(&live, || true, true));
        assert!(session_dead_after_tick(
            &SessionReading::SignedOut,
            || true,
            false
        ));
        // A deliberate sign-out cleared the bundle.
        assert!(!session_dead_after_tick(
            &SessionReading::SignedOut,
            || false,
            true
        ));
        assert!(!session_dead_after_tick(
            &SessionReading::Unavailable,
            || true,
            false
        ));
        assert!(session_dead_after_tick(
            &SessionReading::Unavailable,
            || true,
            true
        ));
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn codexs_app_server_daemon_is_not_a_running_codex() {
        use std::ffi::OsString;
        let cmd = |args: &[&str]| args.iter().map(OsString::from).collect::<Vec<_>>();
        let daemon_exe = std::path::Path::new(
            "/Users/me/.codex/packages/app-server-daemon/releases/0.159.2/bin/codex",
        );
        // Both of the daemon's processes, as `ps` shows them on macOS.
        for args in [
            cmd(&[
                daemon_exe.to_str().unwrap(),
                "app-server",
                "daemon",
                "pid-update-loop",
            ]),
            cmd(&[
                daemon_exe.to_str().unwrap(),
                "app-server",
                "--listen",
                "unix://",
                "--managed-daemon",
            ]),
        ] {
            assert!(
                !walk_yields("codex", Some(daemon_exe), &args, &["codex"]),
                "{args:?} is the app server, not a session"
            );
        }
        // A real session still counts, including one whose prompt says app-server.
        assert!(walk_yields(
            "codex",
            Some(daemon_exe),
            &cmd(&["codex"]),
            &["codex"]
        ));
        assert!(walk_yields(
            "codex",
            Some(daemon_exe),
            &cmd(&["codex", "exec", "explain the app-server flag"]),
            &["codex"]
        ));
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn hermes_is_found_by_its_script_not_its_interpreter() {
        use std::ffi::OsString;
        let cmd = |args: &[&str]| args.iter().map(OsString::from).collect::<Vec<_>>();
        assert!(is_python_name("python"));
        assert!(is_python_name("Python"));
        assert!(is_python_name("python3.11"));
        assert!(!is_python_name("hermes"));
        // The launcher's exec, and the venv entry point.
        assert!(is_hermes_command(&cmd(&[
            "/Users/me/.hermes/hermes-agent/venv/bin/python",
            "/Users/me/.hermes/hermes-agent/hermes",
            "chat",
        ])));
        assert!(is_hermes_command(&cmd(&["python3", "/venv/bin/hermes"])));
        assert!(is_hermes_command(&cmd(&[
            "python",
            "-u",
            "/opt/hermes-agent/hermes"
        ])));
        // Other Python, including one that merely mentions hermes later on.
        assert!(!is_hermes_command(&cmd(&[
            "python",
            "manage.py",
            "runserver"
        ])));
        assert!(!is_hermes_command(&cmd(&[
            "python",
            "-m",
            "http.server",
            "hermes"
        ])));
        assert!(AGENT_PROCESSES
            .iter()
            .any(|(slug, name, _, _)| *slug == "hermes" && *name == "hermes"));
    }
    use super::*;

    /// The reopen bound must stat the file the covered tools actually read.
    ///
    /// Raised in review on #329, where it statted `ca-bundle.pem` instead. The
    /// `max` tests below pass whichever file is chosen - they only exercise
    /// the comparison - so the choice of source needs its own pin or the next
    /// swap goes unnoticed the same way.
    ///
    /// Asserted against `ca_cert_path` rather than a literal filename,
    /// because that function is what `claude_code.rs` writes into
    /// `NODE_EXTRA_CA_CERTS` and what `proxy_env` exports. If the tools are
    /// ever pointed somewhere else, this should move with them rather than
    /// keep naming a path nobody reads.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn reopen_source_is_the_file_tools_read() {
        let source = reopen_cert_source().expect("a source path");
        let tools_read = gate_connect_core::proxy::ca_cert_path().expect("a cert path");
        let bundle = gate_connect_core::proxy::ca_bundle::path().expect("a bundle path");

        assert_ne!(
            tools_read, bundle,
            "the two must stay distinct, or this test proves nothing"
        );
        // The assertion that actually bites: swap `reopen_cert_source` back to
        // the bundle and this fails. Asserting properties of `ca_cert_path`
        // instead would pass either way, which is how the first attempt at
        // this test was useless.
        assert_eq!(
            source, tools_read,
            "the reopen bound must stat what the covered tools are pointed at \
             (NODE_EXTRA_CA_CERTS), not the bundle, whose only consumer is \
             Hermes and which regenerates on every connect"
        );
        assert_ne!(source, bundle);
    }

    /// Every command that ends the account announces the stored
    /// analytics identity, after the core call that changed it (AG-960). A scan
    /// of this file's own source, because the commands need a running app.
    #[test]
    fn account_changes_announce_the_analytics_identity_after_the_change() {
        let src = include_str!("lib.rs").replace("\r\n", "\n");
        for (start, change) in [
            ("async fn oauth_sign_out()", "oauth::clear()"),
            ("async fn clear_account()", "account::clear()"),
        ] {
            let at = src.find(start).expect(start);
            let body = &src[at..];
            let end = body[start.len()..]
                .find("\n#[tauri::command]")
                .map(|i| i + start.len())
                .unwrap_or(body.len());
            let body = &body[..end];
            let changed = body.find(change).expect(change);
            let announced = body
                .find("announce_stored_analytics_identity();")
                .unwrap_or_else(|| panic!("{start} must announce the identity"));
            assert!(changed < announced, "{start}: announce after {change}");
        }
    }

    /// Serialises the tests that mutate [`PENDING_BACKEND_ERRORS`].
    ///
    /// libtest runs a binary's tests on several threads by default, and the
    /// buffer is a process global that these tests reset and then count. One
    /// such test is deterministic on its own; a second one added later would
    /// flake in whichever direction the scheduler picked. Same reasoning as
    /// `gate_connect_core::env`'s `path_env_lock`, and `into_inner` on a
    /// poisoned lock for the same reason: a panic in one test should fail that
    /// test, not cascade.
    static BACKEND_ERROR_BUFFER_LOCK: Mutex<()> = Mutex::new(());

    /// One shell draining cannot starve the other.
    ///
    /// The buffer was a single `Vec` and the drain took it. That held while one
    /// shell drained; the tray gained a drain, both webviews are mounted from
    /// launch, and `report_backend_error` nudges both - so the two raced over
    /// one take and the loser got nothing. The user-visible halves: a resume
    /// failure raised in the popover taken by the hidden main window, leaving
    /// the tray silent (the dead button this was fixing), and a startup failure
    /// taken by the hidden tray, surfacing later as an unexplained banner.
    ///
    /// Calls [`drain_for_label`], which is what `drain_backend_errors` calls
    /// with `window.label()` - a `tauri::Window` is not buildable in a unit
    /// test, but the take itself is, and reimplementing it here would leave
    /// this green through a regression in the real one.
    ///
    /// Serialised on [`BACKEND_ERROR_BUFFER_LOCK`]: this mutates a process
    /// global and asserts exact counts, so it cannot share the buffer with a
    /// concurrently running test.
    #[test]
    fn each_shell_drains_its_own_copy_of_a_failure() {
        let _guard = BACKEND_ERROR_BUFFER_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let drain = drain_for_label;

        if let Ok(mut guard) = PENDING_BACKEND_ERRORS.lock() {
            *guard = None;
        }
        report_backend_error("provider_restore", "resume failed".into());

        let main_first = drain("main");
        assert_eq!(
            main_first.len(),
            1,
            "the window should see the failure it was told about"
        );

        let tray_after = drain("tray");
        assert_eq!(
            tray_after.len(),
            1,
            "the tray must still see it: with one shared buffer this was empty, \
             and the popover redrew an identical card saying nothing"
        );

        // And a drain is still a drain: neither shell re-reads its own.
        assert!(drain("main").is_empty());
        assert!(drain("tray").is_empty());
    }

    /// The collision this normalisation exists to avoid.
    ///
    /// `AGENT_PROCESSES` maps a process name to the tool it belongs to, and the
    /// only Claude entry is the CLI. Folding case made the desktop app match it,
    /// which put someone's Claude Desktop into the "close these to finish
    /// routing" list on macOS and Windows.
    /// Both are claimed now, and by different rows - which is the point. The
    /// invariant was never "the app claims nothing"; it is that the app is not
    /// the CLI. Getting this wrong once put someone's Claude Desktop into the
    /// set `close_running_agents` SIGTERMs while the dialog said Claude Code.
    #[test]
    fn the_desktop_app_is_not_the_cli() {
        assert_eq!(normalise_agent_name("claude"), "claude");
        assert_eq!(normalise_agent_name("Claude"), "Claude");

        let slug_for = |raw: &str| {
            AGENT_PROCESSES
                .iter()
                .find(|(_, name, _, _)| *name == normalise_agent_name(raw))
                .map(|(slug, _, _, _)| *slug)
        };
        assert_eq!(slug_for("claude"), Some("claude-code"));
        assert_eq!(slug_for("Claude"), Some("anthropic"));
        assert_ne!(slug_for("claude"), slug_for("Claude"));
    }

    /// The toggle's bound. An agent behind an old change stays stale with no
    /// bound, and drops out once the toggle asks only about its own changes -
    /// unless it made one, including a CA it minted in the same second it
    /// began, which is why `since` is taken before the trust step.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn reopen_counts_only_changes_since_the_bound() {
        let missed = |started: u64, config: Option<u64>, ca: Option<u64>, since: Option<u64>| {
            gate_connect_core::reopen::reopen_pending(&gate_connect_core::reopen::ReopenEvidence {
                process_names_known: true,
                process_starts: &[started],
                config_changed_at: at_or_after(config, since),
                ca_cert_changed_at: at_or_after(ca, since),
            })
        };
        let started = 100;
        // Missed a connect at 200, no bound: stale, as the boot probe reports.
        assert!(missed(started, Some(200), None, None));
        // Same agent, a toggle at 300 that rewrote nothing: not its news.
        assert!(!missed(started, Some(200), None, Some(300)));
        // That toggle minted a CA in its own first second: counted.
        assert!(missed(started, Some(200), Some(300), Some(300)));
        // Started after the latest change: never stale.
        assert!(!missed(400, Some(200), Some(300), None));
        // Nothing recorded: no claim.
        assert!(!missed(started, None, None, None));
    }

    /// The Chrome bridge shares the CLI's binary and process name, and only its
    /// arguments tell it apart from a Claude Code session.
    #[test]
    fn the_chrome_native_host_is_not_a_claude_code_session() {
        let cmd = |args: &[&str]| -> Vec<std::ffi::OsString> {
            args.iter().map(std::ffi::OsString::from).collect()
        };
        assert!(is_chrome_native_host(&cmd(&[
            "/home/u/.local/bin/claude",
            "--chrome-native-host"
        ])));
        assert!(!is_chrome_native_host(&cmd(&["claude"])));
        assert!(!is_chrome_native_host(&cmd(&["claude", "--resume"])));
        assert!(!is_chrome_native_host(&cmd(&[])));
    }

    /// The lookup returns *every* name a slug claims, not the first.
    ///
    /// No slug names two processes today, and now for a reason rather than by
    /// accident: the two candidates were Cowork and Work, and both are modes
    /// inside an app already listed rather than apps of their own. A slug could
    /// still grow a second name - a vendor shipping a genuinely separate binary
    /// on one platform would do it.
    ///
    /// The guard is kept because the shape that failed is a `find`, which drops
    /// extra rows in silence: the dropped process reads as not running, so it is
    /// never marked stale, never offered for close and never reopened, with
    /// nothing on screen saying so.
    #[test]
    fn the_lookup_returns_every_name_a_slug_claims() {
        for (slug, name, _, _) in AGENT_PROCESSES {
            assert!(
                agent_process_names(slug).contains(&name),
                "{slug} does not resolve back to {name}"
            );
        }
        assert_eq!(agent_process_names("anthropic"), vec!["Claude"]);
        // Found by its script, not its interpreter - see `is_hermes_command`.
        assert_eq!(agent_process_names("hermes"), vec!["hermes"]);
        assert!(agent_process_names("openclaw").is_empty());
    }

    /// Every row can be named, and only the registry rows can be verified.
    ///
    /// Two halves of the same fact about the two desktop-app rows: their slugs
    /// are proxy-domain keys, so `list_tools` cannot name them and
    /// `routing_verdicts` cannot answer for them. The table carries the name;
    /// `RunningAgent::verifiable` carries the second half, and the reopen flow
    /// needs it to stop waiting for a verdict that is never coming.
    #[test]
    fn desktop_app_rows_are_named_but_not_verifiable() {
        for (slug, _, product, _) in AGENT_PROCESSES {
            assert!(!product.is_empty(), "{slug} has no product name");
            assert_eq!(
                ToolId::from_slug(slug).is_some(),
                matches!(slug, "claude-code" | "codex" | "opencode" | "hermes"),
                "{slug} disagrees with the registry about whether it can be swept"
            );
        }
        assert!(ToolId::from_slug("anthropic").is_none());
        assert!(ToolId::from_slug("chatgpt").is_none());
    }

    /// The CLI rows must never become relaunchable. This is the assertion that
    /// stops someone "fixing" a CLI's `can_reopen` by widening `Surface`:
    /// spawning a terminal program's binary starts a different one, somewhere
    /// else, and drops the session the user agreed to close.
    #[test]
    fn only_apps_are_relaunchable() {
        for (slug, _, _, surface) in AGENT_PROCESSES {
            let expected = match slug {
                "claude-code" | "codex" | "opencode" | "hermes" => Surface::Cli,
                _ => Surface::App,
            };
            assert!(
                surface == expected,
                "{slug} has the wrong surface, which decides whether Gate relaunches it"
            );
        }
    }

    /// Electron's helpers were never at risk - they are a different word - but
    /// they are what a process table is actually full of, so pin it.
    #[test]
    fn electron_helpers_claim_nothing() {
        for helper in [
            "Claude Helper",
            "Claude Helper (Renderer)",
            "Claude Helper (GPU)",
        ] {
            assert!(!AGENT_PROCESSES
                .iter()
                .any(|(_, name, _, _)| *name == normalise_agent_name(helper)));
        }
    }

    /// The `.exe` strip is what the lowercasing was really for, so it has to
    /// survive - including on a capitalised Windows name, where stripping the
    /// suffix must not also fold the name it leaves behind.
    #[test]
    fn strips_a_windows_suffix_in_any_case() {
        assert_eq!(normalise_agent_name("claude.exe"), "claude");
        assert_eq!(normalise_agent_name("codex.EXE"), "codex");
        assert_eq!(normalise_agent_name("opencode.Exe"), "opencode");
        assert_eq!(normalise_agent_name("Claude.exe"), "Claude");
    }

    #[test]
    fn leaves_everything_else_alone() {
        assert_eq!(normalise_agent_name("codex"), "codex");
        assert_eq!(normalise_agent_name("exe"), "exe");
        assert_eq!(normalise_agent_name(""), "");
        // Not a suffix, so not stripped.
        assert_eq!(normalise_agent_name("claude.exec"), "claude.exec");
    }

    /// The name comes from the OS, so the guard is against a panic, not against
    /// a case anyone expects to see.
    #[test]
    fn survives_a_non_ascii_name() {
        assert_eq!(normalise_agent_name("клод"), "клод");
        assert_eq!(normalise_agent_name("日本語.exe"), "日本語");
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|n| n.to_string()).collect()
    }

    fn teardown(
        managed: usize,
        failed: &[&str],
    ) -> Result<gate_connect_core::provider::QuitTeardown, String> {
        Ok(gate_connect_core::provider::QuitTeardown {
            managed,
            failed: names(failed),
        })
    }

    /// The rule the notifications switch has an exception for: a tool the quit
    /// could not put back is said with the switch off, because nothing else is
    /// left running to say it.
    #[test]
    fn quit_says_a_left_behind_tool_with_notifications_off() {
        assert_eq!(
            quit_notice_body(&teardown(2, &["Hermes"]), || false).as_deref(),
            Some("Failed to remove Gate from the Hermes config. Edit it by hand.")
        );
    }

    #[test]
    fn quit_says_a_clean_teardown_only_with_notifications_on() {
        assert_eq!(
            quit_notice_body(&teardown(2, &[]), || true).as_deref(),
            Some("Gate removed from tool configs")
        );
        assert_eq!(quit_notice_body(&teardown(2, &[]), || false), None);
    }

    /// Nothing named Gate, so nothing was removed and there is nothing to say -
    /// not even to a user with notifications on.
    #[test]
    fn quit_says_nothing_when_no_tool_named_gate() {
        assert_eq!(
            quit_notice_body(&teardown(0, &[]), || panic!("preference read")),
            None
        );
    }

    /// The failure notices do not depend on the preference at all, not only on
    /// it reading false: a refactor that read the switch up front would panic
    /// here.
    #[test]
    fn quit_says_a_failure_without_reading_the_switch() {
        assert_eq!(
            quit_notice_body(&teardown(3, &["Codex", "Hermes"]), || {
                panic!("preference read")
            })
            .as_deref(),
            Some("Failed to remove Gate from the Codex and Hermes configs. Edit them by hand.")
        );
        assert_eq!(
            quit_notice_body(&Err("guard held".into()), || panic!("preference read")).as_deref(),
            Some("Failed to remove Gate from tool configs. Edit them by hand.")
        );
    }

    /// AG-947. The ChatGPT desktop app ships the Codex binary and runs it as a
    /// helper, named exactly `codex` - so the name alone reported the app as a
    /// CLI, told the person to reopen a terminal they never opened, and marked
    /// it unrelaunchable.
    #[test]
    fn a_codex_inside_the_chatgpt_app_is_the_app() {
        use std::path::Path;
        // The observed path, on macOS.
        assert!(is_chatgpt_bundled_codex(Some(Path::new(
            "/Applications/ChatGPT.app/Contents/Resources/codex"
        ))));
        // Installed per-user rather than to /Applications.
        assert!(is_chatgpt_bundled_codex(Some(Path::new(
            "/Users/someone/Applications/ChatGPT.app/Contents/Resources/codex"
        ))));
    }

    /// The half that matters more: a CLI the person installed must keep being
    /// read as a CLI, or this fix trades one wrong instruction for a worse one
    /// - `Surface::App` hands a process to the kill-and-relaunch machinery.
    #[test]
    fn a_codex_the_user_installed_is_still_the_cli() {
        use std::path::Path;
        for path in [
            "/opt/homebrew/bin/codex",
            "/usr/local/bin/codex",
            "/Users/someone/.local/bin/codex",
            "/Users/someone/.cargo/bin/codex",
            "/Users/someone/project/node_modules/.bin/codex",
            // Raised in review on #352, and it failed against the first
            // version of this: a project directory that happens to be called
            // `chatgpt`. Matching the word rather than the bundle would have
            // had Gate offer to "reopen ChatGPT" and hand this script to
            // LaunchServices, which opens it in a Terminal window.
            "/Users/someone/code/chatgpt/node_modules/.bin/codex",
            // The same trap one level up: a directory named for the bundle but
            // without the bundle's `Contents` beneath it.
            "/Users/someone/ChatGPT.app/codex",
        ] {
            assert!(
                !is_chatgpt_bundled_codex(Some(Path::new(path))),
                "{path} should read as the CLI"
            );
        }
        // Vendored inside some other app: deliberately NOT claimed, because
        // calling it ChatGPT would be a worse error than the one being fixed.
        assert!(!is_chatgpt_bundled_codex(Some(Path::new(
            "/Applications/SomeEditor.app/Contents/Resources/codex"
        ))));
    }

    /// No path to judge by - a process the OS will not tell us about. The CLI
    /// reading is the safe one: it reports something the person can act on and
    /// never offers to relaunch a process Gate has not identified.
    #[test]
    fn an_unreadable_path_falls_back_to_the_cli() {
        assert!(!is_chatgpt_bundled_codex(None));
        assert_eq!(claude_desktop_part(None), None);
    }

    /// A Windows path with `/` separators, which Windows accepts too, so these
    /// tests split into the same components on every CI host.
    fn win(path: &str) -> std::path::PathBuf {
        path.replace('\\', "/").into()
    }

    /// The Windows Store app reports itself as lowercase `claude.exe`, so the
    /// name matched the CLI row. The paths are the ones `sysinfo` returned on a
    /// real install; see [`claude_desktop_part`].
    #[test]
    fn a_claude_inside_the_desktop_app_is_the_app() {
        assert_eq!(
            claude_desktop_part(Some(&win(
                r"C:\Program Files\WindowsApps\Claude_2.9939.2.0_x64__pzs8sxrjxfjjc\app\claude.exe"
            ))),
            Some(ClaudeDesktopPart::App)
        );
        // The Code tab's binary, as `sysinfo` reports it (virtualised) and as
        // WMI does (not).
        for path in [
            r"C:\Users\someone\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\claude-code\2.1.281\claude.exe",
            r"C:\Users\someone\AppData\Roaming\Claude\claude-code\2.1.281\claude.exe",
        ] {
            assert_eq!(
                claude_desktop_part(Some(&win(path))),
                Some(ClaudeDesktopPart::CodeTab),
                "{path}"
            );
        }
    }

    /// A CLI the person installed keeps reading as the CLI: promoting it to the
    /// app would hand their terminal session to the kill machinery under the
    /// app's name.
    #[test]
    fn a_claude_the_user_installed_is_still_the_cli() {
        for path in [
            r"C:\Users\someone\.local\bin\claude.exe",
            // Under `Roaming` like the Code tab, but not `Claude\claude-code`.
            r"C:\Users\someone\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\claude.exe",
            r"C:\Users\someone\AppData\Local\Microsoft\WinGet\Links\claude.exe",
            // A folder that happens to be named for the layout, not under it.
            r"C:\Users\someone\code\claude-code\2.1.0\claude.exe",
            r"C:\Users\someone\Claude\claude-code\2.1.0\claude.exe",
            // Another publisher's package, or one merely named Claude.
            r"C:\Program Files\WindowsApps\Claude_1.0.0.0_x64__8wekyb3d8bbwe\claude.exe",
            // Something else inside the app's package.
            r"C:\Program Files\WindowsApps\Claude_2.9939.2.0_x64__pzs8sxrjxfjjc\app\other.exe",
            "/opt/homebrew/bin/claude",
        ] {
            assert_eq!(
                claude_desktop_part(Some(&win(path))),
                None,
                "{path} should read as the CLI"
            );
        }
    }

    /// Neither half of the desktop app is launched by path: the Code tab is a
    /// CLI surface, and the app is a Store package. Each is asked with the
    /// surface its own row gives it, which is what the walk does.
    #[test]
    fn the_desktop_app_parts_are_not_relaunched_by_path() {
        for path in [
            r"C:\Program Files\WindowsApps\Claude_2.9939.2.0_x64__pzs8sxrjxfjjc\app\claude.exe",
            r"C:\Users\someone\AppData\Roaming\Claude\claude-code\2.1.281\claude.exe",
        ] {
            let exe = win(path);
            let surface = agent_row_for("claude", Some(&exe))
                .map(|(_, _, _, surface)| *surface)
                .expect(path);
            assert_eq!(relaunch_target_for(Some(&exe), surface), None, "{path}");
        }
    }

    /// Which row each `claude` resolves to, which is what every scan keys on.
    ///
    /// The Code tab is the case that was wrong in review: it is Claude Code
    /// reading `~/.claude/settings.json`, so it must stay on the `claude-code`
    /// slug and name, or a Claude Code config change stops asking it to restart.
    #[test]
    fn each_claude_resolves_to_its_row() {
        let row = |name: &str, path: &str| {
            agent_row_for(name, Some(&win(path))).map(|(slug, n, _, surface)| (*slug, *n, *surface))
        };
        assert_eq!(
            row(
                "claude",
                r"C:\Program Files\WindowsApps\Claude_2.9939.2.0_x64__pzs8sxrjxfjjc\app\claude.exe"
            ),
            Some(("anthropic", "Claude", Surface::App))
        );
        assert_eq!(
            row(
                "claude",
                r"C:\Users\someone\AppData\Roaming\Claude\claude-code\2.1.281\claude.exe"
            ),
            Some(("claude-code", "claude", Surface::Cli))
        );
        assert_eq!(
            row("claude", r"C:\Users\someone\.local\bin\claude.exe"),
            Some(("claude-code", "claude", Surface::Cli))
        );
        // macOS spells the app with a capital, and no path is needed.
        assert_eq!(
            agent_row_for("Claude", None).map(|(slug, _, _, _)| *slug),
            Some("anthropic")
        );
    }

    /// No Store app is launched by path, whoever's it is.
    #[test]
    fn nothing_in_windows_apps_is_relaunched_by_path() {
        assert_eq!(
            relaunch_target_for(
                Some(&win(
                    r"C:\Program Files\WindowsApps\OpenAI.ChatGPT-Desktop_1.2025.0.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe"
                )),
                Surface::App
            ),
            None
        );
    }

    /// Electron's children carry `--type=`; the app's main process does not.
    #[test]
    fn electron_children_are_told_from_the_app() {
        let cmd = |args: &[&str]| -> Vec<std::ffi::OsString> {
            args.iter().map(|a| (*a).into()).collect()
        };
        assert!(is_electron_child(&cmd(&["claude.exe", "--type=renderer"])));
        assert!(is_electron_child(&cmd(&[
            "claude.exe",
            "--type=gpu-process",
            "--x"
        ])));
        assert!(!is_electron_child(&cmd(&["claude.exe"])));
        assert!(!is_electron_child(&cmd(&["claude.exe", "--resume"])));
    }

    /// The live Windows measurement [`claude_desktop_part`] cites, as a table:
    /// the app's main process, twelve Electron children and four Code-tab
    /// sessions, all `claude`, plus a terminal CLI beside them. The walk must
    /// yield one app and the Code tab plus the CLI, and each scope only its own.
    #[test]
    fn the_walk_yields_one_app_and_the_code_tab_sessions() {
        let app = win(
            r"C:\Program Files\WindowsApps\Claude_2.9939.2.0_x64__pzs8sxrjxfjjc\app\claude.exe",
        );
        let tab = win(
            r"C:\Users\someone\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\claude-code\2.1.281\claude.exe",
        );
        let cli = win(r"C:\Users\someone\.local\bin\claude.exe");
        let args =
            |a: &[&str]| -> Vec<std::ffi::OsString> { a.iter().map(|s| (*s).into()).collect() };

        let mut processes = vec![(&app, args(&["claude.exe"]))];
        for kind in ["renderer", "gpu-process", "utility"] {
            for _ in 0..4 {
                processes.push((&app, args(&["claude.exe", &format!("--type={kind}")])));
            }
        }
        for _ in 0..4 {
            processes.push((&tab, args(&["claude.exe"])));
        }
        processes.push((&cli, args(&["claude.exe"])));

        let yielded = |names: &[&str]| {
            processes
                .iter()
                .filter(|(exe, cmd)| walk_yields("claude", Some(exe.as_path()), cmd, names))
                .count()
        };
        assert_eq!(yielded(&agent_names_for(None)), 6);
        assert_eq!(yielded(&agent_names_for(Some(&["claude-code".into()]))), 5);
        assert_eq!(yielded(&agent_names_for(Some(&["anthropic".into()]))), 1);
    }

    /// The bundled helper is not a thing to launch.
    ///
    /// `close_running_agents` queues one target per killed PROCESS, so closing
    /// ChatGPT queued two `chatgpt` entries once the helper read as an app.
    /// The app's own process is killed and queued alongside it, and relaunching
    /// that is what brings the helper back - raised in review on #352.
    #[test]
    fn the_bundled_helper_is_not_its_own_relaunch_target() {
        use std::path::Path;
        assert_eq!(
            relaunch_target_for(
                Some(Path::new(
                    "/Applications/ChatGPT.app/Contents/Resources/codex"
                )),
                Surface::App
            ),
            None
        );
        // The app itself still is. **What it resolves TO is per platform**, so
        // the shared assertion is only that a target exists: the walk up to the
        // `.app` is `cfg(target_os = "macos")`, and asserting the bundle on
        // every platform failed CI on Linux and Windows, where the same call
        // returns the executable unchanged.
        let app = relaunch_target_for(
            Some(Path::new(
                "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT",
            )),
            Surface::App,
        );
        assert!(app.is_some());
        #[cfg(target_os = "macos")]
        assert_eq!(app, Some(PathBuf::from("/Applications/ChatGPT.app")));
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            app,
            Some(PathBuf::from(
                "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT"
            ))
        );
        // A CLI is never relaunched, whatever its path.
        assert_eq!(
            relaunch_target_for(Some(Path::new("/opt/homebrew/bin/codex")), Surface::Cli),
            None
        );
    }

    /// A Claude Code session Claude Desktop started from a helper, or through
    /// a shell, is still the app's to stop.
    #[test]
    fn an_agent_anywhere_up_the_chain_is_the_host() {
        use sysinfo::Pid;
        // 100 Claude main -> 101 helper -> 102 cmd.exe -> 103 claude (session);
        // 200 terminal -> 201 claude; 300 and 301 are each other's parent.
        let parent = |pid: Pid| -> Option<Pid> {
            let parent = match pid.as_u32() {
                101 => 100,
                102 => 101,
                103 => 102,
                100 | 200 => 1,
                201 => 200,
                300 => 301,
                301 => 300,
                _ => return None,
            };
            Some(Pid::from_u32(parent))
        };
        let agent = |pid: Pid| matches!(pid.as_u32(), 100 | 101 | 103 | 201);
        assert!(hosted_by_agent(Pid::from_u32(103), parent, agent));
        assert!(!hosted_by_agent(Pid::from_u32(100), parent, agent));
        assert!(!hosted_by_agent(Pid::from_u32(201), parent, agent));
        assert!(!hosted_by_agent(Pid::from_u32(300), parent, agent));
    }
    /// The path Claude Desktop runs from on the Windows machine the restart was
    /// built on, and the fields the app ID is assembled from.
    #[test]
    fn a_store_package_is_read_from_its_install_path() {
        let exe =
            r"C:\Program Files\WindowsApps\Claude_2.9939.4.0_x64__pzs8sxrjxfjjc\app\Claude.exe";
        assert_eq!(
            windows_store_package(exe),
            Some(StorePackage {
                install_dir: r"C:\Program Files\WindowsApps\Claude_2.9939.4.0_x64__pzs8sxrjxfjjc"
                    .into(),
                family: "Claude_pzs8sxrjxfjjc".into(),
                name: "Claude".into(),
            })
        );
        // Case does not decide it; Windows paths are not case-sensitive.
        assert!(windows_store_package(&exe.to_ascii_lowercase()).is_some());
        // Not a package folder: too few fields, or not under WindowsApps.
        assert_eq!(
            windows_store_package(r"C:\Program Files\WindowsApps\Claude\Claude.exe"),
            None
        );
        assert_eq!(
            windows_store_package(
                r"C:\Users\u\AppData\Roaming\Claude\claude-code\2.1.284\claude.exe"
            ),
            None
        );
    }

    #[test]
    fn the_app_id_is_the_application_that_was_running() {
        // Trimmed from Claude's own AppxManifest.xml: the helper is a second
        // application, and the list element shares the tag's prefix.
        let manifest = r#"<Package><Applications>
<Application Id="Claude" Executable="app\Claude.exe" EntryPoint="Windows.FullTrustApplication">
</Application>
<Application
  Id="SshAskpass" Executable="app\resources\claude-ssh-askpass.exe" EntryPoint="Windows.FullTrustApplication">
</Application>
</Applications></Package>"#;
        assert_eq!(
            manifest_app_id(manifest, r"app\Claude.exe").as_deref(),
            Some("Claude")
        );
        assert_eq!(
            manifest_app_id(manifest, r"APP\claude.exe").as_deref(),
            Some("Claude")
        );
        assert_eq!(
            manifest_app_id(manifest, r"app\resources\claude-ssh-askpass.exe").as_deref(),
            Some("SshAskpass")
        );
        assert_eq!(manifest_app_id(manifest, r"app\other.exe"), None);
        // `Id` is a whole attribute name, not the tail of another one.
        assert_eq!(xml_attr(r#" AppId="x" Id="y""#, "Id"), Some("y"));
    }
}
