//! The loopback MITM engine. Owns a tokio runtime on a dedicated thread,
//! runs a [`hudsucker`] proxy, and rewrites matched inference requests to
//! the Gate gateway. Cross-platform; the macOS-specific trust/system-proxy
//! wiring lives in sibling modules.
//!
//! The enabled-domain set is hot-swappable via a [`tokio::sync::watch`]
//! channel: because the system proxy routes *all* hosts to us and the
//! handler gates MITM per-host in `should_intercept`, toggling a domain only
//! needs to push new rules - no engine restart, no system-proxy change, no
//! extra admin prompt.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result};
use hudsucker::{
    hyper::{header::HeaderValue, Method, Request, Uri},
    hyper_util::{rt::TokioExecutor, server::conn::auto::Builder as ServerBuilder},
    rcgen::{Issuer, KeyPair},
    rustls::crypto::{aws_lc_rs, CryptoProvider},
    rustls::pki_types::{pem::PemObject, CertificateDer},
    rustls::{ClientConfig, RootCertStore},
    Body, HttpContext, HttpHandler, Proxy, RequestOrResponse,
};
use hyper_rustls::ConfigBuilderExt;
use tokio::sync::{oneshot, watch};

use crate::account::BillingMode;
use crate::proxy::cert_authority::GateCa;
use crate::proxy::{decide, effective_billing_mode, should_intercept_host, Decision, ProxyDomain};

/// Everything the engine needs to run one session. The account + CA are
/// fixed for the engine's lifetime; the domain set can be updated live via
/// [`RunningEngine::update_domains`].
pub struct EngineConfig {
    /// Gate gateway base URL - the rewrite target authority.
    pub gateway_base_url: String,
    /// Gate API key, injected as `X-Gate-Api-Key` when no OAuth token is set
    /// (the legacy credential).
    pub api_key: String,
    /// Cognito access token. When non-empty it's injected as
    /// `X-Gate-Authorization: Bearer <token>` *instead of* the API key;
    /// empty means fall back to `api_key`. Hot-swappable via
    /// [`RunningEngine::update_token`].
    pub oauth_token: String,
    /// Selected org UUID, injected as `X-Gate-Org-Id` alongside the OAuth
    /// token (the gateway requires it on every OAuth request). Empty when no
    /// org is selected or in legacy API-key mode. Hot-swappable via
    /// [`RunningEngine::update_org`].
    pub org_id: String,
    /// Who pays the upstream provider. `Payg` drops `X-Gate-Upstream-Url` and
    /// the client's own credential on the rewrite path, so the gateway bills
    /// the org's balance instead of the tool's provider account.
    /// Hot-swappable via [`RunningEngine::update_mode`].
    pub billing_mode: BillingMode,
    /// Full domain catalog; the engine routes only the `enabled` ones.
    pub domains: Vec<ProxyDomain>,
    /// PEM of the local root CA cert (public).
    pub ca_cert_pem: String,
    /// PEM of the local root CA private key.
    pub ca_key_pem: String,
    /// Preferred loopback port to bind. `Some(p)` asks the engine to reuse a
    /// previously-chosen port so a restart keeps the same address - the system
    /// proxy pointer baked into a login session (Linux) or a client that
    /// resolved the proxy once at its own launch (macOS/Windows) stays valid
    /// across app restarts instead of dangling at a dead ephemeral port. Falls
    /// back to an ephemeral port if `p` is taken (or `None`). All three
    /// platforms persist and pass the last-bound port.
    pub preferred_port: Option<u16>,
    /// Preferred loopback port for the PAC listener, same contract as
    /// [`preferred_port`](Self::preferred_port): reuse the last-bound port so
    /// the `AutoConfigURL` a client captured at its own launch still serves a
    /// fresh PAC after we restart, instead of failing the fetch and silently
    /// falling back to DIRECT (bypassing Gate). Only read on PAC-driven
    /// platforms (macOS/Windows); Linux uses env-var proxies with no PAC and
    /// passes `None` (a plain field, like `owner_uid`, so construction sites
    /// need no cfg attribute).
    pub preferred_pac_port: Option<u16>,
    /// Preferred loopback port for the CLI reverse-proxy relay ([`super::relay`]),
    /// bound alongside the MITM listener. Same stable-port rationale as
    /// `preferred_port`: CLI tools bake this port into their config, so reusing
    /// it across restarts keeps that config valid. Ephemeral if taken/`None`.
    pub preferred_relay_port: Option<u16>,
    /// When `Some(uid)`, only connections from that local UID are intercepted
    /// (MITM'd + rewritten with the Gate key injected); traffic from any other
    /// local user is blind-tunnelled. The loopback listener is plain TCP and
    /// reachable by every local user, so this stops a *different* account from
    /// spending the owner's Gate key through the proxy. Linux sets the daemon's
    /// own UID; macOS/Windows pass `None` (out of scope for this release).
    pub owner_uid: Option<u32>,
    /// The user's pre-existing upstream proxy , used as the
    /// PAC fallback so non-Gate traffic keeps flowing through it instead of
    /// going DIRECT while routing is on. Only read on PAC-driven platforms
    /// (macOS/Windows); Linux passes `None`.
    pub upstream_proxy: Option<String>,
}

/// A running engine. Dropping it signals graceful shutdown (fail-safe so a
/// crashed caller never leaves the loopback listener orphaned); [`stop`]
/// also joins the thread.
///
/// [`stop`]: RunningEngine::stop
pub struct RunningEngine {
    port: u16,
    /// Loopback port of the CLI reverse-proxy relay ([`super::relay`]), bound
    /// alongside the MITM listener. CLI tool configs point their base URL here.
    relay_port: u16,
    /// Loopback port serving the PAC script the system proxy points at.
    /// PAC-driven platforms only (Windows `AutoConfigURL`, macOS
    /// `networksetup -setautoproxyurl`); Linux uses env-var proxies with no PAC.
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    pac_port: u16,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
    rules_tx: watch::Sender<Arc<Vec<ProxyDomain>>>,
    key_tx: watch::Sender<Arc<str>>,
    token_tx: watch::Sender<Arc<str>>,
    org_tx: watch::Sender<Arc<str>>,
    mode_tx: watch::Sender<BillingMode>,
    /// Whether the relay rewrites inference to the gateway (true) or forwards
    /// everything straight to the real upstream (false). See
    /// [`set_relay_intercept`](Self::set_relay_intercept).
    relay_intercept_tx: watch::Sender<bool>,
    /// Set before a deliberate shutdown so the engine thread can tell an
    /// expected stop from an unexpected exit (crash / bind loss).
    stopping: Arc<AtomicBool>,
}

