//! Tauri shell. The Rust surface here is small on purpose: every command
//! delegates to `gate-connect-core` so the CLI and the GUI exercise the
//! same code path. Beyond commands, this file sets up the menu-bar / tray
//! presentation: no dock icon, hidden window on launch, click the tray
//! to toggle a popover-style window anchored under the icon.

use gate_connect_core::{ConnectInput, Status, ToolId, account, registry};

// `claude_session_delegate` backs Claude Code session delegation, available for Cowork
// on macOS and Windows. `migrate` (standard-mode -> 3P-mode userData
// migration) is macOS-only. Gated to keep builds free of unused flows.
#[cfg(any(target_os = "macos", target_os = "windows"))]
use gate_connect_core::claude_session_delegate;
#[cfg(target_os = "macos")]
use gate_connect_core::migrate;
use serde::Serialize;
use tauri::{
    Manager, PhysicalPosition, WindowEvent,
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
#[cfg(not(target_os = "linux"))]
use tauri::{Position, Size};


/// Shape-check a user-supplied key coming over the JS-to-Rust boundary.
/// Refuses empty input, control chars, lengths > 512 bytes, and a missing
/// prefix when one is required. The keychain layer treats keys as opaque
/// bytes — this check exists to fail fast before we persist nonsense.
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
        return Err(format!(
            "API key must start with {required_prefix:?}"
        ));
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
    // Cowork delegation/migration affordances. save_upstream_via_claude_oauth
    // compiles on macOS + Windows; the migrate_* trio is macOS-only. The UI
    // hides whichever affordance isn't backed on the current platform rather
    // than invoking a command that does not exist.
    supports_claude_oauth_delegation: bool,
    supports_migrate: bool,
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
            message: e.to_string(),
        },
    }
}

#[tauri::command]
fn list_tools() -> Vec<ToolDto> {
    registry::registry()
        .iter()
        .map(|integ| ToolDto {
            slug: integ.id().to_string(),
            name: integ.display_name().to_string(),
            upstream_provider_name: integ.upstream_provider_name().to_string(),
            default_upstream_url: integ.default_upstream_url().to_string(),
            requires_upstream_credential: integ.requires_upstream_credential(),
            supports_claude_oauth_delegation: cfg!(any(target_os = "macos", target_os = "windows"))
                && integ.id().to_string() == "cowork",
            supports_migrate: cfg!(target_os = "macos")
                && integ.id().to_string() == "cowork",
            status: status_for(integ.as_ref()),
        })
        .collect()
}

#[tauri::command]
fn tool_status(slug: String) -> Result<StatusDto, String> {
    let id = ToolId::from_slug(&slug).ok_or_else(|| format!("unknown tool {slug:?}"))?;
    let integ = registry::find(id).ok_or_else(|| "integration missing from registry".to_string())?;
    Ok(status_for(integ.as_ref()))
}

#[tauri::command]
fn connect_tool(slug: String, upstream_url: String) -> Result<StatusDto, String> {
    let integ = resolve_integration(&slug)?;
    let account = account::load()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Sign in to Gate AI first".to_string())?;
    if integ.requires_upstream_credential()
        && !integ.has_upstream_credential().map_err(|e| e.to_string())?
    {
        return Err("No upstream Anthropic credential saved. Add one before connecting.".into());
    }
    let input = ConnectInput {
        gateway_base_url: account.gateway_base_url,
        upstream_url,
    };
    integ.connect(&input).map_err(|e| e.to_string())?;
    Ok(status_for(integ.as_ref()))
}

#[tauri::command]
fn disconnect_tool(slug: String) -> Result<StatusDto, String> {
    let integ = resolve_integration(&slug)?;
    integ.disconnect().map_err(|e| e.to_string())?;
    Ok(status_for(integ.as_ref()))
}

#[tauri::command]
fn has_upstream_credential(slug: String) -> Result<bool, String> {
    let integ = resolve_integration(&slug)?;
    integ.has_upstream_credential().map_err(|e| e.to_string())
}

