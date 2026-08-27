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
//! The upshot for the design: **a tool's config carries one value and no
//! headers.** The base URL is `http://127.0.0.1:<port>/<slug><client-path>`,
//! where `<slug>` names the catalog domain; the relay reads it off the path,
//! strips it, and injects `x-gate-upstream-url` itself - the same thing the MITM
//! engine does from the CONNECT host. The credential lives in the keychain and is
//! injected here per request, so a token refresh is invisible to the tool and
//! rotating the key touches nothing on disk. Deriving the upstream from the
//! catalog rather than trusting the caller also means a local process cannot aim
//! the gateway at a host of its choosing.
//!
//! [`serve`] runs the same relay standalone (its own runtime, no MITM/CA/system
//! proxy) as a blocking headless host for environments with no menubar app -
//! containers, servers, CI. That is `proxy relay` on the CLI; `proxy enable`
//! hosts this relay as well as the MITM engine, so the two are alternatives
//! rather than steps - running both means two processes wanting the same
//! persisted relay port.

use std::convert::Infallible;
use std::sync::Arc;

use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::TryStreamExt;
use http::Uri;
use http_body_util::{combinators::BoxBody, BodyExt, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::header::{HeaderMap, HeaderName, HOST, ORIGIN};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::account::BillingMode;
use crate::proxy::{default_domains, ProxyDomain};

/// Where the stable relay port is persisted. CLI tool configs bake
/// `http://127.0.0.1:<port>`, so the port must survive restarts: the manager
/// reuses it as the engine's `preferred_relay_port` and only falls back to a
/// fresh band port if it's taken. All three platforms persist it, since every
/// platform's CLI configs need it.
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
    let path = crate::env::test_seam("GATE_CONNECT_TEST_CA")?;
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
    let url = crate::env::test_seam("GATE_CONNECT_TEST_UPSTREAM")?
        .to_string_lossy()
        .into_owned();
    Some(ProxyDomain {
        slug: "test-upstream".into(),
        display_name: "Test upstream".into(),
        hosts: Vec::new(),
        upstream_url: url,
        rewrite_prefixes: vec!["/v1/".into()],
        passthrough_prefixes: Vec::new(),
        rewrite_suffixes: Vec::new(),
        enabled: true,
        supported: true,
    })
}

