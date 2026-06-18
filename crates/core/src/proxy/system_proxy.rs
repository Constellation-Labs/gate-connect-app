//! macOS system HTTP/HTTPS proxy wiring via `networksetup`. Enabling points
//! every active network service's web + secure-web proxy at our loopback
//! engine; disabling restores the exact prior state from a snapshot (mirrors
//! the `previousEnv` restore pattern in [`crate::env`]). Reads are
//! non-privileged; only the set/restore step needs admin, so the manager
//! batches it with the CA-trust change into one prompt.

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

// ---------------------------------------------------------------------------
// Shell-rc env injection
//
// `networksetup` only steers apps that honor the macOS system proxy (GUI apps
// and the like). Command-line dev tools don't: Node-based CLIs (the Gemini
// CLI) and reqwest-based CLIs (Codex) take their proxy + CA settings from the
// *environment*, never from System Settings. macOS has no `/etc/environment`
// equivalent that reaches login shells, so — mirroring the Linux managed-block
// design — we write a delimited block into the user's shell startup files
// exporting `http(s)_proxy` (points CLIs at the engine) and
// `NODE_EXTRA_CA_CERTS` (so Node trusts the engine's minted leaf certs, since
// it ships its own CA bundle and ignores the system trust store). Enable
// writes the block; disable / reconcile strip it. Only affects *new* shells.
// ---------------------------------------------------------------------------

/// Delimiters bracketing the lines we own in each shell-rc file. Everything
/// between them (inclusive) is ours to add/replace/remove; everything else is
/// left untouched.
const SHELL_BLOCK_BEGIN: &str = "# >>> gate-connect proxy (managed) >>>";
const SHELL_BLOCK_END: &str = "# <<< gate-connect proxy (managed) <<<";

/// Shell startup files we manage. `~/.zshenv` is sourced for every zsh
/// invocation (zsh is the macOS default shell since 10.15), so it reaches CLIs
/// in any new shell; we create it if absent. `~/.bash_profile` is managed only
/// when it already exists — we don't create bash files for a zsh user.
fn shell_rc_files() -> Result<Vec<PathBuf>> {
    let home = env::home()?;
    let mut files = vec![home.join(".zshenv")];
    let bash = home.join(".bash_profile");
    if bash.exists() {
        files.push(bash);
    }
    Ok(files)
}

/// Path to our CA cert, mirrored from [`super::ca`] — used for
/// `NODE_EXTRA_CA_CERTS` so Node CLIs trust the engine's leaf certs.
fn ca_cert_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join("proxy").join("ca-cert.pem"))
}

/// Return `content` with our managed block removed. Lines outside the
/// delimiters are preserved verbatim.
fn strip_shell_block(content: &str) -> String {
    let mut out = Vec::new();
    let mut inside = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == SHELL_BLOCK_BEGIN {
            inside = true;
            continue;
        }
        if trimmed == SHELL_BLOCK_END {
            inside = false;
            continue;
        }
        if !inside {
            out.push(line);
        }
    }
    let mut joined = out.join("\n");
    // Collapse trailing whitespace/newlines left after removal, then restore a
    // single trailing newline if there's any content.
    while joined.ends_with('\n') || joined.ends_with(' ') {
        joined.pop();
    }
    if !joined.is_empty() {
        joined.push('\n');
    }
    joined
}

/// Build the managed block exporting proxy + CA env for `127.0.0.1:port`.
/// Values are shell-quoted because the CA path lives under
/// `~/Library/Application Support/…` (contains spaces).
fn build_shell_block(port: u16) -> Result<String> {
    let endpoint = format!("http://127.0.0.1:{port}");
    let no_proxy = "localhost,127.0.0.1,::1";
    let ca = ca_cert_path()?;
    Ok(format!(
        "{SHELL_BLOCK_BEGIN}\n\
         export http_proxy={ep}\n\
         export https_proxy={ep}\n\
         export HTTP_PROXY={ep}\n\
         export HTTPS_PROXY={ep}\n\
         export no_proxy={np}\n\
         export NO_PROXY={np}\n\
         export NODE_EXTRA_CA_CERTS={ca}\n\
         {SHELL_BLOCK_END}\n",
        ep = sh_quote(&endpoint),
        np = sh_quote(no_proxy),
        ca = sh_quote(&ca.display().to_string()),
    ))
}

