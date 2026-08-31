//! Built-in MITM proxy that routes config-less apps (Claude Desktop /
//! Cowork, ChatGPT desktop, …) through the Gate gateway without editing
//! any app config.
//!
//! How it works: enabling the proxy (1) trusts a locally-generated root CA
//! and (2) points the macOS system HTTPS proxy at a loopback listener owned
//! by [`engine`]. For each TLS CONNECT the engine decides - *before* doing
//! any handshake - whether the target host is one we route. Hosts we don't
//! route are blind-tunnelled untouched (so cert-pinning apps and every other
//! site are unaffected). For hosts we do route, the engine MITMs the
//! connection, and for inference paths rewrites the request to the Gate
//! gateway with `X-Gate-Api-Key` + `X-Gate-Upstream-Url` injected. Non-
//! inference paths on the same host (e.g. an app's auto-updater) pass
//! through to the real upstream.
//!
//! The CA private key lives in the OS keychain; only the public cert is
//! written to disk (and installed into the OS trust store when trusted).
//! Disabling the proxy restores the previous system-proxy state but
//! deliberately leaves the CA trusted, so re-enabling is promptless;
//! untrusting is a separate explicit action ([`ProxyManager::untrust_ca`]).
//!
//! Platform support: macOS, Windows, and Linux. The engine itself is
//! cross-platform; CA trust ([`ca`]) and system-proxy wiring ([`system_proxy`])
//! are platform-specific - macOS via `security` + `networksetup`, Windows via
//! `certutil` + the per-user WinINET registry settings, Linux via the system
//! trust store (`update-ca-certificates` / `update-ca-trust`) + a user-scoped
//! systemd `environment.d` drop-in. Other platforms get no [`ProxyManager`].
//!
//! Each platform wires *two* channels, because they reach different clients.
//! The OS proxy setting (a PAC on macOS/Windows) covers GUI apps and anything
//! on the platform HTTP stack; the proxy environment variables
//! ([`proxy_env`]) cover the command-line AI tools, whose Node/Bun/Python HTTP
//! clients read `HTTPS_PROXY` and ignore the OS setting entirely. On Linux the
//! drop-in has always been both at once; macOS exports the variables via
//! `launchctl setenv` and Windows via `HKCU\Environment` alongside the PAC.

use crate::account::BillingMode;
use anyhow::{Context, Result};
use http::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};

pub mod engine;

/// Platform trust roots plus Gate's CA, for tools that take a single CA file.
pub mod ca_bundle;

mod cert_authority;

/// Plaintext loopback reverse proxy for CLI tools; hosted in the engine.
mod relay;

pub mod config;

pub mod intent;

pub mod autostart_optout;

// Shared load/save for the persisted engine + PAC ports; the per-OS
// `system_proxy` modules wrap it with their platform rationale.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod port_persist;

// Shared names/values for the proxy environment variables, which every
// platform's `system_proxy` exports so CLI tools (Node/Bun/Python) route too.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod proxy_env;

#[cfg(target_os = "macos")]
pub mod ca;
#[cfg(target_os = "windows")]
#[path = "ca_windows.rs"]
pub mod ca;
#[cfg(target_os = "linux")]
#[path = "ca_linux.rs"]
pub mod ca;

#[cfg(target_os = "macos")]
pub mod system_proxy;
#[cfg(target_os = "windows")]
#[path = "system_proxy_windows.rs"]
pub mod system_proxy;
#[cfg(target_os = "linux")]
#[path = "system_proxy_linux.rs"]
pub mod system_proxy;

// The desktop (macOS/Windows) manager sequencing, generic over the
// `DesktopOps` platform seam so one implementation serves both OSes and the
// tests inside it can drive the real engine against a fake platform. Also
// compiled under `test` on other platforms so those tests run everywhere.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
mod manager_core;

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[path = "manager_desktop.rs"]
mod manager;
#[cfg(target_os = "linux")]
#[path = "manager_linux.rs"]
mod manager;

// The long-lived pass-through helper daemon and its control protocol/client
// (Linux only). The daemon owns the loopback listener so the proxy outlives the
// GUI process; see the module docs for the access-control model.
#[cfg(target_os = "linux")]
pub mod control;
#[cfg(target_os = "linux")]
mod flock;
#[cfg(target_os = "linux")]
pub mod helper;
#[cfg(target_os = "linux")]
pub mod helper_client;

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub use manager::{manager, ProxyManager};

/// Observer the desktop shell registers to hear about an unexpected engine
/// exit *after* the fail-safe has reverted the system proxy. The revert
/// itself lives in the manager and needs no observer - but the manager has no
/// window handle, so without this hook the tray kept its green "routing on"
/// dot and an open popover kept rendering On indefinitely while traffic
/// already flowed direct: the one pixel the product calls most important,
/// showing a state that stopped being true. macOS/Windows only in practice
/// (the in-process engine); the Linux engine lives in the helper daemon,
/// whose death the GUI notices through its normal status reads. Compiled
/// under `test` elsewhere too, because the generic manager sequencing
/// (`manager_core`) calls the notify below and its tests run on every OS.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
static ENGINE_CRASH_OBSERVER: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> =
    std::sync::OnceLock::new();

/// Register the crash observer. First registration wins; later calls are
/// ignored (the shell registers exactly once at setup).
#[cfg(any(target_os = "macos", target_os = "windows", test))]
pub fn set_engine_crash_observer(observer: impl Fn() + Send + Sync + 'static) {
    let _ = ENGINE_CRASH_OBSERVER.set(Box::new(observer));
}

/// Invoke the registered crash observer, if any. Called by the manager's
/// crash fail-safe once the revert is done and the engine lock is released.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
pub(crate) fn notify_engine_crash_observer() {
    if let Some(observer) = ENGINE_CRASH_OBSERVER.get() {
        observer();
    }
}

/// Observer the desktop shell registers to hear that a rewritten chatgpt.com
/// app turn was answered with a Cloudflare managed challenge
/// (`cf-mitigated: challenge`). The app shell has no HTML/JS surface to run
/// the interstitial, so the GUI opens a one-time solve webview, captures the
/// resulting `cf_clearance` cookie, and feeds it back through
/// [`engine::RunningEngine::update_cf_clearance`]. Set once at startup. Not
/// cfg-gated like the crash observer above: the notify below is called from
/// `engine::handle_response`, which compiles on every desktop OS (the Linux
/// helper daemon hosts the same engine); a daemon-hosted engine simply has no
/// observer registered, so the notify is a no-op there.
static CF_CHALLENGE_OBSERVER: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> =
    std::sync::OnceLock::new();

/// Debounce latch: set by the notify that fired the observer, cleared by
/// [`cf_challenge_solve_finished`]. While a solve webview is open (or a
/// captured cookie is being fed back) every further challenged turn would
/// otherwise re-fire the observer and stack webviews.
static CF_CHALLENGE_SOLVING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// How long to wait before the solve webview may reopen after an attempt that
/// captured nothing. Without it, a challenge our webview cannot clear reopens
/// the window on every challenged request forever - observed as the window
/// coming back repeatedly while the poll thread logged `cf_clearance present
/// but unchanged`. Long enough to stop being a nag, short enough that a user
/// who fixed the underlying problem is not locked out for the session.
const CF_CHALLENGE_RETRY_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(600);

/// How long the observer stays quiet after an attempt that DID capture a
/// cookie. Responses to turns sent before the cookie was injected can still
/// be in flight carrying `cf-mitigated: challenge`; without a grace the first
/// of them re-fires the observer the moment the latch clears, reopening the
/// window on a cookie that will never change until the poll's deadline gives
/// up and starts the long cooldown above. Long enough for stale responses to
/// drain, short enough that a genuinely re-challenged user is not left
/// waiting.
const CF_CHALLENGE_SUCCESS_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

/// Earliest instant at which the observer may fire again; `None` means "no
/// cooldown running". Set by every finished attempt: the long cooldown after
/// one that captured nothing, the short grace after one that captured.
static CF_CHALLENGE_NEXT_ALLOWED: std::sync::Mutex<Option<std::time::Instant>> =
    std::sync::Mutex::new(None);

/// Register the challenge observer. First registration wins; later calls are
/// ignored (the shell registers exactly once at setup).
pub fn set_cf_challenge_observer(observer: impl Fn() + Send + Sync + 'static) {
    let _ = CF_CHALLENGE_OBSERVER.set(Box::new(observer));
}

/// Invoke the registered challenge observer, if any, unless a solve is
/// already in flight or a finished attempt's cooldown is still running. Called
/// by the engine's `handle_response` when a chatgpt.com app turn comes back
/// `cf-mitigated: challenge`.
pub(crate) fn notify_cf_challenge_observer() {
    let Some(observer) = CF_CHALLENGE_OBSERVER.get() else {
        return;
    };
    let cooling = CF_CHALLENGE_NEXT_ALLOWED
        .lock()
        .ok()
        .and_then(|next| *next)
        .is_some_and(|next| std::time::Instant::now() < next);
    if cooling {
        // Say so. A suppressed challenge is otherwise indistinguishable from
        // a challenge that was never detected - both are silence followed by
        // the app showing Cloudflare's page - and telling those apart is the
        // first question anyone debugging this asks.
        if engine::debug_log() {
            eprintln!(
                "[gate-proxy] challenge detected but the previous solve attempt's \
                 cooldown is still running"
            );
        }
        return;
    }
    if CF_CHALLENGE_SOLVING.swap(true, std::sync::atomic::Ordering::AcqRel) {
        if engine::debug_log() {
            eprintln!("[gate-proxy] challenge detected while a solve is already in flight");
        }
        return;
    }
    observer();
}

/// Release the solve latch and start the cooldown for the next attempt.
/// `captured` reports whether the attempt actually produced a cookie: `false`
/// starts the long cooldown, so a challenge the webview cannot clear stops
/// reopening the window on every subsequent request; `true` starts the short
/// grace, so challenged responses already in flight when the cookie landed
/// cannot immediately reopen the window it just closed. The GUI calls this
/// exactly once per attempt.
pub fn cf_challenge_solve_finished(captured: bool) {
    let cooldown = if captured {
        CF_CHALLENGE_SUCCESS_GRACE
    } else {
        CF_CHALLENGE_RETRY_COOLDOWN
    };
    if let Ok(mut next) = CF_CHALLENGE_NEXT_ALLOWED.lock() {
        *next = Some(std::time::Instant::now() + cooldown);
    }
    CF_CHALLENGE_SOLVING.store(false, std::sync::atomic::Ordering::Release);
}

/// The `user-agent` last seen on an intercepted chatgpt.com app request.
///
/// The solve webview adopts it, for two reasons that both bite. Cloudflare
/// waves a stock WebView2 through - the observed solve window loads the
/// ordinary chat prompt, never an interstitial - and `cf_clearance` exists
/// ONLY as the result of a challenge, so a webview that is never challenged
/// mints nothing to capture. And the cookie is bound to the user-agent it was
/// issued to, so one minted under the webview's own UA would be refused when
/// replayed under the app's. Wearing the app's UA is what makes the challenge
/// fire in a surface that can solve it, and what makes the result usable.
///
/// Read from the wire rather than hardcoded: the app's UA carries its build
/// (`ChatGPTBrowser/<version> …`) and a pinned guess would drift out of date
/// silently, which is the failure this whole flow is least able to diagnose.
static CHATGPT_APP_USER_AGENT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Remember the app's user-agent for the solve webview. Called by the engine
/// on intercepted chatgpt.com app requests.
///
/// Only browser-shaped shell UAs are kept. The app emits several: the shell's
/// `CodexBrowser Mozilla/5.0 …` and the native agent's `Codex Desktop/…`,
/// which is not a browser string at all. Recording the last one seen let the
/// agent's UA win a race and opened a solve webview claiming to be a CLI -
/// observed 2026-08-28, and precisely the client shape Cloudflare is least
/// likely to hand a `cf_clearance` to. The shell is the surface being
/// impersonated, so it is the only one worth recording.
pub(crate) fn record_chatgpt_app_user_agent(user_agent: &str) {
    if browser_ua_without_product_token(user_agent).is_none() {
        return;
    }
    if let Ok(mut held) = CHATGPT_APP_USER_AGENT.lock() {
        if held.as_deref() != Some(user_agent) {
            *held = Some(user_agent.to_owned());
        }
    }
}

/// The app's user-agent, if the engine has seen an app request this run.
/// `None` before the first one, in which case the solve webview keeps its
/// platform default.
pub fn chatgpt_app_user_agent() -> Option<String> {
    CHATGPT_APP_USER_AGENT.lock().ok().and_then(|v| v.clone())
}

/// Whether an HTTP authority (`host` or `host:port`, IPv6 in brackets) names
/// this machine's loopback - the only place our plain-HTTP loopback listeners
/// (the relay, the PAC responder) may be addressed from.
///
/// This is the standard local-daemon DNS-rebinding defense, shared by the
/// relay and the PAC server so they can't drift: a browser always names its
/// target in the `Host` header, so a page that rebound `attacker.example` to
/// 127.0.0.1 still arrives carrying `Host: attacker.example` and is refused,
/// while the CLI tools these listeners exist for dial `127.0.0.1` directly.
/// The port is deliberately not pinned - every listener that calls this binds
/// loopback exclusively, so any request that reached it already used our
/// port, and pinning would only add a way to break legitimate callers.
pub(crate) fn authority_is_loopback(authority: &str) -> bool {
    let authority = authority.trim();
    // Bracketed IPv6 (`[::1]:8080` / `[::1]`) carries colons inside the
    // brackets, so strip that form before splitting off a port.
    let host = if let Some(rest) = authority.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        authority.rsplit_once(':').map_or(authority, |(h, _)| h)
    };
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}

/// Whether an `Origin` header value may talk to our loopback listeners: only
/// a loopback origin qualifies. Anything else - a remote site's origin, or
/// the opaque `null` a sandboxed/rebound context sends - marks a cross-site
/// browser request, which must never spend the owner's Gate credential even
/// though CORS already keeps the page from reading the response ("simple"
/// cross-origin POSTs are delivered without a preflight). Non-browser
/// clients send no `Origin` at all, so they never reach this check.
pub(crate) fn origin_is_loopback(origin: &str) -> bool {
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    authority_is_loopback(rest.split('/').next().unwrap_or(""))
}

/// Cross-process hint that some Gate Connect process has the system proxy
/// routed through a live engine: the snapshot file exists for exactly that
/// duration. The engine holds the key it was started with, so the CLI uses
/// this to warn after a rotation/sign-out it cannot propagate in-process.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn engine_likely_running() -> bool {
    system_proxy::load_snapshot()
        .map(|s| s.is_some())
        .unwrap_or(false)
}

/// The engine port a *different* process is hosting right now, if there is one.
///
/// macOS and Windows keep the engine inside whichever process enabled it, with
/// no daemon to ask, so a second process cannot tell "nobody is routing" from
/// "the menubar app is routing" by looking at its own handle - it holds `None`
/// either way. That blindness is what let a second `enable` start a competing
/// engine and record Gate's own PAC as the state to restore, and what made
/// `proxy status` report "stopped" while the machine was demonstrably routed.
///
/// [`engine_likely_running`] alone is not enough to decide either question: the
/// snapshot survives a crash, so treating its presence as "an engine is live"
/// would refuse the enable that is the user's way back. Probing the persisted
/// port separates the two, exactly as `bind_preferred` separates a live
/// listener from a TIME_WAIT remnant - a live listener accepts, a dead port
/// refuses. Both conditions are required: the snapshot proves *Gate* turned
/// routing on, the probe proves someone is still serving it.
///
/// Linux has no use for this - its engine is a daemon with a control socket, so
/// a second process adopts it rather than guessing (see `manager_linux`).
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn engine_hosted_elsewhere() -> Option<u16> {
    if !engine_likely_running() {
        return None;
    }
    let port = system_proxy::load_port().ok().flatten()?;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(250))
        .ok()
        .map(|_| port)
}

