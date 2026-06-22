//! Integration test for `account::reconcile()` — the startup self-heal that
//! drops a stale half of the account when an uninstall removed Gate Connect's
//! on-disk files but not its OS keychain entry (or vice versa). It only acts
//! when the two halves have drifted, so a signed-in user is never touched.
//!
//! These exercise the real path resolution (`account.json` under the per-OS
//! data dir, derived from `$HOME`/`$XDG_DATA_HOME` via `dirs`) and a process-
//! global in-memory keychain backend, so the real OS secret store is never
//! touched. It lives in its own test binary so those process-wide overrides
//! can't leak into the in-crate unit tests, and a `Mutex` serializes the
//! env-mutating tests within this binary.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use gate_connect_core::{account, env, keychain};

static LOCK: Mutex<()> = Mutex::new(());

/// Point `$HOME` at a fresh temp dir (and clear `$XDG_DATA_HOME`, which would
/// otherwise win on Linux) so `account.json` resolves under the temp tree.
/// Restores the prior values and deletes the dir on drop.
struct TempHome {
    dir: PathBuf,
    prev_home: Option<String>,
    prev_xdg: Option<String>,
}

impl TempHome {
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
        let prev_home = std::env::var("HOME").ok();
        let prev_xdg = std::env::var("XDG_DATA_HOME").ok();
        std::env::set_var("HOME", &dir);
        std::env::remove_var("XDG_DATA_HOME");
        TempHome {
            dir,
            prev_home,
            prev_xdg,
        }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        match &self.prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match &self.prev_xdg {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
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
    let _home = TempHome::set();
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

/// URL present, key gone: a stray `account.json` with no matching key.
/// reconcile() must remove it so the app starts at first-run instead of a
/// keyless, half-signed-in home.
#[test]
fn stray_account_json_without_key_is_removed() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    keychain::use_in_memory_backend();

    account::save("https://gw.example.com", None).unwrap();
    assert!(account::load_base_url().unwrap().is_some());
    assert!(!account::has_api_key().unwrap());

    account::reconcile().unwrap();

    assert!(
        account::load_base_url().unwrap().is_none(),
        "stray account.json must be removed when no key backs it"
    );
}

/// Both halves present (a signed-in user) must be left exactly as-is.
#[test]
fn intact_account_is_left_untouched() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
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
    let _home = TempHome::set();
    keychain::use_in_memory_backend();

    account::reconcile().unwrap();

    assert!(account::load_base_url().unwrap().is_none());
    assert!(!account::has_api_key().unwrap());
}
