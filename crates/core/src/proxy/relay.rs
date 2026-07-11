//! Plaintext loopback reverse proxy for CLI tools (Claude Code, Codex,
//! OpenCode, ...). Hosted inside the engine's tokio runtime, alongside the
//! MITM forward proxy, and sharing its credential watch-channels.
//!
//! Why a *reverse* proxy and not the MITM engine: CLI tools all accept a
//! base-URL override, so we point them at `http://127.0.0.1:<port>` and they
//! send us ordinary origin-form requests. Because that hop is plaintext
//! loopback, this path needs **no CA and no elevation** - unlike the forward
//! MITM proxy, which terminates TLS with a trusted leaf. The relay reads the
//! tool's request, injects the *live* Gate credential (Cognito access token on
//! `x-gate-authorization`, or the legacy `x-gate-api-key`) pulled fresh from
//! the watch-channel per request, and forwards to the gateway over TLS.
//!
//! The upshot for the design: **no credential is ever written to a tool's
//! config file.** The config carries only the loopback base URL and the
//! non-secret `x-gate-upstream-url` hint; the secret lives in the keychain and
//! is injected here at request time, so a token refresh is invisible to the
//! tool and rotating the key touches nothing on disk.

use std::convert::Infallible;
use std::sync::Arc;

use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::TryStreamExt;
use http::Uri;
use http_body_util::{combinators::BoxBody, BodyExt, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::header::{HeaderMap, HeaderName, HeaderValue, HOST};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::proxy::default_domains;

/// Non-secret hint the tool config sets, telling the gateway which upstream to
/// forward to. The relay validates it against the built-in catalog and passes
/// it through untouched.
const UPSTREAM_URL_HEADER: &str = "x-gate-upstream-url";
/// Legacy credential header (Gate workspace key), injected when no OAuth token
/// is present.
const GATE_KEY_HEADER: &str = "x-gate-api-key";
/// OAuth credential header (Cognito access token), injected when a token is
/// present; takes precedence over the API key.
const GATE_AUTHORIZATION_HEADER: &str = "x-gate-authorization";

/// Everything a relay connection needs, shared across all requests.
struct RelayState {
    /// TLS client for the gateway hop. Redirects disabled - a proxy forwards
    /// verbatim and never chases a 3xx itself.
    client: reqwest::Client,
    /// `scheme://authority` of the gateway, no trailing slash. The tool's
    /// original path + query is appended per request.
    gateway_base: String,
    /// Live Gate API key (legacy fallback), hot-swapped by the manager.
    api_key: watch::Receiver<Arc<str>>,
    /// Live Cognito access token; empty string means "fall back to the key".
    token: watch::Receiver<Arc<str>>,
    /// Upstream URLs the built-in catalog permits. The relay refuses to forward
    /// a request whose `x-gate-upstream-url` isn't one of these, so a local
    /// process can't aim the gateway at an arbitrary host .
    allowed_upstreams: Vec<String>,
}

impl RelayState {
    fn new(
        gateway: &Uri,
        api_key: watch::Receiver<Arc<str>>,
        token: watch::Receiver<Arc<str>>,
    ) -> Self {
        let scheme = gateway.scheme_str().unwrap_or("https");
        let authority = gateway.authority().map(|a| a.as_str()).unwrap_or("");
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("building relay reqwest client");
        let allowed_upstreams = default_domains()
            .into_iter()
            .map(|d| d.upstream_url)
            .collect();
        Self {
            client,
            gateway_base: format!("{scheme}://{authority}"),
            api_key,
            token,
            allowed_upstreams,
        }
    }
}

/// Adopt a pre-bound loopback listener and start serving on the current tokio
/// runtime. The accept loop lives until the runtime is dropped (engine stop),
/// mirroring the PAC responder's lifetime.
pub(crate) fn spawn(
    std_listener: std::net::TcpListener,
    gateway: Uri,
    api_key: watch::Receiver<Arc<str>>,
    token: watch::Receiver<Arc<str>>,
) -> Result<()> {
    let listener =
        TcpListener::from_std(std_listener).context("adopting relay loopback listener")?;
    let state = Arc::new(RelayState::new(&gateway, api_key, token));
    tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => continue,
            };
            let state = Arc::clone(&state);
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let service = service_fn(move |req| {
                    let state = Arc::clone(&state);
                    async move { Ok::<_, Infallible>(handle(req, state).await) }
                });
                // http1 only: the CLI -> loopback hop is plaintext HTTP/1.1;
                // the gateway hop (reqwest) negotiates h2 on its own.
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .await;
            });
        }
    });
    Ok(())
}

