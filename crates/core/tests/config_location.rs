//! Every config-editing integration can name the file it rewrites.
//!
//! This backs the drift review's copy, which tells the user *which* file Gate is
//! about to change before asking them to approve the change. A `None` there is
//! not a crash, it is a dialog that quietly stops naming the file - so the useful
//! test is that the integrations which do edit a file all report one, and that
//! the path lands under the user's own home rather than somewhere surprising.
//!
//! Uses the real path resolution: `GATE_CONNECT_TEST_HOME` redirects it, and the
//! guard below serializes the env-mutating tests in this file.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use gate_connect_core::registry::{find, ToolId};

static HOME_LOCK: Mutex<()> = Mutex::new(());

/// Point `GATE_CONNECT_TEST_HOME` at a fresh temp dir for the duration of a
/// test, restoring the prior value (and deleting the dir) on drop. Same shape as
/// the guard in `master_off_sweeps_harnesses.rs`.
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
            "gate-connect-config-location-test-{}-{}",
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

/// The integrations that rewrite a file of their own. `env-proxy` is excluded on
/// purpose: it writes machine-wide environment settings, not a file it owns, so
/// it reports `None` and the dialog names the tool without a location.
const FILE_EDITING_TOOLS: [ToolId; 5] = [
    ToolId::ClaudeCode,
    ToolId::Codex,
    ToolId::OpenCode,
    ToolId::OpenClaw,
    ToolId::Hermes,
];

#[test]
fn every_file_editing_integration_names_its_config() {
    let _lock = HOME_LOCK.lock().unwrap();
    let home = TempHome::set();

    for id in FILE_EDITING_TOOLS {
        let integ = find(id).expect("integration in the registry");
        let location = integ.config_location();
        assert!(
            location.is_some(),
            "{} edits a config file but cannot name it, so the review dialog \
             would ask the user to approve a rewrite of an unnamed file",
            integ.display_name()
        );
        let location = location.unwrap();
        assert!(
            location.starts_with(&home.dir.display().to_string()),
            "{} reported {location:?}, which is outside the resolved home - the \
             path shown to the user must be the one that gets written",
            integ.display_name()
        );
    }
}

/// Not every integration owns a file, and inventing one for the copy would be
/// worse than omitting the line.
#[test]
fn the_environment_channel_names_no_file() {
    let _lock = HOME_LOCK.lock().unwrap();
    let _home = TempHome::set();

    let integ = find(ToolId::EnvProxy).expect("env-proxy in the registry");
    assert_eq!(
        integ.config_location(),
        None,
        "the environment channel writes machine-wide settings, not a config file"
    );
}

/// Reading the location must not create anything. The drift dialog calls this to
/// build a sentence, and a probe that materialised a config directory would make
/// `status()` report a tool as present because the UI had looked at it.
#[test]
fn naming_the_config_does_not_create_it() {
    let _lock = HOME_LOCK.lock().unwrap();
    let home = TempHome::set();

    for id in FILE_EDITING_TOOLS {
        let integ = find(id).expect("integration in the registry");
        let location = integ.config_location().expect("a path");
        assert!(
            !PathBuf::from(&location).exists(),
            "{} created {location:?} just by being asked where it writes",
            integ.display_name()
        );
    }

    // And nothing else appeared in the throwaway home either.
    let entries: Vec<_> = fs::read_dir(&home.dir)
        .expect("temp home readable")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        entries.is_empty(),
        "asking where configs live created {entries:?}"
    );
}
