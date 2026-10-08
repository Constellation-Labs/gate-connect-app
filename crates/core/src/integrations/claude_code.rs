//! Claude Code integration.
//!
//! Configures Anthropic's `claude` CLI to route through Constellation Gate
//! via the MITM engine's forward proxy. `~/.claude/settings.json` receives
//! `HTTPS_PROXY=http://gate-claude-code:route@127.0.0.1:<port>`, a loopback
//! `NO_PROXY` and `NODE_EXTRA_CA_CERTS` pointing at our CA, which Claude Code
//! injects into its own process at every invocation.
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
//!
//! We track our own writes via a sibling `_gateConnect` block so
//! disconnect cleanly reverses what connect did and any prior
//! user-set values are restored.
//!
//! **Gate models are the exception to the canonical base URL, by the user's
//! choice.** Putting Claude Code on Gate models writes the models into its own
//! settings - the default `model`, a `modelPicker` that replaces the built-in
//! lineup with the enabled set, and the model tiers Claude Code falls back to on
//! its own (Default, opus/sonnet/haiku/fable, the small-fast model, subagents) -
//! and points `ANTHROPIC_BASE_URL` at the relay's Gate models route, which
//! serves only those models. A non-Anthropic model has no first-party
//! capability path to keep, and the tiers are pinned because the route refuses
//! any model outside the set: a background request on `claude-haiku-*` would
//! fail rather than quietly spend on a model the user never chose. Recorded
//! under `_gateConnect.gateModels` and put back on App default or disconnect,
//! value by value, unless the user has changed it since.
//! Context-window selection also remains Claude Code-owned: Gate Connect never
//! writes ANTHROPIC_BETAS. Standard variants therefore stay at 200K, while
//! Claude Code's [1m] variants add their own 1M beta per selected model.

//! **Config granularity: per process.** Measured 2026-09-18 on Claude Code
//! 2.1.276, driving `claude --bare -p --input-format stream-json` against two
//! loopback listeners and watching which one a turn reached. A fresh
//! invocation picks up an edited `settings.json` immediately; a session already
//! running kept the old address across a second turn. That matches the
//! mechanism - the `env` block becomes process environment variables, which
//! cannot change under a running process - so "restart it" is the right advice
//! here, unlike Codex. See `integrations::codex` for the tool where it is not.

use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::path::PathBuf;

use crate::env;
use crate::integrations::binaries;
use crate::integrations::precedence::Override;
use crate::registry::{ConnectInput, GateModelState, Integration, Mechanism, Status, ToolId};

const UPSTREAM_PROVIDER_NAME: &str = "Anthropic";
const DEFAULT_UPSTREAM_URL: &str = "https://api.anthropic.com";

const KEY_BASE_URL: &str = "ANTHROPIC_BASE_URL";
const KEY_CUSTOM_HEADERS: &str = "ANTHROPIC_CUSTOM_HEADERS";
const KEY_HTTPS_PROXY: &str = "HTTPS_PROXY";
const KEY_NO_PROXY: &str = "NO_PROXY";
/// Node ships its own trust bundle and never reads the OS trust store, so the
/// system-wide anchor install - the thing that makes curl, git and openssl
/// accept our leaves - does nothing for `claude`. Without this variable the
/// proxy written below routes every request into the engine and Node then
/// rejects the leaf with `UNABLE_TO_VERIFY_LEAF_SIGNATURE`, which Claude Code
/// surfaces as "SSL certificate verification failed. Check your proxy or
/// corporate SSL certificates".
///
/// It has to be written *here* and not left to [`super::env_proxy`], which
/// exports the same variable. That one delivers it through the login
/// environment (`environment.d`, `launchctl`, `HKCU\Environment`), a channel
/// with neither the same reach nor the same timing: `settings.json` takes
/// effect on the next `claude` launch, a login environment on the next login,
/// only for sessions descended from it, and only while the user leaves that
/// separate export switched on. Every gap between the two is a `claude` that
/// is proxied with no CA - which is the whole failure, because a tool routed
/// into the engine it cannot verify is worse off than one never routed at all.
///
/// What this write does *not* do is win a fight. Measured against the 2.1.266
/// bundle, Claude Code reads the settings value only as a fallback: it returns
/// early if the variable is already in its environment, and otherwise applies
/// `env.NODE_EXTRA_CA_CERTS` to `process.env` at startup, in time for the lazy
/// trust-store build. So a machine that exports its own CA - a corporate
/// bundle from a shell rc, or the prior value `env_proxy` hands back on
/// disable - keeps that one and still cannot verify our leaf, while
/// [`Integration::status`] reads the file and reports `Connected`. Closing
/// that hole means the environment carrying both certs, which is
/// [`crate::proxy::ca_bundle`]'s job, not this one's.
const KEY_NODE_EXTRA_CA_CERTS: &str = "NODE_EXTRA_CA_CERTS";
const MANAGED_KEYS: [&str; 5] = [
    KEY_BASE_URL,
    KEY_CUSTOM_HEADERS,
    KEY_HTTPS_PROXY,
    KEY_NO_PROXY,
    KEY_NODE_EXTRA_CA_CERTS,
];

