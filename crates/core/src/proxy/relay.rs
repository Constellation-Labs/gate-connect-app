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
//! the watch-channel per request, and forwards to the gateway over TLS. When
//! interception is off (the Linux daemon with no GUI connected), it instead
//! forwards everything to the real upstream under the tool's own credential -
//! see [`RelayState::intercept`].
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
use hyper::header::{HeaderMap, HeaderName, HOST};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::proxy::{default_domains, ProxyDomain};

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

/// An extra catalog entry admitting a mock upstream, from
/// `GATE_CONNECT_TEST_UPSTREAM` (a base URL). Test seam for the hermetic e2e:
/// the built-in catalog pins real hosts, so without this the direct-forward
/// hop (interception off, or a passthrough path) could never be aimed at a
/// loopback mock. Unset in real builds. Classified with the standard `/v1/`
/// inference prefix so the same request rewrites to the gateway when
/// intercepting and forwards direct when not.
fn test_extra_upstream() -> Option<ProxyDomain> {
    let url = std::env::var("GATE_CONNECT_TEST_UPSTREAM").ok()?;
    Some(ProxyDomain {
        slug: "test-upstream".into(),
        display_name: "Test upstream".into(),
        hosts: Vec::new(),
        host_suffixes: Vec::new(),
        upstream_url: url,
        same_host_upstream: false,
        rewrite_prefixes: vec!["/v1/".into()],
        rewrite_suffixes: Vec::new(),
        passthrough_prefixes: Vec::new(),
        enabled: true,
        supported: true,
    })
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

// The Gate credential/upstream header names and the shared credential-
// injection rule live in the parent module so the relay and the MITM engine
// can't drift; this module just references them.
use super::{
    inject_gate_credential, GATE_AUTHORIZATION_HEADER, GATE_KEY_HEADER, GATE_ORG_HEADER,
    UPSTREAM_URL_HEADER,
};

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
    /// The built-in domain catalog. Used to (a) validate the tool-supplied
    /// `x-gate-upstream-url` against a known upstream - so a local process can't
    /// aim the relay at an arbitrary host - and (b) classify each path the way
    /// the MITM engine's `decide` does: inference (`/v1/`) rewrites to the
    /// gateway, everything else passes through to the real upstream.
    domains: Vec<ProxyDomain>,
    /// Whether inference rewrites to the gateway at all. When false (the Linux
    /// daemon with no GUI connected - see
    /// [`RunningEngine::set_relay_intercept`](super::engine::RunningEngine::set_relay_intercept)),
    /// every request forwards to the real upstream under the tool's own
    /// credential: the relay's analogue of the MITM port's blind tunnel.
    intercept: watch::Receiver<bool>,
    /// The UID allowed to spend the host's Gate credential, or `None` to allow
    /// any loopback peer. Set only where UIDs are resolvable (Linux); mirrors
    /// [`super::engine::EngineConfig::owner_uid`]. See [`RelayState::peer_allowed`].
    owner_uid: Option<u32>,
}

impl RelayState {
    fn new(
        gateway: &Uri,
        api_key: watch::Receiver<Arc<str>>,
        token: watch::Receiver<Arc<str>>,
        org: watch::Receiver<Arc<str>>,
        intercept: watch::Receiver<bool>,
        owner_uid: Option<u32>,
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
        let mut domains = default_domains();
        if let Some(d) = test_extra_upstream() {
            domains.push(d);
        }
        Self {
            client,
            gateway_base: format!("{scheme}://{authority}"),
            api_key,
            token,
            org,
            domains,
            intercept,
            owner_uid,
        }
    }

