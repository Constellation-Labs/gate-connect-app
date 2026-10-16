//! `account.json` is the witness for the cached Gate key read in
//! `account::load` (see `keychain::get_cached`), so it has to move on every
//! sign-in and never ahead of the key it vouches for.
//!
//! Neither property is visible from one process: a write here clears this
//! process's cache whatever the witness does. What they protect is the app
//! while the CLI signs in. If `gate-connect login --api-key` writes the same
//! bytes as before, the app never looks again; if it writes the file before
//! the key, an app tick between the two caches the old key under the new
//! witness, and nothing moves it after that. So these assert on the file
//! itself, which is exactly what the other process sees.
//!
//! Hermetic: the secret store is file-backed (`GATE_CONNECT_TEST_SECRETS`) and
//! the data dir is a throwaway. One test function, because both seams are
//! process-global.

use gate_connect_core::{account, env};

fn temp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gate-connect-account-key-witness-{}-{}",
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
fn every_key_sign_in_moves_the_witness_and_only_after_the_key() {
    let dir = temp_dir();
    let secrets = dir.join("secrets");
    let data = dir.join("data");
    std::fs::create_dir_all(&secrets).expect("secrets dir");
    std::fs::create_dir_all(&data).expect("data dir");
    std::env::set_var("GATE_CONNECT_TEST_SECRETS", &secrets);
    env::set_app_support_dir_for_tests(Some(data.clone()));
    let account_json = data.join("account.json");
    let read_witness = || std::fs::read_to_string(&account_json).expect("account.json");

    // Signing in again with the very same key, gateway and mode is the case
    // that used to write identical bytes.
    account::save("https://gateway.example.com", Some("sk-gw-same-key")).expect("first sign-in");
    account::set_auth_mode(account::AuthMode::ApiKey).expect("key mode");
    let before = read_witness();
    account::save("https://gateway.example.com", Some("sk-gw-same-key")).expect("second sign-in");
    account::set_auth_mode(account::AuthMode::ApiKey).expect("key mode again");
    let after = read_witness();
    assert_ne!(
        before, after,
        "a repeated sign-in must still move account.json, or another process \
         holding a cached read never looks at the store again"
    );

    // Saving only the URL is not a sign-in, and must not churn the witness.
    account::save("https://gateway.example.com", None).expect("URL-only save");
    assert_eq!(
        read_witness(),
        after,
        "a URL-only save with nothing changed leaves the file alone"
    );

    // A key the store refuses must not move the witness either: the file is
    // only written once the key it vouches for is in place.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Empty the store, so the refused write is a new entry in a read-only
        // directory rather than an overwrite the file mode cannot stop.
        for entry in std::fs::read_dir(&secrets).expect("list secrets").flatten() {
            std::fs::remove_file(entry.path()).expect("empty the store");
        }
        let keyless = read_witness();
        std::fs::set_permissions(&secrets, std::fs::Permissions::from_mode(0o500))
            .expect("make the store read-only");
        if std::fs::write(secrets.join("probe"), b"x").is_ok() {
            // Running as root, where permissions do not bind: nothing to test.
            let _ = std::fs::remove_file(secrets.join("probe"));
            eprintln!("skipping: the read-only secret store is still writable here");
        } else {
            assert!(
                account::save("https://gateway.example.com", Some("sk-gw-refused")).is_err(),
                "the store refused the key, so the save fails"
            );
            assert_eq!(
                read_witness(),
                keyless,
                "account.json must not move ahead of a key that never landed"
            );
        }
        let _ = std::fs::set_permissions(&secrets, std::fs::Permissions::from_mode(0o700));
    }

    env::set_app_support_dir_for_tests(None);
    std::env::remove_var("GATE_CONNECT_TEST_SECRETS");
    let _ = std::fs::remove_dir_all(&dir);
}
