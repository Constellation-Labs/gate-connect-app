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
//! - **The engine's relay is up** (it binds [`RELAY_ENGINE_PORT_NAME`]): prove
//!   it is Gate's on the connection itself, then splice the client onto that
//!   same connection untouched. Gate routes exactly as it did when the engine
//!   owned this port.
//! - **Nothing is listening there**: serve the request here, straight to the
//!   provider the slug names, under the tool's own credential. That is what the
//!   engine's relay does while parked, and what the tool would do with Gate not
//!   installed.
//! - **Something is listening there and does not prove itself**: refuse with a
//!   502. Going direct would send traffic around a Gate that may well be
//!   routing (an engine too busy to answer in time), and handing the request
//!   over would give a stranger the tool's own provider key.
//!
//! What the direct path is not allowed to be, and why each holds:
//!
//! - **Not an open proxy.** A request names one of
//!   [`gate_connect_paths::RELAY_UPSTREAMS`] or is refused, so the only hosts
//!   reachable are the providers Gate already knows.
//! - **Not a way to spend Gate's credential.** The forwarder has none. Every
//!   `x-gate-*` header is stripped, not only the ones the relay stamps.
//! - **Not a way to send a pay-as-you-go request without its credential.** A
//!   tool on Gate pay-as-you-go sends no provider key - Gate was going to
//!   supply the provider and the bill - so those slugs get a 503 that says to
//!   open Gate Connect, not a request the provider will refuse.
//! - **Not reachable from a web page.** The same `Host` / `Origin` loopback
//!   boundary the engine's relay applies, from the same definition.
//! - **Not handed to a stranger.** The engine side has to answer a proof on a
//!   path the forwarder itself never answers ([`RELAY_ENGINE_HEALTH_PATH`]), on
//!   the very connection the request then travels over.
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
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use gate_connect_paths::{
    FORWARDER_CHALLENGE_HEADER, FORWARDER_PROOF_HEADER, PAYG_ELIGIBLE_SLUGS,
    RELAY_ENGINE_HEALTH_PATH, RELAY_ENGINE_PORT_NAME, RELAY_FRONT_HEADER, RELAY_HEALTH_PATH,
    RELAY_INTERCEPTING_HEADER, RELAY_LIVENESS_PATH, RELAY_PORT_NAME, RELAY_TOOL_PATH_PREFIX,
};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::net::{TcpListener, TcpStream};

use crate::proxy::{
    EngineLookup, ENGINE_CONNECT_TIMEOUT, HEAD_READ_TIMEOUT, MAX_HEAD, MAX_HEADERS,
};

/// The relay port this forwarder holds, or 0 while it holds none. Reported on
/// the forwarder's own health answer ([`gate_connect_paths::FORWARDER_RELAY_HEADER`]),
/// which is how the app decides whether to bind the engine's relay behind this
/// listener or on the public port itself.
pub static HELD_PORT: AtomicU16 = AtomicU16::new(0);

/// How often to try again for the relay port while something else holds it,
/// and how often a held port is checked against the port file.
///
/// The ordinary holder is the app's own engine, on a session that started
/// before this forwarder did; it lets go when the app quits, and this is what
/// picks the port up so the tools pointed at it keep working. Short, because a
/// re-enable that finds the port just released waits for this forwarder to
/// take it; one loopback probe a second is what it costs.
const CLAIM_RETRY: Duration = Duration::from_secs(1);

/// How long the engine's relay gets to answer its proof. Generous: a live
/// listener that is ours is only slow when it is busy, and a slow proof is
/// refused rather than sent around Gate.
const ENGINE_PROOF_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to wait for the provider to accept a connection.
const UPSTREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a request body may take to arrive in full. Request bodies here are
/// JSON prompts; this bounds a client that declares a body and stops sending.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// How long the provider may take to start its answer. Long, because a
/// non-streaming completion only answers when it is done.
const RESPONSE_HEAD_TIMEOUT: Duration = Duration::from_secs(600);

