//! Built-in MITM proxy that routes config-less apps (Claude Desktop /
//! Cowork, ChatGPT desktop, …) through the Gate gateway without editing
//! any app config.
//!
//! How it works: enabling the proxy (1) trusts a locally-generated root CA
//! and (2) points the macOS system HTTPS proxy at a loopback listener owned
//! by [`engine`]. For each TLS CONNECT the engine decides - *before* doing
//! any handshake - whether the target host is one we route. Hosts we don't
//! route are blind-tunnelled untouched (so cert-pinning apps and every other
//! site are unaffected). For hosts we do route, the engine MITMs the
//! connection, and for inference paths rewrites the request to the Gate
//! gateway with `X-Gate-Api-Key` + `X-Gate-Upstream-Url` injected. Non-
//! inference paths on the same host (e.g. an app's auto-updater) pass
//! through to the real upstream.
//!
//! The CA private key lives in the OS keychain; only the public cert is
//! written to disk (and installed into the OS trust store when trusted).
//! Disabling the proxy restores the previous system-proxy state but
//! deliberately leaves the CA trusted, so re-enabling is promptless;
//! untrusting is a separate explicit action ([`ProxyManager::untrust_ca`]).
//!
//! Platform support: macOS, Windows, and Linux. The engine itself is
//! cross-platform; CA trust ([`ca`]) and system-proxy wiring ([`system_proxy`])
//! are platform-specific - macOS via `security` + `networksetup`, Windows via
//! `certutil` + the per-user WinINET registry settings, Linux via the system
//! trust store (`update-ca-certificates` / `update-ca-trust`) + a user-scoped
//! systemd `environment.d` drop-in (so the proxy reaches command-line tools and
//! GUI apps without root). Other platforms get no [`ProxyManager`].

use anyhow::{Context, Result};
use http::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};

pub mod engine;

mod cert_authority;

/// Plaintext loopback reverse proxy for CLI tools; hosted in the engine.
mod relay;

pub mod config;

pub mod intent;

pub mod autostart_optout;

// Shared load/save for the persisted engine + PAC ports; the per-OS
// `system_proxy` modules wrap it with their platform rationale.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod port_persist;

#[cfg(target_os = "macos")]
pub mod ca;
#[cfg(target_os = "windows")]
#[path = "ca_windows.rs"]
pub mod ca;
#[cfg(target_os = "linux")]
#[path = "ca_linux.rs"]
pub mod ca;

#[cfg(target_os = "macos")]
pub mod system_proxy;
#[cfg(target_os = "windows")]
#[path = "system_proxy_windows.rs"]
pub mod system_proxy;
#[cfg(target_os = "linux")]
#[path = "system_proxy_linux.rs"]
pub mod system_proxy;

#[cfg(target_os = "macos")]
mod manager;
#[cfg(target_os = "windows")]
#[path = "manager_windows.rs"]
mod manager;
#[cfg(target_os = "linux")]
#[path = "manager_linux.rs"]
mod manager;

// The long-lived pass-through helper daemon and its control protocol/client
// (Linux only). The daemon owns the loopback listener so the proxy outlives the
// GUI process; see the module docs for the access-control model.
#[cfg(target_os = "linux")]
pub mod control;
#[cfg(target_os = "linux")]
mod flock;
#[cfg(target_os = "linux")]
pub mod helper;
#[cfg(target_os = "linux")]
pub mod helper_client;

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub use manager::{manager, ProxyManager};

/// Cross-process hint that some Gate Connect process has the system proxy
/// routed through a live engine: the snapshot file exists for exactly that
/// duration. The engine holds the key it was started with, so the CLI uses
/// this to warn after a rotation/sign-out it cannot propagate in-process.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn engine_likely_running() -> bool {
    system_proxy::load_snapshot()
        .map(|s| s.is_some())
        .unwrap_or(false)
}

/// The loopback base URL CLI tools point at to route through the reverse-proxy
/// relay, or `None` if no relay port has ever been bound (so nothing to point
/// at yet). Reads the persisted port, so it's stable across restarts and valid
/// even while the engine is momentarily down. This is the base URL the tool
/// integrations write into their config instead of a gateway URL + key.
pub fn relay_base_url() -> Option<String> {
    relay::load_persisted_port().map(relay::base_url)
}

/// Run the CLI reverse-proxy relay as a standalone, blocking headless host (no
/// MITM, no CA trust, no system-proxy changes). For environments with no
/// menubar app - containers, servers, CI - so CLI tools pointed at the relay
/// still route through Gate. Blocks until the process is killed.
pub fn serve_relay() -> anyhow::Result<()> {
    relay::serve()
}

