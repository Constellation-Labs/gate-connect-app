//! The relay listener: the port every relay tool config names, held here so it
//! keeps answering after the app is gone.
//!
//! Codex and OpenCode do not use a proxy. Their config names a base URL,
//! `http://127.0.0.1:<relay-port>/__gate/t/<tool>/<slug>/...`, and until this
//! listener existed the engine's relay was the only thing that ever bound that
//! port. It lives in the GUI process on macOS and Windows, so quitting took it
//! down, and the app had to rewrite both tools' configs on the way out and
//! again on the way back in - each rewrite costing a restart of every running
//! OpenCode and a resume of every open Codex conversation, and a crash leaving
//! them pointed at a port with nothing behind it.
//!
//! So the forwarder holds the public port and decides per connection:
//!
//! - **The engine's relay is up** (it binds [`RELAY_ENGINE_PORT_NAME`] and
//!   proves it holds the token): splice the connection to it untouched. Gate
//!   routes exactly as it did when the engine owned this port.
//! - **It is not**: serve the request here, straight to the provider the slug
//!   names, under the tool's own credential. That is what the engine's relay
//!   does while parked, and what the tool would do with Gate not installed.
//!
//! What the direct path is not allowed to be, and why each holds:
//!
//! - **Not an open proxy.** A request names one of
//!   [`gate_connect_paths::RELAY_UPSTREAMS`] or is refused, so the only hosts
//!   reachable are the providers Gate already knows.
//! - **Not a way to spend Gate's credential.** The forwarder has none. Every
//!   `x-gate-*` header is stripped, not only the ones the relay stamps.
//! - **Not reachable from a web page.** The same `Host` / `Origin` loopback
//!   boundary the engine's relay applies, from the same definition.
//! - **Not handed to a stranger.** A process squatting the engine's relay port
//!   while the app is closed would otherwise receive every tool's own provider
//!   key in plaintext. The engine side proves itself before a byte is spliced.
//!
//! One request per connection on the direct path, `Connection: close` both
//! ways. The engine can come back at any moment, and a connection pinned to
//! the direct path would keep bypassing it for as long as the tool's pool kept
//! the socket; it also means a client's connection pool keyed on this origin
//! cannot carry a request for one provider onto a TLS session opened to
//! another.

use std::io::Cursor;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use gate_connect_paths::{
    FORWARDER_CHALLENGE_HEADER, FORWARDER_PROOF_HEADER, RELAY_ENGINE_PORT_NAME, RELAY_HEALTH_PATH,
    RELAY_INTERCEPTING_HEADER, RELAY_LIVENESS_PATH, RELAY_PORT_NAME, RELAY_TOOL_PATH_PREFIX,
};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::net::{TcpListener, TcpStream};

use crate::proxy::EngineLookup;

/// The relay port this forwarder holds, or 0 while it holds none. Reported on
/// the forwarder's own health answer ([`gate_connect_paths::FORWARDER_RELAY_HEADER`]),
/// which is how the app decides whether to bind the engine's relay behind this
/// listener or on the public port itself.
pub static HELD_PORT: AtomicU16 = AtomicU16::new(0);

/// How often to try again for the relay port while something else holds it.
///
/// The ordinary holder is the app's own engine, on a session that started
/// before this forwarder did; it lets go when the app quits, and this is what
/// picks the port up so the tools pointed at it keep working. Matches the
/// marker poll, so a quit is covered within one tick.
const CLAIM_RETRY: Duration = Duration::from_secs(2);

/// How long a client has to finish sending its request head. Same bound, same
/// reason, as the forward-proxy listener's.
const HEAD_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// How long to wait for the provider to accept a connection.
const UPSTREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// The biggest request or response head held before giving up on it.
const MAX_HEAD: usize = 64 * 1024;

/// Most headers a head may carry.
const MAX_HEADERS: usize = 128;

/// The longest chunk-size or trailer line accepted in a chunked body.
const MAX_LINE: u64 = 8 * 1024;

/// Connections served at once on this listener, for the reason the forward
/// proxy caps its own.
const MAX_CONNECTIONS: usize = 512;

/// One entry of the routing table: a catalog slug and where it forwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstream {
    pub slug: String,
    /// The catalog's `upstream_url`, verbatim - the legacy header selects an
    /// entry by comparing against exactly this.
    pub url: String,
    pub tls: bool,
    pub host: String,
    pub port: u16,
    /// Path the catalog URL carries (`/api` for `https://claude.ai/api`), or
    /// empty. The request's own path is appended to it.
    pub base_path: String,
}

