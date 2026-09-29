//! When Gate last changed each tool's configuration file.
//!
//! The app's reopen check (`agent_needs_reopen` in `src-tauri`) asks whether a running tool started before the last change
//! to its configuration, because a tool reads that file once, at startup. It
//! used to take the answer from the file's mtime, and the mtime moves on any
//! edit to the file - including every edit the tool makes to itself. Claude
//! Code rewrites `~/.claude/settings.json` when the user approves a permission
//! or switches model, and each of those made every older `claude` process read
//! as stale, although nothing Gate routes by had moved.
//!
//! So Gate records its own changes here instead. Every write or removal of a
//! tool configuration goes through [`write`] or [`remove`], and a stamp is kept
//! only when the file's bytes actually changed: an unattended reconnect that
//! finds its values already in place records nothing. What is recorded is the
//! moment Gate's values in that file last changed, which is the moment a
//! process that predates it stopped being trustworthy.
//!
//! **Only Gate's values going in are recorded, never coming out.** A tool
//! still running after a disconnect keeps the address it loaded, and that
//! address is the forwarder's, which sends the traffic straight to the
//! provider once Gate stops routing the host. Nothing it loaded went bad, so a
//! revert is no reason to tell the user to reopen it. The writes and removals
//! of a disconnect still go through here, as [`Change::Reverted`] and
//! [`remove`], so the file handling stays one path.
//!
//! **A hand edit to a routing value is not recorded.** That is the price of
//! ignoring the tool's own edits, and the status check still reports it: it
//! compares the file with what Gate wrote, so a changed proxy reads as Not
//! protected on the row.
//!
//! Keyed by the path as displayed, which is the form `config_location` returns.
//!
//! **Not locked.** The app and the CLI can both connect tools, and two records
//! racing lose one stamp. That can only drop a reopen notice, never raise a
//! false one, which is the same failure a missing record already has.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

const FILE_NAME: &str = "config-changes.json";

/// Which way a write moves Gate's values in a tool configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Change {
    /// A connect: Gate's values going in. Recorded.
    Applied,
    /// A disconnect: Gate's values coming out. Not recorded; see the module
    /// docs.
    Reverted,
}

/// Write a tool configuration, recording an [`Change::Applied`] change when the
/// bytes differ from what is on disk. Identical bytes still go through
/// [`crate::primitives::write_file`], which leaves the content alone and only
/// corrects the mode.
pub(crate) fn write(path: &Path, bytes: &[u8], mode: u32, change: Change) -> Result<()> {
    let changed = fs::read(path).map_or(true, |current| current != bytes);
    crate::primitives::write_file(path, bytes, mode)?;
    if changed && change == Change::Applied {
        record(path);
    }
    Ok(())
}

/// Remove a tool configuration. Never recorded: removing the file is always a
/// disconnect taking Gate's values out.
pub(crate) fn remove(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(path).with_context(|| format!("removing {}", path.display()))
}

/// Unix seconds at which Gate last changed the file at `path`, or `None` when
/// it has no record of changing it.
pub fn changed_at(path: &Path) -> Option<u64> {
    load().ok()?.get(&key(path)).copied()
}

/// Best effort. The configuration write has already landed when this runs, so
/// failing the caller here would report a connect as failed while the tool is
/// routed. A missing stamp costs a reopen notice, and the log says why.
fn record(path: &Path) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let saved = load().and_then(|mut stamps| {
        stamps.insert(key(path), now);
        let body = serde_json::to_string_pretty(&stamps).context("serializing config changes")?;
        crate::primitives::write_file(&store_path()?, body.as_bytes(), 0o600)
    });
    if let Err(e) = saved {
        eprintln!(
            "[gate] could not record the change to {}: {e:#}",
            path.display()
        );
    }
}

fn load() -> Result<BTreeMap<String, u64>> {
    let path = store_path()?;
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    // An unreadable record starts over rather than failing every later write,
    // which would stop config changes raising a reopen notice for good. Losing
    // the old stamps costs the same as having none.
    Ok(serde_json::from_str(&raw).unwrap_or_else(|e| {
        eprintln!("[gate] discarding unparsable {}: {e}", path.display());
        BTreeMap::new()
    }))
}

fn store_path() -> Result<PathBuf> {
    Ok(crate::env::app_support_dir()?.join(FILE_NAME))
}

fn key(path: &Path) -> String {
    path.display().to_string()
}
