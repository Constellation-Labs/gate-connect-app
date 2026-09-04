//! Authentication tests for the Linux proxy helper control protocol: the
//! per-run token is a real gate, not decoration. A client that connects to the
//! socket but presents the wrong token (or no `Hello` at all) must be answered
//! `Hello { ok: false }` and dropped before any request is honored - and a
//! client that *does* authenticate must still be refused a `SetIntercept`
//! whose domains stray from the built-in catalog. `validate_domains` has unit
//! tests of its own in `control.rs`; what this pins is that the daemon
//! actually enforces both layers on the wire.
//!
//! The remaining access-control layer, the `SO_PEERCRED` UID check, is not
//! driven here: exercising the rejection needs a peer running as a second UID,
//! which an unprivileged single-user test process cannot be.
//!
//! One test, not several: a daemon is a singleton per runtime dir, so tests
//! sharing this binary would share the daemon and interfere with each other's
//! connections. The steps run in sequence against one daemon instead.
//!
//! Hermetic: `$XDG_RUNTIME_DIR` points at a throwaway dir, so the socket, the
//! token, the pidfile and the singleton lock are all this test's own. Every
//! `SetIntercept` sent here is rejected before the engine is touched, so no
//! ports are bound and no system state is changed; the daemon thread is left
//! running and dies with the process.
#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use gate_connect_core::proxy::control::{self, Request, Response, PROTOCOL_VERSION};
use gate_connect_core::proxy::default_domains;
use gate_connect_core::proxy::helper_client::HelperClient;

