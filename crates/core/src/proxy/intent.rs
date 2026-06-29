//! Persisted "route through Gate" intent: the user's last explicit choice,
//! kept separate from the live proxy state and from the provider
//! restore-snapshot. Set true when the user enables routing and false when
//! they disable it; read on launch to decide whether to re-enable routing
//! after a machine restart.
//!
//! Deliberately *not* touched by the exit-time `manager().disable()` - that
//! path reverts the system proxy directly, never through `proxy_disable`, so
//! a clean quit leaves the intent intact and the next launch restores it.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Intent {
    enabled: bool,
}

fn intent_path() -> Result<PathBuf> {
    Ok(crate::env::app_support_dir()?
        .join("proxy")
        .join("routing-intent.json"))
}

/// Record whether the user wants traffic routed through Gate. The caller
/// treats a write failure as non-fatal (routing still toggles), so this only
/// affects whether the choice survives the next restart.
pub fn set_intent(enabled: bool) -> Result<()> {
    let path = intent_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let raw = serde_json::to_string(&Intent { enabled }).context("serializing routing intent")?;
    fs::write(&path, raw).with_context(|| format!("writing {}", path.display()))
}

/// The user's last explicit routing choice, or `false` when none was ever
/// recorded (first run) or the file is missing/unreadable. Best-effort: a
/// corrupt or absent file reads as "off" so a bad file never auto-routes.
pub fn load_intent() -> bool {
    let Ok(path) = intent_path() else {
        return false;
    };
    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str::<Intent>(&raw)
            .map(|i| i.enabled)
            .unwrap_or(false),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The app-support override is process-global, so serialize the cases that
    // flip it and point it at a throwaway dir that never touches real config.
    fn with_temp_env<T>(f: impl FnOnce() -> T) -> T {
        static GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _lock = GUARD.lock().unwrap_or_else(|e| e.into_inner());

        let tmp = std::env::temp_dir().join(format!("gate-intent-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        crate::env::set_app_support_dir_for_tests(Some(tmp.clone()));
        let out = f();
        crate::env::set_app_support_dir_for_tests(None);
        let _ = fs::remove_dir_all(&tmp);
        out
    }

    #[test]
    fn load_defaults_to_false_when_absent() {
        with_temp_env(|| assert!(!load_intent()));
    }

    #[test]
    fn set_then_load_round_trips() {
        with_temp_env(|| {
            set_intent(true).unwrap();
            assert!(load_intent());
            set_intent(false).unwrap();
            assert!(!load_intent());
        });
    }
}