impl RunningEngine {
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Loopback port of the CLI reverse-proxy relay. CLI tool configs point
    /// their base URL at `http://127.0.0.1:<relay_port>`.
    pub fn relay_port(&self) -> u16 {
        self.relay_port
    }

    /// Loopback port serving the PAC script (Windows-only).
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    pub fn pac_port(&self) -> u16 {
        self.pac_port
    }

    /// True once the engine thread has exited (crash or stop) - the
    /// listener is gone and the port is dead.
    pub fn is_finished(&self) -> bool {
        self.thread
            .as_ref()
            .map(|t| t.is_finished())
            .unwrap_or(true)
    }

    /// Push a new enabled-domain set to the live engine. Cheap - no restart.
    pub fn update_domains(&self, domains: &[ProxyDomain]) {
        let _ = self.rules_tx.send(Arc::new(enabled_only(domains)));
    }

    /// How many domains the engine is currently intercepting (0 == pure
    /// pass-through / blind-tunnel everything).
    pub fn intercepting(&self) -> usize {
        self.rules_tx.borrow().len()
    }

    /// Push a rotated Gate API key to the live engine. Cheap - no restart;
    /// without this the engine keeps injecting the key it was started with.
    pub fn update_api_key(&self, api_key: &str) {
        let _ = self.key_tx.send(Arc::from(api_key));
    }

    /// Push a refreshed OAuth access token to the live engine. Empty string
    /// clears it (reverting to the API key). Cheap - no restart; this is how
    /// a silent token refresh reaches in-flight routing.
    pub fn update_token(&self, oauth_token: &str) {
        let _ = self.token_tx.send(Arc::from(oauth_token));
    }

    /// Push the selected org UUID to the live engine (injected as
    /// `X-Gate-Org-Id` alongside the OAuth token). Empty string clears it.
    /// Cheap - no restart; this is how an org switch reaches in-flight routing.
    pub fn update_org(&self, org_id: &str) {
        let _ = self.org_tx.send(Arc::from(org_id));
    }

    /// Push a changed billing mode to the live engine (and the relay it hosts).
    /// Cheap - no restart; this is how flipping BYOK/PAYG reaches in-flight
    /// routing, and it is the only way the shape of subsequent requests changes
    /// without reconnecting every tool.
    pub fn update_mode(&self, mode: BillingMode) {
        let _ = self.mode_tx.send(mode);
    }

    /// Flip the relay between gateway interception (rewrite inference and
    /// inject the Gate credential - the default) and direct forwarding (every
    /// request goes to the tool's real upstream under its own credential, the
    /// relay's analogue of the MITM port's blind tunnel). The Linux helper
    /// daemon flips this alongside the domain set: with no GUI connected
    /// there's nothing keeping the injected token fresh, so routing to Gate
    /// would quietly decay into 401s - going direct keeps the tools working,
    /// just not through Gate. Cheap - no restart.
    pub fn set_relay_intercept(&self, intercept: bool) {
        let _ = self.relay_intercept_tx.send(intercept);
    }

    /// Signal graceful shutdown and wait for the engine thread to exit.
    pub fn stop(mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for RunningEngine {
    fn drop(&mut self) {
        // Best-effort shutdown signal if dropped without an explicit stop().
        // Mark it expected so the fail-safe callback doesn't fire on a
        // normal drop. We don't join in Drop to avoid blocking.
        self.stopping.store(true, Ordering::Release);
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

fn enabled_only(domains: &[ProxyDomain]) -> Vec<ProxyDomain> {
    domains.iter().filter(|d| d.enabled).cloned().collect()
}

/// TLS config for the engine's own outbound hop - to the gateway for a
/// rewritten request, and to the real provider for a passthrough path on an
/// intercepted host.
///
/// Exists only because hudsucker's `with_rustls_connector` pins webpki roots
/// with no way to add one. That is the right default and stays the default, but
/// it also made the real `proxy enable` path untestable: a hermetic test needs a
/// loopback gateway, a loopback gateway needs a private CA, and a private CA is
/// exactly what webpki roots reject. `proxy_e2e`'s module docs have carried that
/// gap as a known exclusion, and the gap is what let two silent-bypass bugs sit
/// in the engine path unnoticed.
///
/// `GATE_CONNECT_TEST_CA` is the same variable and the same seam the relay
/// already reads (see [`super::relay`]), so both halves of the proxy trust the
/// same throwaway CA in a test and neither has one in production - the variable
/// is never set in a shipped build.
///
/// The extra root is ADDED to the public set, never substituted for it. This
/// connector also carries passthrough traffic to real providers, so a store
/// holding only the test CA would fail every host that isn't the mock.
fn upstream_tls_config(provider: CryptoProvider) -> Result<ClientConfig> {
    let builder = ClientConfig::builder_with_provider(Arc::new(provider))
        .with_safe_default_protocol_versions()
        .context("selecting TLS protocol versions for the upstream connector")?;

    let Some(path) = std::env::var_os("GATE_CONNECT_TEST_CA") else {
        return Ok(builder.with_webpki_roots().with_no_client_auth());
    };

    let mut roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let extra = CertificateDer::pem_file_iter(&path)
        .with_context(|| format!("reading GATE_CONNECT_TEST_CA {path:?}"))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("parsing GATE_CONNECT_TEST_CA {path:?}"))?;
    // Loud rather than silent: a seam that quietly added nothing would look
    // exactly like the failure it exists to rule out.
    if extra.is_empty() {
        anyhow::bail!("GATE_CONNECT_TEST_CA {path:?} contains no certificates");
    }
    let added = extra.len();
    for cert in extra {
        roots
            .add(cert)
            .context("adding GATE_CONNECT_TEST_CA to the upstream roots")?;
    }
    eprintln!("[gate-proxy] upstream roots: webpki + {added} from GATE_CONNECT_TEST_CA ({path:?})");
    Ok(builder.with_root_certificates(roots).with_no_client_auth())
}

/// Whether to emit per-request engine logs to stderr. Off unless
/// `GATE_PROXY_DEBUG` is set in the environment, so production stays quiet.
pub(crate) fn debug_log() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("GATE_PROXY_DEBUG").is_some())
}

/// Wire hudsucker's internal `tracing` events (TLS handshake / HTTP2 errors)
/// to stderr once, when debug logging is on. Without this, MITM failures
/// inside hudsucker are silent. Overridable via `RUST_LOG`. Idempotent.
fn init_tracing() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if !debug_log() {
            return;
        }
        let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            tracing_subscriber::EnvFilter::new("hudsucker=debug,rustls=info,info")
        });
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .try_init();
    });
}

