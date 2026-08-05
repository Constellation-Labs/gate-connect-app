//! Claude Code integration.
//!
//! Configures Anthropic's `claude` CLI to route through Constellation
//! Gate via the loopback reverse-proxy relay ([`crate::proxy::relay`]).
//! Mechanism: one entry in `~/.claude/settings.json`'s `env` block, which
//! Claude Code injects into its own process at every invocation:
//!
//! - `ANTHROPIC_BASE_URL` - the relay's loopback URL with the catalog slug on
//!   it (`http://127.0.0.1:<port>/anthropic`), *not* the gateway. Claude Code
//!   appends `/v1/messages`; the relay reads the slug to know the upstream,
//!   strips it, and injects both the upstream hint and the live Gate credential
//!   per request. So **neither a credential nor a header is written to this
//!   file** - `ANTHROPIC_CUSTOM_HEADERS` is no longer ours, and a value an
//!   older build left there is removed on connect and restored from
//!   `previousEnv` on disconnect.
//!
//! This is the successor to the old scheme that baked `X-Gate-Api-Key`
//! into this file and pointed the base URL straight at the gateway. It
//! means the tool only routes while Gate Connect (the relay) is running,
//! which is why connecting auto-enables the proxy.
//!
//! Unlike Cowork, Claude Code does not need a separate upstream
//! credential - it already authenticates to Anthropic with its own
//! OAuth token or `ANTHROPIC_API_KEY`, and Gate passes that through.
//! So [`requires_upstream_credential`] returns `false` and the
//! credential-related trait methods are no-ops.
//!
//! We track our own writes via a sibling `_gateConnect` block so
//! disconnect cleanly reverses what connect did and any prior
//! user-set values are restored.
//!
//! [`requires_upstream_credential`]: crate::Integration::requires_upstream_credential

use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::env;
use crate::registry::{ConnectInput, Integration, Status, ToolId};

const UPSTREAM_PROVIDER_NAME: &str = "Anthropic";
const DEFAULT_UPSTREAM_URL: &str = "https://api.anthropic.com";

const KEY_BASE_URL: &str = "ANTHROPIC_BASE_URL";
const KEY_CUSTOM_HEADERS: &str = "ANTHROPIC_CUSTOM_HEADERS";

const MARKER_KEY: &str = "_gateConnect";

/// Paths to look for the `claude` binary, in priority order. Covers
/// Homebrew on Apple Silicon, Homebrew/Intel + manual installs, plus
/// the standard Linux install location. Windows ships the binary into
/// a per-user npm prefix that's effectively unguessable, so detection
/// there relies on the `~/.claude` config-dir fallback below.
#[cfg(target_os = "macos")]
const CLAUDE_BIN_PATHS: &[&str] = &["/opt/homebrew/bin/claude", "/usr/local/bin/claude"];
#[cfg(all(unix, not(target_os = "macos")))]
const CLAUDE_BIN_PATHS: &[&str] = &["/usr/local/bin/claude", "/usr/bin/claude"];
#[cfg(windows)]
const CLAUDE_BIN_PATHS: &[&str] = &[];

pub struct ClaudeCode;

impl Integration for ClaudeCode {
    fn id(&self) -> ToolId {
        ToolId::ClaudeCode
    }