/// Non-secret hint the tool config (or the MITM rewrite) sets, telling the
/// gateway which upstream to forward to.
pub(crate) const UPSTREAM_URL_HEADER: &str = "x-gate-upstream-url";
/// Legacy credential header (Gate workspace key), injected when no OAuth token
/// is present.
pub(crate) const GATE_KEY_HEADER: &str = "x-gate-api-key";
/// OAuth credential header (Cognito access token); takes precedence over the
/// API key when present.
pub(crate) const GATE_AUTHORIZATION_HEADER: &str = "x-gate-authorization";
/// Selected-org header, injected alongside the OAuth token (the gateway
/// requires it on every OAuth request).
pub(crate) const GATE_ORG_HEADER: &str = "x-gate-org-id";

/// Inject the live Gate credential into `headers`, the single precedence rule
/// shared by the MITM engine ([`engine::apply_rewrite`]) and the loopback
/// [`relay`] so the two paths can't drift.
///
/// If the caller already set an `X-Gate-Api-Key`, that's respected: the Gate
/// headers are left exactly as they arrived and nothing is injected. Otherwise
/// any stray `X-Gate-Authorization` / `X-Gate-Org-Id` are stripped (so an org
/// can't ride alongside the credential we inject) and the live credential is
/// added: a non-empty `oauth_token` wins - `X-Gate-Authorization: Bearer
/// <token>` plus `X-Gate-Org-Id` when `org_id` is `Some` - otherwise the legacy
/// `X-Gate-Api-Key`.
pub(crate) fn inject_gate_credential(
    headers: &mut HeaderMap,
    api_key: &str,
    oauth_token: Option<&str>,
    org_id: Option<&str>,
) -> Result<()> {
    if headers.contains_key(GATE_KEY_HEADER) {
        return Ok(());
    }
    headers.remove(GATE_AUTHORIZATION_HEADER);
    headers.remove(GATE_ORG_HEADER);
    match oauth_token.filter(|t| !t.is_empty()) {
        Some(token) => {
            headers.insert(
                HeaderName::from_static(GATE_AUTHORIZATION_HEADER),
                HeaderValue::from_str(&format!("Bearer {token}"))
                    .context("building x-gate-authorization header")?,
            );
            if let Some(org) = org_id {
                headers.insert(
                    HeaderName::from_static(GATE_ORG_HEADER),
                    HeaderValue::from_str(org).context("building x-gate-org-id header")?,
                );
            }
        }
        None => {
            headers.insert(
                HeaderName::from_static(GATE_KEY_HEADER),
                HeaderValue::from_str(api_key).context("building x-gate-api-key header")?,
            );
        }
    }
    Ok(())
}

/// One routable provider. The built-in set is defined by
/// [`default_domains`]; persisted config only flips `enabled` per `slug`,
/// so adding a new built-in domain automatically surfaces it in the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyDomain {
    /// Stable identifier used in config + the `proxy_set_domain` command.
    pub slug: String,
    /// Human-readable name shown in the UI.
    pub display_name: String,
    /// Exact hostnames to intercept (e.g. `api.anthropic.com`). A CONNECT
    /// to any other host is blind-tunnelled.
    pub hosts: Vec<String>,
    /// Value injected as `X-Gate-Upstream-Url` - where Gate forwards the
    /// rewritten request (e.g. `https://api.anthropic.com`).
    pub upstream_url: String,
    /// Path prefixes that are inference calls and should be rewritten to
    /// the gateway (e.g. `/v1/`).
    pub rewrite_prefixes: Vec<String>,
    /// Path prefixes on an intercepted host that must NOT be rewritten -
    /// they pass through to the real upstream (e.g. an app's
    /// `/api/desktop/` auto-updater channel).
    pub passthrough_prefixes: Vec<String>,
    /// Whether the user has this domain turned on.
    pub enabled: bool,
    /// Whether Gate can actually upstream this provider today. Unsupported
    /// domains render as disabled rows in the UI and can't be enabled.
    pub supported: bool,
}

impl ProxyDomain {
    fn matches_host(&self, host: &str) -> bool {
        self.hosts.iter().any(|h| h.eq_ignore_ascii_case(host))
    }
}

/// Snapshot of the proxy subsystem for the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyState {
    /// Whether the loopback engine is currently running.
    pub running: bool,
    /// Loopback port the engine is bound to (when running).
    pub port: Option<u16>,
    /// Loopback port serving the PAC script the system proxy points at (when
    /// running). PAC-driven platforms only (macOS/Windows); always `None` on
    /// Linux, which wires env-var proxies with no PAC.
    #[serde(default)]
    pub pac_port: Option<u16>,
    /// Whether our root CA is trusted in the OS trust store.
    pub ca_trusted: bool,
    /// The full domain catalog with current enabled flags.
    pub domains: Vec<ProxyDomain>,
}