#[derive(Clone)]
struct GateHandler {
    /// Live-updatable set of enabled domains.
    rules: watch::Receiver<Arc<Vec<ProxyDomain>>>,
    /// Parsed gateway URL - its scheme + authority replace the target's.
    gateway: Uri,
    /// Live-updatable Gate API key (rotations push a new value).
    api_key: watch::Receiver<Arc<str>>,
    /// Live-updatable Cognito access token. Empty string means "unset" -
    /// fall back to `api_key`.
    token: watch::Receiver<Arc<str>>,
    /// Live-updatable selected org UUID. Empty string means "none selected";
    /// injected as `X-Gate-Org-Id` only when an OAuth token is present.
    org: watch::Receiver<Arc<str>>,
    /// Live-updatable billing mode. Resolved per domain before it is applied -
    /// see [`effective_billing_mode`].
    mode: watch::Receiver<BillingMode>,
    /// When `Some`, only intercept connections from this local UID (see
    /// [`EngineConfig::owner_uid`]).
    owner_uid: Option<u32>,
    /// Per-connection memo of the owner-UID verdict, keyed by peer address.
    /// hudsucker clones the handler per connection, so this is resolved once -
    /// while the peer's socket is definitely still in `/proc/net/tcp` - instead
    /// of re-reading it (and risking a TOCTOU miss) on every request.
    peer_verdict: Option<(std::net::SocketAddr, bool)>,
}

impl GateHandler {
    /// Whether the peer behind `ctx` may be intercepted. `true` when no owner
    /// restriction is set; otherwise the peer's UID (resolved from its loopback
    /// socket) must match the owner. Fails **closed**: if the UID can't be
    /// resolved we decline interception (blind-tunnel) rather than risk
    /// injecting the Gate key for an unverified peer. Memoized per peer.
    fn peer_allowed(&mut self, ctx: &HttpContext) -> bool {
        let owner = match self.owner_uid {
            None => return true,
            Some(owner) => owner,
        };
        if let Some((addr, verdict)) = self.peer_verdict {
            if addr == ctx.client_addr {
                return verdict;
            }
        }
        let verdict = peer_uid_for(ctx.client_addr) == Some(owner);
        self.peer_verdict = Some((ctx.client_addr, verdict));
        verdict
    }
}

