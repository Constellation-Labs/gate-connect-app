//! The environment forwarder: a tiny always-there proxy that the machine-wide
//! variables point at, instead of pointing them at the MITM engine.
//!
//! # Why this exists
//!
//! The two channels that carry Gate's proxy fail differently, and only one of
//! them fails safely.
//!
//! A PAC left naming a dead port **fails open**: the fetch fails and the client
//! goes DIRECT. The exported `HTTPS_PROXY` **fails closed**: the client dials a
//! port that refuses and the request dies. That asymmetry is the whole of the
//! routing-off bug. `launchctl unsetenv` (and the Windows registry write beside
//! it) only changes what processes started *afterwards* inherit, so every
//! shell, editor and CLI already running keeps the variable for its whole life
//! - and the moment the engine's port goes away they cannot reach any provider
//! at all. Not just the tools Gate manages: the export is machine-wide, so it
//! catches curl, git, npm, and software Gate does not claim to touch.
//!
//! Pointing the variables at *this* instead gives the env channel the PAC's
//! fail-open behaviour. When the engine is up we hand the connection to it and
//! nothing about routing changes. When it is not - routing switched off, the
//! app quit, the engine crashed - we connect the client straight to where it
//! was going, which is the path it would have taken with Gate not installed.
//!
//! # Why it is a separate process
//!
//! Because the failure it fixes includes the GUI going away. Linux has never
//! had this bug precisely because its engine is a daemon that outlives the GUI
//! (`helper.rs`); on macOS and Windows the engine lives inside whichever
//! process enabled it. This is the same trick at a fraction of the size:
//! `<current-exe> --env-forwarder`, detached, so there is no second binary to
//! package, sign or locate - exactly how the Linux helper is spawned.
//!
//! # What it deliberately does not do
//!
//! It holds no credential, reads no traffic and makes no routing decision. It
//! does not need the catalog, the PAC, or any notion of which hosts are routed:
//! the engine already blind-tunnels whatever it does not route, so "hand
//! everything to the engine when it answers" produces the same result as
//! evaluating the rules, with nothing to keep in sync. That matters especially
//! because the PAC is served *by* the engine, so it disappears at exactly the
//! moment the fallback is needed.
//!
//! Its security posture is in `docs/security-notes-loopback.md`: it is a
//! forward proxy that cannot spend the user's Gate credential, because it never
//! has one.

use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The argv flag that turns the shipped binary into the forwarder. Matches the
/// shape of Linux's `--proxy-helper` so there is one convention to learn.
pub const FORWARDER_FLAG: &str = "--env-forwarder";

/// Name under which the forwarder's own port is persisted. Stability is the
/// point: the exported variables name this port, and a process that read them
/// at its own launch keeps dialing it for the rest of its life.
const PORT_NAME: &str = "forwarder-port";

/// How long to wait for the engine to accept before deciding it is not there.
///
/// This is a loopback connect, so a refusal comes back in well under a
/// millisecond and the timeout only bounds the pathological case (a port held
/// by something that accepts nothing). It is spent once per connection rather
/// than cached, so a toggle takes effect on the next connection with no state
/// to go stale - and a cached "engine is down" is exactly the state that would
/// keep sending traffic direct after the user turned routing back on.
const ENGINE_PROBE_TIMEOUT: Duration = Duration::from_millis(250);

/// How often the forwarder checks whether it is still wanted.
const MARKER_POLL: Duration = Duration::from_secs(2);

/// The biggest request head we will buffer before giving up on a client.
///
/// Only the head is ever held: once the target is known the connection is
/// spliced, so a large upload costs nothing here. 64 KiB is comfortably above
/// what any real client sends and well under what would make this worth
/// attacking.
const MAX_HEAD: usize = 64 * 1024;

/// Marker file whose presence means "the forwarder should be running".
///
/// A file rather than a signal or a control socket, because it is the same on
/// all three platforms and cannot mis-target a recycled PID: stopping is a
/// delete, and the forwarder notices within [`MARKER_POLL`]. It is also
/// self-healing across a crash - the file outliving the process just means the
/// next enable spawns a replacement.
fn marker_path() -> Result<PathBuf> {
    Ok(crate::env::app_support_dir()?
        .join("proxy")
        .join("forwarder-wanted"))
}

fn marker_present() -> bool {
    marker_path().map(|p| p.exists()).unwrap_or(false)
}

/// The port this forwarder last bound, if it has ever run.
pub(crate) fn persisted_port() -> Option<u16> {
    super::port_persist::load(PORT_NAME).ok().flatten()
}

