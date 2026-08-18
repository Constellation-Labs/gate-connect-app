//! A per-entry record of what the last routing restore did, so an interrupted one
//! can be explained rather than only retried.
//!
//! The snapshots in [`crate::provider`] already say *what is left*; that is what
//! makes a resume possible and it is the load-bearing state. This says *what
//! happened*, which is what makes the recovery explainable: which entries were
//! finished, which failed and roughly why, which were skipped and for what reason.
//! Losing this file costs an explanation, never a recovery.
//!
//! **Written as it goes, not at the end.** An operation that is interrupted is
//! exactly the one worth describing, and a journal written only on completion
//! would be missing for every case it exists to serve. Restores are rare and hold
//! a handful of entries, so a small write per entry is affordable.
//!
//! Scope: the restore path only. Master-off, quit and reset are not journalled -
//! `snapshot_and_disable_everything` reports its failures to its caller instead
//! (see the quit flow), and journalling them would mean instrumenting three more
//! code paths for a summary nothing renders yet.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Where the journal lives, beside the snapshots it explains.
fn journal_path() -> Result<PathBuf> {
    Ok(crate::env::app_support_dir()?
        .join("provider")
        .join("restore-journal.json"))
}

/// What kind of thing an entry is, so the UI can group without re-deriving it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Provider,
    Tool,
}

/// What the restore did to one entry on its last attempt.
///
/// A closed set, and every member is derived from the control flow rather than by
/// reading an error string: the restore already branches on these conditions, so
/// classifying them costs nothing and cannot drift the way message-matching does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Recorded but not attempted yet. What an interruption leaves behind.
    Pending,
    /// Back on, as far as the write is concerned. Whether it is *routing* is the
    /// verdict layer's question, not this one's.
    Restored,
    /// The write itself failed. Stays in the snapshot, so a resume retries it.
    WriteFailed,
    /// The tool is not on this machine any more. Dropped from the snapshot: there
    /// is nothing to restore and retrying forever would be wrong.
    NotInstalled,
    /// The slug is not in the registry - written by an older build, or a tool
    /// since removed. Also dropped.
    Unknown,
    /// Nothing was attempted because there is no account to point tools at. Kept
    /// in the snapshot for a later signed-in restore.
    ///
    /// Only the *tool* pass can report this. `enable_skipping` has no signed-out
    /// branch - it returns an ordinary error - so a provider that cannot be
    /// re-enabled for want of an account lands on `WriteFailed` instead. Fixing
    /// that means giving the provider path the same early-out the tool path has,
    /// rather than matching on an error message here.
    DeferredSignedOut,
}

