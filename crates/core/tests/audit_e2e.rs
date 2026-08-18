//! End-to-end coverage of `audit::emit` - specifically the request contract,
//! which is the one thing about this module that no amount of reading catches
//! reliably and which has been wrong twice.
//!
//! The contract under test is dashboard-api's, not the gateway's. `POST
//! /audit/emit` is served by `AuditController`; its `AuditAuthGuard` reads the
//! standard `Authorization: Bearer …` and sniffs the `sk-gw-` prefix itself to
//! pick between API-key and Cognito validation. The org is per-mode: OAuth
//! sends the selected org on `x-org-id` (required - a Cognito user may belong
//! to many orgs), ApiKey sends no org header and the endpoint derives the org
//! from the key's own scope. Putting the token on the gateway's
//! `x-gate-authorization` slot instead is a 401, and a 401 means the audit
//! trail is silently empty. The path matters for the same reason:
//! `/v1/audit/emit` misses the ALB rule that routes `/audit/*` to
//! dashboard-api and lands on the gateway catch-all.
//!
//! Request tests drive `emit` against a loopback mock with an explicit
//! credential; the wrapper tests exercise `credential`'s mode selection and the
//! payload shape against a throwaway data dir and the in-memory keychain. A
//! `Mutex` serializes the whole file, because the
//! `GATE_CONNECT_TEST_AUDIT_ENDPOINT` seam is a process-global env var: a
//! wrapper test that sets it would otherwise redirect a request test running in
//! parallel. That seam earns its place because an account's base URL must be
//! https, so it can never name a loopback mock itself.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread;

use gate_connect_core::audit::{self, Credential};
use gate_connect_core::{account, keychain};

static LOCK: Mutex<()> = Mutex::new(());

/// One captured request: lowercased header names, plus the JSON body.
struct Captured {
    request_line: String,
    headers: HashMap<String, String>,
    body: serde_json::Value,
}

impl Captured {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

/// Serve exactly one HTTP response on a fresh loopback port, forwarding the
/// parsed request over the channel. Returns the base URL.
fn spawn_mock(status_line: &'static str, body: &'static str) -> (String, mpsc::Receiver<Captured>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
    let base = format!("http://{}", listener.local_addr().expect("mock addr"));
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        if let Some(captured) = read_request(&mut stream) {
            tx.send(captured).ok();
        }
        let response = format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).ok();
    });
    (base, rx)
}

/// Read a POST: head first, then exactly `Content-Length` bytes of body.
fn read_request(stream: &mut TcpStream) -> Option<Captured> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    let head_end = loop {
        match stream.read(&mut chunk) {
            Ok(0) => return None,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break i + 4;
                }
            }
            Err(_) => return None,
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default().to_string();
    let headers: HashMap<String, String> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();

    let len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    while buf.len() < head_end + len {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }
    let body = serde_json::from_slice(&buf[head_end..]).unwrap_or(serde_json::Value::Null);
    Some(Captured {
        request_line,
        headers,
        body,
    })
}

/// Clear both seams so a test sees the production path: `audit_endpoint` derives
/// the URL from the gateway base URL it was handed. Called by the header tests,
/// which assert that derivation, so an ambient `GATE_CONNECT_TEST_HOME` in the
/// runner's environment can't turn them into no-ops that then block on `recv`.
/// Safe because every test in this file holds `LOCK`.
fn without_seams() {
    std::env::remove_var("GATE_CONNECT_TEST_HOME");
    std::env::remove_var("GATE_CONNECT_TEST_AUDIT_ENDPOINT");
}

