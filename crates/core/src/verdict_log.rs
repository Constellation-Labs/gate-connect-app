//! What the last routing sweep concluded about each tool, kept across launches.
//!
//! [`crate::routing_health`] computes a verdict from live evidence and throws it
//! away. That is right for the status line - a verdict is only as good as the
//! moment it was taken - but it leaves the recovery summary with nothing to say
//! about the *past*: "last verified route" and "check result" are both questions
//! about a reading that has already happened, and a sweep that ran before the
//! process died is exactly the one worth reporting.
//!
//! So each sweep records itself here. Two readings per tool, deliberately kept
//! apart:
//!
//! - **The latest check**, whatever it concluded. This is the "check result".
//! - **The latest check that reached a verdict**, meaning `on` or `off`. This is
//!   the "last verified route", and it is the one that must not be overwritten by
//!   a failed verification - the whole point of the field is to survive one.
//!   `NeedsAttention` and `NotInstalled` are not routes, so they leave it alone.
//!
//! Non-secret by construction: slugs, four verdict words and two timestamps. No
//! paths, no URLs, no credentials, no request content. Losing the file costs the
//! summary its history and nothing else, so a missing or unparseable one reads as
//! empty rather than failing anything.
//!
//! Two shells sweep independently, so two read-modify-writes can interleave and
//! the later one wins. Left as it is on purpose: both are writing a whole sweep
//! taken from the same files a moment apart, so the loss is a duplicate rather
//! than a divergence, and locking a record that exists to explain a failure would
//! be a new way for it to fail.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::routing_health::RoutingVerdict;

/// Beside the snapshots and the restore journal, for the same reason: this is
/// routing bookkeeping, not account state.
fn log_path() -> Result<PathBuf> {
    Ok(crate::env::app_support_dir()?
        .join("provider")
        .join("verdict-log.json"))
}

/// One tool's sweep history, as far as the summary needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictEntry {
    pub slug: String,
    /// The latest sweep's verdict word (`on`, `off`, `needs_attention`,
    /// `not_installed`).
    pub state: String,
    /// The latest sweep's reason, when it needed attention.
    #[serde(default)]
    pub reason: Option<String>,
    /// When that sweep ran. Unix seconds; 0 when the clock could not be read,
    /// which the UI renders as unknown rather than as 1970.
    #[serde(default)]
    pub at_unix: u64,
    /// The last sweep that actually established a route: `on` or `off`, never a
    /// failed verification. `None` until one has.
    #[serde(default)]
    pub verified_state: Option<String>,
    #[serde(default)]
    pub verified_unix: u64,
}

/// Every tool the last sweep touched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VerdictLog {
    /// Every field is `#[serde(default)]`-backed so a file written by an older
    /// build still loads, on the same reasoning as the restore journal: this is a
    /// record, and refusing to read one loses the record entirely.
    #[serde(default)]
    pub updated_unix: u64,
    #[serde(default)]
    pub entries: Vec<VerdictEntry>,
}

impl VerdictLog {
    pub fn get(&self, slug: &str) -> Option<&VerdictEntry> {
        self.entries.iter().find(|e| e.slug == slug)
    }

    /// Fold one sweep result in, preserving the last verified route when this
    /// reading did not establish one.
    ///
    /// Kept as a method rather than done at the call site so the "a failed
    /// verification never clears the history" rule has exactly one
    /// implementation, and a test can reach it without a filesystem.
    pub fn record(&mut self, slug: &str, verdict: RoutingVerdict, at: u64) {
        let state = verdict.as_str().to_string();
        let reason = verdict.reason().map(|r| r.as_str().to_string());
        // `NotInstalled` is not a route either: a tool that has left the machine
        // must not retro-claim that it was last seen routing nowhere.
        let verified = matches!(verdict, RoutingVerdict::On | RoutingVerdict::Off);
        self.updated_unix = at;
        if let Some(entry) = self.entries.iter_mut().find(|e| e.slug == slug) {
            entry.state = state;
            entry.reason = reason;
            entry.at_unix = at;
            if verified {
                entry.verified_state = Some(verdict.as_str().to_string());
                entry.verified_unix = at;
            }
            return;
        }
        self.entries.push(VerdictEntry {
            slug: slug.to_string(),
            state,
            reason,
            at_unix: at,
            verified_state: verified.then(|| verdict.as_str().to_string()),
            verified_unix: if verified { at } else { 0 },
        });
    }
}

