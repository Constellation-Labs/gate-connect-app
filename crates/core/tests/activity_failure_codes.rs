//! End-to-end coverage of the failure codes `activity::overview_json` reports.
//!
//! The mapping is the contract under test, not an implementation detail: the
//! Overview pane branches on it to choose between Try again, Visit dashboard and
//! Manage API keys (see `src/lib/activityGaps.ts`), and AG-576 requires an
//! unavailable metric to name its cause. A code that collapses "your machine
//! cannot reach the gateway" into "your credential was refused" sends the user
//! to replace a key that was fine.
//!
//! Each test runs its own loopback mock, but the account lives behind the
//! process-global `GATE_CONNECT_TEST_HOME` seam plus an in-memory keychain, so a
//! `Mutex` serializes them within this binary.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;

use gate_connect_core::activity::{overview_json, FailureCode};
use gate_connect_core::{account, keychain};

static LOCK: Mutex<()> = Mutex::new(());

/// Point `app_support_dir()` at a fresh temp dir, as `account_reconcile.rs`
/// does and for the same reason: a bare `$HOME` override does not redirect the
/// data dir on Windows.
struct TempDataDir {
    dir: PathBuf,
}

impl TempDataDir {
    fn set() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "gate-connect-activity-test-{}-{}",
            std::process::id(),
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);
        TempDataDir { dir }
    }
}

impl Drop for TempDataDir {
    fn drop(&mut self) {
        std::env::remove_var("GATE_CONNECT_TEST_HOME");
        std::env::remove_var("GATE_CONNECT_TEST_ACTIVITY_ENDPOINT");
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Serve exactly one HTTP response on a fresh loopback port, and point the
/// endpoint seam at it.
fn mock_endpoint(status_line: &'static str, body: &'static str) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
    let url = format!(
        "http://{}/v1/me/activity",
        listener.local_addr().expect("mock addr")
    );
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        drain_request_head(&mut stream);
        let response = format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).ok();
    });
    std::env::set_var("GATE_CONNECT_TEST_ACTIVITY_ENDPOINT", url);
}

/// A port with nothing listening: bind it, learn the number, then drop it.
fn dead_endpoint() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind probe");
    let addr = listener.local_addr().expect("probe addr");
    drop(listener);
    std::env::set_var(
        "GATE_CONNECT_TEST_ACTIVITY_ENDPOINT",
        format!("http://{addr}/v1/me/activity"),
    );
}

fn drain_request_head(stream: &mut TcpStream) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

fn with_key_account() {
    keychain::use_in_memory_backend();
    account::save("https://gw.example.com", Some("sk-gw-test")).unwrap();
}

#[test]
fn success_returns_the_body_verbatim() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    with_key_account();
    mock_endpoint("200 OK", r#"{"generatedAt":"2026-08-17T12:00:00Z"}"#);

    // Verbatim, not reshaped: `lib/activity.ts` is the only model of this shape,
    // so a second one here could drift from it.
    assert_eq!(
        overview_json(None, None).unwrap(),
        r#"{"generatedAt":"2026-08-17T12:00:00Z"}"#
    );
}

#[test]
fn refused_credential_is_rejected_not_offline() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    with_key_account();
    mock_endpoint("401 Unauthorized", r#"{"message":"invalid api key"}"#);

    let f = overview_json(None, None).expect_err("401 must fail");
    assert_eq!(f.code, FailureCode::Rejected);
    // The gateway's own envelope survives, because a 4xx is the only place the
    // reason is written down.
    assert!(f.message.contains("invalid api key"), "{}", f.message);
}

#[test]
fn forbidden_is_also_rejected() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    with_key_account();
    mock_endpoint("403 Forbidden", r#"{"message":"not a member"}"#);

    assert_eq!(
        overview_json(None, None).expect_err("403 must fail").code,
        FailureCode::Rejected
    );
}

#[test]
fn server_error_is_a_gateway_fault_not_a_credential_fault() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    with_key_account();
    mock_endpoint("500 Internal Server Error", r#"{"message":"boom"}"#);

    // Distinct from Rejected on purpose: nothing about the user's key is wrong,
    // so telling them to manage their API keys would be a wild goose chase.
    assert_eq!(
        overview_json(None, None).expect_err("500 must fail").code,
        FailureCode::Gateway
    );
}

#[test]
fn nothing_listening_is_offline() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    with_key_account();
    dead_endpoint();

    assert_eq!(
        overview_json(None, None)
            .expect_err("a closed port must fail")
            .code,
        FailureCode::Offline
    );
}

#[test]
fn no_account_is_signed_out() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    keychain::use_in_memory_backend();
    // No `account::save`, and no endpoint: this must fail before any request.

    assert_eq!(
        overview_json(None, None)
            .expect_err("no account must fail")
            .code,
        FailureCode::SignedOut
    );
}

#[test]
fn account_without_a_key_is_signed_out() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    keychain::use_in_memory_backend();
    // The legitimate pending-key state: a gateway chosen, no key entered yet.
    account::save("https://gw.example.com", None).unwrap();

    assert_eq!(
        overview_json(None, None)
            .expect_err("a key-less account must fail")
            .code,
        FailureCode::SignedOut
    );
}
