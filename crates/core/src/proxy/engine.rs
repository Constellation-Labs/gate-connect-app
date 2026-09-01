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

use std::borrow::Cow;
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
use rand::Rng;
use tokio::sync::{oneshot, watch};

use crate::proxy::cert_authority::GateCa;
use crate::proxy::{
    browser_ua_without_product_token, classify_client, decide, domain_claiming_host,
    is_non_browser_ua, rules_for_client, should_decline_upgrade, should_intercept_host,
    ClientClass, Decision, ProxyDomain,
};

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
    /// across app restarts instead of dangling at a dead port. Falls back to
    /// a fresh pick from the stable band ([`bind_fresh`]) if `p` is taken (or
    /// `None`). All three
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
    /// Captured chatgpt.com `cf_clearance` cookie for app turns; empty when
    /// none is held. See [`update_cf_clearance`](Self::update_cf_clearance).
    cf_clearance_tx: watch::Sender<Arc<str>>,
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

    /// Loopback port serving the PAC script, and on Windows the CA's CRL (see
    /// `cert_authority::CRL_PATH`).
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

    /// Push a captured chatgpt.com `cf_clearance` cookie to the live engine.
    /// Empty string clears it. Cheap - no restart; this is how a freshly
    /// solved Cloudflare challenge reaches in-flight app turns.
    pub fn update_cf_clearance(&self, cf_clearance: &str) {
        let _ = self.cf_clearance_tx.send(Arc::from(cf_clearance));
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

    let Some(path) = crate::env::test_seam("GATE_CONNECT_TEST_CA") else {
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

/// Whether to decline a Responses WebSocket upgrade so the client falls back to
/// HTTP and its turn becomes visible. Off unless `GATE_PROXY_WS_DOWNGRADE` is
/// set; see [`should_decline_upgrade`] for what it then applies to.
///
/// Default-OFF is the important half. The failure mode if the fallback does not
/// fire is not "work mode stays uncaptured", which is the status quo, but "work
/// mode does not work at all" - strictly worse than doing nothing. So this stays
/// opt-in until measured on a real client.
///
/// An env var, matching [`debug_log`], because that is the right shape for the
/// measurement stages this is for: a single machine, deliberately switched on.
/// Before any broader rollout it wants promoting to a real setting, since
/// `OnceLock` fixes the value for the process and a user cannot turn it off
/// without relaunching the app - too slow for a kill switch if a vendor update
/// ever removes the fallback.
pub(crate) fn responses_ws_downgrade() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("GATE_PROXY_WS_DOWNGRADE").is_some())
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

/// One intercepted chatgpt.com request, as `handle_request` saw it. Carried on
/// the handler so the response hook can name what it is answering; see
/// [`GateHandler::chatgpt_turn`].
#[derive(Clone)]
struct ChatgptTurn {
    client: crate::proxy::ClientClass,
    method: Method,
    path: String,
    /// Whether it was rewritten to the gateway (so it egresses from Gate's IP)
    /// or passed through to chatgpt.com (the user's own IP). The distinction
    /// the Cloudflare investigation turns on.
    rewritten: bool,
    /// Whether a captured `cf_clearance` was merged into this request. Reads
    /// as "our cookie was on the wire" without putting a credential-bearing
    /// header anywhere near the log.
    cf_injected: bool,
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
    /// Live-updatable chatgpt.com `cf_clearance` cookie, captured by the GUI's
    /// challenge-solve webview. Empty string means "none held"; merged into
    /// the `cookie` header on every intercepted chatgpt.com app request,
    /// rewritten and passthrough alike (see [`inject_cf_clearance`] and the
    /// injection site's comment for why passthrough needs it too).
    cf_clearance: watch::Receiver<Arc<str>>,
    /// What `handle_request` saw on an intercepted chatgpt.com request, kept
    /// so the response hook can name what it is answering: a response arrives
    /// with no host, path, or client context of its own. `None` on every
    /// other host.
    ///
    /// Recorded for every client class, not just the app. Arming the solve
    /// webview stays app-only (see [`cf_challenge_detected`]) - a browser
    /// runs its own interstitial - but keeping the browser's turns in the
    /// memo is what let the app's challenged turn be compared against the
    /// website's succeeding one on the same endpoint, which is how the
    /// user-agent finding was made.
    ///
    /// Reliable per request: hudsucker clones the handler per request and
    /// runs both hooks on that clone (`internal.rs`: `service_fn` ->
    /// `self.clone().proxy(req)`), so the memo cannot leak across requests,
    /// HTTP/2 multiplexing included.
    chatgpt_turn: Option<ChatgptTurn>,
    /// Whether *this* request was rewritten to the gateway carrying our OAuth
    /// bearer. Memoed like [`chatgpt_turn`](Self::chatgpt_turn) and for the
    /// same reason: the response hook has no request context of its own, and a
    /// 401 only implicates our session if we were the ones who authenticated
    /// the call. False for API-key routing (a refused `sk-gw-*` key is a
    /// different problem, with a different fix) and for everything we passed
    /// through untouched.
    injected_oauth: bool,
    /// When `Some`, only intercept connections from this local UID (see
    /// [`EngineConfig::owner_uid`]).
    owner_uid: Option<u32>,
    /// Per-connection memo of the owner-UID verdict, keyed by peer address.
    /// hudsucker clones the handler per connection, so this is resolved once -
    /// while the peer's socket is definitely still in `/proc/net/tcp` - instead
    /// of re-reading it (and risking a TOCTOU miss) on every request.
    peer_verdict: Option<(std::net::SocketAddr, bool)>,
    /// Set from the explicit proxy selector on Claude Code's CONNECT request.
    /// The handler is cloned with this value for the decrypted inner requests.
    claude_code_route: bool,
    /// One-shot latch for the "Anthropic without the selector" line in
    /// `should_intercept`. Shared across handler clones, so an engine run
    /// reports it once instead of once per connection.
    anthropic_unselected_logged: Arc<AtomicBool>,
    /// One-shot latch for the "unrecognised app shell" line in
    /// `handle_request`, shared across handler clones for the same reason.
    app_shell_unrecognised_logged: Arc<AtomicBool>,
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

    /// Report a tunnelled CONNECT to Claude Code's own destination that carried
    /// no route selector.
    ///
    /// Outside `debug_log`, unlike every other line here, because this is the
    /// only place in the app that can observe the regression it names: Claude
    /// Code's `status()` compares `settings.json` against our own write and
    /// never learns whether the engine actually received the selector, so a
    /// future release that stopped deriving `Proxy-Authorization` from the
    /// proxy URL's userinfo would blind-tunnel behind a green pill - the O1
    /// failure class in `docs/routing-architecture.md`. The wire is the only
    /// witness.
    ///
    /// Latched to one line per engine run. The same state is the ordinary,
    /// harmless one for any other Anthropic client while the Desktop switch is
    /// off, so this cannot be a per-CONNECT warning without becoming noise -
    /// and noise is what stops a real regression being read.
    fn warn_if_anthropic_is_unselected(&self, intercept: bool, host: Option<&str>) {
        if intercept
            || self.claude_code_route
            || !host.is_some_and(|h| crate::proxy::claude_code_route_domain().matches_host(h))
            || self
                .anthropic_unselected_logged
                .swap(true, Ordering::Relaxed)
        {
            return;
        }
        eprintln!(
            "[gate-proxy] CONNECT {} carried no Claude Code route selector -> tunnel. \
             Expected for other Anthropic clients while their domain is off; if this \
             is a connected Claude Code, it is bypassing Gate.",
            host.unwrap_or("?")
        );
    }

    /// Report a ChatGPT request from a client that is neither the website nor
    /// anything `classify_client` recognises as the app.
    ///
    /// Outside `debug_log`, for the reason above: the wire is the only witness.
    /// The app shell's user-agent has been renamed once already
    /// (`ChatGPTBrowser` -> `CodexBrowser`), and the roster it was matched
    /// against went stale without a single symptom - the app simply classified
    /// as `Unknown`, which routes identically, so nothing looked wrong. This
    /// line is what makes the next rename arrive as a message naming the new
    /// token instead of as app-only handling quietly ceasing to apply.
    ///
    /// The trigger is deliberately wider than the classifier's own
    /// [`super::browser_ua_without_product_token`]: any non-browser user-agent
    /// will do, so a rename that also drops the wrapped `Mozilla/` still
    /// surfaces. That width is why it is latched to one line per engine run -
    /// an unrecognised non-browser client here is the ordinary, harmless state
    /// for anything third-party, and a per-request warning would be noise.
    fn warn_if_an_app_shell_is_unrecognised(
        &self,
        client: ClientClass,
        rules: &[ProxyDomain],
        host: Option<&str>,
        ua: &str,
    ) {
        if !app_shell_is_unrecognised(client, rules, host, ua)
            || self
                .app_shell_unrecognised_logged
                .swap(true, Ordering::Relaxed)
        {
            return;
        }
        eprintln!(
            "[gate-proxy] {} request from an unrecognised client: user-agent {ua:?} \
             classifies as Unknown. Expected for third-party clients; if this is the \
             ChatGPT/Codex app, its shell has been renamed again and app-only handling \
             no longer applies to it (see `classify_client`).",
            host.unwrap_or("?")
        );
    }
}

/// The gating condition of `warn_if_an_app_shell_is_unrecognised`, kept free of
/// the latch and the printing so it can be tested: an `Unknown` classification,
/// a non-browser user-agent, and a host claimed by the `chatgpt-apps` entry.
///
/// Checked against the unnarrowed live rules, and [`domain_claiming_host`]
/// includes disabled entries: this reports on a client, not on a route. Note
/// the reach that buys is bounded by `should_intercept` - a request only gets
/// here when some enabled entry MITMs the host - so with every chatgpt.com
/// entry off the warning cannot fire; the disabled-entry width matters when a
/// sibling entry on the same host is the one switched on.
fn app_shell_is_unrecognised(
    client: ClientClass,
    rules: &[ProxyDomain],
    host: Option<&str>,
    ua: &str,
) -> bool {
    client == ClientClass::Unknown
        && is_non_browser_ua(ua)
        && host
            .and_then(|h| domain_claiming_host(rules, h))
            .is_some_and(|d| d.slug == "chatgpt-apps")
}

/// The rule set a connection's requests must be decided against: the live
/// catalog, plus Claude Code's own entry force-enabled when the CONNECT carried
/// the route selector and the user has that entry switched off.
///
/// Shared by both stages - the CONNECT verdict and the per-request routing - so
/// the two can never disagree about what the selector routes.
///
/// Scoped to that entry's own host, which is what keeps the selector from
/// widening anything: a selected connection to any other host sees the live
/// catalog untouched. It also means only Claude Code's host pays for the clone;
/// everything else borrows.
fn route_rules<'a>(
    live: &'a [ProxyDomain],
    claude_code_route: bool,
    host: Option<&str>,
) -> Cow<'a, [ProxyDomain]> {
    let forced = crate::proxy::claude_code_route_domain();
    let needed = claude_code_route
        && host.is_some_and(|h| forced.matches_host(h) && !should_intercept_host(live, h));
    if !needed {
        return Cow::Borrowed(live);
    }
    let mut rules = live.to_vec();
    rules.push(forced.clone());
    Cow::Owned(rules)
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
        let live_rules = self.rules.borrow().clone();
        let host = req
            .uri()
            .authority()
            .map(|a| a.host())
            .or_else(|| req.uri().host());
        let rules = route_rules(&live_rules, self.claude_code_route, host);
        let intercept = host
            .map(|h| should_intercept_host(&rules, h))
            .unwrap_or(false);
        self.warn_if_anthropic_is_unselected(intercept, host);
        if debug_log() {
            // The enabled set is printed alongside the verdict because the two
            // have been observed to disagree with what `proxy enable` reports:
            // the CLI listed anthropic and openrouter as on while the engine
            // tunnelled both. Without this the log cannot distinguish "the host
            // is not in the catalog" from "the engine is holding a different
            // catalog than the one just configured".
            // `live_rules`, not the possibly-forced set: the disagreement this
            // is here to catch is between the catalog the engine holds and the
            // one that was configured, so the line must report what it holds.
            let enabled: Vec<&str> = live_rules
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
                live_rules.len(),
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
            self.claude_code_route = req
                .headers()
                .get("proxy-authorization")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value == crate::proxy::CLAUDE_CODE_PROXY_AUTH);
            // A proxy credential is hop-by-hop and must never reach either the
            // real Anthropic endpoint or Gate.
            req.headers_mut().remove("proxy-authorization");
            return req.into();
        }
        // Fresh verdict per request; only the chatgpt.com block below sets it.
        self.chatgpt_turn = None;
        // Likewise: only a successful OAuth-bearing rewrite below sets this.
        self.injected_oauth = false;
        // Some entries route every proxy-honouring client EXCEPT the vendor's own
        // website, which shares their host (see `BROWSER_ROUTED`).
        // Classify before `decide` and hand it a narrowed rule set, so the
        // client filter composes with the `enabled` one instead of adding a
        // second kind of veto inside the routing decision.
        let header = |name: &str| req.headers().get(name).and_then(|v| v.to_str().ok());
        let client = classify_client(header);
        let ua = header("user-agent").unwrap_or_default();
        let host = req.uri().host().map(str::to_owned);
        let live_rules = self.rules.borrow().clone();
        let rules = rules_for_client(
            &route_rules(&live_rules, self.claude_code_route, host.as_deref()),
            client,
        );
        // Gated like the rewrite below: a plain-HTTP request from a non-owner
        // local peer reaches here without passing `should_intercept`, and its
        // user-agent does not belong in the owner's log.
        if self.peer_allowed(ctx) {
            self.warn_if_an_app_shell_is_unrecognised(client, &live_rules, host.as_deref(), ua);
        }
        // Path only, never `path_and_query()`: some providers pass the API key
        // as a URL query param (e.g. Google `...?key=...`), and this value is
        // written to the debug log below. `Uri::path()` excludes the query, so
        // URL-embedded keys never reach the log. Keep it that way.
        let path = req.uri().path().to_owned();
        let mut action = "passthrough";
        // A protocol upgrade is never rewritten to the gateway. This is the end
        // state, not a holding position: Gate does not carry upgraded protocols,
        // by decision. It has no WebSocket transport, and `buildForwardHeaders`
        // strips `upgrade` and `connection` outright
        // (gate: utils/proxy-helpers.ts), so a handshake forwarded there can only
        // come back as a plain response and the client's 101 never arrives.
        //
        // The combination is reachable rather than theoretical. ChatGPT's app
        // work mode NEGOTIATES its transport on `/backend-api/codex/responses`:
        // it offers `openai-beta: responses_websockets=<date>` and, when the
        // server accepts, runs the entire turn over a WebSocket. That path is
        // claimed by the `chatgpt` entry's rewrite prefix, so without this guard
        // enabling that row would break the upgraded half outright.
        //
        // What passing them through costs is therefore narrower than "all
        // visibility". The HTTP half of that same negotiation IS captured
        // normally, which means a session that upgrades is a session that
        // silently left coverage - no error, nothing to notice. `originator:
        // codex_work_desktop` on an upgrade is the signature of one, and is what
        // to grep for if work-mode turns stop appearing in Gate.
        if is_upgrade_request(&req) {
            // ...unless declining it is what makes the turn visible. The client
            // that negotiates this transport carries its own HTTP fallback, so a
            // refusal costs one extra round trip and buys the whole turn; see
            // `should_decline_upgrade`. Gated on `peer_allowed` like the rewrite
            // below, so a non-owner peer's traffic is never interfered with.
            if responses_ws_downgrade()
                && self.peer_allowed(ctx)
                && host
                    .as_deref()
                    .is_some_and(|h| should_decline_upgrade(&rules, h, &path))
            {
                if debug_log() {
                    eprintln!("[gate-proxy] {path} is a protocol upgrade -> declined (client should retry over HTTP)");
                }
                // 400, never a drop or a stall. The client distinguishes a failed
                // handshake from a timed-out one and only the first falls back
                // immediately; a black-holed connection would add the full
                // handshake timeout to EVERY turn and be indistinguishable, from
                // the user's side, from Gate being broken.
                return RequestOrResponse::Response(decline_upgrade_response());
            }
            if debug_log() {
                eprintln!("[gate-proxy] {path} is a protocol upgrade -> passthrough (gate has no websocket transport)");
            }
        } else if let Some(host) = host.as_deref() {
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
                let token = self.token.borrow().clone();
                let oauth_token = (!token.is_empty()).then(|| token.as_ref());
                let org = self.org.borrow().clone();
                let org_id = (!org.is_empty()).then(|| org.as_ref());
                match apply_rewrite(
                    &mut req,
                    &self.gateway,
                    &upstream_url,
                    &api_key,
                    oauth_token,
                    org_id,
                ) {
                    Ok(()) => {
                        action = "rewrite->gateway";
                        // Remember that the gateway is answering *our*
                        // credential, so a 401 on the way back can be read as
                        // evidence about the session (see `handle_response`).
                        // Read back what actually went on the wire rather than
                        // what we offered: holding a token is not the same as
                        // sending it, and `inject_gate_credential` leaves a
                        // request alone when the client brought its own
                        // `x-gate-api-key`.
                        self.injected_oauth = req
                            .headers()
                            .contains_key(crate::proxy::GATE_AUTHORIZATION_HEADER);
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
        // EVERY intercepted chatgpt.com app request - not only the rewritten
        // turns - arms challenge detection and carries the captured
        // clearance. The app has no cookie jar of its own, so its passthrough
        // calls (settings, models, sentinel) go out bare through the engine's
        // TLS stack and Cloudflare challenges those too - observed as 403
        // text/html across the whole warm-up sequence, which kills the app
        // before it ever issues a rewritten turn. Scoping either half to the
        // rewrite path deadlocks: no rewritten turn, so no solve webview, so
        // no cookie, so the warm-up keeps 403ing. The browser survives the
        // identical path because its own jar carries `cf_clearance` on every
        // request to the host (and it can render a challenge itself, which is
        // why Web is excluded here); this does for the app what the jar does
        // for the browser. Gated on `peer_allowed` like the rewrite above,
        // and `inject_cf_clearance` never clobbers a cookie the client sent
        // itself.
        if host.as_deref() == Some("chatgpt.com") && self.peer_allowed(ctx) {
            let rewritten = action == "rewrite->gateway";
            let mut cf_injected = false;
            if client == crate::proxy::ClientClass::App {
                // The solve webview wears this UA, or Cloudflare never
                // challenges it and there is no cookie to capture. Recorded
                // BEFORE any strip below, so the webview keeps wearing what
                // the app actually sends.
                crate::proxy::record_chatgpt_app_user_agent(
                    req.headers()
                        .get(hudsucker::hyper::header::USER_AGENT)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default(),
                );

                let cf = self.cf_clearance.borrow().clone();
                // Strip the app shell's product token from the `user-agent`
                // on chatgpt.com turns we forward through the gateway,
                // leaving the browser-shaped remainder the token was
                // prefixed to.
                //
                // What this is testing. Measured 2026-08-28 on one machine,
                // same endpoint, same gateway, same Cloudflare datacenter
                // (`cf-ray` colo `IAD`): `POST /backend-api/f/conversation`
                // from the website answered 200 carrying a single `oai-did`
                // cookie, while the same path from the app answered
                // `cf-mitigated: challenge` carrying MORE cookies, including
                // a freshly solved `cf_clearance`. So neither the cookie nor
                // Gate's egress IP explains the difference, and the app's
                // user-agent is character-for-character the website's with
                // `CodexBrowser ` prefixed. The strip exists to confirm or
                // refute that the prefix is what the challenge rule keys on.
                //
                // Careful: it ships a request that names a different client
                // than the one that sent it, to a third party's bot
                // management; it is fragile (Cloudflare fingerprints far
                // more than this header, so a result today says nothing
                // about next month); and it erases the signal the vendor
                // uses to tell its own clients apart.
                //
                // Skipped whenever a captured `cf_clearance` is about to be
                // injected below: the cookie is bound to the user-agent it
                // was minted under - the full shell UA the solve webview
                // wears - so stripping here would replay it under a UA
                // Cloudflare never issued it to, and the challenge the
                // cookie exists to clear would just fire again.
                if rewritten && cf.is_empty() {
                    let stripped = req
                        .headers()
                        .get(hudsucker::hyper::header::USER_AGENT)
                        .and_then(|v| v.to_str().ok())
                        .and_then(browser_ua_without_product_token)
                        .and_then(|ua| HeaderValue::from_str(ua).ok());
                    if let Some(ua) = stripped {
                        req.headers_mut()
                            .insert(hudsucker::hyper::header::USER_AGENT, ua);
                    }
                }
                if !cf.is_empty() {
                    inject_cf_clearance(&mut req, &cf);
                    cf_injected = true;
                }
            }
            self.chatgpt_turn = Some(ChatgptTurn {
                client,
                method: req.method().clone(),
                path: path.clone(),
                rewritten,
                cf_injected,
            });
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
            // `client` is in here because an app-only entry declining a request
            // and the host not being routed at all both surface as
            // `passthrough`. Without it the log cannot tell "we chose not to
            // take the browser's traffic" from "this domain is off", which is
            // the same class of ambiguity the CONNECT line above prints the
            // enabled set to resolve. Neither signal it reads is a credential.
            eprintln!(
                "[gate-proxy] {} {}{} [{ver}] auth={auth} client={client:?} -> {action}",
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
        // A Cloudflare managed challenge answering a chatgpt.com app turn:
        // the app shell has no HTML/JS surface to run the interstitial, so
        // notify the GUI to open the one-time solve webview. Side-channel
        // notification only - the response is returned unchanged either way.
        if cf_challenge_detected(self.chatgpt_turn.as_ref(), &res) {
            crate::proxy::notify_cf_challenge_observer();
        }
        // The gateway refused a call we authenticated with the OAuth bearer.
        // Tell the shell so it can re-verify the session; the response is
        // returned unchanged either way, exactly like the challenge notify
        // above - a tool that can handle its own 401 must still see it.
        //
        // Status only: the error code that names the reason
        // (`invalid_gate_token`) is in the body, and the body is a stream we
        // must not consume on its way to the client. The status alone is
        // deliberately a weak signal - a rewritten request also carries the
        // client's own upstream credential, so this 401 may not be about us at
        // all - which is why the observer's job is to go and ask the gateway
        // directly rather than to conclude anything from here.
        if self.injected_oauth && res.status() == hudsucker::hyper::StatusCode::UNAUTHORIZED {
            crate::proxy::notify_gate_auth_observer();
        }
        if debug_log() {
            eprintln!(
                "[gate-proxy] <- {} ct={:?}",
                res.status(),
                res.headers()
                    .get(hudsucker::hyper::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
            );
            // One correlated line per chatgpt.com surface we handle. The
            // status line above cannot be paired with its request - responses
            // arrive interleaved under HTTP/2 and name neither path nor
            // client - which made every capture a guess about which 403
            // belonged to what.
            //
            // Scoped to the turns we route or inject into, so a page load's
            // hundred asset fetches stay out of it. `rewritten` says which
            // egress answered (Gate's IP or the user's) and `cf-ray`'s colo
            // suffix corroborates it; `cf-injected` says whether our captured
            // cookie rode along; `cf-mitigated` is Cloudflare's own marker,
            // relayed by the gateway (`CHALLENGE_RELAY_HEADERS` in gate's
            // proxy-helpers.ts), and its absence on a 403 means a plain WAF
            // block that no cookie would fix. `set-cookie` is NAMES only -
            // the values are session credentials. Gate's provenance headers
            // separate a relayed Cloudflare 403 from one the gateway raised.
            if let Some(turn) = self
                .chatgpt_turn
                .as_ref()
                .filter(|t| t.rewritten || t.client == crate::proxy::ClientClass::App)
            {
                let header = |name: &str| {
                    res.headers()
                        .get(name)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("<absent>")
                        .to_owned()
                };
                let set_cookies: Vec<&str> = res
                    .headers()
                    .get_all(hudsucker::hyper::header::SET_COOKIE)
                    .iter()
                    .filter_map(|v| v.to_str().ok())
                    .filter_map(|c| c.split_once('=').map(|(name, _)| name.trim()))
                    .collect();
                eprintln!(
                    "[gate-proxy]   <- {} for {} {} client={:?} rewritten={} \
                     cf-injected={} cf-mitigated={:?} cf-ray={:?} set-cookie=[{}] \
                     gate-error-source={:?} gate-upstream-status={:?}",
                    res.status(),
                    turn.method,
                    turn.path,
                    turn.client,
                    turn.rewritten,
                    turn.cf_injected,
                    header("cf-mitigated"),
                    header("cf-ray"),
                    set_cookies.join(","),
                    header("x-gate-error-source"),
                    header("x-gate-upstream-status"),
                );
            }
        }
        res
    }
}

/// Repoint a request at the gateway: swap scheme + authority for the
/// gateway's, strip the upstream's own path prefix, and inject the Gate
/// headers. The app's own auth header (bearer / `x-api-key`) is left intact -
/// Gate validates the Gate credential and forwards the rest. The credential
/// precedence (a caller-supplied `x-gate-api-key` is respected, else OAuth
/// token wins over the legacy key) lives in [`super::inject_gate_credential`],
/// shared with the relay so the two paths can't drift.
///
/// The path strip is what keeps a provider whose API lives under a reserved
/// prefix routable: Gate appends the forwarded path to `X-Gate-Upstream-Url`,
/// so moving `/api` from the request line into the upstream URL reassembles to
/// the same provider URL while sending Gate a path its ALB won't divert. See
/// the `openrouter` catalog entry in [`super::default_domains`].
/// True when the request is asking to leave HTTP for another protocol.
///
/// Reads `Connection: upgrade` AND an `Upgrade` header, which is what RFC 9110
/// requires a real upgrade to carry, rather than keying on the WebSocket-specific
/// `Sec-WebSocket-*` set: the reason we bail applies to any upgrade, not just
/// WebSocket. `Connection` is a comma-separated list and its tokens are
/// case-insensitive.
/// The response sent in place of a declined upgrade.
///
/// Shaped like the provider's own error envelope so a client that surfaces the
/// body shows something coherent, and typed distinctly (`gate_ws_downgrade`) so
/// this is greppable in a client log and cannot be mistaken for an upstream 400.
fn decline_upgrade_response() -> hudsucker::hyper::Response<Body> {
    hudsucker::hyper::Response::builder()
        .status(hudsucker::hyper::StatusCode::BAD_REQUEST)
        .header(
            hudsucker::hyper::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )
        .body(Body::from(
            r#"{"error":{"message":"websocket transport unavailable through this proxy; retry over HTTP","type":"gate_ws_downgrade"}}"#,
        ))
        // Infallible: every part is a static, pre-validated value.
        .expect("static decline response builds")
}

pub(crate) fn is_upgrade_request<T>(req: &Request<T>) -> bool {
    let headers = req.headers();
    if !headers.contains_key(hudsucker::hyper::header::UPGRADE) {
        return false;
    }
    headers
        .get(hudsucker::hyper::header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .any(|t| t.trim().eq_ignore_ascii_case("upgrade"))
        })
}

/// Whether `res` is a Cloudflare managed challenge answering a chatgpt.com
/// **app** request. `turn` is the per-request memo set by `handle_request`
/// (see [`GateHandler::chatgpt_turn`]) - a response on any other host never
/// triggers, whatever headers it carries, and neither does the browser's:
/// it can run an interstitial itself, so opening our webview for it would be
/// a window the user never needed. `cf-mitigated` is Cloudflare's own marker
/// that the body is its interstitial rather than the origin's answer.
fn cf_challenge_detected<T>(
    turn: Option<&ChatgptTurn>,
    res: &hudsucker::hyper::Response<T>,
) -> bool {
    turn.is_some_and(|t| t.client == crate::proxy::ClientClass::App)
        && res
            .headers()
            .get("cf-mitigated")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("challenge"))
}

/// Merge a `cf_clearance` value into the request's `cookie` header without
/// disturbing cookies the client already sent (append with `; ` when a
/// header is present, set it when none is). No-op if the client already
/// carries its own `cf_clearance`.
fn inject_cf_clearance<T>(req: &mut Request<T>, cf_clearance: &str) {
    use hudsucker::hyper::header::COOKIE;
    let merged = match req.headers().get(COOKIE).and_then(|v| v.to_str().ok()) {
        Some(cookies) => {
            let has_own = cookies.split(';').any(|c| {
                c.split_once('=')
                    .is_some_and(|(name, _)| name.trim() == "cf_clearance")
            });
            if has_own {
                return;
            }
            format!("{cookies}; cf_clearance={cf_clearance}")
        }
        None => format!("cf_clearance={cf_clearance}"),
    };
    if let Ok(value) = HeaderValue::from_str(&merged) {
        req.headers_mut().insert(COOKIE, value);
    }
}

pub(crate) fn apply_rewrite<T>(
    req: &mut Request<T>,
    gateway: &Uri,
    upstream_url: &str,
    api_key: &str,
    oauth_token: Option<&str>,
    org_id: Option<&str>,
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
    super::inject_gate_credential(headers, api_key, oauth_token, org_id)?;
    headers.insert(
        super::UPSTREAM_URL_HEADER,
        HeaderValue::from_str(upstream_url).context("building x-gate-upstream-url header")?,
    );
    Ok(())
}

/// Bind a loopback listener and return it together with the port it landed on.
/// Tries `preferred` first (so a restart can reuse the same port and keep a
/// frozen system-proxy pointer valid); if that's unavailable - taken, or
/// `None` - falls back to a fresh pick from the stable band ([`bind_fresh`]). Returning the *live* listener -
/// rather than probing a port and dropping it before hudsucker binds - closes
/// the TOCTOU window where another process could grab the port in the gap. The
/// socket stays held from here until it's handed to the proxy. Set non-blocking
/// so tokio can adopt it.
fn bind_loopback(preferred: Option<u16>) -> Result<(std::net::TcpListener, u16)> {
    let listener = match preferred {
        Some(p) => bind_preferred(p)
            .or_else(|_| bind_fresh())
            .with_context(|| format!("binding loopback (preferred {p}, then a fresh port)"))?,
        None => bind_fresh().context("binding a free loopback port")?,
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

/// Port band a *fresh* listener is picked from, first free port wins. Sits
/// inside IANA's registered range and below Windows' default dynamic range
/// (49152-65535), clear of assigned neighbours like 47001 (WinRM). One engine
/// brings up three listeners (MITM, relay, PAC), so the band is sized well
/// past what a few concurrent hosts need.
const STABLE_PORT_RANGE: std::ops::Range<u16> = 47100..47200;

/// Bind a fresh loopback listener for a port the caller intends to *persist*
/// and rebind on later runs. Takes the first free port in
/// [`candidate_ports`]'s order, falling back to an OS-assigned ephemeral port
/// only if the whole band is busy.
///
/// Why not `:0` directly: a port the OS hands out as ephemeral is one it also
/// hands out to everything else, and nothing holds it while Gate is stopped -
/// so the next run's rebind races every local process that opened an outbound
/// socket in the meantime. On Windows it is worse than a race: Hyper-V, WSL2
/// and Docker Desktop reserve whole blocks of the dynamic range at boot, and a
/// persisted port inside a reserved block fails to bind on *every* start, so
/// the port silently moves each time. Either way the clients that captured the
/// old port are stranded - `HTTPS_PROXY` is read once per process, and CLI tool
/// configs bake the relay URL - and the user has to restart them. Picking from
/// a quiet band below the dynamic range means the port we persist is one
/// nothing else is handing out.
pub(super) fn bind_fresh() -> std::io::Result<std::net::TcpListener> {
    for port in candidate_ports(&persisted_ports()) {
        if let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", port)) {
            return Ok(listener);
        }
    }
    // Whole band unavailable (exhaustion, or something blanket-bound it): an
    // ephemeral port keeps this session alive, but the next run may not get
    // it back - the moving-port symptom is back, so name the reason here.
    eprintln!(
        "gate proxy: no free port in {STABLE_PORT_RANGE:?}; binding an ephemeral port instead. \
         It may not survive a restart."
    );
    std::net::TcpListener::bind(("127.0.0.1", 0))
}

/// Every port in [`STABLE_PORT_RANGE`], in the order a fresh bind should try
/// them: a random rotation of the band, with the ports this install already
/// remembers for its own listeners (`ours`) moved to the back.
///
/// The rotation is random rather than scanning from the band's start, because a
/// fixed start makes the lowest free port a contended resource: two Gate
/// processes coming up together (the app and a standalone `proxy relay`) would
/// both reach for it, and - worse - a port freed a moment ago is the first one
/// the next scan hands out, so a neighbour can snatch the port a restarting
/// listener is about to reclaim. Every port in the band satisfies the point of
/// the band, so there is nothing to gain by preferring one end of it.
///
/// Deferring `ours` keeps this install from stealing from itself. The three
/// listeners bind in sequence, so without it the MITM listener falling back to
/// a fresh port could take the very port the PAC or relay listener is about to
/// reclaim two lines later, and a standalone `proxy relay` starting while the
/// app is stopped could take the app's remembered port. They are deferred
/// rather than dropped: one of our own free ports still beats leaving the band
/// for an ephemeral port that the next run may not get back.
fn candidate_ports(ours: &[u16]) -> Vec<u16> {
    let span = STABLE_PORT_RANGE.len() as u16;
    let offset = rand::thread_rng().gen_range(0..span);
    let rotated = (0..span).map(|i| STABLE_PORT_RANGE.start + (offset + i) % span);
    let (deferred, first): (Vec<u16>, Vec<u16>) = rotated.partition(|p| ours.contains(p));
    first.into_iter().chain(deferred).collect()
}

/// The ports this install has remembered for its own listeners: the MITM and
/// PAC ports (`proxy/port`, `proxy/pac-port`) and the relay port
/// (`proxy/relay-port`). Best-effort - anything missing or unreadable simply
/// isn't deferred.
fn persisted_ports() -> Vec<u16> {
    let mut ports: Vec<u16> = ["port", "pac-port"]
        .iter()
        .filter_map(|name| super::port_persist::load(name).ok().flatten())
        .collect();
    ports.extend(super::relay::load_persisted_port());
    ports
}

/// Bind `127.0.0.1:port` for the preferred-port reuse path. On unix a plain
/// bind is tried first; when it fails with the port "in use", the previous
/// engine session's connections may just be lingering as server-side
/// TIME_WAIT sockets (they stay for minutes), and without `SO_REUSEADDR` the
/// restart's rebind fails and the port silently moves - defeating the address
/// stability the preferred port exists for. But BSD /
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
fn bind_preferred_once(port: u16) -> std::io::Result<std::net::TcpListener> {
    if let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", port)) {
        return Ok(listener);
    }
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    if std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(250)).is_ok() {
        // Someone is live on this port (loopback or wildcard) - don't shadow
        // it; let the caller handle the taken port (the engine falls back to
        // a fresh band port, the standalone relay host refuses to start).
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
fn bind_preferred_once(port: u16) -> std::io::Result<std::net::TcpListener> {
    std::net::TcpListener::bind(("127.0.0.1", port))
}

/// How long a preferred-port bind keeps retrying before conceding the port.
///
/// Sized for a previous session still letting go, not for waiting out another
/// application: the engine's own listeners close as its runtime winds down, and
/// on macOS that was measured taking anywhere from microseconds to a few
/// milliseconds after `disable()` returned.
const PREFERRED_BIND_GRACE: Duration = Duration::from_millis(250);

/// Bind the preferred port, retrying briefly before falling back.
///
/// The single attempt below answers "is the port free *right now*", and for a
/// port we are trying to *re*claim that is the wrong question: our own previous
/// session may still be releasing it. Conceding on the first refusal is what
/// makes a quick disable/enable land on a fresh port - precisely what the
/// preferred port exists to prevent, and what CLI tools baking the relay port
/// into their configs cannot survive.
///
/// The engine's own teardown is ordered now (see the detached-listener join in
/// `start`), so this is the belt to that braces: it also covers a port held by
/// a `TIME_WAIT` remnant that has not yet expired, or by anything else
/// transient, without needing to know which. Bounded, so a port a *foreign*
/// application genuinely owns costs one short delay at enable time and then
/// falls back exactly as before - the live-listener probe inside still refuses
/// to shadow it.
pub(super) fn bind_preferred(port: u16) -> std::io::Result<std::net::TcpListener> {
    let deadline = std::time::Instant::now() + PREFERRED_BIND_GRACE;
    loop {
        match bind_preferred_once(port) {
            Ok(listener) => return Ok(listener),
            Err(e) if std::time::Instant::now() >= deadline => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
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
            // Loopback must be spelled out. `isPlainHostName` is false for
            // "127.0.0.1" (it has dots), so without this the literal address
            // falls through to the upstream proxy - and the one thing that
            // fetches a loopback URL while routing is on is CryptoAPI going
            // after the CRL our own leaves advertise. Sending that to a
            // corporate proxy fails the fetch, and a CDP that cannot be
            // fetched breaks the handshake it was added to fix.
            s.push_str("  if (h === \"127.0.0.1\") return \"DIRECT\";\n");
            s.push_str("  if (isPlainHostName(h)) return \"DIRECT\";\n");
            s.push_str(&format!("  return \"PROXY {proxy}\";\n"));
        }
        None => s.push_str("  return \"DIRECT\";\n"),
    }
    s.push_str("}\n");
    s
}

/// The `Host` header value of a raw HTTP request head, if one is present in
/// `buf`. Tolerant by design: the PAC responder reads a single buffer and
/// never parses the request beyond this.
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn request_host(buf: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(buf).ok()?;
    text.split("\r\n").skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("host")
            .then(|| value.trim().to_string())
    })
}

/// Serve the PAC script, and the CA's CRL, on a dedicated loopback listener.
///
/// WinINET fetches the `AutoConfigURL` *directly* (not through the proxy), so
/// this must be a plain HTTP responder, separate from the hudsucker proxy on
/// `proxy_port`. The PAC body is rebuilt per request from the live rule set.
/// Runs until the engine's runtime is torn down.
///
/// The CRL rides on this listener rather than on the proxy port for a reason
/// worth keeping: CryptoAPI fetches it *during* a TLS handshake the proxy is in
/// the middle of performing, so answering from the proxy port would make the
/// handshake depend on that same port servicing a nested request. Here it is a
/// separate listener and a separate task, so the fetch cannot contend with the
/// handshake that triggered it. `crl_issuer` is `None` wherever leaves carry no
/// distribution point, in which case nothing will ever ask for it.
///
/// Both routes sit behind the same `Host` guard: the CRL fetcher is CryptoAPI
/// addressing us as loopback, so it passes, while a DNS-rebound browser request
/// is refused before either body is disclosed.
#[cfg(any(target_os = "windows", target_os = "macos"))]
async fn serve_pac(
    listener: tokio::net::TcpListener,
    rules: watch::Receiver<Arc<Vec<ProxyDomain>>>,
    proxy_port: u16,
    upstream: Option<String>,
    crl_issuer: Option<Arc<Issuer<'static, KeyPair>>>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            // Transient accept errors resolve on their own; the pause keeps a
            // permanently failing listener from spinning this loop at 100%
            // CPU for the engine's lifetime.
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            continue;
        };
        let rules = rules.clone();
        let upstream = upstream.clone();
        let crl_issuer = crl_issuer.clone();
        tokio::spawn(async move {
            // Consume the request before replying. Parsed only far enough to
            // read `Host`: the system PAC fetcher addresses us as loopback,
            // while a DNS-rebound browser request arrives under the attacker's
            // hostname - refuse it rather than disclose the engine port the
            // PAC body carries. A request with no Host line (not a browser)
            // stays served. Same rule as the relay's, shared in the parent
            // module.
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let foreign_host =
                request_host(&buf[..n]).is_some_and(|h| !crate::proxy::authority_is_loopback(&h));
            if foreign_host {
                let resp =
                    "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(resp.as_bytes()).await;
                let _ = stream.shutdown().await;
                return;
            }
            // Route on the request target. Only two paths exist, and a partial
            // read simply misses the CRL one and serves the PAC - the same
            // answer this gave before it had two routes to choose between.
            let wants_crl = std::str::from_utf8(&buf[..n])
                .ok()
                .and_then(|req| req.split_whitespace().nth(1))
                .is_some_and(|target| target == crate::proxy::cert_authority::CRL_PATH);

            let resp: Vec<u8> = match (wants_crl, crl_issuer.as_deref()) {
                (true, Some(issuer)) => {
                    match crate::proxy::cert_authority::sign_empty_crl(issuer) {
                        // Signed per request, not once at startup: a CRL is only
                        // valid until its `nextUpdate`, and this listener outlives
                        // that window on any long-running engine. One ECDSA
                        // signature on a path Windows caches for a week is cheaper
                        // than the alternative failure, which is silent.
                        Ok(der) => {
                            let mut r = format!(
                                "HTTP/1.1 200 OK\r\n\
                             Content-Type: application/pkix-crl\r\n\
                             Content-Length: {}\r\n\
                             Connection: close\r\n\r\n",
                                der.len(),
                            )
                            .into_bytes();
                            r.extend_from_slice(&der);
                            r
                        }
                        Err(e) => {
                            eprintln!("gate proxy failed to sign CRL: {e}");
                            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\
                          Connection: close\r\n\r\n"
                                .to_vec()
                        }
                    }
                }
                _ => {
                    let body = pac_script(&rules.borrow(), proxy_port, upstream.as_deref());
                    format!(
                        "HTTP/1.1 200 OK\r\n\
                         Content-Type: application/x-ns-proxy-autoconfig\r\n\
                         Content-Length: {}\r\n\
                         Connection: close\r\n\r\n{body}",
                        body.len(),
                    )
                    .into_bytes()
                }
            };
            let _ = stream.write_all(&resp).await;
            let _ = stream.shutdown().await;
        });
    }
}