impl Upstream {
    /// Parse a catalog entry. `None` for anything that is not a plain
    /// `http(s)://host[:port][/path]`, which no catalog entry is.
    pub fn parse(slug: &str, url: &str) -> Option<Self> {
        let (tls, rest) = if let Some(rest) = url.strip_prefix("https://") {
            (true, rest)
        } else if let Some(rest) = url.strip_prefix("http://") {
            (false, rest)
        } else {
            return None;
        };
        let (authority, path) = match rest.find('/') {
            Some(i) => rest.split_at(i),
            None => (rest, ""),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().ok()?),
            None => (authority.to_string(), if tls { 443 } else { 80 }),
        };
        if host.is_empty() {
            return None;
        }
        Some(Upstream {
            slug: slug.to_string(),
            url: url.to_string(),
            tls,
            host,
            port,
            base_path: path.trim_end_matches('/').to_string(),
        })
    }

    /// The `Host` header an origin expects: the port only when it is not the
    /// scheme's default.
    fn host_header(&self) -> String {
        let default = if self.tls { 443 } else { 80 };
        if self.port == default {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// The routing table, from the shared contract.
pub fn upstreams() -> Vec<Upstream> {
    gate_connect_paths::relay_upstreams()
        .iter()
        .filter_map(|(slug, url)| Upstream::parse(slug, url))
        .collect()
}

/// Take the relay port and serve it, retrying for as long as something else
/// holds it.
///
/// The first attempt may take a fresh port, because on a first run there is no
/// persisted one and no config names any port yet. Every retry asks only for
/// the persisted port: a retry that took a fresh one would persist it, and the
/// next config write would move every tool off the port the running app is
/// serving.
pub fn start(forwarder_port: u16, token: Arc<str>) {
    let table: Arc<Vec<Upstream>> = Arc::new(upstreams());
    let backend: EngineLookup = Arc::new(|| gate_connect_paths::load_port(RELAY_ENGINE_PORT_NAME));
    tokio::spawn(async move {
        let mut first = true;
        loop {
            let initial = first;
            let bound = tokio::task::spawn_blocking(move || bind(initial, forwarder_port))
                .await
                .ok()
                .flatten();
            first = false;
            if let Some(std_listener) = bound {
                let adopted = std_listener
                    .set_nonblocking(true)
                    .ok()
                    .and_then(|()| std_listener.local_addr().ok())
                    .map(|addr| addr.port())
                    .and_then(|port| TcpListener::from_std(std_listener).ok().map(|l| (l, port)));
                if let Some((listener, port)) = adopted {
                    // Recorded once held, so the file the app writes into tool
                    // configs never names a port nothing answers on.
                    let _ = gate_connect_paths::save_port(RELAY_PORT_NAME, port);
                    HELD_PORT.store(port, Ordering::SeqCst);
                    serve(listener, backend, port, token, table).await;
                    HELD_PORT.store(0, Ordering::SeqCst);
                    return;
                }
            }
            tokio::time::sleep(CLAIM_RETRY).await;
        }
    });
}

/// One attempt at the relay port. See [`start`] for why only the first may
/// fall back to a fresh one.
fn bind(first: bool, forwarder_port: u16) -> Option<std::net::TcpListener> {
    if first {
        if let Some(listener) = crate::activated_socket("Relay") {
            return Some(listener);
        }
    }
    match gate_connect_paths::load_port(RELAY_PORT_NAME) {
        // A live listener is the common reason to be retrying at all - the
        // app's engine, on a session that started before this forwarder - and
        // `bind_preferred` would spend its whole grace period probing it. One
        // probe answers the question for this tick.
        Some(port) if !first && gate_connect_paths::port_is_live(port) => None,
        Some(port) => gate_connect_paths::bind_preferred(port).ok(),
        None if first => {
            let mut skip: Vec<u16> = crate::APP_PORT_NAMES
                .iter()
                .filter_map(|n| gate_connect_paths::load_port(n))
                .collect();
            skip.push(forwarder_port);
            gate_connect_paths::bind_fresh(&skip).ok()
        }
        None => None,
    }
}

/// Accept forever, serving each connection with [`handle`].
pub async fn serve(
    listener: TcpListener,
    backend: EngineLookup,
    own_port: u16,
    token: Arc<str>,
    table: Arc<Vec<Upstream>>,
) {
    let slots = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    loop {
        let client = match listener.accept().await {
            Ok((client, _)) => client,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            continue;
        };
        let backend = backend.clone();
        let token = token.clone();
        let table = table.clone();
        tokio::spawn(async move {
            let _slot = slot;
            let _ = handle(client, backend, own_port, token, table).await;
        });
    }
}

/// Serve one connection: hand it to the engine's relay if that is up and ours,
/// otherwise answer it here.
pub async fn handle(
    mut client: TcpStream,
    backend: EngineLookup,
    own_port: u16,
    token: Arc<str>,
    table: Arc<Vec<Upstream>>,
) -> Result<()> {
    if let Some(port) = backend().filter(|p| *p != own_port) {
        if backend_is_ours(port, token.clone()).await {
            let connected = tokio::time::timeout(
                Duration::from_millis(250),
                TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], port))),
            )
            .await;
            if let Ok(Ok(mut engine)) = connected {
                tokio::io::copy_bidirectional(&mut client, &mut engine).await?;
                return Ok(());
            }
        }
    }
    serve_direct(client, &token, &table).await
}

/// Whether the listener on the engine's relay port proves it holds our token.
///
/// Asked per connection rather than cached. The proof is one loopback round
/// trip, and a cache would be a window in which a port the engine had just
/// released could be taken by something else and still be trusted with a
/// plaintext request carrying the tool's own provider key.
async fn backend_is_ours(port: u16, token: Arc<str>) -> bool {
    tokio::task::spawn_blocking(move || {
        gate_connect_paths::proves_ours(port, RELAY_HEALTH_PATH, &token)
    })
    .await
    .unwrap_or(false)
}

/// How a message body is delimited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    None,
    Length(u64),
    Chunked,
    /// Until the sender closes. Only ever a response.
    UntilClose,
}

/// What a request becomes on its way to the provider.
#[derive(Debug)]
pub struct Plan {
    /// Index into the routing table.
    pub upstream: usize,
    /// The rewritten head, ready to send.
    pub head: Vec<u8>,
    pub body: Framing,
    /// The client asked to be told to go ahead before sending its body. The
    /// `Expect` header is not forwarded, so this hop owes it the `100`.
    pub expect_continue: bool,
    /// Whether the request was a `HEAD`, which decides the response framing.
    pub head_request: bool,
}

