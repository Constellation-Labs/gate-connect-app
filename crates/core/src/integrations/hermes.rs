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
//! **`config.yaml` is never written.** Hermes loads `$HERMES_HOME/.env` at CLI
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
//! is no longer a special case: nothing below refuses for want of a model block.
//! See `docs/harness-integration-validation.md`.
//!
//! `NO_PROXY` is set for loopback so a locally-hosted provider keeps talking to
//! itself directly instead of being tunnelled through the engine - the same
//! protection the old `is_local_url` guard gave, expressed where Hermes can
//! actually act on it.
//!
//! **`config.yaml` is read once, to say what Gate will see - never to change
//! it.** A correct `.env` is only half of being visible: the engine MITMs a host
//! only while an enabled catalog domain claims it, and Hermes' documented default
//! upstream (`openrouter.ai`) ships off, so the traffic can be routed through
//! Gate and blind-tunnelled past it at the same time. Connecting Hermes must not
//! quietly fix that by flipping the domain - that would widen what Gate
//! intercepts for every other client on the machine, and for a domain some
//! provider lists it would flip that provider's state too, which
//! `provider::reconcile_enabled` reads as licence to configure that provider's
//! tools. Whose traffic Gate inspects is the user's axis; whether Hermes points
//! at the proxy is this one. So `connect` prints what it found ([`Coverage`]) and
//! leaves the switch alone.
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

        // Read before the write: the refusal below has to tell our own earlier
        // work apart from the user's.
        let mut state = load_state()?.unwrap_or_default();

        let applied = dotenv::add_vars(
            &env_file_path()?,
            &[
                ("HTTPS_PROXY", proxy_url.to_string()),
                ("HTTP_PROXY", proxy_url.to_string()),
                ("NO_PROXY", NO_PROXY_VALUE.to_string()),
                ("HERMES_CA_BUNDLE", bundle.display().to_string()),
            ],
        )?;

        // Nothing added AND nothing we ever added: the variables are the user's
        // and `add_vars` left them alone, which is a refusal. The second half of
        // that test is what keeps a re-connect working. On a re-connect every
        // variable is already present because we wrote it, so testing `added`
        // alone failed with a message about settings that were Gate's own - and
        // re-connect is how a drifted Hermes is meant to be repaired, including
        // by `provider::reconcile_unmapped_tools`, which does it unattended.
        if applied.added.is_empty() && state.added_vars.is_empty() {
            anyhow::bail!(
                "Hermes already has its own proxy settings in ~/.hermes/.env -- Gate left them \
                 alone. Remove them first if you want Hermes to route through Gate."
            );
        }

        // Preserve the ORIGINAL record across re-connects: a second connect
        // must not claim credit for variables the first one added.
        if state.added_vars.is_empty() {
            state.version = 2;
            state.added_vars = applied.added;
            state.env_file_created = applied.file_created;
        }
        save_state(&state)?;

        eprintln!("note: Hermes reads ~/.hermes/.env at startup -- restart it to pick this up.");
        // A correct `.env` is only half of being seen: the engine MITMs a host
        // only while an enabled catalog domain claims it, and Hermes' own
        // default upstream ships off. Which hosts Gate inspects is the user's
        // axis, not this integration's (see [`Coverage`]), so say it rather
        // than silently flip it.
        for line in upstream_coverage().notes() {
            eprintln!("{line}");
        }
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
    let host = url_host(url);
    host == "localhost" || host == "::1" || host.starts_with("127.")
}

/// The host part of a URL, lowercased so it compares against a catalog `hosts`
/// entry directly: `openrouter.ai` for `https://openrouter.ai/api/v1`, `::1` for
/// `http://[::1]:8080`. Deliberately not a URL parser - it also runs over values
/// a user hand-wrote into a YAML file, where a missing scheme is likelier than a
/// query string.
fn url_host(url: &str) -> String {
    let lowered = url.trim().to_ascii_lowercase();
    let rest = lowered
        .split_once("://")
        .map_or(lowered.as_str(), |(_, r)| r);
    let authority = rest.split('/').next().unwrap_or("");
    authority
        .strip_prefix('[')
        .and_then(|a| a.split(']').next())
        .unwrap_or_else(|| authority.split(':').next().unwrap_or(""))
        .to_string()
}

/// What Gate will and won't see of this Hermes install, for the notes `connect`
/// prints.
///
/// Read-only on purpose. Which hosts the engine intercepts is the user's axis -
/// the provider rows and `proxy domain` - and this integration's axis is only
/// whether Hermes points at the proxy. Enabling a domain from here would widen
/// what Gate MITMs for every other client on the machine as a side effect of
/// connecting one tool, and for a domain a provider claims it would flip that
/// provider's state too, which `provider::reconcile_enabled` reads as licence to
/// configure that provider's tools. So this reports, and nothing more.
#[derive(Debug, Default, PartialEq, Eq)]
struct Coverage {
    /// Hosts a catalog entry covers, but whose switch is off: `(host, slug)`.
    switched_off: Vec<(String, String)>,
    /// Hosts no catalog entry claims, which Gate cannot route at all.
    unknown: Vec<String>,
}