/// Read the log. Missing or unparseable reads as empty: this is history, and
/// failing to load it must never block the sweep that would replace it.
pub fn load() -> VerdictLog {
    let Ok(path) = log_path() else {
        return VerdictLog::default();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Overwrite the log. 0644 - non-secret, per this module's header.
pub fn save(log: &VerdictLog) -> Result<()> {
    let path = log_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let raw = serde_json::to_vec_pretty(log).context("serializing the verdict log")?;
    crate::primitives::write_file(&path, &raw, 0o644)
        .with_context(|| format!("writing {}", path.display()))
}

/// Fold a whole sweep in and persist it.
///
/// Best-effort by signature: the caller is a status read, and a status read that
/// fails because its own bookkeeping could not be written would be a worse bug
/// than a summary with a gap in it.
pub fn record_sweep(results: &[(String, RoutingVerdict)]) {
    if results.is_empty() {
        return;
    }
    let at = crate::recovery::now_unix();
    let mut log = load();
    for (slug, verdict) in results {
        log.record(slug, *verdict, at);
    }
    if let Err(e) = save(&log) {
        eprintln!("[gate] writing the verdict log failed: {e:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing_health::Reason;

    /// The rule the whole file exists for: a check that could not conclude
    /// replaces the check result and leaves the last verified route standing.
    #[test]
    fn a_failed_verification_does_not_erase_the_last_verified_route() {
        let mut log = VerdictLog::default();
        log.record("claude-code", RoutingVerdict::On, 100);
        log.record(
            "claude-code",
            RoutingVerdict::NeedsAttention(Reason::VerificationFailed),
            200,
        );

        let entry = log.get("claude-code").expect("recorded");
        assert_eq!(entry.state, "needs_attention");
        assert_eq!(entry.reason.as_deref(), Some("verification_failed"));
        assert_eq!(entry.at_unix, 200);
        // The reading that still stands, and when it was taken.
        assert_eq!(entry.verified_state.as_deref(), Some("on"));
        assert_eq!(entry.verified_unix, 100);
    }

    /// `Off` is a verified route, not the absence of one - the verdict layer only
    /// returns it once it has established that the tool is pointed elsewhere.
    #[test]
    fn off_is_a_verified_route() {
        let mut log = VerdictLog::default();
        log.record("codex", RoutingVerdict::Off, 50);
        assert_eq!(
            log.get("codex").and_then(|e| e.verified_state.as_deref()),
            Some("off")
        );
    }

    /// A tool that has never verified says so, rather than reporting a route it
    /// never had.
    #[test]
    fn a_tool_that_never_verified_has_no_route_to_report() {
        let mut log = VerdictLog::default();
        log.record(
            "opencode",
            RoutingVerdict::NeedsAttention(Reason::ConnectionProblem),
            10,
        );
        let entry = log.get("opencode").expect("recorded");
        assert_eq!(entry.verified_state, None);
        assert_eq!(entry.verified_unix, 0);
    }

    /// An uninstall is not a routing verdict. Recording it as one would let a
    /// tool that has left the machine overwrite the last thing known about it.
    #[test]
    fn not_installed_leaves_the_history_alone() {
        let mut log = VerdictLog::default();
        log.record("hermes", RoutingVerdict::On, 5);
        log.record("hermes", RoutingVerdict::NotInstalled, 6);
        let entry = log.get("hermes").expect("recorded");
        assert_eq!(entry.state, "not_installed");
        assert_eq!(entry.verified_state.as_deref(), Some("on"));
        assert_eq!(entry.verified_unix, 5);
    }

    /// A file from a build that predates a field must still load, for the reason
    /// the restore journal's own test states: the record is the point, and
    /// refusing to read one loses it.
    #[test]
    fn an_older_file_still_loads() {
        let raw = r#"{"entries":[{"slug":"codex","state":"on"}]}"#;
        let log: VerdictLog = serde_json::from_str(raw).expect("older shape loads");
        let entry = log.get("codex").expect("entry");
        assert_eq!(entry.at_unix, 0);
        assert_eq!(entry.verified_state, None);
    }
}
