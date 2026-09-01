//! Hermetic coverage of what the app does when the gateway refuses the bearer
//! it is holding: `org::list_current` (control plane) and
//! `startup::reverify_session` (what a data-plane 401 routes to).
//!
//! The behavior under test is recovery, not reporting. A 401 does not prove
//! the session is dead - the local expiry check the app trusts compares two
//! readings of a clock that can move between them, so a machine coming back
//! from sleep can hold a token that is locally minutes old and hours expired
//! as far as the gateway is concerned. Neither path may conclude anything
//! from the refusal until it has forced a refresh past that check and asked
//! again; only a second refusal means sign-in.
//!
//! And "the renewal failed" is not "the session is dead". The machine that
//! just woke up is also the machine whose network is still coming up, so the
//! renewal attempt is *likelier* than usual to fail on connect at exactly
//! this moment. Steps 3 and 4 pin that: an unreachable identity provider
//! leaves the session alone, because latching a rejection there would turn
//! the outage this path exists to recover from into a permanent sign-out.
//!
//! Fully hermetic, on the same seams as `oauth_session_e2e`: a file-backed
//! secret store, a loopback Cognito mock, a loopback `/v1/me/orgs` mock, and
//! `GATE_COGNITO_*` so `OAuthConfig::from_build_env()` returns `Some`. One
//! test function, because those seams are process-global env vars - and
//! because the rejection latch the last step sets is process-global too, so
//! the steps have to run in this order.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gate_connect_core::env;
use gate_connect_core::oauth::{self, OAuthTokens};
use gate_connect_core::startup::{reverify_session, Recheck};

/// A scripted loopback mock. Accepts forever, answering with `responses` in
/// call order (status line + JSON body) and falling back to a 500 once they
/// run out, so an over-count fails the assertion rather than hanging the
/// client. Records each request head, which is where the bearer is.
struct Mock {
    base: String,
    hits: Arc<AtomicUsize>,
    heads: Arc<Mutex<Vec<String>>>,
}

impl Mock {
    /// The `authorization`-style header sent on call `idx` (0-based).
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
        // One thread per connection: the org calls and the token calls are
        // separate mocks, but a hung read on either must not stall the next.
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

/// Read the request line + headers. Enough for both mocks: the orgs call is a
/// GET with no body, and the token call's body is not what this test asserts on.
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

/// A loopback address with nothing listening: bind, read the port, drop. The
/// faithful shape of "the network is not up yet" - connect fails outright.
fn dead_endpoint() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind for a dead port");
    let addr = listener.local_addr().expect("dead addr");
    drop(listener);
    format!("http://{addr}/oauth2/token")
}