/// Keep loopback off the proxy, the same pairing every other proxy-routed
/// integration writes (`hermes`, `env_proxy`, `dotenv`). It matters more here
/// than there: this variable is injected into `claude`'s own process and
/// inherited by everything it spawns - the Bash tool, stdio MCP servers - so
/// without the bypass a local `https://127.0.0.1` MCP server or dev service
/// would be dialled through the engine, and an engine that is down would take
/// every HTTPS request from `claude` and its children with it rather than just
/// the Anthropic ones.
use crate::proxy::NO_PROXY_VALUE;

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

    fn client(&self) -> crate::taxonomy::Client {
        crate::taxonomy::Client::ClaudeCode
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

    fn binary(&self) -> (&'static [&'static str], &'static [&'static str]) {
        #[cfg(windows)]
        const NAMES: &[&str] = &["claude.exe", "claude.cmd", "claude.bat", "claude"];
        #[cfg(not(windows))]
        const NAMES: &[&str] = &["claude"];
        (CLAUDE_BIN_PATHS, NAMES)
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
        // The packaged paths, then PATH, then the bin directories a login
        // shell adds and a GUI process does not inherit - see
        // `integrations::binaries`. The old check was the first of those three
        // alone, which is why a tool installed anywhere else was only found
        // through the config-directory fallback below, and a tool installed but
        // never run was not found at all.
        let (well_known, names) = self.binary();
        if binaries::resolve_binary(well_known, names).is_some() {
            return Ok(true);
        }
        // Fall back to the per-user config dir Claude Code writes on first
        // launch. Catches Volta/asdf/npx installs that don't land a binary
        // in the well-known paths above.
        Ok(env::claude_code_config_dir()?.exists())
    }

    /// `settings.json` names the forwarder's proxy address.
    fn supports_gate_models(&self) -> bool {
        true
    }

    fn gate_model_state(&self) -> Result<GateModelState> {
        Ok(match load_settings()? {
            Some(settings) => gate_model_state_of(&settings).into_state(),
            None => GateModelState::NotApplied,
        })
    }

    /// Settings only: the proxy keys are routing, and stay.
    fn leave_gate_models(&self, _input: &ConnectInput) -> Result<()> {
        let Some(mut settings) = load_settings()? else {
            return Ok(());
        };
        revert_gate_models(&mut settings);
        write_settings(&settings)
    }

    fn mechanism(&self) -> Mechanism {
        Mechanism::ForwardProxy
    }

    fn configured_addresses(&self) -> Result<Vec<String>> {
        Ok(load_settings()?
            .and_then(|s| {
                s.get("env")?
                    .as_object()?
                    .get(KEY_HTTPS_PROXY)?
                    .as_str()
                    .map(str::to_owned)
            })
            .into_iter()
            .collect())
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
        let on_gate_models = gate_set().is_some()
            && env_block
                .get(KEY_BASE_URL)
                .and_then(|v| v.as_str())
                .is_some_and(|u| {
                    crate::proxy::gate_served::is_relay_base_url(u, ToolId::ClaudeCode)
                });
        if env_block.contains_key(KEY_BASE_URL) && !on_gate_models {
            return Ok(Status::Drifted(format!(
                "managed {KEY_BASE_URL} must be absent so Claude Code keeps first-party model capabilities"
            )));
        }

        // Every address that is ours counts, not only the one `connect` writes
        // today. `proxy::tool_proxy_identity_urls` says why there is more than
        // one: an install configured before tool configs moved to the forwarder
        // holds the engine's address, and that config is correct rather than
        // drifted. Reporting it otherwise would draw a repair over a working
        // file and send the reconcile pass to rewrite one that is already right.
        let ours: Vec<String> = crate::proxy::tool_proxy_identity_urls()
            .iter()
            .map(|url| crate::proxy::claude_code_proxy_url(url))
            .collect::<Result<_>>()?;
        // The liveness half is still `engine_proxy_url`, which is `None` while
        // nothing is routing. An empty list means no port has ever been bound,
        // which reads the same to the user and is folded in here rather than
        // given a second sentence.
        let Some(expected_proxy) = ours.first() else {
            return Ok(Status::Drifted(
                "the Gate proxy has not been enabled yet - turn it on to route Claude Code".into(),
            ));
        };

        let configured = match env_block.get(KEY_HTTPS_PROXY).and_then(|v| v.as_str()) {
            Some(proxy) if ours.iter().any(|ours| ours == proxy) => proxy,
            Some(proxy) => {
                return Ok(Status::Drifted(format!(
                    "{KEY_HTTPS_PROXY} in settings.json is {proxy:?}, expected {expected_proxy:?}"
                )));
            }
            None => {
                return Ok(Status::Drifted(format!(
                    "managed {KEY_HTTPS_PROXY} missing from settings.json env"
                )));
            }
        };

        // The address in the file, not the engine. This used to gate on
        // `engine_proxy_url().is_some()`, which stopped being the same question
        // when tool configs moved to the forwarder: a dead forwarder under a
        // live engine read as Connected over a tool whose every request failed
        // to connect. It also never had the parked wording its two siblings
        // got, so routing-off reported the sentence meant for a first run.
        let health = crate::proxy::address_health(configured);
        if let Some(reason) = proxy_health_drift(configured, health) {
            return Ok(Status::Drifted(reason));
        }

        // Checked after the proxy and never folded into it: the pair fails
        // asymmetrically. A missing proxy leaves Claude Code unrouted, which is
        // visible as traffic that never reaches Gate; a missing CA leaves it
        // routed into an engine whose leaf it cannot verify, which is visible
        // only as a TLS error the user has no reason to attribute to us. This
        // reading as drift rather than `Connected` is also what lets
        // `provider::reconcile_enabled` repair a settings.json written before
        // the variable was managed - those files carry the proxy and no CA, and
        // reported `Connected` all the way through the failure.
        let expected_ca = crate::proxy::ca_cert_path()?.display().to_string();
        match env_block
            .get(KEY_NODE_EXTRA_CA_CERTS)
            .and_then(|v| v.as_str())
        {
            Some(ca) if ca == expected_ca => {}
            Some(ca) => {
                return Ok(Status::Drifted(format!(
                    "{KEY_NODE_EXTRA_CA_CERTS} in settings.json is {ca:?}, expected {expected_ca:?}"
                )));
            }
            None => {
                return Ok(Status::Drifted(format!(
                    "managed {KEY_NODE_EXTRA_CA_CERTS} missing from settings.json env - Claude \
                     Code would route through the proxy without trusting Gate's CA"
                )));
            }
        }

        // Everything we write is on disk and correct. Whether Claude Code uses
        // it is a different question, and the last one asked (AG-674).
        Ok(match managed_settings_override(expected_proxy)? {
            Some(o) => o.into_status(),
            None => Status::Connected,
        })
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

        // A live engine has minted the CA, so this is a should-not-happen
        // state (a cleared app-support dir under a still-running engine). It
        // is worth a refusal rather than a warning because writing the proxy
        // anyway produces exactly the failure this key exists to prevent, and
        // a path Claude Code cannot read is one it reports as an SSL error
        // with no mention of Gate. Refusing leaves the tool unrouted, which is
        // the recoverable half of that pair.
        let ca_cert_path = crate::proxy::ca_cert_path()?;
        if !ca_cert_path.exists() {
            anyhow::bail!(
                "Gate's CA certificate is missing at {} - restart the proxy so the engine mints \
                 it, then connect Claude Code again",
                ca_cert_path.display()
            );
        }

        let mut settings = load_settings()?.unwrap_or_default();
        // Refuse to clobber a malformed non-object `env` before ensure_object
        // would silently replace it with `{}` (see reject_non_object_env).
        reject_non_object_env(&settings)?;

        // Drift is read off the settings AS LOADED, before the routing block
        // below touches them: that block removes `ANTHROPIC_BASE_URL` on every
        // connect (the canonical-URL rule), and a check made after it would
        // read every reconnect of a Claude Code on Gate models as the user
        // having moved off them.
        if gate_model_state_of(&settings).is_drifted() {
            crate::preferences::fall_back_to_tool_model(ToolId::ClaudeCode.slug())?;
        }

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

        // Already connected with these exact values: leave the file alone.
        // `write_object` and `config_changes` would already record nothing for
        // an unchanged file, so this only spares the rewrite of a file Claude
        // Code owns. Only the keys we write are compared; the rest of the file
        // belongs to Claude Code and the user.
        let env_value = |key: &str| settings.get("env").and_then(|env| env.get(key));
        let env_str = |key: &str| env_value(key).and_then(|v| v.as_str());
        let ca_cert_value = ca_cert_path.display().to_string();
        let already_applied = MANAGED_KEYS
            .iter()
            .all(|key| old_managed.iter().any(|managed| managed == key))
            && env_value(KEY_BASE_URL).is_none()
            && env_value(KEY_CUSTOM_HEADERS).is_none()
            && env_str(KEY_HTTPS_PROXY) == Some(claude_proxy_url.as_str())
            && env_str(KEY_NO_PROXY) == Some(NO_PROXY_VALUE)
            && env_str(KEY_NODE_EXTRA_CA_CERTS) == Some(ca_cert_value.as_str());
        // Gate models add keys of their own, so "already applied" also needs
        // there to be none wanted and none left to take out.
        let gate_models_idle = gate_set().is_none()
            && settings
                .get(MARKER_KEY)
                .and_then(|m| m.get(GATE_MODELS_MARKER))
                .is_none();
        if already_applied && gate_models_idle {
            return Ok(());
        }

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
        env_block.insert(
            KEY_NODE_EXTRA_CA_CERTS.into(),
            Value::String(ca_cert_path.display().to_string()),
        );

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

        // Gate models last, over the routing keys above: the base URL they
        // write is the one exception to the canonical-URL rule, and it is only
        // there while the user has Claude Code on Gate models. (Drift was
        // settled against the loaded file at the top.)
        match gate_set() {
            Some(ids) => {
                let relay = input.relay_base_url.as_deref().context(
                    "the Gate relay is not running - Claude Code reaches Gate models through it",
                )?;
                apply_gate_models(&mut settings, &ids, relay);
            }
            None => revert_gate_models(&mut settings),
        }

        write_settings(&settings)
    }

    fn disconnect(&self) -> Result<()> {
        let path = settings_path()?;
        let Some(mut settings) = load_settings()? else {
            return Ok(());
        };
        // Gate models first: their record lives in the marker removed below.
        revert_gate_models(&mut settings);

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
            crate::config_changes::remove(&path)?;
            return Ok(());
        }
        write_settings(&settings)
    }
}

