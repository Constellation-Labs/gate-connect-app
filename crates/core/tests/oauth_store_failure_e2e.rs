//! A refresh Cognito granted is used even when the secret store will not take
//! it.
//!
//! The failure this pins: `refresh_stored` used to fail the whole refresh when
//! writing the renewed bundle failed, as `Unavailable`. Every caller reads that
//! as "keep what you have", so the app's 30s tick left the engine injecting the
//! expired bearer and the 401 re-check answered `Unchanged`. Routed traffic got
//! "X-Gate-Authorization bearer token is invalid or expired" while the window
//! said Protected, until a relaunch.
//!
//! Hermetic: the file-backed secret store (`GATE_CONNECT_TEST_SECRETS`) made
//! read-only so the write fails, and a loopback Cognito mock. Unix only,
//! because a read-only directory is how the write is made to fail.

#![cfg(unix)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

use gate_connect_core::env;
use gate_connect_core::oauth::{self, OAuthTokens};

/// Answer every request with a fresh access token, counting the calls.
fn spawn_cognito() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind cognito mock");
    let addr = listener.local_addr().expect("mock addr");
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_t = hits.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            hits_t.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let body = r#"{"access_token":"at-renewed","expires_in":3600,"token_type":"Bearer"}"#;
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

fn temp_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gate-connect-store-failure-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[test]
fn a_renewed_token_is_used_when_storing_it_fails() {
    let (endpoint, hits) = spawn_cognito();
    let secrets = temp_dir("secrets");
    let support = temp_dir("support");
    std::env::set_var("GATE_CONNECT_TEST_TOKEN_ENDPOINT", &endpoint);
    std::env::set_var("GATE_CONNECT_TEST_SECRETS", &secrets);
    env::set_app_support_dir_for_tests(Some(support.clone()));
    gate_connect_core::account::save("https://gateway.example.com", None).expect("seed account");
    gate_connect_core::account::set_auth_mode(gate_connect_core::account::AuthMode::OAuth)
        .expect("set oauth mode");
    std::env::set_var("GATE_COGNITO_HOSTED_DOMAIN", "unused.in.tests");
    std::env::set_var("GATE_COGNITO_CLIENT_ID", "client123");

    // Expired locally, so the next read refreshes.
    oauth::store(&OAuthTokens {
        access_token: "at-expired".to_string(),
        refresh_token: "rt-1".to_string(),
        id_token: None,
        expires_at_unix: now_unix() - 60,
        client_id: "client123".to_string(),
    })
    .expect("seed bundle");

    // Read-only: the old bundle still reads, but nothing can replace it.
    std::fs::set_permissions(&secrets, std::fs::Permissions::from_mode(0o500))
        .expect("make the secret store read-only");
    if std::fs::write(secrets.join("probe"), b"x").is_ok() {
        // Running as root, where permissions do not bind: nothing to test.
        let _ = std::fs::set_permissions(&secrets, std::fs::Permissions::from_mode(0o700));
        eprintln!("skipping: the read-only secret store is still writable here");
        return;
    }

    // What the app's 30s tick pushes into the engine.
    let live = oauth::live_session().expect("a granted refresh is a live session");
    assert_eq!(
        live.access_token, "at-renewed",
        "the renewed token must reach the engine even though it was not stored"
    );
    assert!(matches!(
        oauth::session_reading(),
        oauth::SessionReading::Live(t) if t.access_token == "at-renewed"
    ));

    // What the 401 re-check runs before it probes the gateway.
    let cfg = oauth::OAuthConfig::from_build_env().expect("cognito env set");
    let forced = oauth::force_refresh(&cfg)
        .expect("a store failure is not a refresh failure")
        .expect("a bundle is stored");
    assert_eq!(forced.access_token, "at-renewed");

    // Nothing was stored, so each read refreshes again rather than serving
    // the expired bundle.
    assert_eq!(hits.load(Ordering::SeqCst), 3);
    assert_eq!(
        oauth::current()
            .expect("bundle readable")
            .map(|t| t.access_token),
        Some("at-expired".to_string()),
        "the store really did refuse the write"
    );

    let _ = std::fs::set_permissions(&secrets, std::fs::Permissions::from_mode(0o700));
}
