//! Gate's changes to each tool's configuration file.
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
//! tool configuration goes through [`write`] or [`remove`] (or their `_with_view`
//! forms), and a change is kept only when the file's bytes actually changed: an
//! unattended reconnect that finds its values already in place records nothing.
//!
//! **A hand edit to a routing value is recorded late.** It is seen when Gate
//! next writes the file (see below), not when it happens. That is the price of
//! ignoring the tool's own edits, and until then the status check still reports
//! it: it compares the file with what Gate wrote, so a changed proxy reads as
//! Not protected on the row.
//!
//! **Two writers can lose a change.** The app and the CLI both write here, and
//! recording is a read, an insert and a write with no lock between them, so two
//! changes that land at once can keep only one. Until the next write to that
//! file finds the view the lost one left behind, a running process that missed
//! it gets no reopen notice: the same "no evidence, no claim" outcome as a
//! missing record, accepted because overlapping writes need a CLI connect
//! racing the app's own pass.
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
//! **What was on disk is recorded too, when it is not what Gate last wrote.**
//! Skipping a write that restores the recorded view is only safe if nothing
//! happened in between, and two things can: a hand edit that strips Gate's
//! values, and a quit whose revert landed while its record did not (a store
//! write that failed, the CLI racing the app, a shutdown between the two).
//! Either way a process could have started on a view the record never saw,
//! and the next write would put the recorded view back and call that process
//! current. So every write that changes the file first fingerprints the file it
//! is about to replace, and a view other than the last recorded one goes in as
//! a change of its own, stamped one second after that last change - the
//! earliest it can have happened, so a process in the gap is judged stale
//! rather than current. Only a write that changes the file looks: a pass that
//! leaves the bytes alone would otherwise record the tool's own edits on every
//! reconnect, and for a tool whose view is the whole file that is a reopen
//! notice for nothing.
//!
//! **Bounded.** Each path keeps its newest [`MAX_CHANGES`], and a path whose
//! newest change is older than [`MAX_AGE_SECS`] is dropped the next time
//! anything is recorded. A process older than every change a path still
//! keeps is judged stale, which is what the single stamp said about it too.
//!
//! Keyed by the path as displayed, which is the form `config_location` returns.
//!
//! **A tool's own edit is not overwritten.** Gate edits a tool's file by
//! reading it, changing its own keys and writing the whole file back, and the
//! tool writes the same file whenever it likes: Claude Code on a permission
//! grant or `/model`, Codex on a project trust answer. An edit that lands
//! between Gate's read and its write was lost. Neither side can take a lock the
//! other honours, so [`guarded`] narrows the window and notices it instead:
//! inside it, the first [`read_to_string`] of a path remembers what was read,
//! every write and removal compares the file with that just before it lands -
//! after the tempfile is synced, immediately ahead of the rename - and a file
//! that moved abandons the write and runs the whole operation again on what is
//! there now. Every integration's `connect`, `disconnect` and
//! `leave_gate_models` runs inside it ([`crate::registry`]), and they are
//! idempotent, which is what makes running one again safe. What is left is the
//! instant between that comparison and the rename.

use std::cell::RefCell;
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

/// Attempts [`guarded`] makes at an operation before it gives up and returns
/// the [`ConcurrentEdit`]. A tool rewrites its file on a user's action, so a
/// second collision in a row is already unusual and a third means something is
/// rewriting the file continuously.
const MAX_ATTEMPTS: usize = 3;

/// A tool's configuration changed between Gate reading it and Gate writing it
/// back, so the write was abandoned rather than overwriting the change.
#[derive(Debug)]
pub struct ConcurrentEdit {
    pub path: PathBuf,
}

impl std::fmt::Display for ConcurrentEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} changed while Gate was updating it; try again",
            self.path.display()
        )
    }
}

impl std::error::Error for ConcurrentEdit {}

thread_local! {
    /// What each path held when the running [`guarded`] operation first read
    /// it (`None` for a file that was not there), kept current with Gate's own
    /// writes. `None` outside an operation, where nothing is remembered and
    /// nothing is checked.
    static READ: RefCell<Option<BTreeMap<PathBuf, Option<Vec<u8>>>>> =
        const { RefCell::new(None) };
}

/// Run `op`, which reads tool configurations and writes them back, so that a
/// tool's edit landing in between is kept: the write that would overwrite it
/// fails with [`ConcurrentEdit`], and `op` runs again from a fresh read, up to
/// [`MAX_ATTEMPTS`] times. Nested calls run inside the outer one, which owns
/// the retry.
pub(crate) fn guarded<T>(mut op: impl FnMut() -> Result<T>) -> Result<T> {
    if READ.with(|read| read.borrow().is_some()) {
        return op();
    }
    let mut attempt = 1;
    loop {
        let result = {
            let _scope = Scope::enter();
            op()
        };
        match result {
            Err(e) if attempt < MAX_ATTEMPTS && is_concurrent_edit(&e) => attempt += 1,
            other => return other,
        }
    }
}

