//! Shared fixtures for the integration tests.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpListener;

/// A stand-in for Gate's relay that answers the identity challenge.
///
/// `relay_listening()` no longer trusts a bare TCP connect: it asks the
/// listener to prove it can read the 0600 token, so a test that wants a tool to
/// read Connected has to answer like the real relay does. Binding a plain
/// listener now means "a stranger holds this port", which is a different
/// fixture with a different expected outcome.
///
/// Answers on a thread for the life of the returned handle and ignores anything
/// that is not the health path, which is all a status probe ever sends.
pub struct RelayStub {
    port: u16,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl RelayStub {
    /// Bind `port` (0 for any free one) and answer the proof from the token
    /// file under the current test home.
    pub fn bind(port: u16) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .unwrap_or_else(|e| panic!("binding 127.0.0.1:{port} for the relay stub: {e}"));
        let port = listener.local_addr().unwrap().port();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if flag.load(std::sync::atomic::Ordering::Relaxed) {
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
                // Read, never create: the prober mints the token, so a stub
                // that created one could hide a prober that never did.
                let token = std::fs::read_to_string(
                    gate_connect_core::env::app_support_dir()
                        .unwrap()
                        .join("proxy")
                        .join("forwarder.token"),
                )
                .unwrap_or_default();
                let proof = gate_connect_paths::forwarder_proof(token.trim(), &challenge);
                let resp = format!(
                    "HTTP/1.1 204 No Content\r\n{}: {proof}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    gate_connect_paths::FORWARDER_PROOF_HEADER
                );
                let _ = sock.write_all(resp.as_bytes());
            }
        });
        RelayStub { port, stop }
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for RelayStub {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        // Unblock the accept loop so the thread notices the flag.
        let _ = std::net::TcpStream::connect(("127.0.0.1", self.port));
    }
}
