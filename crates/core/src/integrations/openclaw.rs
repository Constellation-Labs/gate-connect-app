//! OpenClaw integration.
//!
//! Unlike every other integration here, OpenClaw routes through Gate's MITM
//! proxy engine rather than the reverse-proxy relay, and it is configured with
//! a single key:
//!
//! ```json5
//! "proxy": { "proxyUrl": "http://127.0.0.1:<engine-port>" }
//! ```
//!
//! Setting that puts OpenClaw in *managed proxy mode*, which installs a
//! process-wide interceptor over `fetch`/undici, `node:http(s)` and WebSocket
//! clients, and replaces caller-supplied agents so libraries like axios / got /
//! node-fetch cannot bypass it. That single value is the entire integration:
//! **no per-provider `baseUrl`, no provider discovery, no credential, and no
//! headers.** Which traffic actually reaches Gate is then decided by the
//! enabled catalog domains - the engine MITMs those and blind-tunnels
//! everything else.
//!
//! Why this replaced per-provider `baseUrl` rewriting: discovery could not see
//! providers configured via `openclaw models auth login` (they live in a
//! per-agent store, not `openclaw.json`), and redirecting `anthropic` made
//! OpenClaw drop its implicit beta headers. A process-wide proxy has no
//! discovery step and leaves every `baseUrl` canonical, so both problems stop
//! existing rather than being worked around. See
//! `docs/harness-integration-validation.md` H2 and H3.
//!
//! **`proxy.loopbackMode` is deliberately never written.** Its default,
//! `gateway-only`, lets a configured local provider origin bypass the proxy
//! after proving the target is genuinely loopback (exact origin match, loopback
//! hostname, and *every* resolved address pinned to loopback). That is a
//! stricter version of the local-endpoint protection this integration used to
//! implement by hand, so the right move is to leave it alone.
//!
//! CA trust is mandatory here, unlike the relay (which is plaintext loopback):
//! the engine presents leaf certs minted by Gate's local CA, and Node ships its
//! own trust bundle. `connect` writes `NODE_EXTRA_CA_CERTS` into
//! `~/.openclaw/.env`, which OpenClaw loads pre-bootstrap and propagates into
//! the service unit it generates. `NODE_EXTRA_CA_CERTS` *appends* to Node's
//! roots rather than replacing them, so it cannot break unrelated TLS.
//!
//! Restart required: OpenClaw reads `proxy.proxyUrl` at gateway startup, so
//! `connect` and `disconnect` both tell the user to run `openclaw gateway
//! restart`. We never run it ourselves - restarting the user's gateway is their
//! call, not a side effect of a config write.
//!
//! Config format: `openclaw.json` is JSON5 (comments + trailing commas
//! allowed), so we parse with the `json5` crate and write back with
//! `serde_json` (valid JSON5). NOTE: a connect rewrites the file and so
//! **drops any comments / JSON5 formatting** the user had in it.
//!
//! State tracking: a sidecar at `<app_support_dir>/openclaw-state.json` records
//! the original `proxy.proxyUrl` (and whether a `proxy` block existed at all),
//! keeping our markers out of the user-owned config file.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::env;
use crate::registry::{ConnectInput, Integration, Status, ToolId};

const UPSTREAM_PROVIDER_NAME: &str = "your existing providers";
const DEFAULT_UPSTREAM_URL: &str = "https://api.anthropic.com";

/// Sidecar state file. We keep state out of the user-owned `openclaw.json`.
const STATE_FILENAME: &str = "openclaw-state.json";

/// The one env var we manage in `~/.openclaw/.env`, so the gateway trusts the
/// engine's minted leaf certs.
const CA_ENV_KEY: &str = "NODE_EXTRA_CA_CERTS";

/// What the user has to run for a config change to take effect.
const RESTART_HINT: &str = "run `openclaw gateway restart` for this to take effect";

