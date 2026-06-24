//! GUI-side client for the proxy helper daemon ([`super::helper`]). Blocking
//! `std` sockets, so it drops straight into the synchronous proxy manager with
//! no async runtime.
//!
//! Lifetime contract: the daemon keeps interception active only while a control
//! connection is open. So [`ProxyManager`](super::manager) holds the
//! [`HelperClient`] for as long as the proxy is "on"; dropping it (a clean
//! disable, or the GUI process exiting) closes the connection and the daemon
//! falls back to pass-through - the port stays bound, so frozen sessions keep
//! flowing instead of being stranded.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::proxy::control::{self, Request, Response};
use crate::proxy::ProxyDomain;

/// Timeout for quick control round-trips (status / passthrough / hello). Short
/// so a hung daemon can't stall the caller (and the UI's status polling, which
/// holds the manager lock) for long.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(3);
/// Longer timeout for `SetIntercept`, which may start the engine - binding the
/// port and building the MITM proxy waits up to ~10s for readiness.
const INTERCEPT_TIMEOUT: Duration = Duration::from_secs(15);

/// An authenticated, open control connection to the daemon.
pub struct HelperClient {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl HelperClient {
    /// Connect to a running daemon, spawning one (`<current-exe> --proxy-helper`,
    /// detached) if none is listening yet. Performs the `Hello` token handshake.
    pub fn connect_or_spawn() -> Result<HelperClient> {
        if let Ok(client) = Self::connect_existing() {
            return Ok(client);
        }
        spawn_daemon()?;
        // The daemon binds its socket + writes the token shortly after exec;
        // retry briefly until it's up (or give up so the UI surfaces an error).
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(50));
            if let Ok(client) = Self::connect_existing() {
                return Ok(client);
            }
        }
        anyhow::bail!("proxy helper did not come up within ~2s")
    }

    /// Connect to an already-listening daemon and authenticate, without
    /// spawning one. Errors if none is listening or the token handshake fails.
    pub fn connect_existing() -> Result<HelperClient> {
        let sock = control::socket_path()?;
        let stream = UnixStream::connect(&sock)
            .with_context(|| format!("connecting to {}", sock.display()))?;
        // Default; each round_trip sets the timeout appropriate to its request.
        stream
            .set_read_timeout(Some(CONTROL_TIMEOUT))
            .context("setting control read timeout")?;
        let writer = stream.try_clone().context("cloning control stream")?;
        let mut client = HelperClient {
            reader: BufReader::new(stream),
            writer,
        };

        let token = std::fs::read_to_string(control::token_path()?)
            .context("reading control token")?
            .trim()
            .to_string();
        match client.round_trip(&Request::Hello { token }, CONTROL_TIMEOUT)? {
            Response::Hello { ok: true } => Ok(client),
            Response::Hello { ok: false } => anyhow::bail!("control token rejected"),
            other => anyhow::bail!("unexpected Hello reply: {other:?}"),
        }
    }

    /// Start or live-update interception. Returns the loopback port the daemon
    /// is bound to (so the caller can point the system proxy at it).
    pub fn set_intercept(
        &mut self,
        gateway_base_url: &str,
        api_key: &str,
        ca_cert_pem: &str,
        ca_key_pem: &str,
        domains: &[ProxyDomain],
        preferred_port: Option<u16>,
    ) -> Result<u16> {
        let req = Request::SetIntercept {
            gateway_base_url: gateway_base_url.to_string(),
            api_key: api_key.to_string(),
            ca_cert_pem: ca_cert_pem.to_string(),
            ca_key_pem: ca_key_pem.to_string(),
            domains: domains.to_vec(),
            preferred_port,
        };
        match self.round_trip(&req, INTERCEPT_TIMEOUT)? {
            Response::Intercepting { port } => Ok(port),
            Response::Error { message } => anyhow::bail!("{message}"),
            other => anyhow::bail!("unexpected SetIntercept reply: {other:?}"),
        }
    }

    /// Drop the daemon to pass-through (blind-tunnel everything) while keeping
    /// the port bound.
    pub fn set_passthrough(&mut self) -> Result<()> {
        match self.round_trip(&Request::SetPassthrough, CONTROL_TIMEOUT)? {
            Response::Ok => Ok(()),
            other => anyhow::bail!("unexpected SetPassthrough reply: {other:?}"),
        }
    }

    /// Current daemon state: `(running, port, intercepting_count)`.
    pub fn status(&mut self) -> Result<(bool, Option<u16>, usize)> {
        match self.round_trip(&Request::Status, CONTROL_TIMEOUT)? {
            Response::Status {
                running,
                port,
                intercepting,
            } => Ok((running, port, intercepting)),
            other => anyhow::bail!("unexpected Status reply: {other:?}"),
        }
    }

    fn round_trip(&mut self, req: &Request, timeout: Duration) -> Result<Response> {
        // Bound the read so a hung daemon can't block the caller indefinitely.
        let _ = self.writer.set_read_timeout(Some(timeout));
        let mut line = serde_json::to_string(req).context("serializing request")?;
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .context("writing request")?;
        self.writer.flush().context("flushing request")?;
        let mut resp = String::new();
        let n = self
            .reader
            .read_line(&mut resp)
            .context("reading response")?;
        if n == 0 {
            anyhow::bail!("control connection closed by daemon");
        }
        serde_json::from_str(resp.trim_end()).context("parsing response")
    }
}

/// Spawn the daemon as a detached child: `setsid` so it leaves the GUI's
/// session/controlling terminal and survives the GUI exiting (it's still in the
/// user *login* session, so it's torn down at logout - the lifetime we want).
fn spawn_daemon() -> Result<()> {
    let exe = std::env::current_exe().context("resolving current exe")?;
    let mut cmd = Command::new(exe);
    cmd.arg("--proxy-helper")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe; we ignore its result (it fails only
    // if we're already a session leader, which is fine).
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn().context("spawning proxy helper")?;
    Ok(())
}