/// `_gateConnect.gateModels`: what Gate models wrote and what they replaced.
const GATE_MODELS_MARKER: &str = "gateModels";

/// The `env` keys Gate models set, every one to an enabled id (or a switch
/// that keeps Claude Code on one). Measured on Claude Code 2.1.285: these are
/// every place it picks a model without being asked - the Default row, the
/// tier aliases, the small-fast model its background helpers use, subagents
/// (forced, so an agent that names its own model inherits instead), and the
/// fallback chain.
const GATE_ENV_KEYS: &[&str] = &[
    "ANTHROPIC_DEFAULT_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL_FORCE",
    "CLAUDE_CODE_NO_MODEL_FALLBACK",
];

fn gate_set() -> Option<Vec<String>> {
    crate::preferences::gate_models_for(ToolId::ClaudeCode.slug())
}

/// Where one recorded value lives: `env.<key>` or a top-level key.
fn slot<'a>(settings: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    match key.strip_prefix("env.") {
        Some(k) => settings.get("env")?.as_object()?.get(k),
        None => settings.get(key),
    }
}

fn set_slot(settings: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    match key.strip_prefix("env.") {
        Some(k) => {
            let env = ensure_object(settings, "env");
            match value {
                Some(v) => {
                    env.insert(k.into(), v);
                }
                None => {
                    env.remove(k);
                }
            }
            if env.is_empty() {
                settings.remove("env");
            }
        }
        None => match value {
            Some(v) => {
                settings.insert(key.into(), v);
            }
            None => {
                settings.remove(key);
            }
        },
    }
}

