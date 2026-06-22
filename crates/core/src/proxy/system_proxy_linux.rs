//! Linux system HTTP/HTTPS proxy wiring via a user-scoped systemd
//! `environment.d` drop-in (`~/.config/environment.d/gate-proxy.conf`). Enabling
//! writes `http_proxy`/`https_proxy` (+ upper-case aliases) pointing at our
//! loopback engine, a `no_proxy` that keeps loopback traffic off the proxy, and
//! `NODE_EXTRA_CA_CERTS` pointing at our CA so Node-based CLIs (e.g. Claude
//! Code) — which ship their own bundle and ignore the system trust store —
//! accept the engine's minted leaf certs. Disabling deletes the file again.
//!
//! Why `environment.d` and not `/etc/environment`:
//!
//! - **No root.** The drop-in lives in the user's home, so enable/disable are
//!   unprivileged — no `pkexec`/polkit prompt, and no all-or-nothing privileged
//!   write that strands the toggle when polkit is unavailable. (Trusting the CA
//!   still needs root; that's a separate, one-time step in [`super::ca`].)
//! - **Transient by ownership.** We own the whole file, so "off" is a plain
//!   delete and a stale drop-in never lingers root-owned in a shared file.
//!
//! `systemd --user` reads `environment.d` at login and applies it to the
//! graphical session, so the variables reach GUI apps started afterwards *and*
//! command-line shells spawned from the session. Like `/etc/environment` before
//! it, this only affects **new** sessions — already-running shells keep their
//! environment until restarted. Known limitation: pure non-systemd sessions
//! (rare on modern Ubuntu/GNOME) don't read `environment.d`.
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
/// was already present when we looked — and, more importantly, its existence on
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

/// Path where we persist the engine's chosen loopback port so it can be reused
/// across restarts (keeping a frozen session's proxy pointer valid).
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

/// Path to our CA cert, mirrored from [`super::ca`] — used for
/// `NODE_EXTRA_CA_CERTS` so Node CLIs trust the engine's leaf certs.
fn ca_cert_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join("proxy").join("ca-cert.pem"))
}

/// Whether our drop-in currently exists on disk.
fn dropin_present() -> Result<bool> {
    Ok(dropin_path()?.exists())
}

/// Build the drop-in contents pointing at `127.0.0.1:port`. systemd
/// `environment.d` parses `KEY=VALUE` lines (not shell), so a value may contain
/// spaces; we double-quote the CA path anyway for clarity and to stay safe if a
/// consumer ever sources it more strictly.
fn build_dropin(port: u16) -> Result<String> {
    let endpoint = format!("http://127.0.0.1:{port}");
    let no_proxy = "localhost,127.0.0.1,::1";
    let ca = ca_cert_path()?;
    Ok(format!(
        "# Managed by Gate Connect — do not edit. Removed when the proxy is off.\n\
         http_proxy={endpoint}\n\
         https_proxy={endpoint}\n\
         HTTP_PROXY={endpoint}\n\
         HTTPS_PROXY={endpoint}\n\
         no_proxy={no_proxy}\n\
         NO_PROXY={no_proxy}\n\
         NODE_EXTRA_CA_CERTS=\"{ca}\"\n",
        ca = ca.display(),
    ))
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

/// Point the system proxy at the loopback engine by writing our drop-in.
/// Unprivileged (user's home). Only affects new sessions.
pub fn enable(port: u16) -> Result<()> {
    let path = dropin_path()?;
    crate::primitives::write_file(&path, build_dropin(port)?.as_bytes(), 0o644)
        .with_context(|| format!("writing {}", path.display()))
}

/// Delete our drop-in, restoring the user environment to its prior (proxy-free)
/// state. Restore and force-off are identical here — both just remove our file —
/// so `snapshot` is unused. Unprivileged.
pub fn restore(_snapshot: &ProxySnapshot) -> Result<()> {
    force_off()
}

/// Remove our drop-in. Fail-safe used when no snapshot is available, so a dead
/// engine never strands new shells at an unreachable proxy. Unprivileged.
pub fn force_off() -> Result<()> {
    let path = dropin_path()?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_temp_env<T>(f: impl FnOnce() -> T) -> T {
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

    #[test]
    fn port_round_trips() {
        with_temp_env(|| {
            assert_eq!(load_port().unwrap(), None);
            save_port(40555).unwrap();
            assert_eq!(load_port().unwrap(), Some(40555));
        });
    }
}
