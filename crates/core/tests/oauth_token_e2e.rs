//! Live token-exchange / refresh test for the `oauth` module against a tiny
//! loopback mock of Cognito's `/oauth2/token` endpoint.
//!
//! Fully hermetic and sync: a raw-TCP mock server runs on a background
//! thread (so `reqwest::blocking` in `oauth` isn't called from inside a
//! tokio runtime), and the `GATE_CONNECT_TEST_TOKEN_ENDPOINT` seam points
//! the exchange at it over plain HTTP - no TLS, no network. It asserts the
//! request the app sends and the tokens it derives from the response.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use gate_connect_core::oauth::{self, OAuthConfig};

/// One recorded request body plus the canned response the mock returned.
struct Exchange {
    body: String,
}

/// Serve `responses.len()` sequential requests, returning each canned JSON
/// body in order. Sends every received request body back over `tx`.
fn spawn_mock(responses: Vec<&'static str>) -> (String, mpsc::Receiver<Exchange>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("addr");
    let endpoint = format!("http://{addr}/oauth2/token");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for resp_body in responses {
            let (mut stream, _) = listener.accept().expect("accept");
            // Read headers, then the Content-Length-delimited body.
            let mut buf = Vec::new();
            let mut tmp = [0u8; 1024];
            let header_end = loop {
                let n = stream.read(&mut tmp).expect("read");
                if n == 0 {
                    break None;
                }
                buf.extend_from_slice(&tmp[..n]);
                if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
                    break Some(pos + 4);
                }
            };
            let header_end = header_end.expect("request headers");
            let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
            let content_length = headers
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    (k.trim().eq_ignore_ascii_case("content-length"))
                        .then(|| v.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            while buf.len() < header_end + content_length {
                let n = stream.read(&mut tmp).expect("read body");
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            let body =
                String::from_utf8_lossy(&buf[header_end..header_end + content_length]).to_string();
            tx.send(Exchange { body }).expect("send captured");

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                resp_body.len(),
                resp_body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
            stream.flush().ok();
        }
    });
    (endpoint, rx)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn cfg() -> OAuthConfig {
    OAuthConfig {
        hosted_domain: "unused.in.tests".to_string(),
        client_id: "client123".to_string(),
        scopes: vec!["openid".to_string()],
    }
}

/// Parse an `application/x-www-form-urlencoded` body into a map.
fn form_pairs(body: &str) -> std::collections::HashMap<String, String> {
    url_decode_pairs(body)
}

fn url_decode_pairs(body: &str) -> std::collections::HashMap<String, String> {
    body.split('&')
        .filter(|s| !s.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let s = s.replace('+', " ");
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[test]
fn code_then_refresh_grant_round_trip() {
    // One mock serves both requests in order so the two exchanges share a
    // single process-global endpoint seam without racing.
    let (endpoint, rx) = spawn_mock(vec![
        r#"{"access_token":"at-1","refresh_token":"rt-1","id_token":"it-1","expires_in":3600,"token_type":"Bearer"}"#,
        // Cognito's refresh response omits refresh_token unless rotation is on.
        r#"{"access_token":"at-2","id_token":"it-2","expires_in":3600,"token_type":"Bearer"}"#,
    ]);
    std::env::set_var("GATE_CONNECT_TEST_TOKEN_ENDPOINT", &endpoint);

    // 1. Authorization-code grant sends PKCE + code and parses tokens.
    let tokens = oauth::complete_login(
        &cfg(),
        "auth-code-xyz",
        "verifier-abc",
        "http://127.0.0.1:52847/callback",
    )
    .expect("complete_login");

    let sent = rx.recv().expect("captured code-grant request");
    let pairs = form_pairs(&sent.body);
    assert_eq!(pairs["grant_type"], "authorization_code");
    assert_eq!(pairs["client_id"], "client123");
    assert_eq!(pairs["code"], "auth-code-xyz");
    assert_eq!(pairs["code_verifier"], "verifier-abc");
    assert_eq!(pairs["redirect_uri"], "http://127.0.0.1:52847/callback");

    assert_eq!(tokens.access_token, "at-1");
    assert_eq!(tokens.refresh_token, "rt-1");
    assert_eq!(tokens.id_token.as_deref(), Some("it-1"));

    // 2. Refresh grant carries the prior refresh token forward.
    let refreshed = oauth::refresh(&cfg(), "rt-original").expect("refresh");

    let sent = rx.recv().expect("captured refresh-grant request");
    let pairs = form_pairs(&sent.body);
    assert_eq!(pairs["grant_type"], "refresh_token");
    assert_eq!(pairs["client_id"], "client123");
    assert_eq!(pairs["refresh_token"], "rt-original");

    assert_eq!(refreshed.access_token, "at-2");
    assert_eq!(refreshed.refresh_token, "rt-original"); // carried forward

    std::env::remove_var("GATE_CONNECT_TEST_TOKEN_ENDPOINT");
}
