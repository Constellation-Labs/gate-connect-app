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
use std::path::{Path, PathBuf};

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
    write_intent(&intent_path()?, enabled)
}

/// The user's last explicit routing choice, or `false` when none was ever
/// recorded (first run) or the file is missing/unreadable. Best-effort: a
/// corrupt or absent file reads as "off" so a bad file never auto-routes.
pub fn load_intent() -> bool {
    intent_path().map(|p| read_intent(&p)).unwrap_or(false)
}

fn write_intent(path: &Path, enabled: bool) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let raw = serde_json::to_string(&Intent { enabled }).context("serializing routing intent")?;
    fs::write(path, raw).with_context(|| format!("writing {}", path.display()))
}

fn read_intent(path: &Path) -> bool {
    match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str::<Intent>(&raw)
            .map(|i| i.enabled)
            .unwrap_or(false),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A unique path per test so the file round-trip exercises no process-global
    // state (unlike `app_support_dir`), keeping these safe under parallel runs.
    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("gate-intent-test-{}-{name}", std::process::id()))
            .join("routing-intent.json")
    }

    #[test]
    fn read_defaults_to_false_when_absent() {
        let path = temp_file("absent");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        assert!(!read_intent(&path));
    }

    #[test]
    fn write_then_read_round_trips() {
        let path = temp_file("round-trip");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        write_intent(&path, true).unwrap();
        assert!(read_intent(&path));
        write_intent(&path, false).unwrap();
        assert!(!read_intent(&path));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
