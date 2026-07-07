//! Hermes integration.
//!
//! Hermes is a Python-based OpenAI-compatible agent CLI with a single
//! `model.base_url` in `~/.hermes/config.yaml`. Gate Connect redirects
//! that endpoint through the gateway by setting `model.base_url` to
//! `<gateway>/v1` and injecting Gate's two identification headers into
//! `model.default_headers`. The user's upstream credentials are untouched --
//! Gate is a pure passthrough on `Authorization` / `x-api-key`.
//!
//! State tracking: a sidecar at `<app_support_dir>/hermes-state.json` records
//! the original `base_url` and `default_headers` so `disconnect` can restore
//! them exactly. The sidecar is written atomically before the config is
//! mutated and deleted only after the config is restored.
//!
//! Config format: we parse and re-serialize with `serde_yaml`, which means a
//! connect/disconnect rewrites `config.yaml` and **drops any comments**
//! the user had in it (same precedent as OpenClaw's JSON5 rewrite). The
//! redirected `base_url` / `default_headers` values themselves are snapshotted
//! and restored exactly on disconnect.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::fs;
use std::path::Path;

use crate::registry::{ConnectInput, Integration, Status, ToolId};

const DISPLAY_NAME: &str = "Hermes";
const UPSTREAM_PROVIDER_NAME: &str = "openrouter";
const DEFAULT_UPSTREAM_URL: &str = "https://openrouter.ai/api/v1";

const HERMES_DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
const GATEWAY_PATH_SUFFIX: &str = "/v1";
const GATE_KEY_HEADER: &str = "X-Gate-Api-Key";
const UPSTREAM_URL_HEADER: &str = "X-Gate-Upstream-Url";
const STATE_FILENAME: &str = "hermes-state.json";

#[cfg(unix)]
const CLI_BIN_PATHS: &[&str] = &["/usr/local/bin/hermes", "/usr/bin/hermes"];
#[cfg(not(unix))]
const CLI_BIN_PATHS: &[&str] = &[];

/// Sidecar that records what `connect` changed so `disconnect` can restore it.
#[derive(Debug, Serialize, Deserialize, Default)]
struct State {
    #[serde(default = "default_version")]
    version: u8,
    /// The value of `model.base_url` before connect, or `None` if the key
    /// was absent.
    previous_base_url: Option<String>,
    /// True if `model.base_url` existed in the original config. When false,
    /// disconnect removes the key entirely rather than restoring a `None`.
    #[serde(default)]
    base_url_was_set: bool,
    /// Full contents of `model.default_headers` before connect, or `None` if
    /// the section was absent.
    previous_default_headers: Option<Vec<(String, String)>>,
}

fn default_version() -> u8 {
    1
}

pub struct Hermes;

impl Integration for Hermes {
    fn id(&self) -> ToolId {
        ToolId::Hermes
    }

