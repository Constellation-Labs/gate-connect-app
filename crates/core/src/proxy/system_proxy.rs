//! macOS system HTTP/HTTPS proxy wiring via `networksetup`. Enabling points
//! every active network service at our loopback PAC via the auto-proxy URL
//! (`-setautoproxyurl`) and turns the manual web/secure slots off, so only
//! Gate's intercepted hosts route to the engine and everything else stays
//! direct or falls back to the user's prior proxy; disabling restores the
//! exact prior state from a snapshot (mirrors the `previousEnv` restore pattern
//! in [`crate::env`]). Reads are non-privileged; only the set/restore step
//! needs admin, so the manager batches it with the CA-trust change into one
//! prompt.

use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::env;
use crate::primitives::sh_quote;

const NETWORKSETUP: &str = "/usr/sbin/networksetup";

/// One proxy slot (web or secure-web) for a service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxySetting {
    pub enabled: bool,
    pub server: String,
    pub port: String,
}

/// A service's automatic proxy (PAC) slot - the `networksetup` auto-proxy URL
/// and whether it's on.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AutoProxy {
    pub enabled: bool,
    pub url: String,
}

/// Snapshot of one network service's proxy configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceProxy {
    pub service: String,
    pub web: ProxySetting,
    pub secure: ProxySetting,
    /// Automatic proxy (PAC) slot. Defaulted so a snapshot written by an older
    /// build (before PAC mode) still loads.
    #[serde(default)]
    pub auto: AutoProxy,
}

fn snapshot_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?
        .join("proxy")
        .join("system-proxy.snapshot.json"))
}

/// Path where we persist the engine's chosen loopback port so it can be reused
/// across restarts. macOS reads the system proxy live, but clients that resolve
/// the proxy once at their own launch (e.g. Claude Desktop) keep dialing the
/// old port after we restart - a stable port keeps them working without a
/// client restart.
fn port_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join("proxy").join("port"))
}