/// Clears what [`guarded`] remembered when the operation ends, however it ends.
struct Scope;

impl Scope {
    fn enter() -> Self {
        READ.with(|read| *read.borrow_mut() = Some(BTreeMap::new()));
        Scope
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        READ.with(|read| *read.borrow_mut() = None);
    }
}

fn is_concurrent_edit(e: &anyhow::Error) -> bool {
    e.chain().any(|cause| cause.is::<ConcurrentEdit>())
}

/// [`fs::read_to_string`] for a tool configuration Gate is about to edit:
/// inside [`guarded`], the first read of a path is what its write is checked
/// against. A missing file is remembered as missing.
pub(crate) fn read_to_string(path: &Path) -> std::io::Result<String> {
    let body = fs::read_to_string(path);
    match &body {
        Ok(body) => remember_first(path, Some(body.as_bytes())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => remember_first(path, None),
        Err(_) => {}
    }
    body
}

fn remember_first(path: &Path, seen: Option<&[u8]>) {
    READ.with(|read| {
        if let Some(read) = read.borrow_mut().as_mut() {
            read.entry(path.to_path_buf())
                .or_insert_with(|| seen.map(<[u8]>::to_vec));
        }
    });
}

/// Gate's own write is the new baseline for the rest of the operation.
fn remember_written(path: &Path, written: Option<&[u8]>) {
    READ.with(|read| {
        if let Some(read) = read.borrow_mut().as_mut() {
            read.insert(path.to_path_buf(), written.map(<[u8]>::to_vec));
        }
    });
}

/// Fail with [`ConcurrentEdit`] if `path` no longer holds what the operation
/// read. A path it never read through [`read_to_string`] is not checked.
fn unchanged_since_read(path: &Path) -> Result<()> {
    let Some(expected) = READ.with(|read| {
        read.borrow()
            .as_ref()
            .and_then(|read| read.get(path).cloned())
    }) else {
        return Ok(());
    };
    let now = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    if now != expected {
        return Err(anyhow::Error::new(ConcurrentEdit {
            path: path.to_path_buf(),
        }));
    }
    Ok(())
}

/// Write a tool configuration, recording the change when the bytes differ from
/// what is on disk. The routing view is the whole file; a tool that edits its
/// own file goes through [`write_with_view`] instead. Identical bytes still go
/// through [`crate::primitives::write_file`], which leaves the content alone.
/// `mode` applies to a file Gate creates; one that exists keeps its own
/// ([`kept_mode`]).
pub(crate) fn write(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    write_with_view(path, bytes, mode, whole_file)
}

/// [`write`], with `view` taking a file's bytes to the part of it a running
/// process acts on. It is applied to the file being replaced as well as to the
/// new one, so an absent file reads as `view(b"")`.
pub(crate) fn write_with_view(
    path: &Path,
    bytes: &[u8],
    mode: u32,
    view: fn(&[u8]) -> Vec<u8>,
) -> Result<()> {
    let mode = kept_mode(path, mode);
    let current = fs::read(path).ok();
    crate::primitives::write_file_checked(path, bytes, mode, || unchanged_since_read(path))?;
    remember_written(path, Some(bytes));
    if current.as_deref() != Some(bytes) {
        record(
            path,
            &view(current.as_deref().unwrap_or_default()),
            &view(bytes),
        );
    }
    Ok(())
}

/// Remove a tool configuration, recording the change when there was one to
/// remove. An absent file's view is empty.
pub(crate) fn remove(path: &Path) -> Result<()> {
    remove_with_view(path, whole_file)
}

/// [`remove`] for a tool that writes through [`write_with_view`], so the file
/// being removed is read the same way.
pub(crate) fn remove_with_view(path: &Path, view: fn(&[u8]) -> Vec<u8>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let current = fs::read(path).unwrap_or_default();
    unchanged_since_read(path)?;
    fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    remember_written(path, None);
    record(path, &view(&current), &view(b""));
    Ok(())
}

/// The routing view of a tool that acts on its whole file.
pub(crate) fn whole_file(bytes: &[u8]) -> Vec<u8> {
    bytes.to_vec()
}

/// The mode a tool's file keeps across Gate's write: the one it has, or `mode`
/// when there is no file yet. The file is the user's, and so is its mode -
/// Gate's own values in it are routing settings, not credentials (the relay
/// injects the key per request), so a tool's 0644 is no reason to tighten it,
/// and a disconnect especially should leave nothing of Gate's behind.
/// Resolved through a symlink, as [`crate::primitives::write_file`] writes.
pub(crate) fn kept_mode(path: &Path, mode: u32) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|meta| meta.permissions().mode() & 0o7777)
            .unwrap_or(mode)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        mode
    }
}

