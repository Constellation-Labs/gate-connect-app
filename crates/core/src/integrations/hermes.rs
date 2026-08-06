//! Hermes integration.
//!
//! Hermes routes through Gate's MITM proxy engine, not the reverse-proxy relay,
//! and the entire integration is a few variables in `~/.hermes/.env`:
//!
//! ```text
//! HTTPS_PROXY=http://127.0.0.1:<engine-port>
//! HTTP_PROXY=http://127.0.0.1:<engine-port>
//! NO_PROXY=localhost,127.0.0.1,::1
//! HERMES_CA_BUNDLE=<app-support>/proxy/ca-bundle.pem
//! ```
//!
//! **`config.yaml` is never touched.** Hermes loads `$HERMES_HOME/.env` at CLI
//! startup (`hermes_cli/env_loader.py`, called from `cli.py`) before any client
//! is constructed, and `agent/process_bootstrap.py` reads `HTTPS_PROXY` /
//! `HTTP_PROXY` / `ALL_PROXY` (plus lower-case) from the environment, honouring
//! `NO_PROXY` via `proxy_bypass_environment`. Which traffic then reaches Gate is
//! decided by the enabled catalog domains - the engine MITMs those and
//! blind-tunnels everything else.
//!
//! Why this replaced rewriting `model.base_url`: that redirect was per-endpoint,
//! so anything changing which endpoint is live routed around us while `status()`
//! still said Connected. A process-level proxy catches the socket regardless of
//! which provider config won, which retires that whole class - H1 (the
//! native-Anthropic wire) and H6 (`custom_providers` overriding `model.base_url`)
//! both stop being reachable. It also means a fresh install with no `config.yaml`
//! is no longer a special case: there is no model block to read.
//! See `docs/harness-integration-validation.md`.
//!
//! `NO_PROXY` is set for loopback so a locally-hosted provider keeps talking to
//! itself directly instead of being tunnelled through the engine - the same
//! protection the old `is_local_url` guard gave, expressed where Hermes can
//! actually act on it.
//!
//! Certificates: `HERMES_CA_BUNDLE` points at a bundle carrying the platform's
//! trust roots *plus* Gate's CA ([`crate::proxy::ca_bundle`]). The OS trust
//! store alone is not enough here. Stdlib Python does read it, but Hermes
//! installs into a venv (`setup-hermes.sh` runs `uv venv` then pip), so its
//! `httpx` / `requests` clients fall back to a **pip-installed** certifi that
//! knows nothing about the CA - measured: `certifi.where()` in that venv is
//! `…/site-packages/certifi/cacert.pem`, not the system bundle. `ssl_verify.py`
//! feeds this value to `create_default_context(cafile=…)`, which *replaces* the
//! trust store rather than adding to it, which is why it must be a full bundle
//! and never our single cert. One variable covers both client libraries -
//! `agent/model_metadata.py` reads the same key for its `requests` callsites.
//!
//! State tracking: a sidecar at `<app_support_dir>/hermes-state.json` records
//! which variables we added, so disconnect removes exactly those and leaves a
//! pre-existing `HTTPS_PROXY` (a corporate egress proxy, say) alone.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::integrations::dotenv;
use crate::registry::{ConnectInput, Integration, Status, ToolId};

const DISPLAY_NAME: &str = "Hermes";
const UPSTREAM_PROVIDER_NAME: &str = "your existing providers";
const DEFAULT_UPSTREAM_URL: &str = "https://openrouter.ai/api/v1";
const STATE_FILENAME: &str = "hermes-state.json";

/// Keep loopback off the proxy so a self-hosted provider is reached directly.
const NO_PROXY_VALUE: &str = "localhost,127.0.0.1,::1";

/// The variable status compares against; the others move with it.
const PRIMARY_VAR: &str = "HTTPS_PROXY";