/// How long to wait for the provider's answer once sending the request body
/// failed. Anything it answered early is already buffered by then.
const EARLY_RESPONSE_WAIT: Duration = Duration::from_secs(5);

/// The longest chunk-size or trailer line accepted in a chunked body.
const MAX_LINE: u64 = 8 * 1024;

/// Most trailer lines read after a chunked body's last chunk. They are
/// consumed and not forwarded.
const MAX_TRAILERS: usize = 64;

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

/// Whether the account is on Gate pay-as-you-go, read from `account.json`
/// per request so a switch made while the app is closed (the CLI's
/// `billing-mode`) is honoured at once.
///
/// Anything unreadable reads as not pay-as-you-go, which is the direction
/// core's `billing_mode_for_injection` fails in too: the own-key shape.
fn account_is_payg() -> bool {
    gate_connect_paths::app_support_dir()
        .ok()
        .and_then(|dir| std::fs::read(dir.join(gate_connect_paths::ACCOUNT_FILE_NAME)).ok())
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|account| account["billing_mode"] == "payg")
}

/// How a request's billing is looked up. Injectable so tests need no account
/// file.
pub type PaygLookup = Arc<dyn Fn() -> bool + Send + Sync>;

/// Take the relay port and serve it, retrying for as long as something else
/// holds it, and giving it up if the port file comes to name another.
///
/// The first attempt may take a fresh port, because on a first run there is no
/// persisted one and no config names any port yet. Every retry asks only for
/// the persisted port: a retry that took a fresh one would persist it, and the
/// next config write would move every tool off the port the running app is
/// serving.
///
/// Giving the port up is what recovers from an app session that could not
/// bind behind this listener and wrote its own relay port into the file
/// instead: the configs are then repointed at the engine's port, this listener
/// stops answering on a port nothing names, and it takes the engine's port
/// over the moment the app lets go of it.
pub fn start(forwarder_port: u16, token: Arc<str>) {
    let table: Arc<Vec<Upstream>> = Arc::new(upstreams());
    let backend: EngineLookup = Arc::new(|| gate_connect_paths::load_port(RELAY_ENGINE_PORT_NAME));
    let payg: PaygLookup = Arc::new(account_is_payg);
    // launchd's socket is collected before anything is served, so the health
    // answer never says "holds nothing" while launchd is already holding the
    // port for us - which an enable would read as the port being somebody
    // else's.
    let mut activated = crate::activated_socket("Relay");
    tokio::spawn(async move {
        let mut first = true;
        loop {
            let bound = match activated.take() {
                Some(listener) => Some(listener),
                None => {
                    let initial = first;
                    tokio::task::spawn_blocking(move || bind(initial, forwarder_port))
                        .await
                        .ok()
                        .flatten()
                }
            };
            first = false;
            if let Some((listener, port)) = bound.and_then(adopt) {
                // Recorded once held, so the file the app writes into tool
                // configs never names a port nothing answers on.
                let _ = gate_connect_paths::save_port(RELAY_PORT_NAME, port);
                HELD_PORT.store(port, Ordering::SeqCst);
                let released = async move {
                    loop {
                        tokio::time::sleep(CLAIM_RETRY).await;
                        let named = gate_connect_paths::load_port(RELAY_PORT_NAME);
                        if named.is_some_and(|named| named != port) {
                            return;
                        }
                    }
                };
                let services = Services {
                    backend: backend.clone(),
                    token: token.clone(),
                    table: table.clone(),
                    payg: payg.clone(),
                };
                serve(listener, port, services, released).await;
                HELD_PORT.store(0, Ordering::SeqCst);
            }
            tokio::time::sleep(CLAIM_RETRY).await;
        }
    });
}

fn adopt(std_listener: std::net::TcpListener) -> Option<(TcpListener, u16)> {
    std_listener.set_nonblocking(true).ok()?;
    let port = std_listener.local_addr().ok()?.port();
    Some((TcpListener::from_std(std_listener).ok()?, port))
}

