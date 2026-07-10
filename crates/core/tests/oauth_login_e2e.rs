//! End-to-end test for the interactive OAuth login orchestration
//! (`oauth::login`): bind loopback → open Hosted UI → capture redirect →
//! exchange code → persist tokens.
//!
//! Hermetic: a fake "browser" closure hits the loopback callback (instead of
//! a real browser + Cognito), a raw-TCP mock stands in for the token
//! endpoint (`GATE_CONNECT_TEST_TOKEN_ENDPOINT`), and the secret store is
//! file-backed (`GATE_CONNECT_TEST_SECRETS`). No network, no OS keychain.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use gate_connect_core::oauth::{self, OAuthConfig};

/// Serve exactly one canned JSON response on a loopback token endpoint.
fn spawn_token_mock(response_body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind token mock");
    let addr = listener.local_addr().expect("addr");
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut tmp = [0u8; 2048];
            let _ = stream.read(&mut tmp); // drain the request; we don't assert here
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("http://{addr}/oauth2/token")
}

/// A unique temp dir for the file-backed secret store.
fn temp_secrets_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gate-connect-oauth-login-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp secrets dir");
    dir
}

#[test]
fn interactive_login_captures_redirect_and_persists_tokens() {
    let token_endpoint = spawn_token_mock(
        r#"{"access_token":"at-live","refresh_token":"rt-live","id_token":"it-live","expires_in":3600,"token_type":"Bearer"}"#,
    );
    let secrets = temp_secrets_dir();
    std::env::set_var("GATE_CONNECT_TEST_TOKEN_ENDPOINT", &token_endpoint);
    std::env::set_var("GATE_CONNECT_TEST_SECRETS", &secrets);

    let cfg = OAuthConfig {
        hosted_domain: "unused.in.tests".to_string(),
        client_id: "client123".to_string(),
        scopes: vec!["openid".to_string()],
    };

    // The fake browser: parse state + redirect_uri out of the authorize URL,
    // then (asynchronously, like a real browser) hit the loopback callback.
    let open_url = |authorize_url: &str| -> anyhow::Result<()> {
        let url = reqwest::Url::parse(authorize_url)?;
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        let state = q["state"].clone();
        let redirect = q["redirect_uri"].clone();
        thread::spawn(move || {
            let mut cb = reqwest::Url::parse(&redirect).expect("redirect url");
            cb.query_pairs_mut()
                .append_pair("code", "auth-code-live")
                .append_pair("state", &state);
            // Give the listener a beat to start accepting.
            thread::sleep(Duration::from_millis(50));
            let _ = reqwest::blocking::get(cb);
        });
        Ok(())
    };

    let tokens = oauth::login(&cfg, &[0], open_url).expect("login flow");
    assert_eq!(tokens.access_token, "at-live");
    assert_eq!(tokens.refresh_token, "rt-live");

    // Tokens were persisted to the (file-backed) secret store.
    let stored = oauth::current()
        .expect("read stored tokens")
        .expect("some tokens");
    assert_eq!(stored.access_token, "at-live");
    assert_eq!(stored.refresh_token, "rt-live");
    assert_eq!(stored.id_token.as_deref(), Some("it-live"));

    // Clean up the seams and temp dir.
    oauth::clear().ok();
    std::env::remove_var("GATE_CONNECT_TEST_TOKEN_ENDPOINT");
    std::env::remove_var("GATE_CONNECT_TEST_SECRETS");
    std::fs::remove_dir_all(&secrets).ok();
}
