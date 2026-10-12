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
use std::path::{Path, PathBuf};

use crate::env;
use crate::registry::{ConnectInput, Integration, Mechanism, Status, ToolId};

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
/// The org's compaction window, as Claude Code's own knob: see
/// [`super::compaction`] for the mapping and the measurements behind it. The
/// one managed key that is a preference rather than a routing requirement,
/// which gives it two rules of its own:
///
/// - Written only while the selected org sets a window, and removed again
///   (on the next connect, by `provider::reconcile_enabled` once `status`
///   reads the stale value as drift) when the org stops setting one. The
///   proxy keys are always written; this one tracks a value.
/// - A value the user set themselves is never touched. The proxy keys
///   snapshot a prior user value into `previousEnv` and replace it, because
///   routing does not work without them; a compaction window the user chose
///   by hand is a choice to keep, so a key present in `env` but absent from
///   the marker's `managed` list is theirs - connect leaves it, status does
///   not count it as drift, and disconnect does not remove it. Ownership is
///   the marker's `managed` list, which is the ownership rule the whole file
///   already runs on.
const KEY_AUTO_COMPACT_WINDOW: &str = "CLAUDE_CODE_AUTO_COMPACT_WINDOW";
/// Every key connect may write and disconnect reverts. The membership of the
/// marker's `managed` list per file is [`managed_keys_for`]: this is the
/// universe, that is the subset written into one file.
const MANAGED_KEYS: [&str; 6] = [
    KEY_BASE_URL,
    KEY_CUSTOM_HEADERS,
    KEY_HTTPS_PROXY,
    KEY_NO_PROXY,
    KEY_NODE_EXTRA_CA_CERTS,
    KEY_AUTO_COMPACT_WINDOW,
];

/// Which keys a connect with `window` writes and lists as managed. The five
/// routing keys always; the compaction window only while there is a value to
/// carry, and never while the user owns the key (see [`KEY_AUTO_COMPACT_WINDOW`]).
fn managed_keys_for(window: Option<&str>, window_is_ours: bool) -> Vec<&'static str> {
    MANAGED_KEYS
        .iter()
        .copied()
        .filter(|key| *key != KEY_AUTO_COMPACT_WINDOW || (window.is_some() && window_is_ours))
        .collect()
}

/// Is the compaction window key in this file ours to write? Ours when the
/// marker already lists it, or when nothing holds it; a value present in
/// `env` that we never listed was put there by the user.
fn window_is_ours(managed: &[String], env_block: Option<&Map<String, Value>>) -> bool {
    managed.iter().any(|key| key == KEY_AUTO_COMPACT_WINDOW)
        || !env_block.is_some_and(|env| env.contains_key(KEY_AUTO_COMPACT_WINDOW))
}

