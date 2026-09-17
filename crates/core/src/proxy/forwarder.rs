//! Lifecycle for `gate-connect-forwarder`, the small separate binary the
//! machine-wide environment variables point at.
//!
//! The forwarding itself lives in that binary; this module only starts one,
//! proves the thing answering is ours, and stops it. See the binary's own
//! module docs for why it exists at all, and `docs/security-notes-loopback.md`
//! for its posture.

// Only the desktop managers run a forwarder: Linux routes through a daemon that
// already outlives the GUI, so it has never had the failure this fixes. Kept
// compiled everywhere rather than `cfg`-ed out, so a Linux build still
// type-checks it and its tests run on every OS - the same reason
// `manager_core::hosts_live_engine` carries this attribute.
#![cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};

/// How long to wait for a freshly spawned forwarder to be answering.
const SPAWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Name of the sidecar binary, installed beside the app's own executable.
#[cfg(windows)]
const FORWARDER_BIN: &str = "gate-connect-forwarder.exe";
#[cfg(not(windows))]
const FORWARDER_BIN: &str = "gate-connect-forwarder";

fn proxy_file(name: &str) -> Result<PathBuf> {
    Ok(crate::env::app_support_dir()?.join("proxy").join(name))
}

/// Marker file whose presence means "the forwarder should be running". A file
/// rather than a signal: the same on all three platforms, and it cannot
/// mis-target a recycled PID.
fn marker_path() -> Result<PathBuf> {
    proxy_file("forwarder-wanted")
}

fn token_path() -> Result<PathBuf> {
    proxy_file("forwarder.token")
}

/// The port the forwarder last bound, if it has ever run.
pub(crate) fn persisted_port() -> Option<u16> {
    super::port_persist::load("forwarder-port").ok().flatten()
}

/// Read the shared secret, minting one if this install has none.
///
/// 0600, because it is the only thing separating "our forwarder holds that
/// port" from "something else got there first". A local process running as the
/// owner can read it, which is the same-user boundary this subsystem already
/// accepts; a *different* local user cannot, which is the case that matters.
fn load_or_create_token() -> Result<String> {
    let path = token_path()?;
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let existing = existing.trim().to_string();
        if !existing.is_empty() {
            return Ok(existing);
        }
    }
    use rand::Rng;
    let token: String = {
        let mut rng = rand::thread_rng();
        (0..32)
            .map(|_| format!("{:02x}", rng.gen::<u8>()))
            .collect()
    };
    crate::primitives::write_file(&path, token.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(token)
}

/// Whether the listener on `port` can prove it holds our token.
///
/// A plain TCP connect proves only that *something* is listening, which after a
/// port reuse is not a claim worth making - and here it decides what gets
/// exported machine-wide as `HTTPS_PROXY`, so adopting a stranger's listener
/// would hand it every tool's destinations and the cleartext of every
/// plain-HTTP request.
///
/// Challenge-response rather than sending the token. An earlier version put the
/// secret on the request and accepted any `204`, which proved nothing - any
/// listener can answer `204` - and handed the secret to the very process it was
/// trying to identify, so a squatter both passed the check and harvested the
/// token for next time. Now the app sends a fresh random challenge and requires
/// the SHA-256 of token-then-challenge back; only a process that can read the
/// 0600 token file can produce it, and a captured reply cannot be replayed
/// against the next probe.
///
/// Hand-rolled over a `TcpStream` rather than through `reqwest`: it is one
/// request on loopback, and a client here would have to be told `.no_proxy()`
/// anyway, because the app may have just pointed `HTTPS_PROXY` at this port.
fn health_ok(port: u16, token: &str) -> bool {
    use rand::Rng;
    use std::io::{Read, Write};

    let challenge: String = {
        let mut rng = rand::thread_rng();
        (0..16)
            .map(|_| format!("{:02x}", rng.gen::<u8>()))
            .collect()
    };
    let expected = gate_connect_paths::forwarder_proof(token, &challenge);

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut sock) = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(250))
    else {
        return false;
    };
    let _ = sock.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = sock.set_write_timeout(Some(Duration::from_millis(500)));
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: 127.0.0.1\r\n{}: {}\r\nConnection: close\r\n\r\n",
        gate_connect_paths::FORWARDER_HEALTH_PATH,
        gate_connect_paths::FORWARDER_CHALLENGE_HEADER,
        challenge
    );
    if sock.write_all(req.as_bytes()).is_err() {
        return false;
    }
    // Bounded: a listener that answers slowly forever must not hold up an
    // enable, and the proof is 64 hex characters in a short head.
    let mut buf = Vec::new();
    let mut chunk = [0u8; 512];
    while buf.len() < 4096 {
        match sock.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let Ok(text) = std::str::from_utf8(&buf) else {
        return false;
    };
    text.lines()
        .filter_map(|line| line.split_once(':'))
        .any(|(name, value)| {
            name.trim()
                .eq_ignore_ascii_case(gate_connect_paths::FORWARDER_PROOF_HEADER)
                && gate_connect_paths::constant_time_eq(
                    value.trim().as_bytes(),
                    expected.as_bytes(),
                )
        })
}