/// The loopback base URL CLI tools point at to route through the reverse-proxy
/// relay, or `None` if no relay port has ever been bound (so nothing to point
/// at yet). Reads the persisted port, so it's stable across restarts and valid
/// even while the engine is momentarily down. This is the base URL the tool
/// integrations write into their config instead of a gateway URL + key.
pub fn relay_base_url() -> Option<String> {
    relay::load_persisted_port().map(relay::base_url)
}

/// The relay's own liveness path, re-exported so callers and the e2e suite spell
/// it once.
pub use relay::HEALTH_PATH as RELAY_HEALTH_PATH;

/// Is the relay actually answering on the port config-routed tools are pointed
/// at?
///
/// A TCP connect would only prove *something* is listening on that port, which
/// after a port reuse is a claim we cannot support. Asking for
/// [`RELAY_HEALTH_PATH`] and requiring a 204 proves it is our relay. The request
/// never leaves the loopback interface and never reaches the gateway, so this is
/// free to run on a status refresh.
///
/// `.no_proxy()` for the same reason every control-plane client in this codebase
/// sets it: the app may have pointed `HTTPS_PROXY` at its own engine, and a
/// loopback health check routed back through that would be measuring the wrong
/// hop.
///
/// Scope: this is the route for *config* integrations, which write the relay
/// base URL into their config. Proxy-routed members (the catalog domains) hang
/// off the engine instead and keep their existing certificate-trust treatment.
pub fn probe_relay_route() -> crate::routing_health::RouteHealth {
    use crate::routing_health::RouteHealth;

    let Some(base) = relay_base_url() else {
        // No port has ever been bound, so there is nothing for a tool to be
        // pointed at. That is a definite negative, not an unknown.
        return RouteHealth::Unreachable;
    };
    let Ok(client) = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    else {
        return RouteHealth::Unknown;
    };
    match client.get(format!("{base}{RELAY_HEALTH_PATH}")).send() {
        Ok(resp) if resp.status() == reqwest::StatusCode::NO_CONTENT => RouteHealth::Reachable,
        // Something answered but not our relay - a port collision, or a build
        // old enough to predate this path. Reporting `Unreachable` would claim
        // the port is dead when it is occupied; neither is confirmed, so say so.
        Ok(_) => RouteHealth::Unknown,
        Err(_) => RouteHealth::Unreachable,
    }
}

/// Whether something is accepting connections on the persisted relay port
/// right now - the engine-hosted relay or a standalone `proxy relay` host,
/// either counts (which is why this probes the port instead of reading
/// [`engine_likely_running`]: the standalone host touches no system proxy and
/// leaves no snapshot). This is the *liveness* half of a relay-tool status
/// check; [`relay_base_url`] alone is *identity* - the port file persists
/// across restarts precisely so tool configs stay valid - so checking a
/// config against it reads Connected even while the tool is dialing a dead
/// port. A refused loopback connect returns immediately; the timeout only
/// bounds pathological states.
pub fn relay_listening() -> bool {
    relay::load_persisted_port().is_some_and(|port| {
        std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            std::time::Duration::from_millis(300),
        )
        .is_ok()
    })
}

/// Run the CLI reverse-proxy relay as a standalone, blocking headless host (no
/// MITM, no CA trust, no system-proxy changes). For environments with no
/// menubar app - containers, servers, CI - so CLI tools pointed at the relay
/// still route through Gate. Blocks until the process is killed. This backs
/// `proxy relay`; [`ProxyManager::enable`] hosts the same relay alongside the
/// MITM engine, so the two are alternatives, not steps.
pub fn serve_relay() -> anyhow::Result<()> {
    relay::serve()
}

/// Block until the process is asked to stop: SIGINT or SIGTERM on unix, Ctrl-C
/// on Windows.
///
/// Backs `proxy enable --foreground`. The engine lives in the process-lifetime
/// [`manager`] static, so on macOS - which hosts it in-process, with no daemon
/// to outlive the caller - routing lasts exactly as long as the process that
/// enabled it. `proxy enable` returns immediately, so from the CLI the engine
/// has always died on the way out, leaving the system proxy pointed at a port
/// nothing answers. Parking here is what lets a headless machine host it
/// (launchd, systemd, a CI job) instead of only the menubar app.
///
/// SIGTERM as well as SIGINT because that is what a service manager sends to
/// stop a unit; without it the caller could not restore the system proxy on the
/// way down, which is the whole reason this is worth blocking for.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn wait_for_shutdown() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building the shutdown-wait runtime")?;
    rt.block_on(async {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut term =
                signal(SignalKind::terminate()).context("installing the SIGTERM handler")?;
            let mut int =
                signal(SignalKind::interrupt()).context("installing the SIGINT handler")?;
            tokio::select! {
                _ = term.recv() => {}
                _ = int.recv() => {}
            }
        }
        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c()
                .await
                .context("waiting for Ctrl-C")?;
        }
        Ok::<(), anyhow::Error>(())
    })
}

/// Path to the local root CA's public cert on disk. Tools that ship their own
/// trust bundle instead of using the OS trust store (Node, Python) have to be
/// pointed at this to accept the engine's minted leaf certs.
pub fn ca_cert_path() -> Result<std::path::PathBuf> {
    Ok(crate::env::app_support_dir()?
        .join("proxy")
        .join("ca-cert.pem"))
}

/// The proxy environment variables the system proxy exports for an engine on
/// `port`, as `(name, value)` pairs - the same set [`system_proxy`] writes to
/// the Linux drop-in, macOS `launchctl` and the Windows per-user environment.
///
/// Public so the end-to-end test can route a real external process using
/// exactly what production exports, rather than a hand-copied list that could
/// silently drift from it.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn proxy_env_vars(port: u16) -> Result<Vec<(&'static str, String)>> {
    proxy_env::case_sensitive(port)
}

// --- The environment channel, as a thing the user can decline ---------------
//
// Cross-platform wrappers over the per-OS `system_proxy` env export, so the
// `env-proxy` integration can be one platform-agnostic file. Unsupported
// platforms get inert answers rather than a missing symbol.

/// Can the env export be turned off while the OS proxy setting stays on?
///
/// False on Linux, where the `environment.d` drop-in *is* the system proxy:
/// there is no PAC, so declining the variables would mean declining routing.
/// True on macOS/Windows, where the PAC covers GUI apps independently.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn env_export_is_separable() -> bool {
    system_proxy::ENV_CHANNEL_SEPARABLE
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn env_export_is_separable() -> bool {
    false
}

/// The proxy URL currently in the user's environment, read back from the OS
/// rather than from our own record - so status reports what is true, not what
/// we last tried to write.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn exported_proxy_url() -> Option<String> {
    system_proxy::exported_proxy().ok().flatten()
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn exported_proxy_url() -> Option<String> {
    None
}

/// Whether the user wants the proxy exported into their environment. Defaults
/// to true; see [`proxy_env::export_opted_in`].
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn env_export_opted_in() -> bool {
    proxy_env::export_opted_in()
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn env_export_opted_in() -> bool {
    false
}

/// Record the user's choice about the env export.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn set_env_export_opted_in(enabled: bool) -> Result<()> {
    proxy_env::set_export_opted_in(enabled)
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn set_env_export_opted_in(_enabled: bool) -> Result<()> {
    anyhow::bail!("the proxy environment export is not supported on this platform")
}

/// Export the variables now, without waiting for a routing toggle. Only where
/// the channel is separable; on Linux the drop-in already did it.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn enable_env_export(port: u16) -> Result<()> {
    system_proxy::enable_env(port)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn enable_env_export(_port: u16) -> Result<()> {
    Ok(())
}

/// Withdraw the variables now. Only where the channel is separable.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn disable_env_export() -> Result<()> {
    system_proxy::disable_env()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn disable_env_export() -> Result<()> {
    Ok(())
}

/// Record the user's choice about the env export *and* apply it now.
///
/// One implementation shared by the `env-proxy` integration and the Tauri
/// command behind the UI switch, so the two cannot drift on the part that is
/// easy to get wrong: applying immediately, rather than leaving the user to
/// toggle routing off and on before their choice takes effect.
///
/// Withdrawing is best-effort on the inseparable platform (Linux), where the
/// variables belong to the routing drop-in and only routing-off clears them.
pub fn set_env_export(enabled: bool) -> Result<()> {
    set_env_export_opted_in(enabled)?;
    if !env_export_is_separable() {
        return Ok(());
    }
    if enabled {
        // Only ever export against a *live* engine. The persisted port outlives
        // a disable (so the engine can rebind the same address), so exporting
        // off it with routing off would point every command-line tool at a dead
        // address - the one failure worse than not routing. With routing off we
        // just record the choice; `manager.enable()` applies it next time.
        match engine_port().filter(|_| engine_likely_running()) {
            None => Ok(()),
            Some(port) => enable_env_export(port),
        }
    } else {
        disable_env_export()
    }
}

/// The port the engine is bound to per the persisted record, if any.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn engine_port() -> Option<u16> {
    system_proxy::load_port().ok().flatten()
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn engine_port() -> Option<u16> {
    None
}

/// Loopback URL of the MITM engine's forward (CONNECT) proxy, for tools that
/// take a proxy URL of their own rather than a base URL - as distinct from
/// [`relay_base_url`], which is the *reverse* proxy CLI tool configs point at.
///
/// `None` unless the proxy is actually routing: unlike the relay, a tool that
/// hands all its egress to a proxy URL has no fallback path, so handing one out
/// while the engine is down would take that tool's whole network with it. The
/// persisted port alone can't answer that - it survives a disable so the engine
/// can rebind the same address - hence the [`engine_likely_running`] gate. Use
/// [`persisted_engine_proxy_url`] where the question is "is this address ours"
/// rather than "may I route through it now".
///
/// Both halves of the liveness question are required, for the reason
/// [`engine_hosted_elsewhere`] already spells out: the snapshot proves *Gate*
/// turned routing on, and the probe proves someone is still serving it. The
/// snapshot outlives a process that died without running its revert (SIGKILL,
/// OOM, power loss) - `reconcile_on_startup` clears it, but not until the app
/// is launched again. In that window the snapshot alone reports every
/// proxy-routed tool as Connected while its egress is a dead port, and the
/// process best placed to notice is the CLI, which reads these same files
/// beside a menubar app that is no longer there.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn engine_proxy_url() -> Option<String> {
    if !engine_likely_running() || !engine_listening() {
        return None;
    }
    persisted_engine_proxy_url()
}