/// Whether something is accepting on `port`.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
fn answering(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], port)),
        ENGINE_PROBE_TIMEOUT,
    )
    .is_ok()
}

/// How a connection's upstream is chosen. Injectable so the tests can drive
/// both branches without persisting ports or spawning an engine.
type EngineLookup = Arc<dyn Fn() -> Option<u16> + Send + Sync>;

/// The engine's persisted port, as production reads it.
///
/// Read per connection rather than remembered: the engine moves off its
/// preferred port when something else holds it, and a forwarder holding a stale
/// number would send every request to a stranger.
fn persisted_engine_port() -> Option<u16> {
    super::port_persist::load("port").ok().flatten()
}

/// What the first request line addresses.
#[derive(Debug, PartialEq, Eq)]
enum Target {
    /// `CONNECT host:port` - a tunnel, and the shape essentially all AI tool
    /// traffic takes, since it is all HTTPS.
    Tunnel { host: String, port: u16 },
    /// An absolute-form request (`GET http://host/path`), which is what a
    /// client given `HTTP_PROXY` sends for plain HTTP.
    Absolute { host: String, port: u16 },
}

impl Target {
    fn addr(&self) -> (&str, u16) {
        match self {
            Target::Tunnel { host, port } | Target::Absolute { host, port } => (host, *port),
        }
    }
}

/// Read up to and including the blank line that ends the request head.
///
/// Returns `None` on a client that closed or overran [`MAX_HEAD`] - both of
/// which are "not a request we can route", and neither of which deserves a
/// reply.
async fn read_head(client: &mut TcpStream) -> io::Result<Option<Vec<u8>>> {
    let mut head = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    while head.len() < MAX_HEAD {
        match client.read(&mut byte).await? {
            0 => return Ok(None),
            _ => head.push(byte[0]),
        }
        if head.ends_with(b"\r\n\r\n") {
            return Ok(Some(head));
        }
    }
    Ok(None)
}

/// Split `host:port`, defaulting the port. IPv6 literals arrive bracketed.
fn split_host_port(authority: &str, default_port: u16) -> Option<(String, u16)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        let port = match tail.strip_prefix(':') {
            Some(p) => p.parse().ok()?,
            None => default_port,
        };
        return Some((host.to_string(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => Some((host.to_string(), port.parse().ok()?)),
        None => Some((authority.to_string(), default_port)),
    }
}

/// What the head addresses, or `None` if it is not a proxy request we can act
/// on. Origin-form requests land here too: a client sending one to a proxy has
/// not been configured to use one, and we have no host to send it to.
fn parse_target(head: &[u8]) -> Option<Target> {
    let text = std::str::from_utf8(head).ok()?;
    let line = text.lines().next()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    let uri = parts.next()?;

    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = split_host_port(uri, 443)?;
        return Some(Target::Tunnel { host, port });
    }
    let (scheme_default, rest) = match uri.split_once("://") {
        Some(("http", rest)) => (80u16, rest),
        Some(("https", rest)) => (443u16, rest),
        _ => return None,
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    // Strip any userinfo; it addresses the origin, not us, and is not part of
    // the host.
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host, port) = split_host_port(authority, scheme_default)?;
    Some(Target::Absolute { host, port })
}