    fn display_name(&self) -> &'static str {
        DISPLAY_NAME
    }

    fn upstream_provider_name(&self) -> &'static str {
        UPSTREAM_PROVIDER_NAME
    }

    fn default_upstream_url(&self) -> &'static str {
        DEFAULT_UPSTREAM_URL
    }

    fn requires_upstream_credential(&self) -> bool {
        false
    }

    fn detect(&self) -> Result<bool> {
        if CLI_BIN_PATHS.iter().any(|p| Path::new(p).exists()) {
            return Ok(true);
        }
        Ok(crate::env::hermes_config_dir()?.exists())
    }

    fn status(&self) -> Result<Status> {
        if !self.detect()? {
            return Ok(Status::NotInstalled);
        }

        let Some(state) = load_state()? else {
            return Ok(Status::Detected);
        };

        let expected_base = match crate::account::load_base_url()? {
            Some(u) => compute_base_url(&u),
            None => {
                return Ok(Status::Drifted(
                    "Gate Connect is not signed in -- sign in to validate Hermes config".into(),
                ));
            }
        };

        let Some(settings) = load_settings()? else {
            return Ok(Status::Detected);
        };

        let model = settings.get("model").and_then(|v| v.as_mapping());

        let base_url = model
            .and_then(|m| m.get("base_url"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let headers = model
            .and_then(|m| m.get("default_headers"))
            .and_then(|v| v.as_mapping());

        let has_gate_key = headers.and_then(|h| h.get(GATE_KEY_HEADER)).is_some();

        let has_upstream_url = headers.and_then(|h| h.get(UPSTREAM_URL_HEADER)).is_some();

        // Recompute the upstream we would pin for the *current* provider and
        // compare it to what's stored -- catches the user switching
        // model.provider after connecting without re-running connect. For
        // custom providers the live base_url is now Gate's URL, so we resolve
        // against the original base_url captured in state.
        let provider = model
            .and_then(|m| m.get("provider"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let stored_upstream = headers
            .and_then(|h| h.get(UPSTREAM_URL_HEADER))
            .and_then(|v| v.as_str());
        let expected_upstream = resolve_upstream_url(provider, state.previous_base_url.as_deref());
        let upstream_matches = stored_upstream == Some(expected_upstream.as_str());

        if base_url == expected_base && has_gate_key && has_upstream_url && upstream_matches {
            Ok(Status::Connected)
        } else {
            Ok(Status::Drifted(format!(
                "Hermes config does not match Gate settings (base_url: {base_url:?}, expected: {expected_base:?}; upstream stored: {stored_upstream:?}, expected: {expected_upstream:?})"
            )))
        }
    }

    fn connect(&self, input: &ConnectInput) -> Result<()> {
        if !self.detect()? {
            anyhow::bail!(
                "Hermes is not installed -- install it from https://github.com/nousresearch/hermes-agent first"
            );
        }
        if !input.gateway_base_url.starts_with("https://") {
            anyhow::bail!("gateway base URL must be https://");
        }

        let acct = crate::account::load()?.context("no Gate account found -- sign in first")?;

        let gateway_base_url = compute_base_url(&input.gateway_base_url);

        let mut settings = load_settings()?.unwrap_or_default();

        // Ensure `model` section exists.
        let model_key = Value::String("model".to_string());
        if !settings.contains_key(&model_key) {
            settings.insert(model_key.clone(), Value::Mapping(Mapping::new()));
        }

        let model = settings
            .get_mut(&model_key)
            .and_then(|v| v.as_mapping_mut())
            .context("model is not a mapping")?;

        let base_url_key = Value::String("base_url".to_string());
        let headers_key = Value::String("default_headers".to_string());

        let provider = model
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let base_url = model
            .get(&base_url_key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        // Built-in providers route to their fixed endpoint. For custom /
        // unrecognized providers we derive from base_url; guard against local
        // endpoints, and warn if we have to fall back to the OpenRouter default.
        if builtin_upstream_url(&provider).is_none() {
            match base_url.as_deref() {
                Some(b) if is_local_url(b) => {
                    anyhow::bail!(
                        "Hermes is pointed at a local endpoint ({b:?}) -- skipping to avoid redirecting local traffic through Gate"
                    );
                }
                None => {
                    eprintln!(
                        "[gate] Hermes model.provider {provider:?} is not a recognized built-in provider and model.base_url is not set; defaulting the upstream to OpenRouter ({HERMES_DEFAULT_BASE_URL}). Set model.provider (anthropic/openai/openrouter/google) or model.base_url to route elsewhere."
                    );
                }
                _ => {}
            }
        }

        let upstream_url = resolve_upstream_url(&provider, base_url.as_deref());

        // Snapshot original values before we touch anything.
        let state = State {
            version: 1,
            previous_base_url: model
                .get(&base_url_key)
                .and_then(|v| v.as_str())
                .map(String::from),
            base_url_was_set: model.contains_key(&base_url_key),
            previous_default_headers: model.get(&headers_key).and_then(|v| v.as_mapping()).map(
                |m| {
                    m.iter()
                        .filter_map(|(k, v)| {
                            Some((k.as_str()?.to_string(), v.as_str()?.to_string()))
                        })
                        .collect()
                },
            ),
        };
        save_state(&state)?;

        // Redirect base_url.
        model.insert(base_url_key, Value::String(gateway_base_url));

        // Inject Gate headers into default_headers.
        if !model.contains_key(&headers_key) {
            model.insert(headers_key.clone(), Value::Mapping(Mapping::new()));
        }
        let headers = model
            .get_mut(&headers_key)
            .and_then(|v| v.as_mapping_mut())
            .context("default_headers is not a mapping")?;

        headers.insert(
            Value::String(GATE_KEY_HEADER.to_string()),
            Value::String(acct.api_key.clone()),
        );
        headers.insert(
            Value::String(UPSTREAM_URL_HEADER.to_string()),
            Value::String(upstream_url),
        );

        write_settings(&settings)
    }

    fn disconnect(&self) -> Result<()> {
        let Some(state) = load_state()? else {
            return Ok(());
        };

        if let Some(mut settings) = load_settings()? {
            let model_key = Value::String("model".to_string());
            if let Some(model) = settings
                .get_mut(&model_key)
                .and_then(|v| v.as_mapping_mut())
            {
                let base_url_key = Value::String("base_url".to_string());
                let headers_key = Value::String("default_headers".to_string());

                // Restore base_url.
                if let Some(prev) = &state.previous_base_url {
                    model.insert(base_url_key, Value::String(prev.clone()));
                } else if !state.base_url_was_set {
                    model.remove(&base_url_key);
                }

                // Restore default_headers.
                match &state.previous_default_headers {
                    None => {
                        model.remove(&headers_key);
                    }
                    Some(prev_headers) => {
                        let mut restored = Mapping::new();
                        for (k, v) in prev_headers {
                            restored.insert(Value::String(k.clone()), Value::String(v.clone()));
                        }
                        model.insert(headers_key, Value::Mapping(restored));
                    }
                }
            }
            write_settings(&settings)?;
        }

        clear_state()
    }

    fn refresh_gate_key(&self, api_key: &str) -> Result<()> {
        let Some(_state) = load_state()? else {
            return Ok(());
        };
        let Some(mut settings) = load_settings()? else {
            return Ok(());
        };

        let model_key = Value::String("model".to_string());
        let Some(model) = settings
            .get_mut(&model_key)
            .and_then(|v| v.as_mapping_mut())
        else {
            return Ok(());
        };

        let headers_key = Value::String("default_headers".to_string());
        let Some(headers) = model.get_mut(&headers_key).and_then(|v| v.as_mapping_mut()) else {
            return Ok(());
        };

        headers.insert(
            Value::String(GATE_KEY_HEADER.to_string()),
            Value::String(api_key.to_string()),
        );

        write_settings(&settings)
    }

    fn save_upstream_credential(&self, _credential: &str) -> Result<()> {
        anyhow::bail!(
            "Hermes does not need a separate upstream credential -- Gate Connect injects its headers into ~/.hermes/config.yaml."
        )
    }

    fn has_upstream_credential(&self) -> Result<bool> {
        Ok(true)
    }

    fn clear_upstream_credential(&self) -> Result<()> {
        Ok(())
    }
}

/// Appends `/v1` to the gateway URL, stripping any trailing slash first.
fn compute_base_url(gateway_base_url: &str) -> String {
    format!(
        "{}{}",
        gateway_base_url.trim_end_matches('/'),
        GATEWAY_PATH_SUFFIX
    )
}

/// Strips the trailing `/v1` from a `base_url` to produce the value Gate
/// expects in `X-Gate-Upstream-Url`. Gate prepends this to the inbound
/// request path, so `/v1/chat/completions` must live on the client side.
fn upstream_url_from_base(base_url: &str) -> String {
    let s = base_url.trim_end_matches('/');
    if let Some(stripped) = s.strip_suffix("/v1") {
        stripped.trim_end_matches('/').to_string()
    } else {
        s.to_string()
    }
}

/// Maps a Hermes `model.provider` to the fixed upstream base URL Gate forwards
/// to (no `/v1`; Gate appends the caller's request path). Returns `None` for
/// `custom` / unrecognized providers, whose upstream comes from `model.base_url`.
fn builtin_upstream_url(provider: &str) -> Option<&'static str> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "anthropic" => Some("https://api.anthropic.com"),
        "openai" => Some("https://api.openai.com"),
        "openrouter" => Some("https://openrouter.ai/api"),
        "google" | "gemini" | "google-ai-studio" => {
            Some("https://generativelanguage.googleapis.com")
        }
        _ => None,
    }
}

/// Resolves the upstream base URL Gate should forward to for a Hermes
/// `provider` + optional `base_url`. Built-in providers map to their fixed
/// endpoint; everything else derives from `base_url`, falling back to the
/// OpenRouter default when `base_url` is absent. Shared by `connect` (the value
/// it pins) and `status` (the value it expects to still see).
fn resolve_upstream_url(provider: &str, base_url: Option<&str>) -> String {
    match builtin_upstream_url(provider) {
        Some(url) => url.to_string(),
        None => base_url
            .filter(|s| !s.is_empty())
            .map(upstream_url_from_base)
            .unwrap_or_else(|| upstream_url_from_base(HERMES_DEFAULT_BASE_URL)),
    }
}

/// Returns true if `base_url` targets a local address (loopback, link-local,
/// RFC-1918, `.local`, `.lan`, `.internal`). We skip those to avoid
/// redirecting traffic from private/local endpoints through Gate.
fn is_local_url(base_url: &str) -> bool {
    let authority = base_url
        .strip_prefix("http://")
        .or_else(|| base_url.strip_prefix("https://"))
        .unwrap_or(base_url)
        .split('/')
        .next()
        .unwrap_or("");
    let host = authority.split(':').next().unwrap_or("");
    matches!(
        host,
        "localhost" | "127.0.0.1" | "0.0.0.0" | "::1" | "metadata.google.internal"
    ) || host.ends_with(".local")
        || host.ends_with(".lan")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("169.254.")
        || host.ends_with(".internal")
}

fn state_path() -> Result<std::path::PathBuf> {
    Ok(crate::env::app_support_dir()?.join(STATE_FILENAME))
}

fn settings_path() -> Result<std::path::PathBuf> {
    crate::env::hermes_config_path()
}

fn load_settings() -> Result<Option<Mapping>> {
    let path = settings_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let val: Value =
        serde_yaml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    match val {
        Value::Mapping(m) => Ok(Some(m)),
        Value::Null => Ok(Some(Mapping::new())),
        _ => anyhow::bail!("{} is not a YAML mapping", path.display()),
    }
}

fn write_settings(settings: &Mapping) -> Result<()> {
    let path = settings_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut body = serde_yaml::to_string(settings).context("serializing config.yaml")?;
    if !body.ends_with('\n') {
        body.push('\n');
    }
    crate::primitives::write_file(&path, body.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

fn load_state() -> Result<Option<State>> {
    let path = state_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let state: State =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(state))
}

fn save_state(state: &State) -> Result<()> {
    let path = state_path()?;
    let body = serde_json::to_string_pretty(state).context("serializing hermes-state.json")?;
    crate::primitives::write_file(&path, body.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

fn clear_state() -> Result<()> {
    let path = state_path()?;
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_url_strips_v1() {
        assert_eq!(
            upstream_url_from_base("https://openrouter.ai/api/v1"),
            "https://openrouter.ai/api"
        );
        assert_eq!(
            upstream_url_from_base("https://api.openai.com/v1"),
            "https://api.openai.com"
        );
        assert_eq!(
            upstream_url_from_base("https://api.openai.com/v1/"),
            "https://api.openai.com"
        );
    }

    #[test]
    fn upstream_url_no_v1() {
        assert_eq!(
            upstream_url_from_base("https://api.custom.com/"),
            "https://api.custom.com"
        );
        assert_eq!(
            upstream_url_from_base("https://api.openai-v1.com"),
            "https://api.openai-v1.com"
        );
    }

    #[test]
    fn local_url_detection() {
        assert!(is_local_url("http://localhost:1234/v1"));
        assert!(is_local_url("http://127.0.0.1:8080/v1"));
        assert!(is_local_url("http://my-service.local/v1"));
        assert!(!is_local_url("https://openrouter.ai/api/v1"));
        assert!(!is_local_url("https://api.openai.com/v1"));
    }

    #[test]
    fn compute_base_url_appends_suffix() {
        assert_eq!(
            compute_base_url("https://gate.example.com"),
            "https://gate.example.com/v1"
        );
        assert_eq!(
            compute_base_url("https://gate.example.com/"),
            "https://gate.example.com/v1"
        );
    }

    #[test]
    fn builtin_upstream_url_maps_known_providers() {
        assert_eq!(
            builtin_upstream_url("anthropic"),
            Some("https://api.anthropic.com")
        );
        assert_eq!(
            builtin_upstream_url("openai"),
            Some("https://api.openai.com")
        );
        assert_eq!(
            builtin_upstream_url("openrouter"),
            Some("https://openrouter.ai/api")
        );
        assert_eq!(
            builtin_upstream_url("google"),
            Some("https://generativelanguage.googleapis.com")
        );
        assert_eq!(
            builtin_upstream_url("gemini"),
            Some("https://generativelanguage.googleapis.com")
        );
    }

    #[test]
    fn builtin_upstream_url_is_case_insensitive_and_trims() {
        assert_eq!(
            builtin_upstream_url("Anthropic"),
            Some("https://api.anthropic.com")
        );
        assert_eq!(
            builtin_upstream_url("  OpenAI  "),
            Some("https://api.openai.com")
        );
    }

    #[test]
    fn builtin_upstream_url_none_for_custom_and_unknown() {
        assert_eq!(builtin_upstream_url("custom"), None);
        assert_eq!(builtin_upstream_url(""), None);
        assert_eq!(builtin_upstream_url("mystery"), None);
    }

    #[test]
    fn resolve_upstream_url_prefers_provider_over_base_url() {
        // A built-in provider pins its endpoint regardless of base_url.
        assert_eq!(
            resolve_upstream_url("anthropic", None),
            "https://api.anthropic.com"
        );
        assert_eq!(
            resolve_upstream_url("anthropic", Some("https://openrouter.ai/api/v1")),
            "https://api.anthropic.com"
        );
        // Distinct providers resolve differently -- this is what drives drift
        // detection when a connected user switches model.provider.
        assert_ne!(
            resolve_upstream_url("anthropic", None),
            resolve_upstream_url("openai", None)
        );
    }

    #[test]
    fn resolve_upstream_url_custom_uses_base_url_else_warns_to_openrouter() {
        // custom with an explicit base_url derives from it.
        assert_eq!(
            resolve_upstream_url("custom", Some("https://api.mistral.ai/v1")),
            "https://api.mistral.ai"
        );
        // custom (or unknown) with no base_url falls back to the OpenRouter
        // default (the connect path additionally warns via eprintln).
        assert_eq!(
            resolve_upstream_url("custom", None),
            "https://openrouter.ai/api"
        );
        assert_eq!(
            resolve_upstream_url("", Some("")),
            "https://openrouter.ai/api"
        );
    }
}