impl Coverage {
    /// The lines `connect` prints, or nothing at all when every upstream is
    /// already covered - a note that says "all good" on every connect is noise,
    /// and the `proxy domains` listing is the place to confirm it.
    fn notes(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.switched_off.is_empty() {
            let list = self
                .switched_off
                .iter()
                .map(|(host, slug)| format!("{host} (`gate-connect proxy domain {slug} on`)"))
                .collect::<Vec<_>>()
                .join(", ");
            out.push(format!(
                "note: Hermes is routed through Gate, but Gate is not inspecting its provider \
                 yet -- {list}."
            ));
        }
        if !self.unknown.is_empty() {
            out.push(format!(
                "note: Gate has no proxy domain for {} -- Hermes' calls there keep working, \
                 tunnelled through unseen.",
                self.unknown.join(", ")
            ));
        }
        out
    }
}

fn upstream_coverage() -> Coverage {
    // Fall back to the built-in catalog rather than an empty one: on an
    // unreadable domains file the slugs are still right and only the enabled
    // flags are guesses, which beats reporting every host as unroutable.
    let catalog =
        crate::proxy::config::load_domains().unwrap_or_else(|_| crate::proxy::default_domains());
    coverage_of(&catalog, &config_base_urls())
}

/// The lookup behind [`upstream_coverage`], over an explicit catalog and URL
/// list so it is testable without a `$HOME`.
///
/// Loopback hosts are absent from both lists: `NO_PROXY` exempts them, so a
/// self-hosted provider is reached directly and never passes the engine at all.
fn coverage_of(catalog: &[crate::proxy::ProxyDomain], urls: &[String]) -> Coverage {
    let mut coverage = Coverage::default();
    for url in urls {
        if is_loopback_url(url) {
            continue;
        }
        let host = url_host(url);
        match crate::proxy::domain_claiming_host(catalog, &host) {
            Some(d) if d.enabled => {}
            Some(d) => {
                let claim = (host, d.slug.clone());
                if !coverage.switched_off.contains(&claim) {
                    coverage.switched_off.push(claim);
                }
            }
            None => {
                if !coverage.unknown.contains(&host) {
                    coverage.unknown.push(host);
                }
            }
        }
    }
    coverage
}

/// Keys a Hermes endpoint can be written under. `base_url`, `api` and `url` are
/// documented aliases of one another (see
/// `docs/harness-integration-validation.md`, H6), so all three have to be read.
const BASE_URL_KEYS: &[&str] = &["base_url", "api", "url"];

/// `config.yaml` as YAML, or `None` if it is absent or does not parse. Never an
/// error: nothing here is load-bearing enough to fail a connect over.
fn parsed_config() -> Option<serde_yaml::Value> {
    let path = crate::env::hermes_config_path().ok()?;
    let raw = std::fs::read_to_string(path).ok()?;
    serde_yaml::from_str(&raw).ok()
}

/// Every upstream endpoint `config.yaml` names: `model`, plus each entry of the
/// `providers:` mapping and the legacy `custom_providers:` sequence. All of them
/// and not just the live one, because which entry wins is decided inside Hermes
/// at request time - the same reason the `model.base_url` rewrite this
/// integration used to do was retired.
///
/// Falls back to [`DEFAULT_UPSTREAM_URL`] when the file names nothing, is
/// missing, or does not parse. That is Hermes' own documented default, so it is
/// the best available guess at what an unconfigured install will call.
fn config_base_urls() -> Vec<String> {
    let mut urls = parsed_config()
        .map(|root| base_urls_in(&root))
        .unwrap_or_default();
    if urls.is_empty() {
        urls.push(DEFAULT_UPSTREAM_URL.to_string());
    }
    urls
}

/// The endpoint-collecting half of [`config_base_urls`], over an already-parsed
/// document so the shapes Hermes accepts are testable without a `$HOME`.
fn base_urls_in(root: &serde_yaml::Value) -> Vec<String> {
    let mut urls = Vec::new();
    urls.extend(endpoint_of(root.get("model")));
    if let Some(map) = root
        .get("providers")
        .and_then(serde_yaml::Value::as_mapping)
    {
        urls.extend(map.iter().filter_map(|(_, v)| endpoint_of(Some(v))));
    }
    if let Some(seq) = root
        .get("custom_providers")
        .and_then(serde_yaml::Value::as_sequence)
    {
        urls.extend(seq.iter().filter_map(|v| endpoint_of(Some(v))));
    }
    urls
}