/// Why the compaction window in `settings.json` does not match the org's, or
/// `None` when it does, or when the key is the user's and not ours to judge.
///
/// `managed` is whether the marker lists the key, `in_file` what `env` holds,
/// `expected` what the org asks for now. Pure, so the six cases can be pinned
/// without a file.
fn window_drift(managed: bool, in_file: Option<&str>, expected: Option<&str>) -> Option<String> {
    match (expected, managed, in_file) {
        // Ours, and current.
        (Some(want), true, Some(have)) if have == want => None,
        (Some(want), true, Some(have)) => Some(format!(
            "{KEY_AUTO_COMPACT_WINDOW} in settings.json is {have:?}, expected {want:?}"
        )),
        (Some(want), true, None) => Some(format!(
            "managed {KEY_AUTO_COMPACT_WINDOW} missing from settings.json env, expected {want:?}"
        )),
        // The user's own value: not ours to judge or to replace.
        (Some(_), false, Some(_)) => None,
        (Some(want), false, None) => Some(format!(
            "the org sets a compaction window ({KEY_AUTO_COMPACT_WINDOW}={want}) that \
             settings.json does not carry yet"
        )),
        // The org stopped setting one; a value we wrote must go.
        (None, true, _) => Some(format!(
            "managed {KEY_AUTO_COMPACT_WINDOW} is stale: the org no longer sets a compaction window"
        )),
        (None, false, _) => None,
    }
}

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

    fn upstream_provider_name(&self) -> &'static str {
        UPSTREAM_PROVIDER_NAME
    }

    fn default_upstream_url(&self) -> &'static str {
        DEFAULT_UPSTREAM_URL
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

    /// `settings.json` names the forwarder's proxy address.
    fn mechanism(&self) -> Mechanism {
        Mechanism::ForwardProxy
    }

    fn config_location(&self) -> Option<PathBuf> {
        settings_path().ok()
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
        if env_block.contains_key(KEY_BASE_URL) {
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
                    "managed {KEY_NODE_EXTRA_CA_CERTS} missing from settings.json env - Claude Code \
                     would route through the proxy without trusting Gate's CA"
                )));
            }
        }

        // Last, and after routing is known good: the compaction window is a
        // preference, so it decides nothing about whether traffic reaches
        // Gate, but a managed value that no longer matches the org is still
        // ours to fix, and reading it as drift is what sends
        // `provider::reconcile_enabled` back through `connect` to rewrite it
        // on start and on org switch. The expected value comes from the
        // account file, the same place `connect` is handed it from.
        let knobs = super::compaction::harness_knobs(crate::account::compaction_window());
        let window_managed = managed
            .iter()
            .any(|v| v.as_str() == Some(KEY_AUTO_COMPACT_WINDOW));
        let in_file = env_block
            .get(KEY_AUTO_COMPACT_WINDOW)
            .and_then(|v| v.as_str());
        match window_drift(
            window_managed,
            in_file,
            knobs.claude_code_auto_compact_window.as_deref(),
        ) {
            Some(reason) => Ok(Status::Drifted(reason)),
            None => Ok(Status::Connected),
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
        // The compaction window: what the org asks for, and whether the key
        // in this file is ours to write at all (see KEY_AUTO_COMPACT_WINDOW).
        let window = super::compaction::harness_knobs(input.compaction_window)
            .claude_code_auto_compact_window;
        let window_ours = window_is_ours(
            &old_managed,
            settings.get("env").and_then(|env| env.as_object()),
        );
        let managed_now = managed_keys_for(window.as_deref(), window_ours);
        let window_applied = if window_ours {
            env_str(KEY_AUTO_COMPACT_WINDOW) == window.as_deref()
        } else {
            true // the user's; nothing of ours to apply
        };
        let already_applied = managed_now
            .iter()
            .all(|key| old_managed.iter().any(|managed| managed == key))
            && old_managed
                .iter()
                .all(|key| managed_now.contains(&key.as_str()))
            && env_value(KEY_BASE_URL).is_none()
            && env_value(KEY_CUSTOM_HEADERS).is_none()
            && env_str(KEY_HTTPS_PROXY) == Some(claude_proxy_url.as_str())
            && env_str(KEY_NO_PROXY) == Some(NO_PROXY_VALUE)
            && env_str(KEY_NODE_EXTRA_CA_CERTS) == Some(ca_cert_value.as_str())
            && window_applied;
        if already_applied {
            return Ok(());
        }

        let mut prev = settings
            .get(MARKER_KEY)
            .and_then(|v| v.get("previousEnv"))
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();

        let env_block = ensure_object(&mut settings, "env");
        // Over the keys this connect manages, not the universe: a user-owned
        // compaction window is not in `managed_now`, so it is neither
        // snapshotted nor replaced. (A window that is ours has no prior
        // value to snapshot either way: ours because we listed it, or ours
        // because nothing held it.)
        for key in &managed_now {
            if !old_managed.iter().any(|managed| managed == key) && !prev.contains_key(*key) {
                if let Some(value) = env_block.get(*key) {
                    prev.insert((*key).into(), value.clone());
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
        if window_ours {
            match &window {
                Some(value) => {
                    env_block.insert(KEY_AUTO_COMPACT_WINDOW.into(), Value::String(value.clone()));
                }
                // The org stopped setting one (or never did): a value we
                // wrote earlier goes, and the key leaves the managed list
                // with it.
                None => {
                    env_block.remove(KEY_AUTO_COMPACT_WINDOW);
                }
            }
        }

        let marker = ensure_object(&mut settings, MARKER_KEY);
        marker.insert("previousEnv".into(), Value::Object(prev));
        marker.insert(
            "managed".into(),
            Value::Array(
                managed_now
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
        let managed: Vec<String> = settings
            .get(MARKER_KEY)
            .and_then(|m| m.get("managed"))
            .and_then(|v| v.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();

        if let Some(env_block) = settings.get_mut("env").and_then(|v| v.as_object_mut()) {
            for key in MANAGED_KEYS {
                // The routing keys are reverted whether or not an older
                // build's marker listed them (the legacy relay keys are
                // removed this way). The compaction window is the exception:
                // unlisted means the user's own, and it stays.
                if key == KEY_AUTO_COMPACT_WINDOW && !managed.iter().any(|m| m == key) {
                    continue;
                }
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
        // The compaction window is managed, so disconnect takes it away with
        // the rest (`disconnect_zero_residue.rs` is the behavioural pin).
        assert!(MANAGED_KEYS.contains(&KEY_AUTO_COMPACT_WINDOW));
        assert!(!MANAGED_KEYS.contains(&"ANTHROPIC_BETAS"));
    }

    /// The window is listed only while there is a value and the key is ours;
    /// the routing keys are listed always.
    #[test]
    fn managed_keys_list_the_window_only_while_it_has_a_value_of_ours() {
        let routing = [
            KEY_BASE_URL,
            KEY_CUSTOM_HEADERS,
            KEY_HTTPS_PROXY,
            KEY_NO_PROXY,
            KEY_NODE_EXTRA_CA_CERTS,
        ];
        assert_eq!(managed_keys_for(None, true), routing);
        assert_eq!(managed_keys_for(Some("433000"), false), routing);
        assert_eq!(managed_keys_for(None, false), routing);
        assert_eq!(managed_keys_for(Some("433000"), true), MANAGED_KEYS);
    }

    /// Ownership of the window key is the marker's `managed` list; a value in
    /// `env` the marker never listed is the user's.
    #[test]
    fn a_window_the_marker_never_listed_is_the_users() {
        let mut env = Map::new();
        env.insert(
            KEY_AUTO_COMPACT_WINDOW.into(),
            Value::String("300000".into()),
        );
        let listed = vec![KEY_AUTO_COMPACT_WINDOW.to_string()];
        // Present and listed: ours (we wrote it, the user may have edited it).
        assert!(window_is_ours(&listed, Some(&env)));
        // Present and unlisted: theirs.
        assert!(!window_is_ours(&[], Some(&env)));
        // Absent: ours to write.
        assert!(window_is_ours(&[], Some(&Map::new())));
        assert!(window_is_ours(&[], None));
    }

    #[test]
    fn window_drift_covers_every_ownership_and_value_case() {
        // Ours and current.
        assert_eq!(window_drift(true, Some("433000"), Some("433000")), None);
        // Ours and stale, or ours and missing.
        assert!(window_drift(true, Some("633000"), Some("433000"))
            .unwrap()
            .contains("expected \"433000\""));
        assert!(window_drift(true, None, Some("433000"))
            .unwrap()
            .contains("missing"));
        // The user's: left alone whatever the org says.
        assert_eq!(window_drift(false, Some("300000"), Some("433000")), None);
        assert_eq!(window_drift(false, Some("300000"), None), None);
        // Nothing in the file yet and the org wants one: drift, so the
        // reconcile pass writes it.
        assert!(window_drift(false, None, Some("433000"))
            .unwrap()
            .contains("does not carry yet"));
        // Ours and the org stopped setting one: drift, so it is removed.
        assert!(window_drift(true, Some("433000"), None)
            .unwrap()
            .contains("stale"));
        // Nothing anywhere.
        assert_eq!(window_drift(false, None, None), None);
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
