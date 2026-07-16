//! Hermetic test that a non-2xx response from Cognito's token endpoint makes
//! `oauth::refresh` fail (the status check in `post_token`) instead of being
//! parsed as a token. A rejected refresh (e.g. a revoked refresh token ->
//! `invalid_grant`) is what drives `live_session` to report "no session", so
//! the UI prompts re-sign-in rather than riding a token that no longer works.
//!
//! Sync + hermetic: a raw-TCP mock on a background thread stands in for the
//! token endpoint (`GATE_CONNECT_TEST_TOKEN_ENDPOINT`); `oauth::refresh` takes
//! an explicit config, so no keychain and no `from_build_env`. Its own test
//! binary keeps the process-global endpoint seam isolated from the other oauth
//! tests, and the single request avoids reusing the mock connection.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use gate_connect_core::oauth::{self, OAuthConfig};

/// Serve exactly one response (status line + JSON body) on a loopback token
/// endpoint, then stop.
fn spawn_token_mock(status: &'static str, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind token mock");
    let addr = listener.local_addr().expect("addr");
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut tmp = [0u8; 2048];
            let _ = stream.read(&mut tmp); // drain the request; not asserted here
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("http://{addr}/oauth2/token")
}

fn cfg() -> OAuthConfig {
    OAuthConfig {
        hosted_domain: "unused.in.tests".to_string(),
        client_id: "client123".to_string(),
        scopes: vec!["openid".to_string()],
    }
}

#[test]
fn refresh_rejects_non_success_status() {
    let endpoint = spawn_token_mock("400 Bad Request", r#"{"error":"invalid_grant"}"#);
    std::env::set_var("GATE_CONNECT_TEST_TOKEN_ENDPOINT", &endpoint);

    let err = oauth::refresh(&cfg(), "revoked-refresh-token")
        .expect_err("a 400 from the token endpoint must fail, not parse as tokens");

    // The error carries the status and body so a failed refresh is diagnosable
    // in logs (and is distinct from a network error).
    let msg = format!("{err:#}");
    assert!(msg.contains("400"), "error should name the status: {msg}");
    assert!(
        msg.contains("invalid_grant"),
        "error should carry the response body: {msg}"
    );

    std::env::remove_var("GATE_CONNECT_TEST_TOKEN_ENDPOINT");
}