/// The values Gate models write for `ids`, keyed as [`slot`] reads them.
fn gate_values(ids: &[String], default: &str, relay: &str) -> Vec<(String, Value)> {
    let mut out = vec![(
        format!("env.{KEY_BASE_URL}"),
        Value::String(crate::proxy::gate_served::relay_root_url(
            relay,
            ToolId::ClaudeCode,
        )),
    )];
    for key in GATE_ENV_KEYS {
        let v = match *key {
            "CLAUDE_CODE_SUBAGENT_MODEL" => "inherit".to_string(),
            "CLAUDE_CODE_SUBAGENT_MODEL_FORCE" | "CLAUDE_CODE_NO_MODEL_FALLBACK" => "1".to_string(),
            _ => default.to_string(),
        };
        out.push((format!("env.{key}"), Value::String(v)));
    }
    // The top-level `model`, not `env.ANTHROPIC_MODEL`: the env value would
    // override the user's own `/model` pick on every launch.
    out.push(("model".into(), Value::String(default.into())));
    let options: Vec<Value> = ids
        .iter()
        .map(|id| {
            let label = crate::preferences::gate_model_meta(id)
                .and_then(|m| m.name)
                .unwrap_or_else(|| id.clone());
            serde_json::json!({
                "model": id,
                "label": label,
                "description": "Gate model, billed to your organization's Gate credits",
            })
        })
        .collect();
    out.push((
        "modelPicker".into(),
        serde_json::json!({ "replaceBuiltInOptions": true, "options": options }),
    ));
    out
}