/// What the engine should do with a request on an intercepted host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Host isn't one of ours - should never reach here post-intercept,
    /// but treated as a safe passthrough if it does.
    Tunnel,
    /// Matched host but not an inference path: forward to the real
    /// upstream unchanged.
    Passthrough,
    /// Rewrite to the gateway, injecting this upstream URL.
    Rewrite { upstream_url: String },
}

/// True if any enabled domain claims `host`. Used by the engine's
/// `should_intercept` to gate MITM at the CONNECT stage.
pub(crate) fn should_intercept_host(domains: &[ProxyDomain], host: &str) -> bool {
    domains.iter().any(|d| d.enabled && d.matches_host(host))
}

/// The path component of a catalog `upstream_url` - `/api` for
/// `https://openrouter.ai/api`, `""` for a bare host. Gate appends the
/// forwarded path to the upstream URL verbatim, so this segment is the part of
/// the provider's path that travels in the `X-Gate-Upstream-Url` header rather
/// than in the request line.
pub(crate) fn upstream_path(upstream_url: &str) -> &str {
    let after_scheme = upstream_url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(upstream_url);
    after_scheme
        .find('/')
        .map(|i| after_scheme[i..].trim_end_matches('/'))
        .unwrap_or("")
}

/// Remove an upstream URL's own path prefix from a request path, on a path
/// boundary: `/api/v1/chat` under upstream path `/api` becomes `/v1/chat`.
///
/// `None` means the request is outside the upstream's subtree (`/apifoo`, or a
/// wholly unrelated path), which callers treat as "not ours".
pub(crate) fn strip_upstream_path<'a>(path: &'a str, upstream_path: &str) -> Option<&'a str> {
    if upstream_path.is_empty() {
        return Some(path);
    }
    let rest = path.strip_prefix(upstream_path)?;
    if rest.is_empty() {
        Some("/")
    } else if rest.starts_with('/') {
        Some(rest)
    } else {
        None
    }
}

/// Decide what to do with a request given its host + path. Passthrough
/// prefixes win over rewrite prefixes; a matched host with an unmatched
/// path is left alone (passthrough) rather than rewritten.
///
/// Prefixes are matched against the path *as Gate will receive it* - i.e. after
/// the domain's own [`upstream_path`] is removed, since that segment rides in
/// the upstream header instead. For a bare-host upstream the two are identical.
pub(crate) fn decide(domains: &[ProxyDomain], host: &str, path: &str) -> Decision {
    for d in domains.iter().filter(|d| d.enabled) {
        if !d.matches_host(host) {
            continue;
        }
        // A path outside the upstream's own subtree is not this domain's
        // traffic at all; leave it alone rather than forwarding a path Gate
        // would reassemble into a URL the provider never served.
        let Some(path) = strip_upstream_path(path, upstream_path(&d.upstream_url)) else {
            return Decision::Passthrough;
        };
        if d.passthrough_prefixes
            .iter()
            .any(|p| path.starts_with(p.as_str()))
        {
            return Decision::Passthrough;
        }
        if d.rewrite_prefixes
            .iter()
            .any(|p| path.starts_with(p.as_str()))
        {
            return Decision::Rewrite {
                upstream_url: d.upstream_url.clone(),
            };
        }
        return Decision::Passthrough;
    }
    Decision::Tunnel
}

