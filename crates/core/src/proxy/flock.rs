//! Minimal advisory file locking (Linux) for the proxy subsystem.
//!
//! Two uses:
//! - **enable/disable op-lock** (`app_support/proxy/op.lock`): serializes the
//!   snapshot / drop-in / port writes across processes, so the app and the CLI
//!   can't interleave them.
//! - **daemon singleton** (`$XDG_RUNTIME_DIR/gate-connect/proxyd.lock`): held
//!   for the daemon's whole life, so a second daemon defers instead of racing
//!   the socket bind.
//!
//! Backed by `flock(2)`: the lock is tied to the open file description and is
//! released automatically when the process exits — even on a crash — so there's
//! no stale-lock cleanup to get wrong.

use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::path::Path;

use anyhow::{Context, Result};

/// Holds an advisory `flock` for its lifetime; the lock releases when this is
/// dropped (the fd closes) or the process exits.
pub struct FileLock {
    _file: std::fs::File,
}

impl FileLock {
    /// Exclusively lock `path` (created if absent). When `blocking`, waits for
    /// the lock; otherwise returns `Ok(None)` if another holder has it.
    pub fn acquire(path: &Path, blocking: bool) -> Result<Option<FileLock>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false) // a lockfile's contents are irrelevant; never clobber
            .open(path)
            .with_context(|| format!("opening lockfile {}", path.display()))?;
        let mut op = libc::LOCK_EX;
        if !blocking {
            op |= libc::LOCK_NB;
        }
        // SAFETY: `file` owns a valid fd for the duration of the call.
        let rc = unsafe { libc::flock(file.as_raw_fd(), op) };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            // EWOULDBLOCK (== EAGAIN on Linux) means "held by someone else".
            if !blocking && err.raw_os_error() == Some(libc::EWOULDBLOCK) {
                return Ok(None);
            }
            return Err(err).with_context(|| format!("locking {}", path.display()));
        }
        Ok(Some(FileLock { _file: file }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_nonblocking_acquire_is_denied_until_first_drops() {
        let path = std::env::temp_dir().join(format!("gate-flock-test-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let first = FileLock::acquire(&path, false).unwrap();
        assert!(first.is_some(), "first acquire should succeed");

        // A separate open file description on the same path conflicts.
        let second = FileLock::acquire(&path, false).unwrap();
        assert!(
            second.is_none(),
            "second non-blocking acquire should be denied"
        );

        drop(first);
        let third = FileLock::acquire(&path, false).unwrap();
        assert!(
            third.is_some(),
            "acquire should succeed once the first is dropped"
        );

        drop(third);
        let _ = std::fs::remove_file(&path);
    }
}