/// Rewrite an absolute-form head for a direct hop to the origin.
///
/// Three changes, each required rather than tidy: the request line becomes
/// origin-form (an origin server is not a proxy and may reject an absolute
/// URI); `Proxy-Authorization` and `Proxy-Connection` are dropped, because they
/// are hop-by-hop and address *us*; and `Connection: close` is forced, because
/// a proxied connection may carry requests for different hosts one after
/// another and this hop is pinned to one origin. Forcing close ends the
/// connection after this exchange, so the client opens a fresh one for the next
/// host and gets routed correctly.
fn rewrite_direct(head: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(head).ok()?;
    let mut lines = text.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?;
    let uri = parts.next()?;
    let version = parts.next().unwrap_or("HTTP/1.1");

    let rest = uri.split_once("://").map(|(_, rest)| rest)?;
    let path = match rest.find(['/', '?']) {
        Some(i) => &rest[i..],
        None => "/",
    };

    let mut out = format!("{method} {path} {version}\r\n");
    for line in lines {
        if line.is_empty() {
            break;
        }
        let name = line.split(':').next().unwrap_or("").trim();
        if name.eq_ignore_ascii_case("proxy-authorization")
            || name.eq_ignore_ascii_case("proxy-connection")
            || name.eq_ignore_ascii_case("connection")
        {
            continue;
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str("Connection: close\r\n\r\n");
    Some(out.into_bytes())
}

/// Serve one client connection.
///
/// The head is read whole before anything is dialed, because which upstream to
/// dial depends on whether the engine is there, and what to *send* depends on
/// which upstream we chose. After that the connection is spliced in both
/// directions and this stops looking at it.
async fn handle(mut client: TcpStream, engine_port: EngineLookup) -> Result<()> {
    let Some(head) = read_head(&mut client).await? else {
        return Ok(());
    };
    let Some(target) = parse_target(&head) else {
        // Not something we can forward. Say so rather than hanging: a client
        // that gets no reply retries, and a retry loop against a proxy that
        // will never answer is worse than a clear refusal.
        let _ = client
            .write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n")
            .await;
        return Ok(());
    };

    // Engine first, always. It is the one that can route, and while it is up
    // this forwarder is a transparent extra hop: the head goes over verbatim,
    // including the `Proxy-Authorization` selector Claude Code sends, which the
    // engine reads to force its own route.
    if let Some(port) = engine_port() {
        if let Ok(Ok(mut upstream)) = tokio::time::timeout(
            ENGINE_PROBE_TIMEOUT,
            TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], port))),
        )
        .await
        {
            upstream.write_all(&head).await?;
            // The engine answers the CONNECT itself, so its 200 (or its
            // refusal) reaches the client through the splice below.
            tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
            return Ok(());
        }
    }

    // No engine: this is the fail-open path the whole module exists for.
    let (host, port) = target.addr();
    let mut origin = match TcpStream::connect((host, port)).await {
        Ok(s) => s,
        Err(e) => {
            // The origin itself is unreachable - report it the way a proxy
            // should rather than dropping the connection, so the tool's error
            // names the real problem instead of "connection refused" pointing
            // back at us.
            let _ = client
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n")
                .await;
            return Err(e).with_context(|| format!("connecting directly to {host}:{port}"));
        }
    };

    match target {
        Target::Tunnel { .. } => {
            // We are the tunnel's other end now, so we owe the client the
            // response the engine would have sent.
            client
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await?;
        }
        Target::Absolute { .. } => {
            let Some(rewritten) = rewrite_direct(&head) else {
                let _ = client
                    .write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n")
                    .await;
                return Ok(());
            };
            origin.write_all(&rewritten).await?;
        }
    }
    tokio::io::copy_bidirectional(&mut client, &mut origin).await?;
    Ok(())
}

/// Accept until the marker file goes away.
async fn serve(listener: TcpListener, engine_port: EngineLookup, until: Shutdown) -> Result<()> {
    loop {
        tokio::select! {
            _ = until.wait() => return Ok(()),
            accepted = listener.accept() => {
                let (client, _) = match accepted {
                    Ok(pair) => pair,
                    // Pause rather than spin: a permanently failed listener
                    // would otherwise burn a core, the same way the relay and
                    // PAC accept loops guard themselves.
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                };
                let engine_port = engine_port.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle(client, engine_port).await {
                        if super::engine::debug_log() {
                            eprintln!("[gate-forwarder] connection ended: {e:#}");
                        }
                    }
                });
            }
        }
    }
}

/// What ends the accept loop. The marker poll in production; an immediate
/// never-fires in the tests, which stop by dropping the runtime.
#[derive(Clone)]
enum Shutdown {
    WhenUnwanted,
    #[cfg(test)]
    Never,
}

impl Shutdown {
    async fn wait(&self) {
        match self {
            Shutdown::WhenUnwanted => loop {
                tokio::time::sleep(MARKER_POLL).await;
                if !marker_present() {
                    return;
                }
            },
            #[cfg(test)]
            Shutdown::Never => std::future::pending().await,
        }
    }
}

/// Entry point for `<current-exe> --env-forwarder`.
///
/// Binds the persisted port so the variables exported by a previous session
/// stay valid, and only then records it: a forwarder that could not have the
/// old port must not claim it in the file that tells the next enable what to
/// export.
pub fn run() -> Result<()> {
    let listener = match persisted_port() {
        Some(port) => super::engine::bind_preferred(port)
            .or_else(|_| std::net::TcpListener::bind(("127.0.0.1", 0))),
        None => std::net::TcpListener::bind(("127.0.0.1", 0)),
    }
    .context("binding the environment forwarder")?;
    let port = listener.local_addr()?.port();
    super::port_persist::save(PORT_NAME, port)
        .context("recording the environment forwarder port")?;
    listener
        .set_nonblocking(true)
        .context("setting the forwarder listener non-blocking")?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building the forwarder runtime")?;
    rt.block_on(async move {
        let listener = TcpListener::from_std(listener)?;
        serve(
            listener,
            Arc::new(persisted_engine_port),
            Shutdown::WhenUnwanted,
        )
        .await
    })
}

