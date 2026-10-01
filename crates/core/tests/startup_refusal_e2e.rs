//! Hermetic coverage of the launch-time probe's answer to a 401:
//! `startup::refresh_session` must force a refresh and ask again before it
//! signs anyone out, the same rule `session_recovery_e2e` pins for the runtime
//! paths.
//!
//! A clock that moved after the token was stamped keeps a dead token looking
//! fresh, so the first refusal is only a suspicion. Only a second refusal, with
//! a token minted seconds ago, is a verdict. And an identity provider that
//! cannot be reached is not one either: a launch is also when the network is
//! likeliest to still be coming up.
//!
//! Same seams as `session_recovery_e2e`: a file-backed secret store, a loopback
//! Cognito mock, a loopback `/v1/me/orgs` mock, and `GATE_COGNITO_*`. One test
//! function in its own binary, because the seams are process-global env vars
//! and the rejection latch the last step sets is process-global too.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gate_connect_core::env;
use gate_connect_core::oauth::{self, OAuthTokens};
use gate_connect_core::startup::{refresh_session, SessionVerdict};

/// A scripted loopback mock. Accepts forever, answering with `responses` in
/// call order and falling back to a 500 once they run out, so an over-count
/// fails the assertion rather than hanging the client. Records each request
/// head, which is where the bearer is.
struct Mock {
    base: String,
    hits: Arc<AtomicUsize>,
    heads: Arc<Mutex<Vec<String>>>,
}

impl Mock {
    /// The `x-gate-authorization` header sent on call `idx` (0-based).
    fn bearer(&self, idx: usize) -> String {
        let heads = self.heads.lock().unwrap();
        heads
            .get(idx)
            .unwrap_or_else(|| panic!("no request recorded at index {idx}"))
            .lines()
            .find_map(|l| l.split_once(':'))
            .filter(|(k, _)| k.trim().eq_ignore_ascii_case("x-gate-authorization"))
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_default()
    }
}

fn spawn_mock(responses: Vec<(&'static str, &'static str)>) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
    let base = format!("http://{}", listener.local_addr().expect("mock addr"));
    let hits = Arc::new(AtomicUsize::new(0));
    let heads = Arc::new(Mutex::new(Vec::new()));
    let responses = Arc::new(responses);
    let (hits_t, heads_t, resp_t) = (hits.clone(), heads.clone(), responses.clone());
    thread::spawn(move || loop {
        let Ok((stream, _)) = listener.accept() else {
            break;
        };
        let (hits_c, heads_c, resp_c) = (hits_t.clone(), heads_t.clone(), resp_t.clone());
        thread::spawn(move || serve_one(stream, hits_c, heads_c, resp_c));
    });
    Mock { base, hits, heads }
}