/// Where the sidecar lives: beside the executable asking for it.
///
/// The bundler installs it next to the app binary (`externalBin`), and a dev
/// build puts both in the same `target/` directory, so one rule covers both.
fn forwarder_binary() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("resolving current exe")?;
    let dir = exe
        .parent()
        .context("current exe has no parent directory")?;
    let candidate = dir.join(FORWARDER_BIN);
    if !candidate.exists() {
        anyhow::bail!(
            "the forwarder binary is not installed beside {} (looked for {})",
            exe.display(),
            candidate.display()
        );
    }
    Ok(candidate)
}

/// Spawn the forwarder detached, so it outlives this process.
///
/// `setsid` on Unix leaves the GUI's session and controlling terminal (still in
/// the login session, so it goes at logout - the lifetime we want);
/// `DETACHED_PROCESS` is the Windows equivalent.
fn spawn_detached() -> Result<()> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(forwarder_binary()?);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe; its only failure is "already a
        // session leader", which is harmless here.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        cmd.creation_flags(DETACHED_PROCESS);
    }
    cmd.spawn().context("spawning the environment forwarder")?;
    Ok(())
}

/// Socket activation, macOS only.
///
/// With a `Sockets` entry in a LaunchAgent plist, launchd binds and listens on
/// the port **itself, at login**, and starts the forwarder on the first
/// connection, handing over the already-listening descriptor. Three things
/// follow, and together they are why this is preferred over spawning:
///
/// - the address answers from login onward even with no Gate process in
///   existence, so there is no window in which a tool holding our exported
///   variables is stranded - not even between boot and the app launching;
/// - nothing sits resident between uses: the forwarder exits when idle and
///   launchd starts it again on demand;
/// - the port cannot be squatted, because launchd took it before any other
///   process could. That removes the case the health token exists to catch,
///   rather than merely detecting it.
///
/// Everything here is best-effort and self-correcting: if the agent does not
/// produce a healthy forwarder, [`ensure_running`] removes it and falls back to
/// spawning one directly, which is the path the other platforms use anyway.
/// Compiled on every unix, called only on macOS. `launchctl` is not there to
/// answer on Linux, but a module that a Linux build never type-checks is one
/// whose errors surface for the first time on a Mac - and this is the piece
/// least often exercised.
#[cfg(unix)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod launch_agent {
    use super::*;

    const LABEL: &str = "ai.constellation.gate-connect.forwarder";
    /// Must match the socket name the forwarder asks `launch_activate_socket`
    /// for.
    const SOCKET_NAME: &str = "Forwarder";

    fn plist_path() -> Result<PathBuf> {
        let home = dirs::home_dir().context("resolving the home directory")?;
        Ok(home
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{LABEL}.plist")))
    }

    /// Pick the port launchd will hold. Reuses the persisted one so the
    /// variables exported by a previous session stay valid; otherwise takes a
    /// free one from the band and releases it for launchd to claim.
    fn choose_port() -> Result<u16> {
        if let Some(port) = persisted_port() {
            return Ok(port);
        }
        // Skip what this install already remembers, or launchd would hold the
        // engine's or relay's port for good and those listeners would move
        // every run - stranding exactly the baked configs they exist to keep.
        let taken: Vec<u16> = ["port", "pac-port", "relay-port"]
            .iter()
            .filter_map(|n| crate::proxy::port_persist::load(n).ok().flatten())
            .collect();
        let listener =
            gate_connect_paths::bind_fresh(&taken).context("choosing a forwarder port")?;
        let port = listener.local_addr()?.port();
        // Released so launchd can bind it. A third party could take it in the
        // gap, which is why `install` verifies with a health check afterwards
        // rather than assuming.
        drop(listener);
        Ok(port)
    }

    fn plist(binary: &std::path::Path, port: u16) -> String {
        // Hand-written rather than via a plist crate: it is eleven keys, and a
        // dependency that can emit XML is not worth adding for a file this
        // shape. Values are a path we resolved and a number we chose, so there
        // is nothing here that needs escaping.
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{}</string></array>
  <key>Sockets</key>
  <dict>
    <key>{SOCKET_NAME}</key>
    <dict>
      <key>SockNodeName</key><string>127.0.0.1</string>
      <key>SockServiceName</key><string>{port}</string>
      <key>SockType</key><string>stream</string>
      <key>SockFamily</key><string>IPv4</string>
    </dict>
  </dict>
  <key>ProcessType</key><string>Background</string>
</dict>
</plist>
"#,
            binary.display()
        )
    }

    fn launchctl(args: &[&str]) -> bool {
        std::process::Command::new("/bin/launchctl")
            .args(args)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn domain() -> String {
        // SAFETY: `getuid` takes no arguments and cannot fail.
        format!("gui/{}", unsafe { libc::getuid() })
    }

    /// Install (or refresh) the agent and return the port launchd is holding.
    pub(super) fn install() -> Result<u16> {
        let binary = forwarder_binary()?;
        let port = choose_port()?;
        let path = plist_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let body = plist(&binary, port);
        let refresh = std::fs::read_to_string(&path)
            .map(|old| old != body)
            .unwrap_or(true);
        if refresh {
            std::fs::write(&path, &body).with_context(|| format!("writing {}", path.display()))?;
            // Bootout first so a changed plist (new port, or a new binary path
            // after an app update) actually takes effect; a bootstrap over a
            // loaded label is refused.
            let target = format!("{}/{LABEL}", domain());
            launchctl(&["bootout", &target]);
        }
        let path_str = path.to_string_lossy().to_string();
        if !launchctl(&["bootstrap", &domain(), &path_str]) {
            // Already loaded is the common "failure" here, and is fine: the
            // health check below is what decides whether this worked.
        }
        // Record the port only once launchd has it, so the file the app reads
        // never names a port nothing is holding.
        crate::proxy::port_persist::save("forwarder-port", port)
            .context("recording the forwarder port")?;
        Ok(port)
    }

    /// Remove the agent entirely. Used when it did not work, and when the user
    /// asks Gate to let go of the machine.
    pub(super) fn remove() {
        launchctl(&["bootout", &format!("{}/{LABEL}", domain())]);
        if let Ok(path) = plist_path() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Ensure a forwarder is running and return the port the machine-wide variables
/// should name.
///
/// Idempotent, and cheap in the common case: an already-running forwarder costs
/// one loopback request. The marker is written first so a forwarder that starts
/// fast cannot read it before it exists and exit immediately.
pub(crate) fn ensure_running() -> Result<u16> {
    let marker = marker_path()?;
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    crate::primitives::write_file(&marker, b"", 0o600)
        .with_context(|| format!("writing {}", marker.display()))?;
    let token = load_or_create_token()?;

    if let Some(port) = persisted_port() {
        if health_ok(port, &token) {
            return Ok(port);
        }
    }

    // macOS: let launchd own the socket if it will. Verified rather than
    // assumed - if the agent does not produce a forwarder that answers, it is
    // removed and we fall back to spawning one, which is what the other
    // platforms do anyway.
    #[cfg(target_os = "macos")]
    {
        match launch_agent::install() {
            Ok(port) => {
                if await_health(port, &token) {
                    return Ok(port);
                }
                eprintln!(
                    "gate proxy: the forwarder launch agent did not answer; removing it and \
                     starting the forwarder directly"
                );
                launch_agent::remove();
            }
            Err(e) => eprintln!("gate proxy: could not install the forwarder launch agent ({e:#})"),
        }
    }

    spawn_detached()?;

    let deadline = std::time::Instant::now() + SPAWN_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if let Some(port) = persisted_port() {
            if health_ok(port, &token) {
                return Ok(port);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    anyhow::bail!("the environment forwarder did not answer within {SPAWN_TIMEOUT:?}")
}

/// Poll `port` until it answers as ours, or the spawn budget runs out.
fn await_health(port: u16, token: &str) -> bool {
    let deadline = std::time::Instant::now() + SPAWN_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if health_ok(port, token) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Ask a running forwarder to exit, by removing the marker it polls.
///
/// Best-effort and promptless. Deliberately *not* called from `disable`: a
/// forwarder that went away when routing was switched off would strand exactly
/// the processes it exists to protect. The callers are the explicit "Gate
/// should let go of this machine" paths - signing out and untrusting the CA.
pub fn stop() {
    if let Ok(path) = marker_path() {
        let _ = std::fs::remove_file(path);
    }
    // And take the agent down, or launchd would start a fresh forwarder on the
    // next connection - the marker only retires the process, not the thing that
    // keeps re-creating it.
    #[cfg(target_os = "macos")]
    launch_agent::remove();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Per-test hermetic home, serialized process-wide because the path seam is
    /// an environment variable.
    struct TestHome {
        dir: PathBuf,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TestHome {
        fn set(tag: &str) -> Self {
            let guard = crate::env::path_env_lock();
            let dir =
                std::env::temp_dir().join(format!("gate-connect-fwd-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);
            TestHome { dir, _guard: guard }
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            std::env::remove_var("GATE_CONNECT_TEST_HOME");
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn a_minted_token_is_stable_and_not_guessable() {
        let _home = TestHome::set("token");
        let first = load_or_create_token().expect("mint");
        assert_eq!(first.len(), 64, "32 bytes, hex encoded");
        assert_ne!(first, "0".repeat(64));
        // Stable across calls: the forwarder reads the file once at startup, so
        // re-minting on each probe would make every health check fail.
        assert_eq!(first, load_or_create_token().expect("reuse"));
    }

    /// The point of the challenge. A listener that merely answers - including
    /// one that answers `204` to everything, which is exactly what a squatter
    /// would do - must not be adopted, because adopting it exports a stranger's
    /// port as the machine's `HTTPS_PROXY`.
    #[test]
    fn a_listener_that_cannot_prove_the_token_is_not_ours() {
        let _home = TestHome::set("health");

        for reply in [
            &b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n"[..],
            &b"HTTP/1.1 204 No Content\r\nx-gate-forwarder-proof: nope\r\n\r\n"[..],
        ] {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            std::thread::spawn(move || {
                use std::io::{Read, Write};
                if let Ok((mut sock, _)) = listener.accept() {
                    let mut buf = [0u8; 1024];
                    let _ = sock.read(&mut buf);
                    let _ = sock.write_all(reply);
                }
            });
            assert!(
                !health_ok(port, "the-token"),
                "answering without the proof must not be adopted"
            );
        }

        // And a port with nothing on it at all is not ours either.
        let free = {
            let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            l.local_addr().unwrap().port()
        };
        assert!(!health_ok(free, "the-token"));
    }

    /// The other half: a listener that *can* prove it is adopted.
    #[test]
    fn a_listener_that_proves_the_token_is_ours() {
        let _home = TestHome::set("health-ok");
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let n = sock.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let challenge = req
                .lines()
                .find_map(|l| {
                    let (name, value) = l.split_once(':')?;
                    name.trim()
                        .eq_ignore_ascii_case(gate_connect_paths::FORWARDER_CHALLENGE_HEADER)
                        .then(|| value.trim().to_string())
                })
                .unwrap_or_default();
            let proof = gate_connect_paths::forwarder_proof("the-token", &challenge);
            let _ = sock.write_all(
                format!(
                    "HTTP/1.1 204 No Content\r\n{}: {proof}\r\nConnection: close\r\n\r\n",
                    gate_connect_paths::FORWARDER_PROOF_HEADER
                )
                .as_bytes(),
            );
        });
        assert!(health_ok(port, "the-token"));
    }
}