fn temp_secrets_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gate-connect-session-recovery-{}-{}",
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
fn a_refused_bearer_is_renewed_before_it_is_believed() {
    // Every token in the store below is *locally unexpired*, so nothing here
    // refreshes on its own: each refresh that happens is one a 401 forced.
    let cognito = spawn_mock(vec![
        (
            "200 OK",
            r#"{"access_token":"at-renewed-1","expires_in":3600,"token_type":"Bearer"}"#,
        ),
        (
            "200 OK",
            r#"{"access_token":"at-renewed-2","expires_in":3600,"token_type":"Bearer"}"#,
        ),
        (
            "200 OK",
            r#"{"access_token":"at-renewed-3","expires_in":3600,"token_type":"Bearer"}"#,
        ),
    ]);
    let orgs = spawn_mock(vec![
        // Step 1: the stale bearer is refused, the renewed one is taken.
        ("401 Unauthorized", REFUSED),
        ("200 OK", ORGS_BODY),
        // Step 2: the data-plane re-verification's probe.
        ("200 OK", ORGS_BODY),
        // Step 3: refused while the identity provider is unreachable.
        ("401 Unauthorized", REFUSED),
        // Step 5: refused twice - the session really is gone.
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

    // 1. The control plane. The stored token looks fine locally - this is the
    //    post-resume state, where only the gateway knows better - so the 401
    //    must be answered by renewing and retrying, not by reporting.
    oauth::store(&tokens("at-stale", "rt-1", now + 3600)).expect("store");
    let listed = gate_connect_core::org::list_current().expect("recovered and listed");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].org_id, "org-uuid-1");
    assert_eq!(
        cognito.hits.load(Ordering::SeqCst),
        1,
        "exactly one forced refresh: the 401 is what triggered it"
    );
    assert_eq!(
        orgs.bearer(0),
        "Bearer at-stale",
        "first try used the stored token"
    );
    assert_eq!(
        orgs.bearer(1),
        "Bearer at-renewed-1",
        "the retry must carry the renewed token, not the refused one"
    );
    assert_eq!(
        oauth::live_session().expect("still signed in").access_token,
        "at-renewed-1",
        "the renewed token is persisted, so the next caller starts from it"
    );

    // 2. The data plane's answer to a 401 seen on a proxied call: same forced
    //    renewal, then the gateway is asked directly. It accepts, so the
    //    session lives and the caller gets a token to re-seed routing with.
    match reverify_session() {
        Recheck::Recovered(token) => assert_eq!(token, "at-renewed-2"),
        Recheck::Dead => panic!("an accepted probe must not read as dead"),
        Recheck::Unchanged => panic!("an accepted probe is a verdict, not a shrug"),
    }
    assert_eq!(cognito.hits.load(Ordering::SeqCst), 2);

    // 3. The 401 arrives while the identity provider cannot be reached - the
    //    resume-from-sleep case, where the network is still coming up. The
    //    call fails, but nothing may be concluded: a network that is down
    //    cannot testify about a credential.
    std::env::set_var("GATE_CONNECT_TEST_TOKEN_ENDPOINT", dead_endpoint());
    let err = gate_connect_core::org::list_current()
        .expect_err("a 401 the renewal could not answer still fails the call");
    assert!(
        format!("{err:#}").contains("401 Unauthorized"),
        "the user is still shown what the gateway said: {err:#}"
    );
    assert!(
        !oauth::session_rejected(),
        "an unreachable identity provider must not latch the session as rejected"
    );
    assert!(
        oauth::live_session().is_some(),
        "the session must keep reading as live: nothing has refused it"
    );

    // 4. Same rule on the data plane's path. `Unchanged`, not `Dead` - a
    //    verdict needs the gateway or Cognito to have actually answered.
    match reverify_session() {
        Recheck::Unchanged => {}
        Recheck::Dead => panic!("an unreachable identity provider is not a dead session"),
        Recheck::Recovered(_) => panic!("nothing was renewed; there is nothing to recover with"),
    }
    assert!(!oauth::session_rejected());
    assert_eq!(
        cognito.hits.load(Ordering::SeqCst),
        2,
        "steps 3 and 4 never reached the token endpoint"
    );

    // 5. Refused again with a token minted seconds ago: no clock explains
    //    that, so the session is dead and the app must be told - this is what
    //    drops the tray, the status and the injector to signed-out together.
    std::env::set_var(
        "GATE_CONNECT_TEST_TOKEN_ENDPOINT",
        format!("{}/oauth2/token", cognito.base),
    );
    let err = gate_connect_core::org::list_current().expect_err("a twice-refused session fails");
    assert!(
        format!("{err:#}").contains("401 Unauthorized"),
        "the gateway's refusal is what the user is shown: {err:#}"
    );
    assert!(
        oauth::session_rejected(),
        "the gateway's verdict is recorded"
    );
    assert!(
        oauth::live_session().is_none(),
        "a rejected session must stop reading as live everywhere, not just on the screen that saw it"
    );
    assert!(
        oauth::current().expect("bundle readable").is_some(),
        "rejection is a live-session verdict, not a reason to delete the user's tokens"
    );
    assert_eq!(orgs.hits.load(Ordering::SeqCst), 6, "no extra calls");

    // 6. The other verdict: Cognito itself refuses the renewal (a revoked or
    //    expired refresh token). That is an answer, not silence, so it is a
    //    verdict - the half of the distinction step 3 must not blur.
    //    Storing a bundle clears the latch step 5 set, the way a re-login
    //    would, so this step asserts something.
    oauth::store(&tokens("at-fresh", "rt-revoked", now + 3600)).expect("store");
    assert!(
        !oauth::session_rejected(),
        "a stored bundle clears the latch"
    );
    let refusing = spawn_mock(vec![("400 Bad Request", r#"{"error":"invalid_grant"}"#)]);
    std::env::set_var(
        "GATE_CONNECT_TEST_TOKEN_ENDPOINT",
        format!("{}/oauth2/token", refusing.base),
    );
    match reverify_session() {
        Recheck::Dead => {}
        Recheck::Unchanged => panic!("a refused refresh token is a verdict, not an outage"),
        Recheck::Recovered(_) => panic!("nothing was renewed"),
    }
    assert!(oauth::session_rejected());
    assert_eq!(
        orgs.hits.load(Ordering::SeqCst),
        6,
        "a refused renewal never gets as far as probing the gateway"
    );
}
