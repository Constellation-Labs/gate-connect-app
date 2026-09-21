//! The handful of primitives the app and the standalone forwarder binary must
//! agree on exactly: where Gate Connect's per-user files live, how a loopback
//! port is persisted, and how one is bound.
//!
//! This crate exists so `gate-connect-forwarder` can stay a small binary. It
//! cannot depend on `gate-connect-core` - that would link the keychain, the
//! MITM stack, rustls and reqwest into a process whose whole job is to copy
//! bytes between two sockets - and it must not *duplicate* these either: the
//! forwarder and the app read and write the same files, so a disagreement
//! about where they are is a failure with no error message, just two processes
//! quietly looking at different directories.
//!
//! Nothing here touches the network beyond binding a loopback listener, and
//! nothing here handles a credential.

use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

/// Test seam, read from the environment rather than a process-global so it
/// survives into a spawned child. Mirrors `gate_connect_core::env`, which is
/// the definition of record; this crate reads it so a test that redirects the
/// app also redirects the forwarder it spawns.
const TEST_HOME: &str = "GATE_CONNECT_TEST_HOME";

/// Per-OS data directory for Gate Connect's own files.
///
/// - macOS: `~/Library/Application Support/Gate Connect`
/// - Windows: `%LOCALAPPDATA%\Gate Connect`
/// - Linux: `$XDG_DATA_HOME/Gate Connect`
pub fn app_support_dir() -> Result<PathBuf> {
    // Debug builds only, matching `gate_connect_core::env::test_seam`. A
    // release binary that honoured this would let anything able to set the
    // environment - `launchctl setenv`, a mechanism Gate itself uses - redirect
    // which files the app and the forwarder agree on, and with it which port
    // the app probes and then exports machine-wide.
    #[cfg(debug_assertions)]
    if let Some(home) = std::env::var_os(TEST_HOME).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(home).join("app-support").join("Gate Connect"));
    }
    Ok(dirs::data_local_dir()
        .context("could not resolve user data directory")?
        .join("Gate Connect"))
}

/// The `proxy/` subdirectory, where every port file and marker lives.
pub fn proxy_dir() -> Result<PathBuf> {
    Ok(app_support_dir()?.join("proxy"))
}

/// The file a port is persisted in.
///
/// **No extension.** The app has written `proxy/port`, `proxy/pac-port` and
/// `proxy/relay-port` since long before this crate existed, and every install
/// in the field has them. An earlier draft of this function appended `.port`,
/// which type-checked, passed every test - the tests inject the lookup - and
/// silently broke the one thing the forwarder is for: it never found the
/// engine, so every proxied connection went direct, bypassing the MITM engine
/// and its routing rules while the app still reported routing as on. `core`'s
/// `port_persist` calls this rather than keeping its own copy, so there is one
/// definition and it cannot drift again.
pub fn port_file(name: &str) -> Result<PathBuf> {
    Ok(proxy_dir()?.join(name))
}

/// The port persisted under `name`, if any and still parseable. A missing or
/// unreadable file reads as `None` rather than failing the caller: not knowing
/// the previous port is a normal state, and it only ever costs a fresh bind.
pub fn load_port(name: &str) -> Option<u16> {
    let path = port_file(name).ok()?;
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse::<u16>()
        .ok()
}

/// Persist `port` under `name` for reuse on the next run. Non-secret, so 0644
/// (the mode is ignored on Windows); written via a temp file and a rename so a
/// reader never sees a half-written number.
pub fn save_port(name: &str, port: u16) -> Result<()> {
    let path = port_file(name)?;
    let dir = path.parent().context("port file has no parent")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, port.to_string()).with_context(|| format!("writing {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644));
    }
    std::fs::rename(&tmp, &path).with_context(|| format!("renaming into {}", path.display()))
}

/// Liveness path the forwarder answers. Under a reserved prefix so it cannot
/// collide with a real destination - a proxy request names an absolute URI, and
/// this is origin-form, so nothing a client legitimately proxies reaches it.
///
/// Here rather than in either binary because it is a wire contract between
/// them: the app probes it to prove the listener on the persisted port is the
/// forwarder it spawned, and not some other process that bound the port first.
pub const FORWARDER_HEALTH_PATH: &str = "/__gate/forwarder-health";

/// Header carrying the probe's random challenge.
///
/// The app never sends the secret. An earlier design did - it put the token on
/// the request and accepted any `204` - which proved nothing (a squatter can
/// answer `204` to anything) *and* handed the secret to the very process it was
/// trying to identify. The challenge is fresh per probe, so a reply cannot be
/// replayed either.
pub const FORWARDER_CHALLENGE_HEADER: &str = "x-gate-forwarder-challenge";

/// Reserved path the **relay** answers the same proof on.
///
/// A separate path from the forwarder's so a probe cannot mistake one listener
/// for the other, and under the same `/__gate/` prefix the relay already
/// reserves for its own routing segments, so it can never collide with a tool's
/// base URL.
pub const RELAY_HEALTH_PATH: &str = "/__gate/relay-health";