/// Whether something is accepting on the persisted engine port right now.
///
/// Ordered so the common case is free. The process hosting the engine holds a
/// handle that already answers this ([`ProxyManager::hosts_live_engine`]), and
/// it is also the process that polls tool status on a timer, so it never
/// reaches the probe. A process with no handle - the CLI, or a second app
/// instance - pays one loopback connect, which a dead port refuses immediately;
/// the timeout only bounds pathological states, matching
/// [`engine_hosted_elsewhere`].
///
/// Callers gate on [`engine_likely_running`] first, so a machine with routing
/// off costs nothing at all: no snapshot, no probe.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn engine_listening() -> bool {
    if manager().hosts_live_engine() {
        return true;
    }
    engine_port().is_some_and(|port| {
        std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            std::time::Duration::from_millis(250),
        )
        .is_ok()
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn engine_proxy_url() -> Option<String> {
    None
}

/// The engine's forward-proxy URL from the persisted port, whether or not the
/// engine is up. This is the *identity* of our proxy address, which is what a
/// drift check wants: a config pointing here is ours even while routing is off.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn persisted_engine_proxy_url() -> Option<String> {
    system_proxy::load_port()
        .ok()
        .flatten()
        .map(|port| format!("http://127.0.0.1:{port}"))
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn persisted_engine_proxy_url() -> Option<String> {
    None
}

/// Non-secret hint the tool config (or the MITM rewrite) sets, telling the
/// gateway which upstream to forward to.
pub(crate) const UPSTREAM_URL_HEADER: &str = "x-gate-upstream-url";
/// Legacy credential header (Gate workspace key), injected when no OAuth token
/// is present.
pub(crate) const GATE_KEY_HEADER: &str = "x-gate-api-key";
/// OAuth credential header (Cognito access token); takes precedence over the
/// API key when present.
pub(crate) const GATE_AUTHORIZATION_HEADER: &str = "x-gate-authorization";
/// Selected-org header, injected alongside the OAuth token (the gateway
/// requires it on every OAuth request).
pub(crate) const GATE_ORG_HEADER: &str = "x-gate-org-id";
/// This installation's id, so the activity view can group traffic by machine.
/// Self-asserted and non-secret: it identifies nothing to authorize against.
pub(crate) const GATE_INSTALL_ID_HEADER: &str = "x-gate-install-id";
/// Which tool sent the request, when we can tell. Feeds the per-tool series in
/// the activity view.
pub(crate) const GATE_CLIENT_HEADER: &str = "x-gate-client";
/// What the user calls this machine, so the gateway can show traffic under a
/// human name rather than an install id. Self-asserted and non-secret, like the
/// two above.
///
/// Sent only when the user actually named the device. There is no hostname
/// fallback on the wire - see `preferences::device_label` - because onboarding
/// offers to skip naming, and a hostname usually carries a person's name. An
/// unnamed device is attributed by its install id alone.
pub(crate) const GATE_DEVICE_NAME_HEADER: &str = "x-gate-device-name";
/// The models the user chose for this tool, comma-separated and in preference
/// order (AG-588 / AG-590).
///
/// Unlike the two above this is not a label on the request - it **changes what
/// the gateway serves**, so the gateway rewrites the body's `model` to the first
/// entry. Sent only when the user set that tool to a Gate model; absent means
/// the tool's own choice stands, which is the default and must stay the default.
pub(crate) const GATE_MODEL_HEADER: &str = "x-gate-model";

/// Stamp the attribution headers the activity view groups by.
///
/// Deliberately infallible. These headers exist so a dashboard can say "this
/// machine, this tool"; the request they ride on is carrying the user's actual
/// work. Anything we can't determine - no install id, a value the header codec
/// rejects, an unrecognised client - is simply left off, and the gateway
/// records that request as unattributed exactly as it did before attribution
/// existed. Failing a request to protect a chart would be the wrong trade.
///
/// Any value the caller sent is overwritten: a tool cannot label its traffic as
/// another machine's - and, for the model header, a tool cannot ask Gate to
/// serve something the user did not choose.
///
/// **The model header is not attribution and does not follow its rules.** The
/// other three are labels: leaving one off costs a chart a data point. This one
/// decides what the user is billed for, so it is stamped only from stored intent
/// and only when a tool was positively identified. An unrecognised tool sends no
/// override at all rather than a best guess, because guessing here would serve -
/// and charge for - a model chosen for a different tool.
fn inject_attribution(headers: &mut HeaderMap) {
    headers.remove(GATE_INSTALL_ID_HEADER);
    if let Some(id) = crate::primitives::install_id_cached() {
        if let Ok(value) = HeaderValue::from_str(id) {
            headers.insert(HeaderName::from_static(GATE_INSTALL_ID_HEADER), value);
        }
    }
    // Absent unless the user named this device, and a name the header codec
    // rejects (a rename can be any Unicode) is left off rather than escaped, per
    // the rule above. The length is bounded at the preferences layer, so this
    // cannot be the header that blows the block size.
    headers.remove(GATE_DEVICE_NAME_HEADER);
    if let Some(value) =
        crate::preferences::device_label().and_then(|name| HeaderValue::from_str(&name).ok())
    {
        headers.insert(HeaderName::from_static(GATE_DEVICE_NAME_HEADER), value);
    }
    let tool = client_tool(headers);
    headers.remove(GATE_CLIENT_HEADER);
    if let Some(slug) = tool {
        headers.insert(
            HeaderName::from_static(GATE_CLIENT_HEADER),
            HeaderValue::from_static(slug),
        );
    }
    inject_model_choice(headers, tool);
}

/// Stamp the chosen models for `tool`, or strip the header entirely.
///
/// Stripped unconditionally first, and that is the security-relevant half: a
/// tool that set `x-gate-model` itself would otherwise pick its own Gate model
/// and bill the user for it, having never been offered the confirmation. The
/// only thing that may populate this header is a choice the user stored.
///
/// Comma-separated because AG-590 enables a set. Which of the set a request uses
/// is the gateway's decision, not this one - see the header's own doc. The order
/// is the user's, preserved.
fn inject_model_choice(headers: &mut HeaderMap, tool: Option<&'static str>) {
    headers.remove(GATE_MODEL_HEADER);
    let Some(slug) = tool else { return };
    let Some(models) = crate::preferences::gate_models_for(slug) else {
        return;
    };
    // A model id that cannot be a header value is dropped rather than escaped:
    // the ids are `provider/model`, so anything that fails here did not come
    // from a catalogue, and sending part of a set would serve a model the user
    // did not put first.
    if let Ok(value) = HeaderValue::from_str(&models.join(",")) {
        headers.insert(HeaderName::from_static(GATE_MODEL_HEADER), value);
    }
}

/// Is this request one Gate itself will serve, rather than one it forwards to
/// the tool's own provider?
///
/// Answered by the presence of [`GATE_MODEL_HEADER`], which
/// [`inject_model_choice`] has just decided: it is set only when the user put
/// this tool on a Gate model. Reading it back rather than re-deriving the choice
/// keeps one decision in one place - two computations of "is this served?" could
/// disagree, and the disagreement would be a request billed one way and routed
/// the other.
pub(crate) fn serves_gate_model(headers: &HeaderMap) -> bool {
    headers.contains_key(GATE_MODEL_HEADER)
}

/// Test seam for the attribution + model-choice injection.
///
/// The injection itself is `pub(crate)` because nothing outside the proxy should
/// stamp these headers. It still needs covering from an integration test rather
/// than a unit test - it reads the preferences file, and the app-support override
/// is process-global - so this is the narrowest door that allows it.
#[doc(hidden)]
pub mod testing {
    use hyper::header::HeaderMap;

    /// The header name, so a test asserts on the same constant the code sends.
    pub const GATE_MODEL_HEADER_NAME: &str = super::GATE_MODEL_HEADER;

    /// Same, for the device-name label.
    pub const GATE_DEVICE_NAME_HEADER_NAME: &str = super::GATE_DEVICE_NAME_HEADER;

    pub fn inject_attribution_for_tests(headers: &mut HeaderMap) {
        super::inject_attribution(headers);
    }

    /// Whether the injection decided Gate serves this request.
    pub fn serves_gate_model(headers: &HeaderMap) -> bool {
        super::serves_gate_model(headers)
    }

    pub fn strip_tool_credential_for_tests(headers: &mut HeaderMap) {
        super::strip_client_auth(headers);
    }
}

/// Guess which tool sent a request from its own `User-Agent`.
///
/// A heuristic, and the honest ceiling of what either path can know: the relay
/// is keyed by provider slug rather than by tool, and the MITM engine sees only
/// a CONNECT to a host. Matching is substring-based because these agents append
/// their own versions and platform strings, which we don't want to track.
///
/// Unrecognised is `None`, never a guess. A wrong slug is worse than no slug:
/// it would attribute one tool's traffic to another in a view the user reads to
/// find out what their machine is doing.
fn client_tool(headers: &HeaderMap) -> Option<&'static str> {
    let ua = headers.get(hyper::header::USER_AGENT)?.to_str().ok()?;
    let ua = ua.to_ascii_lowercase();
    // `claude-cli` is Claude Code's agent; the rest identify themselves by name.
    // Slugs match `registry::ToolId::slug`, so one tool is one series.
    [
        ("claude-cli", "claude-code"),
        ("codex", "codex"),
        ("opencode", "opencode"),
        ("openclaw", "openclaw"),
        ("hermes", "hermes"),
    ]
    .into_iter()
    .find_map(|(needle, slug)| ua.contains(needle).then_some(slug))
}

/// Inject the live Gate credential into `headers`, the single precedence rule
/// shared by the MITM engine ([`engine::apply_rewrite`]) and the loopback
/// [`relay`] so the two paths can't drift.
///
/// If the caller already set an `X-Gate-Api-Key`, that's respected: the Gate
/// headers are left exactly as they arrived and nothing is injected. Otherwise
/// any stray `X-Gate-Authorization` / `X-Gate-Org-Id` are stripped (so an org
/// can't ride alongside the credential we inject) and the live credential is
/// added: a non-empty `oauth_token` wins - `X-Gate-Authorization: Bearer
/// <token>` plus `X-Gate-Org-Id` when `org_id` is `Some` - otherwise the legacy
/// `X-Gate-Api-Key`.
///
/// Attribution ([`inject_attribution`]) is stamped either way: it says which
/// machine the request left, which is true no matter whose credential carries
/// it, and it is not a credential decision.
///
/// In `Payg` the tool's own credential is REMOVED (`Authorization` /
/// `x-api-key`). The gateway classifies any non-`sk-gw-` value in those slots
/// as a passthrough token, which forces BYOK and is then refused for want of an
/// upstream URL, so a leftover `sk-ant-…` does not merely go unused - it breaks
/// the request. Nothing is lost by dropping it: the gateway strips inbound
/// `authorization` / `x-api-key` before forwarding anyway and re-keys with the
/// provider account's own credential. The strip runs ahead of the
/// caller-supplied-key short-circuit below, because a caller that sets its own
/// `X-Gate-Api-Key` can just as easily be carrying a provider token beside it.
pub(crate) fn inject_gate_credential(
    headers: &mut HeaderMap,
    api_key: &str,
    oauth_token: Option<&str>,
    org_id: Option<&str>,
    mode: BillingMode,
) -> Result<()> {
    inject_attribution(headers);
    if mode == BillingMode::Payg {
        strip_client_auth(headers);
    }
    if headers.contains_key(GATE_KEY_HEADER) {
        return Ok(());
    }
    headers.remove(GATE_AUTHORIZATION_HEADER);
    headers.remove(GATE_ORG_HEADER);
    match oauth_token.filter(|t| !t.is_empty()) {
        Some(token) => {
            headers.insert(
                HeaderName::from_static(GATE_AUTHORIZATION_HEADER),
                HeaderValue::from_str(&format!("Bearer {token}"))
                    .context("building x-gate-authorization header")?,
            );
            if let Some(org) = org_id {
                headers.insert(
                    HeaderName::from_static(GATE_ORG_HEADER),
                    HeaderValue::from_str(org).context("building x-gate-org-id header")?,
                );
            }
        }
        None => {
            headers.insert(
                HeaderName::from_static(GATE_KEY_HEADER),
                HeaderValue::from_str(api_key).context("building x-gate-api-key header")?,
            );
        }
    }
    Ok(())
}

/// Which credential slots a tool authenticates to its provider with. Removed on
/// any rewrite Gate serves; never touched on a passthrough hop, where they are
/// the only thing that can authenticate the request.
///
/// Both slots, because the two providers this routes to disagree: OpenAI-shaped
/// APIs authenticate on `Authorization`, Anthropic on `x-api-key`.
const CLIENT_AUTH_HEADERS: [&str; 2] = ["authorization", "x-api-key"];

/// Drop the tool's own upstream credential from a request Gate is paying for.
/// See [`inject_gate_credential`] for why PAYG requires this rather than merely
/// tolerating the header.
///
/// On a served request the model, the provider and the bill are all Gate's, so
/// the tool's key is not needed and is not sent. The gateway would strip it
/// before forwarding upstream anyway - `buildForwardHeaders` removes
/// `authorization` and `x-api-key` and re-injects the right credential - so this
/// is not what stands between the user's key and a third party. It is narrower
/// and still worth doing: there is no reason for Gate to *receive* a credential
/// it will not use, and not sending it is cheaper than trusting every future
/// code path on the far side to keep discarding it.
fn strip_client_auth(headers: &mut HeaderMap) {
    for name in CLIENT_AUTH_HEADERS {
        headers.remove(name);
    }
}

/// Catalog slugs PAYG can serve, i.e. the ones whose forwarded path is a shape
/// the gateway's reseller router understands (`/v1/messages`,
/// `/v1/chat/completions`, `/v1/responses`).
///
/// An allowlist, not a denylist, so a domain added later defaults to BYOK and a
/// new entry can never start spending an org's balance by omission.
///
/// Everything left out is left out for a reason:
/// - `claude-web`, `chatgpt-apps` - consumer chat surfaces authenticated by a
///   session cookie and covered by the user's own subscription. Gate estimates
///   their cost rather than billing it, and their paths are not inference-API
///   shapes the reseller router serves.
/// - `chatgpt` - Codex's ChatGPT-subscription Responses route. Subscription
///   traffic is by definition not pay-as-you-go; Codex reaches PAYG through the
///   `openai` entry instead (see `integrations::codex`).
/// - `opencode` - its inference lives under `/zen/v1/…`, which is not a path
///   the reseller router recognises.
const PAYG_ELIGIBLE_SLUGS: [&str; 3] = ["anthropic", "openai", "openrouter"];

/// The mode to actually route `slug` under. PAYG only applies to the domains in
/// [`PAYG_ELIGIBLE_SLUGS`]; every other domain keeps its BYOK shape even while
/// the account is in PAYG, because rewriting it without an upstream URL would
/// break it and route nothing.
pub(crate) fn effective_billing_mode(mode: BillingMode, slug: &str) -> BillingMode {
    match mode {
        BillingMode::Byok => BillingMode::Byok,
        BillingMode::Payg if PAYG_ELIGIBLE_SLUGS.contains(&slug) => BillingMode::Payg,
        BillingMode::Payg => BillingMode::Byok,
    }
}

/// One routable provider. The built-in set is defined by
/// [`default_domains`]; persisted config only flips `enabled` per `slug`,
/// so adding a new built-in domain automatically surfaces it in the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyDomain {
    /// Stable identifier used in config + the `proxy_set_domain` command.
    pub slug: String,
    /// Human-readable name shown in the UI.
    pub display_name: String,
    /// Exact hostnames to intercept (e.g. `api.anthropic.com`). A CONNECT
    /// to any other host is blind-tunnelled.
    pub hosts: Vec<String>,
    /// Value injected as `X-Gate-Upstream-Url` - where Gate forwards the
    /// rewritten request (e.g. `https://api.anthropic.com`).
    pub upstream_url: String,
    /// Path prefixes that are inference calls and should be rewritten to
    /// the gateway (e.g. `/v1/`).
    pub rewrite_prefixes: Vec<String>,
    /// Extra constraint: when non-empty, a path matched by `rewrite_prefixes`
    /// must ALSO end with one of these to be rewritten. Anything else abstains.
    ///
    /// The catalog never sets this and config cannot supply it (`serde(default)`,
    /// so an older persisted file still loads). It exists for one job:
    /// [`rules_for_client`] uses it to hand a BROWSER just the chat turn on an
    /// entry whose turn cannot be isolated by prefix - claude.ai's completion
    /// path carries the conversation id BEFORE the distinguishing final segment,
    /// so `/organizations/` is the only prefix available and it covers 27
    /// endpoints.
    #[serde(default)]
    pub rewrite_suffixes: Vec<String>,
    /// Path prefixes on an intercepted host that must NOT be rewritten -
    /// they pass through to the real upstream (e.g. an app's
    /// `/api/desktop/` auto-updater channel).
    pub passthrough_prefixes: Vec<String>,
    /// Whether the user has this domain turned on.
    pub enabled: bool,
    /// Whether Gate can actually upstream this provider today. Unsupported
    /// domains render as disabled rows in the UI and can't be enabled.
    pub supported: bool,
}

impl ProxyDomain {
    fn matches_host(&self, host: &str) -> bool {
        self.hosts.iter().any(|h| h.eq_ignore_ascii_case(host))
    }
}

/// Snapshot of the proxy subsystem for the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyState {
    /// Whether the loopback engine is currently running.
    pub running: bool,
    /// Loopback port the engine is bound to (when running).
    pub port: Option<u16>,
    /// Loopback port serving the PAC script the system proxy points at (when
    /// running). PAC-driven platforms only (macOS/Windows); always `None` on
    /// Linux, which wires env-var proxies with no PAC.
    #[serde(default)]
    pub pac_port: Option<u16>,
    /// Whether our root CA is trusted in the OS trust store.
    pub ca_trusted: bool,
    /// Whether Gate is putting its proxy into the user's environment - the
    /// channel that routes command-line tools, as distinct from the OS proxy
    /// setting that routes GUI apps. A user-held choice, because the variables
    /// are machine-wide; see [`crate::integrations::env_proxy`].
    #[serde(default)]
    pub env_export_opted_in: bool,
    /// Whether that choice is offerable at all. False on Linux, where the
    /// `environment.d` drop-in *is* the system proxy, so the two channels
    /// cannot be separated and the UI must not present a switch for it.
    #[serde(default)]
    pub env_export_separable: bool,
    /// Loopback base URL config-routed tools are pointed at, from the persisted
    /// relay port - `None` before any port has been bound.
    ///
    /// Non-secret, and already written verbatim into every config-routed tool's
    /// own file, so surfacing it reveals nothing the user cannot read on disk.
    /// The drift-review dialog needs it: telling someone Gate will overwrite
    /// their routing values without showing what it will write in their place
    /// asks them to approve a value they cannot see.
    #[serde(default)]
    pub relay_base_url: Option<String>,
    /// The full domain catalog with current enabled flags.
    pub domains: Vec<ProxyDomain>,
}

/// What the engine should do with a request on an intercepted host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Host isn't one of ours - should never reach here post-intercept,
    /// but treated as a safe passthrough if it does.
    Tunnel,
    /// Matched host but not an inference path: forward to the real
    /// upstream unchanged.
    Passthrough,
    /// Rewrite to the gateway, injecting this upstream URL. `slug` names the
    /// catalog entry that claimed the path, so the caller can resolve the
    /// billing shape for it ([`effective_billing_mode`]).
    Rewrite { upstream_url: String, slug: String },
}

