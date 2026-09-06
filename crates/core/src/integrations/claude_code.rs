//! Claude Code integration.
//!
//! Configures Anthropic's `claude` CLI to route through Constellation Gate
//! via the MITM engine's forward proxy. `~/.claude/settings.json` receives
//! `HTTPS_PROXY=http://gate-claude-code:route@127.0.0.1:<port>` plus a loopback
//! `NO_PROXY`, which Claude Code injects into its own process at every
//! invocation.
//!
//! Keeping `ANTHROPIC_BASE_URL` unset is essential. Claude Code treats a
//! custom base URL as non-first-party for capability checks performed before
//! any request reaches Gate, including its context window and auto-compaction
//! threshold. With the canonical Anthropic URL intact, model selection behaves
//! exactly as it does direct: standard variants remain 200K and explicit
//! `[1m]` variants enable 1M. The forward proxy routes the socket without
//! changing that capability classification.
//!
//! Connect removes values written by the older reverse-relay scheme
//! (`ANTHROPIC_BASE_URL` and `ANTHROPIC_CUSTOM_HEADERS`) and restores every
//! user-owned value on disconnect. No credential or Anthropic beta is written.
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
//! Context-window selection also remains Claude Code-owned: Gate Connect never
//! writes ANTHROPIC_BETAS. Standard variants therefore stay at 200K, while
//! Claude Code's [1m] variants add their own 1M beta per selected model.
//!
//! [`requires_upstream_credential`]: crate::Integration::requires_upstream_credential

use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::env;
use crate::integrations::precedence::Override;
use crate::registry::{ConnectInput, Integration, Status, ToolId};

const UPSTREAM_PROVIDER_NAME: &str = "Anthropic";
const DEFAULT_UPSTREAM_URL: &str = "https://api.anthropic.com";

const KEY_BASE_URL: &str = "ANTHROPIC_BASE_URL";
const KEY_CUSTOM_HEADERS: &str = "ANTHROPIC_CUSTOM_HEADERS";
const KEY_HTTPS_PROXY: &str = "HTTPS_PROXY";
const KEY_NO_PROXY: &str = "NO_PROXY";
const MANAGED_KEYS: [&str; 4] = [
    KEY_BASE_URL,
    KEY_CUSTOM_HEADERS,
    KEY_HTTPS_PROXY,
    KEY_NO_PROXY,
];

