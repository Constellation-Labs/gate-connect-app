//! The backend half of the copy-pasteable diagnostics report.
//!
//! Everything here is a *read*, and every read is best-effort: a field that
//! can't be resolved comes back `None` rather than failing the call. A report
//! that refuses to render because one probe errored is worth less than a
//! report with a hole in it, and the holes are themselves diagnostic.
//!
//! What is deliberately absent: the Gate key, its recorded prefix, the OAuth
//! token bundle, and every upstream provider credential. The report exists to
//! be pasted into a support thread, so the rule is that nothing in it is a
//! secret even once it has left this machine. Identity (email, org) comes from
//! the frontend, which already has it on screen.
//!
//! The rest of the report - account, providers, tools, proxy domains - is
//! composed frontend-side from state the popover already holds, so the paste
//! matches the screen the user is looking at rather than a second, racing read.

use serde::Serialize;

/// Backend-only facts about this install. Serialized to the webview as-is.
#[derive(Debug, Clone, Serialize)]
pub struct Diagnostics {
    /// OS marketing name and version, as the platform's own tooling reports
    /// it ("Ubuntu 25.10", "macOS 15.3 (24D60)"). Distro and version decide
    /// entire classes of bug here - the AppImage's Wayland loader problem is
    /// an Ubuntu-version question, and CA trust paths differ per distro.
    pub os_name: String,
    /// Kernel release, Linux only ("" elsewhere): the other half of the
    /// AppImage/Wayland question.
    pub os_kernel: String,
    /// CPU architecture. An x86_64 build under Rosetta and a native arm64
    /// build fail differently.
    pub arch: String,
    /// Where account.json, the proxy snapshot and the persisted ports live -
    /// so support can name an exact path instead of guessing per platform.
    pub data_dir: Option<String>,
    /// The local CA's public cert, and whether it is actually on disk. A
    /// `ca_trusted` of true with no cert file is a real (and otherwise
    /// invisible) state: the OS trust store holds a cert we can no longer
    /// mint leaves from.
    pub ca_cert_path: Option<String>,
    pub ca_cert_present: bool,
    /// Linux only: whether every per-user NSS database found holds our current
    /// CA. Chromium-based browsers read that store and never the system one, so
    /// `Some(false)` next to a `ca_trusted` of true is exactly the "Firefox
    /// works, Chrome doesn't" report, and it is invisible from the popover.
    /// `None` where the question does not apply: not Linux, or no Chromium
    /// browser has ever run for this user.
    pub ca_nss_trusted: Option<bool>,
    /// The persisted "routing should be on" intent. Compared against the live
    /// `running` flag it answers the commonest report we get: routing was on
    /// yesterday and the app came back with it off.
    pub routing_intent: bool,
    /// The engine's forward-proxy address from the persisted port, whether or
    /// not the engine is up. A tool configured against a *different* loopback
    /// port is pointing at a dead session.
    pub persisted_engine_proxy_url: Option<String>,
    /// Loopback base URL the CLI integrations write into their configs.
    pub relay_base_url: Option<String>,
    /// The proxy URL currently in the user's environment, read back from the
    /// OS rather than from our own record - so this reports what is true, not
    /// what we last tried to write. Disagreement with the running port is the
    /// signature of a stale shell or a half-applied export.
    pub exported_proxy_url: Option<String>,
    /// The OS proxy setting, read back live: the channel that routes GUI apps
    /// (PAC on macOS/Windows, the `environment.d` drop-in on Linux), as
    /// opposed to the environment variables above that route CLI tools. The
    /// two disagreeing explains "my terminal routes but Claude Desktop
    /// doesn't", which is otherwise pure guesswork.
    pub system_proxy: Option<String>,
}

/// Take one snapshot. Cheap on Linux/Windows (file and registry reads); on
/// macOS the system-proxy readback shells out to `networksetup` once per
/// active network service, so this is for an explicit user action, not a poll.
pub fn collect() -> Diagnostics {
    let ca_cert_path = crate::proxy::ca_cert_path().ok();
    Diagnostics {
        os_name: os_name(),
        os_kernel: os_kernel(),
        arch: std::env::consts::ARCH.to_string(),
        data_dir: crate::env::app_support_dir()
            .ok()
            .map(|p| p.display().to_string()),
        ca_cert_present: ca_cert_path.as_ref().is_some_and(|p| p.exists()),
        ca_cert_path: ca_cert_path.map(|p| p.display().to_string()),
        ca_nss_trusted: ca_nss_trusted(),
        routing_intent: crate::proxy::intent::load_intent(),
        persisted_engine_proxy_url: crate::proxy::persisted_engine_proxy_url(),
        relay_base_url: crate::proxy::relay_base_url(),
        exported_proxy_url: crate::proxy::exported_proxy_url(),
        system_proxy: system_proxy_summary(),
    }
}

#[cfg(target_os = "linux")]
fn ca_nss_trusted() -> Option<bool> {
    crate::proxy::ca::nss_ca_trusted()
}

