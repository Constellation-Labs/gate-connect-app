//! When Gate last changed each tool's configuration file.
//!
//! [`crate::reopen`] asks whether a running tool started before the last change
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
//! **A hand edit to a routing value is not recorded.** That is the price of
//! ignoring the tool's own edits, and the status check still reports it: it
//! compares the file with what Gate wrote, so a changed proxy reads as Not
//! protected on the row.
//!
//! **Two writers can lose a stamp.** The app and the CLI both write here, and a
//! stamp is a read, an insert and a write with no lock between them, so two
//! changes that land at once can keep only one. The one lost is a change a
//! running process missed with no reopen notice for it: the same "no evidence,
//! no claim" outcome as a missing record, accepted because overlapping writes
//! need a CLI connect racing the app's own pass.
//!
//! **A change that is later undone is not news to a process that predates
//! both.** Every quit takes Gate out of every tool's configuration and the
//! next launch puts the same values back, so a bare "last changed" stamp moved
//! twice across a relaunch and every tool session that outlived it read as
//! stale, although what it loaded and what the file says again are the same.
//! So each change also carries a fingerprint of the file's *routing view* -
//! the part a running process acted on, chosen per tool (Claude Code's `env`
//! block, Codex's document without its own trust and notice tables, the whole
//! file elsewhere) - and [`crate::reopen`] compares the view a process loaded
//! with the view now in place, not the times alone. A change whose view
//! equals the last one recorded adds nothing to that comparison and is not
//! kept.
//!
//! **Bounded.** Each path keeps its newest [`MAX_CHANGES`], and a path whose
//! newest change is older than [`MAX_AGE_SECS`] is dropped the next time
//! anything is recorded. A process older than every change a path still
//! keeps is judged stale, which is what the single stamp said about it too.
//!
//! Keyed by the path as displayed, which is the form `config_location` returns.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "config-changes.json";

/// Changes kept per path. A quit and the launch after it are two, so this
/// covers sixteen relaunches of Gate under one long-running tool session.
const MAX_CHANGES: usize = 32;

/// A path whose newest change is older than this is forgotten. Thirty days:
/// no tool session Gate judges runs that long, and the paths that get here
/// are mostly `.env` files of projects nobody routes any more.
const MAX_AGE_SECS: u64 = 30 * 24 * 60 * 60;

/// One change Gate made to a configuration file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    /// Unix seconds.
    pub at: u64,
    /// SHA-256 of the file's routing view after the change, hex. `None` on a
    /// change recorded before fingerprints existed, which matches nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
}

/// One path's record on disk: a list now, a bare timestamp before
/// fingerprints, read as a single change with no view.
#[derive(Deserialize)]
#[serde(untagged)]
enum Stored {
    Changes(Vec<Change>),
    Legacy(u64),
}

impl Stored {
    fn into_changes(self) -> Vec<Change> {
        match self {
            Stored::Changes(changes) => changes,
            Stored::Legacy(at) => vec![Change { at, view: None }],
        }
    }
}

/// Write a tool configuration, recording the change when the bytes differ from
/// what is on disk. The routing view is the whole file; a tool that edits its
/// own file goes through [`write_with_view`] instead. Identical bytes still go
/// through [`crate::primitives::write_file`], which leaves the content alone
/// and only corrects the mode.
pub(crate) fn write(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    write_with_view(path, bytes, mode, bytes)
}

/// [`write`], with `view` as the part of the file a running process acts on.
pub(crate) fn write_with_view(path: &Path, bytes: &[u8], mode: u32, view: &[u8]) -> Result<()> {
    let changed = fs::read(path).map_or(true, |current| current != bytes);
    crate::primitives::write_file(path, bytes, mode)?;
    if changed {
        record(path, view);
    }
    Ok(())
}

/// Remove a tool configuration, recording the change when there was one to
/// remove. An absent file's view is empty.
pub(crate) fn remove(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    record(path, b"");
    Ok(())
}

/// Unix seconds at which Gate last changed the file at `path`, or `None` when
/// it has no record of changing it.
pub fn changed_at(path: &Path) -> Option<u64> {
    history(path).last().map(|change| change.at)
}