/// One attempt at the relay port. See [`start`] for why only the first may
/// fall back to a fresh one.
fn bind(first: bool, forwarder_port: u16) -> Option<std::net::TcpListener> {
    match gate_connect_paths::load_port(RELAY_PORT_NAME) {
        // A live listener is the common reason to be retrying at all - the
        // app's engine, on a session that started before this forwarder - and
        // `bind_preferred` would spend its whole grace period probing it. One
        // probe answers the question for this tick.
        Some(port) if !first && gate_connect_paths::port_is_live(port) => None,
        Some(port) => gate_connect_paths::bind_preferred(port).ok(),
        None if first => {
            let mut skip = gate_connect_paths::remembered_ports_except(RELAY_PORT_NAME);
            skip.push(forwarder_port);
            gate_connect_paths::bind_fresh(&skip).ok()
        }
        None => None,
    }
}

/// What one relay connection needs besides the socket.
#[derive(Clone)]
pub struct Services {
    pub backend: EngineLookup,
    pub token: Arc<str>,
    pub table: Arc<Vec<Upstream>>,
    pub payg: PaygLookup,
}

/// Accept until `released` completes, serving each connection with
/// [`handle`]. Connections already accepted run to completion.
pub async fn serve(
    listener: TcpListener,
    own_port: u16,
    services: Services,
    released: impl std::future::Future<Output = ()>,
) {
    let slots = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    tokio::pin!(released);
    loop {
        let client = tokio::select! {
            _ = &mut released => return,
            accepted = listener.accept() => match accepted {
                Ok((client, _)) => client,
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            },
        };
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            continue;
        };
        let services = services.clone();
        tokio::spawn(async move {
            let _slot = slot;
            let _ = handle(client, own_port, services).await;
        });
    }
}

/// What the engine side of the relay turned out to be.
enum Engine {
    /// Gate's relay, proven on this connection, which is ready to carry the
    /// client's bytes.
    Proved(TcpStream),
    /// Nothing accepted a connection: the engine is gone.
    Absent,
    /// Something accepted and did not prove itself.
    Unproven,
}

/// Serve one connection: hand it to the engine's relay if that is up and
/// proves itself, answer it here if nothing is there, refuse it otherwise.
pub async fn handle(mut client: TcpStream, own_port: u16, services: Services) -> Result<()> {
    if let Some(port) = (services.backend)().filter(|p| *p != own_port) {
        match engine_session(port, &services.token).await {
            Engine::Proved(mut engine) => {
                tokio::io::copy_bidirectional(&mut client, &mut engine).await?;
                return Ok(());
            }
            Engine::Absent => {}
            Engine::Unproven => {
                respond(
                    &mut client,
                    502,
                    &format!(
                        "Gate Connect's relay on 127.0.0.1:{port} did not answer as Gate's. \
                         Retry, or restart Gate Connect."
                    ),
                )
                .await;
                return Ok(());
            }
        }
    }
    serve_direct(client, &services).await
}