#[cfg(unix)]
const CLI_BIN_PATHS: &[&str] = &["/usr/local/bin/hermes", "/usr/bin/hermes"];
#[cfg(not(unix))]
const CLI_BIN_PATHS: &[&str] = &[];

/// Sidecar that records what `connect` changed so `disconnect` can undo exactly
/// that and nothing else.
#[derive(Debug, Serialize, Deserialize, Default)]
struct State {
    #[serde(default = "default_version")]
    version: u8,
    /// Variables connect added to `~/.hermes/.env`. Anything the user had
    /// already set is absent, so disconnect leaves it be.
    #[serde(default)]
    added_vars: Vec<String>,
    /// Whether connect created `.env` itself.
    #[serde(default)]
    env_file_created: bool,
}

fn default_version() -> u8 {
    2
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
        // but on its own it can't tell our stale write apart from a proxy the
        // user has since repointed by hand - so also require that what's on
        // disk still aims at loopback, which is only ever us.
        if load_state()?.is_none() {
            return Ok(false);
        }
        Ok(configured_proxy()?.as_deref().is_some_and(is_loopback_url))
    }

    fn status(&self) -> Result<Status> {
        if !self.detect()? {
            return Ok(Status::NotInstalled);
        }
        if load_state()?.is_none() {
            return Ok(Status::Detected);
        }
        Ok(compute_status(
            configured_proxy()?.as_deref().unwrap_or(""),
            crate::proxy::persisted_engine_proxy_url().as_deref(),
            crate::proxy::engine_proxy_url().is_some(),
        ))
    }

    fn connect(&self, input: &ConnectInput) -> Result<()> {
        if !self.detect()? {
            anyhow::bail!(
                "Hermes is not installed -- install it from https://github.com/nousresearch/hermes-agent first"
            );
        }

        // Hard requirement: Hermes sends its traffic to whatever `HTTPS_PROXY`
        // names, so pointing it at an engine that is not running would break
        // its requests rather than merely un-routing them.
        let proxy_url = input.engine_proxy_url.as_deref().context(
            "the Gate proxy is not running -- turn routing on before connecting Hermes, which \
             sends its traffic through the proxy",
        )?;

        // Built before the .env write so a failure here leaves nothing behind.
        let bundle = crate::proxy::ca_bundle::ensure()?;

        let applied = dotenv::add_vars(
            &env_file_path()?,
            &[
                ("HTTPS_PROXY", proxy_url.to_string()),
                ("HTTP_PROXY", proxy_url.to_string()),
                ("NO_PROXY", NO_PROXY_VALUE.to_string()),
                ("HERMES_CA_BUNDLE", bundle.display().to_string()),
            ],
        )?;

        if applied.added.is_empty() {
            anyhow::bail!(
                "Hermes already has its own proxy settings in ~/.hermes/.env -- Gate left them \
                 alone. Remove them first if you want Hermes to route through Gate."
            );
        }

        // Preserve the ORIGINAL record across re-connects: a second connect
        // must not claim credit for variables the first one added.
        let mut state = load_state()?.unwrap_or_default();
        if state.added_vars.is_empty() {
            state.version = 2;
            state.added_vars = applied.added;
            state.env_file_created = applied.file_created;
        }
        save_state(&state)?;

        eprintln!("note: Hermes reads ~/.hermes/.env at startup -- restart it to pick this up.");
        Ok(())
    }

    fn disconnect(&self) -> Result<()> {
        let Some(state) = load_state()? else {
            return Ok(());
        };
        dotenv::remove_vars(&env_file_path()?, &state.added_vars, state.env_file_created)?;
        // Only drop the sidecar once the file is back: losing it first would
        // leave our variables in place while status reports the tool clean.
        clear_state()
    }

    fn save_upstream_credential(&self, _credential: &str) -> Result<()> {
        anyhow::bail!(
            "Hermes does not need a separate upstream credential -- Gate routes its traffic through the proxy and passes your provider credentials through untouched."
        )
    }

    fn has_upstream_credential(&self) -> Result<bool> {
        Ok(true)
    }

    fn clear_upstream_credential(&self) -> Result<()> {
        Ok(())
    }
}