    fn display_name(&self) -> &'static str {
        "Claude Code"
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
        if CLAUDE_BIN_PATHS.iter().any(|p| Path::new(p).exists()) {
            return Ok(true);
        }
        // Fall back to the per-user config dir Claude Code writes on first
        // launch. Catches Volta/asdf/npx installs that don't land a binary
        // in the well-known paths above.
        Ok(env::claude_code_config_dir()?.exists())
    }

    fn status(&self) -> Result<Status> {
        if !self.detect()? {
            return Ok(Status::NotInstalled);
        }
        let settings = match load_settings()? {
            Some(s) => s,
            None => return Ok(Status::Detected),
        };

        let env_block = settings.get("env").and_then(|v| v.as_object());
        let marker = settings
            .get(MARKER_KEY)
            .and_then(|v| v.as_object())
            .and_then(|m| m.get("managed"));
        if env_block.is_none() || marker.is_none() {
            return Ok(Status::Detected);
        }
        let env_block = env_block.unwrap();

        let base_url = env_block.get(KEY_BASE_URL).and_then(|v| v.as_str());

        // The tool points at the relay's loopback URL, not the gateway; the
        // relay only exists once the proxy has been enabled. The base URL is the
        // whole test: it carries the relay origin plus the catalog slug the relay
        // routes on, and no header or credential is written alongside it.
        let expected_base = match crate::proxy::relay_base_url() {
            Some(relay) => match crate::proxy::resolve_endpoint(DEFAULT_UPSTREAM_URL) {
                Some(r) => r.relay_base_url(&relay),
                None => {
                    return Ok(Status::Drifted(format!(
                        "Gate has no upstream domain for {DEFAULT_UPSTREAM_URL:?}"
                    )));
                }
            },
            None => {
                return Ok(Status::Drifted(
                    "the Gate proxy has not been enabled yet - turn it on to route Claude Code"
                        .into(),
                ));
            }
        };

        match base_url {
            Some(b) if b == expected_base => Ok(Status::Connected),
            Some(b) => Ok(Status::Drifted(format!(
                "{KEY_BASE_URL} in settings.json is {b:?}, expected {expected_base:?}"
            ))),
            None => Ok(Status::Drifted(format!(
                "managed {KEY_BASE_URL} missing from settings.json env"
            ))),
        }
    }

    fn config_is_managed(&self) -> Result<bool> {
        // The same marker check status() gates on: only a settings.json we
        // wrote carries `_gateConnect.managed`.
        Ok(load_settings()?
            .as_ref()
            .and_then(|s| s.get(MARKER_KEY))
            .and_then(|v| v.as_object())
            .and_then(|m| m.get("managed"))
            .is_some())
    }

    fn connect(&self, input: &ConnectInput) -> Result<()> {
        if !self.detect()? {
            anyhow::bail!(
                "Claude Code is not installed on this machine - install it from https://claude.com/code first"
            );
        }
        let relay_base_url = input.relay_base_url.as_deref().context(
            "the Gate proxy relay is not running - enable the proxy before connecting Claude Code",
        )?;
        if !input.upstream_url.starts_with("https://") {
            anyhow::bail!("upstream URL must be https://");
        }

        // Resolve the upstream against the catalog: the slug it yields goes into
        // the base URL, which is how the relay knows where to forward. No
        // credential and no header is written - the relay injects both.
        let resolved = crate::proxy::resolve_endpoint(&input.upstream_url)
            .with_context(|| format!("Gate has no upstream domain for {:?}", input.upstream_url))?;
        let base_url = resolved.relay_base_url(relay_base_url);

        let mut settings = load_settings()?.unwrap_or_default();
        // Refuse to clobber a malformed non-object `env` before ensure_object
        // would silently replace it with `{}` (see reject_non_object_env).
        reject_non_object_env(&settings)?;
        let env_block = ensure_object(&mut settings, "env");

        // Save the prior values exactly once so disconnect can restore them.
        let mut prev = Map::new();
        if let Some(v) = env_block.get(KEY_BASE_URL) {
            prev.insert(KEY_BASE_URL.into(), v.clone());
        }
        if let Some(v) = env_block.get(KEY_CUSTOM_HEADERS) {
            prev.insert(KEY_CUSTOM_HEADERS.into(), v.clone());
        }

        env_block.insert(KEY_BASE_URL.into(), Value::String(base_url));
        // ANTHROPIC_CUSTOM_HEADERS is no longer ours. Remove any value an older
        // build left so no stale Gate header survives an upgrade; disconnect
        // restores the user's own value from `previousEnv` either way.
        env_block.remove(KEY_CUSTOM_HEADERS);

        let marker = ensure_object(&mut settings, MARKER_KEY);
        // Only stash previousEnv on the first connect; preserve it on
        // subsequent re-connects so disconnect always restores the
        // user's original state, not our own intermediate one.
        if !marker.contains_key("previousEnv") {
            marker.insert("previousEnv".into(), Value::Object(prev));
        }
        marker.insert(
            "managed".into(),
            Value::Array(vec![
                Value::String(KEY_BASE_URL.into()),
                Value::String(KEY_CUSTOM_HEADERS.into()),
            ]),
        );

        write_settings(&settings)
    }

    fn disconnect(&self) -> Result<()> {
        let path = settings_path()?;
        let Some(mut settings) = load_settings()? else {
            return Ok(());
        };

        let prev = settings
            .get(MARKER_KEY)
            .and_then(|m| m.get("previousEnv"))
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();

        if let Some(env_block) = settings.get_mut("env").and_then(|v| v.as_object_mut()) {
            for key in [KEY_BASE_URL, KEY_CUSTOM_HEADERS] {
                match prev.get(key) {
                    Some(v) => {
                        env_block.insert(key.into(), v.clone());
                    }
                    None => {
                        env_block.remove(key);
                    }
                }
            }
            // Drop the env block entirely if we left it empty so settings.json
            // stays tidy.
            if env_block.is_empty() {
                settings.remove("env");
            }
        }
        settings.remove(MARKER_KEY);

        // The file now holds nothing but our additions - remove it rather
        // than leaving a stray `{}` behind (matching Codex's disconnect).
        if settings.is_empty() {
            if path.exists() {
                fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
            }
            return Ok(());
        }
        write_settings(&settings)
    }

    fn save_upstream_credential(&self, _credential: &str) -> Result<()> {
        anyhow::bail!(
            "Claude Code does not need a separate upstream credential - it uses its own Anthropic auth"
        );
    }

    fn has_upstream_credential(&self) -> Result<bool> {
        Ok(true)
    }

    fn clear_upstream_credential(&self) -> Result<()> {
        Ok(())
    }
}