/// The last engine port we persisted, if any and still parseable.
pub fn load_port() -> Result<Option<u16>> {
    let path = port_path()?;
    match fs::read_to_string(&path) {
        Ok(raw) => Ok(raw.trim().parse::<u16>().ok()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Persist the engine port for reuse on the next run. Best-effort durability;
/// non-secret, so written 0644.
pub fn save_port(port: u16) -> Result<()> {
    let path = port_path()?;
    crate::primitives::write_file(&path, port.to_string().as_bytes(), 0o644)
        .with_context(|| format!("writing {}", path.display()))
}

/// Path where we persist the PAC listener's port, companion to [`port_path`]:
/// the system proxy's `AutoConfigURL` bakes this port in, and a client that
/// captured that URL at its own launch must find a fresh PAC there after we
/// restart, or its PAC fetch fails and it silently falls back to DIRECT
/// (bypassing Gate).
fn pac_port_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join("proxy").join("pac-port"))
}

/// The last PAC port we persisted, if any and still parseable.
pub fn load_pac_port() -> Result<Option<u16>> {
    let path = pac_port_path()?;
    match fs::read_to_string(&path) {
        Ok(raw) => Ok(raw.trim().parse::<u16>().ok()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Persist the PAC port for reuse on the next run. Best-effort durability;
/// non-secret, so written 0644.
pub fn save_pac_port(port: u16) -> Result<()> {
    let path = pac_port_path()?;
    crate::primitives::write_file(&path, port.to_string().as_bytes(), 0o644)
        .with_context(|| format!("writing {}", path.display()))
}

/// Active (non-disabled) network services. The first output line is a
/// header; services prefixed with `*` are disabled and skipped.
pub fn active_services() -> Result<Vec<String>> {
    let out = Command::new(NETWORKSETUP)
        .arg("-listallnetworkservices")
        .output()
        .context("running networksetup -listallnetworkservices")?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut services = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if i == 0 || line.starts_with('*') {
            continue;
        }
        let s = line.trim();
        if !s.is_empty() {
            services.push(s.to_string());
        }
    }
    Ok(services)
}

fn parse_proxy(output: &str) -> ProxySetting {
    let mut setting = ProxySetting {
        enabled: false,
        server: String::new(),
        port: String::new(),
    };
    for line in output.lines() {
        if let Some(v) = line.strip_prefix("Enabled:") {
            setting.enabled = v.trim().eq_ignore_ascii_case("Yes");
        } else if let Some(v) = line.strip_prefix("Server:") {
            setting.server = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Port:") {
            setting.port = v.trim().to_string();
        }
    }
    setting
}

fn get_proxy(flag: &str, service: &str) -> Result<ProxySetting> {
    let out = Command::new(NETWORKSETUP)
        .args([flag, service])
        .output()
        .with_context(|| format!("running networksetup {flag} {service:?}"))?;
    Ok(parse_proxy(&String::from_utf8_lossy(&out.stdout)))
}

/// Parse `networksetup -getautoproxyurl` output. When no PAC is set macOS
/// prints `URL: (null)`, which we normalize to an empty URL.
fn parse_autoproxy(output: &str) -> AutoProxy {
    let mut auto = AutoProxy::default();
    for line in output.lines() {
        if let Some(v) = line.strip_prefix("URL:") {
            let v = v.trim();
            auto.url = if v == "(null)" {
                String::new()
            } else {
                v.to_string()
            };
        } else if let Some(v) = line.strip_prefix("Enabled:") {
            auto.enabled = v.trim().eq_ignore_ascii_case("Yes");
        }
    }
    auto
}

fn get_autoproxy(service: &str) -> Result<AutoProxy> {
    let out = Command::new(NETWORKSETUP)
        .args(["-getautoproxyurl", service])
        .output()
        .with_context(|| format!("running networksetup -getautoproxyurl {service:?}"))?;
    Ok(parse_autoproxy(&String::from_utf8_lossy(&out.stdout)))
}

/// Read the current proxy config for every active service. Non-privileged.
pub fn snapshot() -> Result<Vec<ServiceProxy>> {
    let mut snapshot = Vec::new();
    for service in active_services()? {
        let web = get_proxy("-getwebproxy", &service)?;
        let secure = get_proxy("-getsecurewebproxy", &service)?;
        let auto = get_autoproxy(&service)?;
        snapshot.push(ServiceProxy {
            service,
            web,
            secure,
            auto,
        });
    }
    Ok(snapshot)
}

pub fn save_snapshot(snapshot: &[ServiceProxy]) -> Result<()> {
    let path = snapshot_path()?;
    let raw = serde_json::to_string_pretty(snapshot).context("serializing proxy snapshot")?;
    // Atomic write (handles parent dirs too): a torn snapshot would make
    // disable/reconcile fall back to force-off instead of an exact restore.
    crate::primitives::write_file(&path, raw.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

pub fn load_snapshot() -> Result<Option<Vec<ServiceProxy>>> {
    let path = snapshot_path()?;
    match fs::read_to_string(&path) {
        Ok(raw) => {
            Ok(Some(serde_json::from_str(&raw).with_context(|| {
                format!("parsing {} as JSON", path.display())
            })?))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

pub fn clear_snapshot() -> Result<()> {
    let path = snapshot_path()?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

/// Privileged shell command that points all `services` at our loopback PAC
/// and turns their manual web/secure slots off, so only the PAC governs
/// routing. `-setautoproxyurl` also enables the auto-proxy state; the explicit
/// `-setautoproxystate on` keeps it unambiguous.
pub fn enable_pac_command(pac_url: &str, services: &[String]) -> String {
    let mut parts = Vec::new();
    let url = sh_quote(pac_url);
    for service in services {
        let q = sh_quote(service);
        parts.push(format!("{NETWORKSETUP} -setautoproxyurl {q} {url}"));
        parts.push(format!("{NETWORKSETUP} -setautoproxystate {q} on"));
        parts.push(format!("{NETWORKSETUP} -setwebproxystate {q} off"));
        parts.push(format!("{NETWORKSETUP} -setsecurewebproxystate {q} off"));
    }
    parts.join(" && ")
}

/// The user's pre-existing upstream proxy as a `host:port` string for the PAC
/// fallback, so non-Gate traffic keeps flowing through it while routing is on.
/// Prefers a service's secure (HTTPS) slot, then its web (HTTP) slot; skips
/// disabled and loopback slots ; returns
/// `None` when no service had an enabled manual proxy.
pub fn upstream_proxy(snapshot: &[ServiceProxy]) -> Option<String> {
    let directive = |p: &ProxySetting| {
        (p.enabled && !p.server.is_empty() && !is_loopback(&p.server)).then(|| {
            if p.port.is_empty() {
                p.server.clone()
            } else {
                format!("{}:{}", p.server, p.port)
            }
        })
    };
    snapshot
        .iter()
        .find_map(|s| directive(&s.secure).or_else(|| directive(&s.web)))
}

/// Privileged shell command that restores each service to its snapshot.
/// Enabling only turns the manual web/secure slots *off* (it drives routing
/// through the auto-proxy/PAC slot instead), so their saved server/port are
/// untouched; restoring rewrites the saved server whenever present and puts the
/// state back, which is a harmless no-op on the server but faithfully restores
/// the on/off state. The PAC slot *is* overwritten by enable , so its saved URL is rewritten when present and the state
/// restored - otherwise a stale loopback PAC would linger in System Settings.
pub fn restore_command(snapshot: &[ServiceProxy]) -> String {
    let mut parts = Vec::new();
    for s in snapshot {
        let q = sh_quote(&s.service);

        if !s.web.server.is_empty() {
            let port = if s.web.port.is_empty() {
                "80"
            } else {
                &s.web.port
            };
            parts.push(format!(
                "{NETWORKSETUP} -setwebproxy {q} {srv} {port}",
                srv = sh_quote(&s.web.server)
            ));
        }
        parts.push(format!(
            "{NETWORKSETUP} -setwebproxystate {q} {state}",
            state = if s.web.enabled && !s.web.server.is_empty() {
                "on"
            } else {
                "off"
            }
        ));

        if !s.secure.server.is_empty() {
            let port = if s.secure.port.is_empty() {
                "443"
            } else {
                &s.secure.port
            };
            parts.push(format!(
                "{NETWORKSETUP} -setsecurewebproxy {q} {srv} {port}",
                srv = sh_quote(&s.secure.server)
            ));
        }
        parts.push(format!(
            "{NETWORKSETUP} -setsecurewebproxystate {q} {state}",
            state = if s.secure.enabled && !s.secure.server.is_empty() {
                "on"
            } else {
                "off"
            }
        ));

        // Restore the PAC slot. enable overwrote the URL with our loopback PAC,
        // so rewrite the saved one whenever present, then restore the state -
        // mirrors the web/secure handling above.
        if !s.auto.url.is_empty() {
            parts.push(format!(
                "{NETWORKSETUP} -setautoproxyurl {q} {url}",
                url = sh_quote(&s.auto.url)
            ));
        }
        parts.push(format!(
            "{NETWORKSETUP} -setautoproxystate {q} {state}",
            state = if s.auto.enabled && !s.auto.url.is_empty() {
                "on"
            } else {
                "off"
            }
        ));
    }
    parts.join(" && ")
}

/// Restore-all command derived from a freshly enumerated service list, used
/// as a fail-safe when no snapshot is available - turns every service's
/// web + secure-web proxy off so a dead engine never strands traffic.
pub fn force_off_command(services: &[String]) -> String {
    let mut parts = Vec::new();
    for service in services {
        let q = sh_quote(service);
        parts.push(format!("{NETWORKSETUP} -setwebproxystate {q} off"));
        parts.push(format!("{NETWORKSETUP} -setsecurewebproxystate {q} off"));
        parts.push(format!("{NETWORKSETUP} -setautoproxystate {q} off"));
    }
    parts.join(" && ")
}

/// Apply a `networksetup` script. Changing proxy settings does not require
/// admin on a standard macOS account, so we run it unprivileged first - this
/// is what keeps disable / restore / reconcile promptless, so they can never
/// be canceled and strand the system's traffic. Only if the unprivileged run
/// is actually rejected do we fall back to an elevated run.
fn apply(script: &str) -> Result<()> {
    if script.is_empty() {
        return Ok(());
    }
    let status = Command::new("/bin/sh")
        .args(["-c", script])
        .status()
        .context("running networksetup")?;
    if status.success() {
        return Ok(());
    }
    crate::primitives::run_as_admin(script).context("running networksetup (elevated)")
}

/// Point every active service at the loopback PAC. Promptless on a standard
/// account.
pub fn enable_pac(pac_url: &str, services: &[String]) -> Result<()> {
    apply(&enable_pac_command(pac_url, services))
}

/// Restore every service to its snapshot. Promptless; safety-critical.
pub fn restore(snapshot: &[ServiceProxy]) -> Result<()> {
    apply(&restore_command(snapshot))
}

/// Turn every service's proxy off. Promptless fail-safe.
pub fn force_off(services: &[String]) -> Result<()> {
    apply(&force_off_command(services))
}

/// True if `server` is a loopback address - what our engine binds to. Used to
/// distinguish a stranded Gate proxy from a user's real (remote) proxy.
fn is_loopback(server: &str) -> bool {
    matches!(server, "127.0.0.1" | "::1" | "localhost" | "0.0.0.0")
}

/// The port of a loopback PAC URL (e.g. `http://127.0.0.1:8123/proxy.pac`), or
/// `None` when the URL is empty or not a loopback address we served. Used to
/// spot a stranded PAC pointing at a dead engine.
fn autoproxy_loopback_port(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let authority = rest.split('/').next()?;
    let (host, port) = authority.rsplit_once(':')?;
    is_loopback(host).then(|| port.to_string())
}

/// True if a proxy slot points at a loopback address with nothing listening on
/// its port - i.e. a dead engine that would strand traffic. Pure so it can be
/// unit-tested without touching the network.
fn slot_is_stranded(setting: &ProxySetting, port_alive: bool) -> bool {
    setting.enabled && is_loopback(&setting.server) && !port_alive
}

/// True if something is accepting connections on 127.0.0.1:`port` right now.
/// A refused/timed-out connect means the listener is gone.
fn loopback_port_alive(port: &str) -> bool {
    let Ok(p) = port.parse::<u16>() else {
        return false;
    };
    let addr = SocketAddr::from(([127, 0, 0, 1], p));
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// Startup fail-safe for when the snapshot can't save us: a hard kill or OS
/// shutdown bypasses the graceful-disable `Drop`, leaving every active
/// service's proxy pointed at our loopback engine after it's gone. On next
/// launch that strands all proxy-honoring apps with ERR_PROXY_CONNECTION_FAILED
/// while Gate itself shows "off". Turn off any enabled slot that points at a
/// loopback address with no listener; leave real (remote) proxies untouched.
/// Returns the services it cleared. Promptless.
pub fn clear_stranded_loopback() -> Result<Vec<String>> {
    use std::collections::HashMap;

    // Cache liveness per port so N services sharing one dead port probe once.
    let mut alive: HashMap<String, bool> = HashMap::new();
    let mut parts = Vec::new();
    let mut cleared = Vec::new();

    for service in active_services()? {
        let web = get_proxy("-getwebproxy", &service)?;
        let secure = get_proxy("-getsecurewebproxy", &service)?;
        let q = sh_quote(&service);
        let mut touched = false;

        for (off_flag, setting) in [
            ("-setwebproxystate", &web),
            ("-setsecurewebproxystate", &secure),
        ] {
            let port_alive = *alive
                .entry(setting.port.clone())
                .or_insert_with(|| loopback_port_alive(&setting.port));
            if slot_is_stranded(setting, port_alive) {
                parts.push(format!("{NETWORKSETUP} {off_flag} {q} off"));
                touched = true;
            }
        }

        // PAC slot: stranded if the auto-proxy URL points at a dead loopback PAC.
        let auto = get_autoproxy(&service)?;
        if auto.enabled {
            if let Some(port) = autoproxy_loopback_port(&auto.url) {
                let port_alive = *alive
                    .entry(port.clone())
                    .or_insert_with(|| loopback_port_alive(&port));
                if !port_alive {
                    parts.push(format!("{NETWORKSETUP} -setautoproxystate {q} off"));
                    touched = true;
                }
            }
        }

        if touched {
            cleared.push(service);
        }
    }

    apply(&parts.join(" && "))?;
    Ok(cleared)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Point `app_support_dir` (which `port_path` keys off) at a throwaway
    /// dir so the test never touches the real user data. Serialized because
    /// the override is process-global.
    fn with_temp_env<T>(f: impl FnOnce() -> T) -> T {
        static GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _lock = GUARD.lock().unwrap_or_else(|e| e.into_inner());

        let tmp = std::env::temp_dir().join(format!("gate-proxy-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        env::set_app_support_dir_for_tests(Some(tmp.clone()));
        let out = f();
        env::set_app_support_dir_for_tests(None);
        let _ = fs::remove_dir_all(&tmp);
        out
    }

    #[test]
    fn port_round_trips() {
        with_temp_env(|| {
            assert_eq!(load_port().unwrap(), None);
            save_port(40555).unwrap();
            assert_eq!(load_port().unwrap(), Some(40555));

            // The PAC port is a separate file so the two never clobber.
            assert_eq!(load_pac_port().unwrap(), None);
            save_pac_port(40556).unwrap();
            assert_eq!(load_pac_port().unwrap(), Some(40556));
            assert_eq!(load_port().unwrap(), Some(40555));
        });
    }

    fn slot(enabled: bool, server: &str, port: &str) -> ProxySetting {
        ProxySetting {
            enabled,
            server: server.into(),
            port: port.into(),
        }
    }

    #[test]
    fn loopback_servers_recognized() {
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("::1"));
        assert!(is_loopback("localhost"));
        assert!(!is_loopback("proxy.corp.example.com"));
        assert!(!is_loopback("10.0.0.5"));
    }

    #[test]
    fn stranded_only_when_enabled_loopback_and_dead() {
        // The bug we're fixing: enabled, loopback, nothing listening.
        assert!(slot_is_stranded(&slot(true, "127.0.0.1", "61722"), false));
        // Loopback but the engine is alive - leave it.
        assert!(!slot_is_stranded(&slot(true, "127.0.0.1", "61722"), true));
        // A real remote proxy that happens to be unreachable - never ours.
        assert!(!slot_is_stranded(&slot(true, "proxy.corp", "8080"), false));
        // Disabled slot - nothing to clear.
        assert!(!slot_is_stranded(&slot(false, "127.0.0.1", "61722"), false));
    }

    #[test]
    fn autoproxy_loopback_port_extracts_only_our_pac() {
        assert_eq!(
            autoproxy_loopback_port("http://127.0.0.1:8123/proxy.pac"),
            Some("8123".into())
        );
        // A remote PAC is the user's own - never ours.
        assert_eq!(
            autoproxy_loopback_port("http://pac.corp.example.com/proxy.pac"),
            None
        );
        assert_eq!(autoproxy_loopback_port(""), None);
    }

    #[test]
    fn upstream_proxy_prefers_secure_and_skips_disabled_and_loopback() {
        let svc = |web: ProxySetting, secure: ProxySetting| ServiceProxy {
            service: "Wi-Fi".into(),
            web,
            secure,
            auto: AutoProxy::default(),
        };
        // Secure wins over web.
        let snap = vec![svc(
            slot(true, "web.corp", "80"),
            slot(true, "secure.corp", "443"),
        )];
        assert_eq!(upstream_proxy(&snap).as_deref(), Some("secure.corp:443"));
        // Falls back to web when secure is off.
        let snap = vec![svc(
            slot(true, "web.corp", "8080"),
            slot(false, "secure.corp", "443"),
        )];
        assert_eq!(upstream_proxy(&snap).as_deref(), Some("web.corp:8080"));
        // A stranded loopback slot is never treated as an upstream.
        let snap = vec![svc(slot(false, "", ""), slot(true, "127.0.0.1", "61722"))];
        assert_eq!(upstream_proxy(&snap), None);
    }
}