/// Pure drift evaluation, split out of [`Hermes::status`] so all four states are
/// testable without a live engine.
///
/// `expected` is our proxy address from the persisted port (identity: a config
/// pointing here is ours even while routing is off); `running` is whether the
/// engine is actually up. They are separate because "pointed at us but the
/// engine is down" is a broken tool, not a cosmetic mismatch, and it is reported
/// as drift rather than Connected so the master-off sweep still disconnects it.
fn compute_status(configured: &str, expected: Option<&str>, running: bool) -> Status {
    let Some(expected) = expected else {
        return Status::Drifted(
            "Gate has never bound a proxy port, so nothing can be routing yet".into(),
        );
    };
    if configured != expected {
        return Status::Drifted(format!(
            "Hermes config does not match Gate settings (HTTPS_PROXY: {configured:?}, expected: \
             {expected:?})"
        ));
    }
    if !running {
        return Status::Drifted(format!(
            "the Gate proxy is not running, so Hermes cannot reach its provider ({configured:?} \
             is a dead address) -- turn the proxy on, or disconnect Hermes to restore it"
        ));
    }
    Status::Connected
}

/// The proxy Hermes is currently pointed at, per its own `.env`.
fn configured_proxy() -> Result<Option<String>> {
    dotenv::read_var(&env_file_path()?, PRIMARY_VAR)
}

/// Whether a proxy URL points at loopback - i.e. is one of ours rather than a
/// corporate egress proxy the user configured themselves.
fn is_loopback_url(url: &str) -> bool {
    let lowered = url.trim().to_ascii_lowercase();
    let rest = lowered
        .split_once("://")
        .map_or(lowered.as_str(), |(_, r)| r);
    let authority = rest.split('/').next().unwrap_or("");
    let host = authority
        .strip_prefix('[')
        .and_then(|a| a.split(']').next())
        .unwrap_or_else(|| authority.split(':').next().unwrap_or(""));
    host == "localhost" || host == "::1" || host.starts_with("127.")
}

fn env_file_path() -> Result<PathBuf> {
    Ok(crate::env::hermes_config_dir()?.join(".env"))
}

/// Where the Hermes launcher actually lands, checked instead of "the config
/// directory exists".
///
/// The installer writes `~/.hermes/` and drops the launcher in `~/.local/bin` -
/// so treating the config directory as proof of installation reported Hermes as
/// installed long after it was removed, and left the app offering to configure a
/// CLI that wasn't there.
fn launcher_paths() -> Result<Vec<PathBuf>> {
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

fn state_path() -> Result<PathBuf> {
    Ok(crate::env::app_support_dir()?.join(STATE_FILENAME))
}

fn load_state() -> Result<Option<State>> {
    let path = state_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
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
        std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_status_covers_the_four_states() {
        let ours = "http://127.0.0.1:9977";

        assert_eq!(compute_status(ours, Some(ours), true), Status::Connected);

        // Pointed at us but the engine is down: Hermes' requests go nowhere, so
        // this must never read as Connected.
        match compute_status(ours, Some(ours), false) {
            Status::Drifted(m) => {
                assert!(m.contains("not running"), "unexpected message: {m}");
                assert!(m.contains("disconnect"), "must offer a way out: {m}");
            }
            other => panic!("expected drift, got {other:?}"),
        }

        // A corporate proxy the user set by hand is not ours.
        match compute_status("http://proxy.corp.example:3128", Some(ours), true) {
            Status::Drifted(m) => assert!(m.contains("does not match"), "unexpected: {m}"),
            other => panic!("expected drift, got {other:?}"),
        }

        match compute_status(ours, None, false) {
            Status::Drifted(m) => assert!(m.contains("never bound"), "unexpected: {m}"),
            other => panic!("expected drift, got {other:?}"),
        }
    }

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
}