/// Point the control channel at a throwaway runtime dir and start the real
/// daemon on it, returning once it is answering on the socket.
fn start_daemon() {
    let tmp = std::env::temp_dir().join(format!("gate-daemon-token-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::env::set_var("XDG_RUNTIME_DIR", &tmp);
    // The control channel follows `GATE_CONNECT_TEST_HOME` in preference to
    // `$XDG_RUNTIME_DIR`, so an ambient one (exported in the shell) would
    // override the per-pid dir this test isolates itself with - and every
    // daemon test binary, which `cargo test` runs concurrently, would land on
    // one socket path and race. This test wants production resolution of the
    // var it just set, so drop the seam, as `audit_e2e` does for the same
    // reason.
    std::env::remove_var("GATE_CONNECT_TEST_HOME");

    std::thread::spawn(|| {
        // Blocks for the rest of the process; an error here surfaces as the
        // wait below timing out.
        let _ = gate_connect_core::proxy::helper::run_daemon();
    });

    // The daemon writes the token before it binds, so a connectable socket
    // means the whole handshake is ready.
    let sock = control::socket_path().expect("socket path");
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if UnixStream::connect(&sock).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("daemon never came up on {}", sock.display());
}

/// Connect raw (no [`HelperClient`], which would read the real token), send one
/// line, and return the daemon's `Hello` reply plus whether the connection was
/// then closed on us (EOF on the next read).
fn raw_first_message(line: &str) -> (Response, bool) {
    let sock = control::socket_path().expect("socket path");
    let stream = UnixStream::connect(&sock).expect("connect to daemon");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set read timeout");
    let mut writer = stream.try_clone().expect("clone stream");
    let mut reader = BufReader::new(stream);

    writeln!(writer, "{line}").expect("write first message");
    writer.flush().expect("flush first message");
    let mut reply = String::new();
    reader.read_line(&mut reply).expect("read Hello reply");
    let hello: Response = serde_json::from_str(reply.trim()).expect("parse Hello reply");

    // A refused client must not be kept around to try requests: probe with a
    // Status and expect the connection to be gone. Depending on how quickly
    // the daemon's side dropped, that surfaces as a broken pipe on the write,
    // a reset on the read, or a plain EOF - never as an answer, and never as
    // the read timeout expiring on a connection the daemon kept open.
    let probe = serde_json::to_string(&Request::Status).unwrap();
    let closed = match writeln!(writer, "{probe}").and_then(|_| writer.flush()) {
        Err(_) => true,
        Ok(()) => {
            let mut next = String::new();
            match reader.read_line(&mut next) {
                Ok(0) => true,
                Ok(_) => false,
                Err(e) => !matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ),
            }
        }
    };
    (hello, closed)
}

/// Drive one tampered `SetIntercept` through an authenticated [`HelperClient`]
/// and return the daemon's refusal message.
fn rejected_intercept(domains: Vec<gate_connect_core::proxy::ProxyDomain>) -> String {
    let mut client = HelperClient::connect_existing().expect("authenticated client connects");
    let err = match client.set_intercept(
        "https://gw.example.com",
        "sk-gw-testkey123",
        "",
        "",
        "",
        "",
        &domains,
        false,
        None,
        None,
    ) {
        Ok(_) => panic!("a non-catalog intercept must be refused"),
        Err(e) => e.to_string(),
    };
    // The refusal must not have armed anything: the engine was never started.
    let (running, port, intercepting) = client.status().expect("status after refusal");
    assert!(!running, "no engine may start off a refused intercept");
    assert_eq!(port, None);
    assert_eq!(intercepting, 0);
    err
}

#[test]
fn token_gate_and_catalog_gate_hold_on_the_wire() {
    start_daemon();

    // A wrong token is answered ok:false and the connection is dropped before
    // any request is served.
    let bad_hello = serde_json::to_string(&Request::Hello {
        token: "0000000000000000000000000000dead".into(),
        version: PROTOCOL_VERSION,
    })
    .unwrap();
    let (reply, closed) = raw_first_message(&bad_hello);
    assert!(
        matches!(reply, Response::Hello { ok: false, .. }),
        "a wrong token must be refused, got {reply:?}"
    );
    assert!(closed, "a refused client must be disconnected, not served");

    // So is skipping the handshake entirely: a first message that isn't Hello
    // gets the same refusal, even though it parses as a valid request.
    let status = serde_json::to_string(&Request::Status).unwrap();
    let (reply, closed) = raw_first_message(&status);
    assert!(
        matches!(reply, Response::Hello { ok: false, .. }),
        "a first message that isn't Hello must be refused, got {reply:?}"
    );
    assert!(closed, "an unauthenticated client must be disconnected");

    // The real token (read from the 0600 file, exactly as the GUI does) gets
    // through - the gate refuses strangers, not the owner.
    let mut client = HelperClient::connect_existing().expect("the real token authenticates");
    let (running, _, _) = client.status().expect("an authenticated Status is served");
    assert!(!running, "nothing armed yet");
    drop(client);

    // Past the token, the catalog gate still holds: an authenticated caller
    // cannot point the MITM at a host or upstream we don't ship, nor smuggle
    // one in under a known slug.
    let mut unknown_slug = default_domains();
    unknown_slug[0].slug = "evil".into();
    let err = rejected_intercept(unknown_slug);
    assert!(err.contains("not in the built-in catalog"), "{err}");

    let mut tampered_upstream = default_domains();
    tampered_upstream[0].upstream_url = "https://attacker.example".into();
    let err = rejected_intercept(tampered_upstream);
    assert!(err.contains("does not match the catalog"), "{err}");

    let mut extra_host = default_domains();
    extra_host[0].hosts.push("evil.example".into());
    let err = rejected_intercept(extra_host);
    assert!(err.contains("hosts not in the catalog"), "{err}");

    // And after all that abuse the daemon still serves the owner.
    let mut client = HelperClient::connect_existing().expect("daemon still serves after refusals");
    client.status().expect("status still answered");

    // The raw-socket refusal above and `HelperClient`'s own reading of it must
    // agree: `connect_existing` against a token file that says one thing while
    // the daemon expects another surfaces "token rejected", not a stale-daemon
    // or I/O error. The real token is restored right after, since the daemon
    // is shared by the whole test.
    let token_path = control::token_path().expect("token path");
    let real = std::fs::read_to_string(&token_path).expect("read real token");
    std::fs::write(&token_path, "not-the-token").expect("plant wrong token");
    let err = HelperClient::connect_existing()
        .err()
        .expect("a wrong token must fail the handshake")
        .to_string();
    std::fs::write(&token_path, real).expect("restore real token");
    assert!(err.contains("token rejected"), "{err}");
}