impl Outcome {
    /// Whether this outcome still owes the user something. `NotInstalled` and
    /// `Unknown` do not - they were dropped deliberately, and reporting them as
    /// outstanding would ask for action nobody can take.
    pub const fn is_outstanding(self) -> bool {
        matches!(
            self,
            Outcome::Pending | Outcome::WriteFailed | Outcome::DeferredSignedOut
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryRecord {
    pub slug: String,
    /// Display name at the time of the attempt, so the record still reads properly
    /// after the tool is uninstalled.
    pub name: String,
    pub kind: EntryKind,
    pub outcome: Outcome,
    /// When this entry was last touched. Unix seconds; 0 when the clock could not
    /// be read, which the UI renders as unknown rather than as 1970.
    pub at_unix: u64,
}

/// The last restore, entry by entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RestoreJournal {
    /// When the journal was last written.
    ///
    /// Every field here is `#[serde(default)]`-backed so a file written by an older
    /// build still loads. `load` turns a parse failure into "no journal", which
    /// would lose the explanation this file exists to carry - so the parse has to
    /// tolerate a shape it has not seen.
    #[serde(default)]
    pub updated_unix: u64,
    /// What the operation was trying to achieve. Always true today - the only
    /// journalled operation is the restore, which exists to turn routing back on -
    /// and recorded rather than assumed so a future master-off journal does not
    /// silently invert the meaning of every existing file.
    #[serde(default)]
    pub requested_routing_on: bool,
    #[serde(default)]
    pub entries: Vec<EntryRecord>,
}

impl RestoreJournal {
    /// Entries that still owe something.
    pub fn outstanding(&self) -> impl Iterator<Item = &EntryRecord> {
        self.entries.iter().filter(|e| e.outcome.is_outstanding())
    }
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Read the journal. A missing or unparseable file reads as absent: this is an
/// explanation, and failing to load one must never block the recovery it describes.
pub fn load() -> Option<RestoreJournal> {
    let path = journal_path().ok()?;
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Overwrite the journal. 0644 - non-secret: slugs, display names and outcomes,
/// with no paths, credentials or request content anywhere in it.
pub fn save(journal: &RestoreJournal) -> Result<()> {
    let path = journal_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let raw = serde_json::to_vec_pretty(journal).context("serializing the restore journal")?;
    crate::primitives::write_file(&path, &raw, 0o644)
        .with_context(|| format!("writing {}", path.display()))
}

/// Discard the journal. Called when a restore completes with nothing outstanding,
/// so a later launch does not offer to explain an operation that finished.
pub fn clear() -> Result<()> {
    let path = journal_path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

/// Builds a journal while a restore runs, saving after every entry.
///
/// Best-effort throughout: a write failure here is logged and dropped rather than
/// propagated, because the journal explains the restore and must never be the
/// reason one fails.
pub struct JournalWriter {
    journal: RestoreJournal,
}

impl JournalWriter {
    /// Start a journal for a restore that intends to turn routing on, seeding every
    /// known entry as `Pending`. The seed is the point: an operation interrupted
    /// before it reached entry three has to leave those entries visibly untouched.
    pub fn begin(entries: Vec<(String, String, EntryKind)>) -> Self {
        let at = now_unix();
        let journal = RestoreJournal {
            updated_unix: at,
            requested_routing_on: true,
            entries: entries
                .into_iter()
                .map(|(slug, name, kind)| EntryRecord {
                    slug,
                    name,
                    kind,
                    outcome: Outcome::Pending,
                    at_unix: at,
                })
                .collect(),
        };
        let writer = Self { journal };
        writer.flush();
        writer
    }

    /// Record what happened to one entry and persist immediately.
    pub fn record(&mut self, slug: &str, outcome: Outcome) {
        let at = now_unix();
        if let Some(entry) = self.journal.entries.iter_mut().find(|e| e.slug == slug) {
            entry.outcome = outcome;
            entry.at_unix = at;
        }
        self.journal.updated_unix = at;
        self.flush();
    }

    /// Finish: keep the journal while anything is outstanding, discard it when the
    /// restore actually completed. A kept journal is what "Review details" reads.
    pub fn finish(self) {
        if self.journal.outstanding().next().is_none() {
            if let Err(e) = clear() {
                eprintln!("[gate] clearing the restore journal failed: {e:#}");
            }
            return;
        }
        self.flush();
    }

    fn flush(&self) {
        if let Err(e) = save(&self.journal) {
            eprintln!("[gate] writing the restore journal failed: {e:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(slug: &str, outcome: Outcome) -> EntryRecord {
        EntryRecord {
            slug: slug.into(),
            name: slug.into(),
            kind: EntryKind::Tool,
            outcome,
            at_unix: 1,
        }
    }

    /// The distinction the recovery notice depends on: dropped entries are not
    /// outstanding. Reporting them would ask for action nobody can take.
    #[test]
    fn dropped_entries_owe_nothing() {
        assert!(!Outcome::Restored.is_outstanding());
        assert!(!Outcome::NotInstalled.is_outstanding());
        assert!(!Outcome::Unknown.is_outstanding());
    }

    #[test]
    fn unfinished_entries_are_outstanding() {
        assert!(Outcome::Pending.is_outstanding());
        assert!(Outcome::WriteFailed.is_outstanding());
        assert!(Outcome::DeferredSignedOut.is_outstanding());
    }

    /// An interruption is the case this exists for: entries never reached stay
    /// `Pending`, and a journal holding them is still outstanding.
    #[test]
    fn a_journal_with_untouched_entries_is_outstanding() {
        let journal = RestoreJournal {
            updated_unix: 1,
            requested_routing_on: true,
            entries: vec![
                entry("codex", Outcome::Restored),
                entry("opencode", Outcome::Pending),
            ],
        };
        let names: Vec<_> = journal.outstanding().map(|e| e.slug.as_str()).collect();
        assert_eq!(names, ["opencode"]);
    }

    #[test]
    fn a_finished_journal_owes_nothing() {
        let journal = RestoreJournal {
            updated_unix: 1,
            requested_routing_on: true,
            entries: vec![
                entry("codex", Outcome::Restored),
                entry("hermes", Outcome::NotInstalled),
            ],
        };
        assert!(journal.outstanding().next().is_none());
    }

    #[test]
    fn recording_moves_an_entry_off_pending() {
        let mut journal = RestoreJournal {
            updated_unix: 1,
            requested_routing_on: true,
            entries: vec![entry("codex", Outcome::Pending)],
        };
        // The same mutation `record` performs, without touching the filesystem.
        if let Some(e) = journal.entries.iter_mut().find(|e| e.slug == "codex") {
            e.outcome = Outcome::WriteFailed;
        }
        assert_eq!(journal.entries[0].outcome, Outcome::WriteFailed);
        assert_eq!(journal.outstanding().count(), 1);
    }

    /// A file from a build that predates a field must still load. The journal is an
    /// explanation, and refusing to read one would lose the explanation entirely.
    #[test]
    fn an_older_file_still_parses() {
        let journal: RestoreJournal = serde_json::from_str("{}").expect("empty object parses");
        assert_eq!(journal, RestoreJournal::default());
        assert!(journal.entries.is_empty());
    }
}