/// Likely install locations of the `openclaw` binary. Detection falls back to
/// the config dir, which is the primary signal for OpenClaw (it always writes
/// `~/.openclaw`), so this stays empty rather than guessing per-user prefixes.
#[cfg(unix)]
const CLI_BIN_PATHS: &[&str] = &[];
#[cfg(windows)]
const CLI_BIN_PATHS: &[&str] = &[];

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    /// Original `proxy.proxyUrl`. `None` means the key was absent.
    #[serde(default)]
    previous_proxy_url: Option<Value>,
    /// Whether a `proxy` object existed at all before connect. When false,
    /// disconnect removes the whole block rather than leaving an empty one.
    #[serde(default)]
    proxy_existed: bool,
    /// Whether connect added the `NODE_EXTRA_CA_CERTS` line to
    /// `~/.openclaw/.env`. When false, the user already had one and we left it
    /// alone - so disconnect must not remove it.
    #[serde(default)]
    ca_env_added: bool,
    /// Whether connect created `~/.openclaw/.env` itself. Disconnect deletes
    /// the file only in that case, and only if removing our line empties it.
    #[serde(default)]
    ca_env_file_created: bool,
}

pub struct OpenClaw;

impl Integration for OpenClaw {
    fn id(&self) -> ToolId {
        ToolId::OpenClaw
    }