/// Keep loopback off the proxy, the same pairing every other proxy-routed
/// integration writes (`hermes`, `env_proxy`, `dotenv`). It matters more here
/// than there: this variable is injected into `claude`'s own process and
/// inherited by everything it spawns - the Bash tool, stdio MCP servers - so
/// without the bypass a local `https://127.0.0.1` MCP server or dev service
/// would be dialled through the engine, and an engine that is down would take
/// every HTTPS request from `claude` and its children with it rather than just
/// the Anthropic ones.
const NO_PROXY_VALUE: &str = "localhost,127.0.0.1,::1";

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

    /// The row label. Rows sit under a family heading that already names the
    /// vendor, so the label separates the surfaces inside that family: "App" for
    /// the desktop apps, "Web" for the browser tab, "CLI" for the terminal. The
    /// sentence that says which binary this is lives with the UI copy
    /// (`src/lib/groups.ts`); it is a description of the surface, not something
    /// the integration knows.
    fn row_label(&self) -> &'static str {
        "CLI"
    }

    fn upstream_provider_name(&self) -> &'static str {
        UPSTREAM_PROVIDER_NAME
    }

    fn default_upstream_url(&self) -> &'static str {
        DEFAULT_UPSTREAM_URL
    }

    fn config_location(&self) -> Option<String> {
        settings_path().ok().map(|p| p.display().to_string())
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        // Exactly what `detect` and `status` read: the well-known binaries, the
        // config directory whose existence stands in for a Volta/asdf/npx
        // install, and the settings file inside it that decides Connected from
        // Drifted.
        let mut paths: Vec<PathBuf> = CLAUDE_BIN_PATHS.iter().map(PathBuf::from).collect();
        paths.extend(env::claude_code_config_dir());
        paths.extend(settings_path());
        paths
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
        let Some(managed) = marker.and_then(|v| v.as_array()) else {
            return Ok(Status::Drifted(
                "Gate's Claude Code management marker is malformed".into(),
            ));
        };

        if !managed.iter().any(|v| v.as_str() == Some(KEY_HTTPS_PROXY)) {
            return Ok(Status::Drifted(
                "Claude Code still uses Gate's legacy custom-base-URL routing".into(),
            ));
        }
        if env_block.contains_key(KEY_BASE_URL) {
            return Ok(Status::Drifted(format!(
                "managed {KEY_BASE_URL} must be absent so Claude Code keeps first-party model capabilities"
            )));
        }

        let expected_proxy = match crate::proxy::engine_proxy_url() {
            Some(proxy) => crate::proxy::claude_code_proxy_url(&proxy)?,
            None => {
                return Ok(Status::Drifted(
                    "the Gate proxy has not been enabled yet - turn it on to route Claude Code"
                        .into(),
                ));
            }
        };

        match env_block.get(KEY_HTTPS_PROXY).and_then(|v| v.as_str()) {
            Some(proxy) if proxy == expected_proxy => {
                // Our value is on disk and correct. Whether Claude Code uses it
                // is a different question, and the last one asked (AG-674).
                Ok(match managed_settings_override(&expected_proxy)? {
                    Some(o) => o.into_status(),
                    None => Status::Connected,
                })
            }
            Some(proxy) => Ok(Status::Drifted(format!(
                "{KEY_HTTPS_PROXY} in settings.json is {proxy:?}, expected {expected_proxy:?}"
            ))),
            None => Ok(Status::Drifted(format!(
                "managed {KEY_HTTPS_PROXY} missing from settings.json env"
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
        let engine_proxy_url = input.engine_proxy_url.as_deref().context(
            "the Gate proxy engine is not running - enable the proxy before connecting Claude Code",
        )?;
        let claude_proxy_url = crate::proxy::claude_code_proxy_url(engine_proxy_url)?;
        if !input.upstream_url.starts_with("https://") {
            anyhow::bail!("upstream URL must be https://");
        }
        // Unlike a config-editing integration, this one cannot be retargeted:
        // what makes it work is that the destination stays canonical, and the
        // route the engine forces for our selector is Anthropic's entry alone
        // (`proxy::claude_code_route_domain`). So a different `--upstream-url`
        // has nowhere to go, and accepting it would write a Claude Code that
        // routes Anthropic traffic anyway - a silent no-op. Refuse instead.
        let endpoint = crate::proxy::resolve_endpoint(&input.upstream_url)
            .with_context(|| format!("Gate has no upstream domain for {:?}", input.upstream_url))?;
        let route = crate::proxy::claude_code_route_domain();
        if endpoint.slug != route.slug {
            anyhow::bail!(
                "Claude Code can only route to {DEFAULT_UPSTREAM_URL}, not {:?} - it reaches Gate \
                 through the local forward proxy, which keeps Anthropic's address canonical so \
                 Claude Code keeps its first-party model capabilities",
                input.upstream_url
            );
        }

        let mut settings = load_settings()?.unwrap_or_default();
        // Refuse to clobber a malformed non-object `env` before ensure_object
        // would silently replace it with `{}` (see reject_non_object_env).
        reject_non_object_env(&settings)?;

        // Preserve the original values across reconnects and migrations. A key
        // already listed as managed is ours; a newly managed key still belongs
        // to the user and must be snapshotted before we replace it.
        let old_managed: Vec<String> = settings
            .get(MARKER_KEY)
            .and_then(|v| v.get("managed"))
            .and_then(|v| v.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let mut prev = settings
            .get(MARKER_KEY)
            .and_then(|v| v.get("previousEnv"))
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();

        let env_block = ensure_object(&mut settings, "env");
        for key in MANAGED_KEYS {
            if !old_managed.iter().any(|managed| managed == key) && !prev.contains_key(key) {
                if let Some(value) = env_block.get(key) {
                    prev.insert(key.into(), value.clone());
                }
            }
        }

        // The canonical Anthropic base URL is what keeps Claude Code on its
        // first-party capability path. Only the transport is redirected.
        env_block.remove(KEY_BASE_URL);
        env_block.remove(KEY_CUSTOM_HEADERS);
        env_block.insert(KEY_HTTPS_PROXY.into(), Value::String(claude_proxy_url));
        env_block.insert(KEY_NO_PROXY.into(), Value::String(NO_PROXY_VALUE.into()));

        let marker = ensure_object(&mut settings, MARKER_KEY);
        marker.insert("previousEnv".into(), Value::Object(prev));
        marker.insert(
            "managed".into(),
            Value::Array(
                MANAGED_KEYS
                    .iter()
                    .map(|key| Value::String((*key).into()))
                    .collect(),
            ),
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
            for key in MANAGED_KEYS {
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

/// The enterprise managed settings, when they decide the route instead of us.
///
/// Claude Code merges five layers, and `~/.claude/settings.json` - the only one
/// Gate writes - is the *bottom* of them. Four sit above it: the project's
/// `.claude/settings.json`, its `.claude/settings.local.json`, the command
/// line, and, above everything including the flags, the enterprise managed
/// settings this reads.
///
/// **Only that top layer is visible from here**, and the reason is worth
/// stating because it is the shape of the whole story: the three layers in
/// between are chosen by the directory `claude` was started in, and this
/// process does not know that directory. A repo-local `settings.local.json`
/// that sets its own `HTTPS_PROXY` still reads as connected, and the honest
/// place for that is [`crate::integrations::precedence`]'s note rather than a
/// check here pretending to cover it.
///
/// Two keys displace us, for different reasons. `HTTPS_PROXY` is the socket:
/// a different value there and the traffic never reaches our engine at all.
/// `ANTHROPIC_BASE_URL` is subtler and is why it counts even though we route by
/// proxy - the engine's route selector is scoped to Anthropic's canonical
/// address (see the module header), so a base URL pointing somewhere else is
/// decided by the catalog alone, which is not a route this integration can
/// claim.
fn managed_settings_override(expected_proxy: &str) -> Result<Option<Override>> {
    let path = env::claude_code_managed_settings_path();
    // A parse failure here must not fail `status`. This file belongs to an
    // administrator, we never write it, and an unreadable one is not evidence
    // that anything overrides us - reporting the tool as unreadable off
    // somebody else's malformed JSON would be a worse answer than the one we
    // already have.
    let Some(settings) = super::json_config::load_object(&path).ok().flatten() else {
        return Ok(None);
    };
    Ok(override_in(
        &settings,
        &path.display().to_string(),
        expected_proxy,
    ))
}

/// The reading itself, split from the file it comes from so it can be tested:
/// the real path is machine-wide (`/etc/claude-code`, `/Library/Application
/// Support`) and no test may write there.
fn override_in(
    settings: &Map<String, Value>,
    source: &str,
    expected_proxy: &str,
) -> Option<Override> {
    let env_block = settings.get("env").and_then(|v| v.as_object())?;
    if let Some(proxy) = env_block.get(KEY_HTTPS_PROXY).and_then(|v| v.as_str()) {
        if proxy != expected_proxy {
            return Some(Override::new(
                source,
                format!(
                    "sets {KEY_HTTPS_PROXY} to {proxy:?}, which Claude Code loads over the \
                     {expected_proxy:?} in settings.json"
                ),
            ));
        }
    }
    if let Some(base) = env_block.get(KEY_BASE_URL).and_then(|v| v.as_str()) {
        return Some(Override::new(
            source,
            format!(
                "sets {KEY_BASE_URL} to {base:?}, so Claude Code addresses that host instead of \
                 the canonical Anthropic one Gate's proxy route is scoped to"
            ),
        ));
    }
    None
}

fn load_settings() -> Result<Option<Map<String, Value>>> {
    super::json_config::load_object(&settings_path()?)
}

fn write_settings(settings: &Map<String, Value>) -> Result<()> {
    super::json_config::write_object(&settings_path()?, settings)
}

use super::json_config::ensure_object;

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
    fn managed_keys_keep_the_anthropic_base_url_canonical() {
        assert!(MANAGED_KEYS.contains(&KEY_BASE_URL));
        assert!(MANAGED_KEYS.contains(&KEY_HTTPS_PROXY));
        // The proxy variable never travels without its loopback bypass.
        assert!(MANAGED_KEYS.contains(&KEY_NO_PROXY));
        assert!(!MANAGED_KEYS.contains(&"ANTHROPIC_BETAS"));
    }

    /// AG-674's disagreement case for this integration: our `HTTPS_PROXY` is in
    /// `settings.json` and correct, and the managed layer above it points the
    /// CLI at a different proxy. The old reading - ours is on disk, so we are
    /// connected - is the one this test exists to prevent.
    #[test]
    fn a_managed_proxy_beats_the_one_we_wrote() {
        let managed: Map<String, Value> =
            serde_json::from_str(r#"{"env": {"HTTPS_PROXY": "http://corp-egress.example:3128"}}"#)
                .unwrap();
        let o = override_in(
            &managed,
            "/etc/claude-code/managed-settings.json",
            "http://127.0.0.1:1234",
        )
        .expect("a different managed proxy is an override");
        // The path is half the answer: a status line that says the traffic is
        // not ours has to say where to go and look.
        assert!(o.source.contains("managed-settings.json"));
        assert!(o.to_string().contains("corp-egress.example:3128"));
    }

    /// The same value is not a disagreement. An administrator who exports Gate's
    /// own proxy machine-wide has not taken the route away from us, and saying
    /// so would send the user hunting for a conflict that does not exist.
    #[test]
    fn a_managed_layer_repeating_our_proxy_is_not_an_override() {
        let managed: Map<String, Value> =
            serde_json::from_str(r#"{"env": {"HTTPS_PROXY": "http://127.0.0.1:1234"}}"#).unwrap();
        assert_eq!(
            override_in(
                &managed,
                "/etc/claude-code/managed-settings.json",
                "http://127.0.0.1:1234"
            ),
            None
        );
    }

    /// A base URL displaces us even though we route by proxy: the engine's route
    /// selector is scoped to Anthropic's canonical address, so traffic addressed
    /// elsewhere is not on the route this integration configures.
    #[test]
    fn a_managed_base_url_is_an_override_even_with_our_proxy_intact() {
        let managed: Map<String, Value> = serde_json::from_str(
            r#"{"env": {"HTTPS_PROXY": "http://127.0.0.1:1234",
                        "ANTHROPIC_BASE_URL": "https://gateway.example/anthropic"}}"#,
        )
        .unwrap();
        let o = override_in(
            &managed,
            "/etc/claude-code/managed-settings.json",
            "http://127.0.0.1:1234",
        )
        .expect("a managed base URL is an override");
        assert!(o.to_string().contains("gateway.example"));
    }

    /// Nothing above us, nothing to say. Includes the file existing but carrying
    /// unrelated policy, which is the common case on a managed machine.
    #[test]
    fn managed_settings_without_routing_keys_say_nothing() {
        let managed: Map<String, Value> =
            serde_json::from_str(r#"{"permissions": {"defaultMode": "acceptEdits"}}"#).unwrap();
        assert_eq!(
            override_in(
                &managed,
                "/etc/claude-code/managed-settings.json",
                "http://127.0.0.1:1234"
            ),
            None
        );
        assert_eq!(
            override_in(
                &Map::new(),
                "/etc/claude-code/managed-settings.json",
                "http://127.0.0.1:1234"
            ),
            None
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