/// Spawn the forwarder as a detached child, so it outlives this process.
///
/// `setsid` on Unix leaves the GUI's session and controlling terminal (still in
/// the login session, so it goes at logout - the lifetime we want);
/// `DETACHED_PROCESS` is the Windows equivalent. Same argv trick as the Linux
/// helper: no second binary to package, sign or find.
/// Only the desktop managers spawn a forwarder: Linux routes through a daemon
/// that already outlives the GUI, so it has never had the failure this fixes.
/// Kept compiled everywhere rather than `cfg`-ed out, so a Linux build still
/// type-checks it and the protocol tests below run on every OS.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
fn spawn_detached() -> Result<()> {
    use std::process::{Command, Stdio};
    let exe = std::env::current_exe().context("resolving current exe")?;
    let mut cmd = Command::new(exe);
    cmd.arg(FORWARDER_FLAG)
        .stdin(Stdio::null())
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

/// How long to wait for a freshly spawned forwarder to be accepting.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
const SPAWN_TIMEOUT: Duration = Duration::from_secs(3);

/// Ensure a forwarder is running and return the port the machine-wide
/// variables should name.
///
/// Idempotent, and cheap in the common case: an already-running forwarder is
/// one loopback connect away. The marker is written first so a forwarder that
/// starts fast cannot read it before it exists and exit immediately.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
pub(crate) fn ensure_running() -> Result<u16> {
    let marker = marker_path()?;
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    crate::primitives::write_file(&marker, b"", 0o600)
        .with_context(|| format!("writing {}", marker.display()))?;

    if let Some(port) = persisted_port() {
        if answering(port) {
            return Ok(port);
        }
    }
    spawn_detached()?;

    let deadline = std::time::Instant::now() + SPAWN_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if let Some(port) = persisted_port() {
            if answering(port) {
                return Ok(port);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    anyhow::bail!("the environment forwarder did not start within {SPAWN_TIMEOUT:?}")
}

/// Ask a running forwarder to exit, by removing the marker it polls.
///
/// Best-effort and promptless. Deliberately *not* called from `disable`: a
/// forwarder that went away when routing was switched off would strand exactly
/// the processes it exists to protect.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
pub(crate) fn stop() {
    if let Ok(path) = marker_path() {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Start a forwarder on an ephemeral port, told to look for an engine at
    /// `engine_port`. Returns the port to point a client at.
    async fn start_forwarder(engine_port: Option<u16>) -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            serve(listener, Arc::new(move || engine_port), Shutdown::Never).await
        });
        port
    }

    /// A loopback port that nothing is listening on: bound to reserve it, then
    /// dropped. Modelling "the engine went away" by picking a number would be
    /// a test that passes for the wrong reason if something else is there.
    fn dead_port() -> u16 {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        l.local_addr().unwrap().port()
    }

    /// Accept one connection, hand back everything sent before the blank line,
    /// then run `reply`.
    fn one_shot(reply: &'static [u8]) -> (u16, tokio::sync::oneshot::Receiver<Vec<u8>>) {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut sock, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                if sock.read(&mut byte).unwrap() == 0 {
                    break;
                }
                head.push(byte[0]);
            }
            let _ = sock.write_all(reply);
            let _ = sock.flush();
            let _ = tx.send(head);
            // Hold the socket open briefly so the client can read the reply.
            std::thread::sleep(Duration::from_millis(200));
        });
        (port, rx)
    }

    async fn read_some(stream: &mut TcpStream) -> String {
        let mut buf = vec![0u8; 512];
        let n = tokio::time::timeout(Duration::from_secs(3), stream.read(&mut buf))
            .await
            .expect("upstream should answer")
            .expect("read");
        String::from_utf8_lossy(&buf[..n]).to_string()
    }

    /// While the engine is up this is a transparent extra hop. In particular
    /// the `Proxy-Authorization` selector has to arrive verbatim: it is how the
    /// engine knows to force Claude Code's route, and swallowing it here would
    /// silently unroute Claude Code while every status still read Connected.
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
    /// still reaches its provider - the path it would have taken with Gate not
    /// installed - instead of getting the connection refused that made turning
    /// routing off break every already-running tool.
    #[tokio::test]
    async fn goes_direct_when_the_engine_is_gone() {
        let (origin_port, origin_saw) = one_shot(b"pong");
        let port = start_forwarder(Some(dead_port())).await;

        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client
            .write_all(format!("CONNECT 127.0.0.1:{origin_port} HTTP/1.1\r\n\r\n").as_bytes())
            .await
            .unwrap();
        assert!(
            read_some(&mut client).await.starts_with("HTTP/1.1 200"),
            "the forwarder owes the client the response the engine would have sent"
        );

        // And it is a real tunnel to the origin, not a reply invented here.
        client.write_all(b"ping\r\n\r\n").await.unwrap();
        let seen = String::from_utf8(origin_saw.await.unwrap()).unwrap();
        assert!(seen.starts_with("ping"), "{seen}");
        assert!(
            !seen.contains("CONNECT"),
            "the CONNECT terminates here; the origin must never see it: {seen}"
        );
    }

    /// Going direct must not carry the credential that addressed *us* to a
    /// third party. Hop-by-hop in the RFC, and a real leak if forwarded.
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

    /// A request we cannot route gets a refusal, not silence. A client that
    /// gets no reply retries, and a retry loop against a proxy that will never
    /// answer is worse than a clear error.
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

    #[test]
    fn a_connect_line_names_its_tunnel() {
        assert_eq!(
            parse_target(b"CONNECT api.anthropic.com:443 HTTP/1.1\r\n\r\n"),
            Some(Target::Tunnel {
                host: "api.anthropic.com".into(),
                port: 443
            })
        );
        // A CONNECT without a port is still a tunnel; 443 is the only sensible
        // reading and refusing it would break a client for no gain.
        assert_eq!(
            parse_target(b"CONNECT example.com HTTP/1.1\r\n\r\n"),
            Some(Target::Tunnel {
                host: "example.com".into(),
                port: 443
            })
        );
    }

    #[test]
    fn an_absolute_form_request_names_its_origin() {
        assert_eq!(
            parse_target(b"GET http://example.com/v1/x?q=1 HTTP/1.1\r\n\r\n"),
            Some(Target::Absolute {
                host: "example.com".into(),
                port: 80
            })
        );
        assert_eq!(
            parse_target(b"POST https://example.com:8443/x HTTP/1.1\r\n\r\n"),
            Some(Target::Absolute {
                host: "example.com".into(),
                port: 8443
            })
        );
        // Userinfo addresses the origin, not us, and is not part of the host.
        assert_eq!(
            parse_target(b"GET http://user:pw@example.com/x HTTP/1.1\r\n\r\n"),
            Some(Target::Absolute {
                host: "example.com".into(),
                port: 80
            })
        );
    }

    #[test]
    fn an_ipv6_literal_keeps_its_brackets_off_the_host() {
        assert_eq!(
            parse_target(b"CONNECT [::1]:8080 HTTP/1.1\r\n\r\n"),
            Some(Target::Tunnel {
                host: "::1".into(),
                port: 8080
            })
        );
    }

    #[test]
    fn an_origin_form_request_is_not_ours_to_route() {
        // No host to send it to: a client sending this was not configured to
        // use a proxy at all.
        assert_eq!(parse_target(b"GET /v1/messages HTTP/1.1\r\n\r\n"), None);
    }

    #[test]
    fn the_direct_rewrite_drops_the_hop_that_addressed_us() {
        let head = b"GET http://example.com/v1/x HTTP/1.1\r\n\
                     Host: example.com\r\n\
                     Proxy-Authorization: Basic Z2F0ZQ==\r\n\
                     Proxy-Connection: keep-alive\r\n\
                     Connection: keep-alive\r\n\
                     Accept: */*\r\n\r\n";
        let out = String::from_utf8(rewrite_direct(head).expect("rewritten")).unwrap();

        // Origin-form, because an origin server is not a proxy.
        assert!(out.starts_with("GET /v1/x HTTP/1.1\r\n"), "{out}");
        // The proxy credential addresses this hop and must not go further. It
        // is also the one header whose leak would be a real finding.
        assert!(!out.to_lowercase().contains("proxy-authorization"), "{out}");
        assert!(!out.to_lowercase().contains("proxy-connection"), "{out}");
        // Pinned to one origin, so the connection must not be reused for the
        // next host the client asks for.
        assert!(out.contains("Connection: close\r\n"), "{out}");
        assert_eq!(
            out.to_lowercase().matches("connection: ").count(),
            1,
            "{out}"
        );
        // Everything else survives untouched.
        assert!(out.contains("Host: example.com\r\n"), "{out}");
        assert!(out.contains("Accept: */*\r\n"), "{out}");
    }
}