/// Resolve the local UID that owns the socket whose *local* address is `addr`
/// (i.e. the connecting peer's socket) by scanning the kernel's TCP table.
/// Linux-only; other platforms never set `owner_uid`, so this just reports
/// "unknown". Returns `None` if the socket isn't found or can't be parsed.
/// Shared with the relay (`super::relay`), which gates its accept loop the
/// same way the MITM path gates interception.
#[cfg(target_os = "linux")]
pub(crate) fn peer_uid_for(addr: std::net::SocketAddr) -> Option<u32> {
    use std::net::{Ipv4Addr, SocketAddr};
    // Clients reach us at http://127.0.0.1:<port>, so the peer is IPv4 loopback;
    // we only parse /proc/net/tcp (v4). A v6 peer (shouldn't happen) fails
    // closed.
    let SocketAddr::V4(want) = addr else {
        return None;
    };
    let content = std::fs::read_to_string("/proc/net/tcp").ok()?;
    for line in content.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // Columns: 0 sl, 1 local_address, 2 rem_address, 3 st, ..., 7 uid.
        let (Some(local), Some(uid)) = (fields.get(1), fields.get(7)) else {
            continue;
        };
        let (ip_hex, port_hex) = match local.split_once(':') {
            Some(p) => p,
            None => continue,
        };
        // /proc stores the v4 address as a host-byte-order (little-endian on
        // x86) hex u32; swap to network order for Ipv4Addr. Port is plain hex.
        let (Ok(ip_raw), Ok(port)) = (
            u32::from_str_radix(ip_hex, 16),
            u16::from_str_radix(port_hex, 16),
        ) else {
            continue;
        };
        let ip = Ipv4Addr::from(ip_raw.swap_bytes());
        if &ip == want.ip() && port == want.port() {
            return uid.parse::<u32>().ok();
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn peer_uid_for(_addr: std::net::SocketAddr) -> Option<u32> {
    None
}

impl HttpHandler for GateHandler {
    async fn should_intercept(&mut self, ctx: &HttpContext, req: &Request<Body>) -> bool {
        // Called on the CONNECT request *before* any TLS handshake. Only
        // MITM hosts we actually route; everything else is blind-tunnelled,
        // so cert-pinning apps and unrelated traffic are untouched.
        // Traffic from a non-owner local user is never MITM'd.
        if !self.peer_allowed(ctx) {
            if debug_log() {
                eprintln!("[gate-proxy] CONNECT from non-owner peer -> tunnel");
            }
            return false;
        }
        let rules = self.rules.borrow().clone();
        let host = req
            .uri()
            .authority()
            .map(|a| a.host())
            .or_else(|| req.uri().host());
        let intercept = host
            .map(|h| should_intercept_host(&rules, h))
            .unwrap_or(false);
        if debug_log() {
            // The enabled set is printed alongside the verdict because the two
            // have been observed to disagree with what `proxy enable` reports:
            // the CLI listed anthropic and openrouter as on while the engine
            // tunnelled both. Without this the log cannot distinguish "the host
            // is not in the catalog" from "the engine is holding a different
            // catalog than the one just configured".
            let enabled: Vec<&str> = rules
                .iter()
                .filter(|d| d.enabled)
                .map(|d| d.slug.as_str())
                .collect();
            eprintln!(
                // `rules` is already enabled_only(), so its length IS the
                // enabled count - saying "total" would imply the catalog size.
                "[gate-proxy] CONNECT {} -> {} (engine has {} enabled: [{}])",
                host.unwrap_or("?"),
                if intercept { "intercept" } else { "tunnel" },
                rules.len(),
                enabled.join(",")
            );
        }
        intercept
    }

    async fn handle_request(
        &mut self,
        ctx: &HttpContext,
        mut req: Request<Body>,
    ) -> RequestOrResponse {
        // The CONNECT request itself flows through here first; nothing to
        // rewrite on it. Intercepted inner requests arrive in absolute form
        // (scheme + authority + path), which is what `decide` expects.
        if req.method() == Method::CONNECT {
            return req.into();
        }
        let rules = self.rules.borrow().clone();
        let host = req.uri().host().map(str::to_owned);
        // Path only, never `path_and_query()`: some providers pass the API key
        // as a URL query param (e.g. Google `...?key=...`), and this value is
        // written to the debug log below. `Uri::path()` excludes the query, so
        // URL-embedded keys never reach the log. Keep it that way.
        let path = req.uri().path().to_owned();
        let mut action = "passthrough";
        if let Some(host) = host.as_deref() {
            // Rewrite matched inference paths to the gateway, forwarding to
            // the domain's configured upstream - for Anthropic that's the same
            // api.anthropic.com the request came from , validated
            // against a real Cowork generation: 200 text/event-stream.
            // Gate the rewrite on owner UID too: plain-HTTP requests reach here
            // without a CONNECT (so `should_intercept` never gated them), and we
            // must not inject the Gate key for a non-owner peer.
            if let (Decision::Rewrite { upstream_url, slug }, true) =
                (decide(&rules, host, &path), self.peer_allowed(ctx))
            {
                let api_key = self.api_key.borrow().clone();
                let token = self.token.borrow().clone();
                let oauth_token = (!token.is_empty()).then(|| token.as_ref());
                let org = self.org.borrow().clone();
                let org_id = (!org.is_empty()).then(|| org.as_ref());
                let mode = effective_billing_mode(*self.mode.borrow(), &slug);
                match apply_rewrite(
                    &mut req,
                    &self.gateway,
                    &upstream_url,
                    &api_key,
                    oauth_token,
                    org_id,
                    mode,
                ) {
                    Ok(()) => {
                        action = "rewrite->gateway";
                    }
                    Err(e) => {
                        action = "rewrite-FAILED";
                        if debug_log() {
                            eprintln!("[gate-proxy] rewrite failed: {e}");
                        }
                    }
                }
            }
        }
        if debug_log() {
            let auth = if req
                .headers()
                .contains_key(hudsucker::hyper::header::AUTHORIZATION)
            {
                "bearer"
            } else if req.headers().contains_key("x-api-key") {
                "x-api-key"
            } else {
                "none"
            };
            let ver = format!("{:?}", req.version());
            eprintln!(
                "[gate-proxy] {} {}{} [{ver}] auth={auth} -> {action}",
                req.method(),
                host.as_deref().unwrap_or("?"),
                path,
            );
            // Dump x-goog-* request headers as forwarded to the gateway.
            // The Code Assist project rides in `x-goog-user-project` (when
            // not in the JSON body); present here but missing at Google
            // means the gateway drops it. The api-key value is a secret, so
            // log only its presence.
            for (name, value) in req.headers() {
                let n = name.as_str();
                if n.starts_with("x-goog") {
                    let shown = if n == "x-goog-api-key" {
                        "<redacted>"
                    } else {
                        value.to_str().unwrap_or("<binary>")
                    };
                    eprintln!("[gate-proxy]   {n}: {shown}");
                }
            }
        }
        req.into()
    }

    async fn handle_response(
        &mut self,
        _ctx: &HttpContext,
        res: hudsucker::hyper::Response<Body>,
    ) -> hudsucker::hyper::Response<Body> {
        if debug_log() {
            eprintln!(
                "[gate-proxy] <- {} ct={:?}",
                res.status(),
                res.headers()
                    .get(hudsucker::hyper::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
            );
        }
        res
    }
}

/// Repoint a request at the gateway: swap scheme + authority for the
/// gateway's, strip the upstream's own path prefix, and inject the Gate
/// headers. In BYOK the app's own auth header (bearer / `x-api-key`) is left
/// intact - Gate validates the Gate credential and forwards the rest. In PAYG
/// both that header and the upstream hint are dropped, which is what tells the
/// gateway to bill the org and forward under its own provider account; the
/// credential precedence and the strip both live in
/// [`super::inject_gate_credential`], shared with the relay so the two paths
/// can't drift.
///
/// The path strip is what keeps a provider whose API lives under a reserved
/// prefix routable: Gate appends the forwarded path to `X-Gate-Upstream-Url`,
/// so moving `/api` from the request line into the upstream URL reassembles to
/// the same provider URL while sending Gate a path its ALB won't divert. See
/// the `openrouter` catalog entry in [`super::default_domains`]. In PAYG there
/// is no upstream URL to reassemble against, and the strip is what leaves the
/// gateway-native path (`/v1/chat/completions`) its reseller router expects.
pub(crate) fn apply_rewrite<T>(
    req: &mut Request<T>,
    gateway: &Uri,
    upstream_url: &str,
    api_key: &str,
    oauth_token: Option<&str>,
    org_id: Option<&str>,
    mode: BillingMode,
) -> Result<()> {
    let gw = gateway.clone().into_parts();
    let mut parts = req.uri().clone().into_parts();
    parts.scheme = gw.scheme;
    parts.authority = gw.authority;

    let upstream_path = super::upstream_path(upstream_url);
    if !upstream_path.is_empty() {
        let pq = parts.path_and_query.as_ref().map_or("/", |pq| pq.as_str());
        let (path, query) = pq.split_once('?').map_or((pq, None), |(p, q)| (p, Some(q)));
        let stripped = super::strip_upstream_path(path, upstream_path).with_context(|| {
            format!("request path {path:?} is outside upstream {upstream_url:?}")
        })?;
        let rebuilt = match query {
            Some(q) => format!("{stripped}?{q}"),
            None => stripped.to_string(),
        };
        parts.path_and_query = Some(rebuilt.parse().with_context(|| {
            format!("rebuilding request path after stripping {upstream_path:?}")
        })?);
    }

    *req.uri_mut() = Uri::from_parts(parts).context("rebuilding rewritten request URI")?;

    let headers = req.headers_mut();
    super::inject_gate_credential(headers, api_key, oauth_token, org_id, mode)?;
    // PAYG is the ABSENCE of this header: with it the gateway forwards under
    // the caller's own credential (BYOK), without it the gateway routes to one
    // of the org's provider accounts and debits its balance. Nothing else in
    // the request says which mode it is.
    if mode == BillingMode::Byok {
        headers.insert(
            super::UPSTREAM_URL_HEADER,
            HeaderValue::from_str(upstream_url).context("building x-gate-upstream-url header")?,
        );
    } else {
        // A caller cannot smuggle BYOK back in on a PAYG rewrite.
        headers.remove(super::UPSTREAM_URL_HEADER);
    }
    Ok(())
}

/// Bind a loopback listener and return it together with the port it landed on.
/// Tries `preferred` first (so a restart can reuse the same port and keep a
/// frozen system-proxy pointer valid); if that's unavailable - taken, or
/// `None` - falls back to an ephemeral port. Returning the *live* listener -
/// rather than probing a port and dropping it before hudsucker binds - closes
/// the TOCTOU window where another process could grab the port in the gap. The
/// socket stays held from here until it's handed to the proxy. Set non-blocking
/// so tokio can adopt it.
fn bind_loopback(preferred: Option<u16>) -> Result<(std::net::TcpListener, u16)> {
    let listener = match preferred {
        Some(p) => bind_preferred(p)
            .or_else(|_| std::net::TcpListener::bind(("127.0.0.1", 0)))
            .with_context(|| format!("binding loopback (preferred {p}, then ephemeral)"))?,
        None => {
            std::net::TcpListener::bind(("127.0.0.1", 0)).context("binding a free loopback port")?
        }
    };
    let port = listener
        .local_addr()
        .context("reading listener socket address")?
        .port();
    // Losing the preferred port is the quiet half of a restart. The PAC is
    // rewritten and self-heals, but the exported `HTTPS_PROXY` is read once at
    // a process's launch, so every tool already running keeps dialling the old
    // port - and a proxy that refuses fails every request, where a stale PAC
    // merely falls back to DIRECT. Say so: the only symptom otherwise is a tool
    // that cannot connect, with nothing anywhere naming the reason.
    if let Some(wanted) = preferred {
        if wanted != port {
            eprintln!(
                "gate proxy: wanted the previous loopback port {wanted}, but it was taken; \
                 bound {port} instead. Tools started before this keep using {wanted} until \
                 they restart."
            );
        }
    }
    listener
        .set_nonblocking(true)
        .context("setting the loopback listener non-blocking")?;
    Ok((listener, port))
}

/// Bind `127.0.0.1:port` for the preferred-port reuse path. On unix a plain
/// bind is tried first; when it fails with the port "in use", the previous
/// engine session's connections may just be lingering as server-side
/// TIME_WAIT sockets (they stay for minutes), and without `SO_REUSEADDR` the
/// restart's rebind fails and silently falls back to an ephemeral port -
/// defeating the address stability the preferred port exists for. But BSD /
/// macOS `SO_REUSEADDR` also permits a `127.0.0.1:P` bind while another
/// process holds a *live* `0.0.0.0:P` listener, which would silently shadow
/// that app's loopback traffic. A connect probe tells the two apart: a port
/// held only by TIME_WAIT remnants refuses, a live listener accepts - so the
/// `SO_REUSEADDR` rebind runs only after a refused probe, and a live listener
/// keeps the taken-port fallback. (A process binding the wildcard in the
/// probe-to-bind gap can still be shadowed; that race is unavoidable and
/// vanishingly narrow.) Windows keeps the plain bind - its defaults already
/// allow the rebind, and `SO_REUSEADDR` there *would* let another local
/// process hijack a live port.
/// Shared with [`super::relay::serve`], the standalone host, so the two paths
/// agree on what "the port is taken" means - a live listener, not a TIME_WAIT
/// remnant of the host that just exited.
#[cfg(unix)]
pub(super) fn bind_preferred(port: u16) -> std::io::Result<std::net::TcpListener> {
    if let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", port)) {
        return Ok(listener);
    }
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    if std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(250)).is_ok() {
        // Someone is live on this port (loopback or wildcard) - don't shadow
        // it; let the caller fall back to an ephemeral port.
        return Err(std::io::Error::from(std::io::ErrorKind::AddrInUse));
    }
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )?;
    socket.set_reuse_address(true)?;
    socket.bind(&addr.into())?;
    // Match std's TcpListener::bind backlog.
    socket.listen(128)?;
    Ok(socket.into())
}