/// Point the data dir at a fresh temp dir, via the same `GATE_CONNECT_TEST_HOME`
/// seam the other integration tests use (a bare `$HOME` override doesn't
/// redirect the data dir on Windows). That seam also makes `audit` skip emitting
/// until a test names an endpoint, so the `account::save` in each setup below
/// stays local. Clears both seams and deletes the dir on drop.
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
            "gate-connect-audit-test-{}-{}",
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
        std::env::remove_var("GATE_CONNECT_TEST_AUDIT_ENDPOINT");
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn a_cognito_token_authenticates_on_the_standard_slot() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    without_seams();
    let (base, rx) = spawn_mock("201 Created", "{}");

    audit::emit(
        &base,
        &Credential {
            token: "cognito-access-token".into(),
            org_id: Some("org-uuid-1".into()),
        },
        "Proxy enabled",
        serde_json::json!({ "action": "proxy_enabled" }),
    )
    .expect("a 2xx must be Ok");

    let req = rx.recv().expect("mock captured the request");
    // The path regression: `/v1/audit/emit` misses the ALB rule for `/audit/*`
    // and falls through to the gateway catch-all, which answers a misleading
    // 401 about passthrough tokens rather than a 404.
    assert!(
        req.request_line.starts_with("POST /audit/emit "),
        "emit must POST the un-versioned dashboard-api route: {}",
        req.request_line
    );
    // The header regression: AuditAuthGuard reads only `Authorization` and
    // sniffs the `sk-gw-` prefix itself, so a Cognito token belongs here too.
    assert_eq!(
        req.header("authorization"),
        Some("Bearer cognito-access-token")
    );
    assert_eq!(req.header("x-org-id"), Some("org-uuid-1"));
    // Checked as absent headers rather than a missing substring -
    // `x-gate-authorization` contains the other name.
    assert_eq!(
        req.header("x-gate-authorization"),
        None,
        "the gateway's own slots are a data-plane convention; dashboard-api ignores them"
    );
    assert_eq!(req.header("x-gate-org-id"), None);
}

#[test]
fn an_api_key_authenticates_on_the_same_slot_with_no_org_header() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    without_seams();
    let (base, rx) = spawn_mock("201 Created", "{}");

    audit::emit(
        &base,
        &Credential {
            token: "sk-gw-audit".into(),
            org_id: None,
        },
        "Proxy disabled",
        serde_json::json!({ "action": "proxy_disabled" }),
    )
    .expect("a 2xx must be Ok");

    let req = rx.recv().expect("mock captured the request");
    assert_eq!(req.header("authorization"), Some("Bearer sk-gw-audit"));
    // No org header in ApiKey mode: the endpoint derives the org from the
    // key's own scope, the same way the gateway resolves the key's data-plane
    // tenancy. Sending a selected org instead would 403 whenever it differs
    // from the key's scope.
    assert_eq!(req.header("x-org-id"), None);
}

#[test]
fn body_carries_the_event_type_and_action() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    without_seams();
    let (base, rx) = spawn_mock("200 OK", "{}");

    audit::emit(
        &base,
        &Credential {
            token: "sk-gw-audit".into(),
            org_id: None,
        },
        "Provider 'Claude' enabled",
        serde_json::json!({ "action": "provider_enabled" }),
    )
    .unwrap();

    let req = rx.recv().expect("mock captured the request");
    assert_eq!(req.body["eventType"], "admin.config.changed");
    assert_eq!(req.body["message"], "Provider 'Claude' enabled");
    assert_eq!(req.body["data"]["action"], "provider_enabled");
}

/// `message` is `@MaxLength(512)` server-side, counted in UTF-16 code units. The
/// only caller-shaped input reaching it is the org name in `org_selected`, so an
/// operator could otherwise name an org long enough to 400 the record of their
/// own switch. Truncation keeps the event.
#[test]
fn an_over_long_message_is_truncated_rather_than_rejected() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    without_seams();
    let (base, rx) = spawn_mock("201 Created", "{}");

    audit::emit(
        &base,
        &Credential {
            token: "cognito-access-token".into(),
            org_id: Some("org-uuid-1".into()),
        },
        &"\u{1f512}".repeat(600),
        serde_json::json!({ "action": "org_selected" }),
    )
    .unwrap();

    let req = rx.recv().expect("mock captured the request");
    let sent = req.body["message"].as_str().expect("message is a string");
    assert!(
        sent.encode_utf16().count() <= 512,
        "sent {} UTF-16 units",
        sent.encode_utf16().count()
    );
    assert!(sent.ends_with('\u{2026}'), "truncation must be visible");
}

