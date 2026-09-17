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
//! fail-open behaviour.
//!
//! # Why a separate binary
//!
//! Because the failure it fixes includes the GUI going away, so it cannot live
//! in the GUI process. It is a separate *binary* rather than the app re-invoked
//! with a flag (which is how the Linux helper daemon works) for three reasons:
//! a running process holds a file lock on its own executable, and on Windows
//! that would block the updater from replacing the app; a distinct name in
//! Activity Monitor and Task Manager answers "why are there two Gate Connects"
//! honestly; and linking none of the app's machinery is what keeps a
//! permanently-running process small and free of anything worth stealing.
//!
//! It holds no credential, terminates no TLS, mints no certificate and makes no
//! routing decision. Its security posture is recorded in
//! `docs/security-notes-loopback.md`.

mod proxy;

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

/// Ports the app may already have remembered for its own listeners, so a fresh
/// forwarder bind does not take one out from under them.
const APP_PORT_NAMES: [&str; 3] = [ENGINE_PORT_NAME, "pac-port", "relay-port"];

/// How often the forwarder checks whether it is still wanted.
const MARKER_POLL: Duration = Duration::from_secs(2);

fn proxy_file(name: &str) -> Result<std::path::PathBuf> {
    Ok(gate_connect_paths::proxy_dir()?.join(name))
}

/// Marker file whose presence means "the forwarder should be running".
///
/// A file rather than a signal or a control socket: it is the same on all three
/// platforms and cannot mis-target a recycled PID. Stopping is a delete, and
/// the forwarder notices within [`MARKER_POLL`].
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
        serve(
            listener,
            Arc::new(|| gate_connect_paths::load_port(ENGINE_PORT_NAME)),
            port,
            token,
        )
        .await
    })
}

/// Take the listening socket, from launchd where it offers one and by binding
/// otherwise.
fn bind() -> Result<std::net::TcpListener> {
    if let Some(listener) = activated_socket() {
        return Ok(listener);
    }
    let skip: Vec<u16> = APP_PORT_NAMES
        .iter()
        .filter_map(|n| gate_connect_paths::load_port(n))
        .collect();
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

/// The listening socket launchd is holding for us, if we were socket-activated.
///
/// With a `Sockets` entry in the LaunchAgent plist, launchd binds and listens on
/// the port itself at login and starts this process on the first connection,
/// handing over the already-listening descriptor. Two things follow, and both
/// are the reason to prefer it: the address answers from login onward even with
/// no Gate process in existence, so there is no window where a tool is
/// stranded; and nothing can squat the port, because launchd took it before any
/// other process could.
#[cfg(target_os = "macos")]
fn activated_socket() -> Option<std::net::TcpListener> {
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

    let name = std::ffi::CString::new("Forwarder").ok()?;
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
fn activated_socket() -> Option<std::net::TcpListener> {
    None
}

/// Accept until the marker file goes away.
async fn serve(
    listener: TcpListener,
    engine_port: proxy::EngineLookup,
    own_port: u16,
    token: Arc<str>,
) -> Result<()> {
    // Built once, outside the loop, and polled in place. A future created
    // inside `select!` is dropped and rebuilt on every iteration, so each
    // accepted connection restarted the marker poll from zero and a forwarder
    // seeing traffic more often than MARKER_POLL would never notice it was no
    // longer wanted - it would run until logout.
    let unwanted = tokio::spawn(async {
        loop {
            tokio::time::sleep(MARKER_POLL).await;
            let gone = marker_path().map(|p| !p.exists()).unwrap_or(false);
            if gone {
                return;
            }
        }
    });
    tokio::pin!(unwanted);

    // A cap on connections being served at once. Each one costs a task and two
    // descriptors, and without a ceiling a single local process can open
    // sockets until this one runs out of them.
    let slots = Arc::new(tokio::sync::Semaphore::new(512));

    loop {
        tokio::select! {
            _ = &mut unwanted => return Ok(()),
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
                let engine_port = engine_port.clone();
                let token = token.clone();
                tokio::spawn(async move {
                    let _slot = slot;
                    let _ = proxy::handle(client, engine_port, own_port, token).await;
                });
            }
        }
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
        let expected = gate_connect_paths::forwarder_proof(TOKEN, challenge);
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
}