/// Slots that name a model, as opposed to a switch or the route.
fn is_model_slot(key: &str) -> bool {
    key == "model"
        || key
            .strip_prefix("env.")
            .is_some_and(|k| k.ends_with("_MODEL") && k != "CLAUDE_CODE_SUBAGENT_MODEL")
}

/// What the settings say about Gate models.
enum Reading {
    NotApplied,
    Applied(String),
    Drifted(Option<String>),
}

impl Reading {
    fn is_drifted(&self) -> bool {
        matches!(self, Reading::Drifted(_))
    }

    fn into_state(self) -> GateModelState {
        match self {
            Reading::NotApplied => GateModelState::NotApplied,
            Reading::Applied(model) => GateModelState::Applied { model },
            Reading::Drifted(model) => GateModelState::Drifted { model },
        }
    }
}

fn written_ids(settings: &Map<String, Value>) -> Option<Vec<String>> {
    let list = settings
        .get(MARKER_KEY)?
        .get(GATE_MODELS_MARKER)?
        .get("ids")?
        .as_array()?;
    Some(
        list.iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
    )
}

/// The Gate model Claude Code is running on, while it is connected on Gate
/// models; `None` otherwise, including when `settings.json` cannot be read.
///
/// For the engine, which moves the desktop Code tab onto Gate models only while
/// this answers (`code_tab_gate_models` in `proxy::engine`) and asks once per
/// request. So the reading is cached on the file's modified time and length,
/// the way `preferences` caches its own file and for the same reason: a stat is
/// cheap and cannot go stale, and on Linux the engine runs in a daemon that no
/// write from the GUI or the CLI would otherwise refresh.
pub fn applied_gate_model() -> Option<String> {
    type Stamp = Option<(std::time::SystemTime, u64)>;
    static CACHE: std::sync::RwLock<Option<(Stamp, Option<String>)>> = std::sync::RwLock::new(None);
    let path = settings_path().ok()?;
    let stamp: Stamp = std::fs::metadata(&path)
        .ok()
        .and_then(|m| Some((m.modified().ok()?, m.len())));
    if let Some((cached, model)) = CACHE.read().ok()?.as_ref() {
        if *cached == stamp {
            return model.clone();
        }
    }
    let model =
        load_settings()
            .ok()
            .flatten()
            .and_then(|settings| match gate_model_state_of(&settings) {
                Reading::Applied(model) => Some(model),
                Reading::NotApplied | Reading::Drifted(_) => None,
            });
    if let Ok(mut cache) = CACHE.write() {
        *cache = Some((stamp, model.clone()));
    }
    model
}