#[test]
fn non_2xx_is_an_error_carrying_the_status_and_body() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    without_seams();
    let (base, _rx) = spawn_mock("401 Unauthorized", r#"{"error":"invalid_token"}"#);

    let err = audit::emit(
        &base,
        &Credential {
            token: "sk-gw-audit".into(),
            org_id: None,
        },
        "Proxy enabled",
        serde_json::json!({ "action": "proxy_enabled" }),
    )
    .expect_err("a 401 must not read as a recorded event");

    let msg = format!("{err:#}");
    assert!(msg.contains("401"), "status must surface: {msg}");
    assert!(msg.contains("invalid_token"), "body must surface: {msg}");
}

#[test]
fn unreachable_gateway_is_an_error() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    without_seams();
    // Bind then drop so the port is closed and the connection is refused.
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        let p = l.local_addr().expect("addr").port();
        drop(l);
        p
    };
    assert!(audit::emit(
        &format!("http://127.0.0.1:{port}"),
        &Credential {
            token: "sk-gw-audit".into(),
            org_id: None,
        },
        "Proxy disabled",
        serde_json::json!({ "action": "proxy_disabled" }),
    )
    .is_err());
}

/// ApiKey mode must reach the keychain when the call site has no key in hand,
/// which is the case on every disable path. Skipping there once cost five of the
/// nine events their ApiKey-mode coverage; the read is the same one `enable`
/// already makes, so it buys that coverage back for nothing. No org is needed:
/// the endpoint derives it from the key's scope.
#[test]
fn api_key_mode_falls_back_to_the_stored_key() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    keychain::use_in_memory_backend();
    account::save("https://gw.example.test", Some("sk-gw-stored")).unwrap();

    assert!(
        matches!(
            audit::credential(None),
            Some(Credential { token, org_id: None }) if token == "sk-gw-stored"
        ),
        "a disable path with no key in hand must still authenticate"
    );
    // An in-hand key still wins, so a rotation audits under the key it just set
    // rather than under whatever the keychain happens to hold.
    assert!(matches!(
        audit::credential(Some("sk-gw-inhand")),
        Some(Credential { token, org_id: None }) if token == "sk-gw-inhand"
    ));
}

/// With no key stored and none in hand there is nothing to authenticate with.
#[test]
fn api_key_mode_without_any_key_skips_the_emit() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    keychain::use_in_memory_backend();
    account::save("https://gw.example.test", None).unwrap();

    assert!(audit::credential(None).is_none());
    assert!(
        audit::credential(Some("")).is_none(),
        "an empty key is not a credential"
    );
}

/// The point of deriving the org server-side: an ApiKey-mode operator who never
/// went through the OAuth org picker still emits every event. A selected org
/// must also not leak into the credential - the key's scope is authoritative,
/// and a mismatched header would 403 the emit.
#[test]
fn api_key_mode_emits_without_a_selected_org_and_ignores_one() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    keychain::use_in_memory_backend();
    account::save("https://gw.example.test", Some("sk-gw-stored")).unwrap();

    // Pure ApiKey user: no org ever selected.
    assert!(
        matches!(
            audit::credential(None),
            Some(Credential { org_id: None, .. })
        ),
        "no selected org must not skip an ApiKey-mode emit"
    );

    // Leftover org from an earlier OAuth session: still not sent.
    account::set_org("org-uuid-1", "Example Org").unwrap();
    assert!(matches!(
        audit::credential(None),
        Some(Credential { org_id: None, .. })
    ));
}

/// OAuth mode still requires a selected org - the endpoint cannot derive one
/// for a Cognito user (they may belong to several orgs), so an org-less OAuth
/// state skips rather than spending a round trip earning a 400. The window is
/// sign-in-to-first-org-pick and nothing more; the picker is forced before the
/// rest of the app is usable.
#[test]
fn oauth_mode_without_an_org_skips_the_emit() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    keychain::use_in_memory_backend();
    account::save("https://gw.example.test", Some("sk-gw-stored")).unwrap();
    account::set_auth_mode(account::AuthMode::OAuth).unwrap();

    assert!(
        audit::credential(Some("sk-gw-stored")).is_none(),
        "OAuth mode with no org has nothing the endpoint would accept"
    );
}

