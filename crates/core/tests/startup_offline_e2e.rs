//! Hermetic coverage of the launch-time probe, `startup::refresh_session`, when
//! the stored access token has expired and the refresh has to reach Cognito.
//!
//! The distinction under test: a refresh Cognito *refused* is a verdict, and a
//! refresh that got no answer is not. A machine that launches offline must not
//! be told to sign in again, or every boot before the network comes up would
//! read as an expired session.
//!
//! Same seams as `session_recovery_e2e`: a file-backed secret store, a loopback
//! Cognito mock, and `GATE_COGNITO_*` so `OAuthConfig::from_build_env()`
//! returns `Some`. One test function, because those seams are process-global.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use gate_connect_core::env;
use gate_connect_core::oauth::{self, OAuthTokens};
use gate_connect_core::startup::{refresh_session, SessionVerdict};

/// A loopback token endpoint that answers every call with `status` and `body`.
fn spawn_token_endpoint(status: &'static str, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
    let base = format!("http://{}", listener.local_addr().expect("mock addr"));
    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("{base}/oauth2/token")
}

/// A loopback address with nothing listening: the network is not up yet.
fn dead_endpoint() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind for a dead port");
    let addr = listener.local_addr().expect("dead addr");
    drop(listener);
    format!("http://{addr}/oauth2/token")
}

fn temp_secrets_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gate-connect-startup-offline-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp secrets dir");
    dir
}

/// A bundle whose access token expired a minute ago, so the probe must refresh.
fn expired_tokens() -> OAuthTokens {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    OAuthTokens {
        access_token: "at-expired".to_string(),
        refresh_token: "rt-1".to_string(),
        id_token: None,
        expires_at_unix: now - 60,
        client_id: "client123".to_string(),
    }
}

#[test]
fn an_unreachable_refresh_at_launch_is_no_verdict() {
    let secrets = temp_secrets_dir();
    std::env::set_var("GATE_CONNECT_TEST_SECRETS", &secrets);
    env::set_app_support_dir_for_tests(Some(secrets.clone()));
    gate_connect_core::account::save("https://gateway.example.com", None).expect("seed account");
    gate_connect_core::account::set_auth_mode(gate_connect_core::account::AuthMode::OAuth)
        .expect("set oauth mode");
    std::env::set_var("GATE_COGNITO_HOSTED_DOMAIN", "unused.in.tests");
    std::env::set_var("GATE_COGNITO_CLIENT_ID", "client123");

    // 1. Offline launch: Cognito cannot be reached. No verdict, and nothing
    //    latched, so the tray and the status stay where they were.
    oauth::store(&expired_tokens()).expect("store");
    std::env::set_var("GATE_CONNECT_TEST_TOKEN_ENDPOINT", dead_endpoint());
    assert!(
        matches!(refresh_session(), SessionVerdict::Unavailable),
        "a refresh that got no answer must not ask for a sign-in"
    );
    assert!(!oauth::session_rejected());
    assert!(
        oauth::current().expect("bundle readable").is_some(),
        "an outage must not cost the user their stored session"
    );

    // 2. A 5xx is an answer about Cognito, not about the credential.
    std::env::set_var(
        "GATE_CONNECT_TEST_TOKEN_ENDPOINT",
        spawn_token_endpoint("503 Service Unavailable", r#"{"error":"busy"}"#),
    );
    assert!(matches!(refresh_session(), SessionVerdict::Unavailable));

    // 3. Cognito refuses the refresh token: that is a verdict.
    std::env::set_var(
        "GATE_CONNECT_TEST_TOKEN_ENDPOINT",
        spawn_token_endpoint("400 Bad Request", r#"{"error":"invalid_grant"}"#),
    );
    assert!(
        matches!(refresh_session(), SessionVerdict::NeedsSignIn),
        "a refused refresh token means the user must sign in"
    );
}
