//! The forwarding logic: read one request head, decide an upstream, splice.
//!
//! Everything here is deliberately dumb. The forwarder holds no credential,
//! terminates no TLS and knows nothing about which hosts Gate routes: the
//! engine already blind-tunnels whatever it does not route, so "hand it over
//! when it answers" needs no rules and cannot disagree with the engine's.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// The wire contract with the app, defined once in `gate-connect-paths` so the
/// two binaries cannot drift.
pub use gate_connect_paths::{
    FORWARDER_CHALLENGE_HEADER as CHALLENGE_HEADER, FORWARDER_HEALTH_PATH as HEALTH_PATH,
    FORWARDER_PROOF_HEADER as PROOF_HEADER,
};

/// How long to wait for the engine to accept before going direct. A loopback
/// refusal comes back in microseconds; this only bounds a port held by
/// something that accepts nothing.
pub(crate) const ENGINE_CONNECT_TIMEOUT: Duration = Duration::from_millis(250);

/// How long to wait for the engine's first response byte on a CONNECT before
/// concluding it died mid-handshake and going direct instead.
const ENGINE_FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a client has to finish sending its request head.
///
/// Without this a client that opens a connection and sends nothing holds a task
/// and a file descriptor forever, which is a denial primitive that costs an
/// attacker one socket.
pub(crate) const HEAD_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// The biggest request head buffered before giving up on a client. Only the
/// head is ever held; once the target is known the connection is spliced.
pub(crate) const MAX_HEAD: usize = 64 * 1024;

/// Most headers any real client sends on a proxy request.
pub(crate) const MAX_HEADERS: usize = 128;

/// What the first request line addresses.
#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    /// `CONNECT host:port` - a tunnel, and the shape essentially all AI tool
    /// traffic takes, since it is all HTTPS.
    Tunnel { host: String, port: u16 },
    /// An absolute-form request (`GET http://host/path`), which is what a
    /// client given `HTTP_PROXY` sends for plain HTTP.
    Absolute { host: String, port: u16 },
    /// A health probe. Answered here rather than forwarded: it is how the app
    /// proves the listener on the persisted port is the forwarder it spawned,
    /// and not some other process that happened to bind it first. Carries the
    /// proof this forwarder owes for the probe's challenge.
    Health { proof: String },
}

/// Everything read from the client before a decision could be made: the head
/// itself, plus whatever arrived in the same read.
pub struct Head {
    pub target: Target,
    /// The head as the client sent it, byte for byte. Forwarded verbatim when
    /// the engine takes the connection.
    pub raw: Vec<u8>,
    /// Bytes read past the end of the head - an early TLS ClientHello, or the
    /// start of a request body. Never dropped: they belong to whichever
    /// upstream we pick.
    pub leftover: Vec<u8>,
    /// Declared body length, when the head declares one plainly.
    ///
    /// `None` means "no body, or one this hop will not relay": absent
    /// `Content-Length`, or a `Transfer-Encoding` we would have to decode.
    /// Only the direct path consults it, and only to stop reading - see
    /// [`go_direct`].
    pub body_len: Option<u64>,
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

/// Where an absolute-form URI points, and the origin-form path to send onward.
fn absolute_parts(uri: &str) -> Option<(String, u16, String)> {
    let (default_port, rest) = match uri.split_once("://") {
        Some(("http", rest)) => (80u16, rest),
        Some(("https", rest)) => (443u16, rest),
        _ => return None,
    };
    let split = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(split);
    // Userinfo addresses the origin, not us, and is not part of the host.
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host, port) = split_host_port(authority, default_port)?;
    // A URI with no path at all is still a request for `/`. A query with no
    // path is too - `http://h?q=1` must become `/?q=1`, never a bare `?q=1`,
    // which is not a valid request target and which some origins reject and
    // others mis-route.
    let path = if tail.is_empty() {
        "/".to_string()
    } else if tail.starts_with('/') {
        tail.to_string()
    } else {
        format!("/{tail}")
    };
    Some((host, port, path))
}