#[tauri::command]
fn save_upstream_api_key(slug: String, api_key: String) -> Result<(), String> {
    let integ = resolve_integration(&slug)?;
    // Enforce the integration's expected credential prefix (e.g. "sk-ant-"
    // for Cowork/Anthropic) in addition to the empty/control-char/oversize
    // checks, so a compromised renderer can't write arbitrary bytes to a
    // tool's keychain entry under a mismatched slug.
    validate_api_key(api_key.trim(), integ.upstream_credential_prefix())?;
    integ
        .save_upstream_credential(&api_key)
        .map_err(|e| e.to_string())
}

/// Delegate the upstream credential to the live Claude Code session.
/// We verify that session exists right now (which triggers macOS's
/// cross-app keychain authorization prompt the first time), then save
/// a sentinel marker in our per-tool entry. The credential helper
/// re-reads Claude Code's token on every Cowork request, so refresh
/// is automatic.
///
/// Available where the Cowork integration is: macOS and Windows.
#[cfg(any(target_os = "macos", target_os = "windows"))]
#[tauri::command]
async fn save_upstream_via_claude_oauth(slug: String) -> Result<(), String> {
    let integ = resolve_integration(&slug)?;
    let sentinel =
        tauri::async_runtime::spawn_blocking(claude_session_delegate::verify_claude_code_session)
            .await
            .map_err(|e| format!("verify join error: {e}"))?
            .map_err(|e| e.to_string())?;
    integ
        .save_upstream_credential(sentinel)
        .map_err(|e| e.to_string())
}

/// Probe whether Claude Code currently has a usable signed-in session
/// (a parseable `sk-ant-oat` access token in its session store). Lets the
/// UI pre-select and label the Claude-subscription credential source.
#[cfg(any(target_os = "macos", target_os = "windows"))]
#[tauri::command]
fn detect_claude_code_session() -> bool {
    claude_session_delegate::verify_claude_code_session().is_ok()
}

