//! Persistence for the account's billing mode - who pays the upstream provider.
//!
//! The mode decides the *shape* of every routed request (see
//! `proxy::inject_gate_credential`), so the two things worth pinning on disk are
//! that an account written before PAYG existed still loads as BYOK, and that a
//! switch survives the writes that happen around it: saving a URL, rotating a
//! key, repointing at another gateway.
//!
//! Same harness as `account_reconcile`: a throwaway data dir via the
//! `GATE_CONNECT_TEST_HOME` seam plus the in-memory keychain, in its own test
//! binary so those process-wide overrides can't leak, with a `Mutex`
//! serializing within it.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use gate_connect_core::account::{self, BillingMode};
use gate_connect_core::keychain;

static LOCK: Mutex<()> = Mutex::new(());

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
            "gate-connect-billing-mode-test-{}-{}",
            std::process::id(),
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);
        TempDataDir { dir }
    }
}

impl Drop for TempDataDir {
    fn drop(&mut self) {
        std::env::remove_var("GATE_CONNECT_TEST_HOME");
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// An `account.json` written before this field existed - i.e. every install
/// upgrading into PAYG support - must read as BYOK, the shape it has always
/// routed under. Written as raw JSON on purpose: this is the one case no
/// round-trip through our own writer can produce.
#[test]
fn account_json_without_the_field_loads_as_byok() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    keychain::use_in_memory_backend();

    let path = gate_connect_core::env::app_support_dir()
        .unwrap()
        .join("account.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        r#"{"gateway_base_url":"https://gw.example.com","auth_mode":"api_key"}"#,
    )
    .unwrap();

    assert_eq!(account::billing_mode().unwrap(), BillingMode::Byok);
    keychain::set(
        &account::service(),
        &gate_connect_core::env::current_user().unwrap(),
        "sk-gw-live",
    )
    .unwrap();
    assert_eq!(
        account::load().unwrap().unwrap().billing_mode,
        BillingMode::Byok
    );
}

/// No account at all also reads as BYOK rather than erroring, because the proxy
/// managers seed the engine from this on every start.
#[test]
fn no_account_reads_as_byok() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    keychain::use_in_memory_backend();

    assert_eq!(account::billing_mode().unwrap(), BillingMode::Byok);
    assert_eq!(account::billing_mode_for_injection(), BillingMode::Byok);
}

#[test]
fn payg_round_trips_and_reaches_the_loaded_account() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    keychain::use_in_memory_backend();

    account::save("https://gw.example.com", Some("sk-gw-live")).unwrap();
    account::set_billing_mode(BillingMode::Payg).unwrap();

    assert_eq!(account::billing_mode().unwrap(), BillingMode::Payg);
    assert_eq!(account::billing_mode_for_injection(), BillingMode::Payg);
    // `load()` is what the proxy managers read to seed the engine, so the mode
    // has to arrive there and not just in the standalone accessor.
    assert_eq!(
        account::load().unwrap().unwrap().billing_mode,
        BillingMode::Payg
    );
}

/// The writes that happen around a mode switch must not silently revert it.
/// `save` is called on a URL edit and on every key rotation, and it rebuilds the
/// file from the parts it was given - so anything it does not carry forward is
/// lost, and losing this one would move a user's spend back onto their own
/// provider key with nothing said.
#[test]
fn saving_a_url_or_rotating_a_key_preserves_payg() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    keychain::use_in_memory_backend();

    account::save("https://gw.example.com", Some("sk-gw-first")).unwrap();
    account::set_billing_mode(BillingMode::Payg).unwrap();

    account::save("https://gw.example.com", None).unwrap(); // URL-only edit
    assert_eq!(account::billing_mode().unwrap(), BillingMode::Payg);

    account::save("https://gw.example.com", Some("sk-gw-rotated")).unwrap();
    assert_eq!(account::billing_mode().unwrap(), BillingMode::Payg);

    // A gateway repoint clears the org and the key, which are
    // environment-specific. Who pays is not, so it carries over.
    account::switch_gateway("https://other-gw.example.com").unwrap();
    assert_eq!(account::billing_mode().unwrap(), BillingMode::Payg);
}

#[test]
fn switching_back_to_byok_persists() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    keychain::use_in_memory_backend();

    account::save("https://gw.example.com", Some("sk-gw-live")).unwrap();
    account::set_billing_mode(BillingMode::Payg).unwrap();
    account::set_billing_mode(BillingMode::Byok).unwrap();

    assert_eq!(account::billing_mode().unwrap(), BillingMode::Byok);
}

/// Setting a mode with no account is a caller bug, not something to paper over
/// by inventing an account file with no gateway URL.
#[test]
fn setting_a_mode_without_an_account_fails() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    keychain::use_in_memory_backend();

    assert!(account::set_billing_mode(BillingMode::Payg).is_err());
}