/// Start the engine on a loopback port - the preferred one when the config
/// carries it, else a fresh pick from the stable band. Blocks until the proxy
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

    // Windows is the only platform with a TLS client that hard-fails a leaf
    // carrying no CRL distribution point (see `cert_authority::sign_empty_crl`),
    // so it is the only one whose leaves advertise one. The URL names the PAC
    // listener bound just above, which serves `CRL_PATH` for as long as the
    // engine minting those leaves is up - the two lifetimes have to match,
    // because an unfetchable CDP fails handshakes rather than fixing them.
    #[cfg(target_os = "windows")]
    let crl_url = Some(format!(
        "http://127.0.0.1:{pac_port}{}",
        crate::proxy::cert_authority::CRL_PATH
    ));
    #[cfg(not(target_os = "windows"))]
    let crl_url: Option<String> = None;

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
    // Starts empty: a cf_clearance only exists once the GUI's challenge-solve
    // webview captures one (memory-only; a restart re-solves on demand).
    let (cf_clearance_tx, cf_clearance_rx) = watch::channel::<Arc<str>>(Arc::from(""));
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
    // The relay gates its accept loop on the same owner UID the MITM path uses.
    let relay_owner_uid = cfg.owner_uid;
    let handler = GateHandler {
        rules: rules_rx,
        gateway,
        api_key: key_rx,
        token: token_rx,
        org: org_rx,
        cf_clearance: cf_clearance_rx,
        chatgpt_turn: None,
        injected_oauth: false,
        owner_uid: cfg.owner_uid,
        peer_verdict: None,
        claude_code_route: false,
        anthropic_unselected_logged: Arc::new(AtomicBool::new(false)),
        app_shell_unrecognised_logged: Arc::new(AtomicBool::new(false)),
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
                        Ok(issuer) => {
                            GateCa::new(issuer, aws_lc_rs::default_provider(), crl_url.clone())
                        }
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("parsing CA certificate: {e}")));
                            return;
                        }
                    }
                };

                // A second issuer over the same PEMs, for signing the CRL. The
                // first one is owned by the CA that hudsucker takes by value,
                // and `Issuer` holds a `KeyPair` that is not `Clone`, so the
                // cheapest way to have a signer on both sides is to parse
                // twice. Only built where leaves advertise a CDP; anywhere else
                // nothing would ever fetch it.
                #[cfg(any(target_os = "windows", target_os = "macos"))]
                let crl_issuer = crl_url.as_ref().and_then(|_| {
                    let key_pair = KeyPair::from_pem(&key_pem).ok()?;
                    Issuer::from_ca_cert_pem(&cert_pem, key_pair)
                        .ok()
                        .map(Arc::new)
                });

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
                // The PAC responder and the relay run as detached tasks on this
                // runtime, and neither watches the shutdown channel - they are
                // accept loops with no exit condition. Their handles are kept so
                // the teardown below can close their listeners *before* this
                // future returns; see the note there.
                let mut detached: Vec<tokio::task::JoinHandle<()>> = Vec::new();

                // Bring up the PAC responder on the engine runtime. Non-fatal if
                // it can't start - the proxy still runs, WinINET just fails the
                // PAC fetch and falls back to DIRECT (no interception) rather
                // than stranding traffic.
                #[cfg(any(target_os = "windows", target_os = "macos"))]
                {
                    match tokio::net::TcpListener::from_std(pac_listener) {
                        Ok(pac) => {
                            detached.push(tokio::spawn(serve_pac(
                                pac,
                                pac_rules_rx,
                                port,
                                upstream_proxy,
                                crl_issuer,
                            )));
                        }
                        Err(e) => eprintln!("gate proxy PAC listener failed to start: {e}"),
                    }
                }
                // Bring up the CLI reverse-proxy relay on the same runtime.
                // Non-fatal: if it can't adopt its listener the MITM proxy
                // still runs, only CLI tools pointed at the relay fail.
                match super::relay::spawn(
                    relay_listener,
                    relay_gateway,
                    relay_key_rx,
                    relay_token_rx,
                    relay_org_rx,
                    relay_intercept_rx,
                    relay_owner_uid,
                ) {
                    Ok(handle) => detached.push(handle),
                    Err(e) => eprintln!("gate proxy relay failed to start: {e}"),
                }
                if let Err(e) = proxy.start().await {
                    eprintln!("gate proxy engine stopped with error: {e}");
                }

                // Close the detached listeners here, rather than leaving them to
                // the runtime drop. `stop()` joins this thread, so a caller is
                // entitled to read a returned `disable()` as "the ports are
                // free": the persisted-port reuse on the next enable depends on
                // exactly that, and the relay port is baked into CLI tool
                // configs, so losing it points those tools at a dead address.
                // Runtime drop does eventually cancel these tasks, but it is not
                // ordered against this future returning - measured on macOS,
                // both listeners were still accepting after `disable()` returned
                // in a few percent of enable/disable cycles.
                for handle in detached {
                    handle.abort();
                    let _ = handle.await;
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
            cf_clearance_tx,
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
    use crate::proxy::ClientClass;

    /// Cowork's turn, as captured: a GET on a path the `chatgpt` entry claims,
    /// upgraded to a WebSocket.
    fn cowork_upgrade() -> Request<()> {
        Request::builder()
            .method("GET")
            .uri("https://chatgpt.com/backend-api/codex/responses")
            .header("connection", "Upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .body(())
            .unwrap()
    }

    #[test]
    fn the_app_shell_drift_warning_gates_on_all_three_signals() {
        // The latch and the printing live on the handler; this is the condition
        // that decides whether a request is a drift worth naming.
        let rules = crate::proxy::default_domains();
        let shell = "SomeNewShell Mozilla/5.0 (Windows NT 10.0; Win64; x64)";
        assert!(app_shell_is_unrecognised(
            ClientClass::Unknown,
            &rules,
            Some("chatgpt.com"),
            shell
        ));
        // Each leg of the condition, dropped one at a time.
        assert!(
            !app_shell_is_unrecognised(ClientClass::App, &rules, Some("chatgpt.com"), shell),
            "a recognised app is not a drift"
        );
        assert!(
            !app_shell_is_unrecognised(
                ClientClass::Unknown,
                &rules,
                Some("chatgpt.com"),
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64)"
            ),
            "the browser's own user-agent is the other ordinary state"
        );
        assert!(
            !app_shell_is_unrecognised(ClientClass::Unknown, &rules, Some("chatgpt.com"), ""),
            "an absent user-agent names nothing"
        );
        assert!(
            !app_shell_is_unrecognised(ClientClass::Unknown, &rules, Some("api.openai.com"), shell),
            "only the app's own host is watched"
        );
        assert!(!app_shell_is_unrecognised(
            ClientClass::Unknown,
            &rules,
            None,
            shell
        ));
    }

    #[test]
    fn a_websocket_upgrade_is_recognised_whatever_the_casing() {
        assert!(is_upgrade_request(&cowork_upgrade()));
        // Real clients send `Connection: keep-alive, Upgrade`; the tokens are a
        // comma-separated, case-insensitive list.
        let multi = Request::builder()
            .uri("https://chatgpt.com/backend-api/codex/responses")
            .header("connection", "keep-alive, UPGRADE")
            .header("upgrade", "WebSocket")
            .body(())
            .unwrap();
        assert!(is_upgrade_request(&multi));
    }

    #[test]
    fn an_ordinary_request_is_not_an_upgrade() {
        let plain = Request::builder()
            .method("POST")
            .uri("https://chatgpt.com/backend-api/codex/responses")
            .header("connection", "keep-alive")
            .body(())
            .unwrap();
        assert!(!is_upgrade_request(&plain));
        // `Upgrade` alone, without the `Connection` token, is not a real upgrade
        // per RFC 9110 and must not cost the request its route.
        let dangling = Request::builder()
            .uri("https://chatgpt.com/backend-api/codex/responses")
            .header("upgrade", "websocket")
            .body(())
            .unwrap();
        assert!(!is_upgrade_request(&dangling));
    }

    #[test]
    fn the_path_that_carries_the_upgrade_is_one_we_would_otherwise_rewrite() {
        // The guard only earns its place if the router would have claimed this
        // path. If this assertion ever fails the upgrade was passing through
        // anyway and the guard is dead code.
        let mut relay: Vec<ProxyDomain> = crate::proxy::default_domains()
            .into_iter()
            .filter(|d| d.slug == "chatgpt")
            .collect();
        relay[0].enabled = true;
        let req = cowork_upgrade();
        assert_eq!(
            decide(
                &rules_for_client(&relay, ClientClass::App),
                req.uri().host().unwrap(),
                req.uri().path()
            ),
            Decision::Rewrite {
                upstream_url: "https://chatgpt.com/backend-api".into()
            },
        );
    }

    /// The chat turn the challenge fix exists for, cookie header as given.
    fn app_chat_turn(cookie: Option<&str>) -> Request<()> {
        let builder = Request::builder()
            .method("POST")
            .uri("https://chatgpt.com/backend-api/f/conversation");
        let builder = match cookie {
            Some(c) => builder.header("cookie", c),
            None => builder,
        };
        builder.body(()).unwrap()
    }

    fn cookie_header(req: &Request<()>) -> Option<&str> {
        req.headers()
            .get(hudsucker::hyper::header::COOKIE)
            .and_then(|v| v.to_str().ok())
    }

    #[test]
    fn inject_cf_clearance_sets_the_header_when_none_is_present() {
        let mut req = app_chat_turn(None);
        inject_cf_clearance(&mut req, "abc123");
        assert_eq!(cookie_header(&req), Some("cf_clearance=abc123"));
    }

    #[test]
    fn inject_cf_clearance_appends_without_disturbing_existing_cookies() {
        let mut req = app_chat_turn(Some(
            "__Secure-next-auth.session-token=s3ss10n; oai-did=dev",
        ));
        inject_cf_clearance(&mut req, "abc123");
        assert_eq!(
            cookie_header(&req),
            Some("__Secure-next-auth.session-token=s3ss10n; oai-did=dev; cf_clearance=abc123")
        );
    }

    #[test]
    fn inject_cf_clearance_never_clobbers_a_client_supplied_cf_clearance() {
        let original = "oai-did=dev; cf_clearance=client-owned";
        let mut req = app_chat_turn(Some(original));
        inject_cf_clearance(&mut req, "ours");
        assert_eq!(cookie_header(&req), Some(original));
    }

    #[test]
    fn a_challenge_is_detected_only_on_a_chatgpt_app_turn() {
        let turn = |client| ChatgptTurn {
            client,
            method: Method::POST,
            path: "/backend-api/f/conversation".into(),
            rewritten: true,
            cf_injected: false,
        };
        let app = turn(ClientClass::App);
        let challenge = hudsucker::hyper::Response::builder()
            .status(403)
            .header("cf-mitigated", "challenge")
            .body(())
            .unwrap();
        assert!(cf_challenge_detected(Some(&app), &challenge));
        // No memo means the response answered some other host.
        assert!(!cf_challenge_detected(None, &challenge));
        // The browser runs its own interstitial, so the same challenge on the
        // website must not open a webview the user never needed.
        let web = turn(ClientClass::Web);
        assert!(!cf_challenge_detected(Some(&web), &challenge));
        // And an app turn answered normally (or with a non-challenge
        // mitigation) must not either.
        let plain = hudsucker::hyper::Response::builder()
            .status(200)
            .body(())
            .unwrap();
        assert!(!cf_challenge_detected(Some(&app), &plain));
        let other = hudsucker::hyper::Response::builder()
            .status(403)
            .header("cf-mitigated", "block")
            .body(())
            .unwrap();
        assert!(!cf_challenge_detected(Some(&app), &other));
    }

    /// Serialize the tests that bind real listeners in [`STABLE_PORT_RANGE`].
    ///
    /// [`bind_fresh`] reads the persisted ports through the path seams to know
    /// what to *skip*, which already puts these tests under the rule
    /// `path_env_lock` states: anything reading those paths that would be wrong
    /// to see another test's must take the lock. Skipping it also let them race
    /// the real-engine tests in `manager_core`, which hold the same lock: this
    /// band is 100 ports wide, an engine holds three of them, and a fresh bind
    /// here could take the very port an enable/disable cycle there was about to
    /// reclaim. That surfaced as a ~7%-per-run failure of
    /// `engine_port_persists_across_an_enable_cycle` on macOS, which binds one
    /// more listener per engine than Linux does.
    fn band_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::env::path_env_lock()
    }

    /// Everything switched off, as the forced-route tests want it.
    fn all_off() -> Vec<ProxyDomain> {
        crate::proxy::default_domains()
            .into_iter()
            .map(|mut d| {
                d.enabled = false;
                d
            })
            .collect()
    }

    /// The forced route must not become the default route. Without the
    /// selector, Claude Code's own host is decided by the catalog like any
    /// other - so a switched-off Anthropic entry stays switched off, and the
    /// `claude_code_selector_routes_when_desktop_domain_is_off` e2e proves a
    /// property of the selector rather than of the host.
    #[test]
    fn anthropic_without_the_selector_keeps_the_live_catalog() {
        let live = all_off();
        let rules = route_rules(&live, false, Some("api.anthropic.com"));
        assert!(matches!(rules, Cow::Borrowed(_)), "nothing to force");
        assert!(
            !should_intercept_host(&rules, "api.anthropic.com"),
            "an unselected connection must be tunnelled while the entry is off"
        );
        // And the selector is what changes that, on this same input.
        assert!(should_intercept_host(
            &route_rules(&live, true, Some("api.anthropic.com")),
            "api.anthropic.com"
        ));
    }

    /// The forced entry is pushed with `enabled: true`, so the one thing that
    /// keeps it from routing every host is that it is scoped to its own. A
    /// selected connection to anything else must see the catalog untouched.
    #[test]
    fn the_selector_forces_nothing_for_another_host() {
        let live = all_off();
        for host in ["api.openai.com", "openrouter.ai", "claude.ai"] {
            let rules = route_rules(&live, true, Some(host));
            assert!(
                matches!(rules, Cow::Borrowed(_)),
                "{host} is not Claude Code's destination"
            );
            assert!(
                !should_intercept_host(&rules, host),
                "{host} must still tunnel with every domain off"
            );
        }
    }

    /// A fresh listener must land in the band we can actually rebind next run,
    /// not on an OS-assigned ephemeral port.
    #[test]
    fn bind_fresh_picks_from_the_stable_band() {
        let _lock = band_lock();
        let listener = bind_fresh().expect("a free port in the band");
        let port = listener.local_addr().unwrap().port();
        assert!(
            STABLE_PORT_RANGE.contains(&port),
            "{port} is outside {STABLE_PORT_RANGE:?}"
        );
    }

    /// Successive binds skip what the previous one holds, so the three
    /// listeners one engine brings up never collide.
    #[test]
    fn bind_fresh_skips_ports_already_held() {
        let _lock = band_lock();
        let first = bind_fresh().expect("first port");
        let second = bind_fresh().expect("second port");
        let (a, b) = (
            first.local_addr().unwrap().port(),
            second.local_addr().unwrap().port(),
        );
        assert_ne!(a, b);
        assert!(STABLE_PORT_RANGE.contains(&a) && STABLE_PORT_RANGE.contains(&b));
    }

    /// The band is offered in full, so a fresh bind never runs out of it while
    /// a port is still free.
    #[test]
    fn candidate_ports_covers_the_band_exactly_once() {
        let mut ports = candidate_ports(&[]);
        ports.sort_unstable();
        assert_eq!(ports, STABLE_PORT_RANGE.collect::<Vec<_>>());
    }

    /// Ports this install remembers go to the back: the PAC and relay
    /// listeners bind after the MITM one and reclaim theirs by preference, so a
    /// fresh pick must not take a port that is about to be wanted.
    #[test]
    fn candidate_ports_defers_the_ports_we_remember() {
        let ours = vec![
            STABLE_PORT_RANGE.start + 7,
            STABLE_PORT_RANGE.start + 42,
            STABLE_PORT_RANGE.start + 91,
        ];
        let ports = candidate_ports(&ours);
        let tail = &ports[ports.len() - ours.len()..];
        for port in &ours {
            assert!(tail.contains(port), "{port} should be deferred to the tail");
        }
        // Deferred, not dropped: still every port in the band.
        let mut sorted = ports.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, STABLE_PORT_RANGE.collect::<Vec<_>>());
    }

    /// The rotation is random, so concurrent hosts don't converge on one port.
    #[test]
    fn candidate_ports_starts_somewhere_different() {
        let firsts: std::collections::HashSet<u16> =
            (0..8).map(|_| candidate_ports(&[])[0]).collect();
        assert!(
            firsts.len() > 1,
            "eight draws all starting at the same port is not a rotation: {firsts:?}"
        );
    }

    /// Losing the persisted port must fall back into the band too - falling
    /// back to an ephemeral port is what made the next restart move again.
    #[test]
    fn bind_loopback_falls_into_the_band_when_the_preferred_port_is_taken() {
        let _lock = band_lock();
        // A *live* listener on an ephemeral port: `bind_preferred` treats this
        // as taken (as opposed to a TIME_WAIT remnant, which it reclaims).
        let squatter = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("squatter");
        let taken = squatter.local_addr().unwrap().port();
        let (_listener, port) = bind_loopback(Some(taken)).expect("fallback binds");
        assert_ne!(port, taken);
        assert!(
            STABLE_PORT_RANGE.contains(&port),
            "{port} is outside {STABLE_PORT_RANGE:?}"
        );
    }

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
            rewrite_suffixes: Vec::new(),
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
        // Loopback must be DIRECT even behind an upstream proxy: it is where
        // CryptoAPI goes to fetch the CRL our leaves advertise, and
        // `isPlainHostName` is false for a dotted literal address, so without
        // an explicit rule that fetch would be handed to the corporate proxy
        // and fail - breaking the handshake the CDP exists to fix.
        assert!(pac.contains("if (h === \"127.0.0.1\") return \"DIRECT\";"));
        assert!(pac
            .trim_end()
            .ends_with("return \"PROXY proxy.corp.com:8080\";\n}"));
    }
}
