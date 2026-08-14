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

#[cfg(target_os = "macos")]
mod manager;
#[cfg(target_os = "windows")]
#[path = "manager_windows.rs"]
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
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn engine_proxy_url() -> Option<String> {
    if !engine_likely_running() {
        return None;
    }
    persisted_engine_proxy_url()
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
pub(crate) fn inject_gate_credential(
    headers: &mut HeaderMap,
    api_key: &str,
    oauth_token: Option<&str>,
    org_id: Option<&str>,
) -> Result<()> {
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
    /// Rewrite to the gateway, injecting this upstream URL.
    Rewrite { upstream_url: String },
}

/// True if any enabled domain claims `host`. Used by the engine's
/// `should_intercept` to gate MITM at the CONNECT stage.
pub(crate) fn should_intercept_host(domains: &[ProxyDomain], host: &str) -> bool {
    domains.iter().any(|d| d.enabled && d.matches_host(host))
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

/// Entries that decline BROWSER traffic while routing everything else.
///
/// `chatgpt-apps` is here because its switch is named after an app but matches
/// on host, so flipping it also routed the user's browser tabs, carrying the
/// browsing session's own cookie to Gate. The exclusion is deliberately narrow:
/// it drops only what is positively identified as the website, so OpenClaw,
/// Hermes and anything else proxy-honouring keep their route.
///
/// That choice sets the failure direction, and it is the opposite of the one an
/// app-only rule would give. If the website stops sending the markers
/// [`classify_client`] looks for, browser traffic classifies as `Unknown` and is
/// routed again. The alternative - route only a positively-identified app - fails
/// the other way and would have silently dropped every third-party client, which
/// is the larger loss.
///
/// `claude-web` is NOT here, and that is a gap rather than a decision: claude.ai
/// has the same app-and-browser overlap, but no capture of it exists yet, so
/// there are no markers to key on. Add it when there are.
///
/// A code-level policy rather than a `ProxyDomain` field on purpose: a
/// serialized field would imply a per-domain switch the UI does not have. When
/// it grows one, this becomes that field and the constant goes away.
const BROWSER_EXCLUDED_SLUGS: &[&str] = &["chatgpt-apps"];

/// Classify a request from its headers.
///
/// `originator` is the primary signal: it is the vendor's own "which front-end
/// is this" field, it was present on EVERY app request to a routed path in the
/// captures, and on none of the web ones. The user-agent prefix is the fallback,
/// because a build that drops `originator` but still says `ChatGPTBrowser`
/// should keep routing.
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
    if header("originator").is_some_and(|v| !v.trim().is_empty()) {
        return ClientClass::App;
    }
    let ua = header("user-agent").unwrap_or_default();
    // `ChatGPTBrowser` prefixes the app shell's otherwise browser-shaped UA;
    // the other two are Codex's native agent and its MCP client.
    if ua.starts_with("ChatGPTBrowser")
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
            if client != ClientClass::Web || !BROWSER_EXCLUDED_SLUGS.contains(&d.slug.as_str()) {
                return d.clone();
            }
            ProxyDomain {
                rewrite_prefixes: Vec::new(),
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
        if d.rewrite_prefixes
            .iter()
            .any(|p| path.starts_with(p.as_str()))
        {
            return Decision::Rewrite {
                upstream_url: d.upstream_url.clone(),
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

/// The built-in domain catalog. All entries ship `supported:true` (Anthropic
/// is also `enabled` by default; the rest are opt-in). New providers can be
/// added here and surface in the UI automatically; gate a provider behind
/// `supported:false` until Gate's upstream support for it is confirmed.
pub fn default_domains() -> Vec<ProxyDomain> {
    vec![
        ProxyDomain {
            slug: "anthropic".into(),
            // Named for what it covers (the apps whose traffic this
            // intercepts), not the vendor: on the UI ledger a vendor name
            // here would read as if it included Claude Code, which routes by
            // config instead. The host line carries api.anthropic.com.
            display_name: "Claude Desktop / Cowork".into(),
            // Inference for Claude Code, Claude Desktop, and Cowork all goes
            // to api.anthropic.com /v1/messages (OAuth bearer or API key),
            // confirmed against a real Cowork generation. a-api.anthropic.com
            // is Anthropic's telemetry host (Segment-style /v1/b ingestion)
            // and is deliberately left tunnelled, never intercepted. claude.ai
            // is the web/chat/login surface and is NOT part of this entry — it
            // speaks a different protocol and has its own opt-in `claude-web`
            // domain below.
            hosts: vec!["api.anthropic.com".into()],
            // Applies to every host above. Only group hosts that genuinely
            // share this upstream - never collapse distinct API hosts onto one.
            upstream_url: "https://api.anthropic.com".into(),
            // Only genuine inference endpoints are rewritten to the gateway.
            // Scoped deliberately narrow: Claude Desktop / Cowork also make
            // OAuth + account calls on this same host under /v1/ (e.g.
            // /v1/oauth/*, /v1/organizations/*), and those carry no model, so
            // the gateway can't classify them and rejects them 503 ("AI
            // unknown"). Rewriting only /v1/messages (covers count_tokens +
            // batches sub-paths) and legacy /v1/complete lets every other /v1/
            // path fall through to `decide`'s default Passthrough and reach the
            // real host unchanged. Do NOT widen this back to "/v1/".
            rewrite_prefixes: vec!["/v1/messages".into(), "/v1/complete".into()],
            // Paths outside the inference set already pass through; this keeps
            // the Squirrel auto-updater explicit. Other /api/* paths
            // (claude_code, event_logging, bootstrap) also reach the real host
            // unrewritten.
            passthrough_prefixes: vec!["/api/desktop/".into()],
            enabled: true,
            supported: true,
        },
        ProxyDomain {
            slug: "claude-web".into(),
            // Claude Desktop's CHAT surface, which is a different protocol from
            // the entry above rather than more of the same host. That one covers
            // api.anthropic.com /v1/messages; this one covers claude.ai, where
            // the desktop app sends a bare `prompt` string and Anthropic keeps
            // the conversation history server-side. Gate recognises it as the
            // `claude-web-chat` surface and treats it as inspection + audit, not
            // as key-brokered routing: there is no API key involved at all.
            display_name: "Claude Desktop chat".into(),
            hosts: vec!["claude.ai".into()],
            // The `/api` MUST ride in the upstream URL, not the forwarded path,
            // for the same reason it does on OpenRouter below: Gate's ALB routes
            // `/api/*` to the dashboard API, so a forwarded
            // `/api/organizations/...` never reaches the gateway proxy at all.
            // Every path this entry cares about lives under `/api`, so the whole
            // segment moves upstream-side and the prefixes below are written
            // POST-STRIP - `decide` and `apply_rewrite` both match and forward
            // the path Gate will actually see.
            //
            // Gate must accept the stripped spelling for the chat surface to
            // stay classified (gateway-proxy: `CLAUDE_WEB_CHAT_COMPLETION_RE` in
            // utils/proxy-helpers.ts, which anchors on `^/api/organizations/`).
            // The Codex and ChatGPT anchors beside it already tolerate both
            // splits with an optional prefix group; this one needs the same
            // treatment or the completion call is tagged `api` and loses the
            // additive-credential policy the session cookie depends on.
            upstream_url: "https://claude.ai/api".into(),
            // Prefix matching cannot isolate the chat call on its own: the
            // endpoint is `/api/organizations/{org}/chat_conversations/{conv}/completion`,
            // so the varying id sits BEFORE the distinguishing final segment.
            // Rewriting the whole organizations tree is deliberate and
            // safe — Gate classifies only the completion path as the chat
            // surface and forwards the sibling calls (skills, usage,
            // conversation reads) as ordinary passthrough, which it has explicit
            // coverage for. They add audit rows, not behaviour changes.
            rewrite_prefixes: vec!["/organizations/".into()],
            // Everything here would be pure noise or actively harmful to route:
            // the updater channel, telemetry batches, and the bootstrap/account
            // calls the app makes before any conversation exists. Written
            // post-strip, so these are the app's `/api/desktop/` etc.
            passthrough_prefixes: vec![
                "/desktop/".into(),
                "/event_logging/".into(),
                "/bootstrap/".into(),
            ],
            // Opt-in. This surface carries the user's Claude SESSION cookie
            // rather than an API key, so it should never start intercepting
            // without a deliberate toggle.
            //
            // Deliberately NOT attached to the `anthropic` provider's
            // `proxy_domain_slugs` (see `provider.rs`): `provider::enable` turns
            // on every domain a provider lists, so attaching it would route the
            // session surface the moment someone enabled "Claude" - defeating
            // the opt-in above. It rides that provider's `chat_domain_slugs`
            // instead, which is how it reaches the Home ledger: `buildGroups`
            // (src/lib/groups.ts) gives it a row and a switch under Claude, and
            // `setGroupRouted` filters it out of the family cascade, so the only
            // thing that can enable it is that row's own switch (or
            // `proxy domain claude-web on` from the CLI).
            enabled: false,
            supported: true,
        },
        ProxyDomain {
            slug: "openai".into(),
            // "apps", not the vendor name: covers any system-proxy-honoring
            // client of api.openai.com, and must not read as including Codex
            // (config-routed; its embedded agent ignores the system proxy).
            display_name: "OpenAI apps".into(),
            // The OpenAI API host. Catches OpenAI-compatible clients that
            // honor the macOS system proxy and hit /v1/. Note: the Codex
            // desktop app's model calls come from its embedded Rust agent,
            // which ignores the system proxy and reaches chatgpt.com
            // directly, so the proxy can't capture them - route Codex via the
            // manual integration (config.toml base_url) instead.
            hosts: vec!["api.openai.com".into()],
            upstream_url: "https://api.openai.com".into(),
            // Inference endpoints only, same reasoning as Anthropic above: a
            // client's non-inference /v1/ calls (e.g. /v1/models preflight)
            // carry no model, so the gateway can't classify them and 503s.
            // Rewrite only the model-call paths; everything else on the host
            // passes through to real api.openai.com. Do NOT widen back to "/v1/".
            rewrite_prefixes: vec![
                "/v1/chat/completions".into(),
                "/v1/completions".into(),
                "/v1/responses".into(),
                "/v1/embeddings".into(),
            ],
            passthrough_prefixes: vec![],
            enabled: false,
            supported: true,
        },
        ProxyDomain {
            slug: "chatgpt-apps".into(),
            display_name: "ChatGPT app chat + Codex tools".into(),
            // Codex Desktop's TOOL traffic, which is a separate route from the
            // `chatgpt` entry below even though both name chatgpt.com.
            //
            // That entry is RELAY-only: it exists so the relay recognises the
            // upstream hint `integrations/codex.rs` writes, and it is matched by
            // `relay::route` on `upstream_url`. This entry is the MITM half,
            // matched by `decide` on HOST — which is why the two can share a
            // host without colliding, and why the split below differs.
            //
            // What this can and cannot capture. The Electron shell honours the
            // system proxy, and the tool-plane calls observed in a capture came
            // from it: `/backend-api/wham/*` carried a Chromium user-agent.
            // `/backend-api/ps/mcp` sends no user-agent at all, so which
            // component emits it is still unverified, though the engine does see
            // it.
            //
            // The desktop app's MODEL call is visible too, and is NOT served by
            // this entry — it is served by the `chatgpt` entry below, whose
            // upstream path absorbs `/backend-api` and leaves
            // `/codex/responses` for its rewrite prefix. Confirmed 2026-08-14
            // from a captured Gate row whose body carried
            // `<app-context># Codex desktop context`, `workspace_kind:
            // "projectless"` and Windows paths under `Documents\Codex`.
            //
            // This comment previously said the opposite — that Codex's embedded
            // Rust agent ignores the system proxy, so its model calls stay
            // invisible to the engine. That holds for the standalone CLI, whose
            // agent routes via the relay, and it is why the exclusion below is
            // still correct. It does NOT hold for the desktop app, and stating it
            // unconditionally cost two debugging sessions: the traffic was
            // assumed unreachable when it was merely on the other row. Which row
            // is the whole point, because `chatgpt-apps` and `chatgpt` are
            // separate switches and this one's NAME implies it carries Codex's
            // prompts. It does not.
            hosts: vec!["chatgpt.com".into()],
            // MITM convention: `engine::apply_rewrite` preserves the request path
            // and query VERBATIM and swaps only scheme + authority, so the
            // upstream is the BARE host and the paths below are the app's real
            // ones. The relay entry uses the opposite split (`/backend-api` in
            // the upstream, short client path) because the relay sees the path
            // Codex rewrote, not the real one. Gate accepts both spellings.
            upstream_url: "https://chatgpt.com".into(),
            // Only the two path families Gate classifies as native surfaces:
            // the MCP tool plane (`codex-mcp`, scanned for indirect injection)
            // and the task/settings reads (`codex-tasks`). Deliberately NOT
            // `/backend-api/codex/responses` — that path belongs to the `chatgpt`
            // entry, which serves it on BOTH routes, and claiming it here would
            // send it upstream under this entry's bare-host split. The URL would
            // still resolve; what breaks is that one endpoint would then carry two
            // different `X-Gate-Upstream-Url` values depending on how it arrived,
            // which is the split-mismatch class that once left MITM traffic
            // classified as plain `api`. Excluding it is not a coverage gap: this
            // entry sits FIRST in catalog order, so claiming it would shadow the
            // other. Plugin-store listings are left out as pure noise.
            // `/backend-api/f/conversation` is the ChatGPT app's own chat turn
            // (Gate's `chatgpt-web-chat` surface): one message per request, reply
            // as a `delta_encoding: v1` SSE stream. It lives in THIS entry rather
            // than its own because `decide` returns on the first enabled
            // host match — a second chatgpt.com entry would be dead code.
            //
            // The `…/f/conversation/prepare` sibling is deliberately absent: it
            // only mints a short-lived `conduit_token` and carries neither prompt
            // nor reply, so routing it would add audit noise and nothing else.
            rewrite_prefixes: vec![
                "/backend-api/f/conversation".into(),
                "/backend-api/ps/mcp".into(),
                "/backend-api/wham/".into(),
            ],
            // `/backend-api/f/conversation/prepare` starts with the chat prefix
            // above, so it needs an explicit passthrough to stay unrouted —
            // passthrough prefixes are checked first in `decide`.
            passthrough_prefixes: vec!["/backend-api/f/conversation/prepare".into()],
            // Opt-in, and no longer order-sensitive against the `chatgpt` entry
            // below: `decide` consults every enabled entry claiming the host, so
            // each of the two serves its own paths whether one, the other, or
            // both are switched on. It used to stop at the first host match,
            // which made this entry swallow the Responses call sitting in that
            // one - a hazard that mattered the moment either became togglable
            // without reading this file.
            //
            // Like `claude-web` above, this slug is in no provider's
            // `proxy_domain_slugs` and rides `chat_domain_slugs` instead: it
            // gets a Home ledger row and a switch under OpenAI, and the family
            // switch's cascade skips it, so the chat half of this entry
            // (`/backend-api/f/conversation`, a session-cookie surface) can only
            // be enabled from its own row or from `proxy domain chatgpt-apps
            // on`. Whatever else exposes this entry must keep that property.
            enabled: false,
            supported: true,
        },
        ProxyDomain {
            slug: "chatgpt".into(),
            display_name: "ChatGPT (Codex subscription)".into(),
            // The Responses API a ChatGPT-subscription login talks to:
            // chatgpt.com/backend-api/codex/responses, bearer = the user's
            // ChatGPT OAuth token, passed through. TWO clients arrive here by
            // different routes, which is why the entry has to serve both:
            //
            // - Codex, via the relay. Its `base_url` points at the loopback
            //   relay (integrations/codex.rs) because its embedded agent ignores
            //   the system proxy; the relay matches this entry on `upstream_url`.
            // - OpenClaw, via the MITM engine. Managed proxy mode honours the
            //   proxy, so `decide` matches this entry on HOST and the engine
            //   rewrites the call - provided this domain is on, which is the
            //   user's own switch to flip (`provider::chat_domain_slugs` gives
            //   it a row under OpenAI). `integrations/openclaw.rs` prints a note
            //   naming this slug rather than enabling it.
            //
            // Both routes work off one split because `engine::apply_rewrite`
            // strips the upstream's own path from the forwarded path exactly as
            // the relay does, so a real `/backend-api/codex/responses` and
            // Codex's rewritten `/codex/responses` both arrive at the gateway as
            // `/codex/responses` under this upstream.
            hosts: vec!["chatgpt.com".into()],
            // Shape matches integrations/codex.rs exactly, because the relay
            // exact-matches the `X-Gate-Upstream-Url` header codex.rs writes:
            // the `/backend-api` segment rides in the upstream here, and the
            // client-side path is the short `/codex/responses` (Codex's
            // base_url is `<relay>/codex`, wire_api appends `/responses`). The
            // gateway concatenates path onto upstream, yielding
            // `https://chatgpt.com/backend-api/codex/responses`. That is the
            // opposite split from the `chatgpt-apps` entry above, which carries
            // the app's real paths off a bare host. The two coexist on one host
            // because `decide` consults every enabled entry rather than
            // stopping at the first - see its docs.
            upstream_url: "https://chatgpt.com/backend-api".into(),
            rewrite_prefixes: vec!["/codex/responses".into()],
            passthrough_prefixes: vec![],
            enabled: false,
            supported: true,
        },
        ProxyDomain {
            slug: "openrouter".into(),
            display_name: "OpenRouter apps".into(),
            // OpenRouter's API lives at openrouter.ai/api/v1/* (OpenAI-shaped
            // chat/completions). Opt-in like OpenAI; intercepts OpenRouter
            // clients that honor the system proxy.
            hosts: vec!["openrouter.ai".into()],
            // The `/api` MUST ride in the upstream URL, not the forwarded path.
            // Gate's ALB routes `/api/*` (plus /orgs/, /admin/, /me/,
            // /agent-templates/) to the dashboard API, so a forwarded
            // `/api/v1/chat/completions` never reaches the gateway proxy at all
            // - it 404s out of a service that has no such route. Keeping `/api`
            // upstream-side sends `/v1/chat/completions`, which clears the rule,
            // and Gate reassembles the two into the URL OpenRouter serves.
            // `forwarded_paths_avoid_gate_reserved_prefixes` pins this.
            upstream_url: "https://openrouter.ai/api".into(),
            rewrite_prefixes: vec!["/v1/".into()],
            passthrough_prefixes: vec![],
            enabled: false,
            supported: true,
        },
        ProxyDomain {
            slug: "opencode".into(),
            display_name: "OpenCode Zen / Go".into(),
            // Zen (`/zen/v1/*`) and Go (`/zen/go/v1/*`) are the same host and
            // the same upstream, separated only by path, so they are ONE entry:
            // `decide` returns on the first host match, so a second entry
            // sharing `opencode.ai` would never be consulted.
            hosts: vec!["opencode.ai".into()],
            upstream_url: "https://opencode.ai".into(),
            // Inference endpoints only, same reasoning as Anthropic and OpenAI
            // above: a `/zen/v1/models` preflight carries no model, so the
            // gateway can't classify it and 503s. Both Zen and Go host
            // OpenAI-shaped and Anthropic-shaped endpoints under the same
            // prefix, hence two leaves each. Do NOT widen to "/zen/".
            rewrite_prefixes: vec![
                "/zen/v1/chat/completions".into(),
                "/zen/v1/messages".into(),
                "/zen/go/v1/chat/completions".into(),
                "/zen/go/v1/messages".into(),
            ],
            passthrough_prefixes: vec![],
            enabled: false,
            supported: true,
        },
    ]
}

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
                        upstream_url: d.upstream_url.clone()
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
            "ChatGPTBrowser Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/151.0.0.0",
            "Codex Desktop/0.148.0-alpha.9 (Windows 10.0.26200; x86_64)",
            "codex-mcp-client/0.148.0-alpha.9",
        ] {
            assert_eq!(
                classify_client(hdrs(&[("user-agent", ua)])),
                ClientClass::App,
                "{ua}"
            );
        }
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
    fn only_the_browser_loses_the_shared_chat_turn() {
        // The whole point: the app and the website POST the same endpoint on the
        // same host, so only the client class separates them. Everything that is
        // not the website keeps the route, which is what lets OpenClaw, Hermes
        // and any other proxy-honouring client through the same entry.
        const TURN: &str = "/backend-api/f/conversation";
        let all: Vec<ProxyDomain> = default_domains()
            .into_iter()
            .map(|mut d| {
                d.enabled = d.slug == "chatgpt-apps";
                d
            })
            .collect();

        for class in [ClientClass::App, ClientClass::Unknown] {
            assert_eq!(
                decide(&rules_for_client(&all, class), "chatgpt.com", TURN),
                Decision::Rewrite {
                    upstream_url: "https://chatgpt.com".into()
                },
                "{class:?} must keep the route"
            );
        }
        assert_eq!(
            decide(
                &rules_for_client(&all, ClientClass::Web),
                "chatgpt.com",
                TURN
            ),
            Decision::Passthrough,
            "the website must reach the real host untouched"
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
                upstream_url: "https://chatgpt.com/backend-api".into()
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
                upstream_url: "https://api.anthropic.com".into()
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
                upstream_url: "https://api.anthropic.com".into()
            }
        );
        // count_tokens rides under /v1/messages, so the prefix still catches it.
        assert_eq!(
            decide(&d, "api.anthropic.com", "/v1/messages/count_tokens"),
            Decision::Rewrite {
                upstream_url: "https://api.anthropic.com".into()
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
                upstream_url: "https://openrouter.ai/api".into()
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
                upstream_url: "https://api.openai.com".into()
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
                    upstream_url: "https://api.openai.com".into()
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
                upstream_url: "https://claude.ai/api".into()
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
                upstream_url: "https://claude.ai/api".into()
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
                    upstream_url: "https://claude.ai/api".into()
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
                    upstream_url: "https://chatgpt.com".into()
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
                upstream_url: "https://chatgpt.com/backend-api".into()
            },
            "the Responses call belongs to the `chatgpt` entry's split"
        );
        assert_eq!(
            decide(&both, "chatgpt.com", "/backend-api/f/conversation"),
            Decision::Rewrite {
                upstream_url: "https://chatgpt.com".into()
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
                upstream_url: "https://chatgpt.com".into()
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
}
