//! A stand-in for Gate's relay, for tests that need one to answer.
//!
//! Since `relay_listening` began verifying identity, binding a plain listener
//! on the relay port means "a stranger holds this port", which is a different
//! fixture with a different expected outcome. Anything that wants a tool or a
//! manager to see a *relay* has to answer the challenge, and anything that
//! wants to test the stranger case should keep binding a plain listener.
//!
//! In-crate so the unit tests in `mod` and `manager_core` share one copy. The
//! integration tests keep their own in `tests/common`, because an integration
//! test cannot reach a `cfg(test)` item.

// Some of this is used only by tests that are themselves gated to macOS and
// Windows (`engine_hosted_elsewhere` exists only there), so on a Linux build
// those methods have no caller. Gating each one to match would put the platform
// condition in two places and make the fixture harder to read than the thing it
// tests.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub(crate) struct TestRelay {
    port: u16,
    stop: Arc<AtomicBool>,
    intercepting: Arc<AtomicBool>,
}

impl TestRelay {
    /// Bind `port` (0 for any free one) and answer as a relay that is routing.
    pub(crate) fn start(port: u16) -> Self {
        Self::with_interception(port, true)
    }

    pub(crate) fn with_interception(port: u16, intercepting: bool) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .unwrap_or_else(|e| panic!("binding 127.0.0.1:{port} for the relay stub: {e}"));
        let port = listener.local_addr().expect("listener address").port();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let intercepting = Arc::new(AtomicBool::new(intercepting));
        let reports = intercepting.clone();
        // The token is read per request rather than captured: the prober mints
        // it, and a stub that cached an absent one would answer with the wrong
        // proof for the rest of the test.
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if flag.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(mut sock) = stream else { return };
                let mut buf = [0u8; 2048];
                let Ok(n) = sock.read(&mut buf) else { continue };
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                let challenge = head
                    .lines()
                    .filter_map(|l| l.split_once(':'))
                    .find(|(name, _)| {
                        name.trim()
                            .eq_ignore_ascii_case(gate_connect_paths::FORWARDER_CHALLENGE_HEADER)
                    })
                    .map(|(_, v)| v.trim().to_string())
                    .unwrap_or_default();
                let token = super::forwarder::load_or_create_token().unwrap_or_default();
                let proof = gate_connect_paths::forwarder_proof(&token, &challenge);
                let resp = format!(
                    "HTTP/1.1 204 No Content\r\n{}: {proof}\r\n{}: {}\r\n\
                     Content-Length: 0\r\nConnection: close\r\n\r\n",
                    gate_connect_paths::FORWARDER_PROOF_HEADER,
                    gate_connect_paths::RELAY_INTERCEPTING_HEADER,
                    if reports.load(Ordering::Relaxed) {
                        "1"
                    } else {
                        "0"
                    },
                );
                let _ = sock.write_all(resp.as_bytes());
            }
        });
        TestRelay {
            port,
            stop,
            intercepting,
        }
    }

    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    /// Park or unpark without rebinding, which is what a real park does and
    /// what keeps a test off the race with its own dropped socket.
    pub(crate) fn set_intercepting(&self, intercepting: bool) {
        self.intercepting.store(intercepting, Ordering::Relaxed);
    }

    /// Persist this relay's port so the production readers find it.
    pub(crate) fn persist_port(&self) {
        let dir = crate::env::app_support_dir()
            .expect("app support dir")
            .join("proxy");
        std::fs::create_dir_all(&dir).expect("creating the proxy dir");
        std::fs::write(dir.join("relay-port"), self.port.to_string()).expect("persisting the port");
    }
}

impl Drop for TestRelay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = std::net::TcpStream::connect(("127.0.0.1", self.port));
    }
}
