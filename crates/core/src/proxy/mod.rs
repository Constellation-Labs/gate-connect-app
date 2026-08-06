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

/// Decide what to do with a request given its host + path. Passthrough
/// prefixes win over rewrite prefixes; a matched host with an unmatched
/// path is left alone (passthrough) rather than rewritten.
pub(crate) fn decide(domains: &[ProxyDomain], host: &str, path: &str) -> Decision {
    for d in domains.iter().filter(|d| d.enabled) {
        if !d.matches_host(host) {
            continue;
        }
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
            // and is deliberately left tunnelled, never intercepted. claude.ai
            // is the web/chat/login surface and is NOT part of this entry — it
            // speaks a different protocol and has its own opt-in `claude-web`
            // domain below.
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
            slug: "claude-web".into(),
            // Claude Desktop's CHAT surface, which is a different protocol from
            // the entry above rather than more of the same host. That one covers
            // api.anthropic.com /v1/messages; this one covers claude.ai, where
            // the desktop app sends a bare `prompt` string and Anthropic keeps
            // the conversation history server-side. Gate recognises it as the
            // `claude-web-chat` surface and treats it as inspection + audit, not
            // as key-brokered routing: there is no API key involved at all.
            display_name: "Claude Desktop chat".into(),
            hosts: vec!["claude.ai".into()],
            upstream_url: "https://claude.ai".into(),
            // Prefix matching cannot isolate the chat call on its own: the
            // endpoint is `/api/organizations/{org}/chat_conversations/{conv}/completion`,
            // so the varying id sits BEFORE the distinguishing final segment.
            // Rewriting the whole `/api/organizations/` tree is deliberate and
            // safe — Gate classifies only the completion path as the chat
            // surface and forwards the sibling calls (skills, usage,
            // conversation reads) as ordinary passthrough, which it has explicit
            // coverage for. They add audit rows, not behaviour changes.
            rewrite_prefixes: vec!["/api/organizations/".into()],
            // Everything here would be pure noise or actively harmful to route:
            // the updater channel, telemetry batches, and the bootstrap/account
            // calls the app makes before any conversation exists.
            passthrough_prefixes: vec![
                "/api/desktop/".into(),
                "/api/event_logging/".into(),
                "/api/bootstrap/".into(),
            ],
            // Opt-in. This surface carries the user's Claude SESSION cookie
            // rather than an API key, so it should never start intercepting
            // without a deliberate toggle.
            //
            // Deliberately NOT attached to the `anthropic` provider's
            // `proxy_domain_slugs` (see `provider.rs`): `provider::enable` turns
            // on every domain a provider lists, so attaching it would route the
            // session surface the moment someone enabled "Claude" — defeating
            // the opt-in above. It is reached only through the domain-level
            // toggle (`proxy domain claude-web on`), like the google domains.
            enabled: false,
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
            slug: "chatgpt-apps".into(),
            display_name: "ChatGPT app chat + Codex tools".into(),
            // Codex Desktop's TOOL traffic, which is a separate route from the
            // `chatgpt` entry below even though both name chatgpt.com.
            //
            // That entry is RELAY-only: it exists so the relay recognises the
            // upstream hint `integrations/codex.rs` writes, and it is matched by
            // `relay::route` on `upstream_url`. This entry is the MITM half,
            // matched by `decide` on HOST — which is why the two can share a
            // host without colliding, and why the split below differs.
            //
            // What this can and cannot capture: Codex's embedded Rust agent
            // ignores the system proxy, so its MODEL calls stay invisible to the
            // engine (they route via the relay instead — see below). The Electron
            // shell DOES honour the system proxy, and the tool-plane calls
            // observed in a capture came from it: `/backend-api/wham/*` carried a
            // Chromium user-agent. `/backend-api/ps/mcp` sends no user-agent at
            // all, so whether the engine sees it is genuinely unverified — this
            // entry is how we find out.
            hosts: vec!["chatgpt.com".into()],
            // MITM convention: `engine::apply_rewrite` preserves the request path
            // and query VERBATIM and swaps only scheme + authority, so the
            // upstream is the BARE host and the paths below are the app's real
            // ones. The relay entry uses the opposite split (`/backend-api` in
            // the upstream, short client path) because the relay sees the path
            // Codex rewrote, not the real one. Gate accepts both spellings.
            upstream_url: "https://chatgpt.com".into(),
            // Only the two path families Gate classifies as native surfaces:
            // the MCP tool plane (`codex-mcp`, scanned for indirect injection)
            // and the task/settings reads (`codex-tasks`). Deliberately NOT
            // `/backend-api/codex/responses` — the model call belongs to the
            // relay route, and rewriting it here would send it upstream with the
            // wrong split. Plugin-store listings are left out as pure noise.
            // `/backend-api/f/conversation` is the ChatGPT app's own chat turn
            // (Gate's `chatgpt-web-chat` surface): one message per request, reply
            // as a `delta_encoding: v1` SSE stream. It lives in THIS entry rather
            // than its own because `decide` returns on the first enabled
            // host match — a second chatgpt.com entry would be dead code.
            //
            // The `…/f/conversation/prepare` sibling is deliberately absent: it
            // only mints a short-lived `conduit_token` and carries neither prompt
            // nor reply, so routing it would add audit noise and nothing else.
            rewrite_prefixes: vec![
                "/backend-api/f/conversation".into(),
                "/backend-api/ps/mcp".into(),
                "/backend-api/wham/".into(),
            ],
            // `/backend-api/f/conversation/prepare` starts with the chat prefix
            // above, so it needs an explicit passthrough to stay unrouted —
            // passthrough prefixes are checked first in `decide`.
            passthrough_prefixes: vec!["/backend-api/f/conversation/prepare".into()],
            // Opt-in, and ordered BEFORE the relay `chatgpt` entry on purpose:
            // `decide` returns on the FIRST enabled host match, so with the relay
            // entry first this one would be unreachable for MITM traffic.
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
            upstream_url: "https://openrouter.ai".into(),
            rewrite_prefixes: vec!["/api/".into()],
            passthrough_prefixes: vec![],
            enabled: false,
            supported: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anthropic() -> Vec<ProxyDomain> {
        vec![default_domains().into_iter().next().unwrap()]
    }

    /// The `claude-web` entry, force-enabled: it ships opt-out because the
    /// surface carries a session cookie rather than an API key.
    fn claude_web() -> Vec<ProxyDomain> {
        let mut d: ProxyDomain = default_domains()
            .into_iter()
            .find(|d| d.slug == "claude-web")
            .expect("claude-web is in the catalog");
        d.enabled = true;
        vec![d]
    }

    /// The `chatgpt-apps` MITM entry (chat + Codex tool plane), force-enabled.
    fn chatgpt_apps() -> Vec<ProxyDomain> {
        let mut d: ProxyDomain = default_domains()
            .into_iter()
            .find(|d| d.slug == "chatgpt-apps")
            .expect("chatgpt-apps is in the catalog");
        d.enabled = true;
        vec![d]
    }

    const CLAUDE_COMPLETION: &str =
        "/api/organizations/b44129f9-a8ea-4f96-a137-b14a560e58d3/chat_conversations/2f261f16-2b31-41f8-b441-6067464c6504/completion";

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
        // OpenRouter's chat/completions lives at openrouter.ai/api/v1/*, which
        // must rewrite to the gateway with the OpenRouter upstream injected.
        assert_eq!(
            decide(&d, "openrouter.ai", "/api/v1/chat/completions"),
            Decision::Rewrite {
                upstream_url: "https://openrouter.ai".into()
            }
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
    #[test]
    fn claude_web_ships_disabled_so_a_session_surface_is_never_routed_silently() {
        let catalog = default_domains();
        let d = catalog.iter().find(|d| d.slug == "claude-web").unwrap();
        assert!(!d.enabled, "claude-web must be opt-in");
        assert!(d.supported);
        assert_eq!(d.hosts, vec!["claude.ai".to_string()]);
        assert_eq!(d.upstream_url, "https://claude.ai");
    }

    #[test]
    fn claude_web_rewrites_the_chat_completion_call() {
        let d = claude_web();
        assert_eq!(
            decide(&d, "claude.ai", CLAUDE_COMPLETION),
            Decision::Rewrite {
                upstream_url: "https://claude.ai".into()
            }
        );
        // Query strings must not change the verdict.
        assert_eq!(
            decide(
                &d,
                "claude.ai",
                &format!("{CLAUDE_COMPLETION}?rendering_mode=messages")
            ),
            Decision::Rewrite {
                upstream_url: "https://claude.ai".into()
            }
        );
    }

    #[test]
    fn claude_web_leaves_updater_telemetry_and_bootstrap_alone() {
        let d = claude_web();
        for path in [
            "/api/desktop/RELEASES",
            "/api/event_logging/v2/batch",
            "/api/bootstrap/b44129f9/current_user_access",
        ] {
            assert_eq!(
                decide(&d, "claude.ai", path),
                Decision::Passthrough,
                "{path} must reach the real host untouched"
            );
        }
    }

    #[test]
    fn claude_web_routes_sibling_org_calls_but_gate_classifies_them_separately() {
        // Prefix matching cannot isolate the completion path (the conversation id
        // precedes the distinguishing final segment), so these ride along. That
        // is deliberate: Gate tags only the completion call as the chat surface
        // and forwards the rest as ordinary passthrough.
        let d = claude_web();
        for path in [
            "/api/organizations/b44129f9/skills/list-skills",
            "/api/organizations/b44129f9/usage",
        ] {
            assert_eq!(
                decide(&d, "claude.ai", path),
                Decision::Rewrite {
                    upstream_url: "https://claude.ai".into()
                }
            );
        }
    }

    #[test]
    fn claude_web_does_not_touch_paths_outside_the_api_tree() {
        let d = claude_web();
        assert_eq!(decide(&d, "claude.ai", "/chat/abc"), Decision::Passthrough);
        assert_eq!(
            decide(&d, "claude.ai", "/_next/static/x.js"),
            Decision::Passthrough
        );
    }

    #[test]
    fn claude_web_and_anthropic_stay_separate_domains() {
        // The api.anthropic.com entry must not start matching claude.ai, and the
        // chat entry must not claim the API host: they are different protocols.
        assert_eq!(
            decide(&anthropic(), "claude.ai", CLAUDE_COMPLETION),
            Decision::Tunnel
        );
        assert_eq!(
            decide(&claude_web(), "api.anthropic.com", "/v1/messages"),
            Decision::Tunnel
        );
    }

    #[test]
    fn ca_name_constraints_cover_claude_ai_once_the_domain_ships() {
        // The CA's permitted subtrees are built from the WHOLE catalog, so a
        // missing host here means interception fails at the handshake.
        let hosts: Vec<String> = default_domains()
            .iter()
            .flat_map(|d| d.hosts.iter().cloned())
            .collect();
        assert!(hosts.contains(&"claude.ai".to_string()));
    }
    #[test]
    fn chatgpt_apps_rewrites_the_tool_plane_paths() {
        let d = chatgpt_apps();
        for path in [
            "/backend-api/ps/mcp",
            "/backend-api/wham/tasks/list",
            "/backend-api/wham/usage",
        ] {
            assert_eq!(
                decide(&d, "chatgpt.com", path),
                Decision::Rewrite {
                    upstream_url: "https://chatgpt.com".into()
                },
                "{path} should route to Gate"
            );
        }
    }

    #[test]
    fn chatgpt_apps_leaves_the_model_call_to_the_relay_route() {
        // The embedded agent reaches chatgpt.com directly and is routed by
        // base_url through the relay, whose entry uses the other URL split.
        // Rewriting it here would send it upstream with `/backend-api` doubled.
        let d = chatgpt_apps();
        assert_eq!(
            decide(&d, "chatgpt.com", "/backend-api/codex/responses"),
            Decision::Passthrough
        );
    }

    #[test]
    fn chatgpt_apps_ignores_plugin_store_and_auth_noise() {
        let d = chatgpt_apps();
        for path in [
            "/backend-api/ps/plugins/installed",
            "/backend-api/settings/user",
            "/api/auth/session",
        ] {
            assert_eq!(decide(&d, "chatgpt.com", path), Decision::Passthrough);
        }
    }

    #[test]
    fn chatgpt_apps_is_ordered_before_the_relay_chatgpt_entry() {
        // Load-bearing: `decide` returns on the FIRST enabled host match, and both
        // entries name chatgpt.com. With the relay entry first, the MITM entry
        // would be unreachable and the tool plane would silently pass through.
        let catalog = default_domains();
        let mitm = catalog
            .iter()
            .position(|d| d.slug == "chatgpt-apps")
            .unwrap();
        let relay = catalog.iter().position(|d| d.slug == "chatgpt").unwrap();
        assert!(
            mitm < relay,
            "chatgpt-apps must precede chatgpt in the catalog"
        );
    }

    #[test]
    fn the_two_chatgpt_entries_use_opposite_url_splits() {
        // `relay::route` matches on upstream_url and `decide` on host, so the
        // entries coexist — but only because their upstreams differ. Collapsing
        // them onto one upstream would break whichever route lost.
        let catalog = default_domains();
        let mitm = catalog.iter().find(|d| d.slug == "chatgpt-apps").unwrap();
        let relay = catalog.iter().find(|d| d.slug == "chatgpt").unwrap();
        assert_eq!(mitm.upstream_url, "https://chatgpt.com");
        assert_eq!(relay.upstream_url, "https://chatgpt.com/backend-api");
        assert_ne!(mitm.upstream_url, relay.upstream_url);
        // Both opt-in.
        assert!(!mitm.enabled && !relay.enabled);
    }

    #[test]
    fn enabling_only_the_relay_entry_keeps_todays_passthrough_behaviour() {
        // A user who enabled Codex CLI but not the desktop tools must see no
        // change: the relay entry matches the host first and passes everything
        // except its own short path through.
        let mut relay_only: Vec<ProxyDomain> = default_domains()
            .into_iter()
            .filter(|d| d.slug == "chatgpt")
            .collect();
        relay_only[0].enabled = true;
        assert_eq!(
            decide(&relay_only, "chatgpt.com", "/backend-api/ps/mcp"),
            Decision::Passthrough
        );
    }
    #[test]
    fn chatgpt_apps_rewrites_the_chat_turn() {
        let d = chatgpt_apps();
        assert_eq!(
            decide(&d, "chatgpt.com", "/backend-api/f/conversation"),
            Decision::Rewrite {
                upstream_url: "https://chatgpt.com".into()
            }
        );
    }

    #[test]
    fn chatgpt_apps_leaves_the_conduit_prepare_call_alone() {
        // `…/f/conversation/prepare` only mints a short-lived conduit token and
        // carries neither prompt nor reply. It shares the chat prefix, so it needs
        // the explicit passthrough — which `decide` checks BEFORE rewrites.
        let d = chatgpt_apps();
        assert_eq!(
            decide(&d, "chatgpt.com", "/backend-api/f/conversation/prepare"),
            Decision::Passthrough
        );
    }

    #[test]
    fn chatgpt_apps_leaves_the_sentinel_proof_of_work_alone() {
        // The app computes its own sentinel tokens; routing those adds nothing.
        let d = chatgpt_apps();
        assert_eq!(
            decide(
                &d,
                "chatgpt.com",
                "/backend-api/sentinel/chat-requirements/prepare"
            ),
            Decision::Passthrough
        );
    }
}
