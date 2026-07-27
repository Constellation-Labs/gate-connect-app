//! End-to-end coverage of `org::probe_session`, the startup check that asks
//! the gateway whether a locally-valid OAuth session actually works. The
//! verdict mapping is the contract under test: only a definite 401 is a
//! rejection; anything short of a verdict (unreachable, 5xx, garbage body)
//! must read as `Unavailable` so an offline start never signs the user out.
//!
//! Each test runs its own loopback HTTP mock and passes its base URL as the
//! gateway, so no env-var seams are involved and the tests can run in
//! parallel.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;

use gate_connect_core::org::{probe_session, SessionProbe};

/// Serve exactly one HTTP response on a fresh loopback port, forwarding the
/// raw request (line + headers) over the channel. Returns the base URL.
fn spawn_mock(status_line: &'static str, body: &'static str) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
    let base = format!("http://{}", listener.local_addr().expect("mock addr"));
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        tx.send(read_request_head(&mut stream)).ok();
        let response = format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).ok();
    });
    (base, rx)
}

/// Read the request line + headers (a GET has no body to wait for).
fn read_request_head(stream: &mut TcpStream) -> String {
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
    String::from_utf8_lossy(&buf).into_owned()
}

#[test]
fn accepted_parses_orgs_and_sends_the_bearer() {
    let (base, rx) = spawn_mock(
        "200 OK",
        r#"{
          "user": { "id": "u-1", "email": "dev@example.test" },
          "orgs": [
            { "orgId": "org-uuid-1", "name": "Acme Inc", "slug": "acme", "role": "owner" },
            { "orgId": "org-uuid-2", "name": "Side Co", "slug": "side", "role": "member" }
          ]
        }"#,
    );
    let verdict = probe_session(&base, "at-probe");
    let SessionProbe::Accepted(orgs) = verdict else {
        panic!("expected Accepted, got {verdict:?}");
    };
    assert_eq!(orgs.len(), 2);
    assert_eq!(orgs[0].org_id, "org-uuid-1");
    assert_eq!(orgs[1].org_id, "org-uuid-2");

    let request = rx.recv().expect("mock captured the request");
    assert!(
        request.starts_with("GET /v1/me/orgs "),
        "probe must hit the orgs endpoint: {request}"
    );
    assert!(
        request
            .to_ascii_lowercase()
            .contains("x-gate-authorization: bearer at-probe"),
        "probe must authenticate on the gateway's custom slot: {request}"
    );
}

#[test]
fn unauthorized_is_a_rejection() {
    let (base, _rx) = spawn_mock("401 Unauthorized", r#"{"error":"invalid_key"}"#);
    assert!(matches!(
        probe_session(&base, "at-stale"),
        SessionProbe::Rejected
    ));
}

#[test]
fn server_error_is_no_verdict() {
    let (base, _rx) = spawn_mock("500 Internal Server Error", "boom");
    assert!(matches!(
        probe_session(&base, "at-fine"),
        SessionProbe::Unavailable
    ));
}

#[test]
fn garbage_body_is_no_verdict() {
    let (base, _rx) = spawn_mock("200 OK", "not json");
    assert!(matches!(
        probe_session(&base, "at-fine"),
        SessionProbe::Unavailable
    ));
}

#[test]
fn unreachable_gateway_is_no_verdict() {
    // Bind then drop so the port is closed and the connection is refused.
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        let p = l.local_addr().expect("addr").port();
        drop(l);
        p
    };
    assert!(matches!(
        probe_session(&format!("http://127.0.0.1:{port}"), "at-offline"),
        SessionProbe::Unavailable
    ));
}
