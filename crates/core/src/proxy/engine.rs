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
    rcgen::{Issuer, KeyPair},
    rustls::crypto::aws_lc_rs,
    Body, HttpContext, HttpHandler, Proxy, RequestOrResponse,
};
use tokio::sync::{oneshot, watch};

use crate::proxy::cert_authority::GateCa;
use crate::proxy::{decide, should_intercept_host, Decision, ProxyDomain};

/// Everything the engine needs to run one session. The account + CA are
/// fixed for the engine's lifetime; the domain set can be updated live via
/// [`RunningEngine::update_domains`].
pub struct EngineConfig {
    /// Gate gateway base URL - the rewrite target authority.
    pub gateway_base_url: String,
    /// Gate API key, injected as `X-Gate-Api-Key`.
    pub api_key: String,
    /// Full domain catalog; the engine routes only the `enabled` ones.
    pub domains: Vec<ProxyDomain>,
    /// PEM of the local root CA cert (public).
    pub ca_cert_pem: String,
    /// PEM of the local root CA private key.
    pub ca_key_pem: String,
    /// Preferred loopback port to bind. `Some(p)` asks the engine to reuse a
    /// previously-chosen port so a restart keeps the same address - the system
    /// proxy pointer baked into a login session stays valid across app
    /// restarts instead of dangling at a dead ephemeral port. Falls back to an
    /// ephemeral port if `p` is taken (or `None`). Linux sets this; macOS and
    /// Windows pass `None` (their system proxy is read live, so a changing port
    /// never strands a frozen session).
    pub preferred_port: Option<u16>,
    /// When `Some(uid)`, only connections from that local UID are intercepted
    /// (MITM'd + rewritten with the Gate key injected); traffic from any other
    /// local user is blind-tunnelled. The loopback listener is plain TCP and
    /// reachable by every local user, so this stops a *different* account from
    /// spending the owner's Gate key through the proxy. Linux sets the daemon's
    /// own UID; macOS/Windows pass `None` (out of scope for this release).
    pub owner_uid: Option<u32>,
    /// The user's pre-existing upstream proxy , used as the
    /// PAC fallback so non-Gate traffic keeps flowing through it instead of
    /// going DIRECT while routing is on. PAC-driven platforms only
    /// ; Linux uses env-var proxies with no PAC.
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    pub upstream_proxy: Option<String>,
}

/// A running engine. Dropping it signals graceful shutdown (fail-safe so a
/// crashed caller never leaves the loopback listener orphaned); [`stop`]
/// also joins the thread.
///
/// [`stop`]: RunningEngine::stop
pub struct RunningEngine {
    port: u16,
    /// Loopback port serving the PAC script the system proxy points at.
    /// PAC-driven platforms only (Windows `AutoConfigURL`, macOS
    /// `networksetup -setautoproxyurl`); Linux uses env-var proxies with no PAC.
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    pac_port: u16,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
    rules_tx: watch::Sender<Arc<Vec<ProxyDomain>>>,
    key_tx: watch::Sender<Arc<str>>,
    /// Set before a deliberate shutdown so the engine thread can tell an
    /// expected stop from an unexpected exit (crash / bind loss).
    stopping: Arc<AtomicBool>,
}

impl RunningEngine {
    pub fn port(&self) -> u16 {
        self.port
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

/// Whether to emit per-request engine logs to stderr. Off unless
/// `GATE_PROXY_DEBUG` is set in the environment, so production stays quiet.
fn debug_log() -> bool {
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
#[cfg(target_os = "linux")]
fn peer_uid_for(addr: std::net::SocketAddr) -> Option<u32> {
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
fn peer_uid_for(_addr: std::net::SocketAddr) -> Option<u32> {
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
            eprintln!(
                "[gate-proxy] CONNECT {} -> {}",
                host.unwrap_or("?"),
                if intercept { "intercept" } else { "tunnel" }
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
            if let (Decision::Rewrite { upstream_url }, true) =
                (decide(&rules, host, &path), self.peer_allowed(ctx))
            {
                let api_key = self.api_key.borrow().clone();
                match apply_rewrite(&mut req, &self.gateway, &upstream_url, &api_key) {
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
/// gateway's, keep the original path/query, and inject the Gate headers.
/// The app's own auth header (bearer / `x-api-key`) is left intact -
/// Gate validates `X-Gate-Api-Key` and forwards the rest.
pub(crate) fn apply_rewrite<T>(
    req: &mut Request<T>,
    gateway: &Uri,
    upstream_url: &str,
    api_key: &str,
) -> Result<()> {
    let gw = gateway.clone().into_parts();
    let mut parts = req.uri().clone().into_parts();
    parts.scheme = gw.scheme;
    parts.authority = gw.authority;
    *req.uri_mut() = Uri::from_parts(parts).context("rebuilding rewritten request URI")?;

    let headers = req.headers_mut();
    headers.insert(
        "x-gate-api-key",
        HeaderValue::from_str(api_key).context("building x-gate-api-key header")?,
    );
    headers.insert(
        "x-gate-upstream-url",
        HeaderValue::from_str(upstream_url).context("building x-gate-upstream-url header")?,
    );
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
        Some(p) => std::net::TcpListener::bind(("127.0.0.1", p))
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
    listener
        .set_nonblocking(true)
        .context("setting the loopback listener non-blocking")?;
    Ok((listener, port))
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
    // Windows points WinINET at a PAC served on its own loopback port (see
    // `serve_pac`); the proxy port itself is baked into the PAC body.
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let (pac_listener, pac_port) = bind_loopback(None).context("binding the PAC loopback port")?;

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
    let handler = GateHandler {
        rules: rules_rx,
        gateway,
        api_key: key_rx,
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

                let proxy = match Proxy::builder()
                    .with_listener(listener)
                    .with_ca(ca)
                    .with_rustls_connector(aws_lc_rs::default_provider())
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
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            pac_port,
            shutdown: Some(shutdown_tx),
            thread: Some(thread),
            rules_tx,
            key_tx,
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

        apply_rewrite(
            &mut req,
            &gateway,
            "https://api.anthropic.com",
            "sk-gw-test",
        )
        .unwrap();

        assert_eq!(
            req.uri().to_string(),
            "https://gateway-staging.constellationgate.ai/v1/messages?beta=true"
        );
        assert_eq!(req.headers().get("x-gate-api-key").unwrap(), "sk-gw-test");
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
