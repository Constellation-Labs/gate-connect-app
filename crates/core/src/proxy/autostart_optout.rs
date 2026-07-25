//! Launch-at-login safety net: a marker recording that the OS login item is
//! registered even though the user's launch-at-login choice is OFF, kept
//! only so a crash while routing is on can self-heal. Without it, a crash
//! leaves the system proxy pointed at a dead port with nothing relaunching
//! at boot to run the startup self-heal. The marker defers the
//! deregistration to the next point where the system proxy is known safe:
//! routing turned off, a clean quit (the exit handler reverts the proxy
//! first), or the next login-item launch (whose startup reconcile reverts
//! any stale proxy before the app deregisters and exits). While the marker
//! is pending, the launch-at-login status reported to the UI is the user's
//! choice (off), not the OS registration.
//!
//! Two transitions arm the same marker, with the same meaning:
//! - Deferred opt-out: the user turns launch-at-login off while routing is
//!   on ([`record_disable`]); the existing registration stays.
//! - Safety-net registration: the user turns routing on while
//!   launch-at-login is already off ([`record_safety_net_registration`]);
//!   the shell registers the login item just for the crash window.
//!
//! Only the marker persistence and the defer-vs-deregister decision live
//! here; the OS login item itself is owned by the desktop shell (the
//! tauri-plugin-autostart calls in `src-tauri`).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

fn marker_path() -> Result<PathBuf> {
    // Root of the support dir, next to the `autostart-defaulted` marker the
    // desktop shell writes for the first-run default.
    Ok(crate::env::app_support_dir()?.join("autostart-pending-disable"))
}

/// Whether a deferred opt-out is waiting for its deregistration to complete.
/// Best-effort: an unresolvable support dir reads as "not pending", so it
/// can never block a direct deregistration.
pub fn pending() -> bool {
    marker_path().map(|p| pending_at(&p)).unwrap_or(false)
}

/// Set or clear the marker. Clearing an absent marker is a no-op, so
/// completion paths can call this unconditionally.
pub fn set_pending(pending: bool) -> Result<()> {
    set_pending_at(&marker_path()?, pending)
}

/// Record the user turning launch-at-login off. With routing on, the
/// opt-out defers: the marker arms and the login item must stay registered.
/// Otherwise any stale marker clears. Returns whether the caller should
/// deregister the login item now.
///
/// "Routing on" is judged from the persisted intent OR the live snapshot
/// marker ([`crate::proxy::engine_likely_running`]): `proxy_enable` persists
/// the intent best-effort *after* routing is already up, so a failed intent
/// write must not let this opt-out deregister the safety net while the
/// system proxy is routed - a later crash would strand it with nothing
/// relaunching at boot to self-heal.
pub fn record_disable() -> Result<bool> {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    let routing_on = crate::proxy::intent::load_intent() || crate::proxy::engine_likely_running();
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    let routing_on = crate::proxy::intent::load_intent();
    record_disable_at(&marker_path()?, routing_on)
}

fn record_disable_at(path: &Path, routing_intent: bool) -> Result<bool> {
    set_pending_at(path, routing_intent)?;
    Ok(!routing_intent)
}

/// Record a safety-net login-item registration: routing just turned on while
/// launch-at-login was off, so the shell registers the login item purely for
/// the crash window. Arm the marker so the registration is reported as
/// pending (not the user's choice) and deregistered at the next safe point.
/// Idempotent: re-arming an already-pending marker is a no-op.
pub fn record_safety_net_registration() -> Result<()> {
    record_safety_net_registration_at(&marker_path()?)
}

fn record_safety_net_registration_at(path: &Path) -> Result<()> {
    set_pending_at(path, true)
}

fn pending_at(path: &Path) -> bool {
    path.exists()
}

fn set_pending_at(path: &Path, pending: bool) -> Result<()> {
    if pending {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        fs::write(path, b"1").with_context(|| format!("writing {}", path.display()))
    } else {
        match fs::remove_file(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => r.with_context(|| format!("removing {}", path.display())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A unique path per test so the marker round-trip exercises no
    // process-global state (unlike `app_support_dir`), keeping these safe
    // under parallel runs.
    fn temp_marker(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "gate-autostart-optout-test-{}-{name}",
                std::process::id()
            ))
            .join("autostart-pending-disable")
    }

    #[test]
    fn absent_marker_reads_not_pending() {
        let path = temp_marker("absent");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        assert!(!pending_at(&path));
    }

    #[test]
    fn set_then_clear_round_trips() {
        let path = temp_marker("round-trip");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        set_pending_at(&path, true).unwrap();
        assert!(pending_at(&path));
        set_pending_at(&path, false).unwrap();
        assert!(!pending_at(&path));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn clearing_absent_marker_is_a_no_op() {
        let path = temp_marker("clear-absent");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        set_pending_at(&path, false).unwrap();
        assert!(!pending_at(&path));
    }

    #[test]
    fn disable_with_routing_on_defers_and_arms_marker() {
        let path = temp_marker("defer");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        let deregister_now = record_disable_at(&path, true).unwrap();
        assert!(!deregister_now);
        assert!(pending_at(&path));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn disable_with_routing_off_deregisters_now_and_clears_stale_marker() {
        let path = temp_marker("immediate");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        set_pending_at(&path, true).unwrap();
        let deregister_now = record_disable_at(&path, false).unwrap();
        assert!(deregister_now);
        assert!(!pending_at(&path));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn repeated_deferred_disable_stays_pending() {
        let path = temp_marker("repeat");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        assert!(!record_disable_at(&path, true).unwrap());
        assert!(!record_disable_at(&path, true).unwrap());
        assert!(pending_at(&path));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn safety_net_registration_arms_marker() {
        let path = temp_marker("safety-net");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        record_safety_net_registration_at(&path).unwrap();
        assert!(pending_at(&path));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn safety_net_registration_is_idempotent() {
        let path = temp_marker("safety-net-repeat");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        record_safety_net_registration_at(&path).unwrap();
        record_safety_net_registration_at(&path).unwrap();
        assert!(pending_at(&path));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn disable_with_routing_on_keeps_safety_net_marker_pending() {
        let path = temp_marker("safety-net-then-disable-on");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        record_safety_net_registration_at(&path).unwrap();
        assert!(!record_disable_at(&path, true).unwrap());
        assert!(pending_at(&path));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn disable_with_routing_off_clears_safety_net_marker() {
        let path = temp_marker("safety-net-then-disable-off");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        record_safety_net_registration_at(&path).unwrap();
        assert!(record_disable_at(&path, false).unwrap());
        assert!(!pending_at(&path));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
