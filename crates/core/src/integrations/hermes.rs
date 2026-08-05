//! Hermes integration.
//!
//! Hermes is a Python-based OpenAI-compatible agent CLI with a single
//! `model.base_url` in `~/.hermes/config.yaml`. Gate Connect redirects that
//! endpoint through the loopback reverse-proxy relay ([`crate::proxy::relay`])
//! by setting `model.base_url` to
//! `http://127.0.0.1:<relay-port>/<slug><client-path>`, and writes nothing
//! else. Both parts come from [`crate::proxy::resolve_endpoint`]: `<slug>` names
//! the catalog domain, which is how the relay knows where to forward, and
//! `<client-path>` keeps whatever sits between the upstream host and Hermes' own
//! suffix (`/api/v1` for the OpenRouter default, `/v1` for OpenAI).
//!
//! **No header and no credential is written.** The relay injects the Gate
//! credential and the upstream hint per request, so a token refresh is invisible
//! here. That also means we no longer touch `model.default_headers`, which is
//! not a documented Hermes field - it worked only because the model block is
//! passed through to the OpenAI client constructor, and upstream is still
//! deciding what to name it (NousResearch/hermes-agent#12785). The user's own
//! upstream credentials are untouched -- Gate is a pure passthrough on
//! `Authorization` / `x-api-key`.
//!
//! State tracking: a sidecar at `<app_support_dir>/hermes-state.json` records
//! the original `base_url` so `disconnect` can restore it exactly. The sidecar is
//! written atomically before the config is mutated and deleted only after the
//! config is restored.
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
        if launcher_paths()?.iter().any(|p| p.exists()) {
            return Ok(true);
        }
        Ok(launcher_on_path())
    }

    fn config_is_managed(&self) -> Result<bool> {
        // Two-part marker. The sidecar only exists because connect() wrote it,
        // but on its own it can't tell our stale write apart from a base_url
        // the user has since repointed by hand - so also require that what's on
        // disk still aims at loopback, which is only ever us. That keeps the
        // reconcile pass reasserting a dead relay port while leaving a
        // deliberate out-of-app endpoint alone.
        if load_state()?.is_none() {
            return Ok(false);
        }
        let Some(settings) = load_settings()? else {
            return Ok(false);
        };
        Ok(settings
            .get("model")
            .and_then(|v| v.as_mapping())
            .and_then(|m| m.get("base_url"))
            .and_then(|v| v.as_str())
            .is_some_and(is_local_url))
    }

    fn status(&self) -> Result<Status> {
        if !self.detect()? {
            return Ok(Status::NotInstalled);
        }

        let Some(state) = load_state()? else {
            return Ok(Status::Detected);
        };

        let expected_base = match crate::proxy::relay_base_url() {
            Some(relay) => {
                // Which path Hermes must keep depends on the endpoint it was
                // pointed at before we redirected it (`/v1` for OpenAI,
                // `/api/v1` for OpenRouter), and the sidecar is where that
                // original lives.
                let original = state
                    .previous_base_url
                    .as_deref()
                    .unwrap_or(HERMES_DEFAULT_BASE_URL);
                match crate::proxy::resolve_endpoint(original) {
                    Some(r) => r.relay_base_url(&relay),
                    None => {
                        return Ok(Status::Drifted(format!(
                            "Gate has no upstream domain for {original:?}"
                        )));
                    }
                }
            }
            None => {
                return Ok(Status::Drifted(
                    "the Gate proxy has not been enabled yet -- turn it on to route Hermes".into(),
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

        // The base_url is the whole test: it carries the relay origin, the
        // catalog slug the relay routes on, and the endpoint's own path. No
        // credential and no header is written, so there is nothing else to check.
        if base_url == expected_base {
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
        let relay_base_url = input.relay_base_url.as_deref().context(
            "the Gate proxy relay is not running -- enable the proxy before connecting Hermes",
        )?;

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

        // Resolve the user's own endpoint against the catalog. This decides both
        // halves at once: the upstream Gate forwards to, and the path Hermes
        // must keep on its side. Deriving them separately is what broke the
        // default OpenRouter config - stripping `/v1` off
        // `https://openrouter.ai/api/v1` yields `https://openrouter.ai/api`,
        // which matches no catalog entry, so every request 403'd.
        let resolved = crate::proxy::resolve_endpoint(&current_base_url).with_context(|| {
            format!(
                "Hermes is pointed at {current_base_url:?}, which Gate has no upstream domain for - \
                 point it at a supported provider first"
            )
        })?;
        let relay_base = resolved.relay_base_url(relay_base_url);

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

        // Redirect base_url at the relay. The slug segment it carries is how the
        // relay knows which upstream this is, so nothing else has to be written.
        model.insert(base_url_key, Value::String(relay_base));

        // `model.default_headers` is left alone entirely. It is not a documented
        // Hermes field - it works only because the model block is passed through
        // to the OpenAI client constructor, and upstream is still deciding what to
        // call it (NousResearch/hermes-agent#12785) - so depending on it was the
        // most fragile thing this integration did.

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

    fn save_upstream_credential(&self, _credential: &str) -> Result<()> {
        anyhow::bail!(
            "Hermes does not need a separate upstream credential -- Gate Connect injects its headers into ~/.hermes/config.yaml."
        )
    }

    /// Hidden from the popover: `model.default_headers` applies on the OpenAI
    /// wire only, and we never read `api_mode`. On a native-Anthropic setup the
    /// `X-Gate-Upstream-Url` hint is silently not sent and the relay rejects
    /// the request outright, while `status()` still reports Connected.
    /// See docs/harness-integration-validation.md H1.
    fn hidden_in_ui(&self) -> bool {
        true
    }

    fn has_upstream_credential(&self) -> Result<bool> {
        Ok(true)
    }

    fn clear_upstream_credential(&self) -> Result<()> {
        Ok(())
    }
}

/// Where the Hermes launcher actually lands, checked instead of "the config
/// directory exists".
///
/// The installer writes `~/.hermes/config.yaml` from a template and drops the
/// launcher in `~/.local/bin` - so treating the config directory as proof of
/// installation reported Hermes as installed long after it was removed, and
/// left the app offering to configure a CLI that wasn't there.
fn launcher_paths() -> Result<Vec<std::path::PathBuf>> {
    let home = crate::env::home()?;
    Ok(vec![
        home.join(".local/bin/hermes"),
        crate::env::hermes_config_dir()?.join("bin/hermes"),
    ])
}

/// Whether a `hermes` executable is reachable on `$PATH`.
///
/// A supplement to [`launcher_paths`], never a replacement: launched from Finder
/// or launchd the app inherits a minimal `PATH` that excludes `~/.local/bin`, so
/// this would miss the standard install in exactly the case that matters. It
/// covers the reverse - Hermes somewhere unusual (a venv, `/opt`, a scratch
/// HOME) that the absolute paths don't know about.
fn launcher_on_path() -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        if dir.as_os_str().is_empty() {
            return false;
        }
        #[cfg(target_os = "windows")]
        let names: &[&str] = &["hermes.exe", "hermes.cmd", "hermes.bat"];
        #[cfg(not(target_os = "windows"))]
        let names: &[&str] = &["hermes"];
        names.iter().any(|n| dir.join(n).is_file())
    })
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
    fn resolving_the_default_endpoint_keeps_api_v1_on_the_client() {
        // Hermes ships pointed at OpenRouter, whose API lives under `/api/v1`.
        // The old code derived the upstream by stripping `/v1`, producing
        // `https://openrouter.ai/api` - off-catalog, so the relay 403'd every
        // request from a default Hermes install. The `/api` has to stay on the
        // client side, where it also keeps the forwarded path on OpenRouter's
        // `/api/` inference prefix.
        let r = crate::proxy::resolve_endpoint(HERMES_DEFAULT_BASE_URL)
            .expect("the default Hermes endpoint must be routable");
        assert_eq!(r.upstream_url, "https://openrouter.ai");
        assert_eq!(r.client_path, "/api/v1");
        assert_eq!(
            format!("{}{}", "http://127.0.0.1:9977", r.client_path),
            "http://127.0.0.1:9977/api/v1"
        );

        // An OpenAI-shaped endpoint keeps just `/v1`.
        let r = crate::proxy::resolve_endpoint("https://api.openai.com/v1")
            .expect("openai endpoint resolves");
        assert_eq!(r.upstream_url, "https://api.openai.com");
        assert_eq!(r.client_path, "/v1");
    }

    #[test]
    fn an_endpoint_gate_cannot_route_is_refused_not_guessed() {
        // A custom endpoint no catalog domain covers must not be rewritten:
        // connect bails instead of pointing Hermes at a relay that would 403.
        assert!(crate::proxy::resolve_endpoint("https://api.custom.com").is_none());
        // Suffix confusion must not resolve to api.openai.com either.
        assert!(crate::proxy::resolve_endpoint("https://api.openai-v1.com").is_none());
    }

    #[test]
    fn local_url_detection() {
        assert!(is_local_url("http://localhost:1234/v1"));
        assert!(is_local_url("http://127.0.0.1:8080/v1"));
        assert!(is_local_url("http://my-service.local/v1"));
        assert!(!is_local_url("https://openrouter.ai/api/v1"));
        assert!(!is_local_url("https://api.openai.com/v1"));
    }
}
