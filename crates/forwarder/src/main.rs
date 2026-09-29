//! `gate-connect-forwarder` - the tiny always-there proxy the machine-wide
//! environment variables point at.
//!
//! # Why this exists
//!
//! The two channels that carry Gate's proxy fail differently, and only one of
//! them fails safely. A PAC naming a dead port **fails open**: the fetch fails
//! and the client goes DIRECT. An exported `HTTPS_PROXY` naming a dead port
//! **fails closed**: the client dials a port that refuses and the request dies.
//!
//! `launchctl unsetenv` (and the Windows registry write beside it) only changes
//! what processes started *afterwards* inherit, so every shell, editor and CLI
//! already running keeps the variable for its whole life. The moment the
//! engine's port goes away - routing switched off, the app quit, the engine
//! crashed - none of them can reach any provider. Not just the tools Gate
//! manages: the export is machine-wide, so it catches curl, git, npm and
//! software Gate does not claim to touch.
//!
//! Pointing the variables here instead gives the env channel the PAC's
//! fail-open behaviour. The PAC names this port too: a browser caches the
//! script it fetched, so "the fetch fails and the client goes DIRECT" only ever
//! covered a browser that refetched, and one holding the cached body dialed
//! the engine's dead port for every Gate host and failed closed.
//!
//! # Why a separate binary
//!
//! Because the failure it fixes includes the GUI going away, so it cannot live
//! in the GUI process. It is a separate *binary* rather than the app re-invoked
//! with a flag (which is how the Linux helper daemon works) for three reasons:
//! a running process holds a file lock on its own executable, and on Windows
//! that would block the updater from replacing the app; a distinct name in
//! Activity Monitor and Task Manager answers "why are there two Gate Connects"
//! honestly; and linking none of the app's machinery - no keychain, no MITM
//! stack, no Gate credential - is what keeps a permanently-running process free
//! of anything worth stealing. Its one heavy dependency is a TLS client, for
//! the relay listener's direct path.
//!
//! # The relay port
//!
//! It also holds the port relay tool configs name (Codex, OpenCode), for the
//! same reason: a config outlives the process that wrote it. While the app's
//! engine is up, relay connections are handed to it untouched; while it is not,
//! they go straight to the provider their base URL names, under the tool's own
//! credential. That listener is the one place this binary reads a request and
//! opens TLS, and [`relay`] says what bounds it.
//!
//! It holds no Gate credential, terminates no TLS, mints no certificate and
//! makes no routing decision beyond "is the engine there". Its security posture
//! is recorded in `docs/security-notes-loopback.md`.

mod peer;
mod proxy;
mod relay;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::net::TcpListener;

/// Name under which the forwarder's own port is persisted. Stability is the
/// point: the exported variables name this port, and a process that read them
/// at its own launch keeps dialing it for the rest of its life.
pub const PORT_NAME: &str = "forwarder-port";

/// Name under which the *engine's* port is persisted, written by the app.
const ENGINE_PORT_NAME: &str = "port";

/// How often the forwarder checks whether it is still wanted.
const MARKER_POLL: Duration = Duration::from_secs(2);

/// How long a forwarder that is no longer wanted keeps serving the connections
/// it already accepted, having given up both ports at once.
///
/// Retiring is how an out-of-date forwarder is replaced, which after an update
/// is routine, and exiting at once cut every tunnel mid-transfer: a streamed
/// model answer halfway out, a download. Letting go of the ports first is what
/// makes waiting free - the replacement binds them straight away - so the
/// bound is set by the longest single response worth protecting, not by how
/// long the port may sit empty. A tunnel still open past it is one a client is
/// keeping open, not one carrying a response.
const DRAIN_LIMIT: Duration = Duration::from_secs(10 * 60);

/// [`DRAIN_LIMIT`] when the ask to stop was SIGTERM, which leaves far less
/// room - see [`terminated`].
const SIGTERM_DRAIN_LIMIT: Duration = Duration::from_secs(5);

/// How often the engine watcher dials once the engine has answered.
///
/// Only a return needs catching quickly. After that the watcher is looking for
/// the engine going away again, and a tunnel that never goes quiet - a
/// WebSocket with frequent heartbeats, a long stream - would otherwise have it
/// dialing every [`MARKER_POLL`] for that tunnel's whole life. Kept well under
/// [`proxy::RECLAIM_IDLE`]: a tunnel that opens on a stale "up" is not closed
/// before a later check has corrected it.
const ENGINE_POLL_WHILE_UP: Duration = Duration::from_secs(30);

fn proxy_file(name: &str) -> Result<std::path::PathBuf> {
    Ok(gate_connect_paths::proxy_dir()?.join(name))
}

/// Marker file whose presence means "the forwarder should be running".
///
/// A file rather than a signal or a control socket: it is the same on all three
/// platforms and cannot mis-target a recycled PID. Stopping is a delete: the
/// forwarder lets go of its ports within [`MARKER_POLL`], then finishes the
/// connections it already has, for up to [`DRAIN_LIMIT`].
fn marker_path() -> Result<std::path::PathBuf> {
    proxy_file("forwarder-wanted")
}

/// Shared secret proving a listener on the persisted port is the forwarder the
/// app spawned. Written 0600 by the app before spawning; read once here.
///
/// Without it, "something answers on the port we remember" is all the app can
/// check, and it would then export that port machine-wide - handing every
/// tool's traffic to whatever process happened to bind it first.
fn token_path() -> Result<std::path::PathBuf> {
    proxy_file("forwarder.token")
}