/// True if any enabled domain claims `host`. Used by the engine's
/// `should_intercept` to gate MITM at the CONNECT stage.
pub(crate) fn should_intercept_host(domains: &[ProxyDomain], host: &str) -> bool {
    domains.iter().any(|d| d.enabled && d.matches_host(host))
}

/// Route selector carried only by Claude Code's explicit proxy URL. It is not
/// a credential: the proxy is already restricted to the local owner. The
/// selector lets the engine distinguish Claude Code from desktop applications
/// that reach the same `api.anthropic.com` host through the system proxy, so
/// the Desktop domain switch can remain independent without letting a connected
/// Claude Code session blind-tunnel around Gate.
pub(crate) const CLAUDE_CODE_PROXY_AUTH: &str = "Basic Z2F0ZS1jbGF1ZGUtY29kZTpyb3V0ZQ==";

/// The catalog entry Claude Code's selector forces on, already `enabled`.
///
/// Built once: the forced path would otherwise rebuild the whole catalog per
/// request. It is also the single place the forced route's host is named, so
/// the CONNECT-stage check and the rule the engine pushes cannot drift from
/// each other, or from the catalog, if `hosts` ever changes.
pub(crate) fn claude_code_route_domain() -> &'static ProxyDomain {
    static ENTRY: std::sync::OnceLock<ProxyDomain> = std::sync::OnceLock::new();
    ENTRY.get_or_init(|| {
        let mut domain = default_domains()
            .into_iter()
            .find(|domain| domain.slug == "anthropic")
            .expect("the built-in catalog always carries the anthropic entry");
        domain.enabled = true;
        domain
    })
}

/// Add the Claude Code route selector to an engine URL.
pub(crate) fn claude_code_proxy_url(engine_url: &str) -> Result<String> {
    let authority = engine_url
        .strip_prefix("http://")
        .context("Claude Code proxy URL must use http://")?;
    Ok(format!("http://gate-claude-code:route@{authority}"))
}

/// The catalog entry that would MITM `host`, whether or not it is switched on.
/// The question [`should_intercept_host`] cannot answer: a caller that wants to
/// *report* on a host's coverage needs the entry precisely when it is off.
///
/// Catalog order decides, and deliberately so - it is the order [`decide`]
/// resolves in, which returns on the first enabled host match. That is what
/// makes `chatgpt.com` answer with the MITM `chatgpt-apps` entry rather than the
/// relay-only `chatgpt` one behind it on the same host. Unsupported entries are
/// skipped: Gate cannot upstream them, so there is no switch worth naming.
///
/// Host-level, not URL-level, unlike [`resolve_endpoint`]: the engine gates MITM
/// on the CONNECT host alone, so a base URL's path has no bearing on whether its
/// traffic is visible.
pub(crate) fn domain_claiming_host<'a>(
    domains: &'a [ProxyDomain],
    host: &str,
) -> Option<&'a ProxyDomain> {
    domains.iter().find(|d| d.supported && d.matches_host(host))
}

/// The path component of a catalog `upstream_url` - `/api` for
/// `https://openrouter.ai/api`, `""` for a bare host. Gate appends the
/// forwarded path to the upstream URL verbatim, so this segment is the part of
/// the provider's path that travels in the `X-Gate-Upstream-Url` header rather
/// than in the request line.
pub(crate) fn upstream_path(upstream_url: &str) -> &str {
    let after_scheme = upstream_url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(upstream_url);
    after_scheme
        .find('/')
        .map(|i| after_scheme[i..].trim_end_matches('/'))
        .unwrap_or("")
}

/// Remove an upstream URL's own path prefix from a request path, on a path
/// boundary: `/api/v1/chat` under upstream path `/api` becomes `/v1/chat`.
///
/// `None` means the request is outside the upstream's subtree (`/apifoo`, or a
/// wholly unrelated path), which callers treat as "not ours".
pub(crate) fn strip_upstream_path<'a>(path: &'a str, upstream_path: &str) -> Option<&'a str> {
    if upstream_path.is_empty() {
        return Some(path);
    }
    let rest = path.strip_prefix(upstream_path)?;
    if rest.is_empty() {
        Some("/")
    } else if rest.starts_with('/') {
        Some(rest)
    } else {
        None
    }
}

/// Which front-end sent an intercepted request.
///
/// The app and the browser share `chatgpt.com` and even share endpoints - both
/// POST `/backend-api/f/conversation` - so the host+path pair that drives
/// [`decide`] cannot tell them apart. This can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientClass {
    /// A first-party desktop app: the ChatGPT app shell, Codex, or Codex's MCP
    /// client.
    App,
    /// A browser tab on the vendor's website.
    Web,
    /// Neither signature matched. Every third-party client that honours the
    /// system proxy lands here - OpenClaw, Hermes, an in-house script - and all
    /// of them are routed. Only [`Web`](Self::Web) is held back.
    Unknown,
}

/// `anthropic-client-platform` values that name the client for us. The web value
/// is the one routing keys on; the desktop value is here so the debug log can
/// tell them apart, and Gate matches the same string for its `claude-desktop`
/// platform.
const ANTHROPIC_WEB_PLATFORM: &str = "web_claude_ai";
const ANTHROPIC_DESKTOP_PLATFORM: &str = "desktop_app";

/// One entry's browser scope. See [`BROWSER_ROUTED`].
pub(crate) struct BrowserScope {
    slug: &'static str,
    /// Retained from the entry's own `rewrite_prefixes`, by exact match.
    prefixes: &'static [&'static str],
    /// Additionally required, when non-empty. See `ProxyDomain::rewrite_suffixes`.
    suffixes: &'static [&'static str],
}

/// What a BROWSER may route on entries whose host it shares with a first-party
/// app. Everything else on those entries is withheld from it; every other entry,
/// and every other client class, is untouched.
///
/// The rows are named after apps but matched on HOST, so flipping one also
/// intercepts the same site in a browser. Capturing the browser's CONVERSATION is
/// wanted - that is a chat turn like any other, and the whole point of the row.
/// Proxying the rest of the site's plumbing through Gate is not: it carries the
/// browsing session's own cookie and a run of model-less requests that no part of
/// the pipeline was built for. So the browser is narrowed to the turn.
///
/// Only what is POSITIVELY identified as the website is narrowed, so OpenClaw,
/// Hermes and anything else proxy-honouring keeps the entry in full. That sets the
/// failure direction: if a vendor stops sending the markers [`classify_client`]
/// reads, its browser traffic classifies as `Unknown` and routes the whole entry
/// again. The alternative - narrow everything that is not a positively-identified
/// app - fails the other way and would silently drop every third-party client,
/// which is the larger loss.
///
/// A code-level policy rather than a `ProxyDomain` field on purpose: a
/// serialized field would imply a per-domain switch the UI does not have. When
/// it grows one, this becomes that field and the constant goes away.
const BROWSER_ROUTED: &[BrowserScope] = &[
    // The chat turn, and nothing else. The browser also hits
    // `/backend-api/wham/*` on this entry, which is task and settings plumbing
    // rather than a conversation, so a prefix alone is enough here.
    BrowserScope {
        slug: "chatgpt-apps",
        prefixes: &["/backend-api/f/conversation"],
        suffixes: &[],
    },
    // claude.ai needs the suffix. Its only prefix is `/organizations/`, and the
    // completion path carries the conversation id BEFORE the distinguishing final
    // segment, so no prefix can separate the chat turn from the other 26
    // endpoints under that tree - memory, projects, skills, plugins, MCP servers,
    // member invites, subscription status. Requiring `/completion` isolates
    // exactly the turn.
    //
    // `.../chat_conversations/{id}/title` is excluded by the same rule, which is
    // right: that is the vendor's own title generator, not a user turn.
    BrowserScope {
        slug: "claude-web",
        prefixes: &["/organizations/"],
        suffixes: &["/completion"],
    },
];

/// The token every real browser's user-agent opens with.
const BROWSER_UA_PREFIX: &str = "Mozilla/";

/// True for a user-agent that is not a browser's own. Non-empty, and it does
/// not open with `Mozilla/`.
///
/// Wider than [`browser_ua_without_product_token`] on purpose: the reporting
/// net has to be wider than the classifier's for a rename to show up in it at
/// all. Deliberately not a classification signal on its own - plenty of
/// third-party clients look exactly like this and they are ordinary. The
/// classifier only consumes it through [`browser_ua_without_product_token`],
/// which additionally demands the wrapped shape.
pub(crate) fn is_non_browser_ua(ua: &str) -> bool {
    let ua = ua.trim_start();
    !ua.is_empty() && !ua.starts_with(BROWSER_UA_PREFIX)
}

/// The browser-shaped remainder of an app shell's user-agent, or `None` when
/// there is no product token in front of one.
///
/// This is both the app-shell SIGNAL for [`classify_client`] and the value the
/// chatgpt.com strip experiment forwards, deliberately one function: the thing
/// that makes a UA recognisable as an app shell is exactly the thing being
/// removed, and two definitions of that could drift apart.
///
/// Keyed on the SHAPE - a product token, then an otherwise ordinary browser UA
/// (`CodexBrowser Mozilla/5.0 …`) - rather than on a roster of names. The
/// vendor has already renamed this token once (`ChatGPTBrowser` ->
/// `CodexBrowser`, observed 2026-08-28) and the fixed list silently stopped
/// matching, which is the failure this shape avoids repeating. Also
/// deliberately NOT keyed on the substrings `codex`/`chatgpt`: that is the
/// same roster problem one level down.
///
/// `None` for a UA that already starts with `Mozilla/` - a real browser, and
/// the direction that must never match, since mistaking the website for the
/// app would cost it the `BROWSER_ROUTED` narrowing. Also `None` for anything
/// with no `Mozilla/` token at all (`Codex Desktop/…`, `codex-mcp-client/…`):
/// those are app clients too, but they are matched by their own exact
/// prefixes and have no browser string hiding inside them to fall back to.
///
/// A third-party client that prepends its own token to a browser UA reads as
/// `App` here rather than `Unknown`. Both route identically, so that costs
/// nothing, and the direction that matters holds: the browser cannot be
/// mistaken for the app.
pub(crate) fn browser_ua_without_product_token(user_agent: &str) -> Option<&str> {
    if user_agent.starts_with(BROWSER_UA_PREFIX) {
        return None;
    }
    let (token, rest) = user_agent.split_once(' ')?;
    (!token.is_empty() && rest.starts_with(BROWSER_UA_PREFIX)).then_some(rest)
}

/// Classify a request from its headers.
///
/// `originator` is the primary signal: it is the vendor's own "which front-end
/// is this" field, it was present on EVERY app request to a routed path in the
/// captures, and on none of the web ones. The user-agent is the fallback,
/// because a build that drops `originator` but still names itself should keep
/// routing - and because that fallback is load-bearing in a way it was not
/// when it was written: the Cloudflare handling in `engine` is gated on
/// [`ClientClass::App`], so a missed app signal now silently disables it.
///
/// The fallback is matched by SHAPE, not by product name. The first version
/// listed `ChatGPTBrowser`; the vendor renamed the shell to `CodexBrowser` and
/// the check quietly stopped covering the app it was written for, leaving
/// `originator` as the single point of failure it exists to remove. Observed
/// 2026-08-28 in the wire log, where the same app produced both `App`
/// (`originator` present) and `Unknown` (absent) on one session, and only the
/// former got its Cloudflare cookie. See [`browser_ua_without_product_token`].
///
/// The web markers are what routing actually keys on, so they are matched
/// POSITIVELY: `Web` means "this is the website", never "no app signal found".
/// Anything unrecognised is [`ClientClass::Unknown`] and gets routed.
///
/// The app check runs FIRST and is load-bearing even though `App` and `Unknown`
/// route alike today. If a future app build starts sending one of the web
/// markers, that check is what stops it being mistaken for the browser and
/// losing its route.
///
/// A bare browser-shaped user-agent is deliberately NOT a web signal. Plenty of
/// agents copy a Chrome user-agent verbatim, so treating it as one would exclude
/// exactly the third-party clients this is meant to keep.
///
/// Both signals are trivially spoofable, which is fine: this decides whose
/// traffic Gate is allowed to *see*, not what it is allowed to do. Forging it
/// only affects the forger's own requests.
pub fn classify_client<'a>(header: impl Fn(&str) -> Option<&'a str>) -> ClientClass {
    // ── Anthropic states the client outright ────────────────────────────────
    // One header, two values, on the very endpoint the app and the browser
    // share. Gate already matches `desktop_app` for its `claude-desktop`
    // platform (gateway: utils/platform-registry.ts); `web_claude_ai` is the
    // value nothing had been told about, captured 2026-08-17 from claude.ai in
    // Chrome. Checked before the OpenAI signals only because it is decisive:
    // no inference, no prefix matching, the vendor simply says which it is.
    if let Some(platform) = header("anthropic-client-platform").map(str::trim) {
        if platform.eq_ignore_ascii_case(ANTHROPIC_WEB_PLATFORM) {
            return ClientClass::Web;
        }
        if platform.eq_ignore_ascii_case(ANTHROPIC_DESKTOP_PLATFORM) {
            return ClientClass::App;
        }
        // Any OTHER value falls through deliberately rather than being read as
        // App. Claude Code and future first-party clients may spell themselves
        // differently, and `Unknown` routes anyway - guessing App here would buy
        // nothing and could mislabel a client we have never seen.
    }
    // The app sends this too, and the browser never does. Kept as a second
    // app signal so a build that drops the platform header still classifies.
    if header("anthropic-client-app").is_some_and(|v| !v.trim().is_empty()) {
        return ClientClass::App;
    }

    if header("originator").is_some_and(|v| !v.trim().is_empty()) {
        return ClientClass::App;
    }
    let ua = header("user-agent").unwrap_or_default();
    // An app shell wraps an ordinary browser UA in its own product token; the
    // other two are Codex's native agent and its MCP client, neither of which
    // is browser-shaped. See `browser_ua_without_product_token` for why the
    // shell is matched by shape rather than by name.
    if browser_ua_without_product_token(ua).is_some()
        || ua.starts_with("Codex Desktop/")
        || ua.starts_with("codex-mcp-client/")
    {
        return ClientClass::App;
    }
    // Emitted by the website and never by the app in any capture.
    if [
        "oai-device-id",
        "oai-client-version",
        "x-openai-target-route",
    ]
    .iter()
    .any(|h| header(h).is_some())
    {
        return ClientClass::Web;
    }
    ClientClass::Unknown
}