#[cfg(not(unix))]
pub(super) fn bind_preferred(port: u16) -> std::io::Result<std::net::TcpListener> {
    std::net::TcpListener::bind(("127.0.0.1", port))
}

/// Build the PAC (proxy auto-config) script WinINET runs for every connection.
/// Enabled Gate hosts route to the loopback proxy; everything else falls to
/// `upstream` when the user already had a proxy (preserving a corporate proxy),
/// or DIRECT otherwise. Host matching mirrors [`ProxyDomain::matches_host`]:
/// exact, case-insensitive hostnames.
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn pac_script(domains: &[ProxyDomain], proxy_port: u16, upstream: Option<&str>) -> String {
    let mut s = String::from("function FindProxyForURL(url, host) {\n");
    s.push_str("  var h = host.toLowerCase();\n");
    for host in domains.iter().flat_map(|d| d.hosts.iter()) {
        s.push_str(&format!(
            "  if (h === \"{}\") return \"PROXY 127.0.0.1:{proxy_port}\";\n",
            host.to_ascii_lowercase()
        ));
    }
    match upstream {
        // Preserve the user's prior upstream proxy for all other traffic, but
        // keep plain/local hostnames DIRECT as WinINET's `<local>` bypass did.
        Some(proxy) => {
            s.push_str("  if (isPlainHostName(h)) return \"DIRECT\";\n");
            s.push_str(&format!("  return \"PROXY {proxy}\";\n"));
        }
        None => s.push_str("  return \"DIRECT\";\n"),
    }
    s.push_str("}\n");
    s
}