/// An unknown engine port must serialize as `null`, not as a placeholder `0` that
/// a reader cannot tell from a real port. Also pins the wrapper path end-to-end
/// in ApiKey mode: even with an org selected, no `x-org-id` goes out.
#[test]
fn unknown_port_is_null_rather_than_zero() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    keychain::use_in_memory_backend();
    account::save("https://gw.example.test", Some("sk-gw-stored")).unwrap();
    account::set_org("org-uuid-1", "Example Org").unwrap();

    // Drain the arrange emits before the mock exists. `emit_best_effort` spawns
    // a detached thread that resolves the endpoint seam when it *runs*, not when
    // it is spawned, so an emit still in flight here would read the endpoint set
    // below and POST to this test's own mock: that consumes the mock's single
    // connection and hands `rx.recv()` the wrong event, while the event under
    // test gets a connection reset. Flushing while the seam is still unset is
    // what makes the skip documented on `TempDataDir` actually hold.
    audit::flush();

    let (base, rx) = spawn_mock("200 OK", "{}");
    std::env::set_var(
        "GATE_CONNECT_TEST_AUDIT_ENDPOINT",
        format!("{base}/audit/emit"),
    );

    audit::proxy_enabled("https://gw.example.test", Some("sk-gw-stored"), None);

    let req = rx.recv().expect("mock captured the request");
    assert_eq!(req.body["data"]["proxy"]["port"], serde_json::Value::Null);
    assert_eq!(req.body["data"]["proxy"]["newState"], "on");
    assert_eq!(
        req.header("x-org-id"),
        None,
        "ApiKey mode must leave the org to the key's scope"
    );
}

/// `clear()` is the recovery path for a corrupt `account.json`, so the audit
/// read at its top must never stop the deletion. A propagating `?` on
/// `load_base_url()` once made disconnect fail before removing anything -
/// stranding the operator on the exact file they were trying to reset.
#[test]
fn clear_succeeds_when_account_json_is_corrupt() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = TempDataDir::set();
    keychain::use_in_memory_backend();
    account::save("https://gw.example.test", Some("sk-gw-stored")).unwrap();

    // Corrupt the file the save just wrote, wherever the platform put it.
    let account_json = find_file(&data.dir, "account.json").expect("save wrote account.json");
    fs::write(&account_json, "{ not json").unwrap();

    account::clear().expect("disconnect must survive a corrupt account.json");
    assert!(
        !account_json.exists(),
        "the corrupt file is exactly what clear() must remove"
    );
}

/// The repoint event must land at the OLD gateway, carry the new URL, and be
/// authenticated with the credential as it was *before* the switch -
/// `switch_gateway` deletes the stored key right after, so a late credential
/// read would find nothing and skip the one record of where the trail went.
#[test]
fn gateway_switch_emits_to_the_old_gateway_naming_the_new_one() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = TempDataDir::set();
    keychain::use_in_memory_backend();
    account::save("https://old.example.test", Some("sk-gw-stored")).unwrap();

    // Same reason as above: drain the `save` emit while the endpoint seam is
    // still unset, so it cannot land on the mock spawned just below.
    audit::flush();

    let (base, rx) = spawn_mock("201 Created", "{}");
    std::env::set_var(
        "GATE_CONNECT_TEST_AUDIT_ENDPOINT",
        format!("{base}/audit/emit"),
    );

    account::switch_gateway("https://new.example.test").unwrap();

    let req = rx.recv().expect("mock captured the request");
    assert_eq!(req.header("authorization"), Some("Bearer sk-gw-stored"));
    assert_eq!(req.body["data"]["action"], "gateway_switched");
    assert_eq!(
        req.body["data"]["gateway"]["previousUrl"],
        "https://old.example.test"
    );
    assert_eq!(
        req.body["data"]["gateway"]["newUrl"],
        "https://new.example.test"
    );
}

/// Walk `dir` for a file named `name`. The app-support layout under the
/// test-home seam differs per platform, so the tests locate rather than
/// construct the path.
fn find_file(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|f| f == name) {
                return Some(path);
            }
        }
    }
    None
}