/// Connect to the engine's relay and prove it on that same connection.
///
/// A refused or timed-out connect is the engine being gone. The timeout counts
/// as gone, not as unproven, because on Windows a connect to a closed loopback
/// port is not refused at once: the stack retries the SYN for about two
/// seconds, and a live listener's connect completes from the accept backlog
/// in microseconds.
///
/// The proof and the request share the connection, so there is no window
/// between the two in which the port could change hands. The engine's relay
/// speaks HTTP/1.1 with keep-alive, so the request that follows is served on
/// the connection the proof was answered on.
async fn engine_session(port: u16, token: &str) -> Engine {
    let connected = tokio::time::timeout(
        ENGINE_CONNECT_TIMEOUT,
        TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], port))),
    )
    .await;
    let mut engine = match connected {
        Ok(Ok(engine)) => engine,
        _ => return Engine::Absent,
    };
    let challenge = gate_connect_paths::fresh_challenge();
    let expected = gate_connect_paths::forwarder_proof(token, RELAY_ENGINE_HEALTH_PATH, &challenge);
    let proved = tokio::time::timeout(ENGINE_PROOF_TIMEOUT, async {
        engine
            .write_all(
                format!(
                    "GET {RELAY_ENGINE_HEALTH_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\n\
                     {FORWARDER_CHALLENGE_HEADER}: {challenge}\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .ok()?;
        let mut head = Vec::new();
        let end = read_head(&mut engine, &mut head, false).await.ok()??;
        // Nothing may follow the answer: the engine speaks only when asked, so
        // bytes past it would be bytes the client's request gets mixed with.
        if end != head.len() || !head.starts_with(b"HTTP/1.1 204") {
            return None;
        }
        gate_connect_paths::parse_proof(&head, &expected)
    })
    .await;
    match proved {
        Ok(Some(_)) => Engine::Proved(engine),
        _ => Engine::Unproven,
    }
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

/// Headers that address this hop and never go further: the RFC set both relays
/// share, plus `host`, which is rewritten, and `expect`, which is answered
/// here - see [`Plan::expect_continue`].
fn is_hop_by_hop(name: &str) -> bool {
    gate_connect_paths::HOP_BY_HOP
        .iter()
        .chain(["host", "expect"].iter())
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

/// Every value of the header `name`, raw.
fn values<'a>(headers: &'a [(String, Vec<u8>)], name: &str) -> Vec<&'a [u8]> {
    headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_slice())
        .collect()
}

/// The header `name` as text: `None` when absent, `Some(None)` when present and
/// not UTF-8.
fn text<'a>(headers: &'a [(String, Vec<u8>)], name: &str) -> Option<Option<&'a str>> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| std::str::from_utf8(v).ok())
}

/// Body framing from a head's headers, strictly: at most one
/// `Transfer-Encoding` and one `Content-Length`, a length of digits only, and a
/// transfer coding that ends in `chunked`. `Transfer-Encoding` wins over
/// `Content-Length`, as RFC 9112 says, and the caller drops the length so the
/// two cannot disagree further along. Anything looser is refused rather than
/// passed on for the next hop to read differently.
fn framing(headers: &[(String, Vec<u8>)]) -> Result<Framing, ()> {
    let te = values(headers, "transfer-encoding");
    let cl = values(headers, "content-length");
    if te.len() > 1 || cl.len() > 1 {
        return Err(());
    }
    if let Some(te) = te.first() {
        let te = std::str::from_utf8(te).map_err(|_| ())?;
        let last = te.rsplit(',').next().unwrap_or("").trim();
        return if last.eq_ignore_ascii_case("chunked") {
            Ok(Framing::Chunked)
        } else {
            Err(())
        };
    }
    match cl.first() {
        Some(v) => {
            let v = std::str::from_utf8(v).map_err(|_| ())?.trim();
            if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
                return Err(());
            }
            v.parse().map(Framing::Length).map_err(|_| ())
        }
        None => Ok(Framing::None),
    }
}

