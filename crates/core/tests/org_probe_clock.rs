//! `org::probe_session` records the gateway's clock from its `Date` header, so
//! a refused session can be logged with how far off the local clock was.
//!
//! Its own test binary because the reading is process-wide: the mocks in
//! `org_probe_e2e.rs` answer without a `Date` header, and running beside them
//! would race this test's reading.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, SystemTime};

use gate_connect_core::org::{clock_skew_secs, clock_skewed, probe_session, SessionProbe};

/// Serve one 401 whose `Date` is `server_now`. Returns the base URL.
fn spawn_refusal_dated(server_now: SystemTime) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
    let base = format!("http://{}", listener.local_addr().expect("mock addr"));
    let date = httpdate::fmt_http_date(server_now);
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        let body = r#"{"error":{"code":"invalid_gate_token"}}"#;
        let response = format!(
            "HTTP/1.1 401 Unauthorized\r\nDate: {date}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).ok();
    });
    base
}

#[test]
fn a_refusal_records_how_far_off_the_local_clock_is() {
    // The gateway two hours ahead of us: our clock is behind.
    let base = spawn_refusal_dated(SystemTime::now() + Duration::from_secs(2 * 3600));
    assert!(matches!(
        probe_session(&base, "at-skewed"),
        SessionProbe::Rejected
    ));
    assert!(clock_skewed());
    let skew = clock_skew_secs().expect("a reading");
    assert!((7190..=7210).contains(&skew), "skew {skew}");

    // Clock put right: the next answer clears it.
    let base = spawn_refusal_dated(SystemTime::now());
    assert!(matches!(
        probe_session(&base, "at-fine-clock"),
        SessionProbe::Rejected
    ));
    assert!(!clock_skewed());
}
