//! Linux system HTTP/HTTPS proxy wiring via a user-scoped systemd
//! `environment.d` drop-in (`~/.config/environment.d/gate-proxy.conf`). Enabling
//! writes `http_proxy`/`https_proxy` (+ upper-case aliases) pointing at our
//! loopback engine, a `no_proxy` that keeps loopback traffic off the proxy, and
//! `NODE_EXTRA_CA_CERTS` pointing at our CA so Node-based CLIs (e.g. Claude
//! Code) - which ship their own bundle and ignore the system trust store -
//! accept the engine's minted leaf certs. Disabling deletes the file again.
//!
//! Why `environment.d` and not `/etc/environment`:
//!
//! - **No root.** The drop-in lives in the user's home, so enable/disable are
//!   unprivileged - no `pkexec`/polkit prompt, and no all-or-nothing privileged
//!   write that strands the toggle when polkit is unavailable. (Trusting the CA
//!   still needs root; that's a separate, one-time step in [`super::ca`].)
//! - **Transient by ownership.** We own the whole file, so "off" is a plain
//!   delete and a stale drop-in never lingers root-owned in a shared file.
//!
//! `systemd --user` reads `environment.d` at login and applies it to the
//! graphical session, so the variables reach GUI apps started afterwards *and*
//! command-line shells spawned from the session. On its own that only affects
//! **new** login sessions, which would force a logout. To avoid that, enabling
//! also pushes the same variables into the *running* session via
//! `dbus-update-activation-environment --systemd`, which updates the D-Bus
//! activation environment and the `systemd --user` manager that modern desktops
//! use to launch apps - so a tool relaunched after enabling picks up the proxy
//! immediately, no logout. That push is best-effort: with no session bus, or on
//! a pure non-systemd session (rare on modern Ubuntu/GNOME), it's a no-op and
//! the drop-in still applies at the next login. Either way, already-running
//! processes keep their environment until relaunched - nothing can change that.
//!
//! Pairs with a *stable* engine port (persisted via [`load_port`]/[`save_port`]):
//! a session freezes the proxy pointer at login, so the engine must come back on
//! the same port across restarts or that frozen pointer dangles at a dead port.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::env;

/// Basename of our user-scoped systemd environment drop-in.
const DROPIN_NAME: &str = "gate-proxy.conf";

/// Path to our `environment.d` drop-in: `$XDG_CONFIG_HOME/environment.d/gate-proxy.conf`
/// (i.e. `~/.config/environment.d/gate-proxy.conf`).
fn dropin_path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("could not resolve user config directory")?
        .join("environment.d")
        .join(DROPIN_NAME))
}

/// Marker recorded on enable. The drop-in design needs no captured prior state
/// to revert (we just delete our file), so this only notes whether the drop-in
/// was already present when we looked - and, more importantly, its existence on
/// disk is what tells [`super::manager`] a previous session left the proxy on
/// (crash reconcile).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxySnapshot {
    /// Whether our drop-in was already present when we snapshotted (i.e. an
    /// earlier unclean session left it behind).
    pub block_present: bool,
}

fn snapshot_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?
        .join("proxy")
        .join("system-proxy.snapshot.json"))
}

/// The engine's chosen loopback port persists (via [`super::port_persist`])
/// so it can be reused across restarts (keeping a frozen session's proxy
/// pointer valid). No PAC port here: Linux wires env-var proxies straight at
/// the engine.
pub fn load_port() -> Result<Option<u16>> {
    super::port_persist::load("port")
}

/// Persist the engine port for reuse on the next run (see [`load_port`]).
pub fn save_port(port: u16) -> Result<()> {
    super::port_persist::save("port", port)
}

/// Cross-process lock serializing enable/disable, so the app and the CLI can't
/// interleave the snapshot / drop-in / port writes (see [`super::flock`]).
pub fn op_lock_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join("proxy").join("op.lock"))
}

/// Path to our CA cert, mirrored from [`super::ca`] - used for
/// `NODE_EXTRA_CA_CERTS` so Node CLIs trust the engine's leaf certs.
fn ca_cert_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join("proxy").join("ca-cert.pem"))
}

/// Whether our drop-in currently exists on disk.
fn dropin_present() -> Result<bool> {
    Ok(dropin_path()?.exists())
}

/// The proxy-related environment variables we manage, in a stable order.
/// Enabling sets them (drop-in + live push); disabling blanks them in the
/// running session. Single source of truth so the two paths can't drift.
const PROXY_VARS: [&str; 7] = [
    "http_proxy",
    "https_proxy",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "no_proxy",
    "NO_PROXY",
    "NODE_EXTRA_CA_CERTS",
];

/// The name/value pairs for an *enabled* proxy pointing at `127.0.0.1:port`,
/// keyed by [`PROXY_VARS`]. Consumed by both [`build_dropin`] and the live
/// session push in [`enable`].
fn proxy_env(port: u16) -> Result<Vec<(&'static str, String)>> {
    let endpoint = format!("http://127.0.0.1:{port}");
    let no_proxy = "localhost,127.0.0.1,::1".to_string();
    let ca = ca_cert_path()?.display().to_string();
    let values = [
        endpoint.clone(),
        endpoint.clone(),
        endpoint.clone(),
        endpoint,
        no_proxy.clone(),
        no_proxy,
        ca,
    ];
    Ok(PROXY_VARS.into_iter().zip(values).collect())
}

