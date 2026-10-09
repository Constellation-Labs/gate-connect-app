//! A refresh Cognito granted is the session, even when the secret store will
//! not take it.
//!
//! The failure this pins: `refresh_stored` used to fail the whole refresh when
//! writing the renewed bundle failed, as `Unavailable`. Every caller reads that
//! as "keep what you have", so the app's 30s tick left the engine injecting the
//! expired bearer and the 401 re-check answered `Unchanged`. Routed traffic got
//! "X-Gate-Authorization bearer token is invalid or expired" while the window
//! said Protected, until a relaunch.
//!
//! Returning the renewed token was not enough on its own, because the next read
//! went back to the store: an expired bundle (refreshed again), a locally fresh
//! one the gateway had refused (served again), or nothing readable (no session
//! at all). So the steps below each read *after* the failed store, not just the
//! call that made it.
//!
//! Hermetic: the file-backed secret store (`GATE_CONNECT_TEST_SECRETS`) made
//! read-only so writes fail, and a loopback Cognito mock. One test function,
//! because the seams and the session state are process-global and the steps
//! build on each other. Unix only, because file permissions are how the store
//! is made to fail.

#![cfg(unix)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

use gate_connect_core::env;
use gate_connect_core::oauth::{self, OAuthTokens};

/// Answer every request with a new access token, `at-renewed-<n>` for the
/// n-th call (1-based), counting the calls.
fn spawn_cognito() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind cognito mock");
    let addr = listener.local_addr().expect("mock addr");
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_t = hits.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let n = hits_t.fetch_add(1, Ordering::SeqCst) + 1;
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let body = format!(
                r#"{{"access_token":"at-renewed-{n}","expires_in":3600,"token_type":"Bearer"}}"#
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    (format!("http://{addr}/oauth2/token"), hits)
}

/// Owns the test's temp dirs. Puts permissions back and removes both on drop,
/// so a failed assertion does not leave a read-only directory behind.
struct Dirs {
    secrets: PathBuf,
    support: PathBuf,
}

impl Dirs {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "gate-connect-store-failure-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dirs = Dirs {
            secrets: base.join("secrets"),
            support: base.join("support"),
        };
        std::fs::create_dir_all(&dirs.secrets).expect("create secrets dir");
        std::fs::create_dir_all(&dirs.support).expect("create support dir");
        dirs
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        set_mode(&self.secrets, 0o700);
        if let Ok(entries) = std::fs::read_dir(&self.secrets) {
            for entry in entries.flatten() {
                set_mode(&entry.path(), 0o600);
            }
        }
        if let Some(base) = self.secrets.parent() {
            let _ = std::fs::remove_dir_all(base);
        }
        env::set_app_support_dir_for_tests(None);
        for var in [
            "GATE_CONNECT_TEST_TOKEN_ENDPOINT",
            "GATE_CONNECT_TEST_SECRETS",
            "GATE_COGNITO_HOSTED_DOMAIN",
            "GATE_COGNITO_CLIENT_ID",
        ] {
            std::env::remove_var(var);
        }
    }
}

