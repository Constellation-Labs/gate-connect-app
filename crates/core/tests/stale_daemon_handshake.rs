//! Handshake test for the Linux proxy helper control protocol: the client must
//! reuse a daemon that reports the same [`PROTOCOL_VERSION`] and detect one that
//! reports a different version as stale (asking it to shut down so it can be
//! replaced). A version-mismatched daemon is exactly the "leftover from an older
//! build" case that used to make a later request fail on an unrecognized reply.
//!
//! Hermetic: `$XDG_RUNTIME_DIR` is pointed at a throwaway dir and a fake daemon
//! is stood up on the real control socket. Nothing is spawned - the test drives
//! [`HelperClient::connect_existing`], which only talks to an already-listening
//! socket.
#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;

use gate_connect_core::proxy::control::{self, Request, Response, PROTOCOL_VERSION};
use gate_connect_core::proxy::helper_client::HelperClient;

/// Point the control channel at a throwaway runtime dir and drop in the token
/// the client echoes back in `Hello`.
fn setup() {
    let tmp = std::env::temp_dir().join(format!("gate-stale-handshake-{}", std::process::id()));
    std::env::set_var("XDG_RUNTIME_DIR", &tmp);
    // `token_path()` resolves (and creates, 0700) the runtime dir for us.
    let token_path = control::token_path().expect("token path");
    std::fs::write(&token_path, "test-token").expect("write token");
}

/// Stand up a fake daemon on the control socket that answers `Hello` with the
/// given protocol version, then drive `connect_existing`. Returns whether the
/// client accepted the daemon (mapping any error to its message), and whether
/// the daemon was subsequently asked to `Shutdown`.
fn run_scenario(daemon_version: u32) -> (Result<(), String>, bool) {
    let sock = control::socket_path().expect("socket path");
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock).expect("bind fake daemon socket");

    let daemon = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept client");
        let mut writer = stream.try_clone().expect("clone stream");
        let mut reader = BufReader::new(stream);

        // Consume the client's Hello, then advertise our protocol version.
        let mut hello = String::new();
        reader.read_line(&mut hello).expect("read Hello");
        let reply = Response::Hello {
            ok: true,
            version: daemon_version,
        };
        writeln!(writer, "{}", serde_json::to_string(&reply).unwrap()).expect("write Hello reply");
        writer.flush().unwrap();

        // A stale-version client follows up with `Shutdown`; a matching one just
        // holds the connection, so we see EOF once the test drops the client.
        let mut next = String::new();
        match reader.read_line(&mut next) {
            Ok(n) if n > 0 => {
                let is_shutdown = matches!(
                    serde_json::from_str::<Request>(next.trim()),
                    Ok(Request::Shutdown)
                );
                if is_shutdown {
                    let _ = writeln!(writer, "{}", serde_json::to_string(&Response::Ok).unwrap());
                    let _ = writer.flush();
                }
                is_shutdown
            }
            _ => false,
        }
    });

    // `map(|_| ())` drops the returned client, closing the connection so the
    // daemon's second read unblocks (EOF) in the matching case.
    let outcome = HelperClient::connect_existing()
        .map(|_| ())
        .map_err(|e| e.to_string());
    let asked_to_shut_down = daemon.join().expect("join fake daemon");
    let _ = std::fs::remove_file(&sock);
    (outcome, asked_to_shut_down)
}

#[test]
fn same_version_reused_mismatch_replaced() {
    setup();

    // Same version: the daemon is reused and never asked to shut down.
    let (reused, reused_shutdown) = run_scenario(PROTOCOL_VERSION);
    assert!(
        reused.is_ok(),
        "a same-version daemon should be reused, got {reused:?}"
    );
    assert!(
        !reused_shutdown,
        "a same-version daemon should not be asked to shut down"
    );

    // Different version: detected as stale and asked to shut down, so it can be
    // replaced rather than reused.
    let (stale, stale_shutdown) = run_scenario(PROTOCOL_VERSION + 1);
    let err = stale.expect_err("a mismatched-version daemon must not be reused");
    assert!(
        err.contains("incompatible protocol"),
        "expected a stale-daemon error, got {err:?}"
    );
    assert!(
        stale_shutdown,
        "a stale daemon should be asked to shut down"
    );
}