/// Serve the PAC script on a dedicated loopback listener. WinINET fetches the
/// `AutoConfigURL` *directly* (not through the proxy), so this must be a plain
/// HTTP responder, separate from the hudsucker proxy on `proxy_port`. The body
/// is rebuilt per request from the live rule set. Runs until the engine's
/// runtime is torn down.
#[cfg(any(target_os = "windows", target_os = "macos"))]
async fn serve_pac(
    listener: tokio::net::TcpListener,
    rules: watch::Receiver<Arc<Vec<ProxyDomain>>>,
    proxy_port: u16,
    upstream: Option<String>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            continue;
        };
        let rules = rules.clone();
        let upstream = upstream.clone();
        tokio::spawn(async move {
            // Consume the request (a small GET we don't parse) before replying.
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let body = pac_script(&rules.borrow(), proxy_port, upstream.as_deref());
            let resp = format!(
                "HTTP/1.1 200 OK\r\n\
                 Content-Type: application/x-ns-proxy-autoconfig\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len(),
            );
            let _ = stream.write_all(resp.as_bytes()).await;
            let _ = stream.shutdown().await;
        });
    }
}

/// Start the engine on an ephemeral loopback port. Blocks until the proxy
/// has built and bound (or fails), then returns a handle. The tokio runtime
/// lives on the spawned thread and is torn down when the handle is stopped
/// or dropped.
///
/// `on_unexpected_exit` fires if the server loop ends *without* a deliberate
/// stop - i.e. the engine crashed or lost its bind. Callers use it to revert
/// the system proxy so traffic is never stranded at a dead listener.
pub fn start<F>(cfg: EngineConfig, on_unexpected_exit: F) -> Result<RunningEngine>
where
    F: FnOnce() + Send + 'static,
{
    init_tracing();
    let gateway: Uri = cfg
        .gateway_base_url
        .parse()
        .with_context(|| format!("parsing gateway URL {:?}", cfg.gateway_base_url))?;
    if gateway.host().is_none() {
        anyhow::bail!("gateway URL {:?} has no host", cfg.gateway_base_url);
    }

    let (listener, port) = bind_loopback(cfg.preferred_port)?;
    // CLI reverse-proxy relay ([`super::relay`]) on its own loopback port; CLI
    // tool configs point their base URL here. Bound eagerly so the port is
    // known before the engine thread starts, like the MITM listener.
    let (relay_listener, relay_port) =
        bind_loopback(cfg.preferred_relay_port).context("binding the relay loopback port")?;
    // Windows points WinINET at a PAC served on its own loopback port (see
    // `serve_pac`); the proxy port itself is baked into the PAC body.
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let (pac_listener, pac_port) =
        bind_loopback(cfg.preferred_pac_port).context("binding the PAC loopback port")?;

    let (rules_tx, rules_rx) = watch::channel(Arc::new(enabled_only(&cfg.domains)));
    // The PAC body is regenerated per request from this live rule set, so a
    // domain toggle needs no registry write.
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let pac_rules_rx = rules_rx.clone();
    // Fallback proxy baked into the PAC so non-Gate traffic keeps using the
    // user's prior proxy instead of going DIRECT.
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let upstream_proxy = cfg.upstream_proxy.clone();
    let (key_tx, key_rx) = watch::channel::<Arc<str>>(Arc::from(cfg.api_key.as_str()));
    let (token_tx, token_rx) = watch::channel::<Arc<str>>(Arc::from(cfg.oauth_token.as_str()));
    let (org_tx, org_rx) = watch::channel::<Arc<str>>(Arc::from(cfg.org_id.as_str()));
    let (mode_tx, mode_rx) = watch::channel(cfg.billing_mode);
    // Intercepting until told otherwise: the engine only starts on an explicit
    // enable / SetIntercept, both of which mean "route through Gate".
    let (relay_intercept_tx, relay_intercept_rx) = watch::channel(true);
    // The relay shares the same credential channels , so a
    // token refresh, key rotation, or org switch reaches CLI tools and GUI apps
    // alike. Clone before the handler moves the originals.
    let relay_gateway = gateway.clone();
    let relay_key_rx = key_rx.clone();
    let relay_token_rx = token_rx.clone();
    let relay_org_rx = org_rx.clone();
    let relay_mode_rx = mode_rx.clone();
    // The relay gates its accept loop on the same owner UID the MITM path uses.
    let relay_owner_uid = cfg.owner_uid;
    let handler = GateHandler {
        rules: rules_rx,
        gateway,
        api_key: key_rx,
        token: token_rx,
        org: org_rx,
        mode: mode_rx,
        owner_uid: cfg.owner_uid,
        peer_verdict: None,
    };

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let cert_pem = cfg.ca_cert_pem;
    let key_pem = cfg.ca_key_pem;
    let stopping = Arc::new(AtomicBool::new(false));
    let stopping_thread = Arc::clone(&stopping);

    let thread = std::thread::Builder::new()
        .name("gate-proxy".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("building tokio runtime: {e}")));
                    return;
                }
            };
            rt.block_on(async move {
                let ca = {
                    let key_pair = match KeyPair::from_pem(&key_pem) {
                        Ok(k) => k,
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("parsing CA private key: {e}")));
                            return;
                        }
                    };
                    match Issuer::from_ca_cert_pem(&cert_pem, key_pair) {
                        Ok(issuer) => GateCa::new(issuer, aws_lc_rs::default_provider()),
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("parsing CA certificate: {e}")));
                            return;
                        }
                    }
                };

                let listener = match tokio::net::TcpListener::from_std(listener) {
                    Ok(l) => l,
                    Err(e) => {
                        let _ = ready_tx.send(Err(format!("adopting loopback listener: {e}")));
                        return;
                    }
                };

                // Hand-built connector rather than `with_rustls_connector`,
                // which pins webpki roots - see `upstream_tls_config`.
                let tls = match upstream_tls_config(aws_lc_rs::default_provider()) {
                    Ok(cfg) => cfg,
                    Err(e) => {
                        let _ = ready_tx.send(Err(format!("building upstream TLS config: {e:#}")));
                        return;
                    }
                };
                let https = hyper_rustls::HttpsConnectorBuilder::new()
                    .with_tls_config(tls)
                    .https_or_http()
                    .enable_http1()
                    .enable_http2()
                    .build();
                // Supplying our own server builder to raise the h2 header
                // limit. hudsucker's fallback builder only configures `http1()`,
                // so h2 keeps the `h2` crate default
                // `SETTINGS_MAX_HEADER_LIST_SIZE` of 16 KiB - and the browser
                // surfaces blow straight through it: chatgpt.com's web client
                // sends ~8.3 KB of its own headers on every call
                // (`x-oai-is-pending-updates` alone was 5446 B in a capture, and
                // it grows until the server acks it) plus a session cookie jar,
                // landing just over 16 KB. hyper answers that with a bare
                // `431` - no headers, no body - so the request never leaves the
                // machine and the client's chat wedges before its first turn.
                // Measured at the engine: 16000 B of headers passed, 17000 B
                // got the 431, and the same 20 KB forced to HTTP/1.1 passed
                // (h1's limits are generous), which is why only h2 clients with
                // fat jars - i.e. browsers, not the CLI tools - ever saw it.
                //
                // The `http1()` calls are NOT optional decoration: they
                // replicate what hudsucker sets when no builder is supplied
                // (`proxy/mod.rs`), and dropping them would silently give up
                // header-case preservation for every upstream.
                let mut server = ServerBuilder::new(TokioExecutor::new());
                server
                    .http1()
                    .title_case_headers(true)
                    .preserve_header_case(true);
                server.http2().max_header_list_size(64 * 1024);
                let proxy = match Proxy::builder()
                    .with_listener(listener)
                    .with_ca(ca)
                    .with_http_connector(https)
                    .with_server(server)
                    .with_http_handler(handler)
                    .with_graceful_shutdown(async move {
                        let _ = shutdown_rx.await;
                    })
                    .build()
                {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = ready_tx.send(Err(format!("building proxy: {e}")));
                        return;
                    }
                };

                let _ = ready_tx.send(Ok(()));
                // Bring up the PAC responder on the engine runtime; it dies with
                // the runtime when the engine stops. Non-fatal if it can't start
                // - the proxy still runs, WinINET just fails the PAC fetch and
                // falls back to DIRECT (no interception) rather than stranding
                // traffic.
                #[cfg(any(target_os = "windows", target_os = "macos"))]
                {
                    match tokio::net::TcpListener::from_std(pac_listener) {
                        Ok(pac) => {
                            tokio::spawn(serve_pac(pac, pac_rules_rx, port, upstream_proxy));
                        }
                        Err(e) => eprintln!("gate proxy PAC listener failed to start: {e}"),
                    }
                }
                // Bring up the CLI reverse-proxy relay on the same runtime;
                // like the PAC responder it dies with the runtime on stop.
                // Non-fatal: if it can't adopt its listener the MITM proxy
                // still runs, only CLI tools pointed at the relay fail.
                if let Err(e) = super::relay::spawn(
                    relay_listener,
                    relay_gateway,
                    relay_key_rx,
                    relay_token_rx,
                    relay_org_rx,
                    relay_mode_rx,
                    relay_intercept_rx,
                    relay_owner_uid,
                ) {
                    eprintln!("gate proxy relay failed to start: {e}");
                }
                if let Err(e) = proxy.start().await {
                    eprintln!("gate proxy engine stopped with error: {e}");
                }
            });

            // Loop ended. If it wasn't a deliberate stop, run the fail-safe.
            if !stopping_thread.load(Ordering::Acquire) {
                on_unexpected_exit();
            }
        })
        .context("spawning proxy engine thread")?;

    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => Ok(RunningEngine {
            port,
            relay_port,
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            pac_port,
            shutdown: Some(shutdown_tx),
            thread: Some(thread),
            rules_tx,
            key_tx,
            token_tx,
            org_tx,
            mode_tx,
            relay_intercept_tx,
            stopping,
        }),
        Ok(Err(e)) => anyhow::bail!("proxy engine failed to start: {e}"),
        Err(_) => anyhow::bail!("proxy engine did not signal readiness within 10s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_swaps_authority_keeps_path_and_injects_headers() {
        let gateway: Uri = "https://gateway-staging.constellationgate.ai"
            .parse()
            .unwrap();
        let mut req = Request::builder()
            .method("POST")
            .uri("https://api.anthropic.com/v1/messages?beta=true")
            .header("authorization", "Bearer app-token")
            .body(())
            .unwrap();

        // No OAuth token: legacy API-key header, and no org header.
        apply_rewrite(
            &mut req,
            &gateway,
            "https://api.anthropic.com",
            "sk-gw-test",
            None,
            None,
            BillingMode::Byok,
        )
        .unwrap();

        assert_eq!(
            req.uri().to_string(),
            "https://gateway-staging.constellationgate.ai/v1/messages?beta=true"
        );
        assert_eq!(req.headers().get("x-gate-api-key").unwrap(), "sk-gw-test");
        assert!(req.headers().get("x-gate-org-id").is_none());
        assert_eq!(
            req.headers().get("x-gate-upstream-url").unwrap(),
            "https://api.anthropic.com"
        );
        // The app's own credential is preserved.
        assert_eq!(
            req.headers().get("authorization").unwrap(),
            "Bearer app-token"
        );
    }

    #[test]
    fn rewrite_with_oauth_token_injects_bearer_not_api_key() {
        let gateway: Uri = "https://gateway-staging.constellationgate.ai"
            .parse()
            .unwrap();
        let mut req = Request::builder()
            .method("POST")
            .uri("https://api.anthropic.com/v1/messages")
            .header("authorization", "Bearer app-token")
            .body(())
            .unwrap();

        apply_rewrite(
            &mut req,
            &gateway,
            "https://api.anthropic.com",
            "sk-gw-test",
            Some("cognito-access-token"),
            Some("org-uuid-1"),
            BillingMode::Byok,
        )
        .unwrap();

        // OAuth token wins: bearer on x-gate-authorization, no api-key header,
        // and the selected org rides on x-gate-org-id.
        assert_eq!(
            req.headers().get("x-gate-authorization").unwrap(),
            "Bearer cognito-access-token"
        );
        assert_eq!(req.headers().get("x-gate-org-id").unwrap(), "org-uuid-1");
        assert!(req.headers().get("x-gate-api-key").is_none());
        assert_eq!(
            req.headers().get("x-gate-upstream-url").unwrap(),
            "https://api.anthropic.com"
        );
        // The app's own credential is still preserved.
        assert_eq!(
            req.headers().get("authorization").unwrap(),
            "Bearer app-token"
        );
    }

    /// PAYG is defined by what is NOT on the request: no upstream hint (the
    /// gateway's switch into reseller routing) and no credential of the app's
    /// own (which the gateway would classify as a passthrough token, forcing
    /// BYOK and then refusing the request for want of an upstream URL).
    #[test]
    fn payg_rewrite_drops_the_upstream_hint_and_the_apps_own_credential() {
        let gateway: Uri = "https://gateway-staging.constellationgate.ai"
            .parse()
            .unwrap();
        let mut req = Request::builder()
            .method("POST")
            .uri("https://api.anthropic.com/v1/messages?beta=true")
            .header("authorization", "Bearer sk-ant-oat01-app-token")
            .header("x-api-key", "sk-ant-api03-app-key")
            .body(())
            .unwrap();

        apply_rewrite(
            &mut req,
            &gateway,
            "https://api.anthropic.com",
            "sk-gw-test",
            None,
            None,
            BillingMode::Payg,
        )
        .unwrap();

        // Still repointed at the gateway, path and query intact.
        assert_eq!(
            req.uri().to_string(),
            "https://gateway-staging.constellationgate.ai/v1/messages?beta=true"
        );
        // Our own credential still identifies the workspace.
        assert_eq!(req.headers().get("x-gate-api-key").unwrap(), "sk-gw-test");
        // The two absences that ARE pay-as-you-go.
        assert!(
            req.headers().get("x-gate-upstream-url").is_none(),
            "the upstream hint's absence is what selects reseller routing"
        );
        assert!(
            req.headers().get("authorization").is_none(),
            "a provider token here would be read as passthrough and force BYOK"
        );
        assert!(req.headers().get("x-api-key").is_none());
    }

    /// A caller that sets the upstream hint itself must not be able to force
    /// BYOK - and so spend the tool's own provider credential - on an account
    /// that is in PAYG.
    #[test]
    fn payg_rewrite_removes_a_caller_supplied_upstream_hint() {
        let gateway: Uri = "https://gateway-staging.constellationgate.ai"
            .parse()
            .unwrap();
        let mut req = Request::builder()
            .method("POST")
            .uri("https://api.anthropic.com/v1/messages")
            .header("x-gate-upstream-url", "https://api.anthropic.com")
            .body(())
            .unwrap();

        apply_rewrite(
            &mut req,
            &gateway,
            "https://api.anthropic.com",
            "sk-gw-test",
            None,
            None,
            BillingMode::Payg,
        )
        .unwrap();

        assert!(req.headers().get("x-gate-upstream-url").is_none());
    }

    /// Exercises the `/proc/net/tcp` parse (incl. the address byte-swap) against
    /// a real loopback socket owned by this test process: the resolved UID must
    /// be our own.
    #[cfg(target_os = "linux")]
    #[test]
    fn peer_uid_resolves_own_loopback_connection() {
        use std::net::{TcpListener, TcpStream};
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let server_addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(server_addr).expect("connect loopback");
        // From the server's perspective the peer is the client's local address.
        let peer = client.local_addr().unwrap();
        // SAFETY: geteuid never fails.
        let me = unsafe { libc::geteuid() };
        assert_eq!(peer_uid_for(peer), Some(me));
        // A port nothing owns resolves to None (fails closed).
        let unused = std::net::SocketAddr::from(([127, 0, 0, 1], 1));
        assert_eq!(peer_uid_for(unused), None);
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    #[test]
    fn pac_routes_only_listed_hosts_to_proxy() {
        let domains = vec![ProxyDomain {
            slug: "anthropic".into(),
            display_name: "Anthropic".into(),
            hosts: vec!["api.anthropic.com".into(), "API.OTHER.com".into()],
            upstream_url: "https://api.anthropic.com".into(),
            rewrite_prefixes: vec!["/v1/".into()],
            passthrough_prefixes: vec![],
            enabled: true,
            supported: true,
        }];

        // No prior proxy: listed hosts hit the engine, everything else DIRECT.
        let pac = pac_script(&domains, 8123, None);
        assert!(pac.contains("if (h === \"api.anthropic.com\") return \"PROXY 127.0.0.1:8123\";"));
        assert!(pac.contains("if (h === \"api.other.com\") return \"PROXY 127.0.0.1:8123\";"));
        assert!(pac.trim_end().ends_with("return \"DIRECT\";\n}"));
        assert!(!pac.contains("teams"));

        // With a prior proxy: Gate hosts still hit the engine, plain hostnames
        // stay direct, and everything else falls back to the upstream proxy.
        let pac = pac_script(&domains, 8123, Some("proxy.corp.com:8080"));
        assert!(pac.contains("if (h === \"api.anthropic.com\") return \"PROXY 127.0.0.1:8123\";"));
        assert!(pac.contains("if (isPlainHostName(h)) return \"DIRECT\";"));
        assert!(pac
            .trim_end()
            .ends_with("return \"PROXY proxy.corp.com:8080\";\n}"));
    }
}
