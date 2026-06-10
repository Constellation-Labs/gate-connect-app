//! macOS system HTTP/HTTPS proxy wiring via `networksetup`. Enabling points
//! every active network service's web + secure-web proxy at our loopback
//! engine; disabling restores the exact prior state from a snapshot (mirrors
//! the `previousEnv` restore pattern in [`crate::env`]). Reads are
//! non-privileged; only the set/restore step needs admin, so the manager
//! batches it with the CA-trust change into one prompt.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

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

/// Snapshot of one network service's proxy configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceProxy {
    pub service: String,
    pub web: ProxySetting,
    pub secure: ProxySetting,
}

fn snapshot_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?
        .join("proxy")
        .join("system-proxy.snapshot.json"))
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

/// Read the current proxy config for every active service. Non-privileged.
pub fn snapshot() -> Result<Vec<ServiceProxy>> {
    let mut snapshot = Vec::new();
    for service in active_services()? {
        let web = get_proxy("-getwebproxy", &service)?;
        let secure = get_proxy("-getsecurewebproxy", &service)?;
        snapshot.push(ServiceProxy {
            service,
            web,
            secure,
        });
    }
    Ok(snapshot)
}

pub fn save_snapshot(snapshot: &[ServiceProxy]) -> Result<()> {
    let path = snapshot_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(snapshot).context("serializing proxy snapshot")?;
    fs::write(&path, raw).with_context(|| format!("writing {}", path.display()))
}

pub fn load_snapshot() -> Result<Option<Vec<ServiceProxy>>> {
    let path = snapshot_path()?;
    match fs::read_to_string(&path) {
        Ok(raw) => Ok(Some(
            serde_json::from_str(&raw)
                .with_context(|| format!("parsing {} as JSON", path.display()))?,
        )),
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

/// Privileged shell command that points all `services` at our loopback
/// engine (both HTTP and HTTPS proxy slots).
pub fn enable_command(port: u16, services: &[String]) -> String {
    let mut parts = Vec::new();
    for service in services {
        let q = sh_quote(service);
        parts.push(format!("{NETWORKSETUP} -setwebproxy {q} 127.0.0.1 {port}"));
        parts.push(format!("{NETWORKSETUP} -setwebproxystate {q} on"));
        parts.push(format!("{NETWORKSETUP} -setsecurewebproxy {q} 127.0.0.1 {port}"));
        parts.push(format!("{NETWORKSETUP} -setsecurewebproxystate {q} on"));
    }
    parts.join(" && ")
}

/// Privileged shell command that restores each service to its snapshot.
/// When a slot was on with a real server, that server/port is restored;
/// otherwise the slot is simply turned off.
pub fn restore_command(snapshot: &[ServiceProxy]) -> String {
    let mut parts = Vec::new();
    for s in snapshot {
        let q = sh_quote(&s.service);

        if s.web.enabled && !s.web.server.is_empty() {
            let port = if s.web.port.is_empty() { "80" } else { &s.web.port };
            parts.push(format!(
                "{NETWORKSETUP} -setwebproxy {q} {srv} {port}",
                srv = sh_quote(&s.web.server)
            ));
            parts.push(format!("{NETWORKSETUP} -setwebproxystate {q} on"));
        } else {
            parts.push(format!("{NETWORKSETUP} -setwebproxystate {q} off"));
        }

        if s.secure.enabled && !s.secure.server.is_empty() {
            let port = if s.secure.port.is_empty() {
                "443"
            } else {
                &s.secure.port
            };
            parts.push(format!(
                "{NETWORKSETUP} -setsecurewebproxy {q} {srv} {port}",
                srv = sh_quote(&s.secure.server)
            ));
            parts.push(format!("{NETWORKSETUP} -setsecurewebproxystate {q} on"));
        } else {
            parts.push(format!("{NETWORKSETUP} -setsecurewebproxystate {q} off"));
        }
    }
    parts.join(" && ")
}

/// Restore-all command derived from a freshly enumerated service list, used
/// as a fail-safe when no snapshot is available — turns every service's
/// web + secure-web proxy off so a dead engine never strands traffic.
pub fn force_off_command(services: &[String]) -> String {
    let mut parts = Vec::new();
    for service in services {
        let q = sh_quote(service);
        parts.push(format!("{NETWORKSETUP} -setwebproxystate {q} off"));
        parts.push(format!("{NETWORKSETUP} -setsecurewebproxystate {q} off"));
    }
    parts.join(" && ")
}

/// Apply a `networksetup` script. Changing proxy settings does not require
/// admin on a standard macOS account, so we run it unprivileged first — this
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

/// Point every active service at the loopback engine. Promptless on a
/// standard account.
pub fn enable(port: u16, services: &[String]) -> Result<()> {
    apply(&enable_command(port, services))
}

/// Restore every service to its snapshot. Promptless; safety-critical.
pub fn restore(snapshot: &[ServiceProxy]) -> Result<()> {
    apply(&restore_command(snapshot))
}

/// Turn every service's proxy off. Promptless fail-safe.
pub fn force_off(services: &[String]) -> Result<()> {
    apply(&force_off_command(services))
}