/// The endpoint an entry names, under whichever of [`BASE_URL_KEYS`] it used.
/// `None` for anything that isn't a mapping carrying one - a fresh install's
/// `model: ""`, or a `providers` value that is just a model name.
fn endpoint_of(entry: Option<&serde_yaml::Value>) -> Option<String> {
    let entry = entry?;
    BASE_URL_KEYS
        .iter()
        .filter_map(|k| entry.get(*k).and_then(serde_yaml::Value::as_str))
        .map(str::to_string)
        .next()
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

    /// The catalog with one domain forced on, since only `anthropic` ships
    /// enabled and the interesting case is a switch the user has already flipped.
    fn catalog_with(enabled: &str) -> Vec<crate::proxy::ProxyDomain> {
        let mut all = crate::proxy::default_domains();
        for d in &mut all {
            d.enabled = d.slug == enabled;
        }
        all
    }

    fn urls(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_covered_upstream_prints_nothing() {
        let coverage = coverage_of(
            &catalog_with("openrouter"),
            &urls(&["https://openrouter.ai/api/v1"]),
        );
        assert_eq!(coverage, Coverage::default());
        assert!(
            coverage.notes().is_empty(),
            "an all-good note on every connect is noise"
        );
    }

    #[test]
    fn an_upstream_behind_a_switched_off_domain_names_the_switch() {
        let coverage = coverage_of(
            &catalog_with("anthropic"),
            &urls(&["https://openrouter.ai/api/v1"]),
        );
        assert_eq!(
            coverage.switched_off,
            vec![("openrouter.ai".to_string(), "openrouter".to_string())]
        );

        let notes = coverage.notes();
        assert_eq!(notes.len(), 1, "one line, not one per axis: {notes:?}");
        assert!(
            notes[0].contains("gate-connect proxy domain openrouter on"),
            "the note has to carry the command that fixes it: {}",
            notes[0]
        );
        assert!(
            notes[0].contains("routed through Gate"),
            "and must not read as though Hermes failed to connect: {}",
            notes[0]
        );
    }

    #[test]
    fn an_upstream_gate_cannot_route_says_so_instead() {
        let coverage = coverage_of(
            &catalog_with("anthropic"),
            &urls(&["https://api.together.xyz/v1"]),
        );
        assert!(
            coverage.switched_off.is_empty(),
            "there is no switch to name"
        );
        assert_eq!(coverage.unknown, vec!["api.together.xyz".to_string()]);

        let notes = coverage.notes();
        assert!(
            notes[0].contains("keep working"),
            "the tool still works; only Gate's view is missing: {}",
            notes[0]
        );
    }

    #[test]
    fn loopback_and_duplicate_hosts_are_left_out() {
        let coverage = coverage_of(
            &catalog_with("anthropic"),
            &urls(&[
                // Exempted by NO_PROXY - never reaches the engine.
                "http://127.0.0.1:11434/v1",
                "http://localhost:8080",
                // The same host twice, and the same unknown host twice.
                "https://openrouter.ai/api/v1",
                "https://openrouter.ai/api",
                "https://api.together.xyz/v1",
                "https://api.together.xyz/v2",
            ]),
        );
        assert_eq!(coverage.switched_off.len(), 1, "{coverage:?}");
        assert_eq!(coverage.unknown.len(), 1, "{coverage:?}");
    }

    #[test]
    fn every_endpoint_shape_in_config_yaml_is_found() {
        let root: serde_yaml::Value = serde_yaml::from_str(
            "model:\n  provider: custom\n  base_url: https://openrouter.ai/api/v1\n\
             providers:\n  mine:\n    api: https://api.openai.com/v1\n  named: gpt-4o\n\
             custom_providers:\n  - url: https://api.anthropic.com\n",
        )
        .unwrap();

        assert_eq!(
            base_urls_in(&root),
            vec![
                "https://openrouter.ai/api/v1",
                "https://api.openai.com/v1",
                "https://api.anthropic.com",
            ],
            "all three aliases, in all three places, and a bare model name skipped"
        );
    }

    #[test]
    fn an_unconfigured_model_block_yields_no_endpoint() {
        // A fresh install ships `model: ""` (H5), and a provider entry may be a
        // plain model name. Neither is a mapping, so neither names a host - the
        // caller falls back to Hermes' documented default.
        for body in ["model: \"\"\n", "model: gpt-4o\n", "providers:\n  a: b\n"] {
            let root: serde_yaml::Value = serde_yaml::from_str(body).unwrap();
            assert!(
                base_urls_in(&root).is_empty(),
                "expected no endpoints from {body:?}"
            );
        }
    }

    #[test]
    fn host_extraction_survives_the_shapes_a_user_hand_writes() {
        for (url, host) in [
            ("https://openrouter.ai/api/v1", "openrouter.ai"),
            ("openrouter.ai/api/v1", "openrouter.ai"),
            ("HTTPS://OpenRouter.AI/api", "openrouter.ai"),
            ("http://192.168.1.9:8080/v1", "192.168.1.9"),
            ("http://[::1]:8080/v1", "::1"),
            ("  https://api.openai.com  ", "api.openai.com"),
        ] {
            assert_eq!(url_host(url), host, "for {url}");
        }
    }

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
