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

fn port_file(name: &str) -> Result<PathBuf> {
    Ok(proxy_dir()?.join(format!("{name}.port")))
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
    let tmp = path.with_extension("port.tmp");
    std::fs::write(&tmp, port.to_string()).with_context(|| format!("writing {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644));
    }
    std::fs::rename(&tmp, &path).with_context(|| format!("renaming into {}", path.display()))
}

/// Liveness path the forwarder answers, under the same reserved prefix the
/// relay uses so no real destination can collide with it.
///
/// Here rather than in either binary because it is a wire contract between
/// them: the app probes it to prove the listener on the persisted port is the
/// forwarder it spawned, and not some other process that bound the port first.
pub const FORWARDER_HEALTH_PATH: &str = "/__gate/forwarder-health";

/// Header carrying the shared secret on a health probe. The secret lives in a
/// 0600 file the app writes before spawning; a probe without it is refused the
/// same way any unroutable request is, so probing cannot tell "wrong token"
/// from "not a forwarder".
pub const FORWARDER_HEALTH_TOKEN_HEADER: &str = "x-gate-forwarder-token";

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

#[cfg(test)]
mod tests {
    use super::*;

    struct TestHome(PathBuf);

    impl TestHome {
        fn set(tag: &str) -> Self {
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
            TestHome(dir)
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            std::env::remove_var(TEST_HOME);
            let _ = std::fs::remove_dir_all(&self.0);
        }
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
