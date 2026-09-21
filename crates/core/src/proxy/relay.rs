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
//! headers.** The base URL is
//! `http://127.0.0.1:<port>/__gate/t/<tool>/<slug><client-path>`, where
//! `<slug>` names the catalog domain and `<tool>` names the integration that
//! wrote the URL; the relay reads both off the path, strips them, and injects
//! `x-gate-upstream-url` and `x-gate-client` itself - the same thing the MITM
//! engine does from the CONNECT host, except that the engine has no URL to read
//! and must guess the tool from the `User-Agent`. See [`TOOL_PATH_PREFIX`] for
//! why the tool segment is worth a path segment. The credential lives in the
//! keychain and is injected here per request, so a token refresh is invisible to
//! the tool and
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

use std::borrow::Cow;
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
        // A test seam, never a ledger row: nothing groups it, and it must not
        // ride a family switch if something ever does enumerate it. `AnyApp`
        // plus `Observed` is the inert answer; `Client` is the relay's own hop,
        // which touches no other program.
        client: crate::taxonomy::Client::AnyApp,
        credential: crate::taxonomy::Credential::Observed,
        scope: crate::taxonomy::Scope::Client,
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
    inject_gate_credential, GATE_AUTHORIZATION_HEADER, GATE_CLIENT_HEADER, GATE_DEVICE_NAME_HEADER,
    GATE_INSTALL_ID_HEADER, GATE_KEY_HEADER, GATE_MODEL_HEADER, GATE_ORG_HEADER,
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
    /// [`RunningEngine::set_intercept`](super::engine::RunningEngine::set_intercept)),
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
    // Prove we are Gate's relay, before anything else looks at this request.
    //
    // A bare TCP connect cannot tell this listener from any other process that
    // happens to accept on the port, and a status check that cannot tell them
    // apart reports a stranger as a healthy relay. Only a process that can read
    // the 0600 token can answer, so this identifies without authorising: it
    // reaches no credential, resolves no route and returns no traffic. Placed
    // above the loopback guards deliberately - it is cheaper than they are and
    // a prober that cannot get an answer has no way to tell a squatted port
    // from a refused one.
    if req.method() == hyper::Method::GET
        && req.uri().path() == gate_connect_paths::RELAY_HEALTH_PATH
    {
        let challenge = req
            .headers()
            .get(gate_connect_paths::FORWARDER_CHALLENGE_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        // Minted here when this install has none: the prober creates it too, so
        // whichever runs first wins and the other reads what it wrote. A
        // failure to read it is answered as "not ours", which is the safe
        // direction - a relay that cannot prove itself must not be trusted.
        let proof = super::forwarder::load_or_create_token()
            .ok()
            .map(|token| gate_connect_paths::forwarder_proof(&token, &challenge));
        let mut builder = Response::builder().status(StatusCode::NO_CONTENT);
        if let Some(proof) = proof {
            builder = builder.header(gate_connect_paths::FORWARDER_PROOF_HEADER, proof);
        }
        // What this relay is actually doing, which is the thing a status check
        // wants and could not otherwise learn: parked and routing look
        // identical from outside, so the tools were reading the user's stored
        // intent instead - a preference standing in for a measurement.
        builder = builder.header(
            gate_connect_paths::RELAY_INTERCEPTING_HEADER,
            if *state.intercept.borrow() { "1" } else { "0" },
        );
        return Ok(builder
            .body(
                Full::new(Bytes::new())
                    .map_err(|never| match never {})
                    .boxed(),
            )
            .expect("building relay health response"));
    }
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
            inject_credential(&mut headers, state, mode, &routed.slug, routed.tool).map_err(
                |e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("injecting Gate credential: {e:#}"),
                    )
                },
            )?;
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
            //
            // The Gate-model half also turns on the PATH: Gate can only answer
            // on a route it implements, and withholding the hint on any other
            // leaves the gateway with nothing to do and the caller waiting. PAYG
            // is not gated that way - the org routes that domain and its
            // forwarded path is already a shape the gateway serves. See
            // `serve_path`.
            let (req_path, req_query) = routed
                .path_and_query
                .split_once('?')
                .map_or((routed.path_and_query.as_str(), None), |(p, q)| {
                    (p, Some(q))
                });
            let model_serve_path = if super::serves_gate_model(&headers) {
                super::serve_path(req_path)
            } else {
                None
            };
            if mode == BillingMode::Byok && model_serve_path.is_none() {
                set_upstream_header(&mut headers, &routed.upstream_url).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("building {UPSTREAM_URL_HEADER}: {e:#}"),
                    )
                })?;
                // The model header goes too. It is not a label: its own contract says it
                // CHANGES WHAT THE GATEWAY SERVES, and it is sent only when the user put
                // this tool on a Gate model. Leaving it on a forwarded request states
                // both "Gate serves this, bill the org" and "send this to my own
                // provider under my own key" at once, and the body's model would be
                // rewritten to a Gate id the tool's own provider has never heard of.
                // Unreachable before the serve rewrite existed, because the request hung
                // instead of falling back; reachable now on any path Gate does not
                // serve, such as `count_tokens`.
                headers.remove(GATE_MODEL_HEADER);
                format!("{}{}", state.gateway_base, routed.path_and_query)
            } else {
                headers.remove(UPSTREAM_URL_HEADER);
                // The tool's own key goes with it - on a served request the
                // model, the provider and the bill are all Gate's.
                // `inject_credential` has already done this for PAYG; this
                // covers the Gate-model case, where the org is still BYOK.
                super::strip_client_auth(&mut headers);
                // Onto the path that can answer, which is not always the one the
                // tool asked on: Codex's `/codex/responses` is served at
                // `/v1/responses`, the same wire format under a route the
                // gateway implements. A PAYG request with no model override
                // keeps the path it arrived on.
                match model_serve_path {
                    Some(gateway_path) => match req_query {
                        Some(q) => format!("{}{gateway_path}?{q}", state.gateway_base),
                        None => format!("{}{gateway_path}", state.gateway_base),
                    },
                    None => format!("{}{}", state.gateway_base, routed.path_and_query),
                }
            }
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
fn inject_credential(
    headers: &mut HeaderMap,
    state: &RelayState,
    mode: BillingMode,
    domain: &str,
    tool: Option<&'static str>,
) -> Result<()> {
    // Clone the values out of the watch guards so no lock is held.
    let token: Arc<str> = state.token.borrow().clone();
    let api_key: Arc<str> = state.api_key.borrow().clone();
    let org: Arc<str> = state.org.borrow().clone();
    let oauth_token = (!token.is_empty()).then(|| token.as_ref());
    let org_id = (!org.is_empty()).then(|| org.as_ref());
    // The relay has no response hook to feed, so what was injected is not
    // news here.
    inject_gate_credential(
        headers,
        &api_key,
        oauth_token,
        org_id,
        mode,
        Some(domain),
        tool,
    )
    .map(|_| ())
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
    /// The tool named by the base URL's [`TOOL_PATH_PREFIX`] marker, when it
    /// carried one and the slug is a tool we know. `None` for a base URL
    /// written before the marker existed, or one hand-edited to name something
    /// else - attribution then falls back to the `User-Agent` guess, which is
    /// what every relay-routed request used before this.
    tool: Option<&'static str>,
}