/// Bind the relay's loopback port, reusing `preferred` (the persisted port) when
/// there is one.
///
/// A taken preferred port is an error rather than a fall back to a fresh
/// one. The fallback looks harmless and is not: the caller persists whatever
/// port it ends up with, so a second host started while the first is live
/// repoints the persisted port at itself, and the next `connect` bakes that
/// into every tool config - while the process actually serving traffic is the
/// other one, on the old port. Measured: a `proxy relay` run alongside the
/// desktop app moved the persisted port from 45981 to 44225 while the app kept
/// serving 45981. A freshly picked port is still right when there is no
/// preferred port (first run), where there is no baked URL to invalidate;
/// [`super::engine::bind_fresh`] takes it from a band outside the OS's
/// ephemeral range so the next run can actually rebind it.
///
/// [`super::engine::bind_preferred`] does the binding so this agrees with the
/// engine on what "taken" means: a live listener, not a TIME_WAIT remnant of a
/// host that just exited (which would otherwise refuse a legitimate restart).
fn bind_relay(preferred: Option<u16>) -> Result<(std::net::TcpListener, u16)> {
    let listener = match preferred {
        Some(p) => super::engine::bind_preferred(p).with_context(|| {
            format!(
                "the relay port {p} is already in use. Another relay host is likely running \
                 (`gate-connect proxy relay`, or the Gate app with the proxy enabled - it hosts \
                 this same relay). Stop that one first; tool configs point at this port, so \
                 moving to another would silently take them off the running host."
            )
        })?,
        None => super::engine::bind_fresh().context("binding relay loopback port")?,
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
    inject_gate_credential, GATE_AUTHORIZATION_HEADER, GATE_CLIENT_HEADER, GATE_INSTALL_ID_HEADER,
    GATE_KEY_HEADER, GATE_ORG_HEADER, UPSTREAM_URL_HEADER,
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
    /// Live billing mode. `Payg` drops the upstream hint and the tool's own
    /// credential on a rewrite, so the gateway bills the org's balance; `Byok`
    /// is today's shape. Resolved per domain - see
    /// [`effective_billing_mode`](super::effective_billing_mode).
    mode: watch::Receiver<BillingMode>,
    /// The built-in domain catalog. Used to (a) resolve the leading path segment
    /// of a request to a known upstream - so a local process can't aim the relay
    /// at an arbitrary host - and (b) classify the remaining path the way the
    /// MITM engine's `decide` does: inference rewrites to the gateway,
    /// everything else passes through to the real upstream.
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
        mode: watch::Receiver<BillingMode>,
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
            mode,
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
// The engine's live channels passed straight through to [`RelayState`]; bundling
// them into a struct would just restate that struct's fields at the one call
// site. Same reasoning as `helper_client::set_intercept`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn(
    std_listener: std::net::TcpListener,
    gateway: Uri,
    api_key: watch::Receiver<Arc<str>>,
    token: watch::Receiver<Arc<str>>,
    org: watch::Receiver<Arc<str>>,
    mode: watch::Receiver<BillingMode>,
    intercept: watch::Receiver<bool>,
    owner_uid: Option<u32>,
) -> Result<tokio::task::JoinHandle<()>> {
    let listener =
        TcpListener::from_std(std_listener).context("adopting relay loopback listener")?;
    let state = Arc::new(RelayState::new(
        &gateway, api_key, token, org, mode, intercept, owner_uid,
    ));
    Ok(tokio::spawn(accept_loop(listener, state)))
}

/// Accept connections forever, serving each on the relay handler. Shared by the
/// engine-hosted [`spawn`] and the standalone [`serve`].
async fn accept_loop(listener: TcpListener, state: Arc<RelayState>) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(pair) => pair,
            // Transient accept errors (ECONNABORTED, fd exhaustion) resolve
            // on their own; the pause keeps a *permanently* failing listener
            // from turning this loop into a silent 100% CPU spin for the
            // engine's lifetime.
            Err(_) => {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
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
    // An enabled proxy already hosts this relay, so a second host is never what
    // the user wants. `bind_relay` would catch it on the port, but only if the
    // engine's relay is on the port *this* process would pick; refusing up
    // front also names the cause, which "port in use" cannot. Checked before
    // the account load so the message doesn't depend on being signed in.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    if super::engine_likely_running() {
        let where_ = load_persisted_port()
            .map(|p| format!(" on {}", base_url(p)))
            .unwrap_or_default();
        anyhow::bail!(
            "the Gate proxy is enabled, and it already hosts this relay{where_}. \
             `proxy relay` is the alternative for machines with no app, not an addition to \
             it - point your tools at that URL, or run `gate-connect proxy disable` first."
        );
    }

    let account = crate::account::load()?
        .context("no Gate account configured - sign in before `proxy relay`")?;
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
        // Refreshed in the same loop as the org below: a headless host is
        // long-lived, and `gate-connect billing-mode` writes the account file
        // from a different process, so re-reading is the only way this host
        // learns of a switch.
        let (mode_tx, mode_rx) = watch::channel(account.billing_mode);
        // The standalone host always intercepts - routing through Gate is the
        // whole point of `proxy relay`, and its own loop below keeps the token
        // fresh. The sender lives for the whole (never-ending) block.
        let (_intercept_tx, intercept_rx) = watch::channel(true);

        let listener =
            TcpListener::from_std(std_listener).context("adopting relay loopback listener")?;
        // Only the user who launched `proxy relay` may spend its credential.
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
            mode_rx,
            intercept_rx,
            owner_uid,
        ));
        tokio::spawn(accept_loop(listener, state));

        // Keep `relay listening on <url>` as the first line, and the only one
        // carrying a URL: the e2e waits on that substring and scrapes the first
        // `http://` in the file as the relay base.
        println!("gate-connect relay listening on {}", base_url(port));
        // Say what this mode is *not*, because the name can't. Nothing here has
        // touched the CA or the system proxy, so a tool that wasn't pointed at
        // the relay is still talking to its own provider directly - and that is
        // indistinguishable from "routing is on" unless we spell it out.
        println!("  relay only: no CA installed, no system-proxy setting changed.");
        println!("  Routes only tools whose config points at the URL above.");
        println!(
            "  For config-less apps and machine-wide routing, use `gate-connect proxy enable`\n  \
             instead; it hosts this same relay, so the two are not meant to run together."
        );

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
            let _ = mode_tx.send(crate::account::billing_mode_for_injection());
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
    // Browser boundary, before anything is resolved or injected: the relay is
    // a plain-HTTP loopback responder, so a web page can drive it with no
    // local foothold - a "simple" cross-origin fetch to
    // `http://127.0.0.1:<port>` is delivered without a preflight (CORS only
    // blocks the *read*), and DNS rebinding delivers the same request under
    // an attacker hostname. Either way billed inference would run on the
    // owner's credential. A browser always names its target in `Host` and
    // stamps cross-site requests with `Origin`; the CLI tools this relay
    // serves dial 127.0.0.1 directly and send no `Origin`. See
    // `authority_is_loopback` / `origin_is_loopback` in the parent module.
    if let Some(host) = req.headers().get(HOST) {
        let ok = host
            .to_str()
            .map(super::authority_is_loopback)
            .unwrap_or(false);
        if !ok {
            return Err((
                StatusCode::FORBIDDEN,
                "the Gate relay only serves requests addressed to 127.0.0.1/localhost".into(),
            ));
        }
    }
    if let Some(origin) = req.headers().get(ORIGIN) {
        let ok = origin
            .to_str()
            .map(super::origin_is_loopback)
            .unwrap_or(false);
        if !ok {
            return Err((
                StatusCode::FORBIDDEN,
                "the Gate relay does not serve cross-origin browser requests".into(),
            ));
        }
    }

    let method = req.method().clone();
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());

    // Answered by the relay itself, ahead of catalog resolution: this is the
    // liveness check `routing_health` probes, so it must not need a catalog
    // entry, must not reach the gateway, and must not spend a token. Under the
    // reserved `/__gate/` prefix, which no catalog domain can claim, so it can
    // never shadow a real upstream path. GET only - a stray POST to this path
    // is a tool misconfigured, not a health check, and should fall through to
    // the resolver and get the usual error.
    if method == hyper::Method::GET && path_and_query == HEALTH_PATH {
        return Ok(health_response());
    }

    // Which upstream this request belongs to comes from the leading path
    // segment the tool's base URL carries, so no tool config has to hold a
    // header. Inference paths rewrite to the gateway under the Gate credential;
    // account/metadata paths (e.g. Claude Code's `/api/oauth/usage`) pass
    // through to the real upstream under the tool's own credential - mirroring
    // the MITM engine's `decide`. Without this the relay would funnel every
    // path to the gateway, which only serves inference and 404s the rest.
    let routed = resolve_route(&state.domains, &path_and_query, req.headers())?;
    // Not intercepting (Linux daemon, GUI gone): forward everything to the
    // real upstream under the tool's own credential, the same way the MITM
    // port blind-tunnels. The catalog resolution above still applies - direct
    // mode doesn't make the relay an open proxy.
    let route = if *state.intercept.borrow() {
        routed.route
    } else {
        Route::Passthrough
    };

    let mut headers = req.headers().clone();
    strip_hop_by_hop(&mut headers);
    headers.remove(HOST);
    let target = match route {
        Route::Rewrite => {
            let mode = super::effective_billing_mode(*state.mode.borrow(), &routed.slug);
            inject_credential(&mut headers, state, mode).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("injecting Gate credential: {e:#}"),
                )
            })?;
            // Forwarded: we set the upstream hint, overwriting anything the
            // caller sent. The value comes from the catalog entry we resolved,
            // so a local process can't aim the gateway at a host of its
            // choosing.
            //
            // Served: the hint's ABSENCE is the whole switch, so it is removed
            // instead - including anything the caller sent, which would
            // otherwise be a way for a local process to force a forward and
            // spend the tool's own credential.
            //
            // Two independent things ask Gate to serve, and either is enough.
            // The org routes this domain pay-as-you-go, so the gateway resolves
            // a provider and debits its balance. Or the user put this tool on a
            // Gate model, which is why a chosen model had no effect until this
            // branch existed: with the hint present the gateway forwards to the
            // tool's own provider and never reaches the override. That half is
            // read back from the header `inject_credential` has just stamped
            // rather than derived a second time - two computations of "is this
            // served?" could disagree, and the disagreement would be a request
            // billed one way and routed the other.
            if mode == BillingMode::Byok && !super::serves_gate_model(&headers) {
                set_upstream_header(&mut headers, &routed.upstream_url).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("building {UPSTREAM_URL_HEADER}: {e:#}"),
                    )
                })?;
            } else {
                headers.remove(UPSTREAM_URL_HEADER);
                // The tool's own key goes with it - on a served request the
                // model, the provider and the bill are all Gate's.
                // `inject_credential` has already done this for PAYG; this
                // covers the Gate-model case, where the org is still BYOK.
                super::strip_client_auth(&mut headers);
            }
            format!("{}{}", state.gateway_base, routed.path_and_query)
        }
        Route::Passthrough => {
            // Strip every Gate-internal header and forward under the tool's own
            // `Authorization`; never inject the Gate credential here.
            strip_gate_headers(&mut headers);
            format!("{}{}", routed.upstream_url, routed.path_and_query)
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
/// untouched; otherwise an OAuth token wins over the legacy key. In `Payg` the
/// same helper also strips the tool's own upstream credential.
fn inject_credential(headers: &mut HeaderMap, state: &RelayState, mode: BillingMode) -> Result<()> {
    // Clone the values out of the watch guards so no lock is held.
    let token: Arc<str> = state.token.borrow().clone();
    let api_key: Arc<str> = state.api_key.borrow().clone();
    let org: Arc<str> = state.org.borrow().clone();
    let oauth_token = (!token.is_empty()).then(|| token.as_ref());
    let org_id = (!org.is_empty()).then(|| org.as_ref());
    inject_gate_credential(headers, &api_key, oauth_token, org_id, mode)
}

/// Where a relayed request should go. The relay's analogue of the MITM
/// engine's [`Decision`](crate::proxy::Decision), but keyed by the leading path
/// segment of the tool's base URL instead of a CONNECT host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    /// Inference path: rewrite to the gateway with the Gate credential injected.
    Rewrite,
    /// Account/metadata path: forward to the real upstream under the tool's own
    /// credential.
    Passthrough,
}