fn settings_path() -> Result<PathBuf> {
    env::claude_code_settings_path()
}

fn load_settings() -> Result<Option<Map<String, Value>>> {
    let path = settings_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {} as JSON", path.display()))?;
    match value {
        Value::Object(m) => Ok(Some(m)),
        _ => anyhow::bail!("{} top level must be a JSON object", path.display()),
    }
}

fn write_settings(settings: &Map<String, Value>) -> Result<()> {
    let path = settings_path()?;
    let mut body = serde_json::to_string_pretty(settings).context("serializing settings.json")?;
    body.push('\n');
    // 0o600: settings.json may carry the Gate API key via apiKeyHelper
    // env vars / headers. Atomic-write protects against partial writes.
    crate::primitives::write_file(&path, body.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

fn ensure_object<'a>(parent: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    if !matches!(parent.get(key), Some(Value::Object(_))) {
        parent.insert(key.into(), Value::Object(Map::new()));
    }
    parent
        .get_mut(key)
        .and_then(|v| v.as_object_mut())
        .expect("just inserted an object")
}

/// Guard against silently destroying a hand-edited, malformed `env`.
/// `ensure_object` would replace a non-object `env` with an empty object,
/// and disconnect - which only restores the keys we snapshotted - could
/// never bring the original value back. A `null` `env` carries no data, so
/// it is allowed to fall through and be replaced.
fn reject_non_object_env(settings: &Map<String, Value>) -> Result<()> {
    let bad = settings
        .get("env")
        .is_some_and(|v| !v.is_object() && !v.is_null());
    if bad {
        anyhow::bail!("~/.claude/settings.json has a non-object \"env\"; refusing to overwrite it");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_carries_the_catalog_slug_and_no_header_is_written() {
        // Everything the relay needs is in one documented env var. Nothing else
        // is written: no credential, and no ANTHROPIC_CUSTOM_HEADERS - Claude
        // Code appends `/v1/messages` onto this, and the relay strips the slug.
        let r = crate::proxy::resolve_endpoint(DEFAULT_UPSTREAM_URL).expect("anthropic resolves");
        assert_eq!(
            r.relay_base_url("http://127.0.0.1:9977"),
            "http://127.0.0.1:9977/anthropic"
        );
    }

    #[test]
    fn ensure_object_replaces_non_object() {
        let mut m = Map::new();
        m.insert("env".into(), Value::String("oops".into()));
        let obj = ensure_object(&mut m, "env");
        assert!(obj.is_empty());
        assert!(matches!(m.get("env"), Some(Value::Object(_))));
    }

    #[test]
    fn reject_non_object_env_bails_on_non_object() {
        let mut m = Map::new();
        m.insert("env".into(), Value::String("oops".into()));
        assert!(reject_non_object_env(&m).is_err());

        m.insert("env".into(), Value::Array(vec![]));
        assert!(reject_non_object_env(&m).is_err());
    }

    #[test]
    fn reject_non_object_env_allows_object_null_and_absent() {
        let mut m = Map::new();
        // Absent `env` - first connect on a fresh settings file.
        assert!(reject_non_object_env(&m).is_ok());
        // `null` carries no data, so replacement is harmless.
        m.insert("env".into(), Value::Null);
        assert!(reject_non_object_env(&m).is_ok());
        // The normal case: an existing object is left for ensure_object.
        m.insert("env".into(), Value::Object(Map::new()));
        assert!(reject_non_object_env(&m).is_ok());
    }
}
