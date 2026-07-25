//! Shared persistence for the engine's loopback ports. Each port is a bare
//! integer in its own file under the per-user support dir's `proxy/` folder,
//! so the engine can come back on the same address across app restarts (see
//! the platform `system_proxy` modules for why that matters on each OS).
//! Non-secret, written 0644 via `primitives::write_file` (temp + rename).
//! One implementation serves all three platforms; only the wrapper doc
//! comments (the per-OS rationale) live platform-side.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

fn path(name: &str) -> Result<PathBuf> {
    Ok(crate::env::app_support_dir()?.join("proxy").join(name))
}

/// The last port persisted under `name`, if any and still parseable.
/// A missing file reads as `None`; unparseable content degrades to `None`
/// rather than failing the caller.
pub(super) fn load(name: &str) -> Result<Option<u16>> {
    load_at(&path(name)?)
}

/// Persist `port` under `name` for reuse on the next run. Best-effort
/// durability; non-secret, so written 0644 (mode ignored on Windows).
pub(super) fn save(name: &str, port: u16) -> Result<()> {
    save_at(&path(name)?, port)
}

fn load_at(path: &Path) -> Result<Option<u16>> {
    match fs::read_to_string(path) {
        Ok(raw) => Ok(raw.trim().parse::<u16>().ok()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn save_at(path: &Path, port: u16) -> Result<()> {
    crate::primitives::write_file(path, port.to_string().as_bytes(), 0o644)
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unique paths per test so the round-trips exercise no process-global
    // state (unlike `app_support_dir`), keeping these safe under parallel
    // runs - same pattern as the autostart_optout tests.
    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("gate-port-persist-test-{}", std::process::id()))
            .join(name)
    }

    #[test]
    fn port_round_trips() {
        let path = temp_file("round-trip");
        let _ = fs::remove_file(&path);
        assert_eq!(load_at(&path).unwrap(), None);
        save_at(&path, 40555).unwrap();
        assert_eq!(load_at(&path).unwrap(), Some(40555));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn distinct_names_do_not_clobber() {
        // The engine and PAC ports persist under different names; writing one
        // must leave the other untouched.
        let a = temp_file("port-a");
        let b = temp_file("port-b");
        let _ = fs::remove_file(&a);
        let _ = fs::remove_file(&b);
        save_at(&a, 40555).unwrap();
        save_at(&b, 40556).unwrap();
        assert_eq!(load_at(&a).unwrap(), Some(40555));
        assert_eq!(load_at(&b).unwrap(), Some(40556));
        let _ = fs::remove_file(&a);
        let _ = fs::remove_file(&b);
    }

    #[test]
    fn unparseable_content_reads_as_none() {
        let path = temp_file("garbage");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not-a-port").unwrap();
        assert_eq!(load_at(&path).unwrap(), None);
        let _ = fs::remove_file(&path);
    }
}