fn set_mode(path: &Path, mode: u32) {
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn live_token() -> Option<String> {
    oauth::live_session().map(|t| t.access_token)
}

#[test]
fn a_renewed_session_outlives_a_store_that_refuses_it() {
    let (endpoint, hits) = spawn_cognito();
    let dirs = Dirs::new();
    std::env::set_var("GATE_CONNECT_TEST_TOKEN_ENDPOINT", &endpoint);
    std::env::set_var("GATE_CONNECT_TEST_SECRETS", &dirs.secrets);
    env::set_app_support_dir_for_tests(Some(dirs.support.clone()));
    gate_connect_core::account::save("https://gateway.example.com", None).expect("seed account");
    gate_connect_core::account::set_auth_mode(gate_connect_core::account::AuthMode::OAuth)
        .expect("set oauth mode");
    std::env::set_var("GATE_COGNITO_HOSTED_DOMAIN", "unused.in.tests");
    std::env::set_var("GATE_COGNITO_CLIENT_ID", "client123");
    let cfg = oauth::OAuthConfig::from_build_env().expect("cognito env set");

    // Expired locally, so the first read refreshes.
    oauth::store(&OAuthTokens {
        access_token: "at-expired".to_string(),
        refresh_token: "rt-1".to_string(),
        id_token: None,
        expires_at_unix: now_unix() - 60,
        client_id: "client123".to_string(),
    })
    .expect("seed bundle");
    let bundle_files: Vec<PathBuf> = std::fs::read_dir(&dirs.secrets)
        .expect("list secrets")
        .flatten()
        .map(|e| e.path())
        .collect();
    assert!(!bundle_files.is_empty(), "the seeded bundle is on disk");

    // Read-only: the old bundle still reads, but nothing can replace it.
    set_mode(&dirs.secrets, 0o500);
    if std::fs::write(dirs.secrets.join("probe"), b"x").is_ok() {
        // Running as root, where permissions do not bind: nothing to test.
        let _ = std::fs::remove_file(dirs.secrets.join("probe"));
        eprintln!("skipping: the read-only secret store is still writable here");
        return;
    }

    // 1. What the app's 30s tick pushes into the engine: the renewed token,
    //    though the store kept the expired one.
    assert_eq!(live_token().as_deref(), Some("at-renewed-1"));
    assert!(
        matches!(
            oauth::session_reading(),
            oauth::SessionReading::Live(ref t) if t.access_token == "at-renewed-1"
        ),
        "the tick's reading must be the renewed session"
    );
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "later reads serve the renewed session rather than refreshing the stored one again"
    );

    // 2. The store becomes unreadable as well. The renewed session does not
    //    depend on it: reads still serve it, and the 401 re-check's forced
    //    refresh still works from its refresh token.
    for file in &bundle_files {
        set_mode(file, 0o000);
    }
    assert!(
        oauth::current().is_err(),
        "the store really is unreadable now"
    );
    assert_eq!(live_token().as_deref(), Some("at-renewed-1"));
    let forced = oauth::force_refresh(&cfg)
        .expect("a store failure is not a refresh failure")
        .expect("there is a session to refresh");
    assert_eq!(forced.access_token, "at-renewed-2");

    // 3. A locally fresh bundle the gateway refused is not served again after
    //    the re-check renewed it: the next tick reads the renewal.
    assert_eq!(live_token().as_deref(), Some("at-renewed-2"));
    assert_eq!(hits.load(Ordering::SeqCst), 2);

    // 4. A gateway rejection is cleared by a renewal kept in memory, as it is
    //    by one the store took. Otherwise the tick after a recovery would read
    //    signed out and push the empty bearer.
    oauth::mark_session_rejected();
    assert_eq!(live_token(), None, "a rejected session is not live");
    let forced = oauth::force_refresh(&cfg)
        .expect("refresh")
        .expect("session");
    assert_eq!(forced.access_token, "at-renewed-3");
    assert!(
        !oauth::session_rejected(),
        "the renewal supersedes the rejection"
    );
    assert_eq!(live_token().as_deref(), Some("at-renewed-3"));

    // 5. Another process signing in or out moves `account.json`, and the
    //    renewal kept here no longer stands: reads go back to the store, which
    //    accepts writes again.
    for file in &bundle_files {
        set_mode(file, 0o600);
    }
    set_mode(&dirs.secrets, 0o700);
    gate_connect_core::account::save("https://gateway-2.example.com", None)
        .expect("rewrite account");
    assert_eq!(
        live_token().as_deref(),
        Some("at-renewed-4"),
        "the stored, expired bundle is refreshed, not the in-memory one served"
    );
    assert_eq!(
        oauth::current()
            .expect("readable")
            .map(|t| t.access_token)
            .as_deref(),
        Some("at-renewed-4"),
        "and this time the store took it"
    );

    // 6. A sign-out in this process ends a renewal kept in memory too.
    set_mode(&dirs.secrets, 0o500);
    let forced = oauth::force_refresh(&cfg)
        .expect("refresh")
        .expect("session");
    assert_eq!(forced.access_token, "at-renewed-5");
    set_mode(&dirs.secrets, 0o700);
    oauth::clear().expect("sign out");
    assert_eq!(live_token(), None, "nothing outlives a sign-out");
    assert_eq!(hits.load(Ordering::SeqCst), 5, "no extra refreshes");
}