/// Build the drop-in body from name/value pairs. systemd `environment.d` parses
/// `KEY=VALUE` lines (not shell), so a value may contain spaces; we double-quote
/// the CA path anyway for clarity and to stay safe if a consumer ever sources it
/// more strictly.
fn build_dropin(assignments: &[(&'static str, String)]) -> String {
    let mut body =
        String::from("# Managed by Gate Connect - do not edit. Removed when the proxy is off.\n");
    for (key, value) in assignments {
        if *key == "NODE_EXTRA_CA_CERTS" {
            body.push_str(&format!("{key}=\"{value}\"\n"));
        } else {
            body.push_str(&format!("{key}={value}\n"));
        }
    }
    body
}

/// Push proxy variable assignments into the *running* login session so tools
/// launched (or relaunched) now pick them up without waiting for the next
/// login. `dbus-update-activation-environment --systemd` updates both the D-Bus
/// activation environment and the `systemd --user` manager that modern desktops
/// use to spawn apps. Best-effort: no session bus, or a desktop that doesn't
/// ship the tool, just means the `environment.d` drop-in applies at next login
/// instead. Already-running processes keep their old environment until
/// relaunched - nothing can change that.
fn push_to_session(assignments: &[(&'static str, String)]) {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        return; // no graphical session bus to update; drop-in covers next login
    }
    let mut cmd = std::process::Command::new("dbus-update-activation-environment");
    cmd.arg("--systemd");
    for (key, value) in assignments {
        cmd.arg(format!("{key}={value}"));
    }
    match cmd.output() {
        Ok(out) if out.status.success() => {}
        Ok(out) => eprintln!(
            "[gate] live proxy env push exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // tool absent; fine
        Err(e) => eprintln!("[gate] could not run dbus-update-activation-environment: {e}"),
    }
}

/// Note whether our drop-in is currently present. Non-privileged.
pub fn snapshot() -> Result<ProxySnapshot> {
    Ok(ProxySnapshot {
        block_present: dropin_present()?,
    })
}

pub fn save_snapshot(snapshot: &ProxySnapshot) -> Result<()> {
    let path = snapshot_path()?;
    let raw = serde_json::to_string_pretty(snapshot).context("serializing proxy snapshot")?;
    // Atomic write (handles parent dirs too): a torn snapshot would make
    // disable/reconcile fall back to force-off instead of an exact restore.
    crate::primitives::write_file(&path, raw.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

pub fn load_snapshot() -> Result<Option<ProxySnapshot>> {
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

/// Point the system proxy at the loopback engine: write our `environment.d`
/// drop-in (applied to future login sessions) and push the same variables into
/// the running session so a tool relaunched now picks them up without a logout.
/// Unprivileged (user's home).
pub fn enable(port: u16) -> Result<()> {
    let assignments = proxy_env(port)?;
    let path = dropin_path()?;
    crate::primitives::write_file(&path, build_dropin(&assignments).as_bytes(), 0o644)
        .with_context(|| format!("writing {}", path.display()))?;
    push_to_session(&assignments);
    Ok(())
}

/// Delete our drop-in, restoring the user environment to its prior (proxy-free)
/// state. Restore and force-off are identical here - both just remove our file -
/// so `snapshot` is unused. Unprivileged.
pub fn restore(_snapshot: &ProxySnapshot) -> Result<()> {
    force_off()
}

/// Remove our drop-in. Fail-safe used when no snapshot is available, so a dead
/// engine never strands new shells at an unreachable proxy. Unprivileged.
pub fn force_off() -> Result<()> {
    let path = dropin_path()?;
    let result = match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    };
    // Blank the vars in the running session too, so tools launched after turning
    // the proxy off stop routing through a possibly-dead engine without waiting
    // for the next login. (Already-running processes keep them until relaunched.)
    let cleared: Vec<(&'static str, String)> = PROXY_VARS
        .into_iter()
        .map(|key| (key, String::new()))
        .collect();
    push_to_session(&cleared);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_temp_env<T>(f: impl FnOnce() -> T) -> T {
        // These tests mutate process-global state (XDG_CONFIG_HOME and the
        // app_support_dir test override) and share a single temp dir, so they
        // must not run concurrently: otherwise one test's teardown
        // `remove_dir_all` races another's writes and the atomic rename fails
        // with ENOENT. Serialize them. (`unwrap_or_else` swallows a poisoned
        // lock from an earlier panicking test so the rest still run.)
        static GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _lock = GUARD.lock().unwrap_or_else(|e| e.into_inner());

        // `dropin_path` keys off `dirs::config_dir()` (XDG_CONFIG_HOME), and the
        // snapshot/port paths off `app_support_dir`; point both at a throwaway
        // dir so the test never touches the real user config.
        let tmp = std::env::temp_dir().join(format!("gate-proxy-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        std::env::set_var("XDG_CONFIG_HOME", tmp.join("config"));
        env::set_app_support_dir_for_tests(Some(tmp.join("data")));
        let out = f();
        env::set_app_support_dir_for_tests(None);
        let _ = fs::remove_dir_all(&tmp);
        out
    }

    #[test]
    fn enable_writes_dropin_then_force_off_removes_it() {
        with_temp_env(|| {
            assert!(!dropin_present().unwrap());
            enable(41234).unwrap();
            assert!(dropin_present().unwrap());
            let body = fs::read_to_string(dropin_path().unwrap()).unwrap();
            assert!(body.contains("http_proxy=http://127.0.0.1:41234"));
            assert!(body.contains("HTTPS_PROXY=http://127.0.0.1:41234"));
            // CA path is double-quoted so an embedded space is safe.
            assert!(body.contains("NODE_EXTRA_CA_CERTS=\""));
            force_off().unwrap();
            assert!(!dropin_present().unwrap());
        });
    }

    #[test]
    fn force_off_is_noop_without_dropin() {
        with_temp_env(|| {
            assert!(force_off().is_ok());
        });
    }
}
