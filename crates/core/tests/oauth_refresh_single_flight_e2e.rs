//! Every reader that finds the session expired at once gets one refresh, not
//! one each.
//!
//! After a sleep or a reboot the stored token is expired for everyone at the
//! same moment: startup, the security feed, the window's reads and the 30s tick
//! all refresh. Each used to mint its own token and store it, and two of those
//! stores overlapping deadlocked the macOS keychain - the app came up with
//! blank windows and no proxy, with nothing in the log. Storing one refresh
//! also means one Cognito call instead of one per reader.
//!
//! Hermetic: the file-backed secret store and a loopback Cognito mock that
//! answers slowly, so the readers really are waiting on each other. One test
//! function, because the seams and the session state are process-global.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use gate_connect_core::env;
use gate_connect_core::oauth::{self, OAuthTokens};

/// Answer each request after a pause with `at-renewed-<n>`, n counting from 1.
fn spawn_slow_cognito() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind cognito mock");
    let addr = listener.local_addr().expect("mock addr");
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_t = hits.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let hits = hits_t.clone();
            thread::spawn(move || {
                let n = hits.fetch_add(1, Ordering::SeqCst) + 1;
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                thread::sleep(Duration::from_millis(300));
                let body = format!(
                    r#"{{"access_token":"at-renewed-{n}","expires_in":3600,"token_type":"Bearer"}}"#
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            });
        }
    });
    (format!("http://{addr}/oauth2/token"), hits)
}

/// Run `f` on `n` threads released together, collecting what each returned.
fn all_at_once<T: Send + 'static>(n: usize, f: fn() -> T) -> Vec<T> {
    let start = Arc::new(Barrier::new(n));
    let handles: Vec<_> = (0..n)
        .map(|_| {
            let start = start.clone();
            thread::spawn(move || {
                start.wait();
                f()
            })
        })
        .collect();
    handles
        .into_iter()
        .map(|h| h.join().expect("reader panicked"))
        .collect()
}

fn forced() -> Option<String> {
    let cfg = oauth::OAuthConfig::from_build_env().expect("cognito env set");
    oauth::force_refresh(&cfg)
        .expect("refresh")
        .map(|t| t.access_token)
}

#[test]
fn readers_of_an_expired_session_share_one_refresh() {
    let (endpoint, hits) = spawn_slow_cognito();
    let base = std::env::temp_dir().join(format!(
        "gate-connect-single-flight-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
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

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    oauth::store(&OAuthTokens {
        access_token: "at-expired".to_string(),
        refresh_token: "rt-1".to_string(),
        id_token: None,
        expires_at_unix: now - 60,
        client_id: "client123".to_string(),
    })
    .expect("seed bundle");

    // 1. The reboot: everyone reads the expired session at once.
    let seen = all_at_once(8, || oauth::live_session().map(|t| t.access_token));
    assert!(
        seen.iter().all(|t| t.as_deref() == Some("at-renewed-1")),
        "every reader gets the one refresh: {seen:?}"
    );
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "one Cognito call, not eight"
    );

    // 2. The 401 re-check, raised by several refused requests together. Each
    //    one wants a token newer than the refused one, and the first refresh is
    //    exactly that, so the others take it.
    let seen = all_at_once(4, forced);
    assert!(
        seen.iter().all(|t| t.as_deref() == Some("at-renewed-2")),
        "every forced caller gets the one forced refresh: {seen:?}"
    );
    assert_eq!(hits.load(Ordering::SeqCst), 2);

    // 3. A later forced refresh, with nothing in flight, still refreshes.
    assert_eq!(forced().as_deref(), Some("at-renewed-3"));
    assert_eq!(hits.load(Ordering::SeqCst), 3);

    let _ = std::fs::remove_dir_all(&base);
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