/// Header the relay reports interception on: `1` while it rewrites to the
/// gateway, `0` while it is parked and forwarding straight through.
///
/// Readable by any process that can reach the port, which is the same-user
/// boundary `docs/security-notes-loopback.md` already accepts, and it carries
/// nothing secret: whether Gate is routing is what the app's own window says.
pub const RELAY_INTERCEPTING_HEADER: &str = "x-gate-relay-intercepting";

/// Header carrying the forwarder's answer: hex SHA-256 of the token followed by
/// the challenge. Only a process that can read the 0600 token file can produce
/// it, which is exactly the claim the app needs before exporting the port it
/// answers on as the machine's `HTTPS_PROXY`.
pub const FORWARDER_PROOF_HEADER: &str = "x-gate-forwarder-proof";

/// The proof a forwarder holding `token` owes for `challenge`.
///
/// Defined here so the two binaries cannot disagree about it, and so the one
/// property that matters is testable in one place: knowing the challenge is not
/// enough to produce the answer.
pub fn forwarder_proof(token: &str, challenge: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.update(challenge.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Compare without an early exit on the first differing byte. Timing a local
/// comparison over loopback is far-fetched, but one that leaks its own prefix
/// is not worth keeping.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Port band a *fresh* listener is picked from. Sits inside IANA's registered
/// range and below Windows' default dynamic range (49152-65535), clear of
/// assigned neighbours like 47001 (WinRM).
///
/// Defined here because the engine and the forwarder pick from the same band
/// and must not collide with each other's remembered ports.
pub const STABLE_PORT_RANGE: std::ops::Range<u16> = 47100..47200;

/// How long a preferred-port bind keeps retrying before conceding the port.
/// Sized for a previous session still letting go, not for waiting out another
/// application.
const PREFERRED_BIND_GRACE: Duration = Duration::from_millis(250);

/// Whether something is accepting on a loopback port right now.
///
/// The one question a bind failure cannot answer on its own: a port held only
/// by `TIME_WAIT` remnants refuses, a live listener accepts.
pub fn port_is_live(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(250),
    )
    .is_ok()
}

#[cfg(unix)]
fn bind_preferred_once(port: u16) -> std::io::Result<TcpListener> {
    if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
        return Ok(listener);
    }
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    if port_is_live(port) {
        // Someone is live on this port - never shadow it. `SO_REUSEADDR` below
        // would happily bind `127.0.0.1:P` underneath another process's
        // `0.0.0.0:P` listener and silently steal its loopback traffic.
        return Err(std::io::Error::from(std::io::ErrorKind::AddrInUse));
    }
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )?;
    socket.set_reuse_address(true)?;
    socket.bind(&addr.into())?;
    socket.listen(128)?;
    Ok(socket.into())
}

#[cfg(not(unix))]
fn bind_preferred_once(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port))
}

