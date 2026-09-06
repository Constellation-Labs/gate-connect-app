//! A diagnostic log, for local and staging builds only.
//!
//! Gate Connect wrote nothing to disk until this existed. Backend output went to
//! stdout, which only a `tauri dev` terminal ever sees, and the front end logged
//! to a webview console nobody can read after the fact - so a failure in the UI
//! left no trace at all. A routing switch that silently stopped responding was
//! undiagnosable without devtools open at the moment it happened, which is not a
//! thing you can ask a user to have done.
//!
//! ## Off in production, by two independent gates
//!
//! This app mediates people's API traffic and holds their credentials. A log
//! file is a liability in that setting, so it is written **only** when the
//! configured gateway is not production - and, failing that, only in a debug
//! build. Both conditions are checked; either one alone would be enough to keep
//! it out of a shipped, production-pointed app, which is the belt-and-braces
//! `config.ts` already uses for the localhost gateway entry.
//!
//! ## What may be written
//!
//! Callers pass a finished message and this writes it verbatim. Nothing here
//! inspects headers, bodies or account state, so the rule is the caller's to
//! keep: **never pass a credential, a prompt, or a request body.** [`redact`] is
//! a backstop for the one shape that would otherwise slip through by accident -
//! a Gate key pasted into an error string - not a licence to log secrets and
//! rely on scrubbing.

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::env;

/// Where production points. Anything else is local or staging.
const PRODUCTION_GATEWAY: &str = "https://gateway.constellationgate.ai";

/// Biggest the log may get before it is rotated, in bytes.
///
/// One rotation, not a series: this exists to explain the last session, and a
/// developer machine does not need a month of history. Two files bound the disk
/// cost at twice this and make "the log" an unambiguous thing to ask someone for.
const MAX_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }

    pub fn from_wire(s: &str) -> Level {
        match s {
            "error" => Level::Error,
            "warn" => Level::Warn,
            // Anything unrecognised is information rather than an error: a
            // mislabelled line is a nuisance, an invented error is a false alarm.
            _ => Level::Info,
        }
    }
}

/// Whether anything is written at all, decided once per process.
///
/// Once, because the answer is about which deployment this is, and that does not
/// meaningfully change while the app runs. A gateway switched at runtime keeps
/// the startup decision until restart - which errs towards *not* starting to
/// write in a session that began pointed at production.
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        // `load_base_url` rather than `load`: it reads the file and never the
        // keychain. This runs on every logging path, and a decision about
        // whether to write a log file must never be the thing that raises a
        // credential prompt.
        //
        // No account yet (first run) falls back to the build kind, so a dev
        // build still records what happened before anyone signed in.
        match crate::account::load_base_url() {
            Ok(Some(url)) => url.trim_end_matches('/') != PRODUCTION_GATEWAY,
            _ => cfg!(debug_assertions),
        }
    })
}

fn log_path() -> Option<PathBuf> {
    env::app_support_dir()
        .ok()
        .map(|d| d.join("gate-connect.log"))
}

/// Blunt scrub for the one secret shape that reaches a message by accident: a
/// Gate key interpolated into an error string.
///
/// Deliberately not clever. It is a backstop, and a thorough redactor would
/// invite callers to treat logging as safe for anything - see the module note.
fn redact(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    for word in message.split_inclusive(char::is_whitespace) {
        let trimmed = word.trim_end();
        if trimmed.starts_with("sk-gw-") || trimmed.starts_with("sk-ant-") {
            out.push_str("<redacted>");
            out.push_str(&word[trimmed.len()..]);
        } else {
            out.push_str(word);
        }
    }
    out
}

/// Append one line, or do nothing.
///
/// Infallible by design. Every caller is on a path doing something the user
/// asked for, and a log that cannot be written is not a reason to fail their
/// work - the same trade [`crate::primitives::install_id_cached`] makes.
pub fn log(level: Level, message: &str) {
    if !enabled() {
        return;
    }
    let Some(path) = log_path() else { return };
    rotate_if_large(&path);

    let stamp = time::OffsetDateTime::now_utc().unix_timestamp();
    let line = format!("{stamp} {} {}\n", level.label(), redact(message));
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| f.write_all(line.as_bytes()));
}

fn rotate_if_large(path: &std::path::Path) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    if meta.len() < MAX_BYTES {
        return;
    }
    // Replaces the previous rotation rather than accumulating. Errors are
    // ignored: a failed rotation means the file keeps growing, which is a much
    // smaller problem than a failed write on a path the user is waiting on.
    let _ = std::fs::rename(path, path.with_extension("log.1"));
}

/// The log's location, for the diagnostics report and for telling someone which
/// file to send. `None` when logging is off, so a production build offers no
/// path to a file it never writes.
pub fn path_for_report() -> Option<PathBuf> {
    enabled().then(log_path).flatten()
}