/// Peel the [`TOOL_PATH_PREFIX`] marker, returning the tool it names and the
/// path with the marker gone. The unmarked path - every request that predates
/// the marker, and every proxy-routed one - is returned borrowed.
///
/// An unrecognised tool slug is dropped rather than refused, and the segment is
/// still removed so the catalog lookup behind it succeeds. That asymmetry is
/// deliberate and matches `inject_attribution`: a request whose tool we cannot
/// name is worth serving unlabelled, and failing it would be trading the user's
/// actual work for a data point on a chart.
///
/// `env-proxy` is dropped along with the unrecognised ones even though
/// [`crate::registry::ToolId`] accepts it: it is the environment channel rather
/// than a program, it has no [`crate::taxonomy::Client`] slug, and stamping it
/// would put a value in `x-gate-client` that the ledger's own vocabulary has no
/// name for. Nothing writes such a URL; this keeps the set we accept equal to
/// the set we write.
fn split_tool_segment(path_and_query: &str) -> (Option<&'static str>, Cow<'_, str>) {
    let Some(rest) = path_and_query.strip_prefix(TOOL_PATH_PREFIX) else {
        return (None, Cow::Borrowed(path_and_query));
    };
    // `rest` is `<tool>/<catalog-slug>...`; the leading `/` goes back on so the
    // segment splitter below sees the shape it documents.
    let with_slash = format!("/{rest}");
    let Some((segment, inner)) = split_leading_segment(&with_slash) else {
        return (None, Cow::Borrowed(path_and_query));
    };
    let tool = crate::registry::ToolId::from_slug(segment)
        .filter(|id| *id != crate::registry::ToolId::EnvProxy)
        .map(crate::registry::ToolId::slug);
    (tool, Cow::Owned(inner))
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
/// don't serve is refused (403), while one that named nothing at all, or one
/// that hid a dot segment in its path, is a malformed request (400).
fn resolve_route(
    domains: &[ProxyDomain],
    path_and_query: &str,
    headers: &HeaderMap,
) -> Result<Routed, (StatusCode, String)> {
    // The path we classify has to be the path we send, and it is not if a dot
    // segment survives to the URL parser - see [`has_dot_segment`]. Checked on
    // the raw request target, before the marker comes off, because a dot segment
    // anywhere in it changes where the concatenated URL lands.
    if has_dot_segment(path_and_query) {
        return Err((
            StatusCode::BAD_REQUEST,
            "request path contains a `.` or `..` segment".to_string(),
        ));
    }
    // The tool marker sits in front of the catalog slug, so it comes off first
    // and everything below sees the path it always saw. `original` is kept for
    // the error text: the caller typed that, not the stripped version.
    let original = path_and_query;
    let (tool, path_and_query) = split_tool_segment(path_and_query);
    let path_and_query = path_and_query.as_ref();
    if let Some((segment, inner)) = split_leading_segment(path_and_query) {
        if let Some(d) = domains.iter().find(|d| d.slug == segment) {
            return Ok(Routed {
                upstream_url: d.upstream_url.clone(),
                slug: d.slug.clone(),
                route: classify(d, &inner),
                path_and_query: inner,
                tool,
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
                "{original:?} does not start with a known upstream slug, and no \
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
        tool,
    })
}

/// Does the request target carry a `.` or `..` path segment?
///
/// It matters because [`classify`] decides Rewrite vs Passthrough by
/// `starts_with` on the raw path, while the URL we actually send is built by
/// concatenation and handed to `reqwest`, whose `Url::parse` collapses dot
/// segments per the WHATWG rules. Those two readings disagree:
/// `/anthropic/v1/../../x` classifies as Rewrite - it starts with the `/v1/`
/// prefix - gets the live Gate credential injected, and is then sent to
/// `<gateway>/x`, a path `classify` would never have credentialed. Rejecting is
/// preferred over normalizing because it keeps one string all the way through
/// rather than adding a second one to keep in step.
///
/// The encoded spellings count too: the URL parser treats `%2e` as a dot when it
/// looks for these segments, so a check that only matched the literal form would
/// be the same bug with an extra step.
fn has_dot_segment(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(p, _)| p)
        .unwrap_or(path_and_query);
    path.split('/').any(|segment| {
        [".", "%2e", "..", ".%2e", "%2e.", "%2e%2e"]
            .iter()
            .any(|form| segment.eq_ignore_ascii_case(form))
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
    // Same argument, and now load-bearing rather than tidy: `x-gate-model`
    // rewrites the served model and decides what the user is billed for
    // (`client_tool`'s doc has the note), so the strip list has to name it. The
    // passthrough arm never stamps either of these, so the only value that can
    // be here is one the caller sent - which is exactly the one to drop.
    headers.remove(GATE_MODEL_HEADER);
    headers.remove(GATE_DEVICE_NAME_HEADER);
    // Relayed tools are named by the path marker, so this header has no job
    // here - but a tool configured for both routes could still send it, and a
    // passthrough hop is the one place a Gate-internal header would reach the
    // real provider.
    headers.remove(super::GATE_TOOL_HEADER);
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

/// Marker a relay base URL carries ahead of the catalog slug to name the tool
/// that was configured with it, e.g.
/// `http://127.0.0.1:PORT/__gate/t/opencode/anthropic/v1`.
///
/// **This is what makes per-tool attribution structural rather than a guess.**
/// The alternative signal is the request's own `User-Agent`, which Gate neither
/// controls nor versions: it identifies a tool only for as long as that tool
/// keeps spelling itself the same way, and a prefix arriving in front of the
/// token is enough to lose it. Gate Connect writes this base URL itself, from
/// inside the integration that knows which tool it is configuring, so the claim
/// comes from our own config write instead.
///
/// Under the same reserved prefix as [`HEALTH_PATH`] and for the same reason: a
/// bare `t` segment would be a name the domain catalog could later take, and
/// then a new upstream would silently shadow every configured tool. `__gate` is
/// the segment the catalog cannot claim.
///
/// It does not make attribution *trustworthy* - any process on the loopback
/// interface can call any path, exactly as it can send any `User-Agent`. What it
/// buys is that the honest case stops depending on a string nobody here owns.
///
/// Worth knowing before treating this as cosmetic: attribution authorizes
/// nothing, but it is not inert either. `client_tool`'s result also gates
/// `inject_model_choice`, so naming a tool correctly can start applying a
/// Gate-model choice the user stored and the tool was too anonymous to receive.
/// That is the intent; `client_tool`'s doc has the full note.
///
/// **Rolling back is a hard break, not a soft one.** A build that predates this
/// prefix reads `__gate` as a leading catalog slug, finds no domain and no
/// `x-gate-upstream-url`, and answers 400 to every request from a config written
/// by a newer build. Forward compatibility was free and backward was not: an old
/// URL still routes here (`tool` is simply `None`), a new URL does not route
/// there. So a downgrade takes the configured tools offline until they are
/// reconnected, which makes this a poor release lever to reach for in a hurry.
pub(crate) const TOOL_PATH_PREFIX: &str = "/__gate/t/";

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

    /// The marker names the tool without disturbing anything behind it: the
    /// catalog still resolves off the slug, and the path forwarded upstream is
    /// byte-identical to the same request without a marker. That equality is
    /// the whole safety claim - a tool segment that changed the forwarded path
    /// would be a routing change wearing an attribution change's clothes.
    #[test]
    fn a_tool_marker_names_the_tool_and_leaves_the_route_untouched() {
        let marked = resolved("/__gate/t/opencode/anthropic/v1/messages?beta=true")
            .expect("a marked base URL resolves");
        let bare = resolved("/anthropic/v1/messages?beta=true").expect("resolves");

        assert_eq!(marked.tool, Some("opencode"));
        assert_eq!(bare.tool, None);
        assert_eq!(marked.path_and_query, bare.path_and_query);
        assert_eq!(marked.upstream_url, bare.upstream_url);
        assert_eq!(marked.slug, bare.slug);
        assert_eq!(marked.route, bare.route);

        // The passthrough arm too: account paths are the ones most likely to be
        // hit by a tool whose base URL we rewrote, and they carry the tool's own
        // credential, so a marker that broke their path would be the loudest bug
        // available.
        let marked = resolved("/__gate/t/codex/anthropic/api/oauth/usage").expect("resolves");
        assert_eq!(marked.tool, Some("codex"));
        assert_eq!(marked.route, Route::Passthrough);
        assert_eq!(marked.path_and_query, "/api/oauth/usage");

        // The one shape where the two segments are the same word: OpenCode's
        // Zen endpoints are on the `opencode` catalog slug, so the URL this
        // writes is `/__gate/t/opencode/opencode/zen/v1`. It works because the
        // positions are fixed, and it is the case a future edit to either
        // splitter would break while looking like a catalog miss rather than a
        // marker bug.
        let marked = resolved("/__gate/t/opencode/opencode/zen/v1/messages").expect("resolves");
        let bare = resolved("/opencode/zen/v1/messages").expect("resolves");
        assert_eq!(marked.tool, Some("opencode"));
        assert_eq!(marked.slug, bare.slug);
        assert_eq!(marked.path_and_query, bare.path_and_query);
    }

    /// `env-proxy` is a [`crate::registry::ToolId`] and not a tool, so it is
    /// dropped like any slug we cannot read. `x-gate-client` is defined over
    /// [`crate::taxonomy::Client`] slugs, which has no `env-proxy`, so stamping
    /// it would put a value in the ledger's column that its own vocabulary has
    /// no name for.
    #[test]
    fn the_environment_channel_is_not_a_tool_the_marker_can_name() {
        let r = resolved("/__gate/t/env-proxy/anthropic/v1/messages").expect("still routes");
        assert_eq!(r.tool, None);
        assert_eq!(r.path_and_query, "/v1/messages");

        // Every slug it does accept is one the ledger can name.
        for id in [
            crate::registry::ToolId::ClaudeCode,
            crate::registry::ToolId::Codex,
            crate::registry::ToolId::OpenCode,
        ] {
            let path = format!("/__gate/t/{}/anthropic/v1/messages", id.slug());
            let r = resolved(&path).expect("routes");
            assert_eq!(r.tool, Some(id.slug()));
            assert!(
                crate::taxonomy::Client::ALL
                    .iter()
                    .any(|c| c.slug() == id.slug()),
                "{} is stamped into x-gate-client but is not a Client slug",
                id.slug()
            );
        }
    }

    /// A dot segment is refused rather than forwarded, because the path we
    /// classify has to be the path we send.
    ///
    /// `classify` reads the raw string, but the URL is built by concatenation
    /// and parsed by `reqwest`, which collapses `.` and `..`. So
    /// `/anthropic/v1/../../x` reads as Rewrite - it starts with the `/v1/`
    /// prefix - takes the live Gate credential, and then lands on `<gateway>/x`,
    /// somewhere `classify` would never have sent a credentialed request.
    #[test]
    fn a_dot_segment_is_refused_rather_than_silently_renormalized() {
        for path in [
            "/anthropic/v1/../../admin",
            "/anthropic/v1/./messages",
            // The encoded spellings are the same segment to a URL parser, so a
            // check that only matched the literal one would be the same bug.
            "/anthropic/v1/%2e%2e/%2E%2E/admin",
            "/__gate/t/opencode/anthropic/v1/../../admin",
        ] {
            let err = resolve_route(&default_domains(), path, &HeaderMap::new())
                .expect_err("a dot segment is refused");
            assert_eq!(err.0, StatusCode::BAD_REQUEST, "{path}");
        }

        // A dot inside a segment is not a dot segment, and is none of our
        // business: plenty of real API paths carry one.
        let r = resolved("/anthropic/v1/messages.json?a=..").expect("routes");
        assert_eq!(r.path_and_query, "/v1/messages.json?a=..");
    }

    /// The `__gate` prefix's whole no-shadowing argument is that the catalog
    /// cannot claim it. That is prose in [`TOOL_PATH_PREFIX`]'s doc and nothing
    /// else, so pin it: a new upstream named `__gate` would silently shadow the
    /// health path and every configured tool at once.
    #[test]
    fn the_catalog_cannot_claim_the_reserved_prefix() {
        let reserved = TOOL_PATH_PREFIX.trim_start_matches('/');
        let reserved = reserved.split('/').next().expect("a first segment");
        assert_eq!(reserved, "__gate");
        for d in default_domains() {
            assert_ne!(d.slug, reserved, "a catalog domain claimed the reservation");
        }
    }

    /// A marker we cannot read loses the label and keeps the request.
    ///
    /// Both halves matter. Refusing would fail a request that is otherwise
    /// perfectly routable to protect a chart, which `inject_attribution` already
    /// rejects as the wrong trade. Keeping the segment would be worse than
    /// either: the catalog lookup behind it would miss, and the user would get a
    /// 400 naming a path they never typed.
    #[test]
    fn an_unreadable_tool_marker_is_dropped_rather_than_served_or_refused() {
        // A slug that is not a tool - a hand-edited config, or a tool this build
        // predates.
        let r = resolved("/__gate/t/notatool/anthropic/v1/messages").expect("still routes");
        assert_eq!(r.tool, None);
        assert_eq!(r.path_and_query, "/v1/messages");
        assert_eq!(r.slug, "anthropic");

        // The marker with nothing after it is not a marker.
        assert!(resolved("/__gate/t/").is_none());

        // `__gate` is reserved, so the health path cannot be read as a tool and
        // the two reservations cannot collide.
        assert!(resolved(HEALTH_PATH).is_none());
    }

    /// The marker only means anything in the leading position it is written in.
    /// A catalog domain whose own path happens to contain the prefix's spelling
    /// must not have it stripped out of the middle of a forwarded URL.
    #[test]
    fn the_marker_is_only_read_at_the_front() {
        let r = resolved("/anthropic/v1/__gate/t/opencode/messages").expect("resolves");
        assert_eq!(r.tool, None);
        assert_eq!(r.path_and_query, "/v1/__gate/t/opencode/messages");
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