/// Bind the preferred port, retrying briefly before conceding it.
///
/// A single attempt answers "is the port free right now", and for a port we are
/// trying to *re*claim that is the wrong question: our own previous session may
/// still be releasing it, and conceding on the first refusal is what makes a
/// quick restart land somewhere new - precisely what a persisted port exists to
/// prevent.
pub fn bind_preferred(port: u16) -> std::io::Result<TcpListener> {
    let deadline = Instant::now() + PREFERRED_BIND_GRACE;
    loop {
        match bind_preferred_once(port) {
            Ok(listener) => return Ok(listener),
            Err(e) if Instant::now() >= deadline => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// Bind a fresh listener for a port the caller intends to persist and rebind on
/// later runs, skipping ports this install already remembers.
///
/// Why not `:0`: a port the OS hands out as ephemeral is one it hands out to
/// everything else, and nothing holds it while Gate is stopped - so the next
/// run's rebind races every local process that opened an outbound socket in the
/// meantime. On Windows it is worse than a race: Hyper-V, WSL2 and Docker
/// Desktop reserve whole blocks of the dynamic range at boot, so a persisted
/// port inside a reserved block fails to bind on *every* start and silently
/// moves each time. Either way the clients that captured the old port are
/// stranded, which for this crate's callers is the entire failure being
/// avoided.
pub fn bind_fresh(skip: &[u16]) -> std::io::Result<TcpListener> {
    for port in STABLE_PORT_RANGE {
        if skip.contains(&port) {
            continue;
        }
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return Ok(listener);
        }
    }
    // Whole band unavailable: an ephemeral port keeps this session alive, but
    // the next run may not get it back, so name the reason rather than letting
    // the moving-port symptom come back unexplained.
    eprintln!(
        "gate: no free port in {STABLE_PORT_RANGE:?}; binding an ephemeral port instead. \
         It may not survive a restart."
    );
    TcpListener::bind(("127.0.0.1", 0))
}

/// Does the listener on `port` prove it holds `token`?
///
/// The client half of the challenge-response both of Gate's loopback listeners
/// answer: a random challenge goes out on one header, and only a process that
/// can read the 0600 token file can return the matching hash. A bare TCP
/// connect cannot tell our listener from anything else that happens to accept,
/// which is the difference between "the port is taken" and "the port is ours".
///
/// Shared rather than written twice because the forwarder and the relay differ
/// only in which reserved path they answer on. Every timeout is short and
/// bounded: a listener that answers slowly forever must not hold up an enable
/// or a status read.
pub fn proves_ours(port: u16, health_path: &str, token: &str) -> bool {
    probe_with_proof(port, health_path, token).is_some()
}

/// [`proves_ours`], keeping the headers the listener returned.
///
/// The proof says the listener is ours; the headers are what it reports about
/// itself. Returned together and only together, so nothing can read a claim
/// from something that failed to prove it made it.
pub fn probe_with_proof(
    port: u16,
    health_path: &str,
    token: &str,
) -> Option<Vec<(String, String)>> {
    use std::io::{Read, Write};
    use std::time::Duration;

    let challenge: String = {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        (0..16)
            .map(|_| format!("{:02x}", rng.gen::<u8>()))
            .collect()
    };
    let expected = forwarder_proof(token, &challenge);

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut sock = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(250)).ok()?;
    let _ = sock.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = sock.set_write_timeout(Some(Duration::from_millis(500)));
    let req = format!(
        "GET {health_path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{FORWARDER_CHALLENGE_HEADER}: \
         {challenge}\r\nConnection: close\r\n\r\n"
    );
    sock.write_all(req.as_bytes()).ok()?;
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
    let text = std::str::from_utf8(&buf).ok()?;
    let headers: Vec<(String, String)> = text
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    let proved = headers.iter().any(|(name, value)| {
        name == FORWARDER_PROOF_HEADER && constant_time_eq(value.as_bytes(), expected.as_bytes())
    });
    proved.then_some(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The path seam is an environment variable, which is process-global, so
    /// tests that set it cannot overlap. `core` carries the same lock for the
    /// same reason; without one these fail under the default parallel runner
    /// and pass under `--test-threads=1`, which is the worst way to find out.
    fn path_env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    struct TestHome {
        dir: PathBuf,
        /// Held for the test's life; released on drop with the env var.
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TestHome {
        fn set(tag: &str) -> Self {
            let guard = path_env_lock();
            let dir = std::env::temp_dir().join(format!(
                "gate-connect-paths-{}-{}-{tag}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var(TEST_HOME, &dir);
            TestHome { dir, _guard: guard }
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            std::env::remove_var(TEST_HOME);
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// The point of the challenge: the answer cannot be produced from the
    /// challenge alone, so a process that cannot read the token file cannot
    /// pass, however it replies.
    #[test]
    fn a_proof_needs_the_token_not_just_the_challenge() {
        let proof = forwarder_proof("the-token", "abc123");
        assert_eq!(proof.len(), 64);
        assert_eq!(proof, forwarder_proof("the-token", "abc123"));
        assert_ne!(proof, forwarder_proof("another-token", "abc123"));
        // And it is bound to the challenge, so one reply cannot be replayed
        // against the next probe.
        assert_ne!(proof, forwarder_proof("the-token", "abc124"));
    }

    #[test]
    fn constant_time_eq_is_still_an_equality() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn a_port_round_trips_and_a_missing_one_reads_as_none() {
        let _home = TestHome::set("round-trip");
        assert_eq!(load_port("never-written"), None);
        save_port("engine", 47_123).unwrap();
        assert_eq!(load_port("engine"), Some(47_123));
        // Distinct names must not clobber each other.
        save_port("forwarder", 47_124).unwrap();
        assert_eq!(load_port("engine"), Some(47_123));
        assert_eq!(load_port("forwarder"), Some(47_124));
    }

    #[test]
    fn unparseable_content_reads_as_none_rather_than_failing() {
        let _home = TestHome::set("garbage");
        let path = port_file("torn").unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not a port").unwrap();
        assert_eq!(load_port("torn"), None);
    }

    #[test]
    fn a_fresh_bind_lands_in_the_band_and_skips_what_we_already_hold() {
        let held = bind_fresh(&[]).unwrap();
        let held_port = held.local_addr().unwrap().port();
        assert!(
            STABLE_PORT_RANGE.contains(&held_port),
            "fresh binds come from the band, not the ephemeral range"
        );
        let other = bind_fresh(&[held_port]).unwrap();
        assert_ne!(other.local_addr().unwrap().port(), held_port);
    }

    #[test]
    fn a_preferred_bind_refuses_to_shadow_a_live_listener() {
        let live = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = live.local_addr().unwrap().port();
        // Shadowing is the failure that matters: `SO_REUSEADDR` makes it
        // possible, and it would silently steal another app's loopback traffic.
        assert!(bind_preferred(port).is_err());
        assert!(port_is_live(port));
        drop(live);
    }
}
