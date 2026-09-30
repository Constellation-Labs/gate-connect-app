//! `account::load` hands out no API key on an OAuth account, even when one is
//! still in the keychain from before the switch to OAuth, and finds that key
//! again on the way back to key mode.
//!
//! The first half is what stops a dead session from quietly falling back to a
//! key the user thought they had replaced; the second is why the key is left
//! in the keychain rather than deleted. Hermetic: the secret store is
//! file-backed (`GATE_CONNECT_TEST_SECRETS`) and the data dir is a throwaway.

use gate_connect_core::{account, env};

fn temp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gate-connect-account-oauth-key-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn oauth_mode_loads_no_api_key_and_key_mode_finds_it_again() {
    let dir = temp_dir();
    std::env::set_var("GATE_CONNECT_TEST_SECRETS", &dir);
    env::set_app_support_dir_for_tests(Some(dir.clone()));

    account::save("https://gateway.example.com", Some("sk-gw-pasted")).expect("seed account");
    let loaded = account::load().expect("load").expect("account");
    assert_eq!(
        loaded.api_key, "sk-gw-pasted",
        "key mode loads the pasted key"
    );

    account::set_auth_mode(account::AuthMode::OAuth).expect("switch to oauth");
    let loaded = account::load().expect("load").expect("account");
    assert!(matches!(loaded.auth_mode, account::AuthMode::OAuth));
    assert_eq!(
        loaded.api_key, "",
        "an OAuth account holds no key, whatever the keychain still has"
    );
    assert!(
        account::has_api_key().expect("has_api_key"),
        "the pasted key stays in the keychain for the way back"
    );

    account::set_auth_mode(account::AuthMode::ApiKey).expect("switch back to key mode");
    let loaded = account::load().expect("load").expect("account");
    assert_eq!(
        loaded.api_key, "sk-gw-pasted",
        "switching back to key mode finds the key again"
    );

    env::set_app_support_dir_for_tests(None);
    std::env::remove_var("GATE_CONNECT_TEST_SECRETS");
    let _ = std::fs::remove_dir_all(&dir);
}
