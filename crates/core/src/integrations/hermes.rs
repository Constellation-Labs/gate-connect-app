//! Hermes integration.
//!
//! Hermes is a Python-based OpenAI-compatible agent CLI with a single
//! `model.base_url` in `~/.hermes/cli-config.yaml`. Gate Connect redirects
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
//! connect/disconnect rewrites `cli-config.yaml` and **drops any comments**
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

        let Some(_state) = load_state()? else {
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

        if base_url == expected_base && has_gate_key && has_upstream_url {
            Ok(Status::Connected)
        } else {
            Ok(Status::Drifted(format!(
                "Hermes config does not match Gate settings (base_url: {base_url:?}, expected: {expected_base:?})"
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

        let current_base_url = model
            .get(&base_url_key)
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| HERMES_DEFAULT_BASE_URL.to_string());

        if is_local_url(&current_base_url) {
            anyhow::bail!(
                "Hermes is pointed at a local endpoint ({current_base_url:?}) -- skipping to avoid redirecting local traffic through Gate"
            );
        }

        let upstream_url = upstream_url_from_base(&current_base_url);

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
            "Hermes does not need a separate upstream credential -- Gate Connect injects its headers into ~/.hermes/cli-config.yaml."
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
    let mut body = serde_yaml::to_string(settings).context("serializing cli-config.yaml")?;
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
}