/// A request this listener will not forward, and the answer it gets instead.
#[derive(Debug, PartialEq, Eq)]
pub struct Refusal {
    pub status: u16,
    pub message: String,
}

fn refuse(status: u16, message: impl Into<String>) -> Refusal {
    Refusal {
        status,
        message: message.into(),
    }
}

/// Headers that address this hop and never go further. `host` is replaced, and
/// `expect` is answered here - see [`Plan::expect_continue`].
fn is_hop_by_hop(name: &str) -> bool {
    [
        "host",
        "connection",
        "keep-alive",
        "proxy-connection",
        "proxy-authorization",
        "proxy-authenticate",
        "te",
        "trailer",
        "upgrade",
        "expect",
    ]
    .iter()
    .any(|h| name.eq_ignore_ascii_case(h))
}

/// Whether a header is Gate-internal. Every `x-gate-*` name, not a list of the
/// ones the relay stamps: nothing under that prefix is the provider's business,
/// and a list is something a new header can be missing from.
fn is_gate_header(name: &str) -> bool {
    name.len() >= 7 && name[..7].eq_ignore_ascii_case("x-gate-")
}

/// Header names a `Connection` value nominates as hop-by-hop.
fn nominated(headers: &[(String, Vec<u8>)]) -> Vec<String> {
    headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case("connection"))
        .filter_map(|(_, v)| std::str::from_utf8(v).ok())
        .flat_map(|v| v.split(','))
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

fn header<'a>(headers: &'a [(String, Vec<u8>)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .and_then(|(_, v)| std::str::from_utf8(v).ok())
}

/// Body framing from a head's headers. `Transfer-Encoding` wins over
/// `Content-Length`, as RFC 9112 says, and the caller drops the length so the
/// two cannot disagree further along.
fn framing(headers: &[(String, Vec<u8>)]) -> Result<Framing, ()> {
    if let Some(te) = header(headers, "transfer-encoding") {
        let last = te.rsplit(',').next().unwrap_or("").trim();
        return if last.eq_ignore_ascii_case("chunked") {
            Ok(Framing::Chunked)
        } else {
            Err(())
        };
    }
    match header(headers, "content-length") {
        Some(v) => v.trim().parse().map(Framing::Length).map_err(|_| ()),
        None => Ok(Framing::None),
    }
}

/// Does the request target carry a `.` or `..` path segment? Refused for the
/// reason the engine's relay refuses it: the path that is classified has to be
/// the path that is sent.
fn has_dot_segment(target: &str) -> bool {
    let path = target.split_once('?').map_or(target, |(p, _)| p);
    path.split('/').any(|segment| {
        [".", "%2e", "..", ".%2e", "%2e.", "%2e%2e"]
            .iter()
            .any(|form| segment.eq_ignore_ascii_case(form))
    })
}

/// Split `/<segment>/rest?query` into `("<segment>", "/rest?query")`. Same rule
/// as the engine's relay.
fn split_leading_segment(target: &str) -> Option<(&str, String)> {
    let rest = target.strip_prefix('/')?;
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (segment, tail) = rest.split_at(end);
    if segment.is_empty() {
        return None;
    }
    let inner = if tail.is_empty() {
        "/".to_string()
    } else if tail.starts_with('?') {
        format!("/{tail}")
    } else {
        tail.to_string()
    };
    Some((segment, inner))
}

/// Decide where a relay request goes and rewrite its head for the provider.
///
/// Pure, so every refusal and rewrite is pinned without a socket.
pub fn plan(
    method: &str,
    target: &str,
    headers: &[(String, Vec<u8>)],
    table: &[Upstream],
) -> Result<Plan, Refusal> {
    if let Some(host) = header(headers, "host") {
        if !gate_connect_paths::authority_is_loopback(host) {
            return Err(refuse(
                403,
                "the Gate relay only serves requests addressed to 127.0.0.1/localhost",
            ));
        }
    }
    if let Some(origin) = header(headers, "origin") {
        if !gate_connect_paths::origin_is_loopback(origin) {
            return Err(refuse(
                403,
                "the Gate relay does not serve cross-origin browser requests",
            ));
        }
    }
    if !target.starts_with('/') {
        return Err(refuse(
            400,
            "the Gate relay only serves origin-form requests",
        ));
    }
    if has_dot_segment(target) {
        return Err(refuse(400, "request path contains a `.` or `..` segment"));
    }

    // The tool marker comes off first, and its segment goes with it whatever it
    // names: attribution is Gate's business, and there is no Gate on this path.
    let unmarked = match target.strip_prefix(RELAY_TOOL_PATH_PREFIX) {
        Some(rest) => split_leading_segment(&format!("/{rest}"))
            .map(|(_, inner)| inner)
            .unwrap_or_else(|| target.to_string()),
        None => target.to_string(),
    };
    let by_slug = split_leading_segment(&unmarked).and_then(|(segment, inner)| {
        table
            .iter()
            .position(|u| u.slug == segment)
            .map(|i| (i, inner))
    });
    let (index, inner) = match by_slug {
        Some(found) => found,
        // A config written before the slug moved into the path carries the
        // upstream as a header instead. It only selects an entry; the value
        // used is the entry's own.
        None => match header(headers, "x-gate-upstream-url") {
            Some(named) => match table.iter().position(|u| u.url == named) {
                Some(i) => (i, unmarked.clone()),
                None => {
                    return Err(refuse(
                        403,
                        format!("upstream {named:?} is not in the built-in catalog"),
                    ))
                }
            },
            None => {
                return Err(refuse(
                    400,
                    format!("{target:?} does not start with a known upstream slug"),
                ))
            }
        },
    };
    let upstream = &table[index];

    let body = framing(headers).map_err(|()| refuse(400, "unsupported request body framing"))?;
    let nominated = nominated(headers);
    let mut head = format!(
        "{method} {}{inner} HTTP/1.1\r\nHost: {}\r\n",
        upstream.base_path,
        upstream.host_header()
    )
    .into_bytes();
    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        if is_hop_by_hop(name)
            || is_gate_header(name)
            || nominated.contains(&lower)
            || (body == Framing::Chunked && lower == "content-length")
        {
            continue;
        }
        head.extend_from_slice(name.as_bytes());
        head.extend_from_slice(b": ");
        head.extend_from_slice(value);
        head.extend_from_slice(b"\r\n");
    }
    head.extend_from_slice(b"Connection: close\r\n\r\n");

    let expect_continue =
        header(headers, "expect").is_some_and(|v| v.trim().eq_ignore_ascii_case("100-continue"));
    Ok(Plan {
        upstream: index,
        head,
        body,
        expect_continue,
        head_request: method.eq_ignore_ascii_case("HEAD"),
    })
}