    fn display_name(&self) -> &'static str {
        "OpenClaw"
    }

    fn upstream_provider_name(&self) -> &'static str {
        UPSTREAM_PROVIDER_NAME
    }

    fn default_upstream_url(&self) -> &'static str {
        DEFAULT_UPSTREAM_URL
    }

    fn requires_upstream_credential(&self) -> bool {
        // OpenClaw already has the user's upstream credentials. Gate is a pure
        // passthrough on `Authorization` / `x-api-key`, so no separate key.
        false
    }

    fn detect(&self) -> Result<bool> {
        if CLI_BIN_PATHS.iter().any(|p| Path::new(p).exists()) {
            return Ok(true);
        }
        Ok(env::openclaw_config_dir()?.exists())
    }

    fn config_is_managed(&self) -> Result<bool> {
        // Two-part marker. The sidecar only exists because connect() wrote it,
        // but on its own it can't tell our stale write apart from a proxyUrl
        // the user has since repointed by hand - so also require that what's on
        // disk still aims at loopback, which is only ever us.
        if load_state()?.is_none() {
            return Ok(false);
        }
        let settings = load_settings()?.unwrap_or_default();
        Ok(current_proxy_url(&settings).is_some_and(is_loopback_url))
    }

    fn status(&self) -> Result<Status> {
        if !self.detect()? {
            return Ok(Status::NotInstalled);
        }
        if load_state()?.is_none() {
            return Ok(Status::Detected);
        }
        let settings = load_settings()?.unwrap_or_default();
        Ok(compute_status(
            current_proxy_url(&settings).unwrap_or(""),
            crate::proxy::persisted_engine_proxy_url().as_deref(),
            crate::proxy::engine_proxy_url().is_some(),
        ))
    }

    fn connect(&self, input: &ConnectInput) -> Result<()> {
        if !self.detect()? {
            anyhow::bail!(
                "OpenClaw is not installed on this machine -- install it from https://docs.openclaw.ai first"
            );
        }

        // Hard requirement, not a nicety: managed proxy mode routes *all* of
        // OpenClaw's egress and force-clears no_proxy, so pointing it at an
        // engine that isn't running takes the tool's whole network down rather
        // than degrading its inference.
        let proxy_url = input.engine_proxy_url.as_deref().context(
            "the Gate proxy is not running -- turn routing on before connecting OpenClaw, which \
             sends all of its traffic through the proxy",
        )?;

        // The engine mints leaf certs from Gate's local CA, so OpenClaw must
        // trust it. There is no check here because there is no reachable state
        // to check for: `engine_proxy_url` is `Some` only while the proxy is
        // routing, and turning the proxy on runs `ca::ensure_trusted()` in the
        // same flow (see proxy::manager). A check would also be untestable -
        // it reads the real OS trust store, which a temp HOME cannot fake.
        let mut settings = load_settings()?.unwrap_or_default();

        // Preserve the ORIGINAL snapshot across re-connects: a second connect
        // must not record our own intermediate proxyUrl as the user's.
        let mut state = load_state()?.unwrap_or_default();
        if load_state()?.is_none() {
            state.proxy_existed = settings.get("proxy").is_some_and(Value::is_object);
            state.previous_proxy_url = settings
                .get("proxy")
                .and_then(|v| v.as_object())
                .and_then(|p| p.get("proxyUrl"))
                .cloned();
        }

        let (ca_env_added, ca_env_file_created) = write_ca_env()?;
        // Only record a fresh write; a re-connect must keep the first answer,
        // or disconnect would leave behind a line we did add.
        if load_state()?.is_none() {
            state.ca_env_added = ca_env_added;
            state.ca_env_file_created = ca_env_file_created;
        }

        // `proxy.loopbackMode` and `proxy.tls` are deliberately untouched - see
        // the module docs. We own exactly one key.
        let proxy = ensure_object(&mut settings, "proxy");
        proxy.insert("proxyUrl".to_string(), Value::String(proxy_url.to_string()));

        save_state(&state)?;
        write_settings(&settings)?;

        eprintln!("note: OpenClaw reads proxy.proxyUrl at gateway startup -- {RESTART_HINT}.");
        Ok(())
    }

    fn disconnect(&self) -> Result<()> {
        let Some(state) = load_state()? else {
            return Ok(());
        };

        if let Some(mut settings) = load_settings()? {
            if let Some(proxy) = settings.get_mut("proxy").and_then(|v| v.as_object_mut()) {
                match &state.previous_proxy_url {
                    Some(v) => {
                        proxy.insert("proxyUrl".to_string(), v.clone());
                    }
                    None => {
                        proxy.remove("proxyUrl");
                    }
                }
                // Drop a `proxy` block that only existed because we made it,
                // and never strip keys (loopbackMode, tls) the user set.
                if proxy.is_empty() && !state.proxy_existed {
                    settings.remove("proxy");
                }
            }

            if settings.is_empty() {
                let path = env::openclaw_config_path()?;
                if path.exists() {
                    fs::remove_file(&path)
                        .with_context(|| format!("removing {}", path.display()))?;
                }
            } else {
                write_settings(&settings)?;
            }
        }

        if state.ca_env_added {
            remove_ca_env(state.ca_env_file_created)?;
        }

        // Only drop the sidecar once the restored config is on disk: losing it
        // before a failed write would leave our proxyUrl in openclaw.json while
        // status reports the tool clean and re-disconnect no-ops.
        remove_state()?;
        eprintln!("note: OpenClaw is still using the old proxy setting -- {RESTART_HINT}.");
        Ok(())
    }

    fn save_upstream_credential(&self, _credential: &str) -> Result<()> {
        anyhow::bail!(
            "OpenClaw does not need a separate upstream credential -- Gate routes its traffic through the proxy and passes your provider credentials through untouched."
        )
    }

    /// Hidden from the popover pending an end-to-end run against a real
    /// install: the mechanism is verified in OpenClaw's source but the
    /// integration has not been exercised against a live gateway.
    /// See docs/harness-integration-validation.md.
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

/// Pure drift evaluation, split out of [`OpenClaw::status`] so all four states
/// are testable without a live engine.
///
/// `expected` is our proxy address from the persisted port (identity: a config
/// pointing here is ours even while routing is off); `running` is whether the
/// engine is actually up. The two are separate because "pointed at us but the
/// engine is down" is the dangerous state, not a cosmetic one - managed proxy
/// mode force-clears `no_proxy` and has no bypass list, so OpenClaw has *no*
/// egress at all until it is cleared. Reported as drift rather than Connected
/// so the master-off sweep still picks it up for disconnect.
fn compute_status(configured: &str, expected: Option<&str>, running: bool) -> Status {
    let Some(expected) = expected else {
        return Status::Drifted(
            "Gate has never bound a proxy port, so nothing can be routing yet".into(),
        );
    };
    if configured != expected {
        return Status::Drifted(format!(
            "OpenClaw config does not match Gate settings (proxy.proxyUrl: {configured:?}, \
             expected: {expected:?})"
        ));
    }
    if !running {
        return Status::Drifted(format!(
            "the Gate proxy is not running, so OpenClaw has no route out ({configured:?} is a \
             dead address) -- turn the proxy on, or disconnect OpenClaw to restore it"
        ));
    }
    Status::Connected
}