/// Read one request head from the client, bounded in both size and time.
///
/// Parsing with `httparse` rather than scanning for a blank line: it handles
/// the header grammar (including rejecting the obsolete line folding that a
/// hand-rolled splitter silently passes through), it does not require the head
/// to be UTF-8, and it tells us exactly where the head ended so the bytes after
/// it can be kept.
pub async fn read_head(client: &mut TcpStream, token: &str) -> Result<Option<Head>> {
    let mut buf = Vec::with_capacity(2048);
    let deadline = tokio::time::Instant::now() + HEAD_READ_TIMEOUT;
    loop {
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(&buf) {
            Ok(httparse::Status::Complete(end)) => {
                let method = req.method.unwrap_or_default().to_string();
                let uri = req.path.unwrap_or_default().to_string();
                let target = if method.eq_ignore_ascii_case("CONNECT") {
                    let (host, port) =
                        split_host_port(&uri, 443).context("CONNECT target is not host:port")?;
                    Target::Tunnel { host, port }
                } else if uri == HEALTH_PATH && method.eq_ignore_ascii_case("GET") {
                    // Answer only a probe that brought a challenge, and answer
                    // it with proof of the token rather than a bare status: a
                    // `204` on its own is something any listener can say.
                    let challenge = req
                        .headers
                        .iter()
                        .find(|h| h.name.eq_ignore_ascii_case(CHALLENGE_HEADER))
                        .and_then(|h| std::str::from_utf8(h.value).ok());
                    match challenge {
                        Some(challenge) => Target::Health {
                            proof: gate_connect_paths::forwarder_proof(
                                token,
                                HEALTH_PATH,
                                challenge,
                            ),
                        },
                        None => return Ok(None),
                    }
                } else {
                    match absolute_parts(&uri) {
                        Some((host, port, _)) => Target::Absolute { host, port },
                        // Origin-form: a client sending this was not configured
                        // to use a proxy, so there is no host to send it to.
                        None => return Ok(None),
                    }
                };
                let chunked = req
                    .headers
                    .iter()
                    .any(|h| h.name.eq_ignore_ascii_case("transfer-encoding"));
                let body_len = if chunked {
                    None
                } else {
                    req.headers
                        .iter()
                        .find(|h| h.name.eq_ignore_ascii_case("content-length"))
                        .and_then(|h| std::str::from_utf8(h.value).ok())
                        .and_then(|v| v.trim().parse::<u64>().ok())
                };
                let leftover = buf[end..].to_vec();
                buf.truncate(end);
                return Ok(Some(Head {
                    target,
                    raw: buf,
                    leftover,
                    body_len,
                }));
            }
            Ok(httparse::Status::Partial) => {}
            Err(_) => return Ok(None),
        }
        if buf.len() >= MAX_HEAD {
            return Ok(None);
        }
        let mut chunk = [0u8; 2048];
        let n = match tokio::time::timeout_at(deadline, client.read(&mut chunk)).await {
            Ok(r) => r.context("reading the request head")?,
            // A client that opens a connection and says nothing gets dropped
            // rather than holding a task and an fd forever.
            Err(_) => return Ok(None),
        };
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// Rebuild an absolute-form head for a direct hop to the origin.
///
/// Rebuilt from the parsed headers rather than edited in place, so anything the
/// grammar does not admit cannot survive the rewrite. Three changes, each
/// required rather than tidy: the request line becomes origin-form (an origin
/// server is not a proxy and may reject an absolute URI); `Proxy-Authorization`
/// and `Proxy-Connection` are dropped, because they are hop-by-hop and address
/// *us*; and `Connection: close` is forced, because a proxied connection may
/// carry requests for different hosts one after another while this hop is
/// pinned to one origin.
pub fn rewrite_direct(head: &[u8]) -> Option<Vec<u8>> {
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut req = httparse::Request::new(&mut headers);
    if !matches!(req.parse(head), Ok(httparse::Status::Complete(_))) {
        return None;
    }
    let method = req.method?;
    let (_, _, path) = absolute_parts(req.path?)?;

    let mut out = format!("{method} {path} HTTP/1.1\r\n").into_bytes();
    for header in req.headers.iter() {
        let name = header.name;
        if name.eq_ignore_ascii_case("proxy-authorization")
            || name.eq_ignore_ascii_case("proxy-connection")
            || name.eq_ignore_ascii_case("connection")
        {
            continue;
        }
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(header.value);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"Connection: close\r\n\r\n");
    Some(out)
}

/// How a connection's upstream is chosen. Injectable so the tests can drive
/// every branch without persisting ports or starting an engine.
pub type EngineLookup = Arc<dyn Fn() -> Option<u16> + Send + Sync>;

const BAD_REQUEST: &[u8] = b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n";
const BAD_GATEWAY: &[u8] = b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n";
const TUNNEL_OK: &[u8] = b"HTTP/1.1 200 Connection Established\r\n\r\n";

/// Serve one client connection.
pub async fn handle(
    mut client: TcpStream,
    engine_port: EngineLookup,
    own_port: u16,
    token: Arc<str>,
) -> Result<()> {
    let Some(head) = read_head(&mut client, &token).await? else {
        // Not something we can forward. Say so rather than hanging: a client
        // that gets no reply retries, and a retry loop against a proxy that
        // will never answer is worse than a clear refusal.
        let _ = client.write_all(BAD_REQUEST).await;
        return Ok(());
    };

    if let Target::Health { proof } = &head.target {
        // Which relay port this forwarder holds rides along, so the app can
        // tell whether to put the engine's relay behind it. A forwarder built
        // before that existed sends no such header at all, which the app reads
        // as "stale, replace it".
        let relay = match crate::relay::HELD_PORT.load(std::sync::atomic::Ordering::SeqCst) {
            0 => "none".to_string(),
            port => port.to_string(),
        };
        let _ = client
            .write_all(
                format!(
                    "HTTP/1.1 204 No Content\r\n{PROOF_HEADER}: {proof}\r\n\
                     {}: {relay}\r\nConnection: close\r\n\r\n",
                    gate_connect_paths::FORWARDER_RELAY_HEADER
                )
                .as_bytes(),
            )
            .await;
        return Ok(());
    }

    // Engine first, always: it is the one that can route, and while it is up
    // this is a transparent extra hop. The head crosses verbatim, including the
    // `Proxy-Authorization` selector, which the engine reads to force a route
    // and which is the one header a proxy must not invent or drop on this path.
    if let Some(port) = engine_port() {
        // Never dial ourselves. The engine and forwarder port files are written
        // by different processes, and a stale or crossed pair would otherwise
        // produce a connection that recurses until something runs out.
        if port != own_port {
            if let Some(upstream) = connect_engine(port, &head).await {
                return splice_engine(client, upstream, head).await;
            }
        }
    }

    // No engine: the fail-open path this whole binary exists for.
    go_direct(client, head).await
}

/// Connect to the engine and hand it the head, returning the socket only if the
/// engine is actually talking to us.
async fn connect_engine(port: u16, head: &Head) -> Option<TcpStream> {
    let mut upstream = tokio::time::timeout(
        ENGINE_CONNECT_TIMEOUT,
        TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], port))),
    )
    .await
    .ok()?
    .ok()?;
    upstream.write_all(&head.raw).await.ok()?;
    if !head.leftover.is_empty() {
        upstream.write_all(&head.leftover).await.ok()?;
    }
    Some(upstream)
}