/// Rewrite a response head for the client: the provider's own connection
/// management goes, and `Connection: close` replaces it. Returns the head and
/// how its body is framed.
pub fn rewrite_response(raw: &[u8], head_request: bool) -> Option<(Vec<u8>, u16, Framing)> {
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut resp = httparse::Response::new(&mut headers);
    if !matches!(resp.parse(raw), Ok(httparse::Status::Complete(_))) {
        return None;
    }
    let status = resp.code?;
    let reason = resp.reason.unwrap_or("");
    let owned: Vec<(String, Vec<u8>)> = resp
        .headers
        .iter()
        .map(|h| (h.name.to_string(), h.value.to_vec()))
        .collect();
    let framing = if head_request || (100..200).contains(&status) || status == 204 || status == 304
    {
        Framing::None
    } else {
        match framing(&owned) {
            Ok(Framing::None) => Framing::UntilClose,
            Ok(f) => f,
            Err(()) => Framing::UntilClose,
        }
    };
    let nominated = nominated(&owned);
    let mut out = format!("HTTP/1.1 {status} {reason}\r\n").into_bytes();
    for (name, value) in &owned {
        let lower = name.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "connection" | "keep-alive" | "proxy-connection"
        ) || nominated.contains(&lower)
            || (framing == Framing::Chunked && lower == "content-length")
        {
            continue;
        }
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(value);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"Connection: close\r\n\r\n");
    Some((out, status, framing))
}