fn main() {
    if let Err(e) = run() {
        eprintln!("gate-connect-forwarder: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let token: Arc<str> = std::fs::read_to_string(token_path()?)
        .context("reading the forwarder token (the app writes it before spawning)")?
        .trim()
        .into();
    if token.is_empty() {
        anyhow::bail!("the forwarder token is empty; refusing to start unidentifiable");
    }
    // Taken before anything else can happen, and never re-read: the file is
    // what an update replaces, and the point is to report the build this
    // process is running, not the one that has since landed on disk.
    let _ = proxy::BUILD.set(
        std::env::current_exe()
            .ok()
            .and_then(|exe| gate_connect_paths::binary_identity(&exe)),
    );

    let listener = bind()?;
    let port = listener
        .local_addr()
        .context("reading the forwarder's own address")?
        .port();
    // Recorded only once we hold the port, so the file the app reads to decide
    // what to export never names a port we failed to get.
    gate_connect_paths::save_port(PORT_NAME, port).context("recording the forwarder port")?;
    listener
        .set_nonblocking(true)
        .context("setting the listener non-blocking")?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building the forwarder runtime")?;
    rt.block_on(async move {
        let listener = TcpListener::from_std(listener)?;
        let mut serving = Serving::new();
        serving.sigterm = true;
        // The relay port is taken beside this one, not instead of it: a
        // forwarder that cannot get it still does its first job.
        relay::start(
            port,
            token.clone(),
            serving.in_flight.clone(),
            serving.retire.subscribe(),
        );
        serve_with(
            listener,
            Arc::new(|| gate_connect_paths::load_port(ENGINE_PORT_NAME)),
            port,
            token,
            serving,
        )
        .await
    })
}

/// Take the listening socket, from launchd where it offers one and by binding
/// otherwise.
fn bind() -> Result<std::net::TcpListener> {
    if let Some(listener) = activated_socket("Forwarder") {
        return Ok(listener);
    }
    let skip = gate_connect_paths::remembered_ports_except(PORT_NAME);
    match gate_connect_paths::load_port(PORT_NAME) {
        // Reclaim the port the exported variables already name. `bind_preferred`
        // refuses to shadow a live listener, so if something else holds it we
        // fall through to a fresh one rather than silently stealing its traffic.
        Some(port) => gate_connect_paths::bind_preferred(port)
            .or_else(|_| gate_connect_paths::bind_fresh(&skip)),
        None => gate_connect_paths::bind_fresh(&skip),
    }
    .context("binding the forwarder listener")
}

/// The listening socket launchd is holding for us under `name`, if we were
/// socket-activated. The agent declares `Forwarder` always and `Relay` when the
/// relay port was free to hand launchd at install.
///
/// With a `Sockets` entry in the LaunchAgent plist, launchd binds and listens on
/// the port itself at login and starts this process on the first connection,
/// handing over the already-listening descriptor. Two things follow, and both
/// are the reason to prefer it: the address answers from login onward even with
/// no Gate process in existence, so there is no window where a tool is
/// stranded; and nothing can squat the port, because launchd took it before any
/// other process could.
#[cfg(target_os = "macos")]
pub(crate) fn activated_socket(name: &str) -> Option<std::net::TcpListener> {
    use std::os::fd::FromRawFd;

    // `launch_activate_socket` is the supported way to collect a socket a
    // LaunchAgent declared. It is not in the `libc` crate, so it is declared
    // here; the signature is from `<launch.h>`.
    unsafe extern "C" {
        fn launch_activate_socket(
            name: *const std::ffi::c_char,
            fds: *mut *mut std::ffi::c_int,
            cnt: *mut usize,
        ) -> std::ffi::c_int;
    }

    let name = std::ffi::CString::new(name).ok()?;
    let mut fds: *mut std::ffi::c_int = std::ptr::null_mut();
    let mut count: usize = 0;
    // SAFETY: `name` is a valid NUL-terminated string that outlives the call;
    // `fds` and `count` are out-parameters the API writes. A non-zero return
    // means nothing was handed over and neither was touched.
    let rc = unsafe { launch_activate_socket(name.as_ptr(), &mut fds, &mut count) };
    if rc != 0 || fds.is_null() || count == 0 {
        // Not socket-activated (spawned directly, or no `Sockets` entry). Not
        // an error: binding ourselves is the ordinary path.
        return None;
    }
    // SAFETY: on success the API hands back an array of `count` descriptors
    // allocated with `malloc`, which the caller owns and must free.
    let first = unsafe { *fds };
    unsafe { libc::free(fds as *mut libc::c_void) };
    if first < 0 {
        return None;
    }
    // SAFETY: `first` is a listening socket descriptor launchd transferred to
    // this process, and nothing else holds it.
    Some(unsafe { std::net::TcpListener::from_raw_fd(first) })
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn activated_socket(_name: &str) -> Option<std::net::TcpListener> {
    None
}

/// What `serve` shares with the relay listener, and the timings the tests
/// shorten.
struct Serving {
    /// Connections accepted on either listener and not yet finished.
    in_flight: proxy::InFlight,
    /// Turned true once this forwarder is no longer wanted. Both listeners let
    /// go of their ports on it; see [`DRAIN_LIMIT`].
    retire: Arc<tokio::sync::watch::Sender<bool>>,
    reclaim: proxy::Reclaim,
    engine_poll: Duration,
    engine_poll_while_up: Duration,
    drain_limit: Duration,
    sigterm_drain_limit: Duration,
    /// Also retire on SIGTERM. Only the real process asks for it: a handler
    /// installed from a test would swallow the signal meant for the test
    /// runner.
    sigterm: bool,
    /// Stands in for SIGTERM, for the tests.
    terminate: Arc<tokio::sync::Notify>,
    /// The marker to watch, when not the real one. Tests point it at a file
    /// they control, so they neither depend on nor disturb a running app.
    marker: Option<std::path::PathBuf>,
}

impl Serving {
    fn new() -> Self {
        let retire = Arc::new(tokio::sync::watch::Sender::new(false));
        Serving {
            in_flight: proxy::InFlight::default(),
            reclaim: proxy::Reclaim::new(proxy::RECLAIM_IDLE, retire.clone()),
            retire,
            engine_poll: MARKER_POLL,
            engine_poll_while_up: ENGINE_POLL_WHILE_UP,
            drain_limit: DRAIN_LIMIT,
            sigterm_drain_limit: SIGTERM_DRAIN_LIMIT,
            sigterm: false,
            terminate: Arc::new(tokio::sync::Notify::new()),
            marker: None,
        }
    }
}

/// [`serve_with`] with the defaults, for the tests that need nothing else.
#[cfg(test)]
async fn serve(
    listener: TcpListener,
    engine_port: proxy::EngineLookup,
    own_port: u16,
    token: Arc<str>,
) -> Result<()> {
    serve_with(listener, engine_port, own_port, token, Serving::new()).await
}

/// Accept until the marker file goes away, or SIGTERM where `serving` asks.
///
/// Once the forwarder is no longer wanted it stops accepting and gives up its
/// port at once, then keeps serving what it already accepted until that
/// finishes or [`Serving::drain_limit`] runs out.
async fn serve_with(
    listener: TcpListener,
    engine_port: proxy::EngineLookup,
    own_port: u16,
    token: Arc<str>,
    serving: Serving,
) -> Result<()> {
    // Built once, outside the loop, and polled in place. A future created
    // inside `select!` is dropped and rebuilt on every iteration, so each
    // accepted connection restarted the marker poll from zero and a forwarder
    // seeing traffic more often than MARKER_POLL would never notice it was no
    // longer wanted - it would run until logout.
    let unwanted = tokio::spawn(unwanted(serving.marker.clone()));
    tokio::pin!(unwanted);
    // Its own task, listened to through the drain as well: the app removes the
    // marker and boots the agent out back to back, so SIGTERM often lands
    // after a drain the marker already started.
    let terminate = serving.terminate.clone();
    let sigterm = serving.sigterm;
    let terminated = tokio::spawn(async move {
        tokio::select! {
            () = terminated(sigterm) => {}
            () = terminate.notified() => {}
        }
    });
    tokio::pin!(terminated);

    let watcher = tokio::spawn(watch_engine(
        serving.reclaim.engine_up.clone(),
        engine_port.clone(),
        own_port,
        serving.engine_poll,
        serving.engine_poll_while_up,
    ));

    // A cap on connections being served at once. Each one costs a task and two
    // descriptors, and without a ceiling a single local process can open
    // sockets until this one runs out of them.
    let slots = Arc::new(tokio::sync::Semaphore::new(512));

    let mut signalled = false;
    loop {
        tokio::select! {
            _ = &mut unwanted => break,
            _ = &mut terminated => {
                signalled = true;
                break;
            }
            accepted = listener.accept() => {
                let (client, _) = match accepted {
                    Ok(pair) => pair,
                    // Pause rather than spin: a permanently failed listener
                    // would otherwise burn a core.
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                };
                let Ok(slot) = slots.clone().try_acquire_owned() else {
                    // At capacity. Dropping the connection is the honest
                    // answer: queueing it would just move the exhaustion.
                    continue;
                };
                let serving_one = serving.in_flight.enter();
                let engine_port = engine_port.clone();
                let token = token.clone();
                let reclaim = serving.reclaim.clone();
                tokio::spawn(async move {
                    let _slot = slot;
                    let _serving = serving_one;
                    let _ = proxy::handle(client, engine_port, own_port, token, reclaim).await;
                });
            }
        }
    }

    // Both ports go first, so whatever replaces this forwarder can bind them
    // while the connections below finish. The engine watcher stays: handing
    // direct tunnels back to a live engine is also what ends them sooner.
    drop(listener);
    serving.retire.send_replace(true);
    let started = tokio::time::Instant::now();
    let mut deadline = started
        + if signalled {
            serving.sigterm_drain_limit
        } else {
            serving.drain_limit
        };
    while serving.in_flight.count() > 0 && tokio::time::Instant::now() < deadline {
        tokio::select! {
            () = tokio::time::sleep(Duration::from_millis(100)) => {}
            _ = &mut terminated, if !signalled => {
                signalled = true;
                deadline = deadline.min(tokio::time::Instant::now() + serving.sigterm_drain_limit);
            }
        }
    }
    watcher.abort();
    Ok(())
}

/// Resolve once this forwarder's marker is gone.
async fn unwanted(marker: Option<std::path::PathBuf>) {
    let marker = marker.or_else(|| marker_path().ok());
    loop {
        tokio::time::sleep(MARKER_POLL).await;
        if marker.as_ref().is_some_and(|p| !p.exists()) {
            return;
        }
    }
}

/// Resolve on SIGTERM, when `sigterm` asks for it; never otherwise.
///
/// SIGTERM is how launchd stops a job it boots out, which is how the app
/// replaces or stops a socket-activated forwarder. launchd owns that listening
/// socket, so the port is handed on only by the bootout itself, and launchd
/// kills the job once its exit timeout runs out - 20s unless the plist says
/// otherwise - while `launchctl bootout` waits. So this drain is capped at
/// [`SIGTERM_DRAIN_LIMIT`]: long enough for a short response to finish, short
/// enough that the process exits itself instead of being killed, and that the
/// app thread waiting on the bootout is not held for the whole timeout.
#[cfg(unix)]
async fn terminated(sigterm: bool) {
    use tokio::signal::unix::{signal, SignalKind};
    // Checked before installing: creating the listener is itself what stops
    // SIGTERM ending the process, so it must not happen unasked.
    if !sigterm {
        return std::future::pending().await;
    }
    match signal(SignalKind::terminate()) {
        Ok(mut term) => {
            term.recv().await;
        }
        Err(_) => std::future::pending().await,
    }
}

#[cfg(not(unix))]
async fn terminated(_sigterm: bool) {
    std::future::pending().await
}

/// Keep `engine_up` saying whether the engine accepts connections, for the
/// direct tunnels waiting to be handed back to it.
///
/// Dials only while a direct tunnel is subscribed. With none open there is
/// nothing to reclaim, and a forwarder that knocked on the engine every couple
/// of seconds for the whole session would be noise in its accept loop for no
/// one's benefit. The value drops back to `false` while unwatched, so a tunnel
/// that subscribes later never acts on an answer from before it existed. Once
/// the engine has answered it dials only every `while_up`.
async fn watch_engine(
    engine_up: Arc<tokio::sync::watch::Sender<bool>>,
    engine_port: proxy::EngineLookup,
    own_port: u16,
    every: Duration,
    while_up: Duration,
) {
    loop {
        tokio::time::sleep(if *engine_up.borrow() { while_up } else { every }).await;
        let up = if engine_up.receiver_count() == 0 {
            false
        } else {
            match engine_port().filter(|port| *port != own_port) {
                Some(port) => matches!(
                    tokio::time::timeout(
                        proxy::ENGINE_CONNECT_TIMEOUT,
                        tokio::net::TcpStream::connect(("127.0.0.1", port)),
                    )
                    .await,
                    Ok(Ok(_))
                ),
                None => false,
            }
        };
        engine_up.send_if_modified(|seen| std::mem::replace(seen, up) != up);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    const TOKEN: &str = "test-token-abc";

    async fn start_forwarder(engine_port: Option<u16>) -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            serve(
                listener,
                Arc::new(move || engine_port),
                port,
                Arc::from(TOKEN),
            )
            .await
        });
        port
    }

    /// A loopback port nothing is listening on: bound to reserve it, then
    /// dropped. Modelling "the engine went away" by picking a number would be a
    /// test that passes for the wrong reason if something else is there.
    fn dead_port() -> u16 {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        l.local_addr().unwrap().port()
    }

    /// Accept one connection, hand back everything sent before the blank line
    /// plus `trailing` bytes after it, then write `reply` and hold the socket
    /// briefly.
    fn one_shot_with(
        reply: &'static [u8],
        trailing: usize,
    ) -> (u16, tokio::sync::oneshot::Receiver<Vec<u8>>) {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut sock, _) = listener.accept().unwrap();
            let mut seen = Vec::new();
            let mut byte = [0u8; 1];
            while !seen.ends_with(b"\r\n\r\n") {
                if sock.read(&mut byte).unwrap_or(0) == 0 {
                    break;
                }
                seen.push(byte[0]);
            }
            // A deadline, because "nothing more arrives" is a result these
            // tests assert rather than an error: without it, the test that
            // proves a pipelined request is *not* relayed would block forever
            // waiting for the bytes whose absence is the point.
            let _ = sock.set_read_timeout(Some(Duration::from_millis(300)));
            for _ in 0..trailing {
                match sock.read(&mut byte) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => seen.push(byte[0]),
                }
            }
            let _ = sock.write_all(reply);
            let _ = sock.flush();
            let _ = tx.send(seen);
            std::thread::sleep(Duration::from_millis(200));
        });
        (port, rx)
    }

    fn one_shot(reply: &'static [u8]) -> (u16, tokio::sync::oneshot::Receiver<Vec<u8>>) {
        one_shot_with(reply, 0)
    }

    /// Accept one connection and close it immediately, modelling an engine that
    /// is torn down between accepting and answering.
    fn accepts_then_dies() -> u16 {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((sock, _)) = listener.accept() {
                drop(sock);
            }
            std::thread::sleep(Duration::from_millis(500));
        });
        port
    }

    async fn read_some(stream: &mut TcpStream) -> String {
        let mut buf = vec![0u8; 512];
        let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf))
            .await
            .expect("upstream should answer")
            .expect("read");
        String::from_utf8_lossy(&buf[..n]).to_string()
    }

    /// While the engine is up this is a transparent extra hop. The
    /// `Proxy-Authorization` selector especially has to arrive verbatim: it is
    /// how the engine knows to force Claude Code's route, and swallowing it
    /// would silently unroute Claude Code while every status still read
    /// Connected.
    #[tokio::test]
    async fn hands_the_connection_to_the_engine_verbatim() {
        let (engine_port, engine_saw) = one_shot(b"HTTP/1.1 200 OK\r\n\r\n");
        let port = start_forwarder(Some(engine_port)).await;

        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client
            .write_all(
                b"CONNECT api.anthropic.com:443 HTTP/1.1\r\n\
                  Host: api.anthropic.com:443\r\n\
                  Proxy-Authorization: Basic Z2F0ZS1jbGF1ZGUtY29kZTpyb3V0ZQ==\r\n\r\n",
            )
            .await
            .unwrap();
        assert!(read_some(&mut client).await.starts_with("HTTP/1.1 200"));

        let head = String::from_utf8(engine_saw.await.unwrap()).unwrap();
        assert!(head.starts_with("CONNECT api.anthropic.com:443"), "{head}");
        assert!(
            head.contains("Proxy-Authorization: Basic Z2F0ZS1jbGF1ZGUtY29kZTpyb3V0ZQ=="),
            "the route selector must reach the engine untouched: {head}"
        );
    }

    /// The whole point. With no engine to hand the connection to, the client
    /// still reaches its provider instead of getting the connection refused
    /// that made turning routing off break every already-running tool.
    #[tokio::test]
    async fn goes_direct_when_the_engine_is_gone() {
        let (origin_port, origin_saw) = one_shot(b"pong");
        let port = start_forwarder(Some(dead_port())).await;

        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client
            .write_all(format!("CONNECT 127.0.0.1:{origin_port} HTTP/1.1\r\n\r\n").as_bytes())
            .await
            .unwrap();
        assert!(read_some(&mut client).await.starts_with("HTTP/1.1 200"));

        client.write_all(b"ping\r\n\r\n").await.unwrap();
        let seen = String::from_utf8(origin_saw.await.unwrap()).unwrap();
        assert!(seen.starts_with("ping"), "{seen}");
        assert!(
            !seen.contains("CONNECT"),
            "the CONNECT terminates here; the origin must never see it: {seen}"
        );
    }

    /// An engine that accepts and then dies is a real window - a disable stops
    /// it while connections are being accepted - and the client must not be
    /// handed the resulting zero-byte EOF.
    #[tokio::test]
    async fn falls_back_when_the_engine_dies_after_accepting() {
        let (origin_port, origin_saw) = one_shot(b"pong");
        let port = start_forwarder(Some(accepts_then_dies())).await;

        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client
            .write_all(format!("CONNECT 127.0.0.1:{origin_port} HTTP/1.1\r\n\r\n").as_bytes())
            .await
            .unwrap();
        assert!(
            read_some(&mut client).await.starts_with("HTTP/1.1 200"),
            "the fallback must be invisible to the client"
        );
        client.write_all(b"ping\r\n\r\n").await.unwrap();
        assert!(String::from_utf8(origin_saw.await.unwrap())
            .unwrap()
            .starts_with("ping"));
    }

    /// Going direct must not carry the credential that addressed *us* to a
    /// third party.
    #[tokio::test]
    async fn never_carries_the_proxy_credential_to_an_origin() {
        let (origin_port, origin_saw) = one_shot(b"HTTP/1.1 204 No Content\r\n\r\n");
        let port = start_forwarder(Some(dead_port())).await;

        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client
            .write_all(
                format!(
                    "GET http://127.0.0.1:{origin_port}/v1/x HTTP/1.1\r\n\
                     Host: 127.0.0.1\r\n\
                     Proxy-Authorization: Basic Z2F0ZQ==\r\n\
                     Accept: */*\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();

        let seen = String::from_utf8(origin_saw.await.unwrap()).unwrap();
        assert!(
            seen.starts_with("GET /v1/x HTTP/1.1"),
            "origin-form: {seen}"
        );
        assert!(
            !seen.to_lowercase().contains("proxy-authorization"),
            "the proxy credential must not reach the origin: {seen}"
        );
        assert!(seen.contains("Accept: */*"), "{seen}");
    }

    /// A request body arriving in the same packet as the head must reach the
    /// origin. It is read past the head boundary, so it only survives if the
    /// leftover bytes are carried forward.
    #[tokio::test]
    async fn a_body_sent_with_the_head_is_not_lost() {
        let (origin_port, origin_saw) =
            one_shot_with(b"HTTP/1.1 204 No Content\r\n\r\n", "body-here".len());
        let port = start_forwarder(Some(dead_port())).await;

        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client
            .write_all(
                format!(
                    "POST http://127.0.0.1:{origin_port}/v1/x HTTP/1.1\r\n\
                     Host: 127.0.0.1\r\n\
                     Content-Length: 9\r\n\r\n\
                     body-here"
                )
                .as_bytes(),
            )
            .await
            .unwrap();

        let seen = String::from_utf8(origin_saw.await.unwrap()).unwrap();
        assert!(seen.starts_with("POST /v1/x HTTP/1.1"), "{seen}");
        // The body arrived in the same read as the head, so it only survives
        // if the bytes past the head boundary are carried forward.
        assert!(
            seen.ends_with("body-here"),
            "the body must not be dropped: {seen}"
        );
    }

    /// The health path is how the app proves the listener on the persisted port
    /// is the forwarder it spawned. The proof has to depend on the token: a
    /// bare `204` is something any squatter can say, and an earlier design that
    /// sent the token on the request also handed it to whoever was listening.
    #[tokio::test]
    async fn the_health_path_answers_with_proof_of_the_token() {
        let port = start_forwarder(Some(dead_port())).await;

        let challenge = "0123456789abcdef";
        let mut good = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        good.write_all(
            format!(
                "GET {} HTTP/1.1\r\nHost: 127.0.0.1\r\n{}: {challenge}\r\n\r\n",
                proxy::HEALTH_PATH,
                proxy::CHALLENGE_HEADER,
            )
            .as_bytes(),
        )
        .await
        .unwrap();
        let reply = read_some(&mut good).await;
        assert!(reply.starts_with("HTTP/1.1 204"), "{reply}");
        let expected = gate_connect_paths::forwarder_proof(TOKEN, proxy::HEALTH_PATH, challenge);
        assert!(
            reply.to_lowercase().contains(&expected),
            "the reply must carry proof of the token: {reply}"
        );
        // And the token itself is never on the wire in either direction.
        assert!(!reply.contains(TOKEN), "{reply}");

        // No challenge, no answer - and it looks like any other unroutable
        // request, so probing cannot tell a forwarder from anything else.
        let mut bare = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        bare.write_all(
            format!("GET {} HTTP/1.1\r\nHost: x\r\n\r\n", proxy::HEALTH_PATH).as_bytes(),
        )
        .await
        .unwrap();
        assert!(read_some(&mut bare).await.starts_with("HTTP/1.1 400"));
    }

    /// A client may pipeline a second absolute-form request, for a different
    /// host, behind the first. Splicing would hand it to *this* origin verbatim
    /// - wrong destination, and carrying the `Proxy-Authorization` that
    /// `rewrite_direct` strips from request one. `Connection: close` asks the
    /// origin to hang up but does not stop the client having already sent it.
    #[tokio::test]
    async fn a_pipelined_second_request_never_reaches_the_first_origin() {
        let (origin_port, origin_saw) = one_shot_with(b"HTTP/1.1 204 No Content\r\n\r\n", 256);
        let port = start_forwarder(Some(dead_port())).await;

        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client
            .write_all(
                format!(
                    "GET http://127.0.0.1:{origin_port}/first HTTP/1.1\r\n\
                     Host: 127.0.0.1\r\n\
                     Content-Length: 0\r\n\r\n\
                     GET http://evil.example/second HTTP/1.1\r\n\
                     Host: evil.example\r\n\
                     Proxy-Authorization: Basic Z2F0ZQ==\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();

        let seen = String::from_utf8(origin_saw.await.unwrap()).unwrap();
        assert!(seen.starts_with("GET /first HTTP/1.1"), "{seen}");
        assert!(
            !seen.contains("evil.example"),
            "the second request must not be relayed to the first origin: {seen}"
        );
        assert!(
            !seen.to_lowercase().contains("proxy-authorization"),
            "and its proxy credential must not leak with it: {seen}"
        );
    }

    /// A request we cannot route gets a refusal, not silence.
    #[tokio::test]
    async fn refuses_a_request_it_cannot_route() {
        let port = start_forwarder(Some(dead_port())).await;
        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client
            .write_all(b"GET /v1/messages HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        assert!(read_some(&mut client).await.starts_with("HTTP/1.1 400"));
    }

    /// Dialing our own port would recurse until something ran out. The port
    /// files are written by different processes, so a crossed pair is a
    /// configuration accident rather than an impossibility.
    #[tokio::test]
    async fn never_dials_itself() {
        let (origin_port, origin_saw) = one_shot(b"pong");
        // The engine lookup returns the forwarder's own port.
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            serve(
                listener,
                Arc::new(move || Some(port)),
                port,
                Arc::from(TOKEN),
            )
            .await
        });

        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client
            .write_all(format!("CONNECT 127.0.0.1:{origin_port} HTTP/1.1\r\n\r\n").as_bytes())
            .await
            .unwrap();
        assert!(
            read_some(&mut client).await.starts_with("HTTP/1.1 200"),
            "it must go direct rather than dial itself"
        );
        client.write_all(b"ping\r\n\r\n").await.unwrap();
        assert!(String::from_utf8(origin_saw.await.unwrap())
            .unwrap()
            .starts_with("ping"));
    }

    /// A forwarder whose engine can be brought back mid-test, with reclaim
    /// timings short enough to observe. It watches a marker of its own, which
    /// exists until the test removes it.
    async fn start_reclaiming_forwarder(
        engine: Arc<std::sync::atomic::AtomicU16>,
        idle: Duration,
    ) -> u16 {
        let (port, _, _, _) = start_with(engine, idle, Duration::from_secs(30)).await;
        port
    }

    /// A marker file only this test knows about, present until removed.
    fn test_marker() -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "gate-forwarder-test-marker-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        std::fs::write(&path, b"").unwrap();
        path
    }

    /// Start a forwarder on its own marker, returning its port, the marker and
    /// the task, which ends when `serve_with` returns.
    async fn start_with(
        engine: Arc<std::sync::atomic::AtomicU16>,
        idle: Duration,
        drain_limit: Duration,
    ) -> (
        u16,
        std::path::PathBuf,
        tokio::task::JoinHandle<Result<()>>,
        Arc<tokio::sync::Notify>,
    ) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let marker = test_marker();
        let mut serving = Serving::new();
        serving.reclaim.idle = idle;
        serving.engine_poll = Duration::from_millis(50);
        serving.engine_poll_while_up = Duration::from_millis(100);
        serving.drain_limit = drain_limit;
        serving.marker = Some(marker.clone());
        serving.sigterm_drain_limit = Duration::from_millis(300);
        let terminate = serving.terminate.clone();
        let task = tokio::spawn(async move {
            serve_with(
                listener,
                Arc::new(move || Some(engine.load(std::sync::atomic::Ordering::SeqCst))),
                port,
                Arc::from(TOKEN),
                serving,
            )
            .await
        });
        (port, marker, task, terminate)
    }

    /// Retiring is routine now - it is how an update replaces the forwarder -
    /// so it must not cut what is in flight. The port goes at once, so the
    /// replacement can bind it; the tunnel already open keeps working; and the
    /// forwarder exits only when that tunnel is done.
    #[tokio::test]
    async fn a_retiring_forwarder_releases_its_port_and_finishes_what_it_has() {
        let engine = Arc::new(std::sync::atomic::AtomicU16::new(dead_port()));
        let (port, marker, task, _) =
            start_with(engine, Duration::from_secs(60), Duration::from_secs(30)).await;
        let mut client = open_direct_tunnel(port, echo_origin()).await;

        std::fs::remove_file(&marker).unwrap();
        let deadline = tokio::time::Instant::now() + MARKER_POLL * 3;
        loop {
            if TcpStream::connect(("127.0.0.1", port)).await.is_err() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "a retiring forwarder must give its port up"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // The replacement can take it.
        drop(std::net::TcpListener::bind(("127.0.0.1", port)).expect("the port is free"));

        client.write_all(b"still here").await.unwrap();
        assert_eq!(read_some(&mut client).await, "still here");
        assert!(!task.is_finished(), "it must wait for the open tunnel");

        drop(client);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("it exits once the last connection is done")
            .unwrap()
            .unwrap();
    }

    /// The wait is bounded: a tunnel a client keeps open forever must not keep
    /// a retired forwarder around forever.
    #[tokio::test]
    async fn a_retiring_forwarder_stops_waiting_at_the_limit() {
        let engine = Arc::new(std::sync::atomic::AtomicU16::new(dead_port()));
        let (port, marker, task, _) =
            start_with(engine, Duration::from_secs(60), Duration::from_millis(300)).await;
        let _client = open_direct_tunnel(port, echo_origin()).await;

        std::fs::remove_file(&marker).unwrap();
        tokio::time::timeout(MARKER_POLL * 3, task)
            .await
            .expect("it exits at the limit with the tunnel still open")
            .unwrap()
            .unwrap();
    }

    /// An origin that echoes every byte back and holds the connection until the
    /// other side closes it, like a provider holding a keep-alive connection.
    fn echo_origin() -> u16 {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let Ok((mut sock, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 256];
            loop {
                match sock.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        if sock.write_all(&buf[..n]).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        port
    }

    /// Open a direct tunnel to `origin` through the forwarder and prove it
    /// carries bytes.
    async fn open_direct_tunnel(forwarder: u16, origin: u16) -> TcpStream {
        let mut client = TcpStream::connect(("127.0.0.1", forwarder)).await.unwrap();
        client
            .write_all(format!("CONNECT 127.0.0.1:{origin} HTTP/1.1\r\n\r\n").as_bytes())
            .await
            .unwrap();
        assert!(read_some(&mut client).await.starts_with("HTTP/1.1 200"));
        client.write_all(b"ping").await.unwrap();
        assert_eq!(read_some(&mut client).await, "ping");
        client
    }

    /// Whether the forwarder closed `client` within `within`.
    async fn closed_within(client: &mut TcpStream, within: Duration) -> bool {
        let mut buf = [0u8; 64];
        matches!(
            tokio::time::timeout(within, client.read(&mut buf)).await,
            Ok(Ok(0) | Err(_))
        )
    }

    /// The reported bug. Claude Code opened its connection while the app was
    /// gone, and its pool kept reusing it after the app came back, so it stayed
    /// off Gate until it was restarted. Once the engine answers again, a quiet
    /// direct tunnel has to be closed so the client's next connection reaches
    /// the engine.
    #[tokio::test]
    async fn a_direct_tunnel_is_handed_back_once_the_engine_returns() {
        let engine = Arc::new(std::sync::atomic::AtomicU16::new(dead_port()));
        let port = start_reclaiming_forwarder(engine.clone(), Duration::from_millis(200)).await;
        let mut client = open_direct_tunnel(port, echo_origin()).await;

        // With no engine, a quiet tunnel is left alone: closing it would only
        // send the client back to the same direct path.
        assert!(
            !closed_within(&mut client, Duration::from_millis(500)).await,
            "a direct tunnel must stay open while the engine is still gone"
        );

        let back = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        engine.store(
            back.local_addr().unwrap().port(),
            std::sync::atomic::Ordering::SeqCst,
        );
        assert!(
            closed_within(&mut client, Duration::from_secs(2)).await,
            "the tunnel must close once the engine is back and it is idle"
        );
    }

    /// Handing a tunnel back must never cut a response in flight: a streaming
    /// answer that fails halfway is worse than one more request sent direct.
    #[tokio::test]
    async fn a_busy_direct_tunnel_is_not_cut_when_the_engine_returns() {
        let engine = Arc::new(std::sync::atomic::AtomicU16::new(dead_port()));
        let port = start_reclaiming_forwarder(engine.clone(), Duration::from_secs(1)).await;
        let mut client = open_direct_tunnel(port, echo_origin()).await;

        let back = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        engine.store(
            back.local_addr().unwrap().port(),
            std::sync::atomic::Ordering::SeqCst,
        );
        // Keep traffic moving for several idle windows. Every exchange has to
        // make it across.
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(80)).await;
            client.write_all(b"tick").await.unwrap();
            assert_eq!(read_some(&mut client).await, "tick");
        }
        assert!(
            closed_within(&mut client, Duration::from_secs(3)).await,
            "and once it goes quiet it is handed back"
        );
    }

    /// Accept every connection and close it at once: an engine that answers a
    /// connect and then fails whatever it is given.
    fn engine_that_drops_everything() -> u16 {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for sock in listener.incoming() {
                drop(sock);
            }
        });
        port
    }

    /// A tunnel that went direct because the engine took it and failed it is
    /// not handed back. The watcher sees that engine as up, since it accepts,
    /// so handing back would close the tunnel at every quiet moment and send
    /// the client straight into the same failure, and the wait before it goes
    /// direct again.
    #[tokio::test]
    async fn a_tunnel_the_engine_failed_is_not_handed_back_to_it() {
        let engine = Arc::new(std::sync::atomic::AtomicU16::new(
            engine_that_drops_everything(),
        ));
        let port = start_reclaiming_forwarder(engine, Duration::from_millis(200)).await;
        let mut client = open_direct_tunnel(port, echo_origin()).await;

        assert!(
            !closed_within(&mut client, Duration::from_secs(1)).await,
            "a tunnel the engine failed must stay open while that engine answers connects"
        );
    }

    /// An engine that answers a CONNECT and then echoes, holding the
    /// connection until the other side closes it.
    fn engine_tunnel() -> u16 {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let Ok((mut sock, _)) = listener.accept() else {
                return;
            };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                if sock.read(&mut byte).unwrap_or(0) == 0 {
                    return;
                }
                head.push(byte[0]);
            }
            if sock.write_all(b"HTTP/1.1 200 OK\r\n\r\n").is_err() {
                return;
            }
            let mut buf = [0u8; 256];
            loop {
                match sock.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        if sock.write_all(&buf[..n]).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        port
    }

    /// A retiring forwarder closes quiet connections through the engine too.
    /// Reclaim never ends those - the engine is up - so without this a pooled
    /// engine connection holds a retired forwarder for the whole drain limit
    /// and is then cut wherever it happens to be.
    #[tokio::test]
    async fn a_retiring_forwarder_closes_a_quiet_engine_connection() {
        let engine = Arc::new(std::sync::atomic::AtomicU16::new(engine_tunnel()));
        let (port, marker, task, _) =
            start_with(engine, Duration::from_millis(300), Duration::from_secs(30)).await;
        // Through the engine this time: it answers the CONNECT and echoes.
        let mut client = open_direct_tunnel(port, dead_port()).await;

        std::fs::remove_file(&marker).unwrap();
        tokio::time::timeout(MARKER_POLL * 3, task)
            .await
            .expect("the drain ends once the engine connection goes quiet")
            .unwrap()
            .unwrap();
        assert!(closed_within(&mut client, Duration::from_secs(1)).await);
    }

    /// SIGTERM is launchd booting the job out, and launchd kills it at its
    /// exit timeout while `launchctl bootout` holds the app thread that asked.
    /// So a drain the signal starts, or one it lands in, has to be short: the
    /// long one would outlast the timeout and be killed anyway.
    #[tokio::test]
    async fn sigterm_cuts_a_drain_short() {
        let engine = Arc::new(std::sync::atomic::AtomicU16::new(dead_port()));
        let (port, _marker, task, terminate) =
            start_with(engine, Duration::from_secs(60), Duration::from_secs(60)).await;
        let _client = open_direct_tunnel(port, echo_origin()).await;

        terminate.notify_one();
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .expect("a SIGTERM drain ends at its own short limit")
            .unwrap()
            .unwrap();
    }

    /// The app removes the marker and then boots the agent out, so the signal
    /// usually arrives with the long drain already under way.
    #[tokio::test]
    async fn sigterm_during_a_marker_drain_shortens_it() {
        let engine = Arc::new(std::sync::atomic::AtomicU16::new(dead_port()));
        let (port, marker, task, terminate) =
            start_with(engine, Duration::from_secs(60), Duration::from_secs(60)).await;
        let _client = open_direct_tunnel(port, echo_origin()).await;

        std::fs::remove_file(&marker).unwrap();
        // Past the marker poll, so the long drain has begun.
        tokio::time::sleep(MARKER_POLL + Duration::from_millis(500)).await;
        assert!(!task.is_finished(), "the marker drain waits for the tunnel");

        terminate.notify_one();
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .expect("the signal shortens the drain already running")
            .unwrap()
            .unwrap();
    }
}