/// Write our managed block into every managed shell-rc file: strip any prior
/// block, then append the fresh one. Non-privileged (the user's own dotfiles).
fn write_shell_block(port: u16) -> Result<()> {
    let block = build_shell_block(port)?;
    for path in shell_rc_files()? {
        let existing = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let stripped = strip_shell_block(&existing);
        fs::write(&path, format!("{stripped}{block}"))
            .with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// Strip our managed block from every managed shell-rc file that has one.
/// Idempotent; safe on every revert path. If stripping empties `~/.zshenv` —
/// the file we create when absent — remove it rather than leave an empty
/// dotfile behind; other managed files (`~/.bash_profile`) pre-existed, so they
/// are kept even when our block was their only content.
fn strip_shell_blocks() -> Result<()> {
    let zshenv = env::home()?.join(".zshenv");
    for path in shell_rc_files()? {
        let existing = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        if !existing.contains(SHELL_BLOCK_BEGIN) {
            continue;
        }
        let stripped = strip_shell_block(&existing);
        if stripped.is_empty() && path == zshenv {
            fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
            continue;
        }
        fs::write(&path, stripped).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// Privileged shell command that points all `services` at our loopback
/// engine (both HTTP and HTTPS proxy slots).
pub fn enable_command(port: u16, services: &[String]) -> String {
    let mut parts = Vec::new();
    for service in services {
        let q = sh_quote(service);
        parts.push(format!("{NETWORKSETUP} -setwebproxy {q} 127.0.0.1 {port}"));
        parts.push(format!("{NETWORKSETUP} -setwebproxystate {q} on"));
        parts.push(format!(
            "{NETWORKSETUP} -setsecurewebproxy {q} 127.0.0.1 {port}"
        ));
        parts.push(format!("{NETWORKSETUP} -setsecurewebproxystate {q} on"));
    }
    parts.join(" && ")
}

/// Privileged shell command that restores each service to its snapshot.
/// A slot's saved server/port is re-written whenever the snapshot has one —
/// even when the slot was *off*: `enable_command` overwrote it with
/// `127.0.0.1:<our-port>`, and turning the state off alone would leave that
/// stale loopback address saved in System Settings (common case: a corp
/// proxy kept configured but toggled off). The state is then restored to
/// what it was. Windows restores `ProxyServer` verbatim; this keeps the
/// platforms equivalent.
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

/// Point every active service at the loopback engine, and inject the proxy +
/// CA env into the user's shell-rc files so CLIs route too. Promptless on a
/// standard account. If the shell-rc write fails, strip any partial block so
/// new shells aren't left pointed at a port the caller is about to tear down.
pub fn enable(port: u16, services: &[String]) -> Result<()> {
    apply(&enable_command(port, services))?;
    if let Err(e) = write_shell_block(port) {
        let _ = strip_shell_blocks();
        return Err(e);
    }
    Ok(())
}

/// Restore every service to its snapshot and strip our shell-rc block.
/// Promptless; safety-critical. Both reverts are attempted even if one fails.
pub fn restore(snapshot: &[ServiceProxy]) -> Result<()> {
    let net = apply(&restore_command(snapshot));
    let shell = strip_shell_blocks();
    net.and(shell)
}

/// Turn every service's proxy off and strip our shell-rc block. Promptless
/// fail-safe. Both reverts are attempted even if one fails.
pub fn force_off(services: &[String]) -> Result<()> {
    let net = apply(&force_off_command(services));
    let shell = strip_shell_blocks();
    net.and(shell)
}

/// True if `server` is a loopback address — what our engine binds to. Used to
/// distinguish a stranded Gate proxy from a user's real (remote) proxy.
fn is_loopback(server: &str) -> bool {
    matches!(server, "127.0.0.1" | "::1" | "localhost" | "0.0.0.0")
}

/// True if a proxy slot points at a loopback address with nothing listening on
/// its port — i.e. a dead engine that would strand traffic. Pure so it can be
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
        if touched {
            cleared.push(service);
        }
    }

    apply(&parts.join(" && "))?;
    // Also strip any shell-rc block a hard kill left behind — the reconcile's
    // "no snapshot" path doesn't call restore/force_off, so this is the only
    // sweep that catches a block stranded at a dead port. Best-effort.
    let _ = strip_shell_blocks();
    Ok(cleared)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn shell_strip_removes_only_our_block() {
        let original = "export PATH=/usr/bin\n";
        let with_block = format!(
            "export PATH=/usr/bin\n{SHELL_BLOCK_BEGIN}\nexport https_proxy=http://127.0.0.1:9\n{SHELL_BLOCK_END}\n"
        );
        assert_eq!(strip_shell_block(&with_block), original);
    }

    #[test]
    fn shell_strip_is_noop_without_block() {
        let original = "export FOO=bar\nexport BAZ=qux\n";
        assert_eq!(strip_shell_block(original), original);
    }

    #[test]
    fn shell_block_exports_proxy_and_ca() {
        let block = build_shell_block(61722).unwrap();
        assert!(block.contains("export HTTPS_PROXY="));
        assert!(block.contains("http://127.0.0.1:61722"));
        assert!(block.contains("export NODE_EXTRA_CA_CERTS="));
        // CA path is single-quoted so the Application Support space survives.
        assert!(block.contains("ca-cert.pem'"));
        assert!(block.contains("export NO_PROXY="));
    }

    #[test]
    fn stranded_only_when_enabled_loopback_and_dead() {
        // The bug we're fixing: enabled, loopback, nothing listening.
        assert!(slot_is_stranded(&slot(true, "127.0.0.1", "61722"), false));
        // Loopback but the engine is alive — leave it.
        assert!(!slot_is_stranded(&slot(true, "127.0.0.1", "61722"), true));
        // A real remote proxy that happens to be unreachable — never ours.
        assert!(!slot_is_stranded(&slot(true, "proxy.corp", "8080"), false));
        // Disabled slot — nothing to clear.
        assert!(!slot_is_stranded(&slot(false, "127.0.0.1", "61722"), false));
    }
}