    /// Whether `peer` (a loopback connection's remote address) may spend the
    /// host's Gate credential. `true` when no owner restriction is set;
    /// otherwise the peer's resolved UID must equal the owner. Fails **closed**:
    /// an unresolvable UID is rejected rather than served, so we never hand the
    /// credential to an unverified local process. Mirrors the MITM engine's
    /// `peer_allowed`, except the relay drops the connection where the engine
    /// falls back to a blind tunnel.
    fn peer_allowed(&self, peer: std::net::SocketAddr) -> bool {
        match self.owner_uid {
            None => true,
            Some(owner) => super::engine::peer_uid_for(peer) == Some(owner),
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
    intercept: watch::Receiver<bool>,
    owner_uid: Option<u32>,
) -> Result<()> {
    let listener =
        TcpListener::from_std(std_listener).context("adopting relay loopback listener")?;
    let state = Arc::new(RelayState::new(
        &gateway, api_key, token, org, intercept, owner_uid,
    ));
    tokio::spawn(accept_loop(listener, state));
    Ok(())
}

/// Accept connections forever, serving each on the relay handler. Shared by the
/// engine-hosted [`spawn`] and the standalone [`serve`].
async fn accept_loop(listener: TcpListener, state: Arc<RelayState>) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(pair) => pair,
            Err(_) => continue,
        };
        // Only the owner may spend the host's Gate credential. Drop a non-owner
        // (or UID-unresolvable) peer before serving it - unlike the MITM engine,
        // which blind-tunnels, the relay has nowhere to forward without the
        // credential, so refusing the connection is the fail-closed action.
        if !state.peer_allowed(peer) {
            if super::engine::debug_log() {
                eprintln!("[gate-relay] refusing connection from non-owner peer {peer}");
            }
            continue;
        }
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
        // The standalone host always intercepts - routing through Gate is the
        // whole point of `proxy serve`, and its own loop below keeps the token
        // fresh. The sender lives for the whole (never-ending) block.
        let (_intercept_tx, intercept_rx) = watch::channel(true);

        let listener =
            TcpListener::from_std(std_listener).context("adopting relay loopback listener")?;
        // Only the user who launched `proxy serve` may spend its credential.
        // UID gating is Linux-only (see `engine::peer_uid_for`); elsewhere a
        // loopback peer's UID isn't resolvable, so we can't gate.
        #[cfg(target_os = "linux")]
        let owner_uid = Some(unsafe { libc::geteuid() });
        #[cfg(not(target_os = "linux"))]
        let owner_uid: Option<u32> = None;
        let state = Arc::new(RelayState::new(
            &gateway,
            key_rx,
            token_rx,
            org_rx,
            intercept_rx,
            owner_uid,
        ));
        tokio::spawn(accept_loop(listener, state));

        println!("gate-connect relay listening on {}", base_url(port));

        // Keep the OAuth token fresh (`access_token_for_injection` silently
        // refreshes a stale token) and pick up an org switch; block forever
        // hosting the relay.
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(
                crate::oauth::REFRESH_INTERVAL_SECS,
            ))
            .await;
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

    // The tool config sets the (non-secret) upstream hint; classify the request
    // against the catalog before we forward anything. Inference paths rewrite to
    // the gateway under the Gate credential; account/metadata paths (e.g. Claude
    // Code's `/api/oauth/usage`) pass through to the real upstream under the
    // tool's own credential - mirroring the MITM engine's `decide`. Without this
    // the relay funnels every path to the gateway, which only serves inference
    // and 404s the rest.
    let upstream = req
        .headers()
        .get(UPSTREAM_URL_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .ok_or((
            StatusCode::BAD_REQUEST,
            format!("missing {UPSTREAM_URL_HEADER} header"),
        ))?;
    let classified = route(&state.domains, &upstream, &path_and_query).ok_or((
        StatusCode::FORBIDDEN,
        format!("upstream {upstream:?} is not in the built-in catalog"),
    ))?;
    // Not intercepting (Linux daemon, GUI gone): forward everything to the
    // real upstream under the tool's own credential, the same way the MITM
    // port blind-tunnels. The catalog check above still applies - direct mode
    // doesn't make the relay an open proxy.
    let route = if *state.intercept.borrow() {
        classified
    } else {
        Route::Passthrough
    };

    let mut headers = req.headers().clone();
    strip_hop_by_hop(&mut headers);
    headers.remove(HOST);
    let target = match route {
        Route::Rewrite => {
            inject_credential(&mut headers, state).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("injecting Gate credential: {e:#}"),
                )
            })?;
            format!("{}{}", state.gateway_base, path_and_query)
        }
        Route::Passthrough => {
            // Strip every Gate-internal header and forward under the tool's own
            // `Authorization`; never inject the Gate credential here.
            strip_gate_headers(&mut headers);
            format!("{upstream}{path_and_query}")
        }
    };

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