/// Narrow what each entry CLAIMS for this client, before [`decide`] reads them.
///
/// A browser-excluding entry facing a [`ClientClass::Web`] request keeps its
/// hosts and loses its rewrite prefixes. It therefore still matches the host,
/// claims no path, and falls through to `decide`'s existing unclaimed-path
/// `Passthrough` - no new branch inside the routing decision, which stays a pure
/// function of the entries it is handed. Every other client class passes through
/// untouched.
///
/// Removing the entry outright is the obvious implementation and is wrong.
/// `decide` reports `Tunnel` when NO enabled entry names the host, meaning "not
/// a host we intercept" - and by the time this runs the CONNECT has already been
/// intercepted and the bytes decrypted, because `should_intercept_host` matches
/// on host alone and cannot see these headers. Reporting `Tunnel` there would
/// state something untrue about a connection we are already inside. Declining to
/// rewrite is what actually happened, and `Passthrough` is the word for it.
pub fn rules_for_client(domains: &[ProxyDomain], client: ClientClass) -> Vec<ProxyDomain> {
    domains
        .iter()
        .map(|d| {
            if client != ClientClass::Web {
                return d.clone();
            }
            let Some(scope) = BROWSER_ROUTED.iter().find(|s| s.slug == d.slug.as_str()) else {
                return d.clone();
            };
            // Prefixes are retained by exact match against the entry's OWN list,
            // so a scope that drifts out of the catalog narrows to nothing rather
            // than silently widening what a browser may route. Suffixes are
            // additive and need no such check - they only ever remove paths.
            ProxyDomain {
                rewrite_prefixes: d
                    .rewrite_prefixes
                    .iter()
                    .filter(|p| scope.prefixes.contains(&p.as_str()))
                    .cloned()
                    .collect(),
                rewrite_suffixes: scope.suffixes.iter().map(|s| (*s).to_string()).collect(),
                ..d.clone()
            }
        })
        .collect()
}

/// Decide what to do with a request given its host + path. Passthrough
/// prefixes win over rewrite prefixes; an intercepted host whose path no entry
/// claims is left alone (passthrough) rather than rewritten.
///
/// Prefixes are matched against the path *as Gate will receive it* - i.e. after
/// the domain's own [`upstream_path`] is removed, since that segment rides in
/// the upstream header instead. For a bare-host upstream the two are identical.
///
/// **Every enabled entry claiming the host gets a look, not just the first.**
/// Two entries can name one host with different URL splits - `chatgpt-apps`
/// carries the app's real paths off a bare host, `chatgpt` carries Codex's
/// Responses call with `/backend-api` in the upstream - and each deliberately
/// ignores the other's paths. Stopping at the first host match made the earlier
/// entry silently swallow the later one's traffic as an unclaimed passthrough,
/// so enabling both switches routed less than enabling one. Since both are now
/// rows the user can toggle independently (`provider::chat_domain_slugs`), that
/// combination has to behave. A host-matching entry that claims neither the path
/// nor its subtree simply abstains; only if nobody claims it does the request
/// fall through to `Passthrough`.
/// The Responses endpoint's final segment. Matched as a SUFFIX so this does not
/// hard-code one of the two URL splits that reach the endpoint (the app's real
/// `/backend-api/codex/responses` and Codex's relayed `/codex/responses`);
/// which of them is actually routed is `decide`'s business, not this
/// constant's.
const RESPONSES_PATH_SUFFIX: &str = "/responses";

/// True when an upgrade on this host+path should be DECLINED rather than passed
/// through, so the client falls back to HTTP and the turn becomes visible.
///
/// ChatGPT desktop work mode sends its entire turn as WebSocket frame 0 on the
/// Responses endpoint, so passing the upgrade through means Gate sees none of
/// it. The app carries an HTTP fallback for exactly this situation - its bundled
/// Codex binary logs "falling back to HTTP" and diagnoses
/// `network.websocket_reachability` with "Check proxy, VPN, firewall, DNS,
/// custom CA, and WebSocket policy support" - so declining uses a path the
/// vendor built for proxies rather than a trick. See
/// `docs/responses-websocket-downgrade.md`.
///
/// Two conditions, both required:
///
/// - [`decide`] would REWRITE this path. That is the point: declining only helps
///   where the fallback request would then be routed to Gate, and it also means
///   a disabled row - or a client narrowed away from the path by
///   [`rules_for_client`] - is never touched. Reusing `decide` rather than
///   re-deriving the match keeps the two from drifting apart.
/// - the path is a Responses endpoint. The fallback evidence is specific to that
///   endpoint, so an upgrade on any other routed path is passed through as
///   before rather than broken on the assumption it recovers the same way.
///
/// Deliberately NOT keyed on `openai-beta: responses_websockets=<date>`. The
/// same binary also carries `responses_websockets_v2`, so the negotiation string
/// is still moving, and a matcher pinned to it would silently stop firing on a
/// version bump - a failure that reads as "work mode went dark again" rather
/// than as a stale constant.
pub(crate) fn should_decline_upgrade(domains: &[ProxyDomain], host: &str, path: &str) -> bool {
    path.ends_with(RESPONSES_PATH_SUFFIX)
        && matches!(decide(domains, host, path), Decision::Rewrite { .. })
}

pub(crate) fn decide(domains: &[ProxyDomain], host: &str, path: &str) -> Decision {
    let mut host_matched = false;
    for d in domains.iter().filter(|d| d.enabled) {
        if !d.matches_host(host) {
            continue;
        }
        host_matched = true;
        // A path outside the upstream's own subtree is not this domain's
        // traffic at all; abstain rather than forwarding a path Gate would
        // reassemble into a URL the provider never served.
        let Some(path) = strip_upstream_path(path, upstream_path(&d.upstream_url)) else {
            continue;
        };
        if d.passthrough_prefixes
            .iter()
            .any(|p| path.starts_with(p.as_str()))
        {
            return Decision::Passthrough;
        }
        let prefix_hit = d
            .rewrite_prefixes
            .iter()
            .any(|p| path.starts_with(p.as_str()));
        // An empty suffix list means "no extra constraint"; a non-empty one must
        // also match, and a miss ABSTAINS rather than deciding, so a later entry
        // still gets its look and an unclaimed path falls through to Passthrough.
        let suffix_ok = d.rewrite_suffixes.is_empty()
            || d.rewrite_suffixes
                .iter()
                .any(|sfx| path.ends_with(sfx.as_str()));
        if prefix_hit && suffix_ok {
            return Decision::Rewrite {
                upstream_url: d.upstream_url.clone(),
                slug: d.slug.clone(),
            };
        }
    }
    // Intercepted (some enabled entry owns the host) but claimed by no entry's
    // prefixes: forward to the real upstream untouched, which is what an
    // unrecognised path on a routed host has always done.
    if host_matched {
        return Decision::Passthrough;
    }
    Decision::Tunnel
}

// The built-in domain catalog lives in `catalog.rs` (one entry per
// provider, with the per-entry rationale); re-exported here so callers keep
// addressing it as `proxy::default_domains`.
mod catalog;
pub use catalog::default_domains;

/// A provider endpoint split the way the relay and the gateway need it.
pub struct ResolvedEndpoint {
    /// Catalog slug that owns the endpoint.
    pub slug: String,
    /// What `X-Gate-Upstream-Url` must carry. The gateway concatenates the
    /// forwarded request path onto this, and the relay only forwards to
    /// upstreams it can find in the catalog.
    pub upstream_url: String,
    /// The path that has to live in the *tool's* base URL, so that the path the
    /// relay forwards is relative to `upstream_url`. Empty when the endpoint is
    /// the catalog upstream itself.
    pub client_path: String,
}

impl ResolvedEndpoint {
    /// The base URL a tool config points at to route this endpoint through the
    /// relay: `<relay>/<slug><client_path>`.
    ///
    /// The slug segment is how the relay knows which upstream a request belongs
    /// to, so it can inject `x-gate-upstream-url` itself instead of the tool
    /// carrying it in a config file. It is stripped back off before anything is
    /// forwarded, leaving exactly `client_path` + whatever the tool appended.
    pub fn relay_base_url(&self, relay_base_url: &str) -> String {
        format!(
            "{}/{}{}",
            relay_base_url.trim_end_matches('/'),
            self.slug,
            self.client_path
        )
    }
}