/// Gate's changes to the file at `path`, oldest first. Empty when it has no
/// record of changing it.
pub fn history(path: &Path) -> Vec<Change> {
    load()
        .ok()
        .and_then(|mut stamps| stamps.remove(&key(path)))
        .unwrap_or_default()
}

/// Record a change: the view a write found on disk, when it is not the last one
/// recorded, and the view it left there.
///
/// Best effort. The configuration write has already landed when this runs, so
/// failing the caller here would report a connect as failed while the tool is
/// routed. A missing stamp can cost a reopen notice, and the log says why; the
/// next write's look at the file is what recovers the change it lost.
fn record(path: &Path, found: &[u8], written: &[u8]) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let saved = load().and_then(|mut stamps| {
        let changes = stamps.entry(key(path)).or_default();
        let observed = observe(changes, now, fingerprint(found));
        let appended = append(changes, now, fingerprint(written));
        if !observed && !appended {
            return Ok(());
        }
        prune(&mut stamps, now);
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

/// Forget every path whose newest change is older than [`MAX_AGE_SECS`].
fn prune(stamps: &mut BTreeMap<String, Vec<Change>>, now: u64) {
    stamps.retain(|_, changes| {
        changes
            .last()
            .is_some_and(|c| c.at.saturating_add(MAX_AGE_SECS) >= now)
    });
}

/// Add the view a write found on disk as a change of its own, when it is not
/// the last one recorded, stamped one second after that last change and no
/// later than `now`. A path with no record has nothing to contradict. Whether
/// anything was added.
fn observe(changes: &mut Vec<Change>, now: u64, view: String) -> bool {
    changes
        .last()
        .filter(|last| last.view.as_deref() != Some(view.as_str()))
        .map(|last| last.at.saturating_add(1).min(now))
        .is_some_and(|at| append(changes, at, view))
}

/// Add a change to one path's list, unless its view is the one already in
/// place, and keep the list to [`MAX_CHANGES`]. Whether it was added.
///
/// Never stamped before the change it follows: a clock stepped back would
/// otherwise leave the list out of order, and [`crate::reopen`] reads what a
/// process loaded off that order.
fn append(changes: &mut Vec<Change>, mut at: u64, view: String) -> bool {
    if let Some(last) = changes.last() {
        if last.view.as_deref() == Some(view.as_str()) {
            return false;
        }
        at = at.max(last.at);
    }
    changes.push(Change {
        at,
        view: Some(view),
    });
    let excess = changes.len().saturating_sub(MAX_CHANGES);
    changes.drain(..excess);
    true
}

fn fingerprint(view: &[u8]) -> String {
    crate::manifest::sha256_hex(view)
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
    fn a_view_gate_did_not_write_goes_in_just_after_the_last_change() {
        let mut changes = Vec::new();
        append(&mut changes, 100, "gate".into());
        assert!(observe(&mut changes, 300, "own".into()));
        append(&mut changes, 300, "gate".into());
        let seen: Vec<(u64, &str)> = changes
            .iter()
            .map(|c| (c.at, c.view.as_deref().unwrap()))
            .collect();
        assert_eq!(seen, [(100, "gate"), (101, "own"), (300, "gate")]);
    }

    #[test]
    fn the_view_last_recorded_is_not_observed_again() {
        let mut changes = Vec::new();
        append(&mut changes, 100, "gate".into());
        assert!(!observe(&mut changes, 300, "gate".into()));
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn a_path_with_no_record_observes_nothing() {
        let mut changes = Vec::new();
        assert!(!observe(&mut changes, 300, "own".into()));
        assert!(changes.is_empty());
    }

    #[test]
    fn an_observed_change_is_never_stamped_after_now() {
        let mut changes = Vec::new();
        append(&mut changes, 300, "gate".into());
        observe(&mut changes, 300, "own".into());
        assert_eq!(changes.last().map(|c| c.at), Some(300));
    }

    #[test]
    fn a_path_is_forgotten_only_once_its_newest_change_is_too_old() {
        let now = 10 * MAX_AGE_SECS;
        let change = |at| Change {
            at,
            view: Some("v".into()),
        };
        let mut stamps = BTreeMap::from([
            ("stale".to_string(), vec![change(now - MAX_AGE_SECS - 1)]),
            ("edge".to_string(), vec![change(now - MAX_AGE_SECS)]),
            ("old-then-new".to_string(), vec![change(1), change(now - 1)]),
        ]);
        prune(&mut stamps, now);
        let kept: Vec<&str> = stamps.keys().map(String::as_str).collect();
        assert_eq!(kept, ["edge", "old-then-new"]);
    }

    #[test]
    fn a_clock_stepped_back_never_puts_a_change_before_the_last() {
        let mut changes = Vec::new();
        append(&mut changes, 300, "gate".into());
        append(&mut changes, 200, "own".into());
        assert_eq!(changes.last().map(|c| c.at), Some(300));
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

    /// A temp `GATE_CONNECT_TEST_HOME`, so the change record these writes keep
    /// lands there rather than in the real app-support directory, and a tool
    /// file inside it holding `body`.
    struct ToolFile {
        home: PathBuf,
        path: PathBuf,
        prev: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl ToolFile {
        fn new(name: &str, body: &str) -> Self {
            let lock = crate::env::path_env_lock();
            let home = std::env::temp_dir()
                .join(format!("gate-config-changes-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&home);
            fs::create_dir_all(&home).unwrap();
            let prev = std::env::var_os("GATE_CONNECT_TEST_HOME");
            std::env::set_var("GATE_CONNECT_TEST_HOME", &home);
            let path = home.join("settings.txt");
            fs::write(&path, body).unwrap();
            ToolFile {
                home,
                path,
                prev,
                _lock: lock,
            }
        }

        fn body(&self) -> String {
            fs::read_to_string(&self.path).unwrap()
        }
    }

    impl Drop for ToolFile {
        fn drop(&mut self) {
            match self.prev.take() {
                Some(v) => std::env::set_var("GATE_CONNECT_TEST_HOME", v),
                None => std::env::remove_var("GATE_CONNECT_TEST_HOME"),
            }
            let _ = fs::remove_dir_all(&self.home);
        }
    }

    /// The race in the ticket: the tool rewrites its file after Gate read it
    /// and before Gate's write lands. Gate's write is abandoned, the operation
    /// runs again on what the tool wrote, and the file ends with both edits.
    #[test]
    fn a_tool_edit_between_read_and_write_is_kept() {
        let file = ToolFile::new("kept", "user\n");
        let mut attempts = 0;
        guarded(|| {
            attempts += 1;
            let body = read_to_string(&file.path)?;
            if attempts == 1 {
                fs::write(&file.path, format!("{body}tool\n")).unwrap();
            }
            write(&file.path, format!("{body}gate\n").as_bytes(), 0o600)
        })
        .unwrap();
        assert_eq!(attempts, 2);
        assert_eq!(file.body(), "user\ntool\ngate\n");
    }

    /// A file that changes under every attempt is left as the tool last wrote
    /// it, and the caller is told why.
    #[test]
    fn a_file_that_keeps_changing_is_left_to_the_tool() {
        let file = ToolFile::new("busy", "user\n");
        let mut attempts = 0;
        let result = guarded(|| {
            attempts += 1;
            let body = read_to_string(&file.path)?;
            fs::write(&file.path, format!("{body}tool {attempts}\n")).unwrap();
            write(&file.path, format!("{body}gate\n").as_bytes(), 0o600)
        });
        let err = result.unwrap_err();
        assert!(is_concurrent_edit(&err), "{err:#}");
        assert_eq!(attempts, MAX_ATTEMPTS);
        assert!(!file.body().contains("gate"), "{}", file.body());
    }

    /// A removal checks the same way: a file Gate would delete as holding
    /// nothing but its own lines is kept once the tool has added one.
    #[test]
    fn a_removal_does_not_take_a_tool_edit_with_it() {
        let file = ToolFile::new("remove", "gate\n");
        let mut attempts = 0;
        guarded(|| {
            attempts += 1;
            let body = read_to_string(&file.path)?;
            if attempts == 1 {
                fs::write(&file.path, "gate\ntool\n").unwrap();
            }
            match body.as_str() {
                "gate\n" => remove(&file.path),
                _ => write(&file.path, body.replace("gate\n", "").as_bytes(), 0o600),
            }
        })
        .unwrap();
        assert_eq!(attempts, 2);
        assert_eq!(file.body(), "tool\n");
    }

    /// Gate's own write is the baseline for the rest of the operation, so a
    /// second write to the same file is not mistaken for someone else's.
    #[test]
    fn gate_writing_one_file_twice_is_not_a_conflict() {
        let file = ToolFile::new("twice", "user\n");
        let mut attempts = 0;
        guarded(|| {
            attempts += 1;
            let body = read_to_string(&file.path)?;
            write(&file.path, format!("{body}one\n").as_bytes(), 0o600)?;
            let body = read_to_string(&file.path)?;
            write(&file.path, format!("{body}two\n").as_bytes(), 0o600)
        })
        .unwrap();
        assert_eq!(attempts, 1);
        assert_eq!(file.body(), "user\none\ntwo\n");
    }
}