/// Splice client and engine, with one fail-open retry for a CONNECT the engine
/// accepts and then abandons.
///
/// The retry is limited to CONNECT because only there does the protocol
/// guarantee the server speaks first: waiting for a response on an
/// absolute-form request whose body the client has not sent yet would deadlock.
/// The window is real rather than theoretical - a disable stops the engine
/// while connections are being accepted - and without this the client sees a
/// zero-byte EOF, which is exactly the "it just fails" symptom the forwarder
/// exists to remove.
async fn splice_engine(mut client: TcpStream, mut upstream: TcpStream, head: Head) -> Result<()> {
    if matches!(head.target, Target::Tunnel { .. }) {
        let mut first = [0u8; 1024];
        let n = match tokio::time::timeout(ENGINE_FIRST_BYTE_TIMEOUT, upstream.read(&mut first))
            .await
        {
            Ok(Ok(n)) => n,
            // Dead or unresponsive after accepting: fall back rather than
            // passing the failure on. Nothing has been written to the client
            // yet, so the fallback is invisible to it.
            _ => return go_direct(client, head).await,
        };
        if n == 0 {
            return go_direct(client, head).await;
        }
        client.write_all(&first[..n]).await?;
    }
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

/// Connect the client straight to where it was going.
async fn go_direct(mut client: TcpStream, head: Head) -> Result<()> {
    let (host, port) = match &head.target {
        Target::Tunnel { host, port } | Target::Absolute { host, port } => (host.clone(), *port),
        Target::Health { .. } => return Ok(()),
    };
    let mut origin = match TcpStream::connect((host.as_str(), port)).await {
        Ok(s) => s,
        Err(e) => {
            // Report it the way a proxy should, so the tool's error names the
            // real problem instead of "connection refused" pointing back at us.
            let _ = client.write_all(BAD_GATEWAY).await;
            return Err(e).with_context(|| format!("connecting directly to {host}:{port}"));
        }
    };

    match head.target {
        Target::Tunnel { .. } => {
            // We are the tunnel's other end now, so we owe the client the
            // response the engine would have sent.
            client.write_all(TUNNEL_OK).await?;
        }
        Target::Absolute { .. } => {
            let Some(rewritten) = rewrite_direct(&head.raw) else {
                let _ = client.write_all(BAD_REQUEST).await;
                return Ok(());
            };
            origin.write_all(&rewritten).await?;
            // This hop is pinned to one origin, but the client's connection is
            // not: a client may pipeline a second absolute-form request for a
            // *different* host behind the first. Splicing would hand that
            // request to this origin verbatim - wrong destination, and with the
            // `Proxy-Authorization` that `rewrite_direct` was careful to strip
            // from request one. `Connection: close` asks the origin not to keep
            // the connection, but it does not stop the client from having
            // already sent request two.
            //
            // So relay exactly this request's declared body and not one byte
            // more, then read the response until the origin closes. Anything
            // the client pipelined behind it is dropped with the connection,
            // which is what `Connection: close` told it to expect.
            let declared = head.body_len.unwrap_or(0);
            let from_leftover = head.leftover.len().min(declared as usize);
            if from_leftover > 0 {
                origin.write_all(&head.leftover[..from_leftover]).await?;
            }
            let mut remaining = declared - from_leftover as u64;
            let mut buf = [0u8; 8192];
            while remaining > 0 {
                let want = buf.len().min(remaining as usize);
                let n = client.read(&mut buf[..want]).await?;
                if n == 0 {
                    break;
                }
                origin.write_all(&buf[..n]).await?;
                remaining -= n as u64;
            }
            tokio::io::copy(&mut origin, &mut client).await?;
            return Ok(());
        }
        Target::Health { .. } => {
            unreachable!("health is answered before any upstream is chosen")
        }
    }
    // Tunnel: the client and the origin own the bytes from here, and there is
    // exactly one destination for the connection's life.
    if !head.leftover.is_empty() {
        origin.write_all(&head.leftover).await?;
    }
    tokio::io::copy_bidirectional(&mut client, &mut origin).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absolute_uri_with_a_query_and_no_path_still_has_a_path() {
        // `GET ?q=1` is not a valid request target. Origins reject or mis-route
        // it, and the bug is invisible until someone proxies a URL shaped this
        // way.
        let (host, port, path) = absolute_parts("http://example.com?q=1").unwrap();
        assert_eq!(
            (host.as_str(), port, path.as_str()),
            ("example.com", 80, "/?q=1")
        );

        let (_, _, path) = absolute_parts("http://example.com").unwrap();
        assert_eq!(path, "/");

        let (host, port, path) = absolute_parts("https://example.com:8443/v1/x").unwrap();
        assert_eq!(
            (host.as_str(), port, path.as_str()),
            ("example.com", 8443, "/v1/x")
        );

        // Userinfo addresses the origin, not us.
        let (host, _, _) = absolute_parts("http://user:pw@example.com/x").unwrap();
        assert_eq!(host, "example.com");
    }

    #[test]
    fn an_ipv6_literal_keeps_its_brackets_off_the_host() {
        assert_eq!(
            split_host_port("[::1]:8080", 443),
            Some(("::1".to_string(), 8080))
        );
        assert_eq!(
            split_host_port("[::1]", 443),
            Some(("::1".to_string(), 443))
        );
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

        assert!(out.starts_with("GET /v1/x HTTP/1.1\r\n"), "{out}");
        // The proxy credential addresses this hop. Forwarding it is a leak.
        assert!(!out.to_lowercase().contains("proxy-authorization"), "{out}");
        assert!(!out.to_lowercase().contains("proxy-connection"), "{out}");
        assert!(out.contains("Connection: close\r\n"), "{out}");
        assert_eq!(
            out.to_lowercase().matches("connection: ").count(),
            1,
            "{out}"
        );
        assert!(out.contains("Host: example.com\r\n"), "{out}");
        assert!(out.contains("Accept: */*\r\n"), "{out}");
    }
}
