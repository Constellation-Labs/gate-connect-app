//! Every reader that finds the session expired at once gets one refresh, not
//! one each, and gets its outcome whichever way it goes.
//!
//! After a sleep or a reboot the stored token is expired for everyone at the
//! same moment: startup, the security feed, the window's reads and the 30s tick
//! all refresh. Each used to mint its own token and store it, and two of those
//! stores overlapping deadlocked the macOS keychain - the app came up with
//! blank windows and no proxy, with nothing in the log.
//!
//! Hermetic: the file-backed secret store and a loopback Cognito mock that can
//! hold its answers until the test releases them, so a refresh can be kept in
//! flight while the test lines other callers up behind it or changes the
//! session under it. One test function, because the seams and the session
//! state are process-global and the steps run in order.
//!
//! The one timing assumption: callers spawned while a refresh is held read the
//! session within `SETTLE` of starting. Each of them only reads two atomics and
//! a file before queueing, so `SETTLE` is generous by orders of magnitude.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use gate_connect_core::env;
use gate_connect_core::oauth::{self, OAuthTokens, SessionReading};

const SETTLE: Duration = Duration::from_millis(300);

/// A token endpoint the test steers. Answers with whatever `script` holds next
/// (a renewal `at-renewed-<n>` once it is empty), and while `held` is set,
/// waits for [`Cognito::release`] before answering.
#[derive(Clone)]
struct Cognito {
    hits: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<String>>>,
    script: Arc<Mutex<VecDeque<(&'static str, &'static str)>>>,
    gate: Arc<(Mutex<bool>, Condvar)>,
}