/// The built-in domain catalog. All entries ship `supported:true` (Anthropic
/// is also `enabled` by default; the rest are opt-in). New providers can be
/// added here and surface in the UI automatically; gate a provider behind
/// `supported:false` until Gate's upstream support for it is confirmed.
pub fn default_domains() -> Vec<ProxyDomain> {
    vec![
        ProxyDomain {
            slug: "anthropic".into(),
            // Named for what it covers (the apps whose traffic this
            // intercepts), not the vendor: on the UI ledger a vendor name
            // here would read as if it included Claude Code, which routes by
            // config instead. The host line carries api.anthropic.com.
            display_name: "Claude Desktop / Cowork".into(),
            // Inference for Claude Code, Claude Desktop, and Cowork all goes
            // to api.anthropic.com /v1/messages (OAuth bearer or API key),
            // confirmed against a real Cowork generation. a-api.anthropic.com
            // is Anthropic's telemetry host (Segment-style /v1/b ingestion)
            // and claude.ai is the web/login surface - both are deliberately
            // left tunnelled, never intercepted.
            hosts: vec!["api.anthropic.com".into()],
            // Applies to every host above. Only group hosts that genuinely
            // share this upstream - never collapse distinct API hosts onto one.
            upstream_url: "https://api.anthropic.com".into(),
            // Only genuine inference endpoints are rewritten to the gateway.
            // Scoped deliberately narrow: Claude Desktop / Cowork also make
            // OAuth + account calls on this same host under /v1/ (e.g.
            // /v1/oauth/*, /v1/organizations/*), and those carry no model, so
            // the gateway can't classify them and rejects them 503 ("AI
            // unknown"). Rewriting only /v1/messages (covers count_tokens +
            // batches sub-paths) and legacy /v1/complete lets every other /v1/
            // path fall through to `decide`'s default Passthrough and reach the
            // real host unchanged. Do NOT widen this back to "/v1/".
            rewrite_prefixes: vec!["/v1/messages".into(), "/v1/complete".into()],
            // Paths outside the inference set already pass through; this keeps
            // the Squirrel auto-updater explicit. Other /api/* paths
            // (claude_code, event_logging, bootstrap) also reach the real host
            // unrewritten.
            passthrough_prefixes: vec!["/api/desktop/".into()],
            enabled: true,
            supported: true,
        },
        ProxyDomain {
            slug: "openai".into(),
            // "apps", not the vendor name: covers any system-proxy-honoring
            // client of api.openai.com, and must not read as including Codex
            // (config-routed; its embedded agent ignores the system proxy).
            display_name: "OpenAI apps".into(),
            // The OpenAI API host. Catches OpenAI-compatible clients that
            // honor the macOS system proxy and hit /v1/. Note: the Codex
            // desktop app's model calls come from its embedded Rust agent,
            // which ignores the system proxy and reaches chatgpt.com
            // directly, so the proxy can't capture them - route Codex via the
            // manual integration (config.toml base_url) instead.
            hosts: vec!["api.openai.com".into()],
            upstream_url: "https://api.openai.com".into(),
            // Inference endpoints only, same reasoning as Anthropic above: a
            // client's non-inference /v1/ calls (e.g. /v1/models preflight)
            // carry no model, so the gateway can't classify them and 503s.
            // Rewrite only the model-call paths; everything else on the host
            // passes through to real api.openai.com. Do NOT widen back to "/v1/".
            rewrite_prefixes: vec![
                "/v1/chat/completions".into(),
                "/v1/completions".into(),
                "/v1/responses".into(),
                "/v1/embeddings".into(),
            ],
            passthrough_prefixes: vec![],
            enabled: false,
            supported: true,
        },
        ProxyDomain {
            slug: "chatgpt".into(),
            display_name: "ChatGPT (Codex subscription)".into(),
            // ChatGPT-subscription Codex talks to the Responses API at
            // chatgpt.com/backend-api/codex/responses (bearer = the user's
            // ChatGPT OAuth token, passed through). Codex reaches Gate via the
            // manual integration (integrations/codex.rs), whose base_url points
            // at the loopback relay; its own embedded agent ignores the system
            // proxy, so the MITM engine never captures this traffic. This entry
            // exists so the relay recognizes the tool-supplied upstream hint.
            hosts: vec!["chatgpt.com".into()],
            // Shape matches integrations/codex.rs exactly, because the relay
            // exact-matches the `X-Gate-Upstream-Url` header codex.rs writes:
            // the `/backend-api` segment rides in the upstream here, and the
            // client-side path is the short `/codex/responses` (Codex's
            // base_url is `<relay>/codex`, wire_api appends `/responses`). The
            // gateway concatenates path onto upstream, yielding
            // `https://chatgpt.com/backend-api/codex/responses`. This is a
            // different split than the MITM convention (bare host + full
            // `/backend-api/codex/responses` path) because the relay sees
            // Codex's rewritten path, not the real upstream path.
            upstream_url: "https://chatgpt.com/backend-api".into(),
            rewrite_prefixes: vec!["/codex/responses".into()],
            passthrough_prefixes: vec![],
            enabled: false,
            supported: true,
        },
        ProxyDomain {
            slug: "openrouter".into(),
            display_name: "OpenRouter apps".into(),
            // OpenRouter's API lives at openrouter.ai/api/v1/* (OpenAI-shaped
            // chat/completions). Opt-in like OpenAI; intercepts OpenRouter
            // clients that honor the system proxy.
            hosts: vec!["openrouter.ai".into()],
            // The `/api` MUST ride in the upstream URL, not the forwarded path.
            // Gate's ALB routes `/api/*` (plus /orgs/, /admin/, /me/,
            // /agent-templates/) to the dashboard API, so a forwarded
            // `/api/v1/chat/completions` never reaches the gateway proxy at all
            // - it 404s out of a service that has no such route. Keeping `/api`
            // upstream-side sends `/v1/chat/completions`, which clears the rule,
            // and Gate reassembles the two into the URL OpenRouter serves.
            // `forwarded_paths_avoid_gate_reserved_prefixes` pins this.
            upstream_url: "https://openrouter.ai/api".into(),
            rewrite_prefixes: vec!["/v1/".into()],
            passthrough_prefixes: vec![],
            enabled: false,
            supported: true,
        },
        ProxyDomain {
            slug: "opencode".into(),
            display_name: "OpenCode Zen / Go".into(),
            // Zen (`/zen/v1/*`) and Go (`/zen/go/v1/*`) are the same host and
            // the same upstream, separated only by path, so they are ONE entry:
            // `decide` returns on the first host match, so a second entry
            // sharing `opencode.ai` would never be consulted.
            hosts: vec!["opencode.ai".into()],
            upstream_url: "https://opencode.ai".into(),
            // Inference endpoints only, same reasoning as Anthropic and OpenAI
            // above: a `/zen/v1/models` preflight carries no model, so the
            // gateway can't classify it and 503s. Both Zen and Go host
            // OpenAI-shaped and Anthropic-shaped endpoints under the same
            // prefix, hence two leaves each. Do NOT widen to "/zen/".
            rewrite_prefixes: vec![
                "/zen/v1/chat/completions".into(),
                "/zen/v1/messages".into(),
                "/zen/go/v1/chat/completions".into(),
                "/zen/go/v1/messages".into(),
            ],
            passthrough_prefixes: vec![],
            enabled: false,
            supported: true,
        },
    ]
}