/// Gate's changes to the file at `path`, oldest first. Empty when it has no
/// record of changing it.
pub fn history(path: &Path) -> Vec<Change> {
    load()
        .ok()
        .and_then(|mut stamps| stamps.remove(&key(path)))
        .unwrap_or_default()
}

/// Best effort. The configuration write has already landed when this runs, so
/// failing the caller here would report a connect as failed while the tool is
/// routed. A missing stamp costs a reopen notice, and the log says why.
fn record(path: &Path, view: &[u8]) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let saved = load().and_then(|mut stamps| {
        append(stamps.entry(key(path)).or_default(), now, fingerprint(view));
        stamps.retain(|_, changes| {
            changes
                .last()
                .is_some_and(|c| c.at.saturating_add(MAX_AGE_SECS) >= now)
        });
        let body = serde_json::to_string_pretty(&stamps).context("serializing config changes")?;
        crate::primitives::write_file(&store_path()?, body.as_bytes(), 0o600)
    });
    if let Err(e) = saved {
        crate::logging::failure(&format!(
            "could not record the change to {}: {e:#}",
            path.display()
        ));
    }
}

/// Add a change to one path's list, unless its view is the one already in
/// place, and keep the list to [`MAX_CHANGES`].
fn append(changes: &mut Vec<Change>, at: u64, view: String) {
    if changes
        .last()
        .is_some_and(|last| last.view.as_deref() == Some(view.as_str()))
    {
        return;
    }
    changes.push(Change {
        at,
        view: Some(view),
    });
    let excess = changes.len().saturating_sub(MAX_CHANGES);
    changes.drain(..excess);
}

fn fingerprint(view: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(view)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn load() -> Result<BTreeMap<String, Vec<Change>>> {
    let path = store_path()?;
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    // An unreadable record starts over rather than failing every later write,
    // which would stop config changes raising a reopen notice for good. Losing
    // the old stamps costs the same as having none.
    let stored: BTreeMap<String, Stored> = serde_json::from_str(&raw).unwrap_or_else(|e| {
        crate::logging::failure(&format!("discarding unparsable {}: {e}", path.display()));
        BTreeMap::new()
    });
    Ok(stored
        .into_iter()
        .map(|(path, stored)| (path, stored.into_changes()))
        .collect())
}

fn store_path() -> Result<PathBuf> {
    Ok(crate::env::app_support_dir()?.join(FILE_NAME))
}

fn key(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_already_in_place_is_not_recorded_again() {
        let mut changes = Vec::new();
        append(&mut changes, 10, "a".into());
        append(&mut changes, 20, "a".into());
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].at, 10);
    }

    #[test]
    fn a_view_put_back_is_recorded_as_a_new_change() {
        let mut changes = Vec::new();
        append(&mut changes, 10, "gate".into());
        append(&mut changes, 20, "own".into());
        append(&mut changes, 30, "gate".into());
        let at: Vec<u64> = changes.iter().map(|c| c.at).collect();
        assert_eq!(at, [10, 20, 30]);
    }

    #[test]
    fn a_path_keeps_only_its_newest_changes() {
        let mut changes = Vec::new();
        for at in 0..(MAX_CHANGES as u64 + 5) {
            append(&mut changes, at, at.to_string());
        }
        assert_eq!(changes.len(), MAX_CHANGES);
        assert_eq!(changes[0].at, 5);
        assert_eq!(changes.last().map(|c| c.at), Some(MAX_CHANGES as u64 + 4));
    }

    #[test]
    fn a_legacy_view_never_matches_a_new_one() {
        let mut changes = vec![Change { at: 10, view: None }];
        append(&mut changes, 20, "a".into());
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn a_bare_timestamp_reads_as_one_change_with_no_view() {
        let mut stored: BTreeMap<String, Stored> =
            serde_json::from_str(r#"{"/a": 7, "/b": [{"at": 9, "view": "x"}]}"#).unwrap();
        let mut read = |k: &str| stored.remove(k).unwrap().into_changes();
        assert_eq!(read("/a"), [Change { at: 7, view: None }]);
        assert_eq!(
            read("/b"),
            [Change {
                at: 9,
                view: Some("x".into())
            }]
        );
    }
}