/// A resolved relay request: which upstream owns it, the path to forward, and
/// whether that path rewrites to the gateway.
#[derive(Debug)]
struct Routed {
    /// The catalog upstream. Sent on as `x-gate-upstream-url` when rewriting
    /// under BYOK, and used as the base of the direct hop when passing through.
    upstream_url: String,
    /// Catalog slug that owns the request, so the caller can resolve the
    /// billing shape for it ([`effective_billing_mode`](super::effective_billing_mode)).
    slug: String,
    /// Path + query **relative to `upstream_url`** - our own slug segment
    /// removed. Both the gateway and the direct upstream append this to their
    /// own base, so it must not carry anything Gate-internal.
    path_and_query: String,
    route: Route,
}

/// Split `/<segment>/rest?query` into `("<segment>", "/rest?query")`, or `None`
/// when there is no leading segment. A path that ends at the segment becomes
/// `"/"`, and a query directly after it keeps a `/` in front so the forwarded
/// path stays absolute.
fn split_leading_segment(path_and_query: &str) -> Option<(&str, String)> {
    let rest = path_and_query.strip_prefix('/')?;
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

/// Resolve a relayed request against the catalog.
///
/// Primary scheme: the tool's base URL carries the catalog slug as its first
/// path segment (`http://127.0.0.1:<port>/anthropic/v1`), so the request itself
/// says which upstream it belongs to and the relay injects
/// `x-gate-upstream-url` from the catalog entry - exactly what the MITM engine
/// does from the CONNECT host. **No tool config has to carry a Gate header.**
///
/// Fallback: a config written before path encoding sends no slug and does carry
/// the header. That shape is still honored so an in-place upgrade keeps routing
/// until the reconcile pass rewrites the config. The header only *selects* a
/// catalog entry - the value forwarded is always the entry's own
/// `upstream_url` - so it cannot widen where the relay will forward.
///
/// `Err` carries the status to answer with: a caller that named an upstream we
/// don't serve is refused (403), while one that named nothing at all is a
/// malformed request (400).
fn resolve_route(
    domains: &[ProxyDomain],
    path_and_query: &str,
    headers: &HeaderMap,
) -> Result<Routed, (StatusCode, String)> {
    if let Some((segment, inner)) = split_leading_segment(path_and_query) {
        if let Some(d) = domains.iter().find(|d| d.slug == segment) {
            return Ok(Routed {
                upstream_url: d.upstream_url.clone(),
                slug: d.slug.clone(),
                route: classify(d, &inner),
                path_and_query: inner,
            });
        }
    }
    let Some(upstream) = headers
        .get(UPSTREAM_URL_HEADER)
        .and_then(|v| v.to_str().ok())
    else {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "{path_and_query:?} does not start with a known upstream slug, and no \
                 {UPSTREAM_URL_HEADER} header was supplied"
            ),
        ));
    };
    let d = domains
        .iter()
        .find(|d| d.upstream_url == upstream)
        .ok_or_else(|| {
            (
                StatusCode::FORBIDDEN,
                format!("upstream {upstream:?} is not in the built-in catalog"),
            )
        })?;
    Ok(Routed {
        upstream_url: d.upstream_url.clone(),
        slug: d.slug.clone(),
        route: classify(d, path_and_query),
        path_and_query: path_and_query.to_string(),
    })
}

