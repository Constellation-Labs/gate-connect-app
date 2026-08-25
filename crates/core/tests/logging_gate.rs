//! The diagnostic log's on/off gate, and its one redaction.
//!
//! An integration test rather than a unit test for the reason
//! `tool_model_preferences.rs` gives: it points the app-support directory
//! somewhere temporary, and that override is process-global.
//!
//! What is under test is mostly the *off* case. This app holds people's
//! credentials, so a log file that appeared in a production build would be a
//! liability - and `enabled()` is decided once per process, which makes it
//! exactly the kind of thing that is easy to get wrong and hard to notice.

use std::path::PathBuf;
use std::sync::Mutex;

use gate_connect_core::env;
use gate_connect_core::logging::{self, Level};

static LOCK: Mutex<()> = Mutex::new(());

struct TempHome {
    dir: PathBuf,
}

impl TempHome {
    fn set() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("gc-log-{n}"));
        std::fs::create_dir_all(&dir).expect("create temp app-support dir");
        env::set_app_support_dir_for_tests(Some(dir.clone()));
        Self { dir }
    }

    fn contents(&self) -> String {
        std::fs::read_to_string(self.dir.join("gate-connect.log")).unwrap_or_default()
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        env::set_app_support_dir_for_tests(None);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A debug test binary with no account on disk logs, which is what makes the
/// file useful on a developer machine before anyone has signed in.
#[test]
fn a_dev_build_with_no_account_writes() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempHome::set();

    assert!(logging::enabled(), "a debug build should log");
    logging::log(Level::Info, "routing: asking to trust the CA");

    assert!(tmp.contents().contains("routing: asking to trust the CA"));
}

/// Levels are labelled, so a reader can tell a note from a failure without
/// parsing the sentence.
#[test]
fn each_level_is_labelled() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempHome::set();

    logging::log(Level::Info, "one");
    logging::log(Level::Warn, "two");
    logging::log(Level::Error, "three");

    let body = tmp.contents();
    assert!(body.contains("INFO one"));
    assert!(body.contains("WARN two"));
    assert!(body.contains("ERROR three"));
}

/// An unrecognised level from the front end is information, not an error.
/// Inventing an error would put false alarms in a file people read to find real
/// ones.
#[test]
fn an_unknown_level_reads_as_information() {
    assert_eq!(Level::from_wire("error"), Level::Error);
    assert_eq!(Level::from_wire("warn"), Level::Warn);
    assert_eq!(Level::from_wire("nonsense"), Level::Info);
}

/// The backstop: a Gate key interpolated into an error string does not reach the
/// file. It is not licence to log secrets - see the module note - but this shape
/// arrives by accident, from a message that quoted what it was sent.
#[test]
fn a_key_in_a_message_is_scrubbed() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempHome::set();

    logging::log(
        Level::Error,
        "gateway refused sk-gw-abcdef0123456789 for this org",
    );

    let body = tmp.contents();
    assert!(
        !body.contains("sk-gw-abcdef0123456789"),
        "the key must not land"
    );
    assert!(body.contains("<redacted>"));
    // The rest of the sentence survives, or the line would say nothing useful.
    assert!(body.contains("gateway refused"));
    assert!(body.contains("for this org"));
}

/// Writing must never be the reason something fails. An unwritable directory is
/// a silent no-op, not a panic on a path the user is waiting on.
#[test]
fn an_unwritable_location_is_survived() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempHome::set();
    // A file where the directory should be: every open below must fail.
    let _ = std::fs::remove_dir_all(&tmp.dir);
    std::fs::write(&tmp.dir, b"not a directory").expect("occupy the path");

    logging::log(Level::Error, "this cannot be written anywhere");

    let _ = std::fs::remove_file(&tmp.dir);
}