fn serve_one(
    mut stream: TcpStream,
    hits: Arc<AtomicUsize>,
    heads: Arc<Mutex<Vec<String>>>,
    responses: Arc<Vec<(&'static str, &'static str)>>,
) {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let idx = hits.fetch_add(1, Ordering::SeqCst);
    heads.lock().unwrap().push(read_request_head(&mut stream));
    let (status, body) = responses
        .get(idx)
        .copied()
        .unwrap_or(("500 Internal Server Error", r#"{"error":"unscripted"}"#));
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn read_request_head(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// A loopback address with nothing listening: the network not up yet.
fn dead_endpoint() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind for a dead port");
    let addr = listener.local_addr().expect("dead addr");
    drop(listener);
    format!("http://{addr}/oauth2/token")
}

fn temp_secrets_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gate-connect-startup-refusal-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp secrets dir");
    dir
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn tokens(access: &str, refresh: &str, expires_at_unix: i64) -> OAuthTokens {
    OAuthTokens {
        access_token: access.to_string(),
        refresh_token: refresh.to_string(),
        id_token: None,
        expires_at_unix,
        client_id: "client123".to_string(),
    }
}

const ORGS_BODY: &str = r#"{"user":{"id":"u-1","email":"dev@example.test"},
"orgs":[{"orgId":"org-uuid-1","name":"Acme","slug":"acme","role":"owner"}]}"#;
const REFUSED: &str =
    r#"{"error":{"code":"invalid_gate_token","message":"expired","source":"validation"}}"#;

#[test]
fn a_refusal_at_launch_is_renewed_before_it_signs_anyone_out() {
    // Every stored token is *locally unexpired*, so the startup refresh never
    // runs on its own: each Cognito hit is one a 401 forced.
    let cognito = spawn_mock(vec![
        (
            "200 OK",
            r#"{"access_token":"at-renewed-1","expires_in":3600,"token_type":"Bearer"}"#,
        ),
        (
            "200 OK",
            r#"{"access_token":"at-renewed-2","expires_in":3600,"token_type":"Bearer"}"#,
        ),
    ]);
    let orgs = spawn_mock(vec![
        // Step 1: the stale bearer is refused, the renewed one is taken.
        ("401 Unauthorized", REFUSED),
        ("200 OK", ORGS_BODY),
        // Step 2: refused while the identity provider is unreachable.
        ("401 Unauthorized", REFUSED),
        // Step 3: refused twice - the session really is gone.
        ("401 Unauthorized", REFUSED),
        ("401 Unauthorized", REFUSED),
    ]);

    let secrets = temp_secrets_dir();
    std::env::set_var(
        "GATE_CONNECT_TEST_TOKEN_ENDPOINT",
        format!("{}/oauth2/token", cognito.base),
    );
    std::env::set_var(
        "GATE_CONNECT_TEST_ORGS_ENDPOINT",
        format!("{}/v1/me/orgs", orgs.base),
    );
    std::env::set_var("GATE_CONNECT_TEST_SECRETS", &secrets);
    env::set_app_support_dir_for_tests(Some(secrets.clone()));
    gate_connect_core::account::save("https://gateway.example.com", None).expect("seed account");
    gate_connect_core::account::set_auth_mode(gate_connect_core::account::AuthMode::OAuth)
        .expect("set oauth mode");
    std::env::set_var("GATE_COGNITO_HOSTED_DOMAIN", "unused.in.tests");
    std::env::set_var("GATE_COGNITO_CLIENT_ID", "client123");

    let now = now_unix();

    // 1. A token the local clock thinks is fresh and the gateway refuses: the
    //    moved-clock case. The forced refresh mints one the gateway takes, so
    //    the launch is healthy and nothing is latched.
    oauth::store(&tokens("at-stale", "rt-1", now + 3600)).expect("store");
    assert!(matches!(refresh_session(), SessionVerdict::Healthy));
    assert_eq!(cognito.hits.load(Ordering::SeqCst), 1, "one forced refresh");
    assert_eq!(orgs.bearer(0), "Bearer at-stale");
    assert_eq!(
        orgs.bearer(1),
        "Bearer at-renewed-1",
        "the second probe carries the renewed token"
    );
    assert!(!oauth::session_rejected());
    assert_eq!(
        oauth::live_session().expect("still signed in").access_token,
        "at-renewed-1",
        "the renewed token is stored, so the engine seeds itself from it"
    );

    // 2. Refused while the identity provider cannot be reached. No verdict: an
    //    offline moment at launch must not sign anyone out.
    std::env::set_var("GATE_CONNECT_TEST_TOKEN_ENDPOINT", dead_endpoint());
    oauth::store(&tokens("at-stale-2", "rt-1", now + 3600)).expect("store");
    assert!(matches!(refresh_session(), SessionVerdict::Healthy));
    assert!(
        !oauth::session_rejected(),
        "an unreachable identity provider must not latch the session as rejected"
    );
    assert!(oauth::live_session().is_some());
    assert_eq!(
        orgs.hits.load(Ordering::SeqCst),
        3,
        "a renewal that failed never gets as far as a second probe"
    );

    // 3. Refused again with a token minted seconds ago: no clock explains
    //    that, so the session is dead and the launch asks for sign-in.
    std::env::set_var(
        "GATE_CONNECT_TEST_TOKEN_ENDPOINT",
        format!("{}/oauth2/token", cognito.base),
    );
    oauth::store(&tokens("at-stale-3", "rt-1", now + 3600)).expect("store");
    assert!(matches!(refresh_session(), SessionVerdict::NeedsSignIn));
    assert_eq!(cognito.hits.load(Ordering::SeqCst), 2);
    assert_eq!(orgs.bearer(4), "Bearer at-renewed-2");
    assert!(
        oauth::session_rejected(),
        "the gateway's verdict is recorded"
    );
    assert!(oauth::live_session().is_none());
    assert!(
        oauth::current().expect("bundle readable").is_some(),
        "rejection is a live-session verdict, not a reason to delete the user's tokens"
    );
    assert_eq!(orgs.hits.load(Ordering::SeqCst), 5, "no extra calls");
}
