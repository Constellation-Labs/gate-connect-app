//! Replacing an API key retires an install id an API-key account has spent,
//! and a failed save cannot hide that from its retry (AG-960).
//!
//! Behavioural: drives the real `account::save` against a throwaway data dir
//! (`GATE_CONNECT_TEST_HOME`) and file-backed secrets
//! (`GATE_CONNECT_TEST_SECRETS`), the seams the other integration tests use.
//! Both are process-global env vars, so `LOCK` serialises the file.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use gate_connect_core::{account, analytics};

static LOCK: Mutex<()> = Mutex::new(());

const GATEWAY: &str = "https://gateway.example";
/// Two keys that differ in their first 12 characters (the recorded prefix).
/// Built at run time: a literal shaped like a key trips secret scanners.
fn key(fill: char) -> String {
    let body: String = std::iter::repeat_n(fill, 24).collect();
    ["sk", "gw", &body].join("-")
}

struct Seams {
    dir: PathBuf,
}

impl Seams {
    fn set(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "gate-connect-key-retire-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("secrets")).unwrap();
        std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);
        std::env::set_var("GATE_CONNECT_TEST_SECRETS", dir.join("secrets"));
        Seams { dir }
    }

    fn secrets(&self) -> PathBuf {
        self.dir.join("secrets")
    }
}

impl Drop for Seams {
    fn drop(&mut self) {
        std::env::remove_var("GATE_CONNECT_TEST_HOME");
        std::env::remove_var("GATE_CONNECT_TEST_SECRETS");
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// An API-key account that has paired: the core records the org it spent the
/// install id on.
fn spend_install_id() {
    analytics::save_identity(analytics::Identity {
        org_id: Some("org-a".into()),
        auth_mode: Some("api_key".into()),
        ..analytics::Identity::default()
    })
    .unwrap();
    let got = analytics::load_identity();
    assert_eq!(got.api_key_org.as_deref(), Some("org-a"));
    assert!(!got.install_id_retired);
}

#[test]
fn saving_the_same_key_again_does_not_retire() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let key_a = key('a');
    let _seams = Seams::set("same");
    account::save(GATEWAY, Some(key_a.as_str())).unwrap();
    spend_install_id();

    account::save(GATEWAY, Some(key_a.as_str())).unwrap();

    assert!(!analytics::load_identity().install_id_retired);
}

#[test]
fn saving_a_different_key_retires_a_spent_install_id() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (key_a, key_b) = (key('a'), key('b'));
    let _seams = Seams::set("different");
    account::save(GATEWAY, Some(key_a.as_str())).unwrap();
    spend_install_id();

    account::save(GATEWAY, Some(key_b.as_str())).unwrap();

    assert!(analytics::load_identity().install_id_retired);
}

/// The keychain write fails after `account.json` already holds the new key's
/// prefix. The retirement must have happened anyway, or the retry - which now
/// compares the new key with itself - would never do it.
#[test]
fn a_save_that_fails_after_writing_the_prefix_still_retired() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (key_a, key_b) = (key('a'), key('b'));
    let seams = Seams::set("failed");
    account::save(GATEWAY, Some(key_a.as_str())).unwrap();
    spend_install_id();

    // Secrets can no longer be written: a file where the directory was.
    fs::remove_dir_all(seams.secrets()).unwrap();
    fs::write(seams.secrets(), "not a directory").unwrap();
    assert!(account::save(GATEWAY, Some(key_b.as_str())).is_err());
    assert_eq!(
        account::api_key_prefix().unwrap().as_deref(),
        Some(&key_b[..12]),
        "the precondition: the prefix landed before the keychain write failed"
    );
    assert!(analytics::load_identity().install_id_retired);

    // And the retry, with the store back, leaves it retired.
    fs::remove_file(seams.secrets()).unwrap();
    fs::create_dir_all(seams.secrets()).unwrap();
    account::save(GATEWAY, Some(key_b.as_str())).unwrap();
    assert!(analytics::load_identity().install_id_retired);
}
