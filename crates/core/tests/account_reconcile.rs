//! Integration test for `account::reconcile()` - the startup self-heal that
//! drops a stale half of the account when an uninstall removed Gate Connect's
//! on-disk files but not its OS keychain entry (or vice versa). It only acts
//! when the two halves have drifted, so a signed-in user is never touched.
//!
//! These exercise the real `account.json` read/write/remove against a throwaway
//! data dir (via the [`env::set_app_support_dir_for_tests`] seam - a `$HOME`
//! override doesn't redirect the data dir on Windows) plus a process-global
//! in-memory keychain backend, so the real OS secret store is never touched. It
//! lives in its own test binary so those process-wide overrides can't leak into
//! the in-crate unit tests, and a `Mutex` serializes them within this binary.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use gate_connect_core::{account, env, keychain};

static LOCK: Mutex<()> = Mutex::new(());

/// Point `app_support_dir()` at a fresh temp dir for the duration of a test, so
/// `account.json` resolves there on every OS. Clears the override and deletes
/// the dir on drop.
struct TempDataDir {
    dir: PathBuf,
}

impl TempDataDir {
    fn set() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "gate-connect-reconcile-test-{}-{}",
            std::process::id(),
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        env::set_app_support_dir_for_tests(Some(dir.clone()));
        TempDataDir { dir }
    }
}

impl Drop for TempDataDir {
    fn drop(&mut self) {
        env::set_app_support_dir_for_tests(None);
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn user() -> String {
    env::current_user().unwrap()
}

/// URL gone, key present: the orphan an uninstall leaves in the keychain after
/// the on-disk account was wiped. reconcile() must delete the stale key.
#[test]
fn orphaned_key_without_account_json_is_deleted() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    keychain::use_in_memory_backend();

    keychain::set(&account::service(), &user(), "sk-gw-orphan").unwrap();
    assert!(account::load_base_url().unwrap().is_none());
    assert!(account::has_api_key().unwrap());

    account::reconcile().unwrap();

    assert!(
        !account::has_api_key().unwrap(),
        "orphaned Gate key must be cleared when no account.json anchors it"
    );
}

/// URL present, key gone: a key-less `account.json` is a legitimate pending-key
/// state - a fresh `switch_gateway`, or a reinstall orphan - so reconcile() must
/// leave it intact and let the app route to key entry pointed at that gateway.
#[test]
fn keyless_account_json_is_left_for_key_entry() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    keychain::use_in_memory_backend();

    account::save("https://gw.example.com", None).unwrap();
    assert!(account::load_base_url().unwrap().is_some());
    assert!(!account::has_api_key().unwrap());

    account::reconcile().unwrap();

    assert!(
        account::load_base_url().unwrap().is_some(),
        "key-less account.json must be kept as a pending-key state"
    );
    assert!(!account::has_api_key().unwrap());
}

/// Both halves present (a signed-in user) must be left exactly as-is.
#[test]
fn intact_account_is_left_untouched() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    keychain::use_in_memory_backend();

    account::save("https://gw.example.com", Some("sk-gw-live")).unwrap();

    account::reconcile().unwrap();

    assert!(account::load_base_url().unwrap().is_some());
    assert!(account::has_api_key().unwrap());
}

/// Neither half present (a fresh machine) is a no-op that must not error.
#[test]
fn fresh_install_is_a_noop() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    keychain::use_in_memory_backend();

    account::reconcile().unwrap();

    assert!(account::load_base_url().unwrap().is_none());
    assert!(!account::has_api_key().unwrap());
}