#[tauri::command]
fn clear_upstream_credential(slug: String) -> Result<(), String> {
    let integ = resolve_integration(&slug)?;
    integ.clear_upstream_credential().map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
#[tauri::command]
async fn migrate_discover(slug: String) -> Result<migrate::MigrateDiscover, String> {
    ensure_cowork(&slug)?;
    tauri::async_runtime::spawn_blocking(migrate::discover)
        .await
        .map_err(|e| format!("discover join error: {e}"))?
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
#[tauri::command]
async fn migrate_preview(
    slug: String,
    options: migrate::MigrateOptions,
) -> Result<migrate::MigrateReport, String> {
    ensure_cowork(&slug)?;
    let mut opts = options;
    opts.dry_run = true;
    tauri::async_runtime::spawn_blocking(move || migrate::execute(&opts))
        .await
        .map_err(|e| format!("preview join error: {e}"))?
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
#[tauri::command]
async fn migrate_execute(
    slug: String,
    options: migrate::MigrateOptions,
) -> Result<migrate::MigrateReport, String> {
    ensure_cowork(&slug)?;
    let opts = options;
    tauri::async_runtime::spawn_blocking(move || migrate::execute(&opts))
        .await
        .map_err(|e| format!("execute join error: {e}"))?
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn ensure_cowork(slug: &str) -> Result<(), String> {
    if slug == "cowork" {
        Ok(())
    } else {
        Err("migrate is only supported for `cowork` today".into())
    }
}

fn resolve_integration(slug: &str) -> Result<Box<dyn gate_connect_core::Integration>, String> {
    let id = ToolId::from_slug(slug).ok_or_else(|| format!("unknown tool {slug:?}"))?;
    registry::find(id).ok_or_else(|| "integration missing from registry".to_string())
}

#[derive(Serialize)]
struct AccountDto {
    gateway_base_url: String,
    has_api_key: bool,
}

#[tauri::command]
fn get_account() -> Result<Option<AccountDto>, String> {
    let Some(gateway_base_url) = account::load_base_url().map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let has_api_key = account::has_api_key().map_err(|e| e.to_string())?;
    Ok(Some(AccountDto {
        gateway_base_url,
        has_api_key,
    }))
}

#[tauri::command]
fn save_account(base_url: String, api_key: Option<String>) -> Result<(), String> {
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
    let key = api_key.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if let Some(k) = key {
        validate_api_key(k, "sk-")?;
    }
    account::save(&base_url, key).map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_account() -> Result<(), String> {
    account::clear().map_err(|e| e.to_string())
}

/// OS identifier ("macos" / "windows" / "linux") so the UI can tailor
/// copy: keychain vs Credential Manager, plist vs registry, whether a
/// password prompt appears, etc.
#[tauri::command]
fn app_platform() -> &'static str {
  std::env::consts::OS
}

// ---- Built-in MITM proxy (macOS + Windows + Linux) ----
//
// These delegate to the process-global `proxy::manager()`. They're gated to
// the platforms where CA trust + system-proxy wiring is implemented (macOS via
// `security`/`networksetup`, Windows via `certutil`/WinINET, Linux via the
// system trust store + `/etc/environment`) and registered in each platform's
// handler block below. The tray status dot is a macOS feature, so only the
// macOS build refreshes it from enable/disable.

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn proxy_status() -> Result<gate_connect_core::proxy::ProxyState, String> {
  gate_connect_core::proxy::manager()
    .status()
    .map_err(|e| e.to_string())
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn proxy_list_domains() -> Result<Vec<gate_connect_core::proxy::ProxyDomain>, String> {
  gate_connect_core::proxy::manager()
    .list_domains()
    .map_err(|e| e.to_string())
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn proxy_enable(app: tauri::AppHandle) -> Result<gate_connect_core::proxy::ProxyState, String> {
  let state = gate_connect_core::proxy::manager()
    .enable()
    .map_err(|e| e.to_string())?;
  #[cfg(target_os = "macos")]
  update_tray_status(&app, state.running);
  #[cfg(not(target_os = "macos"))]
  let _ = &app;
  Ok(state)
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn proxy_disable(app: tauri::AppHandle) -> Result<gate_connect_core::proxy::ProxyState, String> {
  let state = gate_connect_core::proxy::manager()
    .disable()
    .map_err(|e| e.to_string())?;
  #[cfg(target_os = "macos")]
  update_tray_status(&app, state.running);
  #[cfg(not(target_os = "macos"))]
  let _ = &app;
  Ok(state)
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn proxy_set_domain(
  slug: String,
  enabled: bool,
) -> Result<gate_connect_core::proxy::ProxyState, String> {
  gate_connect_core::proxy::manager()
    .set_domain(&slug, enabled)
    .map_err(|e| e.to_string())
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn proxy_trust_ca() -> Result<gate_connect_core::proxy::ProxyState, String> {
  gate_connect_core::proxy::manager()
    .trust_ca()
    .map_err(|e| e.to_string())
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tauri::command]
fn proxy_untrust_ca() -> Result<gate_connect_core::proxy::ProxyState, String> {
  gate_connect_core::proxy::manager()
    .untrust_ca()
    .map_err(|e| e.to_string())
}

/// Bytes of `src-tauri/icons/tray.png` compiled in so the menu bar gets
/// the right asset on first paint without a filesystem lookup at runtime.
const TRAY_ICON_PNG: &[u8] = include_bytes!("../icons/tray.png");

/// When the startup popover was shown. Used to ignore the spurious focus-loss
/// an Accessory (menu-bar) app can emit before it becomes frontmost, which
/// would otherwise immediately hide the popover we open on launch.
#[cfg(target_os = "macos")]
static LAUNCH_SHOWN_AT: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler({
            // Cowork commands fork by platform: save_upstream_via_claude_oauth
            // and detect_claude_code_session exist on macOS + Windows; the
            // migrate-* trio is macOS-only. Forking the whole generate_handler!
            // invocation (rather than per-item cfg) preserves Tauri's
            // compile-time arg/return type-checking for each command.
            #[cfg(target_os = "macos")]
            {
                tauri::generate_handler![
                    list_tools,
                    tool_status,
                    connect_tool,
                    disconnect_tool,
                    has_upstream_credential,
                    save_upstream_api_key,
                    save_upstream_via_claude_oauth,
                    detect_claude_code_session,
                    clear_upstream_credential,
                    get_account,
                    save_account,
                    clear_account,
                    app_platform,
                    migrate_discover,
                    migrate_preview,
                    migrate_execute,
                    proxy_status,
                    proxy_list_domains,
                    proxy_enable,
                    proxy_disable,
                    proxy_set_domain,
                    proxy_trust_ca,
                    proxy_untrust_ca,
                ]
            }
            #[cfg(target_os = "windows")]
            {
                tauri::generate_handler![
                    list_tools,
                    tool_status,
                    connect_tool,
                    disconnect_tool,
                    has_upstream_credential,
                    save_upstream_api_key,
                    save_upstream_via_claude_oauth,
                    detect_claude_code_session,
                    clear_upstream_credential,
                    get_account,
                    save_account,
                    clear_account,
                    app_platform,
                    proxy_status,
                    proxy_list_domains,
                    proxy_enable,
                    proxy_disable,
                    proxy_set_domain,
                    proxy_trust_ca,
                    proxy_untrust_ca,
                ]
            }
            #[cfg(target_os = "linux")]
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
                    save_account,
                    clear_account,
                    app_platform,
                    proxy_status,
                    proxy_list_domains,
                    proxy_enable,
                    proxy_disable,
                    proxy_set_domain,
                    proxy_trust_ca,
                    proxy_untrust_ca,
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
                    save_account,
                    clear_account,
                    app_platform,
                ]
            }
        })
        .on_window_event(|window, event| {
            // X-button on the popover should hide it, not quit the app.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
            // Click outside the popover → dismiss.
            if let WindowEvent::Focused(false) = event {
                // Ignore the spurious blur an Accessory app can emit right
                // after the startup show, before it's frontmost — otherwise the
                // popover we opened on launch would hide before it's even seen.
                #[cfg(target_os = "macos")]
                if LAUNCH_SHOWN_AT
                    .get()
                    .is_some_and(|t| t.elapsed().as_millis() < 1500)
                {
                    return;
                }
                let _ = window.hide();
            }
        })
        .setup(|app| {
            // Hide the dock icon — Gate Connect lives in the menu bar.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // If a previous session left the system proxy on (unclean quit /
            // crash), revert it now so HTTPS isn't routed at a dead loopback
            // port. Off-thread so a rare admin prompt doesn't block the tray;
            // a clean disable leaves nothing to reconcile.
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            std::thread::spawn(|| {
              if let Err(e) = gate_connect_core::proxy::manager().reconcile_on_startup() {
                eprintln!("proxy startup reconcile failed: {e}");
              }
            });

            // Round the NSWindow content view's CALayer so the transparent
            // window itself has rounded corners — without this, CSS-rounded
            // corners on the popover expose the dark window behind them.
            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                apply_window_corner_radius(&window, 12.0);
            }

            let tray_icon = Image::from_bytes(TRAY_ICON_PNG)?;

            let show_item = MenuItemBuilder::with_id("show", "Open Gate Connect").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "Quit Gate Connect").build(app)?;
            let menu = MenuBuilder::new(app)
                .items(&[&show_item, &quit_item])
                .build()?;

            TrayIconBuilder::with_id("main")
                .icon(tray_icon)
                .icon_as_template(true) // macOS auto-tints for dark/light menu bar
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
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
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
                            let is_visible = window.is_visible().unwrap_or(false);
                            if is_visible {
                                let _ = window.hide();
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
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;

            // First impression: a hidden menu-bar app looks broken on launch,
            // so surface the popover once at startup — top-right under the
            // menu-bar tray on the main display. Subsequent opens go through the
            // tray click. (Accessory app with no autostart, so a launch is
            // always an explicit user action — safe to show.)
            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                position_startup(&window);
                let _ = window.show();
                let _ = window.set_focus();
                let _ = LAUNCH_SHOWN_AT.set(std::time::Instant::now());
            }

            // Reflect the current proxy state in the tray dot at launch.
            #[cfg(target_os = "macos")]
            update_tray_status(
                app.handle(),
                gate_connect_core::proxy::manager()
                    .status()
                    .map(|s| s.running)
                    .unwrap_or(false),
            );

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Gate Connect");
}

/// Position the popover window centered horizontally on the tray icon
/// and just above or below it, whichever side has room on the current
/// monitor. macOS's menu bar lives at the top so the popover anchors
/// below the icon; Windows' taskbar is typically at the bottom and
/// anchoring below would push the popover off-screen — so we flip it
/// above the icon. X is clamped to the monitor bounds so a tray icon
/// near a screen edge doesn't push the popover past it.
/// Pick the monitor whose physical bounds contain point (x, y) — used to
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

/// On launch, place the popover at the top-right of the main display — just
/// under the macOS menu bar, where the tray icon lives — so first-time users
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

/// Build the menu-bar tray image with a status dot — green when the proxy is
/// routing, gray when it's off. The base hex mark is a template silhouette; to
/// show a *colored* dot we must render non-template, so we recolor the mark to
/// a high-contrast tone for the current menu-bar appearance (light vs dark).
#[cfg(target_os = "macos")]
fn tray_image(proxy_on: bool, dark_menubar: bool) -> Option<Image<'static>> {
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

    // Composite the status dot, bottom-right.
    let (dr, dg, db): (u8, u8, u8) = if proxy_on {
        (0x2E, 0xCC, 0x71) // green — routing
    } else {
        (0x8A, 0x8F, 0x9A) // gray — off
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

    Some(Image::new_owned(rgba, w, h))
}

/// Update the tray icon so its status dot reflects the current proxy state.
#[cfg(target_os = "macos")]
fn update_tray_status(app: &tauri::AppHandle, proxy_on: bool) {
    use tauri::Manager;
    let dark = app
        .get_webview_window("main")
        .and_then(|win| win.theme().ok())
        .map(|t| t == tauri::Theme::Dark)
        .unwrap_or(true);
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_icon_as_template(false);
        if let Some(img) = tray_image(proxy_on, dark) {
            let _ = tray.set_icon(Some(img));
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn anchor_under_tray(
    window: &tauri::WebviewWindow,
    tray_pos: Position,
    tray_size: Size,
) {
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
        .unwrap_or(520.0 * scale);

    let tray_center_x = pos.x + size.width / 2.0;
    let tray_top_y = pos.y;
    let tray_bottom_y = pos.y + size.height;
    let gap = 6.0 * scale;

    // Fall back to the unbounded below-icon placement if we can't read
    // the monitor — better than refusing to show the window.
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
/// rect — and on GNOME, where the left-click event often never fires
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
        .unwrap_or(520.0 * scale);

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

/// Round the corners of the underlying NSWindow on macOS.
///
/// The window is created with `transparent: false`, so it needs no macOS
/// private API. On its own that yields a square, opaque window. To get a
/// rounded shape we use only public AppKit/QuartzCore:
/// 1. make the NSWindow non-opaque with a clear background, so the corner
///    regions outside the radius render transparent rather than painting
///    the default window background;
/// 2. layer-back the content view and mask its CALayer to a corner radius.
/// The webview body stays opaque (white from CSS), so the corners read as
/// cleanly rounded against the desktop.
#[cfg(target_os = "macos")]
fn apply_window_corner_radius(window: &tauri::WebviewWindow, radius: f64) {
    use objc2::{class, msg_send};
    use objc2::runtime::AnyObject;

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
    }
}
