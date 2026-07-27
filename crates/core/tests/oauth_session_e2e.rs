//! Hermetic test for the refresh-on-read session logic: `oauth::live_session`
//! and the injector `oauth::access_token_for_injection`.
//!
//! These are the single source of truth behind both the token the engine
//! injects and the signed-in state the UI shows, so this pins the behavior the
//! rest of the branch relies on: a still-valid token passes through untouched,
//! an expired one is silently refreshed (not dropped to the API-key fallback),
//! and a dead refresh reports "no session" without wiping the stored bundle.
//!
//! Fully hermetic: the secret store is file-backed (`GATE_CONNECT_TEST_SECRETS`),
//! the Cognito token exchange is pointed at a raw-TCP loopback mock
//! (`GATE_CONNECT_TEST_TOKEN_ENDPOINT`), and `GATE_COGNITO_*` make
//! `OAuthConfig::from_build_env()` (which `live_session` calls) return `Some`.
//! One test function because those seams are process-global env vars.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gate_connect_core::env;
use gate_connect_core::oauth::{self, OAuthTokens};

/// A loopback mock of Cognito's `/oauth2/token`. Accepts forever on a background
/// thread, handing out `responses` in call order (status line + JSON body) and
/// falling back to a 400 once they're exhausted, so an over-count never wedges
/// the client. Records each request body and counts hits so a test can assert
/// both *what* was sent and *whether* a refresh happened at all.
struct TokenMock {
    endpoint: String,
    hits: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<String>>>,
}

fn spawn_token_mock(responses: Vec<(&'static str, &'static str)>) -> TokenMock {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind token mock");
    let addr = listener.local_addr().expect("addr");
    let endpoint = format!("http://{addr}/oauth2/token");
    let hits = Arc::new(AtomicUsize::new(0));
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let responses = Arc::new(responses);
    let (hits_t, bodies_t, resp_t) = (hits.clone(), bodies.clone(), responses.clone());
    thread::spawn(move || loop {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(_) => break,
        };
        // Handle each connection on its own thread so a slow/hung read never
        // stalls accepting (and answering) the next refresh call.
        let (hits_c, bodies_c, resp_c) = (hits_t.clone(), bodies_t.clone(), resp_t.clone());
        thread::spawn(move || serve_one(stream, hits_c, bodies_c, resp_c));
    });
    TokenMock {
        endpoint,
        hits,
        bodies,
    }
}

/// Read one request off `stream` and answer with the response at this call's
/// index (or a 400 once the scripted responses run out).
fn serve_one(
    mut stream: TcpStream,
    hits: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<String>>>,
    responses: Arc<Vec<(&'static str, &'static str)>>,
) {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let idx = hits.fetch_add(1, Ordering::SeqCst);
    bodies.lock().unwrap().push(read_request_body(&mut stream));
    let (status, resp_body) = responses
        .get(idx)
        .copied()
        .unwrap_or(("400 Bad Request", r#"{"error":"invalid_grant"}"#));
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        resp_body.len(),
        resp_body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// Read one HTTP request off `stream` and return its body (headers +
/// Content-Length-delimited). Tolerant of read errors/timeouts (returns what it
/// has) so a stray connection can never wedge the mock thread.
fn read_request_body(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    let header_end = loop {
        match stream.read(&mut tmp) {
            Ok(0) | Err(_) => break None,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
                    break Some(pos + 4);
                }
            }
        }
    };
    let Some(header_end) = header_end else {
        return String::new();
    };
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
        match stream.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
    let end = (header_end + content_length).min(buf.len());
    String::from_utf8_lossy(&buf[header_end..end]).into_owned()
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// A unique temp dir for the file-backed secret store.
fn temp_secrets_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gate-connect-oauth-session-{}-{}",
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
        // Matches the GATE_COGNITO_CLIENT_ID the test sets, so these bundles
        // read as minted by the current build (step 6 covers the mismatch).
        client_id: "client123".to_string(),
    }
}

