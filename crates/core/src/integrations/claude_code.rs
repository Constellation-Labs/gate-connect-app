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
use std::fs;
use std::path::PathBuf;

use crate::env;
use crate::integrations::binaries;
use crate::integrations::precedence::Override;
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

    /// AG-931: Claude Code can be told to skip Anthropic and call Bedrock or
    /// Vertex instead, and then its traffic reaches the engine for a host no
    /// catalog entry claims and is tunnelled through unseen. The route is still
    /// ours - `HTTPS_PROXY` is honoured either way - so the row is routed, and
    /// Protected would be false about the inspection half.
    fn upstream_coverage(&self) -> Option<crate::coverage::UpstreamCoverage> {
        let catalog = crate::proxy::config::load_domains()
            .unwrap_or_else(|_| crate::proxy::default_domains());
        let urls = cloud_endpoints(&effective_env());
        if urls.is_empty() {
            return None;
        }
        let coverage = super::hermes::coverage_of(&catalog, &urls);
        (!coverage.is_covered()).then_some(coverage)
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

/// The `env` block Claude Code will run with, as far as it can be read from
/// here: `~/.claude/settings.json`, overlaid by the enterprise managed settings
/// that outrank it. The project layers and the shell sit in between and are
/// not visible, for the reason [`managed_settings_override`] gives, so a
/// provider selected there reads as Anthropic.
fn effective_env() -> Map<String, Value> {
    let mut merged = Map::new();
    let user = load_settings().ok().flatten();
    let managed = super::json_config::load_object(&env::claude_code_managed_settings_path())
        .ok()
        .flatten();
    for layer in [user, managed].into_iter().flatten() {
        if let Some(block) = layer.get("env").and_then(Value::as_object) {
            merged.extend(block.clone());
        }
    }
    merged
}

/// The cloud endpoint an `env` block sends Claude Code to instead of
/// Anthropic, or nothing when it selects none.
///
/// Bedrock and Vertex are chosen by a flag rather than a URL, so the host is
/// rebuilt the way Claude Code builds it: the explicit base URL when one is
/// set, otherwise the regional runtime host. A missing region falls back to
/// the provider's default, which may not be the one the shell supplies; the
/// finding does not depend on it, since no region's host is in the catalog.
fn cloud_endpoints(env_block: &Map<String, Value>) -> Vec<String> {
    let var = |key: &str| {
        env_block
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };
    let flag = |key: &str| {
        var(key).is_some_and(|v| {
            !matches!(
                v.to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
    };
    let mut urls = Vec::new();
    if flag("CLAUDE_CODE_USE_BEDROCK") {
        urls.push(var("ANTHROPIC_BEDROCK_BASE_URL").map_or_else(
            || {
                let region = var("AWS_REGION").unwrap_or("us-east-1");
                format!("https://bedrock-runtime.{region}.amazonaws.com")
            },
            str::to_string,
        ));
    }
    if flag("CLAUDE_CODE_USE_VERTEX") {
        urls.push(var("ANTHROPIC_VERTEX_BASE_URL").map_or_else(
            || match var("CLOUD_ML_REGION") {
                Some(region) if !region.eq_ignore_ascii_case("global") => {
                    format!("https://{region}-aiplatform.googleapis.com")
                }
                _ => "https://aiplatform.googleapis.com".to_string(),
            },
            str::to_string,
        ));
    }
    urls
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

#[cfg(test)]
mod cloud_endpoint_tests {
    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> Map<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
            .collect()
    }

    /// AG-931: the flag, not a URL, is what sends Claude Code to Bedrock, so
    /// the host has to be rebuilt from the region.
    #[test]
    fn a_bedrock_flag_names_the_regional_runtime_host() {
        let urls = cloud_endpoints(&env_of(&[
            ("CLAUDE_CODE_USE_BEDROCK", "1"),
            ("AWS_REGION", "eu-central-1"),
        ]));
        assert_eq!(
            urls,
            vec!["https://bedrock-runtime.eu-central-1.amazonaws.com"]
        );

        let coverage =
            crate::integrations::hermes::coverage_of(&crate::proxy::default_domains(), &urls);
        assert_eq!(
            coverage.unknown,
            vec!["bedrock-runtime.eu-central-1.amazonaws.com"]
        );
        assert!(!coverage.is_covered());
    }

    #[test]
    fn an_explicit_cloud_base_url_wins_over_the_region() {
        let urls = cloud_endpoints(&env_of(&[
            ("CLAUDE_CODE_USE_BEDROCK", "true"),
            ("AWS_REGION", "eu-central-1"),
            (
                "ANTHROPIC_BEDROCK_BASE_URL",
                "https://bedrock.corp.example/v1",
            ),
            ("CLAUDE_CODE_USE_VERTEX", "1"),
            ("CLOUD_ML_REGION", "us-east5"),
        ]));
        assert_eq!(
            urls,
            vec![
                "https://bedrock.corp.example/v1",
                "https://us-east5-aiplatform.googleapis.com",
            ]
        );
    }

    #[test]
    fn a_flag_set_to_off_selects_no_cloud_provider() {
        // Claude Code reads these as booleans, and a settings file that turned
        // Bedrock off by writing "0" must not be reported as calling it.
        for off in ["0", "false", "FALSE", "", "  "] {
            assert!(
                cloud_endpoints(&env_of(&[("CLAUDE_CODE_USE_BEDROCK", off)])).is_empty(),
                "{off:?} selects nothing"
            );
        }
        assert!(cloud_endpoints(&Map::new()).is_empty());
        assert_eq!(
            cloud_endpoints(&env_of(&[
                ("CLAUDE_CODE_USE_VERTEX", "1"),
                ("CLOUD_ML_REGION", "global")
            ])),
            vec!["https://aiplatform.googleapis.com"]
        );
    }
}