/// Applied while Claude Code still sends to our route and starts on one of the
/// set. An absent `model` is the picker's Default row, which resolves to the
/// `ANTHROPIC_DEFAULT_MODEL` Gate pinned, so it counts.
fn gate_model_state_of(settings: &Map<String, Value>) -> Reading {
    let Some(ids) = written_ids(settings) else {
        return Reading::NotApplied;
    };
    let on_route = slot(settings, &format!("env.{KEY_BASE_URL}"))
        .and_then(|v| v.as_str())
        .is_some_and(|u| crate::proxy::gate_served::is_relay_base_url(u, ToolId::ClaudeCode));
    let model = slot(settings, "model")
        .and_then(|v| v.as_str())
        .or_else(|| slot(settings, "env.ANTHROPIC_DEFAULT_MODEL").and_then(|v| v.as_str()))
        .map(str::to_owned);
    match model {
        Some(m) if on_route && ids.contains(&m) => Reading::Applied(m),
        model => Reading::Drifted(model),
    }
}

/// Write the Gate models, snapshotting Claude Code's own values the first time.
/// The default is the first of the set, unless Claude Code is already on one of
/// an unchanged set: then the user picked it in `/model`.
fn apply_gate_models(settings: &mut Map<String, Value>, ids: &[String], relay: &str) {
    let Some(first) = ids.first() else { return };
    let written = written_ids(settings);
    let current = slot(settings, "model")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let default = match (&written, &current) {
        (Some(w), Some(c)) if w.as_slice() == ids && ids.contains(c) => c.clone(),
        _ => first.clone(),
    };
    let values = gate_values(ids, &default, relay);

    let previous = settings
        .get(MARKER_KEY)
        .and_then(|m| m.get(GATE_MODELS_MARKER))
        .and_then(|g| g.get("previous"))
        .cloned()
        .unwrap_or_else(|| {
            // First apply: what each slot held, `null` for absent. The base URL
            // is routing's to restore - its own snapshot already has it - so it
            // is not recorded twice.
            let mut prev = Map::new();
            for (key, _) in values.iter().skip(1) {
                prev.insert(
                    key.clone(),
                    slot(settings, key).cloned().unwrap_or(Value::Null),
                );
            }
            Value::Object(prev)
        });
    let mut wrote = Map::new();
    for (key, value) in &values {
        set_slot(settings, key, Some(value.clone()));
        wrote.insert(key.clone(), value.clone());
    }
    let marker = ensure_object(settings, MARKER_KEY);
    marker.insert(
        GATE_MODELS_MARKER.into(),
        serde_json::json!({ "ids": ids, "previous": previous, "wrote": wrote }),
    );
}