#[cfg(not(target_os = "linux"))]
fn ca_nss_trusted() -> Option<bool> {
    // macOS and Windows put user-added roots in the same store the browser
    // reads, so there is no second store here to disagree with the first.
    None
}

#[cfg(target_os = "linux")]
fn os_name() -> String {
    // PRETTY_NAME is the one field every distro fills in and the one a bug
    // report wants ("Ubuntu 25.10", "Fedora Linux 41 (Workstation Edition)").
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|body| {
            body.lines()
                .find_map(|line| line.strip_prefix("PRETTY_NAME="))
                .map(|value| value.trim().trim_matches('"').to_string())
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Linux".to_string())
}

#[cfg(target_os = "macos")]
fn os_name() -> String {
    let field = |arg: &str| {
        std::process::Command::new("/usr/bin/sw_vers")
            .arg(arg)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .unwrap_or_default()
    };
    let product = field("-productVersion");
    if product.is_empty() {
        return "macOS".to_string();
    }
    // The build number distinguishes the point releases Apple ships under one
    // marketing version, which is where TLS and trust-store behaviour moves.
    let build = field("-buildVersion");
    if build.is_empty() {
        format!("macOS {product}")
    } else {
        format!("macOS {product} ({build})")
    }
}

#[cfg(target_os = "windows")]
fn os_name() -> String {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
    else {
        return "Windows".to_string();
    };
    // ProductName still says "Windows 10" on Windows 11, so DisplayVersion
    // (24H2) and CurrentBuild carry the actual answer; print all three rather
    // than trying to correct Microsoft's string here.
    let product = key
        .get_value::<String, _>("ProductName")
        .unwrap_or_else(|_| "Windows".to_string());
    let display = key
        .get_value::<String, _>("DisplayVersion")
        .unwrap_or_default();
    let build = key
        .get_value::<String, _>("CurrentBuild")
        .unwrap_or_default();
    let mut name = product;
    if !display.is_empty() {
        name.push(' ');
        name.push_str(&display);
    }
    if !build.is_empty() {
        name.push_str(&format!(" (build {build})"));
    }
    name
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn os_name() -> String {
    std::env::consts::OS.to_string()
}

#[cfg(target_os = "linux")]
fn os_kernel() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|raw| raw.trim().to_string())
        .unwrap_or_default()
}

#[cfg(not(target_os = "linux"))]
fn os_kernel() -> String {
    // macOS and Windows both fold the kernel into the version string above.
    String::new()
}

/// One line describing what the OS proxy setting currently is. Phrased per
/// platform because the setting itself is a different object on each.
#[cfg(target_os = "macos")]
fn system_proxy_summary() -> Option<String> {
    let services = crate::proxy::system_proxy::snapshot().ok()?;
    let summary = services
        .iter()
        .map(|s| {
            if s.auto.enabled && !s.auto.url.is_empty() {
                format!("{}: PAC {}", s.service, s.auto.url)
            } else if s.secure.enabled {
                format!("{}: https {}:{}", s.service, s.secure.server, s.secure.port)
            } else {
                format!("{}: off", s.service)
            }
        })
        .collect::<Vec<_>>()
        .join("; ");
    Some(summary).filter(|s| !s.is_empty())
}

#[cfg(target_os = "windows")]
fn system_proxy_summary() -> Option<String> {
    let snapshot = crate::proxy::system_proxy::snapshot().ok()?;
    let mut parts = Vec::new();
    if !snapshot.auto_config_url.is_empty() {
        parts.push(format!("PAC {}", snapshot.auto_config_url));
    }
    if snapshot.enable != 0 && !snapshot.server.is_empty() {
        parts.push(format!("manual {}", snapshot.server));
    }
    if parts.is_empty() {
        parts.push("off".to_string());
    }
    Some(parts.join("; "))
}

#[cfg(target_os = "linux")]
fn system_proxy_summary() -> Option<String> {
    // The `environment.d` drop-in *is* the system proxy here - there is no
    // separate PAC channel to report.
    let present = crate::proxy::system_proxy::snapshot().ok()?.block_present;
    Some(
        if present {
            "environment.d drop-in present"
        } else {
            "no environment.d drop-in"
        }
        .to_string(),
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn system_proxy_summary() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The contract the whole module rests on: collecting never panics and
    /// never returns an error, whatever state the machine is in. The report is
    /// most needed on the machines where the most probes fail.
    #[test]
    fn collect_always_produces_a_report() {
        let d = collect();
        assert!(!d.os_name.is_empty());
        assert!(!d.arch.is_empty());
    }

    #[test]
    fn no_field_carries_a_credential() {
        // A structural guard, not a string search: the DTO is the whole
        // surface, so serializing it and looking for the shapes we never want
        // to ship catches a field added later without this file being reread.
        let json = serde_json::to_string(&collect()).expect("diagnostics serialize");
        assert!(!json.contains("sk-gw-"));
        assert!(!json.contains("Bearer "));
    }
}
