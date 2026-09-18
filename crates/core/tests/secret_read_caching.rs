//! Hermetic test for the read caching that keeps the 30-second timers off the
//! OS secret store: [`keychain::get_cached`]'s witness and the OAuth bundle
//! cache behind [`oauth::current`].
//!
//! Why this is worth a test binary of its own: on Linux every secret-store read
//! is a fresh D-Bus connection, and `gnome-keyring-daemon` never frees the
//! ~8 KB of per-client state each closed one leaves behind. A caller on a timer
//! therefore grows somebody else's process by ~180 MB a day. The contract these
//! assert is "reads this process already made are not made again", which no
//! other test can see - a correct-looking value proves nothing about how many
//! times it was fetched.
//!
//! Backed by the file seam (`GATE_CONNECT_TEST_SECRETS`), so the real OS secret
//! store is never touched. Writing that file directly is how a read gets
//! counted: it changes the stored value behind the cache's back, so a stale
//! answer proves no read happened, and a fresh one proves it did. One test
//! function, because the seam is a process-global env var.

use gate_connect_core::oauth::OAuthTokens;
use gate_connect_core::{keychain, oauth};

fn temp_secrets_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gate-connect-secret-read-caching-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp secrets dir");
    dir
}

/// The seam's file for one entry. Mirrors `keychain::secret_file`: the name is
/// `{service}__{account}` with the path-separator chars folded.
fn secret_file(dir: &std::path::Path, service: &str, account: &str) -> std::path::PathBuf {
    dir.join(format!("{service}__{account}").replace(['/', '\\', ':'], "_"))
}

#[test]
fn cached_reads_skip_the_store_until_something_says_otherwise() {
    let dir = temp_secrets_dir();
    std::env::set_var("GATE_CONNECT_TEST_SECRETS", &dir);
    // `account.json` is the OAuth bundle's witness, so it has to resolve
    // somewhere disposable rather than at the developer's real one.
    let home = dir.join("home");
    std::env::set_var("GATE_CONNECT_TEST_HOME", &home);
    let account_json = home
        .join("app-support")
        .join("Gate Connect")
        .join("account.json");

    let service = "ai.constellation.gate-connect.test.witness";
    let account = "tester";
    let file = secret_file(&dir, service, account);

    // --- keychain::get_cached -------------------------------------------
    keychain::set(service, account, "first").expect("set");
    assert_eq!(
        keychain::get_cached(service, account, "witness-a").expect("first read"),
        Some("first".to_string()),
        "a cold read returns what is stored"
    );

    // Change the stored value without going through `set`, so nothing has told
    // the cache anything. This is the only way to tell a cached answer from a
    // re-read.
    std::fs::write(&file, "second").expect("overwrite behind the cache");
    assert_eq!(
        keychain::get_cached(service, account, "witness-a").expect("cached read"),
        Some("first".to_string()),
        "an unchanged witness must be served from the first read, not the store"
    );
    assert_eq!(
        keychain::get_cached(service, account, "witness-b").expect("witnessed read"),
        Some("second".to_string()),
        "a moved witness must go back to the store - this is how one process \
         notices another one's write"
    );

    // A write from *this* process invalidates regardless of the witness, which
    // is what keeps a caller whose witness is coarser than its secret honest.
    std::fs::write(&file, "third").expect("overwrite behind the cache");
    keychain::set(service, "unrelated", "x").expect("unrelated set");
    assert_eq!(
        keychain::get_cached(service, account, "witness-b").expect("post-write read"),
        Some("third".to_string()),
        "any write in this process drops what the cache is holding"
    );

    keychain::delete(service, account).expect("delete");
    assert_eq!(
        keychain::get_cached(service, account, "witness-b").expect("post-delete read"),
        None,
        "a delete is not served a cached value either"
    );

    // --- oauth::current --------------------------------------------------
    // `store` is the only writer in-process, so `current` should answer from
    // memory afterwards. The bundle is chunked across six entries, so this is
    // the read that costs the most to repeat.
    let tokens = OAuthTokens {
        access_token: "access-1".to_string(),
        refresh_token: "refresh-1".to_string(),
        id_token: Some("id-1".to_string()),
        expires_at_unix: 4_102_444_800,
        client_id: "client-1".to_string(),
    };
    oauth::store(&tokens).expect("store tokens");
    let oauth_service = keychain::account_service("oauth-tokens");
    let user = gate_connect_core::env::current_user().expect("current user");
    let oauth_file = secret_file(&dir, &oauth_service, &user);

    // One real read to fill the cache. `store` deliberately does not write
    // through - `keychain::set` invalidates, so the read after a refresh is a
    // real one, which costs six lookups an hour and keeps one mechanism
    // instead of two.
    assert_eq!(
        oauth::current()
            .expect("first read after store")
            .expect("a bundle is stored")
            .access_token,
        "access-1"
    );

    // Prove the NEXT one is not re-reading, by making the stored bytes
    // unusable. A re-read would fail to parse; a cached answer cannot notice.
    std::fs::write(&oauth_file, "{\"not\":\"json we would accept\"}")
        .expect("corrupt the stored bundle behind the cache");
    let served = oauth::current()
        .expect("current after store must not re-read")
        .expect("a bundle is stored");
    assert_eq!(
        served.access_token, "access-1",
        "current() must answer from the read it already made"
    );

    // ...but a login in ANOTHER process must still be seen. That is what the
    // witness buys over a plain cache: the CLI (`gate-connect login --oauth`)
    // writes both the bundle and `account.json`, and the moved file is how this
    // process learns to look again. Simulated by writing a valid bundle behind
    // the cache and then moving the witness, which is what the CLI's write
    // looks like from here.
    let replacement = OAuthTokens {
        access_token: "access-2-from-the-cli".to_string(),
        ..tokens.clone()
    };
    std::fs::write(
        &oauth_file,
        serde_json::to_string(&replacement).expect("serialize"),
    )
    .expect("write the CLI's bundle behind the cache");
    std::fs::create_dir_all(account_json.parent().expect("parent")).expect("app support dir");
    std::fs::write(
        &account_json,
        "{\"gateway_base_url\":\"https://example.test\"}",
    )
    .expect("the CLI's account.json");
    let after_login = oauth::current()
        .expect("current after the witness moved")
        .expect("a bundle is stored");
    assert_eq!(
        after_login.access_token, "access-2-from-the-cli",
        "a moved account.json must send current() back to the store, or a CLI \
         login goes unnoticed until the app restarts"
    );

    // Signing out drops it, rather than leaving the last session readable.
    oauth::clear().expect("clear tokens");
    assert!(
        oauth::current().expect("current after clear").is_none(),
        "clear() must empty the cache, not just the store"
    );

    std::env::remove_var("GATE_CONNECT_TEST_SECRETS");
    std::env::remove_var("GATE_CONNECT_TEST_HOME");
}