#[test]
fn live_session_passes_through_valid_refreshes_expired_and_reports_dead() {
    // The mock serves the one successful refresh this test performs (step 3);
    // the failure case (step 4) points at a closed port instead, which is both
    // faster and a faithful trigger of the same `refresh` error path.
    let mock = spawn_token_mock(vec![(
        "200 OK",
        // Cognito omits refresh_token on the refresh grant unless rotation is
        // on, so the prior one should be carried forward.
        r#"{"access_token":"at-refreshed","expires_in":3600,"token_type":"Bearer"}"#,
    )]);
    let secrets = temp_secrets_dir();
    std::env::set_var("GATE_CONNECT_TEST_TOKEN_ENDPOINT", &mock.endpoint);
    std::env::set_var("GATE_CONNECT_TEST_SECRETS", &secrets);
    // Pin the data dir at a throwaway location so `from_build_env()` reads our
    // seeded `account.json` (below) rather than the real machine's, and so the
    // keychain seam stores tokens under it.
    env::set_app_support_dir_for_tests(Some(secrets.clone()));
    // Seed an OAuth-mode account. The gateway host is deliberately non-staging,
    // so `from_build_env()` takes the prod branch matching the prod-name vars
    // below; the auth mode is what `access_token_for_injection` gates on.
    gate_connect_core::account::save("https://gateway.example.com", None).expect("seed account");
    gate_connect_core::account::set_auth_mode(gate_connect_core::account::AuthMode::OAuth)
        .expect("set oauth mode");
    // `live_session` derives its config from the environment; these make
    // `from_build_env()` return Some. The token endpoint is overridden by the
    // seam above, so the domain value itself is never contacted.
    std::env::set_var("GATE_COGNITO_HOSTED_DOMAIN", "unused.in.tests");
    std::env::set_var("GATE_COGNITO_CLIENT_ID", "client123");

    let now = now_unix();

    // 1. No stored bundle: no session, empty injector, and no network attempt.
    oauth::clear().expect("clear");
    assert!(oauth::live_session().is_none(), "no bundle => no session");
    assert_eq!(
        oauth::access_token_for_injection(),
        "",
        "no bundle => fall back to the API key"
    );
    assert_eq!(
        mock.hits.load(Ordering::SeqCst),
        0,
        "a missing bundle must not hit the token endpoint"
    );

    // 2. Valid (unexpired) token: passthrough, unchanged, still no network.
    oauth::store(&tokens("at-valid", "rt-valid", now + 3600)).expect("store valid");
    let session = oauth::live_session().expect("valid session");
    assert_eq!(session.access_token, "at-valid");
    assert_eq!(session.refresh_token, "rt-valid", "left untouched");
    assert_eq!(oauth::access_token_for_injection(), "at-valid");
    assert_eq!(
        mock.hits.load(Ordering::SeqCst),
        0,
        "an unexpired token must not trigger a refresh"
    );

    // 3. Expired token: silently refreshed to a fresh, non-empty token (the
    //    whole fix - it must not drop to the API-key fallback), and persisted.
    oauth::store(&tokens("at-stale", "rt-expired", now - 100)).expect("store expired");
    let refreshed = oauth::live_session().expect("refreshed session");
    assert_eq!(
        refreshed.access_token, "at-refreshed",
        "returns the refreshed token, not an empty string"
    );
    assert_eq!(
        refreshed.refresh_token, "rt-expired",
        "prior refresh token carried forward when the response omits one"
    );
    assert_eq!(
        mock.hits.load(Ordering::SeqCst),
        1,
        "exactly one refresh call"
    );
    let sent = &mock.bodies.lock().unwrap()[0];
    assert!(sent.contains("grant_type=refresh_token"), "sent: {sent}");
    assert!(sent.contains("refresh_token=rt-expired"), "sent: {sent}");

    // The refreshed bundle was persisted, so a subsequent read reuses it with no
    // second network round-trip.
    let persisted = oauth::current().expect("read").expect("some");
    assert_eq!(persisted.access_token, "at-refreshed");
    assert!(persisted.expires_at_unix > now, "expiry advanced");
    assert_eq!(oauth::access_token_for_injection(), "at-refreshed");
    assert_eq!(
        mock.hits.load(Ordering::SeqCst),
        1,
        "a freshly-refreshed token is reused, not refreshed again"
    );

    // 4. Expired token whose refresh fails (revoked token / Cognito unreachable):
    //    no session and an empty injector (API-key fallback), matching the
    //    signed-out state the UI reports - but the stale bundle is kept so a
    //    transient failure recovers on a later refresh instead of signing the
    //    user out. Point the endpoint at a just-closed port so `refresh` errors.
    let dead_port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let p = l.local_addr().unwrap().port();
        drop(l); // release it so the connection is refused
        p
    };
    std::env::set_var(
        "GATE_CONNECT_TEST_TOKEN_ENDPOINT",
        format!("http://127.0.0.1:{dead_port}/oauth2/token"),
    );
    oauth::store(&tokens("at-stale-2", "rt-dead", now - 100)).expect("store expired 2");
    assert!(
        oauth::live_session().is_none(),
        "a failed refresh => no usable session"
    );
    assert_eq!(
        oauth::access_token_for_injection(),
        "",
        "a dead session falls back to the API key"
    );
    assert!(
        oauth::current().expect("read").is_some(),
        "a failed refresh must not clear the stored bundle"
    );

    // 5. Auth mode gates injection independently of the session. A valid OAuth
    //    session persists (so the user can switch back without re-authenticating),
    //    but in ApiKey mode the injector returns empty - a freshly pasted Gate
    //    key must not be overridden by a still-live bearer.
    oauth::store(&tokens("at-mode-gated", "rt-mode", now + 3600)).expect("store valid");
    gate_connect_core::account::set_auth_mode(gate_connect_core::account::AuthMode::ApiKey)
        .expect("switch to api-key mode");
    assert!(
        oauth::live_session().is_some(),
        "the OAuth session survives an auth-mode switch"
    );
    assert_eq!(
        oauth::access_token_for_injection(),
        "",
        "ApiKey mode must not inject the OAuth bearer"
    );
    gate_connect_core::account::set_auth_mode(gate_connect_core::account::AuthMode::OAuth)
        .expect("restore oauth mode");
    assert_eq!(
        oauth::access_token_for_injection(),
        "at-mode-gated",
        "OAuth mode injects the live token again"
    );

    // 6. Unexpired bundle minted by a *different* app client (an upgrade that
    //    switched Cognito pools, or a pre-stamp legacy bundle surviving in the
    //    OS secret store): no session and an empty injector, with no network
    //    attempt - the endpoint still points at the dead port from step 4, so
    //    any refresh try would hang this assertion on a connection error
    //    instead of the local rejection. The bundle is kept, so the tray's
    //    dead-session signal (stored-but-unusable) fires.
    let mut legacy = tokens("at-wrong-client", "rt-wrong-client", now + 3600);
    legacy.client_id = String::new(); // pre-release bundles carry no stamp
    oauth::store(&legacy).expect("store legacy");
    assert!(
        oauth::live_session().is_none(),
        "a bundle from another app client => no usable session, however fresh"
    );
    assert_eq!(
        oauth::access_token_for_injection(),
        "",
        "a mismatched bundle must never be injected"
    );
    assert!(
        oauth::current().expect("read").is_some(),
        "the mismatched bundle is kept so the dead-session signal can fire"
    );

    // 7. A gateway rejection (the startup probe's 401 verdict) kills an
    //    otherwise perfectly valid session: status and injection must both go
    //    dark, the bundle is kept (dead-session signal), and storing fresh
    //    tokens (re-login) lifts the verdict.
    oauth::store(&tokens("at-doomed", "rt-doomed", now + 3600)).expect("store valid");
    assert!(
        oauth::live_session().is_some(),
        "sanity: the fresh bundle reads as live before the verdict"
    );
    oauth::mark_session_rejected();
    assert!(
        oauth::live_session().is_none(),
        "a gateway rejection outranks local freshness"
    );
    assert_eq!(
        oauth::access_token_for_injection(),
        "",
        "a rejected session must not be injected"
    );
    assert!(
        oauth::current().expect("read").is_some(),
        "the rejected bundle is kept so the dead-session signal can fire"
    );
    oauth::store(&tokens("at-relogin", "rt-relogin", now + 3600)).expect("store relogin");
    assert_eq!(
        oauth::access_token_for_injection(),
        "at-relogin",
        "a new login stores fresh tokens and lifts the rejection"
    );

    // Clean up the seams and temp dir.
    oauth::clear().ok();
    for key in [
        "GATE_CONNECT_TEST_TOKEN_ENDPOINT",
        "GATE_CONNECT_TEST_SECRETS",
        "GATE_COGNITO_HOSTED_DOMAIN",
        "GATE_COGNITO_CLIENT_ID",
    ] {
        std::env::remove_var(key);
    }
    std::fs::remove_dir_all(&secrets).ok();
}
