//! Handshake test for the Linux proxy helper control protocol: the client must
//! reuse a daemon that reports the same [`PROTOCOL_VERSION`] and
//! [`BUILD_FINGERPRINT`], and detect one that reports a different version or
//! fingerprint as stale. Both are "leftover from another build" cases: a version
//! mismatch used to make a later request fail on an unrecognized reply, a
//! fingerprint mismatch used to keep old daemon behavior running (e.g. rejecting
//! a catalog domain the client's build knows).
//!
//! Detecting a stale daemon is not licence to end it, and which entry point was
//! used decides: [`HelperClient::connect_existing`] is the read-only probe
//! behind `proxy status`, and it must leave even a mismatched daemon running -
//! it used to `Shutdown` one, so a status call from any differently-built binary
//! (a test binary, a CLI skewed from the installed app) killed the live daemon
//! and the user's routing with it. [`shutdown_daemon`] is the path that means
//! it, and it must still retire a stale daemon gracefully rather than falling
//! through to the force-kill.
//!
//! Hermetic: `$XDG_RUNTIME_DIR` is pointed at a throwaway dir and a fake daemon
//! is stood up on the real control socket. Nothing is spawned - both entry
//! points here only talk to an already-listening socket.
#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;

use gate_connect_core::proxy::control::{self, Request, Response, PROTOCOL_VERSION};
use gate_connect_core::proxy::helper_client::{self, HelperClient};

/// Point the control channel at a throwaway runtime dir and drop in the token
/// the client echoes back in `Hello`.
fn setup() {
    let tmp = std::env::temp_dir().join(format!("gate-stale-handshake-{}", std::process::id()));
    std::env::set_var("XDG_RUNTIME_DIR", &tmp);
    // `token_path()` resolves (and creates, 0700) the runtime dir for us.
    let token_path = control::token_path().expect("token path");
    std::fs::write(&token_path, "test-token").expect("write token");
}

/// The read-only probe: what `proxy status` does to a daemon it finds.
fn probe() -> Result<(), String> {
    HelperClient::connect_existing()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The retire path: what a caller that wants the daemon gone does.
fn retire() -> Result<(), String> {
    helper_client::shutdown_daemon();
    Ok(())
}

/// Stand up a fake daemon on the control socket that answers `Hello` with the
/// given protocol version and build fingerprint, then drive `entry` against it.
/// Returns what `entry` reported (mapping any error to its message), and whether
/// the daemon was subsequently asked to `Shutdown`.
fn run_scenario(
    daemon_version: u32,
    daemon_fingerprint: &str,
    entry: fn() -> Result<(), String>,
) -> (Result<(), String>, bool) {
    let sock = control::socket_path().expect("socket path");
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock).expect("bind fake daemon socket");

    let fingerprint = daemon_fingerprint.to_string();
    let daemon = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept client");
        let mut writer = stream.try_clone().expect("clone stream");
        let mut reader = BufReader::new(stream);

        // Consume the client's Hello, then advertise our protocol version and
        // build fingerprint.
        let mut hello = String::new();
        reader.read_line(&mut hello).expect("read Hello");
        let reply = Response::Hello {
            ok: true,
            version: daemon_version,
            fingerprint,
        };
        writeln!(writer, "{}", serde_json::to_string(&reply).unwrap()).expect("write Hello reply");
        writer.flush().unwrap();

        // A caller retiring us follows up with `Shutdown`; a probe just holds the
        // connection, so we see EOF once the test drops the client.
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

    // Each entry point drops the client it obtained, closing the connection so
    // the daemon's second read unblocks (EOF) when nothing follows the `Hello`.
    let outcome = entry();
    let asked_to_shut_down = daemon.join().expect("join fake daemon");
    let _ = std::fs::remove_file(&sock);
    (outcome, asked_to_shut_down)
}

/// One test, not several: the runtime dir and the control socket inside it are
/// process-global, and libtest would run separate `#[test]` fns as concurrent
/// threads competing for the same socket path.
#[test]
fn probe_leaves_a_stale_daemon_running_and_retire_ends_it() {
    setup();

    // Same version and fingerprint: the daemon is reused and never asked to
    // shut down.
    let (reused, reused_shutdown) =
        run_scenario(PROTOCOL_VERSION, control::BUILD_FINGERPRINT, probe);
    assert!(
        reused.is_ok(),
        "a same-build daemon should be reused, got {reused:?}"
    );
    assert!(
        !reused_shutdown,
        "a same-build daemon should not be asked to shut down"
    );

    // Different version: reported as stale, so a caller that wants the socket
    // knows to replace it - but a probe must not be the one to end it.
    let (stale, stale_shutdown) =
        run_scenario(PROTOCOL_VERSION + 1, control::BUILD_FINGERPRINT, probe);
    let err = stale.expect_err("a mismatched-version daemon must not be reused");
    assert!(
        err.contains("incompatible protocol"),
        "expected a stale-daemon error, got {err:?}"
    );
    assert!(
        !stale_shutdown,
        "probing a stale daemon must leave it running: a status call from a \
         differently-built binary would otherwise kill the live proxy"
    );

    // Same version but a different build fingerprint (a daemon whose Hello
    // predates fingerprints replies with the empty default): same treatment.
    let (skewed, skewed_shutdown) = run_scenario(PROTOCOL_VERSION, "", probe);
    let err = skewed.expect_err("a mismatched-fingerprint daemon must not be reused");
    assert!(
        err.contains("incompatible protocol"),
        "expected a stale-daemon error, got {err:?}"
    );
    assert!(
        !skewed_shutdown,
        "probing a build-skewed daemon must leave it running"
    );

    // The path that does mean it still ends a stale daemon by asking, not by
    // waiting out the timeout and force-killing it.
    let (_, retired_shutdown) = run_scenario(PROTOCOL_VERSION, "", retire);
    assert!(
        retired_shutdown,
        "shutdown_daemon should ask a stale daemon to shut down"
    );

    // And a same-build one, which is the ordinary `shutdown_engine` case.
    let (_, ended_shutdown) = run_scenario(PROTOCOL_VERSION, control::BUILD_FINGERPRINT, retire);
    assert!(
        ended_shutdown,
        "shutdown_daemon should ask a same-build daemon to shut down"
    );
}