/// Proxy one request, converting any failure into an HTTP error response so the
/// service future is infallible.
async fn handle(
    req: Request<Incoming>,
    state: Arc<RelayState>,
) -> Response<BoxBody<Bytes, std::io::Error>> {
    match proxy(req, &state).await {
        Ok(resp) => resp,
        Err((status, message)) => error_response(status, message),
    }
}

async fn proxy(
    req: Request<Incoming>,
    state: &RelayState,
) -> Result<Response<BoxBody<Bytes, std::io::Error>>, (StatusCode, String)> {
    let method = req.method().clone();
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());

    // The tool config sets the (non-secret) upstream hint; validate it against
    // the catalog before we forward anything under the Gate credential.
    let upstream = req
        .headers()
        .get(UPSTREAM_URL_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .ok_or((
            StatusCode::BAD_REQUEST,
            format!("missing {UPSTREAM_URL_HEADER} header"),
        ))?;
    if !state.allowed_upstreams.iter().any(|u| u == &upstream) {
        return Err((
            StatusCode::FORBIDDEN,
            format!("upstream {upstream:?} is not in the built-in catalog"),
        ));
    }

    let mut headers = req.headers().clone();
    strip_hop_by_hop(&mut headers);
    headers.remove(HOST);
    inject_credential(&mut headers, state);

    let body = req
        .into_body()
        .collect()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("reading request body: {e}"),
            )
        })?
        .to_bytes();

    let target = format!("{}{}", state.gateway_base, path_and_query);
    let upstream_resp = state
        .client
        .request(method, &target)
        .headers(headers)
        .body(body)
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("forwarding to gateway: {e}"),
            )
        })?;

    let mut builder = Response::builder().status(upstream_resp.status());
    if let Some(dst) = builder.headers_mut() {
        for (name, value) in upstream_resp.headers() {
            if is_hop_by_hop(name) {
                continue;
            }
            dst.append(name.clone(), value.clone());
        }
    }
    // Stream the response through so token-by-token SSE isn't buffered.
    let stream = upstream_resp
        .bytes_stream()
        .map_ok(Frame::data)
        .map_err(std::io::Error::other);
    let body = StreamBody::new(stream).boxed();
    builder.body(body).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("building response: {e}"),
        )
    })
}

/// Overwrite any client-supplied Gate credential headers with the live one.
/// Precedence mirrors the MITM engine's `apply_rewrite`: an OAuth token wins
/// and the API key is dropped; otherwise the legacy key is injected.
fn inject_credential(headers: &mut HeaderMap, state: &RelayState) {
    // Clone the values out of the watch guards so no lock is held.
    let token: Arc<str> = state.token.borrow().clone();
    let api_key: Arc<str> = state.api_key.borrow().clone();
    headers.remove(GATE_AUTHORIZATION_HEADER);
    headers.remove(GATE_KEY_HEADER);
    if !token.is_empty() {
        if let Ok(value) = HeaderValue::from_str(&format!("Bearer {token}")) {
            headers.insert(HeaderName::from_static(GATE_AUTHORIZATION_HEADER), value);
        }
    } else if let Ok(value) = HeaderValue::from_str(&api_key) {
        headers.insert(HeaderName::from_static(GATE_KEY_HEADER), value);
    }
}

/// Hop-by-hop headers must not be forwarded end-to-end (RFC 9110 §7.6.1).
fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "content-length"
    )
}

fn strip_hop_by_hop(headers: &mut HeaderMap) {
    let names: Vec<HeaderName> = headers
        .keys()
        .filter(|n| is_hop_by_hop(n))
        .cloned()
        .collect();
    for name in names {
        headers.remove(&name);
    }
}

fn error_response(status: StatusCode, message: String) -> Response<BoxBody<Bytes, std::io::Error>> {
    let body = Full::new(Bytes::from(message))
        .map_err(|never| match never {})
        .boxed();
    Response::builder()
        .status(status)
        .body(body)
        .expect("building relay error response")
}