/// Put Claude Code's own values back, each only while it still holds what Gate
/// wrote. The base URL is removed while it is still our route: the canonical
/// URL is what routing wants, and a user value was routing's to snapshot.
fn revert_gate_models(settings: &mut Map<String, Value>) {
    let Some(record) = settings
        .get(MARKER_KEY)
        .and_then(|m| m.get(GATE_MODELS_MARKER))
        .cloned()
    else {
        return;
    };
    let previous = record
        .get("previous")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let wrote = record
        .get("wrote")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let ids: Vec<String> = record
        .get("ids")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    for (key, ours) in &wrote {
        // A model slot holding ANY id of the set is still ours: the user
        // picking another enabled model in `/model` moves `model` off the value
        // Gate wrote without making it theirs. Left behind, that Gate id would
        // be sent to api.anthropic.com once the route below is gone, and every
        // request would fail. Codex and Hermes read their model slot the same
        // way.
        let current = slot(settings, key);
        let in_set = is_model_slot(key)
            && current
                .and_then(|v| v.as_str())
                .is_some_and(|m| ids.iter().any(|id| id == m));
        if current != Some(ours) && !in_set {
            continue;
        }
        if key == &format!("env.{KEY_BASE_URL}") {
            set_slot(settings, key, None);
            continue;
        }
        let back = previous.get(key).cloned().filter(|v| !v.is_null());
        set_slot(settings, key, back);
    }
    if let Some(marker) = settings.get_mut(MARKER_KEY).and_then(|v| v.as_object_mut()) {
        marker.remove(GATE_MODELS_MARKER);
    }
}

/// Why the proxy address in `settings.json` is not usable, or `None` when it is.
///
/// Pure so it can be tested without sockets; the probe is the caller's. See
/// `proxy::AddressHealth` for why "the engine is routing" is not the question.
fn proxy_health_drift(configured: &str, health: crate::proxy::AddressHealth) -> Option<String> {
    match health {
        crate::proxy::AddressHealth::Routing => None,
        crate::proxy::AddressHealth::Parked => Some(format!(
            "routing is off, so Claude Code reaches Anthropic directly rather than through Gate \
             ({configured:?} is forwarding straight through) - turn routing on to route it"
        )),
        crate::proxy::AddressHealth::Dead => Some(format!(
            "nothing is listening at {configured:?}, so Claude Code cannot reach Anthropic - \
             turn routing on, or disconnect Claude Code to put its own settings back"
        )),
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
        // Nor without the CA. Node ignores the OS trust store, so writing the
        // proxy alone routes `claude` straight into a TLS failure.
        assert!(MANAGED_KEYS.contains(&KEY_NODE_EXTRA_CA_CERTS));
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

#[cfg(test)]
mod proxy_health_tests {
    use super::*;

    /// Only a routing address is usable. The dead case is the regression this
    /// round fixes: the file names the forwarder, so a live engine does not
    /// make it reachable.
    #[test]
    fn only_a_routing_address_is_usable() {
        let addr = "http://gate-claude-code:route@127.0.0.1:47150";
        assert_eq!(
            proxy_health_drift(addr, crate::proxy::AddressHealth::Routing),
            None
        );
        let parked =
            proxy_health_drift(addr, crate::proxy::AddressHealth::Parked).expect("parked drifts");
        assert!(parked.contains("routing is off"), "unexpected: {parked}");
        let dead =
            proxy_health_drift(addr, crate::proxy::AddressHealth::Dead).expect("dead drifts");
        assert!(dead.contains("nothing is listening"), "unexpected: {dead}");
        assert!(dead.contains(addr), "must name the address: {dead}");
    }
}