/// Inject the live Gate credential, via the rule shared with the MITM engine
/// ([`inject_gate_credential`]): a caller-supplied `x-gate-api-key` is left
/// untouched; otherwise an OAuth token wins over the legacy key.
fn inject_credential(headers: &mut HeaderMap, state: &RelayState) -> Result<()> {
    // Clone the values out of the watch guards so no lock is held.
    let token: Arc<str> = state.token.borrow().clone();
    let api_key: Arc<str> = state.api_key.borrow().clone();
    let org: Arc<str> = state.org.borrow().clone();
    let oauth_token = (!token.is_empty()).then(|| token.as_ref());
    let org_id = (!org.is_empty()).then(|| org.as_ref());
    inject_gate_credential(headers, &api_key, oauth_token, org_id)
}

/// Where a relayed request should go. The relay's analogue of the MITM
/// engine's [`Decision`](crate::proxy::Decision), but keyed by the
/// tool-supplied `x-gate-upstream-url` hint instead of a CONNECT host.
enum Route {
    /// Inference path: rewrite to the gateway with the Gate credential injected.
    Rewrite,
    /// Account/metadata path: forward to the real upstream under the tool's own
    /// credential.
    Passthrough,
}

/// Classify a relayed request the way the MITM engine's `decide` does:
/// passthrough prefixes win, then inference (`/v1/`) rewrites; any other path on
/// a catalog upstream passes through. Returns `None` when `upstream` isn't in
/// the built-in catalog, so the caller refuses to forward it.
fn route(domains: &[ProxyDomain], upstream: &str, path: &str) -> Option<Route> {
    let d = domains.iter().find(|d| d.upstream_url == upstream)?;
    if d.passthrough_prefixes
        .iter()
        .any(|p| path.starts_with(p.as_str()))
    {
        return Some(Route::Passthrough);
    }
    if d.rewrite_prefixes
        .iter()
        .any(|p| path.starts_with(p.as_str()))
    {
        return Some(Route::Rewrite);
    }
    Some(Route::Passthrough)
}

/// Strip every Gate-internal header before a passthrough hop, so none of them
/// leak to the real upstream. The tool's own `Authorization` is left untouched
/// so account endpoints (usage, profile) authenticate as the tool's identity.
fn strip_gate_headers(headers: &mut HeaderMap) {
    headers.remove(UPSTREAM_URL_HEADER);
    headers.remove(GATE_AUTHORIZATION_HEADER);
    headers.remove(GATE_KEY_HEADER);
    headers.remove(GATE_ORG_HEADER);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_inference_to_gateway_and_account_paths_to_upstream() {
        let domains = default_domains();
        let anthropic = "https://api.anthropic.com";

        // Inference rewrites to the gateway.
        assert!(matches!(
            route(&domains, anthropic, "/v1/messages?beta=true"),
            Some(Route::Rewrite)
        ));
        // Claude Code's usage/account calls pass through to the real upstream -
        // the bug this fixes: they used to be funneled to the gateway and 404.
        assert!(matches!(
            route(&domains, anthropic, "/api/oauth/usage"),
            Some(Route::Passthrough)
        ));
        // An explicit passthrough prefix (the Squirrel updater) also passes
        // through, never rewritten.
        assert!(matches!(
            route(&domains, anthropic, "/api/desktop/RELEASES"),
            Some(Route::Passthrough)
        ));
        // An upstream outside the catalog is refused.
        assert!(route(&domains, "https://attacker.example", "/v1/messages").is_none());
    }

    #[test]
    fn routes_chatgpt_codex_responses_to_gateway() {
        // Regression: ChatGPT-subscription Codex writes
        // `X-Gate-Upstream-Url: https://chatgpt.com/backend-api` and hits the
        // relay at `/codex/responses`. The relay must recognize that upstream
        // (exact-match against the catalog) and classify the path as inference
        // so it rewrites to the gateway - otherwise the relay 403s the request
        // ("upstream ... is not in the built-in catalog").
        let domains = default_domains();
        let chatgpt = "https://chatgpt.com/backend-api";
        assert!(matches!(
            route(&domains, chatgpt, "/codex/responses"),
            Some(Route::Rewrite)
        ));
    }
}
