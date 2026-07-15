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
//!
//! [`serve`] runs the same relay standalone (its own runtime, no MITM/CA/system
//! proxy) as a blocking headless host for environments with no menubar app -
//! containers, servers, CI.

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

/// Where the stable relay port is persisted. CLI tool configs bake
/// `http://127.0.0.1:<port>`, so the port must survive restarts: the manager
/// reuses it as the engine's `preferred_relay_port` and only falls back to a
/// fresh ephemeral port if it's taken. Cross-platform (unlike the MITM port,
/// which only Linux persists), since every platform's CLI configs need it.
pub(crate) fn port_path() -> Result<std::path::PathBuf> {
    Ok(crate::env::app_support_dir()?
        .join("proxy")
        .join("relay-port"))
}

/// The last relay port we persisted, if any and still parseable.
pub(crate) fn load_persisted_port() -> Option<u16> {
    let path = port_path().ok()?;
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse::<u16>()
        .ok()
}

/// Persist the relay port for reuse on the next run. Best-effort durability;
/// non-secret, so written 0644.
pub(crate) fn save_persisted_port(port: u16) -> Result<()> {
    let path = port_path()?;
    crate::primitives::write_file(&path, port.to_string().as_bytes(), 0o644)
        .with_context(|| format!("writing {}", path.display()))
}

/// The loopback base URL a CLI tool points at to route through the relay.
pub(crate) fn base_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// An extra CA to trust on the gateway hop, from `GATE_CONNECT_TEST_CA` (a PEM
/// file path). Test seam for the e2e's self-signed mock gateway; unset in real
/// builds, where the default roots cover the gateway's public cert.
fn test_extra_ca() -> Option<reqwest::Certificate> {
    let path = std::env::var_os("GATE_CONNECT_TEST_CA")?;
    let pem = std::fs::read(path).ok()?;
    reqwest::Certificate::from_pem(&pem).ok()
}

/// Bind the relay's loopback listener, reusing `preferred` (the persisted port)
/// when free, else an ephemeral port. Non-blocking so tokio can adopt it.
fn bind_relay(preferred: Option<u16>) -> Result<(std::net::TcpListener, u16)> {
    let listener = match preferred {
        Some(p) => std::net::TcpListener::bind(("127.0.0.1", p))
            .or_else(|_| std::net::TcpListener::bind(("127.0.0.1", 0)))
            .context("binding relay loopback port")?,
        None => {
            std::net::TcpListener::bind(("127.0.0.1", 0)).context("binding relay loopback port")?
        }
    };
    let port = listener
        .local_addr()
        .context("reading relay listener address")?
        .port();
    listener
        .set_nonblocking(true)
        .context("setting relay listener non-blocking")?;
    Ok((listener, port))
}

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
/// Selected-org header, injected alongside the OAuth token (the gateway
/// requires it on every OAuth request).
const GATE_ORG_HEADER: &str = "x-gate-org-id";

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
    /// Live selected org UUID; empty means "none selected". Injected only when
    /// a token is present.
    org: watch::Receiver<Arc<str>>,
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
        org: watch::Receiver<Arc<str>>,
    ) -> Self {
        let scheme = gateway.scheme_str().unwrap_or("https");
        let authority = gateway.authority().map(|a| a.as_str()).unwrap_or("");
        // `.no_proxy()`: the relay IS a proxy - its gateway hop must go direct,
        // never back through the app's own system proxy (which would loop
        // relay -> engine -> gateway). Ignores any `HTTP(S)_PROXY` in the env.
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none());
        // Test seam (mirrors the other `GATE_CONNECT_TEST_*` seams): trust an
        // extra CA on the gateway hop so the e2e's self-signed mock gateway
        // validates. Unset in real builds, where the default roots cover the
        // real gateway's public cert. `tls_certs_only` verifies against a pure
        // webpki store holding only this CA (dropping the built-in roots) -
        // otherwise reqwest's default (rustls-platform-verifier) routes through
        // macOS Security.framework, whose stricter policy rejects the self-signed
        // mock cert even as an added root (the relay hop 502s on macOS only).
        if let Some(ca) = test_extra_ca() {
            builder = builder.tls_certs_only([ca]);
        }
        let client = builder.build().expect("building relay reqwest client");
        let allowed_upstreams = default_domains()
            .into_iter()
            .map(|d| d.upstream_url)
            .collect();
        Self {
            client,
            gateway_base: format!("{scheme}://{authority}"),
            api_key,
            token,
            org,
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
    org: watch::Receiver<Arc<str>>,
) -> Result<()> {
    let listener =
        TcpListener::from_std(std_listener).context("adopting relay loopback listener")?;
    let state = Arc::new(RelayState::new(&gateway, api_key, token, org));
    tokio::spawn(accept_loop(listener, state));
    Ok(())
}