/// The `proxy.proxyUrl` currently on disk, if any.
fn current_proxy_url(settings: &Map<String, Value>) -> Option<&str> {
    settings
        .get("proxy")
        .and_then(|v| v.as_object())
        .and_then(|p| p.get("proxyUrl"))
        .and_then(|v| v.as_str())
}

/// Whether a proxy URL points at loopback - i.e. is one of ours rather than a
/// corporate egress proxy the user configured themselves.
fn is_loopback_url(url: &str) -> bool {
    let rest = url
        .trim()
        .to_ascii_lowercase()
        .split_once("://")
        .map(|(_, r)| r.to_string())
        .unwrap_or_else(|| url.trim().to_ascii_lowercase());
    let authority = rest.split('/').next().unwrap_or("").to_string();
    let host = authority
        .strip_prefix('[')
        .and_then(|a| a.split(']').next())
        .map(str::to_string)
        .unwrap_or_else(|| authority.split(':').next().unwrap_or("").to_string());
    host == "localhost" || host == "127.0.0.1" || host == "::1" || host.starts_with("127.")
}

// --- ~/.openclaw/.env -------------------------------------------------

fn env_file_path() -> Result<PathBuf> {
    Ok(env::openclaw_config_dir()?.join(".env"))
}

/// Point OpenClaw's Node runtime at Gate's CA. Returns
/// `(line_added, file_created)` so disconnect removes only what we added.
///
/// A `NODE_EXTRA_CA_CERTS` the user already set is left strictly alone: it may
/// be a corporate bundle the rest of their setup depends on, and clobbering it
/// would break TLS well outside Gate's blast radius.
fn write_ca_env() -> Result<(bool, bool)> {
    let path = env_file_path()?;
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let file_created = !path.exists();

    if existing
        .lines()
        .any(|l| l.trim_start().starts_with(CA_ENV_KEY))
    {
        return Ok((false, false));
    }

    let ca = crate::proxy::ca_cert_path()?;
    let mut body = existing;
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str(&format!("{CA_ENV_KEY}={}\n", ca.display()));

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    crate::primitives::write_file(&path, body.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok((true, file_created))
}

/// Remove the `NODE_EXTRA_CA_CERTS` line connect added, and the file too if we
/// created it and nothing else is left in it.
fn remove_ca_env(file_created: bool) -> Result<()> {
    let path = env_file_path()?;
    if !path.exists() {
        return Ok(());
    }
    let existing =
        fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let kept: Vec<&str> = existing
        .lines()
        .filter(|l| !l.trim_start().starts_with(CA_ENV_KEY))
        .collect();

    if kept.iter().all(|l| l.trim().is_empty()) && file_created {
        return fs::remove_file(&path).with_context(|| format!("removing {}", path.display()));
    }

    let mut body = kept.join("\n");
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    crate::primitives::write_file(&path, body.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

// --- file I/O ---------------------------------------------------------

fn settings_path() -> Result<PathBuf> {
    env::openclaw_config_path()
}

fn state_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join(STATE_FILENAME))
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
    // openclaw.json is JSON5 (comments + trailing commas allowed).
    let value: Value =
        json5::from_str(&raw).with_context(|| format!("parsing {} as JSON5", path.display()))?;
    match value {
        Value::Object(m) => Ok(Some(m)),
        _ => anyhow::bail!("{} top level must be a JSON object", path.display()),
    }
}

fn write_settings(settings: &Map<String, Value>) -> Result<()> {
    let path = settings_path()?;
    let mut body = serde_json::to_string_pretty(settings).context("serializing openclaw.json")?;
    body.push('\n');
    // 0o600 defensively - the file holds no Gate credential, but may carry
    // other user config. Atomic-write protects against a crash mid-write
    // corrupting the config.
    crate::primitives::write_file(&path, body.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

fn load_state() -> Result<Option<State>> {
    let path = state_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let state: State = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {} as JSON", path.display()))?;
    Ok(Some(state))
}

fn save_state(state: &State) -> Result<()> {
    let path = state_path()?;
    let body = serde_json::to_string_pretty(state).context("serializing openclaw-state.json")?;
    crate::primitives::write_file(&path, body.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

fn remove_state() -> Result<()> {
    let path = state_path()?;
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

fn ensure_object<'a>(parent: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    if !matches!(parent.get(key), Some(Value::Object(_))) {
        parent.insert(key.into(), Value::Object(Map::new()));
    }
    parent
        .get_mut(key)
        .and_then(|v| v.as_object_mut())
        .expect("inserted an object")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn loopback_detection_separates_our_proxy_from_a_corporate_one() {
        for u in [
            "http://127.0.0.1:9977",
            "http://localhost:9977",
            "http://127.0.0.5:1234",
            "HTTP://LOCALHOST:9977",
        ] {
            assert!(is_loopback_url(u), "expected loopback: {u}");
        }
        for u in [
            "http://proxy.corp.example:3128",
            "https://egress.example.com",
            "http://10.0.0.7:3128",
        ] {
            assert!(!is_loopback_url(u), "expected non-loopback: {u}");
        }
    }

    #[test]
    fn connect_writes_only_proxy_url_and_leaves_siblings_alone() {
        // The whole integration is one key. `loopbackMode` in particular must
        // survive: its default is what lets a configured local provider bypass
        // the proxy, and that is the local-endpoint protection this integration
        // used to hand-roll.
        let mut settings = Map::new();
        settings.insert(
            "proxy".to_string(),
            json!({ "loopbackMode": "gateway-only", "tls": { "caFile": "/user/ca.pem" } }),
        );
        settings.insert("models".to_string(), json!({ "providers": {} }));

        let proxy = ensure_object(&mut settings, "proxy");
        proxy.insert(
            "proxyUrl".to_string(),
            Value::String("http://127.0.0.1:9977".into()),
        );

        let proxy = settings["proxy"].as_object().unwrap();
        assert_eq!(proxy["proxyUrl"], json!("http://127.0.0.1:9977"));
        assert_eq!(proxy["loopbackMode"], json!("gateway-only"));
        assert_eq!(proxy["tls"], json!({ "caFile": "/user/ca.pem" }));
        // No provider was touched: no discovery, no baseUrl rewriting.
        assert_eq!(settings["models"], json!({ "providers": {} }));
    }

    #[test]
    fn compute_status_covers_the_four_states() {
        let ours = "http://127.0.0.1:9977";

        assert_eq!(compute_status(ours, Some(ours), true), Status::Connected);

        // Pointed at us but the engine is down. This is the state that leaves
        // OpenClaw with no egress at all, so it must never read as Connected.
        match compute_status(ours, Some(ours), false) {
            Status::Drifted(m) => {
                assert!(m.contains("no route out"), "unexpected message: {m}");
                assert!(m.contains("disconnect"), "must offer a way out: {m}");
            }
            other => panic!("expected drift, got {other:?}"),
        }

        // Hand-edited to something else - including a real corporate proxy.
        match compute_status("http://proxy.corp.example:3128", Some(ours), true) {
            Status::Drifted(m) => assert!(m.contains("does not match"), "unexpected: {m}"),
            other => panic!("expected drift, got {other:?}"),
        }

        // No port ever bound.
        match compute_status(ours, None, false) {
            Status::Drifted(m) => assert!(m.contains("never bound"), "unexpected: {m}"),
            other => panic!("expected drift, got {other:?}"),
        }
    }

    #[test]
    fn current_proxy_url_reads_the_single_key() {
        let settings = json!({ "proxy": { "proxyUrl": "http://127.0.0.1:9977" } })
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(current_proxy_url(&settings), Some("http://127.0.0.1:9977"));
        assert_eq!(current_proxy_url(&Map::new()), None);
    }
}