/// Resolve a provider's canonical endpoint - the URL a tool would call if Gate
/// were not in the picture, e.g. `https://openrouter.ai/api/v1` - into the
/// catalog upstream plus the path the tool must keep on its own side.
///
/// `None` means no catalog entry covers the endpoint. The relay refuses to
/// forward such an upstream, so a caller must leave that provider's config
/// alone rather than repointing it at a relay that will 403 every request.
///
/// This is deliberately the *only* place the split is decided. Doing it by hand,
/// once per integration, is what first broke OpenRouter: `https://openrouter.ai/api`
/// read like a sensible upstream but matched no catalog entry (which was then
/// `https://openrouter.ai`), so every request 403'd. The catalog entry now
/// carries the `/api` itself, which is what makes that split the correct one -
/// see the entry's comment for why the forwarded path must not begin `/api/`.
pub fn resolve_endpoint(endpoint: &str) -> Option<ResolvedEndpoint> {
    let endpoint = endpoint.trim_end_matches('/');
    default_domains()
        .into_iter()
        .filter_map(|d| {
            let rest = endpoint.strip_prefix(d.upstream_url.as_str())?;
            // Only match on a path boundary, so `https://api.openai.com.evil.test`
            // can never resolve to the `api.openai.com` entry.
            if !rest.is_empty() && !rest.starts_with('/') {
                return None;
            }
            let client_path = rest.to_string();
            Some((d, client_path))
        })
        // Longest upstream wins, so an entry carrying a path
        // (chatgpt.com/backend-api) beats a bare-host entry for the same host.
        .max_by_key(|(d, _)| d.upstream_url.len())
        .map(|(d, client_path)| ResolvedEndpoint {
            slug: d.slug,
            upstream_url: d.upstream_url,
            client_path,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The loopback boundary the relay and the PAC responder share: loopback
    /// authorities in any spelling pass, everything a rebound or cross-site
    /// browser request can carry is refused.
    #[test]
    fn loopback_authority_boundary() {
        for ok in [
            "127.0.0.1:47123",
            "127.0.0.1",
            "localhost:47123",
            "LOCALHOST",
            "[::1]:47123",
            "[::1]",
        ] {
            assert!(authority_is_loopback(ok), "{ok} should pass");
        }
        for bad in [
            "attacker.example",
            "attacker.example:47123",
            "127.0.0.1.attacker.example",
            "gate.constellationnetwork.io",
            "",
        ] {
            assert!(!authority_is_loopback(bad), "{bad} should be refused");
        }
        for ok in ["http://127.0.0.1:5173", "http://localhost:5173"] {
            assert!(origin_is_loopback(ok), "{ok} should pass");
        }
        for bad in [
            "https://attacker.example",
            "null",
            "file://",
            "http://127.0.0.1.attacker.example",
        ] {
            assert!(!origin_is_loopback(bad), "{bad} should be refused");
        }
    }

    fn anthropic() -> Vec<ProxyDomain> {
        vec![default_domains().into_iter().next().unwrap()]
    }

    /// The `claude-web` entry, force-enabled: it ships opt-out because the
    /// surface carries a session cookie rather than an API key.
    fn claude_web() -> Vec<ProxyDomain> {
        let mut d: ProxyDomain = default_domains()
            .into_iter()
            .find(|d| d.slug == "claude-web")
            .expect("claude-web is in the catalog");
        d.enabled = true;
        vec![d]
    }

    /// The `chatgpt-apps` MITM entry (chat + Codex tool plane), force-enabled.
    fn chatgpt_apps() -> Vec<ProxyDomain> {
        let mut d: ProxyDomain = default_domains()
            .into_iter()
            .find(|d| d.slug == "chatgpt-apps")
            .expect("chatgpt-apps is in the catalog");
        d.enabled = true;
        vec![d]
    }

    const CLAUDE_COMPLETION: &str =
        "/api/organizations/b44129f9-a8ea-4f96-a137-b14a560e58d3/chat_conversations/2f261f16-2b31-41f8-b441-6067464c6504/completion";

    #[test]
    fn resolves_endpoints_against_the_catalog() {
        // Bare-host upstream: the whole path stays on the client.
        let r = resolve_endpoint("https://api.anthropic.com/v1").expect("anthropic resolves");
        assert_eq!(r.slug, "anthropic");
        assert_eq!(r.upstream_url, "https://api.anthropic.com");
        assert_eq!(r.client_path, "/v1");

        // OpenRouter's real API lives under /api/v1, and the `/api` belongs in
        // the *upstream*, not the client path: Gate's ALB diverts `/api/*` to
        // the dashboard API, so a forwarded `/api/v1/...` never reaches the
        // gateway proxy. Gate re-joins upstream + path, so the provider still
        // sees /api/v1/chat/completions.
        let r = resolve_endpoint("https://openrouter.ai/api/v1").expect("openrouter resolves");
        assert_eq!(r.slug, "openrouter");
        assert_eq!(r.upstream_url, "https://openrouter.ai/api");
        assert_eq!(r.client_path, "/v1");

        // A catalog upstream that itself carries a path wins over a bare host.
        let r =
            resolve_endpoint("https://chatgpt.com/backend-api/codex").expect("chatgpt resolves");
        assert_eq!(r.slug, "chatgpt");
        assert_eq!(r.upstream_url, "https://chatgpt.com/backend-api");
        assert_eq!(r.client_path, "/codex");

        // Trailing slash and the bare upstream itself.
        let r = resolve_endpoint("https://api.openai.com/").expect("openai resolves");
        assert_eq!(r.client_path, "");

        // Zen and Go share one catalog entry, separated by client path. Longest
        // match is not what distinguishes them - the same upstream serves both -
        // so the path each tool keeps is what routes it.
        let zen = resolve_endpoint("https://opencode.ai/zen/v1").expect("zen resolves");
        assert_eq!(zen.slug, "opencode");
        assert_eq!(zen.upstream_url, "https://opencode.ai");
        assert_eq!(zen.client_path, "/zen/v1");
        let go = resolve_endpoint("https://opencode.ai/zen/go/v1").expect("zen go resolves");
        assert_eq!(go.slug, "opencode");
        assert_eq!(go.client_path, "/zen/go/v1");

        // Off-catalog upstreams do not resolve, so callers leave them alone.
        assert!(resolve_endpoint("https://attacker.example/v1").is_none());
        // Suffix-confusion must not resolve to the api.openai.com entry.
        assert!(resolve_endpoint("https://api.openai.com.evil.test/v1").is_none());
    }

    #[test]
    fn every_resolved_endpoint_lands_on_an_inference_prefix() {
        // The invariant that ties the two halves together: for each catalog
        // entry, the path a tool ends up sending (client_path + the tool's own
        // suffix) must match one of that entry's `rewrite_prefixes`, or the
        // request silently passes through to the user's own account instead of
        // routing through Gate. Checked here for the canonical endpoint of each
        // domain; each integration's own
        // `known_provider_endpoints_all_resolve_against_the_catalog` checks it
        // for the endpoints that integration actually writes.
        for d in default_domains() {
            if d.rewrite_prefixes.is_empty() {
                continue;
            }
            // A prefix ending in `/` is a directory prefix, so give it a leaf to
            // stand in for the tool's own suffix; otherwise the prefix is
            // already a full endpoint path.
            let prefix = &d.rewrite_prefixes[0];
            let path = if prefix.ends_with('/') {
                format!("{prefix}probe")
            } else {
                prefix.clone()
            };
            // A MITM-only entry can be SHADOWED on the relay route: `resolve_endpoint`
            // breaks ties by longest upstream, so `chatgpt-apps` (bare chatgpt.com)
            // always loses its `/backend-api/...` paths to the relay `chatgpt` entry
            // (chatgpt.com/backend-api). That is by design - the two are matched by
            // different mechanisms, `decide` on HOST and `relay::route` on
            // `upstream_url` - and no integration ever hands such an endpoint to
            // `resolve_endpoint`, so the relay invariant does not apply. The
            // equivalent tie for a MITM entry is that `decide` rewrites the path, so
            // assert THAT rather than leaving a hole.
            let shadowed = default_domains().iter().any(|o| {
                o.slug != d.slug
                    && o.upstream_url
                        .starts_with(&format!("{}/", d.upstream_url.trim_end_matches('/')))
            });
            if shadowed {
                let mut mitm = d.clone();
                mitm.enabled = true;
                let request_path = format!("{}{path}", upstream_path(&d.upstream_url));
                assert_eq!(
                    decide(std::slice::from_ref(&mitm), &d.hosts[0], &request_path),
                    Decision::Rewrite {
                        upstream_url: d.upstream_url.clone(),
                        slug: d.slug.clone()
                    },
                    "{}: shadowed on the relay route, so `decide` must carry {request_path}",
                    d.slug
                );
                continue;
            }
            let endpoint = format!("{}{}", d.upstream_url, path);
            let r = resolve_endpoint(&endpoint)
                .unwrap_or_else(|| panic!("{} endpoint {endpoint} must resolve", d.slug));
            assert_eq!(r.slug, d.slug);
            assert!(
                d.rewrite_prefixes
                    .iter()
                    .any(|p| r.client_path.starts_with(p.as_str())),
                "{}: client_path {:?} matches no rewrite prefix {:?}",
                d.slug,
                r.client_path,
                d.rewrite_prefixes
            );
        }
    }

    #[test]
    fn forwarded_paths_avoid_gate_reserved_prefixes() {
        // Gate's ALB routes by path prefix: `/api/*`, `/orgs/*`, `/admin/*`,
        // `/me/*` and `/agent-templates/*` go to the dashboard API, everything
        // else to the gateway proxy (gate: terraform/aws/compute.tf, the
        // `path_patterns` on the dashboard-api listener rule). A forwarded path
        // that starts with one of those never reaches the proxy at all - it
        // 404s out of a service with no such route, which is exactly how the
        // OpenRouter integration failed end-to-end while every catalog
        // self-consistency check stayed green.
        //
        // This list mirrors infrastructure in another repo, so it is a snapshot:
        // if Gate adds a listener rule, this test will not know. It still pins
        // the ones we have measured.
        const RESERVED: &[&str] = &["/api/", "/orgs/", "/admin/", "/me/", "/agent-templates/"];

        for d in default_domains() {
            for prefix in &d.rewrite_prefixes {
                for reserved in RESERVED {
                    assert!(
                        !prefix.starts_with(reserved),
                        "{}: rewrite prefix {:?} lands on Gate's reserved {:?} - the request \
                         would be routed to the dashboard API instead of the gateway proxy. \
                         Move that segment into `upstream_url` so it rides in \
                         X-Gate-Upstream-Url instead of the request line.",
                        d.slug,
                        prefix,
                        reserved
                    );
                }
            }
        }
    }

    #[test]
    fn upstream_path_extracts_the_path_component() {
        assert_eq!(upstream_path("https://openrouter.ai/api"), "/api");
        assert_eq!(
            upstream_path("https://chatgpt.com/backend-api"),
            "/backend-api"
        );
        assert_eq!(upstream_path("https://api.anthropic.com"), "");
        // A trailing slash is not a path segment.
        assert_eq!(upstream_path("https://openrouter.ai/"), "");
    }

    #[test]
    fn strip_upstream_path_respects_path_boundaries() {
        assert_eq!(
            strip_upstream_path("/api/v1/chat", "/api"),
            Some("/v1/chat")
        );
        // The upstream root itself normalises to "/".
        assert_eq!(strip_upstream_path("/api", "/api"), Some("/"));
        // Not a boundary: must not match.
        assert_eq!(strip_upstream_path("/apifoo", "/api"), None);
        assert_eq!(strip_upstream_path("/v1/chat", "/api"), None);
        // A bare-host upstream passes the path through untouched.
        assert_eq!(
            strip_upstream_path("/v1/messages", ""),
            Some("/v1/messages")
        );
    }

    #[test]
    fn intercepts_only_enabled_matching_hosts() {
        let d = anthropic();
        assert!(should_intercept_host(&d, "api.anthropic.com"));
        assert!(should_intercept_host(&d, "API.ANTHROPIC.COM")); // case-insensitive
        assert!(!should_intercept_host(&d, "example.com"));
        assert!(!should_intercept_host(&d, "statsig.anthropic.com"));
    }

    #[test]
    fn disabled_domain_is_not_intercepted() {
        let mut d = anthropic();
        d[0].enabled = false;
        assert!(!should_intercept_host(&d, "api.anthropic.com"));
        assert_eq!(
            decide(&d, "api.anthropic.com", "/v1/messages"),
            Decision::Tunnel
        );
    }

    #[test]
    fn the_entry_that_would_claim_a_host_is_found_while_it_is_off() {
        // What an integration needs to *report* on its tool's upstream: which
        // switch covers this host, given it is not on? Every catalog entry but
        // `anthropic` ships disabled, so `should_intercept_host` cannot say.
        let all = default_domains();
        let d = domain_claiming_host(&all, "openrouter.ai").expect("openrouter.ai is claimed");
        assert_eq!(d.slug, "openrouter");
        assert!(!d.enabled, "and the caller can see it is off");

        assert_eq!(
            domain_claiming_host(&all, "OPENROUTER.AI").map(|d| d.slug.as_str()),
            Some("openrouter"),
            "host matching is case-insensitive"
        );
        assert!(domain_claiming_host(&all, "api.together.xyz").is_none());
        assert!(
            domain_claiming_host(&all, "api.openai.com.evil.test").is_none(),
            "a suffix of a claimed host is not that host"
        );

        // Two entries name chatgpt.com. Catalog order has to answer with the
        // MITM one, because that is the order `decide` resolves in.
        assert_eq!(
            domain_claiming_host(&all, "chatgpt.com").map(|d| d.slug.as_str()),
            Some("chatgpt-apps")
        );
    }

    /// Header lookup over a fixed list, in the shape `classify_client` wants.
    fn hdrs<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<&'a str> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| *v)
        }
    }

    const WEB_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                          (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36";

    #[test]
    fn anthropic_names_the_client_in_one_header_with_two_values() {
        // Unlike OpenAI's presence/absence split, Anthropic sends ONE header and
        // spells out which client it is. Captured from claude.ai in Chrome
        // 2026-08-17; the desktop value is the one Gate already matches for its
        // `claude-desktop` platform.
        assert_eq!(
            classify_client(hdrs(&[
                ("anthropic-client-platform", "web_claude_ai"),
                ("user-agent", WEB_UA)
            ])),
            ClientClass::Web
        );
        assert_eq!(
            classify_client(hdrs(&[("anthropic-client-platform", "desktop_app")])),
            ClientClass::App
        );
        // Case-insensitive, like every other header value read here.
        assert_eq!(
            classify_client(hdrs(&[("Anthropic-Client-Platform", "WEB_CLAUDE_AI")])),
            ClientClass::Web
        );
    }

    #[test]
    fn an_unrecognised_anthropic_platform_is_not_assumed_to_be_the_app() {
        // Claude Code and future first-party clients may spell themselves
        // differently. Unknown routes anyway, so guessing App buys nothing and
        // could mislabel a client nobody has seen.
        assert_eq!(
            classify_client(hdrs(&[("anthropic-client-platform", "cli")])),
            ClientClass::Unknown
        );
        // The app's other header still classifies on its own, so a build that
        // drops the platform header keeps its route.
        assert_eq!(
            classify_client(hdrs(&[(
                "anthropic-client-app",
                "com.anthropic.claudefordesktop"
            )])),
            ClientClass::App
        );
    }

    #[test]
    fn the_browser_gets_only_claude_ais_completion_call() {
        // `claude-web`'s only prefix is `/organizations/`, and a claude.ai capture
        // found 27 distinct endpoints under it of which exactly ONE is the chat
        // turn. The conversation id sits BEFORE the distinguishing final segment,
        // so no prefix can separate them - hence the suffix requirement.
        //
        // The browser gets its conversation captured, which is the point of the
        // row. It does not get memory, projects, skills, plugins, MCP servers or
        // the vendor's own title generator.
        let d = claude_web();
        let org = "/api/organizations/b44129f9-a8ea-4f96-a137-b14a560e58d3";
        let browser = rules_for_client(&d, ClientClass::Web);

        assert_eq!(
            decide(&browser, "claude.ai", CLAUDE_COMPLETION),
            Decision::Rewrite {
                upstream_url: "https://claude.ai/api".into(),
                slug: "claude-web".into()
            },
            "the browser's chat turn IS captured"
        );

        for path in [
            format!("{org}/memory"),
            format!("{org}/skills/list-skills"),
            format!("{org}/mcp/remote_servers"),
            format!("{org}/projects"),
            // The vendor's title generator, excluded by the same rule.
            format!("{org}/chat_conversations/2f261f16-2b31-41f8-b441-6067464c6504/title"),
        ] {
            assert_eq!(
                decide(&browser, "claude.ai", &path),
                Decision::Passthrough,
                "browser must not send {path} to Gate"
            );
        }

        // The desktop app keeps the whole tree: it is a first-party client on a
        // host the user deliberately routed.
        let app = rules_for_client(&d, ClientClass::App);
        for path in [CLAUDE_COMPLETION, &format!("{org}/memory")] {
            assert_eq!(
                decide(&app, "claude.ai", path),
                Decision::Rewrite {
                    upstream_url: "https://claude.ai/api".into(),
                    slug: "claude-web".into()
                },
                "the app keeps {path}"
            );
        }
    }
    #[test]
    fn the_originator_header_identifies_the_app() {
        // Present on every app request to a routed path in the captures, and on
        // none of the web ones.
        assert_eq!(
            classify_client(hdrs(&[
                ("originator", "Codex Desktop"),
                ("user-agent", WEB_UA)
            ])),
            ClientClass::App,
            "originator outranks a browser-shaped user-agent"
        );
        assert_eq!(
            classify_client(hdrs(&[("originator", "   ")])),
            ClientClass::Unknown,
            "a blank originator is not a claim"
        );
    }

    #[test]
    fn the_user_agent_prefix_is_the_fallback_signal() {
        // A build that drops `originator` but still names itself should keep
        // routing, so each app UA family is matched on its own.
        for ua in [
            // The shipping Windows app, captured verbatim 2026-08-28. The shell
            // was renamed `ChatGPTBrowser` -> `CodexBrowser` and the roster of
            // prefixes stopped covering it, with nothing here to say so; this
            // string is the assertion whose absence let that through.
            "CodexBrowser Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36",
            // The older shipped build. The rename is additive; it must not regress.
            "ChatGPTBrowser Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/151.0.0.0",
            "Codex Desktop/0.148.0-alpha.9 (Windows 10.0.26200; x86_64)",
            "Codex Desktop/26.825.32147 (Windows NT 10.0; x64)",
            "codex-mcp-client/0.148.0-alpha.9",
        ] {
            assert_eq!(
                classify_client(hdrs(&[("user-agent", ua)])),
                ClientClass::App,
                "{ua}"
            );
        }
        // The one direction the shape rule must never allow: a browser's own UA
        // opens with `Mozilla/`, so no amount of what follows makes it the app.
        // Getting this wrong would cost the browser its narrowing.
        for ua in [
            WEB_UA,
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) CodexBrowser/1.0",
        ] {
            assert_ne!(
                classify_client(hdrs(&[("user-agent", ua)])),
                ClientClass::App,
                "{ua}"
            );
        }
    }

    #[test]
    fn only_the_browser_shaped_shell_user_agent_is_offered_to_the_solve_webview() {
        // The app emits both, and the webview must impersonate the shell: a
        // window claiming to be a CLI agent is the client shape Cloudflare is
        // least likely to hand a cf_clearance to. Both strings captured
        // 2026-08-28 from one session, where the agent's UA won the race and
        // the solve window opened wearing it.
        let shell = "CodexBrowser Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                     (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36";
        record_chatgpt_app_user_agent(shell);
        record_chatgpt_app_user_agent(
            "Codex Desktop/0.150.0-alpha.12.2 (Windows 10.0.26200; x86_64) unknown",
        );
        record_chatgpt_app_user_agent("");
        assert_eq!(chatgpt_app_user_agent().as_deref(), Some(shell));
    }

    #[test]
    fn stripping_the_product_token_leaves_the_website_user_agent() {
        // Both captured 2026-08-28 from one machine, one session: the app's
        // rewritten chat turn was challenged and the website's was answered
        // 200. The app's UA is the website's with a token prefixed, which is
        // what makes the shape match sound AND what the strip forwards -
        // assert it, so a build where that stops holding fails here.
        let app = "CodexBrowser Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                   (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36";
        let website = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                       (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36";
        assert_eq!(browser_ua_without_product_token(app), Some(website));
        // A real browser has no token to drop - and must never read as an app,
        // or the website would lose its BROWSER_ROUTED narrowing.
        assert_eq!(browser_ua_without_product_token(website), None);
        assert_eq!(
            classify_client(hdrs(&[("user-agent", website)])),
            ClientClass::Unknown
        );
        // Neither do the non-browser-shaped agents: there is no browser string
        // hiding inside them, so leaving them alone is the only option.
        assert_eq!(
            browser_ua_without_product_token("Codex Desktop/0.148.0-alpha.9 (Windows 10.0.26200)"),
            None
        );
        assert_eq!(
            browser_ua_without_product_token("codex-mcp-client/0.148.0-alpha.9"),
            None
        );
        assert_eq!(browser_ua_without_product_token(""), None);
    }

    #[test]
    fn the_website_is_identified_positively_not_by_absence() {
        // "Definitely the browser" is its own finding; otherwise every unknown
        // client would read as web and silently lose routing it should keep.
        assert_eq!(
            classify_client(hdrs(&[("user-agent", WEB_UA), ("oai-device-id", "abc")])),
            ClientClass::Web
        );
        assert_eq!(
            classify_client(hdrs(&[("user-agent", WEB_UA)])),
            ClientClass::Unknown,
            "a bare browser UA on its own is not enough to call it the website"
        );
        assert_eq!(classify_client(hdrs(&[])), ClientClass::Unknown);
    }

    #[test]
    fn the_browser_routes_its_chat_turn_but_not_the_app_plumbing() {
        // The app and the website POST the same endpoint on the same host, and
        // BOTH are routed: capturing the browser's conversation is the point of
        // the row. What the browser does not get is the rest of the entry -
        // `/wham/*` is task and settings plumbing, not a conversation, and the
        // browser hits it too.
        const TURN: &str = "/backend-api/f/conversation";
        const PLUMBING: &str = "/backend-api/wham/usage";
        let all: Vec<ProxyDomain> = default_domains()
            .into_iter()
            .map(|mut d| {
                d.enabled = d.slug == "chatgpt-apps";
                d
            })
            .collect();

        for class in [ClientClass::App, ClientClass::Web, ClientClass::Unknown] {
            assert_eq!(
                decide(&rules_for_client(&all, class), "chatgpt.com", TURN),
                Decision::Rewrite {
                    upstream_url: "https://chatgpt.com".into(),
                    slug: "chatgpt-apps".into()
                },
                "{class:?} must have its chat turn captured"
            );
        }

        assert_eq!(
            decide(
                &rules_for_client(&all, ClientClass::Web),
                "chatgpt.com",
                PLUMBING
            ),
            Decision::Passthrough,
            "the browser's task plumbing is not ours"
        );
        assert_eq!(
            decide(
                &rules_for_client(&all, ClientClass::App),
                "chatgpt.com",
                PLUMBING
            ),
            Decision::Rewrite {
                upstream_url: "https://chatgpt.com".into(),
                slug: "chatgpt-apps".into()
            },
            "the app keeps it"
        );
    }

    #[test]
    fn an_unidentified_client_still_routes_every_entry() {
        // OpenClaw reaches the `chatgpt` entry through the MITM engine and sends
        // neither an originator nor an app user-agent, so it classifies as
        // Unknown. It must keep its route on every entry, browser-excluding or
        // not, or this change silently breaks subscription-mode OpenClaw.
        let mut relay: Vec<ProxyDomain> = default_domains()
            .into_iter()
            .filter(|d| d.slug == "chatgpt")
            .collect();
        relay[0].enabled = true;
        assert_eq!(
            decide(
                &rules_for_client(&relay, ClientClass::Unknown),
                "chatgpt.com",
                "/backend-api/codex/responses"
            ),
            Decision::Rewrite {
                upstream_url: "https://chatgpt.com/backend-api".into(),
                slug: "chatgpt".into()
            }
        );
        // And an entry that is not browser-excluding keeps every prefix even for
        // the website, so this cannot quietly become a global browser ban.
        let anth = rules_for_client(&anthropic(), ClientClass::Web);
        assert_eq!(anth.len(), anthropic().len());
        assert_eq!(anth[0].rewrite_prefixes, anthropic()[0].rewrite_prefixes);
    }

    #[test]
    fn rewrites_inference_path() {
        let d = anthropic();
        assert_eq!(
            decide(&d, "api.anthropic.com", "/v1/messages?beta=true"),
            Decision::Rewrite {
                upstream_url: "https://api.anthropic.com".into(),
                slug: "anthropic".into()
            }
        );
    }

    #[test]
    fn passes_through_updater_path_on_matched_host() {
        let d = anthropic();
        // /api/desktop/* is a passthrough prefix → must not be rewritten.
        assert_eq!(
            decide(&d, "api.anthropic.com", "/api/desktop/RELEASES"),
            Decision::Passthrough
        );
    }

    #[test]
    fn passes_through_unknown_path_on_matched_host() {
        let d = anthropic();
        assert_eq!(
            decide(&d, "api.anthropic.com", "/healthz"),
            Decision::Passthrough
        );
    }

    /// Regression for AG (Claude Desktop 503s): OAuth/account calls live under
    /// /v1/ on the same intercepted host but carry no model, so they must reach
    /// real api.anthropic.com untouched - never be rewritten to the gateway
    /// (which rejects them 503 "AI unknown"). Guards against re-widening
    /// `rewrite_prefixes` back to "/v1/".
    #[test]
    fn passes_through_oauth_and_account_paths() {
        let d = anthropic();
        for path in [
            "/v1/oauth/token",
            "/v1/organizations",
            "/v1/organizations/me",
            "/v1/models",
        ] {
            assert_eq!(
                decide(&d, "api.anthropic.com", path),
                Decision::Passthrough,
                "non-inference path {path} must pass through, not rewrite"
            );
        }
    }

    #[test]
    fn rewrites_legacy_complete_and_count_tokens() {
        let d = anthropic();
        // Legacy text-completions endpoint.
        assert_eq!(
            decide(&d, "api.anthropic.com", "/v1/complete"),
            Decision::Rewrite {
                upstream_url: "https://api.anthropic.com".into(),
                slug: "anthropic".into()
            }
        );
        // count_tokens rides under /v1/messages, so the prefix still catches it.
        assert_eq!(
            decide(&d, "api.anthropic.com", "/v1/messages/count_tokens"),
            Decision::Rewrite {
                upstream_url: "https://api.anthropic.com".into(),
                slug: "anthropic".into()
            }
        );
    }

    /// Anthropic's OpenAI-compatible endpoint is inference like the other two
    /// and must reach the gateway. openclaw 2026.8.1 sends its anthropic
    /// traffic here rather than to /v1/messages; while the path was missing
    /// from `rewrite_prefixes` it passed through to the real API with the
    /// switch on, which is a silent bypass rather than a visible failure.
    #[test]
    fn rewrites_openai_compatible_completions() {
        let d = anthropic();
        assert_eq!(
            decide(&d, "api.anthropic.com", "/v1/chat/completions"),
            Decision::Rewrite {
                upstream_url: "https://api.anthropic.com".into(),
                slug: "anthropic".into()
            }
        );
    }

    #[test]
    fn ignores_unmatched_host() {
        let d = anthropic();
        assert_eq!(decide(&d, "example.com", "/v1/messages"), Decision::Tunnel);
    }

    /// The catalog's `openai` entry must be a supported, routable upstream so
    /// the proxy can intercept Codex (API-key mode) and other OpenAI clients.
    fn openai() -> Vec<ProxyDomain> {
        let mut d = default_domains()
            .into_iter()
            .find(|d| d.slug == "openai")
            .expect("openai domain present in catalog");
        d.enabled = true; // catalog default is opt-in; enable for the test
        vec![d]
    }

    #[test]
    fn openai_is_supported() {
        let d = default_domains()
            .into_iter()
            .find(|d| d.slug == "openai")
            .unwrap();
        assert!(d.supported, "openai must be a supported upstream");
    }

    #[test]
    fn openrouter_is_supported() {
        let d = default_domains()
            .into_iter()
            .find(|d| d.slug == "openrouter")
            .unwrap();
        assert!(d.supported, "openrouter must be a supported upstream");
    }

    #[test]
    fn rewrites_openrouter_api_path() {
        let mut d = default_domains()
            .into_iter()
            .find(|d| d.slug == "openrouter")
            .expect("openrouter domain present in catalog");
        d.enabled = true; // catalog default is opt-in; enable for the test
        let d = vec![d];
        // OpenRouter's chat/completions lives at openrouter.ai/api/v1/*. The
        // client still calls that, but `decide` matches on the path Gate will
        // see - `/v1/...` - because the `/api` travels in the upstream URL to
        // clear Gate's ALB rule on `/api/*`.
        assert_eq!(
            decide(&d, "openrouter.ai", "/api/v1/chat/completions"),
            Decision::Rewrite {
                upstream_url: "https://openrouter.ai/api".into(),
                slug: "openrouter".into()
            }
        );
        // Outside the upstream's subtree: not this domain's traffic.
        assert_eq!(
            decide(&d, "openrouter.ai", "/v1/chat/completions"),
            Decision::Passthrough
        );
        // Path-boundary guard - `/apifoo` must not read as `/api` + `foo`.
        assert_eq!(
            decide(&d, "openrouter.ai", "/apifoo/v1/chat"),
            Decision::Passthrough
        );
        assert!(should_intercept_host(&d, "OPENROUTER.AI"));
    }

    #[test]
    fn rewrites_codex_apikey_responses_path() {
        let d = openai();
        // Codex in API-key mode (and the OpenAI SDK) hit
        // api.openai.com/v1/responses, which must rewrite to the gateway
        // with the OpenAI upstream injected.
        assert_eq!(
            decide(&d, "api.openai.com", "/v1/responses"),
            Decision::Rewrite {
                upstream_url: "https://api.openai.com".into(),
                slug: "openai".into()
            }
        );
        // case-insensitive host match
        assert!(should_intercept_host(&d, "API.OPENAI.COM"));
    }

    /// OpenAI mirror of `passes_through_oauth_and_account_paths`: non-inference
    /// /v1/ calls must pass through to real api.openai.com, not be rewritten to
    /// the gateway (which 503s on a modelless request). Chat + legacy
    /// completions still rewrite.
    #[test]
    fn openai_passes_through_non_inference_and_rewrites_chat() {
        let d = openai();
        for path in ["/v1/models", "/v1/files", "/v1/assistants"] {
            assert_eq!(
                decide(&d, "api.openai.com", path),
                Decision::Passthrough,
                "non-inference path {path} must pass through, not rewrite"
            );
        }
        for path in ["/v1/chat/completions", "/v1/completions", "/v1/embeddings"] {
            assert_eq!(
                decide(&d, "api.openai.com", path),
                Decision::Rewrite {
                    upstream_url: "https://api.openai.com".into(),
                    slug: "openai".into()
                },
                "inference path {path} must rewrite to the gateway"
            );
        }
    }

    #[test]
    fn openai_domain_does_not_match_chatgpt_host() {
        // The api.openai.com domain is scoped to that host only - it must not
        // match chatgpt.com. (Codex's chatgpt.com traffic comes from its Rust
        // agent, which bypasses the system proxy, so it's out of the proxy's
        // reach entirely - covered by the manual Codex integration instead.)
        let d = openai();
        assert!(!should_intercept_host(&d, "chatgpt.com"));
        assert_eq!(
            decide(&d, "chatgpt.com", "/backend-api/codex/responses"),
            Decision::Tunnel
        );
    }
    #[test]
    fn claude_web_ships_disabled_so_a_session_surface_is_never_routed_silently() {
        let catalog = default_domains();
        let d = catalog.iter().find(|d| d.slug == "claude-web").unwrap();
        assert!(!d.enabled, "claude-web must be opt-in");
        assert!(d.supported);
        assert_eq!(d.hosts, vec!["claude.ai".to_string()]);
        // `/api` rides upstream-side so the forwarded path clears Gate's ALB
        // rule - see `forwarded_paths_avoid_gate_reserved_prefixes`.
        assert_eq!(d.upstream_url, "https://claude.ai/api");
    }

    #[test]
    fn claude_web_rewrites_the_chat_completion_call() {
        let d = claude_web();
        assert_eq!(
            decide(&d, "claude.ai", CLAUDE_COMPLETION),
            Decision::Rewrite {
                upstream_url: "https://claude.ai/api".into(),
                slug: "claude-web".into()
            }
        );
        // Query strings must not change the verdict.
        assert_eq!(
            decide(
                &d,
                "claude.ai",
                &format!("{CLAUDE_COMPLETION}?rendering_mode=messages")
            ),
            Decision::Rewrite {
                upstream_url: "https://claude.ai/api".into(),
                slug: "claude-web".into()
            }
        );
    }

    #[test]
    fn claude_web_leaves_updater_telemetry_and_bootstrap_alone() {
        let d = claude_web();
        for path in [
            "/api/desktop/RELEASES",
            "/api/event_logging/v2/batch",
            "/api/bootstrap/b44129f9/current_user_access",
        ] {
            assert_eq!(
                decide(&d, "claude.ai", path),
                Decision::Passthrough,
                "{path} must reach the real host untouched"
            );
        }
    }

    #[test]
    fn claude_web_routes_sibling_org_calls_but_gate_classifies_them_separately() {
        // Prefix matching cannot isolate the completion path (the conversation id
        // precedes the distinguishing final segment), so these ride along. That
        // is deliberate: Gate tags only the completion call as the chat surface
        // and forwards the rest as ordinary passthrough.
        let d = claude_web();
        for path in [
            "/api/organizations/b44129f9/skills/list-skills",
            "/api/organizations/b44129f9/usage",
        ] {
            assert_eq!(
                decide(&d, "claude.ai", path),
                Decision::Rewrite {
                    upstream_url: "https://claude.ai/api".into(),
                    slug: "claude-web".into()
                }
            );
        }
    }

    #[test]
    fn claude_web_does_not_touch_paths_outside_the_api_tree() {
        let d = claude_web();
        assert_eq!(decide(&d, "claude.ai", "/chat/abc"), Decision::Passthrough);
        assert_eq!(
            decide(&d, "claude.ai", "/_next/static/x.js"),
            Decision::Passthrough
        );
    }

    #[test]
    fn claude_web_and_anthropic_stay_separate_domains() {
        // The api.anthropic.com entry must not start matching claude.ai, and the
        // chat entry must not claim the API host: they are different protocols.
        assert_eq!(
            decide(&anthropic(), "claude.ai", CLAUDE_COMPLETION),
            Decision::Tunnel
        );
        assert_eq!(
            decide(&claude_web(), "api.anthropic.com", "/v1/messages"),
            Decision::Tunnel
        );
    }

    #[test]
    fn ca_name_constraints_cover_claude_ai_once_the_domain_ships() {
        // The CA's permitted subtrees are built from the WHOLE catalog, so a
        // missing host here means interception fails at the handshake.
        let hosts: Vec<String> = default_domains()
            .iter()
            .flat_map(|d| d.hosts.iter().cloned())
            .collect();
        assert!(hosts.contains(&"claude.ai".to_string()));
    }
    #[test]
    fn chatgpt_apps_rewrites_the_tool_plane_paths() {
        let d = chatgpt_apps();
        for path in [
            "/backend-api/ps/mcp",
            "/backend-api/wham/tasks/list",
            "/backend-api/wham/usage",
        ] {
            assert_eq!(
                decide(&d, "chatgpt.com", path),
                Decision::Rewrite {
                    upstream_url: "https://chatgpt.com".into(),
                    slug: "chatgpt-apps".into()
                },
                "{path} should route to Gate"
            );
        }
    }

    #[test]
    fn chatgpt_apps_leaves_the_model_call_to_the_relay_route() {
        // The embedded agent reaches chatgpt.com directly and is routed by
        // base_url through the relay, whose entry uses the other URL split.
        // Rewriting it here would send it upstream with `/backend-api` doubled.
        let d = chatgpt_apps();
        assert_eq!(
            decide(&d, "chatgpt.com", "/backend-api/codex/responses"),
            Decision::Passthrough
        );
    }

    #[test]
    fn chatgpt_apps_ignores_plugin_store_and_auth_noise() {
        let d = chatgpt_apps();
        for path in [
            "/backend-api/ps/plugins/installed",
            "/backend-api/settings/user",
            "/api/auth/session",
        ] {
            assert_eq!(decide(&d, "chatgpt.com", path), Decision::Passthrough);
        }
    }

    #[test]
    fn chatgpt_apps_is_ordered_before_the_relay_chatgpt_entry() {
        // Load-bearing: `decide` returns on the FIRST enabled host match, and both
        // entries name chatgpt.com. With the relay entry first, the MITM entry
        // would be unreachable and the tool plane would silently pass through.
        let catalog = default_domains();
        let mitm = catalog
            .iter()
            .position(|d| d.slug == "chatgpt-apps")
            .unwrap();
        let relay = catalog.iter().position(|d| d.slug == "chatgpt").unwrap();
        assert!(
            mitm < relay,
            "chatgpt-apps must precede chatgpt in the catalog"
        );
    }

    #[test]
    fn the_two_chatgpt_entries_use_opposite_url_splits() {
        // `relay::route` matches on upstream_url and `decide` on host, so the
        // entries coexist — but only because their upstreams differ. Collapsing
        // them onto one upstream would break whichever route lost.
        let catalog = default_domains();
        let mitm = catalog.iter().find(|d| d.slug == "chatgpt-apps").unwrap();
        let relay = catalog.iter().find(|d| d.slug == "chatgpt").unwrap();
        assert_eq!(mitm.upstream_url, "https://chatgpt.com");
        assert_eq!(relay.upstream_url, "https://chatgpt.com/backend-api");
        assert_ne!(mitm.upstream_url, relay.upstream_url);
        // Both opt-in.
        assert!(!mitm.enabled && !relay.enabled);
    }

    #[test]
    fn enabling_only_the_relay_entry_keeps_todays_passthrough_behaviour() {
        // A user who enabled Codex CLI but not the desktop tools must see no
        // change: the relay entry claims only its own short path and everything
        // else on the host is forwarded untouched.
        let mut relay_only: Vec<ProxyDomain> = default_domains()
            .into_iter()
            .filter(|d| d.slug == "chatgpt")
            .collect();
        relay_only[0].enabled = true;
        assert_eq!(
            decide(&relay_only, "chatgpt.com", "/backend-api/ps/mcp"),
            Decision::Passthrough
        );
    }

    fn chatgpt_relay_entry() -> Vec<ProxyDomain> {
        let mut d: ProxyDomain = default_domains()
            .into_iter()
            .find(|d| d.slug == "chatgpt")
            .expect("chatgpt is in the catalog");
        d.enabled = true;
        vec![d]
    }

    #[test]
    fn declines_the_upgrade_on_the_responses_path_it_would_route() {
        // The whole point: work mode sends its turn as WS frame 0 here, so
        // passing the upgrade through means Gate sees none of it.
        let d = chatgpt_relay_entry();
        assert!(should_decline_upgrade(
            &d,
            "chatgpt.com",
            "/backend-api/codex/responses"
        ));
    }

    #[test]
    fn leaves_the_relay_url_split_alone() {
        // Codex's short `/codex/responses` is NOT routed by `decide`: this
        // entry's upstream carries `/backend-api`, so `strip_upstream_path`
        // finds the path outside its subtree and abstains. That entry serves
        // the relay, which matches it on `upstream_url` and never consults
        // `decide` at all. So the MITM engine declines only the app's real wire
        // path, which is the only one observed upgrading anyway.
        let d = chatgpt_relay_entry();
        assert!(!should_decline_upgrade(
            &d,
            "chatgpt.com",
            "/codex/responses"
        ));
    }

    #[test]
    fn leaves_an_upgrade_alone_when_the_row_is_off() {
        // Declining is only useful where the fallback would then be routed. A
        // disabled row routes nothing, so breaking its upgrade buys nothing and
        // costs the user a working feature.
        let mut d = chatgpt_relay_entry();
        d[0].enabled = false;
        assert!(!should_decline_upgrade(
            &d,
            "chatgpt.com",
            "/backend-api/codex/responses"
        ));
    }

    #[test]
    fn leaves_upgrades_on_other_routed_paths_alone() {
        // The fallback evidence is specific to Responses. Any other path that
        // happens to upgrade is passed through as before rather than broken on
        // the assumption it recovers the same way.
        let d = chatgpt_apps();
        for path in [
            "/backend-api/f/conversation",
            "/backend-api/wham/tasks/list",
            "/backend-api/ps/mcp",
        ] {
            assert!(!should_decline_upgrade(&d, "chatgpt.com", path), "{path}");
        }
    }

    #[test]
    fn leaves_a_responses_lookalike_on_an_unrouted_host_alone() {
        // The suffix alone must not decide: `decide` has to agree the path is
        // one Gate would route, or an unrelated host exposing `/responses` would
        // have its upgrades broken for nothing.
        let d = chatgpt_relay_entry();
        assert!(!should_decline_upgrade(
            &d,
            "example.com",
            "/backend-api/codex/responses"
        ));
    }

    #[test]
    fn declining_stops_wherever_routing_stops() {
        // Declining rides entirely on `decide`, so anything that stops the path
        // being routed also stops it being declined - narrowing a client's
        // prefixes included. Pinned directly rather than through
        // `rules_for_client`, which today narrows only the `BROWSER_ROUTED`
        // slugs and leaves this entry untouched (the website never requests this
        // path). If that ever changes, this coupling still has to hold.
        let mut narrowed = chatgpt_relay_entry();
        narrowed[0].rewrite_prefixes.clear();
        assert!(!should_decline_upgrade(
            &narrowed,
            "chatgpt.com",
            "/backend-api/codex/responses"
        ));
    }

    #[test]
    fn both_chatgpt_entries_on_route_each_others_paths_untouched() {
        // The state a user can now reach from the ledger: two rows under OpenAI,
        // both switched on, one host. Each entry has to serve its own paths.
        // Stopping at the first host match sent the Responses call - the one
        // OpenClaw's subscription traffic depends on - into the chat entry, which
        // ignores that path on purpose, so it was forwarded unrouted while both
        // switches read "on".
        let both: Vec<ProxyDomain> = default_domains()
            .into_iter()
            .map(|mut d| {
                d.enabled = d.slug == "chatgpt" || d.slug == "chatgpt-apps";
                d
            })
            .collect();

        assert_eq!(
            decide(&both, "chatgpt.com", "/backend-api/codex/responses"),
            Decision::Rewrite {
                upstream_url: "https://chatgpt.com/backend-api".into(),
                slug: "chatgpt".into()
            },
            "the Responses call belongs to the `chatgpt` entry's split"
        );
        assert_eq!(
            decide(&both, "chatgpt.com", "/backend-api/f/conversation"),
            Decision::Rewrite {
                upstream_url: "https://chatgpt.com".into(),
                slug: "chatgpt-apps".into()
            },
            "and the app's chat turn still belongs to `chatgpt-apps`"
        );
        // An explicit passthrough in one entry is not overridden by the other
        // abstaining, and a path neither claims still reaches the real host.
        assert_eq!(
            decide(&both, "chatgpt.com", "/backend-api/f/conversation/prepare"),
            Decision::Passthrough
        );
        assert_eq!(
            decide(&both, "chatgpt.com", "/api/auth/session"),
            Decision::Passthrough
        );
    }

    #[test]
    fn chatgpt_apps_rewrites_the_chat_turn() {
        let d = chatgpt_apps();
        assert_eq!(
            decide(&d, "chatgpt.com", "/backend-api/f/conversation"),
            Decision::Rewrite {
                upstream_url: "https://chatgpt.com".into(),
                slug: "chatgpt-apps".into()
            }
        );
    }

    #[test]
    fn chatgpt_apps_leaves_the_conduit_prepare_call_alone() {
        // `…/f/conversation/prepare` only mints a short-lived conduit token and
        // carries neither prompt nor reply. It shares the chat prefix, so it needs
        // the explicit passthrough — which `decide` checks BEFORE rewrites.
        let d = chatgpt_apps();
        assert_eq!(
            decide(&d, "chatgpt.com", "/backend-api/f/conversation/prepare"),
            Decision::Passthrough
        );
    }

    #[test]
    fn chatgpt_apps_leaves_the_sentinel_proof_of_work_alone() {
        // The app computes its own sentinel tokens; routing those adds nothing.
        let d = chatgpt_apps();
        assert_eq!(
            decide(
                &d,
                "chatgpt.com",
                "/backend-api/sentinel/chat-requirements/prepare"
            ),
            Decision::Passthrough
        );
    }

    /// The distinction `engine_hosted_elsewhere` exists to draw, and the one
    /// that decides whether `enable` refuses: a snapshot on disk means Gate
    /// turned routing on, not that anyone is still serving it. A crashed
    /// session leaves the file behind, and refusing an enable on that basis
    /// would lock the user out of the command that fixes their machine.
    ///
    /// macOS/Windows only, like the function - Linux adopts its daemon instead
    /// of probing. Verified on Linux while writing by widening both cfgs, so
    /// the assertion is not taken on trust until a mac or Windows runner sees it.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn a_snapshot_without_a_listener_is_not_a_hosted_engine() {
        use std::net::TcpListener;

        // `app_support_dir` checks the `GATE_CONNECT_TEST_HOME` env seam before
        // the mutex override installed below, so a concurrent test holding that
        // var set would send these writes into *its* temp dir, which then
        // either vanishes when that test cleans up, or leaves the reads
        // resolving somewhere else again once it restores the var. Both were
        // seen before this lock: the first on Windows, the second on macOS.
        // Every path-redirecting test in the crate serializes on this lock;
        // see `crate::env::path_env_lock`.
        let _lock = crate::env::path_env_lock();

        let home = std::env::temp_dir().join(format!(
            "gate-hosted-elsewhere-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock before the epoch")
                .as_nanos()
        ));
        crate::env::set_app_support_dir_for_tests(Some(home.clone()));

        // Nothing on disk at all: nobody has ever routed.
        assert_eq!(engine_hosted_elsewhere(), None, "no snapshot, no engine");

        // A live listener, and a snapshot recording that routing is on. This is
        // the menubar-app-is-running case, and the only one that must refuse.
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("binding a probe listener");
        let port = listener.local_addr().expect("listener address").port();
        system_proxy::save_port(port).expect("persisting the port");
        system_proxy::save_snapshot(&system_proxy::snapshot().expect("reading the system proxy"))
            .expect("saving a snapshot");
        assert_eq!(
            engine_hosted_elsewhere(),
            Some(port),
            "a snapshot plus a live listener is another process hosting the engine"
        );

        // Same snapshot, listener gone: the crashed-session case. Enable has to
        // go through, so this must read as "nobody is hosting".
        drop(listener);
        assert_eq!(
            engine_hosted_elsewhere(),
            None,
            "a snapshot left by a crash must not look like a live engine"
        );

        crate::env::set_app_support_dir_for_tests(None);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The `User-Agent` guess is the only tool signal either path has, so its
    /// misses matter as much as its hits.
    #[test]
    fn identifies_a_tool_only_when_its_agent_says_so() {
        let tool = |ua: &str| {
            let mut h = HeaderMap::new();
            h.insert(
                hyper::header::USER_AGENT,
                HeaderValue::from_str(ua).unwrap(),
            );
            client_tool(&h)
        };

        assert_eq!(
            tool("claude-cli/2.1.0 (external, cli)"),
            Some("claude-code")
        );
        assert_eq!(tool("codex_cli_rs/0.55.0"), Some("codex"));
        assert_eq!(tool("opencode/0.4.2"), Some("opencode"));
        // Case is the tool's business, not ours: one tool has to be one series.
        assert_eq!(tool("Codex/1.0"), Some("codex"));

        // No agent, or one we don't recognise, is unattributed - never a guess.
        // A wrong slug would put one tool's traffic under another's name in the
        // very view the user reads to find out what their machine is doing.
        assert_eq!(client_tool(&HeaderMap::new()), None);
        assert_eq!(tool("curl/8.7.1"), None);
        assert_eq!(tool("Mozilla/5.0 (Macintosh) Chrome/120"), None);
    }

    /// Attribution is stamped from our own state, never from the caller's.
    #[test]
    fn attribution_overwrites_whatever_the_caller_claimed() {
        let mut h = HeaderMap::new();
        h.insert(
            HeaderName::from_static(GATE_INSTALL_ID_HEADER),
            HeaderValue::from_static("someone-elses-machine"),
        );
        h.insert(
            HeaderName::from_static(GATE_CLIENT_HEADER),
            HeaderValue::from_static("claude-code"),
        );

        inject_attribution(&mut h);

        // The client header is derived from the User-Agent, and there is none
        // here, so the claim is dropped rather than believed.
        assert_eq!(h.get(GATE_CLIENT_HEADER), None);
        // Whatever the id resolves to, it is ours or it is absent - a local
        // process cannot label its traffic as another installation's.
        let claimed = h
            .get(GATE_INSTALL_ID_HEADER)
            .map(|v| v.to_str().unwrap().to_string());
        assert_ne!(claimed.as_deref(), Some("someone-elses-machine"));
        assert_eq!(claimed.as_deref(), crate::primitives::install_id_cached());
    }

    /// PAYG applies per domain, and the list is an allowlist: a domain nobody
    /// has cleared for reseller routing keeps its BYOK shape even while the
    /// account bills through Gate. The consumer-chat surfaces are the ones this
    /// protects - they authenticate with a session cookie, so stripping it would
    /// break them and route nothing.
    #[test]
    fn payg_applies_only_to_the_eligible_domains() {
        for slug in ["anthropic", "openai", "openrouter"] {
            assert_eq!(
                effective_billing_mode(BillingMode::Payg, slug),
                BillingMode::Payg,
                "{slug} serves a gateway-native inference path"
            );
        }
        for slug in ["claude-web", "chatgpt-apps", "chatgpt", "opencode"] {
            assert_eq!(
                effective_billing_mode(BillingMode::Payg, slug),
                BillingMode::Byok,
                "{slug} is a subscription or non-reseller path"
            );
        }
        // A domain added later defaults to BYOK rather than silently starting to
        // spend an org's balance.
        assert_eq!(
            effective_billing_mode(BillingMode::Payg, "some-future-provider"),
            BillingMode::Byok
        );
        // And BYOK is never widened by the eligibility list.
        assert_eq!(
            effective_billing_mode(BillingMode::Byok, "anthropic"),
            BillingMode::Byok
        );
    }

    /// Every eligible slug must actually exist in the catalog: a typo here would
    /// silently keep PAYG off for that provider, which is the failure mode this
    /// allowlist is otherwise good at hiding.
    #[test]
    fn every_payg_eligible_slug_is_a_real_catalog_entry() {
        let catalog = default_domains();
        for slug in PAYG_ELIGIBLE_SLUGS {
            assert!(
                catalog.iter().any(|d| d.slug == slug),
                "{slug} is listed as PAYG-eligible but is not in the catalog"
            );
        }
    }

    /// PAYG removes the tool's own credential, and does so even when the caller
    /// supplied its own Gate key - that branch leaves the Gate headers alone,
    /// but a provider token sitting beside them would still force BYOK.
    #[test]
    fn payg_strips_the_client_credential_even_behind_a_caller_supplied_key() {
        let mut h = HeaderMap::new();
        h.insert(
            HeaderName::from_static(GATE_KEY_HEADER),
            HeaderValue::from_static("sk-gw-callers-own"),
        );
        h.insert(
            hyper::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer sk-ant-oat01-app"),
        );
        h.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_static("sk-ant-api03-app"),
        );

        inject_gate_credential(&mut h, "sk-gw-ours", None, None, BillingMode::Payg).unwrap();

        assert_eq!(h.get(GATE_KEY_HEADER).unwrap(), "sk-gw-callers-own");
        assert_eq!(h.get(hyper::header::AUTHORIZATION), None);
        assert_eq!(h.get("x-api-key"), None);
    }

    /// Attribution rides alongside the credential decision without touching it.
    #[test]
    fn a_caller_supplied_key_still_gets_attributed() {
        let mut h = HeaderMap::new();
        h.insert(
            HeaderName::from_static(GATE_KEY_HEADER),
            HeaderValue::from_static("sk-gw-callers-own"),
        );
        h.insert(
            hyper::header::USER_AGENT,
            HeaderValue::from_static("claude-cli/2.1.0"),
        );

        inject_gate_credential(
            &mut h,
            "sk-gw-ours",
            Some("token"),
            Some("org"),
            BillingMode::Byok,
        )
        .unwrap();

        // The credential is left exactly as it arrived: that branch is the
        // caller's to own.
        assert_eq!(h.get(GATE_KEY_HEADER).unwrap(), "sk-gw-callers-own");
        assert_eq!(h.get(GATE_AUTHORIZATION_HEADER), None);
        // The request still left this machine, from this tool, so it is still
        // attributable. Failing to record that would leave an unexplained hole
        // in the activity view.
        assert_eq!(h.get(GATE_CLIENT_HEADER).unwrap(), "claude-code");
    }
}
