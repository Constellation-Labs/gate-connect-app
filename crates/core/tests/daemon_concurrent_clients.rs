//! The helper daemon must serve control connections concurrently.
//!
//! It used to handle one client to completion before accepting the next, which
//! made the GUI's long-lived control connection a lock on the whole control
//! channel: every other caller sat in the accept backlog until the GUI let go.
//! A CLI `proxy status` therefore hit `CONTROL_TIMEOUT` and reported the proxy
//! off while it was on - the daemon was never asked. `connect_existing` maps an
//! unanswered `Hello` to the stale-daemon error, so that is what a regression
//! here looks like.
//!
//! The other half of the contract is the disconnect rule. A disconnect drops
//! the engine to pass-through (unless a client asked to be detached), and with
//! overlapping connections that has to mean the *last* client leaving, not any
//! client leaving - otherwise a `proxy status` that queried and exited would
//! un-route a machine the GUI is still minding.
//!
//! One test, not two: a daemon is a singleton per runtime dir, so tests sharing
//! this binary would share the daemon and interfere with each other's
//! connections. The steps run in sequence against one daemon instead.
//!
//! Hermetic: `$XDG_RUNTIME_DIR` points at a throwaway dir, so the socket, the
//! token, the pidfile and the singleton lock are all this test's own. The
//! daemon runs in a thread and is left running; the process exits at the end of
//! the run. Nothing here arms an engine, so no ports are bound and no system
//! state is touched.
#![cfg(target_os = "linux")]

use std::time::{Duration, Instant};

use gate_connect_core::proxy::control;
use gate_connect_core::proxy::helper_client::HelperClient;

/// Point the control channel at a throwaway runtime dir and start the real
/// daemon on it, returning once it is answering on the socket.
fn start_daemon() {
    let tmp = std::env::temp_dir().join(format!("gate-daemon-concurrent-{}", std::process::id()));
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
        if std::os::unix::net::UnixStream::connect(&sock).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("daemon never came up on {}", sock.display());
}

#[test]
fn overlapping_control_connections_are_served_independently() {
    start_daemon();

    // Client A stands in for the GUI: connects and stays connected.
    let mut a = HelperClient::connect_existing().expect("client A connects");
    a.status()
        .expect("client A can query while it is the only client");

    // Client B stands in for a CLI invocation arriving while A holds its
    // connection. This is the assertion: before the daemon accepted concurrent
    // connections, B's `Hello` was never answered - it waited in the backlog
    // and `connect_existing` gave up after CONTROL_TIMEOUT.
    let mut b = HelperClient::connect_existing().expect("client B connects while A is held open");
    b.status()
        .expect("client B can query while A holds its connection");

    // B leaving must be "one client left", not "the last client left": A is
    // still here. No engine is armed in this test, so what is asserted is the
    // observable half - B's departure disturbs neither the daemon nor A.
    drop(b);
    a.status()
        .expect("client A still works after client B came and went");

    // And a third client can still arrive afterwards, i.e. the accept loop kept
    // going rather than being wedged by the previous connections.
    let mut c = HelperClient::connect_existing().expect("client C connects after B disconnected");
    c.status().expect("client C can query");
}