/// Read until a complete head is buffered. Returns the head's length; bytes
/// past it stay in `buf`.
async fn read_head<R: AsyncRead + Unpin>(
    r: &mut R,
    buf: &mut Vec<u8>,
    request: bool,
) -> Result<Option<usize>> {
    loop {
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let parsed = if request {
            httparse::Request::new(&mut headers).parse(buf)
        } else {
            httparse::Response::new(&mut headers).parse(buf)
        };
        match parsed {
            Ok(httparse::Status::Complete(end)) => return Ok(Some(end)),
            Ok(httparse::Status::Partial) => {}
            Err(_) => return Ok(None),
        }
        if buf.len() >= MAX_HEAD {
            return Ok(None);
        }
        let mut chunk = [0u8; 4096];
        let n = r.read(&mut chunk).await.context("reading a head")?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

async fn respond(client: &mut TcpStream, status: u16, message: &str) {
    let reason = match status {
        400 => "Bad Request",
        403 => "Forbidden",
        502 => "Bad Gateway",
        _ => "Error",
    };
    let _ = client
        .write_all(
            format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{message}",
                message.len()
            )
            .as_bytes(),
        )
        .await;
}

/// Answer one request with no engine behind this listener.
async fn serve_direct(mut client: TcpStream, token: &str, table: &[Upstream]) -> Result<()> {
    let mut buf = Vec::with_capacity(4096);
    let end = match tokio::time::timeout(HEAD_READ_TIMEOUT, read_head(&mut client, &mut buf, true))
        .await
    {
        Ok(Ok(Some(end))) => end,
        Ok(Err(e)) => return Err(e),
        _ => {
            respond(&mut client, 400, "malformed request").await;
            return Ok(());
        }
    };
    let (method, target, headers) = {
        let mut slots = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut req = httparse::Request::new(&mut slots);
        let _ = req.parse(&buf[..end]);
        (
            req.method.unwrap_or_default().to_string(),
            req.path.unwrap_or_default().to_string(),
            req.headers
                .iter()
                .map(|h| (h.name.to_string(), h.value.to_vec()))
                .collect::<Vec<_>>(),
        )
    };
    let leftover = buf.split_off(end);

    // Identity first, as the engine's relay does it. `intercepting: 0` is the
    // truth about this path: nothing here reaches Gate, so the tools' status
    // reads it the way it reads a parked engine.
    if method == "GET" && target == RELAY_HEALTH_PATH {
        let challenge = header(&headers, FORWARDER_CHALLENGE_HEADER).unwrap_or_default();
        let proof = gate_connect_paths::forwarder_proof(token, challenge);
        let _ = client
            .write_all(
                format!(
                    "HTTP/1.1 204 No Content\r\n{FORWARDER_PROOF_HEADER}: {proof}\r\n\
                     {RELAY_INTERCEPTING_HEADER}: 0\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await;
        return Ok(());
    }
    if method == "GET" && target == RELAY_LIVENESS_PATH {
        let _ = client
            .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
            .await;
        return Ok(());
    }

    let plan = match plan(&method, &target, &headers, table) {
        Ok(plan) => plan,
        Err(refusal) => {
            respond(&mut client, refusal.status, &refusal.message).await;
            return Ok(());
        }
    };
    let upstream = &table[plan.upstream];

    let tcp = match tokio::time::timeout(
        UPSTREAM_CONNECT_TIMEOUT,
        TcpStream::connect((upstream.host.as_str(), upstream.port)),
    )
    .await
    {
        Ok(Ok(tcp)) => tcp,
        _ => {
            respond(
                &mut client,
                502,
                &format!("could not reach {}", upstream.host_header()),
            )
            .await;
            return Ok(());
        }
    };
    if !upstream.tls {
        return exchange(client, tcp, plan, leftover).await;
    }
    let tls = match tls_connect(&upstream.host, tcp).await {
        Ok(tls) => tls,
        Err(e) => {
            respond(
                &mut client,
                502,
                &format!("TLS to {} failed: {e:#}", upstream.host_header()),
            )
            .await;
            return Ok(());
        }
    };
    exchange(client, tls, plan, leftover).await
}

/// Send one request and relay its response.
async fn exchange<S>(
    mut client: TcpStream,
    mut upstream: S,
    plan: Plan,
    leftover: Vec<u8>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if plan.expect_continue {
        client.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").await?;
    }
    upstream.write_all(&plan.head).await?;
    {
        // Exactly the declared body and not a byte more. Anything the client
        // pipelined behind it is dropped with the connection, which is what
        // the `Connection: close` it gets back tells it to expect.
        let mut body = BufReader::new(Cursor::new(leftover).chain(&mut client));
        copy_body(&mut body, &mut upstream, plan.body).await?;
    }
    upstream.flush().await?;

    let mut buf = Vec::with_capacity(4096);
    let (head, framing) = loop {
        let Some(end) = read_head(&mut upstream, &mut buf, false).await? else {
            respond(&mut client, 502, "the provider sent a malformed response").await;
            return Ok(());
        };
        let Some((head, status, framing)) = rewrite_response(&buf[..end], plan.head_request) else {
            respond(&mut client, 502, "the provider sent a malformed response").await;
            return Ok(());
        };
        // Interim responses (103 Early Hints) are passed on and the real one
        // is read after them.
        if (100..200).contains(&status) && status != 101 {
            client.write_all(&buf[..end]).await?;
            buf.drain(..end);
            continue;
        }
        buf.drain(..end);
        break (head, framing);
    };
    client.write_all(&head).await?;
    let mut body = BufReader::new(Cursor::new(buf).chain(&mut upstream));
    copy_body(&mut body, &mut client, framing).await?;
    client.flush().await?;
    let _ = client.shutdown().await;
    Ok(())
}

/// Copy one body, delimited as `framing` says, flushing as it goes so a
/// streamed response reaches the tool token by token.
async fn copy_body<R, W>(r: &mut R, w: &mut W, framing: Framing) -> Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    match framing {
        Framing::None => Ok(()),
        Framing::Length(n) => {
            let copied = tokio::io::copy_buf(&mut (&mut *r).take(n), w).await?;
            if copied != n {
                bail!("body ended after {copied} of {n} bytes");
            }
            Ok(())
        }
        Framing::UntilClose => {
            tokio::io::copy_buf(r, w).await?;
            Ok(())
        }
        Framing::Chunked => copy_chunked(r, w).await,
    }
}

/// Relay a chunked body verbatim, reading its framing only to know where it
/// ends.
async fn copy_chunked<R, W>(r: &mut R, w: &mut W) -> Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    loop {
        let line = read_line(r).await?;
        w.write_all(&line).await?;
        let size = parse_chunk_size(&line).context("malformed chunk size")?;
        if size == 0 {
            loop {
                let trailer = read_line(r).await?;
                w.write_all(&trailer).await?;
                if trailer == b"\r\n" || trailer == b"\n" {
                    break;
                }
            }
            w.flush().await?;
            return Ok(());
        }
        let copied = tokio::io::copy_buf(&mut (&mut *r).take(size), w).await?;
        if copied != size {
            bail!("chunk ended after {copied} of {size} bytes");
        }
        let end = read_line(r).await?;
        if end != b"\r\n" && end != b"\n" {
            bail!("chunk not terminated by a line break");
        }
        w.write_all(&end).await?;
        w.flush().await?;
    }
}

async fn read_line<R: AsyncBufRead + Unpin>(r: &mut R) -> Result<Vec<u8>> {
    let mut line = Vec::new();
    (&mut *r)
        .take(MAX_LINE)
        .read_until(b'\n', &mut line)
        .await?;
    if !line.ends_with(b"\n") {
        bail!("chunked body ended mid-line");
    }
    Ok(line)
}

fn parse_chunk_size(line: &[u8]) -> Option<u64> {
    let text = std::str::from_utf8(line).ok()?;
    let hex = text.split(';').next()?.trim();
    if hex.is_empty() {
        return None;
    }
    u64::from_str_radix(hex, 16).ok()
}

/// The TLS client configuration, built once.
///
/// The provider is named rather than taken from the process default: the
/// workspace compiles rustls with more than one, and with two available and
/// none installed, `ClientConfig::builder()` panics.
fn tls_config() -> Result<Arc<rustls::ClientConfig>> {
    static CONFIG: OnceLock<std::result::Result<Arc<rustls::ClientConfig>, String>> =
        OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
            let verifier = rustls_platform_verifier::Verifier::new(provider.clone())
                .map_err(|e| format!("building the certificate verifier: {e}"))?;
            let mut config = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|e| format!("choosing TLS versions: {e}"))?
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(verifier))
                .with_no_client_auth();
            // This hop speaks HTTP/1.1 and nothing else; offering h2 would let a
            // provider pick a protocol the bytes relayed here are not.
            config.alpn_protocols = vec![b"http/1.1".to_vec()];
            Ok(Arc::new(config))
        })
        .clone()
        .map_err(anyhow::Error::msg)
}

