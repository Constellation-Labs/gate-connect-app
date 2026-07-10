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

/// Sentinel error: a daemon is listening and authenticated, but reported a
/// different [`control::PROTOCOL_VERSION`] or [`control::BUILD_FINGERPRINT`]
/// (typically one left over from an older build). Carried via `anyhow` and
/// matched in [`HelperClient::connect_or_spawn`], which replaces the daemon
/// rather than reusing it.
#[derive(Debug)]
struct StaleDaemon;

impl std::fmt::Display for StaleDaemon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("proxy helper is running an incompatible protocol or build version")
    }
}

impl std::error::Error for StaleDaemon {}

impl HelperClient {
    /// Connect to a running daemon, spawning one (`<current-exe> --proxy-helper`,
    /// detached) if none is listening yet. Performs the `Hello` token handshake.
    pub fn connect_or_spawn() -> Result<HelperClient> {
        match Self::connect_existing() {
            Ok(client) => return Ok(client),
            // A daemon is listening but speaks an incompatible protocol (a
            // leftover from another build). Replace it: it was asked to shut
            // down in `connect_existing`; give it a moment, then force-kill if
            // it's still holding the socket, so `spawn_daemon` below isn't
            // wedged behind its singleton flock.
            Err(e) if e.is::<StaleDaemon>() => replace_stale_daemon(),
            Err(_) => {}
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
        // We reached a listening daemon. From here, anything short of a clean,
        // same-version Hello means it's a leftover we can't reuse - classify it
        // as `StaleDaemon` so `connect_or_spawn` replaces it (gracefully if it
        // still speaks the protocol, by force-kill otherwise) instead of getting
        // wedged behind its singleton flock.
        match client.round_trip(
            &Request::Hello {
                token,
                version: control::PROTOCOL_VERSION,
            },
            CONTROL_TIMEOUT,
        ) {
            Ok(Response::Hello { ok: false, .. }) => anyhow::bail!("control token rejected"),
            Ok(Response::Hello {
                ok: true,
                version,
                fingerprint,
            }) if version == control::PROTOCOL_VERSION
                && fingerprint == control::BUILD_FINGERPRINT =>
            {
                Ok(client)
            }
            // Authenticated, but the daemon reports a different protocol
            // version or build fingerprint (e.g. a build predating this one).
            // Ask it to shut down cleanly; `connect_or_spawn` force-kills if
            // it doesn't comply.
            Ok(Response::Hello { ok: true, .. }) => {
                let _ = client.round_trip(&Request::Shutdown, CONTROL_TIMEOUT);
                Err(anyhow::Error::new(StaleDaemon))
            }
            // An unexpected reply, or a reply we couldn't even parse/read: a
            // daemon speaking a protocol we don't understand. Replace it.
            Ok(_) | Err(_) => Err(anyhow::Error::new(StaleDaemon)),
        }
    }

    /// Start or live-update interception. Returns the loopback port the daemon
    /// is bound to (so the caller can point the system proxy at it).
    // Mirrors the `Request::SetIntercept` wire shape one-to-one; grouping the
    // fields into a struct would just duplicate that enum variant.
    #[allow(clippy::too_many_arguments)]
    pub fn set_intercept(
        &mut self,
        gateway_base_url: &str,
        api_key: &str,
        oauth_token: &str,
        ca_cert_pem: &str,
        ca_key_pem: &str,
        domains: &[ProxyDomain],
        preferred_port: Option<u16>,
    ) -> Result<u16> {
        let req = Request::SetIntercept {
            gateway_base_url: gateway_base_url.to_string(),
            api_key: api_key.to_string(),
            oauth_token: oauth_token.to_string(),
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

/// Replace a daemon that reported an incompatible protocol. It was already
/// asked to `Shutdown` in `connect_existing` (a no-op if it didn't understand
/// the request); give it a moment to exit on its own, then force-kill it so the
/// singleton flock is free for the replacement `spawn_daemon`.
fn replace_stale_daemon() {
    if wait_for_daemon_gone() {
        return;
    }
    force_kill_daemon();
    let _ = wait_for_daemon_gone();
}

/// Wait (briefly, bounded ~2s) for a daemon to release its socket, returning
/// whether it's gone. The next `spawn_daemon` guards startup with an exclusive
/// flock, so the old process must exit before we spawn the replacement, or the
/// new daemon would bail as a duplicate. "Gone" == the socket refuses connects.
fn wait_for_daemon_gone() -> bool {
    let Ok(sock) = control::socket_path() else {
        return true;
    };
    for _ in 0..40 {
        if UnixStream::connect(&sock).is_err() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Best-effort `SIGKILL` of the running daemon, identified by the pidfile it
/// wrote at startup. Used only after a graceful `Shutdown` failed to make it
/// exit. Guards against a recycled pid by confirming the target is actually a
/// proxy helper before signalling.
fn force_kill_daemon() {
    let Ok(pid_path) = control::pid_path() else {
        return;
    };
    let Some(pid) = std::fs::read_to_string(&pid_path)
        .ok()
        .and_then(|s| s.trim().parse::<libc::pid_t>().ok())
    else {
        return;
    };
    if pid <= 1 || !is_proxy_helper(pid) {
        return;
    }
    // SAFETY: `kill` with any pid and a valid signal is safe; we ignore the
    // result (the process may have exited between the check and here).
    unsafe {
        libc::kill(pid, libc::SIGKILL);
    }
}

/// Whether `/proc/<pid>/cmdline` looks like our detached proxy helper
/// (`<exe> --proxy-helper`), so `force_kill_daemon` never signals an unrelated
/// process that recycled the pid.
fn is_proxy_helper(pid: libc::pid_t) -> bool {
    std::fs::read(format!("/proc/{pid}/cmdline"))
        // argv entries are NUL-separated.
        .map(|raw| raw.split(|&b| b == 0).any(|arg| arg == b"--proxy-helper"))
        .unwrap_or(false)
}

/// Spawn the daemon as a detached child: `setsid` so it leaves the GUI's
/// session/controlling terminal and survives the GUI exiting (it's still in the
/// user *login* session, so it's torn down at logout - the lifetime we want).
fn spawn_daemon() -> Result<()> {
    let exe = std::env::current_exe().context("resolving current exe")?;
    let mut cmd = Command::new(exe);
    cmd.arg("--proxy-helper").stdin(Stdio::null());
    // The daemon is detached, so its stdio is normally discarded. Under
    // GATE_PROXY_DEBUG, tee it to a logfile so the per-request engine logs
    // are actually readable on Linux (tail proxy/helper.log). Quiet by
    // default in production.
    let debug_log = std::env::var_os("GATE_PROXY_DEBUG")
        .and_then(|_| crate::env::app_support_dir().ok())
        .map(|d| d.join("proxy").join("helper.log"));
    match debug_log {
        Some(path) => {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let open = || {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
            };
            match (open(), open()) {
                (Ok(out), Ok(err)) => {
                    cmd.stdout(Stdio::from(out)).stderr(Stdio::from(err));
                }
                _ => {
                    cmd.stdout(Stdio::null()).stderr(Stdio::null());
                }
            }
        }
        None => {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }
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
