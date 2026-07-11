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

use serde::{Deserialize, Serialize};

pub mod engine;

mod cert_authority;

/// Plaintext loopback reverse proxy for CLI tools; hosted in the engine.
mod relay;

pub mod config;

pub mod intent;

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
            display_name: "Anthropic (Claude Desktop / Cowork)".into(),
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
            display_name: "OpenAI".into(),
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
            slug: "openrouter".into(),
            display_name: "OpenRouter".into(),
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
}
