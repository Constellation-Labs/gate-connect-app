//! The restore journal, through the real master-off / master-on path.
//!
//! `recovery`'s unit tests cover the journal's own logic. This covers the wiring:
//! that `restore_all` actually seeds and records entries, that a restore which
//! finishes leaves no journal behind, and that one which cannot finish keeps a
//! journal explaining why.
//!
//! Uses the real path resolution - `GATE_CONNECT_TEST_HOME` redirects it - and
//! serialises the env-mutating tests in this file, in the same shape as
//! `master_off_sweeps_harnesses.rs`.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use gate_connect_core::recovery::{self, Outcome};
use gate_connect_core::registry::{find, ConnectInput, Status, ToolId};
use gate_connect_core::{env, provider};

static HOME_LOCK: Mutex<()> = Mutex::new(());

struct TempHome {
    dir: PathBuf,
    prev: Option<String>,
}

impl TempHome {
    fn set() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "gate-connect-restore-journal-{}-{}",
            std::process::id(),
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        let prev = std::env::var("GATE_CONNECT_TEST_HOME").ok();
        std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);
        TempHome { dir, prev }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var("GATE_CONNECT_TEST_HOME", v),
            None => std::env::remove_var("GATE_CONNECT_TEST_HOME"),
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Put OpenCode into a Gate-managed state, the same way
/// `master_off_sweeps_harnesses.rs` does. Lands on `Drifted` rather than
/// `Connected` because no engine runs under the test; both are in the managed set
/// the master-off sweep walks, so it exercises the right path.
fn connect_opencode() {
    let dir = env::opencode_config_dir().unwrap();
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        env::opencode_config_path().unwrap(),
        r#"{
  "provider": {
    "anthropic": {
      "options": { "apiKey": "{env:ANTHROPIC_API_KEY}" },
      "models": { "claude-haiku-4-5": {} }
    }
  }
}
"#,
    )
    .unwrap();

    let integ = find(ToolId::OpenCode).expect("opencode integration registered");
    integ
        .connect(&ConnectInput {
            gateway_base_url: "https://gateway.example.com".into(),
            upstream_url: integ.default_upstream_url().to_string(),
            billing_mode: Default::default(),
            relay_base_url: Some("http://127.0.0.1:8402".into()),
            engine_proxy_url: Some("http://127.0.0.1:8403".into()),
        })
        .expect("connect opencode");
    assert!(
        matches!(
            integ.status().unwrap(),
            Status::Connected | Status::Drifted(_)
        ),
        "precondition: opencode must be Gate-managed before master-off"
    );
}

/// The case the journal exists for: a restore that *cannot* finish keeps an
/// explanation. With no account on disk the tool pass attempts nothing and leaves
/// the snapshot for a later signed-in restore, so the journal must say that rather
/// than go quiet.
///
/// `DeferredSignedOut`, not `WriteFailed`: nothing is wrong with the tool, and
/// calling it a failure would send the user hunting for a problem that is really a
/// missing account.
#[test]
fn a_restore_that_cannot_run_explains_itself() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    connect_opencode();

    // Master off records OpenCode in the swept-tools snapshot. No account was ever
    // saved in this throwaway home, so the restore below cannot proceed.
    provider::snapshot_and_disable_everything().expect("master off");
    provider::restore_all().expect("restore is best-effort and must not error");

    let journal = recovery::load().expect("a restore that could not finish keeps a journal");
    let entry = journal
        .entries
        .iter()
        .find(|e| e.slug == "opencode")
        .expect("opencode is in the journal");
    assert_eq!(entry.outcome, Outcome::DeferredSignedOut);
    assert_eq!(entry.name, "OpenCode", "recorded by display name");
    assert!(
        journal.outstanding().any(|e| e.slug == "opencode"),
        "a deferred entry still owes the user something"
    );
    assert!(
        journal.requested_routing_on,
        "the operation was trying to turn routing back on"
    );

    // The provider pass is snapshotted by the same master-off, and with no account
    // every provider fails to re-enable. Recorded as `WriteFailed` rather than
    // deferred, because `enable_skipping` has no signed-out branch to classify on -
    // see the note on `Outcome`. Asserted so the shortcoming is visible rather than
    // discovered later.
    assert!(
        journal
            .entries
            .iter()
            .any(|e| e.kind == recovery::EntryKind::Provider && e.outcome == Outcome::WriteFailed),
        "a provider that could not be re-enabled is recorded, not dropped"
    );
}

/// Nothing recorded means nothing to explain. A restore with empty snapshots must
/// not leave a journal, or every launch would offer to explain an operation that
/// never happened.
#[test]
fn a_restore_with_nothing_to_do_leaves_no_journal() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();

    provider::restore_all().expect("restore with no snapshots");

    assert!(
        recovery::load().is_none(),
        "an empty restore must not leave a journal behind"
    );
}

/// A journal is an explanation, never a blocker: a corrupt one reads as absent and
/// the recovery it describes still works.
#[test]
fn a_corrupt_journal_reads_as_absent() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();

    let path = env::app_support_dir()
        .unwrap()
        .join("provider")
        .join("restore-journal.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "{ not json").unwrap();

    assert!(recovery::load().is_none());
    // And the restore still runs.
    provider::restore_all().expect("a corrupt journal must not break a restore");
}