/// Decide where a relay request goes and rewrite its head for the provider.
///
/// Pure, so every refusal and rewrite is pinned without a socket.
pub fn plan(
    method: &str,
    target: &str,
    http_minor: u8,
    headers: &[(String, Vec<u8>)],
    table: &[Upstream],
) -> Result<Plan, Refusal> {
    // A `Host` or `Origin` that is present but unreadable is refused, as the
    // engine's relay refuses it: reading it as absent would skip the check.
    match text(headers, "host") {
        Some(Some(host)) if gate_connect_paths::authority_is_loopback(host) => {}
        None => {}
        _ => {
            return Err(refuse(
                403,
                "the Gate relay only serves requests addressed to 127.0.0.1/localhost",
            ))
        }
    }
    match text(headers, "origin") {
        Some(Some(origin)) if gate_connect_paths::origin_is_loopback(origin) => {}
        None => {}
        _ => {
            return Err(refuse(
                403,
                "the Gate relay does not serve cross-origin browser requests",
            ))
        }
    }
    if !target.starts_with('/') {
        return Err(refuse(
            400,
            "the Gate relay only serves origin-form requests",
        ));
    }
    if gate_connect_paths::has_dot_segment(target) {
        return Err(refuse(400, "request path contains a `.` or `..` segment"));
    }

    // The tool marker comes off first, and its segment goes with it whatever it
    // names: attribution is Gate's business, and there is no Gate on this path.
    let unmarked = match target.strip_prefix(RELAY_TOOL_PATH_PREFIX) {
        Some(rest) => gate_connect_paths::split_leading_segment(&format!("/{rest}"))
            .map(|(_, inner)| inner)
            .unwrap_or_else(|| target.to_string()),
        None => target.to_string(),
    };
    let by_slug =
        gate_connect_paths::split_leading_segment(&unmarked).and_then(|(segment, inner)| {
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
        None => match text(headers, "x-gate-upstream-url").flatten() {
            Some(named) => match table.iter().position(|u| u.url == named) {
                Some(i) => (i, unmarked.clone()),
                None => {
                    return Err(refuse(
                        403,
                        "the named upstream is not in the built-in catalog",
                    ))
                }
            },
            None => {
                return Err(refuse(
                    400,
                    "the request path does not start with a known upstream slug",
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

    // `100 Continue` is an HTTP/1.1 response; an HTTP/1.0 client never waits
    // for one.
    let expect_continue = http_minor >= 1
        && text(headers, "expect")
            .flatten()
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("100-continue"));
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
            Ok(Framing::None) | Err(()) => Framing::UntilClose,
            Ok(f) => f,
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
            // A length the body is not framed by would make the client read
            // it differently from this hop.
            || (matches!(framing, Framing::Chunked | Framing::UntilClose)
                && lower == "content-length")
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

/// Answer with a plain-text error, then close so the client reads it.
async fn respond(client: &mut TcpStream, status: u16, message: &str) {
    let reason = match status {
        400 => "Bad Request",
        403 => "Forbidden",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let body = message.as_bytes();
    let _ = client
        .write_all(
            format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\n\
                 X-Content-Type-Options: nosniff\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .await;
    let _ = client.write_all(body).await;
    linger(client).await;
}

/// The answer a pay-as-you-go tool gets while the app is closed, shaped so both
/// the OpenAI and the Anthropic SDKs show its message: `error.message` is where
/// each looks.
async fn respond_payg(client: &mut TcpStream) {
    let body = serde_json::json!({
        "type": "error",
        "error": {
            "type": "gate_connect_not_running",
            "code": "gate_connect_not_running",
            "message": "Gate Connect is not running. This tool is set up to use Gate \
                        pay-as-you-go, which works only while Gate Connect is open. Open Gate \
                        Connect and try again.",
        }
    })
    .to_string();
    let _ = client
        .write_all(
            format!(
                "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\n\
                 X-Content-Type-Options: nosniff\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await;
    linger(client).await;
}

/// Close gracefully after an answer written before the request was read in
/// full. Closing a socket that still has unread input sends a reset, and a
/// client mid-upload then sees ECONNRESET instead of the answer. So: stop
/// writing, and read and discard what the client is still sending, bounded in
/// time and size, before letting go.
async fn linger(client: &mut TcpStream) {
    let _ = client.shutdown().await;
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        let mut sink = [0u8; 8192];
        let mut drained = 0usize;
        while drained < 1024 * 1024 {
            match client.read(&mut sink).await {
                Ok(0) | Err(_) => break,
                Ok(n) => drained += n,
            }
        }
    })
    .await;
}

/// Answer one request with no engine behind this listener.
async fn serve_direct(mut client: TcpStream, services: &Services) -> Result<()> {
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
    let (method, target, minor, headers) = {
        let mut slots = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut req = httparse::Request::new(&mut slots);
        let _ = req.parse(&buf[..end]);
        (
            req.method.unwrap_or_default().to_string(),
            req.path.unwrap_or_default().to_string(),
            req.version.unwrap_or(1),
            req.headers
                .iter()
                .map(|h| (h.name.to_string(), h.value.to_vec()))
                .collect::<Vec<_>>(),
        )
    };
    let leftover = buf.split_off(end);

    // Identity first, as the engine's relay does it. `intercepting: 0` is the
    // truth about this path: nothing here reaches Gate, so the tools' status
    // reads it the way it reads a parked engine. The front header says it is
    // the forwarder answering, not an engine.
    if method == "GET" && target == RELAY_HEALTH_PATH {
        let challenge = text(&headers, FORWARDER_CHALLENGE_HEADER)
            .flatten()
            .unwrap_or_default();
        let proof =
            gate_connect_paths::forwarder_proof(&services.token, RELAY_HEALTH_PATH, challenge);
        let _ = client
            .write_all(
                format!(
                    "HTTP/1.1 204 No Content\r\n{FORWARDER_PROOF_HEADER}: {proof}\r\n\
                     {RELAY_INTERCEPTING_HEADER}: 0\r\n{RELAY_FRONT_HEADER}: forwarder\r\n\
                     Connection: close\r\n\r\n"
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

    let plan = match plan(&method, &target, minor, &headers, &services.table) {
        Ok(plan) => plan,
        Err(refusal) => {
            respond(&mut client, refusal.status, &refusal.message).await;
            return Ok(());
        }
    };
    let upstream = &services.table[plan.upstream];
    if PAYG_ELIGIBLE_SLUGS.contains(&upstream.slug.as_str()) && (services.payg)() {
        respond_payg(&mut client).await;
        return Ok(());
    }

    let tcp = match tokio::time::timeout(
        UPSTREAM_CONNECT_TIMEOUT,
        TcpStream::connect((upstream.host.as_str(), upstream.port)),
    )
    .await
    {
        Ok(Ok(tcp)) => tcp,
        _ => {
            let message = format!("could not reach {}", upstream.host_header());
            respond(&mut client, 502, &message).await;
            return Ok(());
        }
    };
    if !upstream.tls {
        return exchange(client, tcp, plan, leftover).await;
    }
    let tls = match tls_connect(&upstream.host, tcp).await {
        Ok(tls) => tls,
        Err(e) => {
            let message = format!("TLS to {} failed: {e:#}", upstream.host_header());
            respond(&mut client, 502, &message).await;
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
    let upload = {
        // Exactly the declared body and not a byte more. Anything the client
        // pipelined behind it is dropped with the connection, which is what
        // the `Connection: close` it gets back tells it to expect.
        let mut body = BufReader::new(Cursor::new(leftover).chain(&mut client));
        match tokio::time::timeout(
            UPLOAD_TIMEOUT,
            copy_body(&mut body, &mut upstream, plan.body),
        )
        .await
        {
            Ok(Ok(())) => upstream.flush().await.map_err(anyhow::Error::from),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(anyhow!("the request body did not arrive in time")),
        }
    };
    // A provider may answer before the body is in - a 401 or a 413 - and then
    // stop reading, which fails the upload. Its answer is what the client
    // needs, so it is still read; only briefly, since a body that failed on
    // the client's side leaves a provider that will never answer.
    let head_wait = if upload.is_ok() {
        RESPONSE_HEAD_TIMEOUT
    } else {
        EARLY_RESPONSE_WAIT
    };

    let mut buf = Vec::with_capacity(4096);
    let response = tokio::time::timeout(head_wait, async {
        loop {
            let end = read_head(&mut upstream, &mut buf, false).await.ok()??;
            let (head, status, framing) = rewrite_response(&buf[..end], plan.head_request)?;
            // Interim responses (103 Early Hints) are passed on and the real
            // one is read after them. 101 cannot happen: `Upgrade` is not
            // forwarded.
            if (100..200).contains(&status) {
                if client.write_all(&buf[..end]).await.is_err() {
                    return None;
                }
                buf.drain(..end);
                continue;
            }
            buf.drain(..end);
            return Some((head, framing));
        }
    })
    .await;
    let Ok(Some((head, framing))) = response else {
        let message = match upload {
            Err(e) => format!("sending the request failed: {e:#}"),
            Ok(()) => "the provider sent no usable response".to_string(),
        };
        respond(&mut client, 502, &message).await;
        return Ok(());
    };
    client.write_all(&head).await?;
    let relayed = {
        let mut body = BufReader::new(Cursor::new(buf).chain(&mut upstream));
        copy_body(&mut body, &mut client, framing).await
    };
    if relayed.is_err() && framing == Framing::UntilClose {
        // A close-delimited body that broke off would read as complete if this
        // side closed cleanly. A reset is how the client learns it was cut.
        let _ = client.set_zero_linger();
        return relayed;
    }
    relayed?;
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

/// Relay a chunked body, re-emitting its framing in canonical form: each size
/// line as bare hex, chunk extensions dropped, and no trailers. Read strictly -
/// CRLF line endings, hex digits only - so what this hop takes to be the end of
/// the body is the only reading the next hop can have.
async fn copy_chunked<R, W>(r: &mut R, w: &mut W) -> Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    loop {
        let line = read_line(r).await?;
        let size = parse_chunk_size(&line).context("malformed chunk size")?;
        w.write_all(format!("{size:x}\r\n").as_bytes()).await?;
        if size == 0 {
            // Trailers are read to find the end and not forwarded.
            for _ in 0..=MAX_TRAILERS {
                if read_line(r).await? == b"\r\n" {
                    w.write_all(b"\r\n").await?;
                    w.flush().await?;
                    return Ok(());
                }
            }
            bail!("too many trailer lines");
        }
        let copied = tokio::io::copy_buf(&mut (&mut *r).take(size), w).await?;
        if copied != size {
            bail!("chunk ended after {copied} of {size} bytes");
        }
        if read_line(r).await? != b"\r\n" {
            bail!("chunk not terminated by CRLF");
        }
        w.write_all(b"\r\n").await?;
        w.flush().await?;
    }
}

/// One CRLF-terminated line, bounded by [`MAX_LINE`].
async fn read_line<R: AsyncBufRead + Unpin>(r: &mut R) -> Result<Vec<u8>> {
    let mut line = Vec::new();
    (&mut *r)
        .take(MAX_LINE)
        .read_until(b'\n', &mut line)
        .await?;
    if !line.ends_with(b"\r\n") {
        bail!("chunked body line not terminated by CRLF");
    }
    Ok(line)
}

/// A chunk-size line: 1 to 16 hex digits, optional whitespace, optional
/// extensions after `;`.
fn parse_chunk_size(line: &[u8]) -> Option<u64> {
    let text = std::str::from_utf8(line.strip_suffix(b"\r\n")?).ok()?;
    let hex = text.split(';').next()?.trim_end_matches([' ', '\t']);
    if hex.is_empty() || hex.len() > 16 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(hex, 16).ok()
}

/// The TLS client configuration, built once it can be.
///
/// Only success is cached: a verifier that failed to build (a platform store
/// unreadable at login, say) is tried again on the next request rather than
/// answering 502 for as long as the forwarder runs, which on macOS is from
/// login onward.
///
/// The provider is named rather than taken from the process default: the
/// workspace compiles rustls with more than one, and with two available and
/// none installed, `ClientConfig::builder()` panics.
fn tls_config() -> Result<Arc<rustls::ClientConfig>> {
    static CONFIG: Mutex<Option<Arc<rustls::ClientConfig>>> = Mutex::new(None);
    let mut cached = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(config) = cached.as_ref() {
        return Ok(config.clone());
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let verifier = rustls_platform_verifier::Verifier::new(provider.clone())
        .context("building the certificate verifier")?;
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .context("choosing TLS versions")?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    // This hop speaks HTTP/1.1 and nothing else; offering h2 would let a
    // provider pick a protocol the bytes relayed here are not.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let config = Arc::new(config);
    *cached = Some(config.clone());
    Ok(config)
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
mod tests;