impl Cognito {
    fn spawn() -> (String, Cognito) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind cognito mock");
        let addr = listener.local_addr().expect("mock addr");
        let mock = Cognito {
            hits: Arc::new(AtomicUsize::new(0)),
            bodies: Arc::new(Mutex::new(Vec::new())),
            script: Arc::new(Mutex::new(VecDeque::new())),
            gate: Arc::new((Mutex::new(false), Condvar::new())),
        };
        let served = mock.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let mock = served.clone();
                thread::spawn(move || mock.answer(stream));
            }
        });
        (format!("http://{addr}/oauth2/token"), mock)
    }

    fn answer(&self, mut stream: std::net::TcpStream) {
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap_or(0);
        self.bodies
            .lock()
            .unwrap()
            .push(String::from_utf8_lossy(&buf[..n]).into_owned());
        let count = self.hits.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let (held, wake) = &*self.gate;
            let mut held = held.lock().unwrap();
            while *held {
                held = wake.wait(held).unwrap();
            }
        }
        let (status, body) = match self.script.lock().unwrap().pop_front() {
            Some((status, body)) => (status, body.to_string()),
            None => (
                "200 OK",
                format!(
                    r#"{{"access_token":"at-renewed-{count}","expires_in":3600,"token_type":"Bearer"}}"#
                ),
            ),
        };
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    }

    fn hold(&self) {
        *self.gate.0.lock().unwrap() = true;
    }

    fn release(&self) {
        *self.gate.0.lock().unwrap() = false;
        self.gate.1.notify_all();
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    /// Wait until `n` requests have arrived, so a refresh is known to be in
    /// flight (and holding the refresh lock) before the test moves on.
    fn await_hits(&self, n: usize) {
        for _ in 0..500 {
            if self.hits() >= n {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("Cognito never saw request {n}");
    }

    fn script(&self, status: &'static str, body: &'static str) {
        self.script.lock().unwrap().push_back((status, body));
    }

    fn last_body(&self) -> String {
        self.bodies
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or_default()
    }
}

/// Removes the test's directories and puts the process-global seams back,
/// whether the test passes or not.
struct Seams {
    base: std::path::PathBuf,
}

impl Drop for Seams {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
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

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn bundle(access: &str, refresh: &str, client: &str, expires_at_unix: i64) -> OAuthTokens {
    OAuthTokens {
        access_token: access.to_string(),
        refresh_token: refresh.to_string(),
        id_token: None,
        expires_at_unix,
        client_id: client.to_string(),
    }
}

fn store_expired() {
    oauth::store(&bundle("at-expired", "rt-1", "client123", now_unix() - 60)).expect("store");
}

/// Start `n` callers of `f` while Cognito is held, give them `SETTLE` to read
/// the session and queue, then release Cognito and collect what each returned.
fn queued_behind_one<T: Send + 'static>(cognito: &Cognito, n: usize, f: fn() -> T) -> Vec<T> {
    cognito.hold();
    let before = cognito.hits();
    let handles: Vec<_> = (0..n).map(|_| thread::spawn(f)).collect();
    cognito.await_hits(before + 1);
    thread::sleep(SETTLE);
    cognito.release();
    handles
        .into_iter()
        .map(|h| h.join().expect("caller panicked"))
        .collect()
}

fn live() -> Option<String> {
    oauth::live_session().map(|t| t.access_token)
}

fn forced() -> Result<Option<String>, bool> {
    let cfg = oauth::OAuthConfig::from_build_env().expect("cognito env set");
    oauth::force_refresh(&cfg)
        .map(|t| t.map(|t| t.access_token))
        .map_err(|e| e.is_refusal())
}

fn reading() -> &'static str {
    match oauth::session_reading() {
        SessionReading::Live(_) => "live",
        SessionReading::SignedOut => "signed-out",
        SessionReading::Unavailable => "unavailable",
    }
}

fn stored_access() -> Option<String> {
    oauth::current()
        .expect("store readable")
        .map(|t| t.access_token)
}

#[test]
fn readers_of_an_expired_session_share_one_refresh() {
    let (endpoint, cognito) = Cognito::spawn();
    let base = std::env::temp_dir().join(format!(
        "gate-connect-single-flight-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _seams = Seams { base: base.clone() };
    let (secrets, support) = (base.join("secrets"), base.join("support"));
    std::fs::create_dir_all(&secrets).expect("create secrets dir");
    std::fs::create_dir_all(&support).expect("create support dir");
    std::env::set_var("GATE_CONNECT_TEST_TOKEN_ENDPOINT", &endpoint);
    std::env::set_var("GATE_CONNECT_TEST_SECRETS", &secrets);
    env::set_app_support_dir_for_tests(Some(support));
    gate_connect_core::account::save("https://gateway.example.com", None).expect("seed account");
    gate_connect_core::account::set_auth_mode(gate_connect_core::account::AuthMode::OAuth)
        .expect("set oauth mode");
    std::env::set_var("GATE_COGNITO_HOSTED_DOMAIN", "unused.in.tests");
    std::env::set_var("GATE_COGNITO_CLIENT_ID", "client123");

    // 1. The reboot: everyone reads the expired session at once, and all get
    //    the one refresh.
    store_expired();
    let seen = queued_behind_one(&cognito, 8, live);
    assert!(
        seen.iter().all(|t| t.as_deref() == Some("at-renewed-1")),
        "every reader gets the one refresh: {seen:?}"
    );
    assert_eq!(cognito.hits(), 1, "one Cognito call, not eight");

    // 2. The 401 re-check, raised by several refused requests together. Each
    //    wants a token newer than the refused one, and the first refresh is
    //    exactly that, so the others take it.
    let seen = queued_behind_one(&cognito, 4, forced);
    assert!(
        seen.iter()
            .all(|t| t.as_ref().ok().cloned().flatten().as_deref() == Some("at-renewed-2")),
        "every forced caller gets the one forced refresh: {seen:?}"
    );
    assert_eq!(cognito.hits(), 2);

    // 3. A later forced refresh, with nothing in flight, still refreshes.
    assert_eq!(forced(), Ok(Some("at-renewed-3".to_string())));
    assert_eq!(cognito.hits(), 3);

    // 4. Cognito does not answer usefully (the network after a wake). The
    //    callers that waited get the same failure rather than each trying in
    //    turn, which would put every one of them behind every earlier one's
    //    timeout.
    store_expired();
    cognito.script("503 Service Unavailable", r#"{"error":"unavailable"}"#);
    let seen = queued_behind_one(&cognito, 6, reading);
    assert!(
        seen.iter().all(|r| *r == "unavailable"),
        "every waiter shares the outage: {seen:?}"
    );
    assert_eq!(cognito.hits(), 4, "one attempt, not six");

    // 5. Same for a refusal: one `invalid_grant`, read as signed out by all.
    cognito.script("400 Bad Request", r#"{"error":"invalid_grant"}"#);
    let seen = queued_behind_one(&cognito, 6, reading);
    assert!(
        seen.iter().all(|r| *r == "signed-out"),
        "every waiter shares the refusal: {seen:?}"
    );
    assert_eq!(cognito.hits(), 5);

    // 6. A failure is only shared with callers that waited on it: the next
    //    read after it tries again.
    assert_eq!(live().as_deref(), Some("at-renewed-6"));
    assert_eq!(cognito.hits(), 6);

    // 7. A sign-out lands while a refresh is in flight. The refresh must not
    //    store its result over it - that would sign the user back in.
    store_expired();
    cognito.hold();
    let refreshing = thread::spawn(live);
    cognito.await_hits(7);
    oauth::clear().expect("sign out");
    cognito.release();
    assert_eq!(
        refreshing.join().unwrap(),
        None,
        "the refresh reports the sign-out"
    );
    assert_eq!(stored_access(), None, "and the sign-out stands");

    // 8. Another build's sign-in lands meanwhile (a bundle minted by another
    //    Cognito client). It is not overwritten, and the refresh says why.
    store_expired();
    cognito.hold();
    let refreshing = thread::spawn(forced);
    cognito.await_hits(8);
    oauth::store(&bundle(
        "at-other-pool",
        "rt-other",
        "other-client",
        now_unix() + 3600,
    ))
    .expect("store");
    cognito.release();
    assert_eq!(
        refreshing.join().unwrap(),
        Err(true),
        "a bundle from another client is refused, not refreshed or replaced"
    );
    assert_eq!(stored_access().as_deref(), Some("at-other-pool"));

    // 9. A different session, already expired, is stored meanwhile. The
    //    refresh of the old one is dropped as a no-verdict, and the next read
    //    refreshes the new one, with its own refresh token.
    store_expired();
    cognito.hold();
    let refreshing = thread::spawn(reading);
    cognito.await_hits(9);
    oauth::store(&bundle("at-replaced", "rt-2", "client123", now_unix() - 60)).expect("store");
    cognito.release();
    assert_eq!(refreshing.join().unwrap(), "unavailable");
    assert_eq!(
        stored_access().as_deref(),
        Some("at-replaced"),
        "not overwritten"
    );
    assert_eq!(live().as_deref(), Some("at-renewed-10"));
    assert!(
        cognito.last_body().contains("refresh_token=rt-2"),
        "the new session is refreshed with its own refresh token"
    );
}