/// A provider endpoint split the way the relay and the gateway need it.
pub struct ResolvedEndpoint {
    /// Catalog slug that owns the endpoint.
    pub slug: String,
    /// What `X-Gate-Upstream-Url` must carry. The gateway concatenates the
    /// forwarded request path onto this, and the relay only forwards to
    /// upstreams it can find in the catalog.
    pub upstream_url: String,
    /// The path that has to live in the *tool's* base URL, so that the path the
    /// relay forwards is relative to `upstream_url`. Empty when the endpoint is
    /// the catalog upstream itself.
    pub client_path: String,
}

impl ResolvedEndpoint {
    /// The base URL a tool config points at to route this endpoint through the
    /// relay: `<relay>/<slug><client_path>`.
    ///
    /// The slug segment is how the relay knows which upstream a request belongs
    /// to, so it can inject `x-gate-upstream-url` itself instead of the tool
    /// carrying it in a config file. It is stripped back off before anything is
    /// forwarded, leaving exactly `client_path` + whatever the tool appended.
    pub fn relay_base_url(&self, relay_base_url: &str) -> String {
        format!(
            "{}/{}{}",
            relay_base_url.trim_end_matches('/'),
            self.slug,
            self.client_path
        )
    }
}

/// Resolve a provider's canonical endpoint - the URL a tool would call if Gate
/// were not in the picture, e.g. `https://openrouter.ai/api/v1` - into the
/// catalog upstream plus the path the tool must keep on its own side.
///
/// `None` means no catalog entry covers the endpoint. The relay refuses to
/// forward such an upstream, so a caller must leave that provider's config
/// alone rather than repointing it at a relay that will 403 every request.
///
/// This is deliberately the *only* place the split is decided. Doing it by hand,
/// once per integration, is what first broke OpenRouter: `https://openrouter.ai/api`
/// read like a sensible upstream but matched no catalog entry (which was then
/// `https://openrouter.ai`), so every request 403'd. The catalog entry now
/// carries the `/api` itself, which is what makes that split the correct one -
/// see the entry's comment for why the forwarded path must not begin `/api/`.
pub fn resolve_endpoint(endpoint: &str) -> Option<ResolvedEndpoint> {
    let endpoint = endpoint.trim_end_matches('/');
    default_domains()
        .into_iter()
        .filter_map(|d| {
            let rest = endpoint.strip_prefix(d.upstream_url.as_str())?;
            // Only match on a path boundary, so `https://api.openai.com.evil.test`
            // can never resolve to the `api.openai.com` entry.
            if !rest.is_empty() && !rest.starts_with('/') {
                return None;
            }
            let client_path = rest.to_string();
            Some((d, client_path))
        })
        // Longest upstream wins, so an entry carrying a path
        // (chatgpt.com/backend-api) beats a bare-host entry for the same host.
        .max_by_key(|(d, _)| d.upstream_url.len())
        .map(|(d, client_path)| ResolvedEndpoint {
            slug: d.slug,
            upstream_url: d.upstream_url,
            client_path,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anthropic() -> Vec<ProxyDomain> {
        vec![default_domains().into_iter().next().unwrap()]
    }

    #[test]
    fn resolves_endpoints_against_the_catalog() {
        // Bare-host upstream: the whole path stays on the client.
        let r = resolve_endpoint("https://api.anthropic.com/v1").expect("anthropic resolves");
        assert_eq!(r.slug, "anthropic");
        assert_eq!(r.upstream_url, "https://api.anthropic.com");
        assert_eq!(r.client_path, "/v1");

        // OpenRouter's real API lives under /api/v1, and the `/api` belongs in
        // the *upstream*, not the client path: Gate's ALB diverts `/api/*` to
        // the dashboard API, so a forwarded `/api/v1/...` never reaches the
        // gateway proxy. Gate re-joins upstream + path, so the provider still
        // sees /api/v1/chat/completions.
        let r = resolve_endpoint("https://openrouter.ai/api/v1").expect("openrouter resolves");
        assert_eq!(r.slug, "openrouter");
        assert_eq!(r.upstream_url, "https://openrouter.ai/api");
        assert_eq!(r.client_path, "/v1");

        // A catalog upstream that itself carries a path wins over a bare host.
        let r =
            resolve_endpoint("https://chatgpt.com/backend-api/codex").expect("chatgpt resolves");
        assert_eq!(r.slug, "chatgpt");
        assert_eq!(r.upstream_url, "https://chatgpt.com/backend-api");
        assert_eq!(r.client_path, "/codex");

        // Trailing slash and the bare upstream itself.
        let r = resolve_endpoint("https://api.openai.com/").expect("openai resolves");
        assert_eq!(r.client_path, "");

        // Zen and Go share one catalog entry, separated by client path. Longest
        // match is not what distinguishes them - the same upstream serves both -
        // so the path each tool keeps is what routes it.
        let zen = resolve_endpoint("https://opencode.ai/zen/v1").expect("zen resolves");
        assert_eq!(zen.slug, "opencode");
        assert_eq!(zen.upstream_url, "https://opencode.ai");
        assert_eq!(zen.client_path, "/zen/v1");
        let go = resolve_endpoint("https://opencode.ai/zen/go/v1").expect("zen go resolves");
        assert_eq!(go.slug, "opencode");
        assert_eq!(go.client_path, "/zen/go/v1");

        // Off-catalog upstreams do not resolve, so callers leave them alone.
        assert!(resolve_endpoint("https://attacker.example/v1").is_none());
        // Suffix-confusion must not resolve to the api.openai.com entry.
        assert!(resolve_endpoint("https://api.openai.com.evil.test/v1").is_none());
    }

    #[test]
    fn every_resolved_endpoint_lands_on_an_inference_prefix() {
        // The invariant that ties the two halves together: for each catalog
        // entry, the path a tool ends up sending (client_path + the tool's own
        // suffix) must match one of that entry's `rewrite_prefixes`, or the
        // request silently passes through to the user's own account instead of
        // routing through Gate. Checked here for the canonical endpoint of each
        // domain; each integration's own
        // `known_provider_endpoints_all_resolve_against_the_catalog` checks it
        // for the endpoints that integration actually writes.
        for d in default_domains() {
            if d.rewrite_prefixes.is_empty() {
                continue;
            }
            // A prefix ending in `/` is a directory prefix, so give it a leaf to
            // stand in for the tool's own suffix; otherwise the prefix is
            // already a full endpoint path.
            let prefix = &d.rewrite_prefixes[0];
            let path = if prefix.ends_with('/') {
                format!("{prefix}probe")
            } else {
                prefix.clone()
            };
            let endpoint = format!("{}{}", d.upstream_url, path);
            let r = resolve_endpoint(&endpoint)
                .unwrap_or_else(|| panic!("{} endpoint {endpoint} must resolve", d.slug));
            assert_eq!(r.slug, d.slug);
            assert!(
                d.rewrite_prefixes
                    .iter()
                    .any(|p| r.client_path.starts_with(p.as_str())),
                "{}: client_path {:?} matches no rewrite prefix {:?}",
                d.slug,
                r.client_path,
                d.rewrite_prefixes
            );
        }
    }

    #[test]
    fn forwarded_paths_avoid_gate_reserved_prefixes() {
        // Gate's ALB routes by path prefix: `/api/*`, `/orgs/*`, `/admin/*`,
        // `/me/*` and `/agent-templates/*` go to the dashboard API, everything
        // else to the gateway proxy (gate: terraform/aws/compute.tf, the
        // `path_patterns` on the dashboard-api listener rule). A forwarded path
        // that starts with one of those never reaches the proxy at all - it
        // 404s out of a service with no such route, which is exactly how the
        // OpenRouter integration failed end-to-end while every catalog
        // self-consistency check stayed green.
        //
        // This list mirrors infrastructure in another repo, so it is a snapshot:
        // if Gate adds a listener rule, this test will not know. It still pins
        // the ones we have measured.
        const RESERVED: &[&str] = &["/api/", "/orgs/", "/admin/", "/me/", "/agent-templates/"];

        for d in default_domains() {
            for prefix in &d.rewrite_prefixes {
                for reserved in RESERVED {
                    assert!(
                        !prefix.starts_with(reserved),
                        "{}: rewrite prefix {:?} lands on Gate's reserved {:?} - the request \
                         would be routed to the dashboard API instead of the gateway proxy. \
                         Move that segment into `upstream_url` so it rides in \
                         X-Gate-Upstream-Url instead of the request line.",
                        d.slug,
                        prefix,
                        reserved
                    );
                }
            }
        }
    }

    #[test]
    fn upstream_path_extracts_the_path_component() {
        assert_eq!(upstream_path("https://openrouter.ai/api"), "/api");
        assert_eq!(
            upstream_path("https://chatgpt.com/backend-api"),
            "/backend-api"
        );
        assert_eq!(upstream_path("https://api.anthropic.com"), "");
        // A trailing slash is not a path segment.
        assert_eq!(upstream_path("https://openrouter.ai/"), "");
    }

    #[test]
    fn strip_upstream_path_respects_path_boundaries() {
        assert_eq!(
            strip_upstream_path("/api/v1/chat", "/api"),
            Some("/v1/chat")
        );
        // The upstream root itself normalises to "/".
        assert_eq!(strip_upstream_path("/api", "/api"), Some("/"));
        // Not a boundary: must not match.
        assert_eq!(strip_upstream_path("/apifoo", "/api"), None);
        assert_eq!(strip_upstream_path("/v1/chat", "/api"), None);
        // A bare-host upstream passes the path through untouched.
        assert_eq!(
            strip_upstream_path("/v1/messages", ""),
            Some("/v1/messages")
        );
    }

    #[test]
    fn intercepts_only_enabled_matching_hosts() {
        let d = anthropic();
        assert!(should_intercept_host(&d, "api.anthropic.com"));
        assert!(should_intercept_host(&d, "API.ANTHROPIC.COM")); // case-insensitive
        assert!(!should_intercept_host(&d, "example.com"));
        assert!(!should_intercept_host(&d, "statsig.anthropic.com"));
    }

    #[test]
    fn disabled_domain_is_not_intercepted() {
        let mut d = anthropic();
        d[0].enabled = false;
        assert!(!should_intercept_host(&d, "api.anthropic.com"));
        assert_eq!(
            decide(&d, "api.anthropic.com", "/v1/messages"),
            Decision::Tunnel
        );
    }

    #[test]
    fn rewrites_inference_path() {
        let d = anthropic();
        assert_eq!(
            decide(&d, "api.anthropic.com", "/v1/messages?beta=true"),
            Decision::Rewrite {
                upstream_url: "https://api.anthropic.com".into()
            }
        );
    }

    #[test]
    fn passes_through_updater_path_on_matched_host() {
        let d = anthropic();
        // /api/desktop/* is a passthrough prefix → must not be rewritten.
        assert_eq!(
            decide(&d, "api.anthropic.com", "/api/desktop/RELEASES"),
            Decision::Passthrough
        );
    }

    #[test]
    fn passes_through_unknown_path_on_matched_host() {
        let d = anthropic();
        assert_eq!(
            decide(&d, "api.anthropic.com", "/healthz"),
            Decision::Passthrough
        );
    }

    /// Regression for AG (Claude Desktop 503s): OAuth/account calls live under
    /// /v1/ on the same intercepted host but carry no model, so they must reach
    /// real api.anthropic.com untouched - never be rewritten to the gateway
    /// (which rejects them 503 "AI unknown"). Guards against re-widening
    /// `rewrite_prefixes` back to "/v1/".
    #[test]
    fn passes_through_oauth_and_account_paths() {
        let d = anthropic();
        for path in [
            "/v1/oauth/token",
            "/v1/organizations",
            "/v1/organizations/me",
            "/v1/models",
        ] {
            assert_eq!(
                decide(&d, "api.anthropic.com", path),
                Decision::Passthrough,
                "non-inference path {path} must pass through, not rewrite"
            );
        }
    }

    #[test]
    fn rewrites_legacy_complete_and_count_tokens() {
        let d = anthropic();
        // Legacy text-completions endpoint.
        assert_eq!(
            decide(&d, "api.anthropic.com", "/v1/complete"),
            Decision::Rewrite {
                upstream_url: "https://api.anthropic.com".into()
            }
        );
        // count_tokens rides under /v1/messages, so the prefix still catches it.
        assert_eq!(
            decide(&d, "api.anthropic.com", "/v1/messages/count_tokens"),
            Decision::Rewrite {
                upstream_url: "https://api.anthropic.com".into()
            }
        );
    }

    #[test]
    fn ignores_unmatched_host() {
        let d = anthropic();
        assert_eq!(decide(&d, "example.com", "/v1/messages"), Decision::Tunnel);
    }

    /// The catalog's `openai` entry must be a supported, routable upstream so
    /// the proxy can intercept Codex (API-key mode) and other OpenAI clients.
    fn openai() -> Vec<ProxyDomain> {
        let mut d = default_domains()
            .into_iter()
            .find(|d| d.slug == "openai")
            .expect("openai domain present in catalog");
        d.enabled = true; // catalog default is opt-in; enable for the test
        vec![d]
    }

    #[test]
    fn openai_is_supported() {
        let d = default_domains()
            .into_iter()
            .find(|d| d.slug == "openai")
            .unwrap();
        assert!(d.supported, "openai must be a supported upstream");
    }

    #[test]
    fn openrouter_is_supported() {
        let d = default_domains()
            .into_iter()
            .find(|d| d.slug == "openrouter")
            .unwrap();
        assert!(d.supported, "openrouter must be a supported upstream");
    }

    #[test]
    fn rewrites_openrouter_api_path() {
        let mut d = default_domains()
            .into_iter()
            .find(|d| d.slug == "openrouter")
            .expect("openrouter domain present in catalog");
        d.enabled = true; // catalog default is opt-in; enable for the test
        let d = vec![d];
        // OpenRouter's chat/completions lives at openrouter.ai/api/v1/*. The
        // client still calls that, but `decide` matches on the path Gate will
        // see - `/v1/...` - because the `/api` travels in the upstream URL to
        // clear Gate's ALB rule on `/api/*`.
        assert_eq!(
            decide(&d, "openrouter.ai", "/api/v1/chat/completions"),
            Decision::Rewrite {
                upstream_url: "https://openrouter.ai/api".into()
            }
        );
        // Outside the upstream's subtree: not this domain's traffic.
        assert_eq!(
            decide(&d, "openrouter.ai", "/v1/chat/completions"),
            Decision::Passthrough
        );
        // Path-boundary guard - `/apifoo` must not read as `/api` + `foo`.
        assert_eq!(
            decide(&d, "openrouter.ai", "/apifoo/v1/chat"),
            Decision::Passthrough
        );
        assert!(should_intercept_host(&d, "OPENROUTER.AI"));
    }

    #[test]
    fn rewrites_codex_apikey_responses_path() {
        let d = openai();
        // Codex in API-key mode (and the OpenAI SDK) hit
        // api.openai.com/v1/responses, which must rewrite to the gateway
        // with the OpenAI upstream injected.
        assert_eq!(
            decide(&d, "api.openai.com", "/v1/responses"),
            Decision::Rewrite {
                upstream_url: "https://api.openai.com".into()
            }
        );
        // case-insensitive host match
        assert!(should_intercept_host(&d, "API.OPENAI.COM"));
    }

    /// OpenAI mirror of `passes_through_oauth_and_account_paths`: non-inference
    /// /v1/ calls must pass through to real api.openai.com, not be rewritten to
    /// the gateway (which 503s on a modelless request). Chat + legacy
    /// completions still rewrite.
    #[test]
    fn openai_passes_through_non_inference_and_rewrites_chat() {
        let d = openai();
        for path in ["/v1/models", "/v1/files", "/v1/assistants"] {
            assert_eq!(
                decide(&d, "api.openai.com", path),
                Decision::Passthrough,
                "non-inference path {path} must pass through, not rewrite"
            );
        }
        for path in ["/v1/chat/completions", "/v1/completions", "/v1/embeddings"] {
            assert_eq!(
                decide(&d, "api.openai.com", path),
                Decision::Rewrite {
                    upstream_url: "https://api.openai.com".into()
                },
                "inference path {path} must rewrite to the gateway"
            );
        }
    }

    #[test]
    fn openai_domain_does_not_match_chatgpt_host() {
        // The api.openai.com domain is scoped to that host only - it must not
        // match chatgpt.com. (Codex's chatgpt.com traffic comes from its Rust
        // agent, which bypasses the system proxy, so it's out of the proxy's
        // reach entirely - covered by the manual Codex integration instead.)
        let d = openai();
        assert!(!should_intercept_host(&d, "chatgpt.com"));
        assert_eq!(
            decide(&d, "chatgpt.com", "/backend-api/codex/responses"),
            Decision::Tunnel
        );
    }
}