/// Accept connections forever, serving each on the relay handler. Shared by the
/// engine-hosted [`spawn`] and the standalone [`serve`].
async fn accept_loop(listener: TcpListener, state: Arc<RelayState>) {
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
}

/// Run ONLY the reverse-proxy relay - no MITM, no CA trust, no system-proxy
/// changes - on the stable loopback port, and block until the process is
/// killed. This is the headless routing host for CLI tools where there's no
/// menubar app (a container, a server, CI): it seeds the current account's
/// credential + OAuth token + org, keeps the token fresh in the background, and
/// serves the relay so tools pointed at `http://127.0.0.1:<port>` route through
/// Gate. Never returns `Ok` while serving.
pub fn serve() -> Result<()> {
    let account = crate::account::load()?
        .context("no Gate account configured - sign in before `proxy serve`")?;
    let gateway: Uri = account
        .gateway_base_url
        .parse()
        .with_context(|| format!("parsing gateway URL {:?}", account.gateway_base_url))?;

    let (std_listener, port) = bind_relay(load_persisted_port())?;
    let _ = save_persisted_port(port);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building relay runtime")?;
    rt.block_on(async move {
        // Seed the credential channels from what's on disk right now. The key
        // rarely changes in a headless host, so it's seeded once; the token and
        // org are refreshed in the loop below.
        let (_key_tx, key_rx) = watch::channel::<Arc<str>>(Arc::from(account.api_key.as_str()));
        let (token_tx, token_rx) = watch::channel::<Arc<str>>(Arc::from(
            crate::oauth::access_token_for_injection().as_str(),
        ));
        let (org_tx, org_rx) =
            watch::channel::<Arc<str>>(Arc::from(crate::account::org_id_for_injection().as_str()));

        let listener =
            TcpListener::from_std(std_listener).context("adopting relay loopback listener")?;
        let state = Arc::new(RelayState::new(&gateway, key_rx, token_rx, org_rx));
        tokio::spawn(accept_loop(listener, state));

        println!("gate-connect relay listening on {}", base_url(port));

        // Keep the OAuth token fresh (silent refresh) and pick up an org switch;
        // block forever hosting the relay.
        let cfg = crate::oauth::OAuthConfig::from_build_env();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(
                crate::oauth::REFRESH_INTERVAL_SECS,
            ))
            .await;
            if let Some(cfg) = cfg.as_ref() {
                let _ = crate::oauth::ensure_fresh(cfg);
            }
            let _ = token_tx.send(Arc::from(
                crate::oauth::access_token_for_injection().as_str(),
            ));
            let _ = org_tx.send(Arc::from(crate::account::org_id_for_injection().as_str()));
        }
    })
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
    let org: Arc<str> = state.org.borrow().clone();
    headers.remove(GATE_AUTHORIZATION_HEADER);
    headers.remove(GATE_KEY_HEADER);
    headers.remove(GATE_ORG_HEADER);
    if !token.is_empty() {
        if let Ok(value) = HeaderValue::from_str(&format!("Bearer {token}")) {
            headers.insert(HeaderName::from_static(GATE_AUTHORIZATION_HEADER), value);
        }
        // The gateway requires the org header on every OAuth request.
        if !org.is_empty() {
            if let Ok(value) = HeaderValue::from_str(&org) {
                headers.insert(HeaderName::from_static(GATE_ORG_HEADER), value);
            }
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