async fn tls_connect(
    host: &str,
    tcp: TcpStream,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>> {
    let name = rustls::pki_types::ServerName::try_from(host.to_string())
        .with_context(|| format!("{host:?} is not a valid server name"))?;
    tokio_rustls::TlsConnector::from(tls_config()?)
        .connect(name, tcp)
        .await
        .context("TLS handshake")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(origin_port: u16) -> Vec<Upstream> {
        vec![
            Upstream::parse("anthropic", &format!("http://127.0.0.1:{origin_port}")).unwrap(),
            Upstream::parse("claude-web", &format!("http://127.0.0.1:{origin_port}/api")).unwrap(),
        ]
    }

    fn hdrs(list: &[(&str, &str)]) -> Vec<(String, Vec<u8>)> {
        list.iter()
            .map(|(n, v)| ((*n).to_string(), v.as_bytes().to_vec()))
            .collect()
    }

    fn head_text(plan: &Plan) -> String {
        String::from_utf8(plan.head.clone()).unwrap()
    }

    #[test]
    fn the_catalog_urls_parse() {
        let u = Upstream::parse("claude-web", "https://claude.ai/api").unwrap();
        assert_eq!(
            (u.tls, u.host.as_str(), u.port, u.base_path.as_str()),
            (true, "claude.ai", 443, "/api")
        );
        assert_eq!(u.host_header(), "claude.ai");
        let u = Upstream::parse("x", "http://127.0.0.1:8080").unwrap();
        assert_eq!((u.tls, u.port, u.base_path.as_str()), (false, 8080, ""));
        assert_eq!(u.host_header(), "127.0.0.1:8080");
        // Every shipped entry has to parse, or its tools lose the direct path.
        assert_eq!(upstreams().len(), gate_connect_paths::RELAY_UPSTREAMS.len());
    }

    /// The shape every config written today sends: the tool marker, then the
    /// slug, then the client's own path.
    #[test]
    fn routes_a_marked_request_by_its_slug() {
        let t = table(9);
        let plan = plan(
            "POST",
            "/__gate/t/opencode/claude-web/v1/messages?beta=true",
            &hdrs(&[("Host", "127.0.0.1:47111"), ("Content-Length", "2")]),
            &t,
        )
        .unwrap();
        assert_eq!(plan.upstream, 1);
        let head = head_text(&plan);
        assert!(
            head.starts_with("POST /api/v1/messages?beta=true HTTP/1.1\r\nHost: 127.0.0.1:9\r\n"),
            "{head}"
        );
        assert_eq!(plan.body, Framing::Length(2));
        assert!(head.contains("Content-Length: 2\r\n"), "{head}");
        assert!(head.ends_with("Connection: close\r\n\r\n"), "{head}");
        assert_eq!(head.matches("Host:").count(), 1, "{head}");
    }

    #[test]
    fn routes_an_unmarked_request_and_a_legacy_header() {
        let t = table(9);
        let plan1 = plan("GET", "/anthropic", &[], &t).unwrap();
        assert!(head_text(&plan1).starts_with("GET / HTTP/1.1\r\n"));

        let legacy = plan(
            "POST",
            "/v1/messages",
            &hdrs(&[("x-gate-upstream-url", "http://127.0.0.1:9/api")]),
            &t,
        )
        .unwrap();
        assert_eq!(legacy.upstream, 1);
        assert!(head_text(&legacy).starts_with("POST /api/v1/messages HTTP/1.1\r\n"));
    }

    /// The property that keeps the listener from being an open proxy.
    #[test]
    fn refuses_anything_outside_the_catalog() {
        let t = table(9);
        assert_eq!(
            plan("GET", "/evil.example/x", &[], &t).unwrap_err().status,
            400
        );
        assert_eq!(
            plan(
                "GET",
                "/x",
                &hdrs(&[("x-gate-upstream-url", "https://evil.example")]),
                &t
            )
            .unwrap_err()
            .status,
            403
        );
        assert_eq!(
            plan("GET", "http://evil.example/x", &[], &t)
                .unwrap_err()
                .status,
            400
        );
    }

    #[test]
    fn refuses_a_dot_segment() {
        let t = table(9);
        for target in ["/anthropic/v1/../../x", "/anthropic/%2e%2e/x"] {
            assert_eq!(
                plan("GET", target, &[], &t).unwrap_err().status,
                400,
                "{target}"
            );
        }
    }

    /// The browser boundary, from the same definition the engine's relay uses.
    #[test]
    fn refuses_a_browser() {
        let t = table(9);
        let rebound = hdrs(&[("Host", "attacker.example")]);
        assert_eq!(
            plan("GET", "/anthropic/x", &rebound, &t)
                .unwrap_err()
                .status,
            403
        );
        let cross_site = hdrs(&[
            ("Host", "127.0.0.1"),
            ("Origin", "https://attacker.example"),
        ]);
        assert_eq!(
            plan("POST", "/anthropic/x", &cross_site, &t)
                .unwrap_err()
                .status,
            403
        );
    }

    /// Nothing Gate-internal reaches a provider, and nothing that addressed
    /// this hop does either. The tool's own credential does: it is the only
    /// credential on this path.
    #[test]
    fn strips_gate_and_hop_by_hop_headers_and_keeps_the_tools_credential() {
        let t = table(9);
        let plan = plan(
            "POST",
            "/anthropic/v1/messages",
            &hdrs(&[
                ("Host", "localhost"),
                ("Authorization", "Bearer sk-own"),
                ("x-api-key", "own-key"),
                ("X-Gate-Authorization", "Bearer gate"),
                ("x-gate-api-key", "gate-key"),
                ("x-gate-something-new", "1"),
                ("Connection", "keep-alive, X-Custom-Hop"),
                ("X-Custom-Hop", "1"),
                ("Keep-Alive", "timeout=5"),
                ("Proxy-Authorization", "Basic Z2F0ZQ=="),
                ("Upgrade", "websocket"),
                ("Content-Length", "0"),
            ]),
            &t,
        )
        .unwrap();
        let head = head_text(&plan).to_ascii_lowercase();
        assert!(head.contains("authorization: bearer sk-own\r\n"), "{head}");
        assert!(head.contains("x-api-key: own-key\r\n"), "{head}");
        for gone in [
            "x-gate-",
            "x-custom-hop",
            "keep-alive",
            "proxy-authorization",
            "upgrade",
            "host: localhost",
        ] {
            assert!(!head.contains(gone), "{gone} survived: {head}");
        }
        assert_eq!(head.matches("connection:").count(), 1, "{head}");
    }

    /// `Transfer-Encoding` and `Content-Length` together is the classic
    /// smuggling shape; the length goes so the next hop cannot read it.
    #[test]
    fn chunked_wins_over_a_length() {
        let t = table(9);
        let plan = plan(
            "POST",
            "/anthropic/v1/messages",
            &hdrs(&[("Transfer-Encoding", "chunked"), ("Content-Length", "5")]),
            &t,
        )
        .unwrap();
        assert_eq!(plan.body, Framing::Chunked);
        assert!(!head_text(&plan)
            .to_ascii_lowercase()
            .contains("content-length"));
    }

    #[test]
    fn a_response_is_closed_and_framed() {
        let (head, status, framing) = rewrite_response(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\
              Connection: keep-alive\r\nKeep-Alive: timeout=5\r\n\r\n",
            false,
        )
        .unwrap();
        let head = String::from_utf8(head).unwrap();
        assert_eq!((status, framing), (200, Framing::Chunked));
        assert!(head.contains("Transfer-Encoding: chunked\r\n"), "{head}");
        assert!(!head.contains("keep-alive"), "{head}");
        assert!(head.ends_with("Connection: close\r\n\r\n"), "{head}");

        let (_, _, framing) =
            rewrite_response(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\n", true).unwrap();
        assert_eq!(framing, Framing::None, "a HEAD response has no body");
        let (_, _, framing) = rewrite_response(b"HTTP/1.1 200 OK\r\n\r\n", false).unwrap();
        assert_eq!(framing, Framing::UntilClose);
    }

    #[tokio::test]
    async fn chunked_bodies_are_relayed_verbatim_and_stop_at_the_end() {
        let body = b"4\r\nWiki\r\n5;ext=1\r\npedia\r\n0\r\nX-Trailer: 1\r\n\r\nNEXT REQUEST";
        let mut reader = BufReader::new(&body[..]);
        let mut out = Vec::new();
        copy_chunked(&mut reader, &mut out).await.unwrap();
        assert_eq!(
            out,
            b"4\r\nWiki\r\n5;ext=1\r\npedia\r\n0\r\nX-Trailer: 1\r\n\r\n".to_vec()
        );
    }

    // ---- over real sockets -------------------------------------------------

    const TOKEN: &str = "relay-test-token";

    /// A plain-HTTP origin that records one request and answers `reply`.
    fn origin(reply: &'static [u8]) -> (u16, tokio::sync::oneshot::Receiver<Vec<u8>>) {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut sock, _) = listener.accept().unwrap();
            let _ = sock.set_read_timeout(Some(Duration::from_millis(300)));
            let mut seen = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                match sock.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => seen.extend_from_slice(&chunk[..n]),
                }
            }
            let _ = sock.write_all(reply);
            let _ = tx.send(seen);
        });
        (port, rx)
    }

    async fn start_relay(backend: Option<u16>, table: Vec<Upstream>) -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(serve(
            listener,
            Arc::new(move || backend),
            port,
            Arc::from(TOKEN),
            Arc::new(table),
        ));
        port
    }

    async fn roundtrip(port: u16, request: &[u8]) -> String {
        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client.write_all(request).await.unwrap();
        let mut out = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut out))
            .await
            .expect("the relay should answer and close")
            .unwrap();
        String::from_utf8_lossy(&out).to_string()
    }

    fn dead_port() -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    /// The whole point: with no engine, a tool's request reaches its provider.
    #[tokio::test]
    async fn goes_to_the_provider_when_the_engine_is_gone() {
        let (origin_port, seen) =
            origin(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: keep-alive\r\n\r\nok");
        let port = start_relay(Some(dead_port()), table(origin_port)).await;

        let reply = roundtrip(
            port,
            b"POST /__gate/t/codex/anthropic/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\n\
              Authorization: Bearer sk-own\r\nContent-Length: 4\r\n\r\nbody",
        )
        .await;
        assert!(reply.starts_with("HTTP/1.1 200 OK\r\n"), "{reply}");
        assert!(reply.contains("Connection: close\r\n"), "{reply}");
        assert!(reply.ends_with("\r\n\r\nok"), "{reply}");

        let seen = String::from_utf8(seen.await.unwrap()).unwrap();
        assert!(seen.starts_with("POST /v1/messages HTTP/1.1\r\n"), "{seen}");
        assert!(seen.contains("Authorization: Bearer sk-own\r\n"), "{seen}");
        assert!(seen.ends_with("\r\n\r\nbody"), "{seen}");
    }

    /// A relay of ours on the engine port gets the connection, byte for byte.
    #[tokio::test]
    async fn hands_the_connection_to_an_engine_that_proves_itself() {
        let engine = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let engine_port = engine.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel::<Vec<u8>>();
        tokio::spawn(async move {
            let mut tx = Some(tx);
            loop {
                let (mut sock, _) = engine.accept().await.unwrap();
                let mut buf = vec![0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                if req.starts_with(&format!("GET {RELAY_HEALTH_PATH}")) {
                    let challenge = req
                        .lines()
                        .find_map(|l| l.strip_prefix(&format!("{FORWARDER_CHALLENGE_HEADER}: ")))
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    let proof = gate_connect_paths::forwarder_proof(TOKEN, &challenge);
                    let _ = sock
                        .write_all(
                            format!(
                                "HTTP/1.1 204 No Content\r\n{FORWARDER_PROOF_HEADER}: {proof}\r\n\r\n"
                            )
                            .as_bytes(),
                        )
                        .await;
                    continue;
                }
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nengine",
                    )
                    .await;
                if let Some(tx) = tx.take() {
                    let _ = tx.send(buf[..n].to_vec());
                }
            }
        });
        let port = start_relay(Some(engine_port), table(dead_port())).await;

        let request = b"POST /__gate/t/codex/anthropic/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\n\
                        x-gate-client: kept-for-the-engine\r\nContent-Length: 0\r\n\r\n";
        let reply = roundtrip(port, request).await;
        assert!(reply.ends_with("engine"), "{reply}");
        assert_eq!(rx.await.unwrap(), request.to_vec(), "spliced untouched");
    }

    /// A stranger on the engine's port is never handed a request: it would
    /// carry the tool's own provider key in plaintext.
    #[tokio::test]
    async fn never_hands_a_request_to_a_listener_that_cannot_prove_itself() {
        let squatter = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let squatter_port = squatter.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for sock in squatter.incoming() {
                let Ok(mut sock) = sock else { continue };
                let _ = sock.set_read_timeout(Some(Duration::from_millis(200)));
                let mut buf = vec![0u8; 4096];
                let n = sock.read(&mut buf).unwrap_or(0);
                let _ = tx.send(buf[..n].to_vec());
                let _ = sock.write_all(b"HTTP/1.1 204 No Content\r\n\r\n");
            }
        });
        let (origin_port, seen) = origin(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        let port = start_relay(Some(squatter_port), table(origin_port)).await;

        let reply = roundtrip(
            port,
            b"GET /anthropic/v1/models HTTP/1.1\r\nHost: 127.0.0.1\r\n\
              Authorization: Bearer sk-own\r\n\r\n",
        )
        .await;
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        assert!(String::from_utf8(seen.await.unwrap())
            .unwrap()
            .contains("Bearer sk-own"));
        while let Ok(got) = rx.try_recv() {
            let got = String::from_utf8_lossy(&got).to_string();
            assert!(
                !got.contains("sk-own"),
                "the squatter must only ever see the challenge: {got}"
            );
        }
    }

    /// With no engine, the listener still proves it is ours, and says it is
    /// not routing - the same answer a parked engine gives.
    #[tokio::test]
    async fn answers_the_relay_proof_as_not_intercepting() {
        let port = start_relay(None, table(dead_port())).await;
        let headers = tokio::task::spawn_blocking(move || {
            gate_connect_paths::probe_with_proof(port, RELAY_HEALTH_PATH, TOKEN)
        })
        .await
        .unwrap()
        .expect("the listener must prove the token");
        assert!(
            headers
                .iter()
                .any(|(n, v)| n == RELAY_INTERCEPTING_HEADER && v == "0"),
            "{headers:?}"
        );
        let reply = roundtrip(
            port,
            format!("GET {RELAY_LIVENESS_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes(),
        )
        .await;
        assert!(reply.starts_with("HTTP/1.1 204"), "{reply}");
    }

    #[tokio::test]
    async fn a_refusal_is_an_answer_not_silence() {
        let port = start_relay(None, table(dead_port())).await;
        let reply = roundtrip(port, b"GET /nowhere/x HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").await;
        assert!(reply.starts_with("HTTP/1.1 400"), "{reply}");

        let reply = roundtrip(
            port,
            b"GET /anthropic/x HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
        )
        .await;
        assert!(
            reply.starts_with("HTTP/1.1 502"),
            "an unreachable provider: {reply}"
        );
    }
}