/// Classify a path within one domain the way the MITM engine's `decide` does:
/// passthrough prefixes win, then inference rewrites; any other path on the
/// domain passes through.
fn classify(d: &ProxyDomain, path: &str) -> Route {
    if d.passthrough_prefixes
        .iter()
        .any(|p| path.starts_with(p.as_str()))
    {
        return Route::Passthrough;
    }
    if d.rewrite_prefixes
        .iter()
        .any(|p| path.starts_with(p.as_str()))
    {
        return Route::Rewrite;
    }
    Route::Passthrough
}

/// Set the upstream hint the gateway forwards on. Always overwrites, so a
/// caller-supplied value can never reach the gateway.
fn set_upstream_header(headers: &mut HeaderMap, upstream_url: &str) -> Result<()> {
    headers.insert(
        HeaderName::from_static(UPSTREAM_URL_HEADER),
        hyper::header::HeaderValue::from_str(upstream_url)
            .context("building the upstream hint header")?,
    );
    Ok(())
}

/// Strip every Gate-internal header before a passthrough hop, so none of them
/// leak to the real upstream. The tool's own `Authorization` is left untouched
/// so account endpoints (usage, profile) authenticate as the tool's identity.
fn strip_gate_headers(headers: &mut HeaderMap) {
    headers.remove(UPSTREAM_URL_HEADER);
    headers.remove(GATE_AUTHORIZATION_HEADER);
    headers.remove(GATE_KEY_HEADER);
    headers.remove(GATE_ORG_HEADER);
    // Attribution is for Gate's own activity view. A provider has no business
    // learning which machine or which tool this was, so it goes no further even
    // though the passthrough path never stamps it itself.
    headers.remove(GATE_INSTALL_ID_HEADER);
    headers.remove(GATE_CLIENT_HEADER);
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

/// Liveness path, served by the relay itself. Under a reserved prefix that the
/// domain catalog cannot name, so adding a real upstream can never collide with
/// it. Public so the prober and its tests spell it once.
pub const HEALTH_PATH: &str = "/__gate/health";

/// 204, no body. The prober only cares that something Gate-shaped answered on
/// the port; a body would invite callers to parse it into a richer contract than
/// this endpoint is willing to keep.
fn health_response() -> Response<BoxBody<Bytes, std::io::Error>> {
    let body = Full::new(Bytes::new())
        .map_err(|never| match never {})
        .boxed();
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(body)
        .expect("building relay health response")
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

    fn resolved(path: &str) -> Option<Routed> {
        resolve_route(&default_domains(), path, &HeaderMap::new()).ok()
    }

    #[test]
    fn routes_inference_to_gateway_and_account_paths_to_upstream() {
        // The leading segment names the catalog domain; everything after it is
        // what gets forwarded, so the slug never reaches the gateway.
        let r = resolved("/anthropic/v1/messages?beta=true").expect("anthropic slug resolves");
        assert_eq!(r.route, Route::Rewrite);
        assert_eq!(r.upstream_url, "https://api.anthropic.com");
        assert_eq!(r.path_and_query, "/v1/messages?beta=true");

        // Claude Code's usage/account calls pass through to the real upstream
        // rather than being funneled to the gateway, which only serves inference.
        let r = resolved("/anthropic/api/oauth/usage").expect("resolves");
        assert_eq!(r.route, Route::Passthrough);
        assert_eq!(r.path_and_query, "/api/oauth/usage");

        // An explicit passthrough prefix (the Squirrel updater) also passes
        // through, never rewritten.
        assert_eq!(
            resolved("/anthropic/api/desktop/RELEASES").unwrap().route,
            Route::Passthrough
        );

        // A slug that names no catalog domain is refused, so a local process
        // can't invent an upstream.
        assert!(resolved("/attacker/v1/messages").is_none());
        assert!(resolved("/").is_none());
    }

    #[test]
    fn routes_chatgpt_codex_responses_to_gateway() {
        // ChatGPT-subscription Codex points at `<relay>/chatgpt/codex` and
        // appends `/responses`. Stripping the slug has to leave exactly
        // `/codex/responses`, which is what the gateway concatenates onto
        // `https://chatgpt.com/backend-api`.
        let r = resolved("/chatgpt/codex/responses").expect("chatgpt slug resolves");
        assert_eq!(r.route, Route::Rewrite);
        assert_eq!(r.upstream_url, "https://chatgpt.com/backend-api");
        assert_eq!(r.path_and_query, "/codex/responses");
    }

    #[test]
    fn routes_openrouter_under_its_api_prefix() {
        // OpenRouter's `/api` rides in the upstream URL, not the forwarded path:
        // Gate's ALB diverts `/api/*` to the dashboard API, so a forwarded
        // `/api/v1/...` 404s before reaching the gateway proxy. Gate re-joins
        // upstream + path, so OpenRouter still sees /api/v1/chat/completions.
        let r = resolved("/openrouter/v1/chat/completions").expect("openrouter resolves");
        assert_eq!(r.route, Route::Rewrite);
        assert_eq!(r.upstream_url, "https://openrouter.ai/api");
        assert_eq!(r.path_and_query, "/v1/chat/completions");
    }

    #[test]
    fn a_bare_slug_forwards_the_root_path() {
        let r = resolved("/anthropic").expect("resolves");
        assert_eq!(r.path_and_query, "/");
        let r = resolved("/anthropic?x=1").expect("resolves");
        assert_eq!(r.path_and_query, "/?x=1");
    }

    #[test]
    fn falls_back_to_the_legacy_upstream_header() {
        // A config written before path encoding sends no slug and does carry the
        // header. Honored so an in-place upgrade keeps routing until the
        // reconcile pass rewrites the config.
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static(UPSTREAM_URL_HEADER),
            hyper::header::HeaderValue::from_static("https://api.anthropic.com"),
        );
        let r = resolve_route(&default_domains(), "/v1/messages", &headers)
            .expect("legacy header resolves");
        assert_eq!(r.route, Route::Rewrite);
        assert_eq!(r.upstream_url, "https://api.anthropic.com");
        // No slug to strip, so the path forwards unchanged.
        assert_eq!(r.path_and_query, "/v1/messages");

        // The header still only *selects* a catalog entry - an upstream outside
        // the catalog is refused just as it was before.
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static(UPSTREAM_URL_HEADER),
            hyper::header::HeaderValue::from_static("https://attacker.example"),
        );
        // Refused, not merely unrouted: naming an upstream we don't serve is a
        // 403, while naming nothing at all is a 400.
        let err = resolve_route(&default_domains(), "/v1/messages", &headers)
            .expect_err("off-catalog upstream must be refused");
        assert_eq!(err.0, StatusCode::FORBIDDEN);
        let err = resolve_route(&default_domains(), "/v1/messages", &HeaderMap::new())
            .expect_err("no slug and no header is malformed");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }
}
