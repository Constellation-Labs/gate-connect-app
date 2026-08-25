//! OpenClaw integration.
//!
//! Unlike every other integration here, OpenClaw routes through Gate's MITM
//! proxy engine rather than the reverse-proxy relay, and it is configured with
//! two keys:
//!
//! ```json5
//! "proxy": { "enabled": true, "proxyUrl": "http://127.0.0.1:<engine-port>" }
//! ```
//!
//! **Both are required, and `enabled` is the load-bearing one.** OpenClaw's
//! `startProxy` opens with `if (config?.enabled !== true) return null`
//! (openclaw: `dist/proxy-lifecycle`), and its config schema declares
//! `enabled: boolean().optional()` with no default, so a `proxy` block carrying
//! only a URL leaves managed proxy mode switched OFF. This integration wrote
//! only the URL until the first end-to-end run against a real install caught
//! it: OpenClaw logged `proxy=none`, went straight to `api.anthropic.com` on
//! the user's own key, and connect + status reported success throughout. Do not
//! "simplify" this back to one key.
//!
//! With both set, OpenClaw enters *managed proxy mode*, which installs a
//! process-wide interceptor over `fetch`/undici, `node:http(s)` and WebSocket
//! clients, and replaces caller-supplied agents so libraries like axios / got /
//! node-fetch cannot bypass it. Those two values are the entire integration:
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
//! **ChatGPT-subscription auth needs a second catalog domain, which the user
//! owns.** When the `openai` entry in `auth.profiles` carries a bearer-style
//! `mode` (`oauth`, the subscription login, or `token`) rather than an API key,
//! OpenClaw's model calls go to `chatgpt.com/backend-api/codex/responses`
//! (openclaw: `packages/ai/src/providers/openai-chatgpt-responses.ts`), not to
//! `api.openai.com` - so the `openai` domain the OpenAI provider switch enables
//! covers none of it, and with the `chatgpt` domain off the engine blind-tunnels
//! the CONNECT, exactly as if the integration were not there.
//!
//! `connect` reports that and stops there. It used to switch the domain on
//! itself, which crossed two axes that have to stay apart: whether a tool points
//! at Gate is this integration's business, whose traffic Gate inspects is the
//! user's. Flipping a domain from here widened interception for every other
//! proxied client on the machine - `chatgpt.com` is a browser-reachable host, so
//! that included web traffic nobody asked to route - and it did so from a toggle
//! whose label says "OpenClaw". The domain has a row and a switch of its own on
//! the ledger now, under OpenAI and outside its cascade
//! (`provider::chat_domain_slugs`), so the note below names something the user
//! can act on.
//!
//! Codex hits the same auth-mode split and solves it the other way - a
//! `base_url` rewrite onto the relay (`integrations/codex.rs`) - because its
//! embedded agent ignores the system proxy and cannot be MITM'd at all.
//! OpenClaw's managed proxy mode does honour the proxy, so the MITM route is
//! available here, and the `baseUrl` route is not (see above). Both routes lean
//! on the same catalog entry; see its comments in `proxy::default_domains`.
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
use crate::integrations::dotenv;
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

/// Catalog domain that carries ChatGPT-subscription model calls
/// (`chatgpt.com/backend-api/codex/responses`). This one and not the
/// `chatgpt-apps` entry beside it, which claims the same host but serves the
/// app's own paths and passes the Responses call through on purpose - see both
/// entries in `proxy::default_domains`.
const CHATGPT_DOMAIN_SLUG: &str = "chatgpt";

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
    /// Original `proxy.enabled`. `None` means the key was absent, which is the
    /// usual case and is why it has to be restored separately from the URL: the
    /// two are independent keys and a user may have set either alone.
    #[serde(default)]
    previous_proxy_enabled: Option<Value>,
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
            proxy_is_enabled(&settings),
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
        // must not record our own intermediate proxyUrl as the user's. Read
        // once - every undo-log field below is conditioned on it, and nothing
        // here writes the sidecar until the end.
        let first_connect = load_state()?.is_none();
        let mut state = load_state()?.unwrap_or_default();
        if first_connect {
            state.proxy_existed = settings.get("proxy").is_some_and(Value::is_object);
            let previous = settings.get("proxy").and_then(|v| v.as_object());
            state.previous_proxy_url = previous.and_then(|p| p.get("proxyUrl")).cloned();
            state.previous_proxy_enabled = previous.and_then(|p| p.get("enabled")).cloned();
        }

        // Point OpenClaw's Node runtime at Gate's CA. A value the user already
        // set is left strictly alone - it may be a corporate bundle the rest of
        // their setup depends on.
        let applied = dotenv::add_vars(
            &env_file_path()?,
            &[(
                CA_ENV_KEY,
                crate::proxy::ca_cert_path()?.display().to_string(),
            )],
        )?;
        // Only record a fresh write; a re-connect must keep the first answer,
        // or disconnect would leave behind a line we did add.
        if first_connect {
            state.ca_env_added = !applied.added.is_empty();
            state.ca_env_file_created = applied.file_created;
        }

        // Both keys are required, and `enabled` is the load-bearing one.
        // OpenClaw's `startProxy` opens with `if (config?.enabled !== true)
        // return null` (openclaw: dist/proxy-lifecycle), and `enabled` is
        // `boolean().optional()` in its config schema with NO default, so a
        // `proxy` block carrying only a URL leaves managed proxy mode switched
        // off. Writing just the URL - which this did until now - meant OpenClaw
        // silently ignored it and went straight to the provider on the user's
        // own key, while connect succeeded and status read Connected.
        //
        // `proxy.loopbackMode` and `proxy.tls` remain deliberately untouched;
        // see the module docs.
        let proxy = ensure_object(&mut settings, "proxy");
        proxy.insert("enabled".to_string(), Value::Bool(true));
        proxy.insert("proxyUrl".to_string(), Value::String(proxy_url.to_string()));

        save_state(&state)?;

        write_settings(&settings)?;

        eprintln!("note: OpenClaw reads proxy.proxyUrl at gateway startup -- {RESTART_HINT}.");
        // Managed proxy mode routes every request to the engine, but the engine
        // MITMs only hosts an enabled catalog domain claims - and a
        // ChatGPT-subscription login never touches api.openai.com, the one domain
        // the OpenAI provider switch covers. Say so instead of switching it on:
        // that domain is a machine-wide choice with a row of its own (see the
        // module docs), so the actionable thing here is naming it.
        if let Some(note) = coverage_note(openai_auth_mode(&settings)) {
            eprintln!("{note}");
        }
        Ok(())
    }

    fn disconnect(&self) -> Result<()> {
        let Some(state) = load_state()? else {
            return Ok(());
        };

        if let Some(mut settings) = load_settings()? {
            if let Some(proxy) = settings.get_mut("proxy").and_then(|v| v.as_object_mut()) {
                for (key, previous) in [
                    ("proxyUrl", &state.previous_proxy_url),
                    ("enabled", &state.previous_proxy_enabled),
                ] {
                    match previous {
                        Some(v) => {
                            proxy.insert(key.to_string(), v.clone());
                        }
                        None => {
                            proxy.remove(key);
                        }
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
            dotenv::remove_vars(
                &env_file_path()?,
                &[CA_ENV_KEY.to_string()],
                state.ca_env_file_created,
            )?;
        }

        // Nothing to hand back on the domain side: connect never took a domain,
        // so a disconnect must not switch one off either. The `chatgpt` row
        // belongs to the user, and they may well have turned it on for the
        // ChatGPT app or Codex rather than for OpenClaw.

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

    fn has_upstream_credential(&self) -> Result<bool> {
        Ok(true)
    }

    fn clear_upstream_credential(&self) -> Result<()> {
        Ok(())
    }
}

/// Pure drift evaluation, split out of [`OpenClaw::status`] so all five states
/// are testable without a live engine.
///
/// `expected` is our proxy address from the persisted port (identity: a config
/// pointing here is ours even while routing is off); `running` is whether the
/// engine is actually up. The two are separate because "pointed at us but the
/// engine is down" is the dangerous state, not a cosmetic one - managed proxy
/// mode force-clears `no_proxy` and has no bypass list, so OpenClaw has *no*
/// egress at all until it is cleared. Reported as drift rather than Connected
/// so the master-off sweep still picks it up for disconnect.
///
/// Domain coverage is deliberately NOT a state here. An auth mode whose host is
/// not intercepted leaves this config perfectly healthy, and the remedy is a
/// switch this integration does not own, so reporting drift would mean a fault
/// that no re-connect can clear while `provider::reconcile_unmapped_tools`
/// retried one on every pass. `connect` prints [`coverage_note`] instead, and the
/// domain's own ledger row carries its state from then on.
fn compute_status(
    configured: &str,
    enabled: bool,
    expected: Option<&str>,
    running: bool,
) -> Status {
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
    // A matching URL is not enough to be routed: `enabled` is what actually
    // switches managed proxy mode on, so without it OpenClaw ignores the URL
    // and reaches the provider directly. Checked separately from the URL
    // because reporting Connected on this state is precisely the silent bypass
    // that made the integration inert.
    if !enabled {
        return Status::Drifted(
            "OpenClaw has Gate's proxy URL but proxy.enabled is not true, so it ignores the \
             proxy and reaches providers directly -- reconnect OpenClaw to fix it"
                .into(),
        );
    }
    if !running {
        return Status::Drifted(format!(
            "the Gate proxy is not running, so OpenClaw has no route out ({configured:?} is a \
             dead address) -- turn the proxy on, or disconnect OpenClaw to restore it"
        ));
    }
    Status::Connected
}

/// What `connect` prints about Gate's view of this OpenClaw, or `None` when
/// there is nothing to act on: an API-key profile calls a host the OpenAI
/// provider switch already covers, and a subscription profile whose domain is
/// already on needs no advice.
///
/// Read-only, and that is the whole point - see the module docs. An unreadable
/// domains file prints the note: being told about a switch that is already on
/// costs a moment's confusion, while staying quiet about one that is off costs
/// the user their visibility without telling them.
fn coverage_note(mode: OpenAiAuthMode) -> Option<String> {
    if mode != OpenAiAuthMode::Bearer || chatgpt_domain_enabled() {
        return None;
    }
    Some(format!(
        "note: OpenClaw is logged into OpenAI with a ChatGPT subscription, so its model calls go \
         to chatgpt.com -- Gate only sees them while the {CHATGPT_DOMAIN_SLUG:?} domain is on \
         (its row is under OpenAI, or `gate-connect proxy domain {CHATGPT_DOMAIN_SLUG} on`)."
    ))
}

/// Whether the domain carrying subscription model calls is switched on.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn chatgpt_domain_enabled() -> bool {
    crate::proxy::config::load_domains()
        .map(|ds| {
            ds.iter()
                .any(|d| d.slug == CHATGPT_DOMAIN_SLUG && d.enabled)
        })
        .unwrap_or(false)
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn chatgpt_domain_enabled() -> bool {
    // No proxy subsystem, so no domain to be on - and nothing here ever
    // enabled one.
    false
}

/// Whether `proxy.enabled` is literally `true` on disk. Anything else - absent,
/// `false`, or a non-boolean - is off, matching OpenClaw's own
/// `config?.enabled !== true` test rather than a looser truthiness read.
fn proxy_is_enabled(settings: &Map<String, Value>) -> bool {
    settings
        .get("proxy")
        .and_then(|v| v.as_object())
        .and_then(|p| p.get("enabled"))
        == Some(&Value::Bool(true))
}

/// The `proxy.proxyUrl` currently on disk, if any.
fn current_proxy_url(settings: &Map<String, Value>) -> Option<&str> {
    settings
        .get("proxy")
        .and_then(|v| v.as_object())
        .and_then(|p| p.get("proxyUrl"))
        .and_then(|v| v.as_str())
}

/// Which credential OpenClaw's OpenAI auth profile carries. The two shapes talk
/// to different hosts, so they need different catalog domains intercepted: a
/// pasted key reaches `api.openai.com` (the `openai` domain, already covered by
/// the OpenAI provider switch), a bearer-style login reaches `chatgpt.com` (the
/// `chatgpt` domain, covered by nothing). Same split `integrations/codex.rs`
/// reads out of `~/.codex/auth.json`, except OpenClaw records it in
/// `openclaw.json` itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenAiAuthMode {
    /// `openclaw models auth login` against a ChatGPT subscription (`oauth`),
    /// or a static bearer pasted in its place (`token`).
    Bearer,
    /// A pasted `sk-...` key.
    ApiKey,
}

/// Modes in OpenClaw's `AuthProfileConfig` (openclaw: `src/config/types.auth.ts`
/// declares `api_key | aws-sdk | oauth | token`) that authenticate with a bearer
/// rather than a provider key.
///
/// `token` is in here even though only `oauth` is the ChatGPT-subscription
/// login, because the honest signal for "does this reach chatgpt.com" is the
/// model's api (`openai-chatgpt-responses`), not the auth mode - the mode only
/// correlates. Given that, the two errors are not symmetric: over-matching
/// intercepts a host the user never calls on a path (`/codex/responses`) they
/// never hit, while under-matching restores the silent tunnel this module
/// exists to close. So the doubtful mode is included.
const BEARER_MODES: &[&str] = &["oauth", "token"];

/// Read `auth.profiles.*` and report how OpenClaw authenticates to OpenAI.
///
/// Profile keys are `openai:<account>` - `openai:default` for a pasted key,
/// `openai:<email>` for a subscription login - so the key is not a reliable
/// discriminator; the `provider` / `mode` pair inside is. *Any* bearer-mode
/// OpenAI profile counts, because a user with both can pick either per agent
/// and the domain has to be on if either can reach chatgpt.com.
///
/// This is the same read OpenClaw's own doctor does (`hasConfigOAuthProfiles`
/// in `src/commands/doctor-auth-legacy-oauth.ts` walks `cfg.auth.profiles` and
/// tests `mode`), so `openclaw.json` is authoritative here and the
/// `auth-profiles.json` sidecar - which holds the secret and spells the field
/// `type` - does not need consulting.
///
/// Anything unreadable - no `auth` block, a non-object profile, an unfamiliar
/// mode string - falls through to `ApiKey`. That is the conservative answer
/// rather than the lenient one: it is the mode that enables no extra domain, so
/// a misread never starts intercepting a host the user isn't talking to.
fn openai_auth_mode(settings: &Map<String, Value>) -> OpenAiAuthMode {
    let bearer = settings
        .get("auth")
        .and_then(|v| v.as_object())
        .and_then(|a| a.get("profiles"))
        .and_then(|v| v.as_object())
        .is_some_and(|profiles| {
            profiles.values().any(|profile| {
                let field = |key: &str| {
                    profile
                        .as_object()
                        .and_then(|p| p.get(key))
                        .and_then(|v| v.as_str())
                };
                field("provider") == Some("openai")
                    && field("mode").is_some_and(|m| BEARER_MODES.contains(&m))
            })
        });
    if bearer {
        OpenAiAuthMode::Bearer
    } else {
        OpenAiAuthMode::ApiKey
    }
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

// --- file I/O ---------------------------------------------------------

fn settings_path() -> Result<PathBuf> {
    env::openclaw_config_path()
}

fn state_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join(STATE_FILENAME))
}

fn load_settings() -> Result<Option<Map<String, Value>>> {
    // openclaw.json is JSON5 (comments + trailing commas allowed).
    super::json_config::load_object_json5(&settings_path()?)
}

fn write_settings(settings: &Map<String, Value>) -> Result<()> {
    super::json_config::write_object(&settings_path()?, settings)
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

use super::json_config::ensure_object;

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
    fn connect_writes_both_proxy_keys_and_leaves_siblings_alone() {
        // The integration is two keys, and `enabled` is not optional garnish:
        // OpenClaw's startProxy returns early unless it is literally `true`, so
        // a URL on its own leaves the tool unrouted while everything here still
        // reports success. `loopbackMode` in particular must survive: its
        // default is what lets a configured local provider bypass the proxy,
        // and that is the local-endpoint protection this integration used to
        // hand-roll.
        let mut settings = Map::new();
        settings.insert(
            "proxy".to_string(),
            json!({ "loopbackMode": "gateway-only", "tls": { "caFile": "/user/ca.pem" } }),
        );
        settings.insert("models".to_string(), json!({ "providers": {} }));

        let proxy = ensure_object(&mut settings, "proxy");
        proxy.insert("enabled".to_string(), Value::Bool(true));
        proxy.insert(
            "proxyUrl".to_string(),
            Value::String("http://127.0.0.1:9977".into()),
        );

        let proxy = settings["proxy"].as_object().unwrap();
        assert_eq!(proxy["proxyUrl"], json!("http://127.0.0.1:9977"));
        // Literally `true`, not a truthy stand-in: OpenClaw tests `!== true`.
        assert_eq!(proxy["enabled"], json!(true));
        assert_eq!(proxy["loopbackMode"], json!("gateway-only"));
        assert_eq!(proxy["tls"], json!({ "caFile": "/user/ca.pem" }));
        // No provider was touched: no discovery, no baseUrl rewriting.
        assert_eq!(settings["models"], json!({ "providers": {} }));
    }

    #[test]
    fn proxy_is_enabled_matches_openclaws_own_strict_test() {
        // OpenClaw gates on `config?.enabled !== true`, so only the boolean
        // counts. A string "true" or a 1 would sail past a truthiness check
        // here and still leave the tool unrouted, which is the failure this
        // whole change exists to stop reporting as Connected.
        let enabled = |v: Value| {
            let mut s = Map::new();
            s.insert("proxy".to_string(), v);
            proxy_is_enabled(&s)
        };
        assert!(enabled(json!({ "enabled": true })));
        assert!(!enabled(json!({ "enabled": false })));
        assert!(!enabled(json!({ "enabled": "true" })));
        assert!(!enabled(json!({ "enabled": 1 })));
        assert!(!enabled(json!({ "proxyUrl": "http://127.0.0.1:9977" })));
        assert!(!proxy_is_enabled(&Map::new()));
    }

    #[test]
    fn compute_status_covers_the_five_states() {
        let ours = "http://127.0.0.1:9977";

        assert_eq!(
            compute_status(ours, true, Some(ours), true),
            Status::Connected
        );

        // Our URL, switch off. The config looks right and the tool is routing
        // nowhere - the exact state that shipped as Connected before, sending
        // traffic to the provider on the user's own key.
        match compute_status(ours, false, Some(ours), true) {
            Status::Drifted(m) => {
                assert!(m.contains("proxy.enabled"), "must name the key: {m}");
                assert!(m.contains("directly"), "must say where traffic goes: {m}");
            }
            other => panic!("expected drift, got {other:?}"),
        }

        // Pointed at us but the engine is down. This is the state that leaves
        // OpenClaw with no egress at all, so it must never read as Connected.
        match compute_status(ours, true, Some(ours), false) {
            Status::Drifted(m) => {
                assert!(m.contains("no route out"), "unexpected message: {m}");
                assert!(m.contains("disconnect"), "must offer a way out: {m}");
            }
            other => panic!("expected drift, got {other:?}"),
        }

        // Hand-edited to something else - including a real corporate proxy.
        match compute_status("http://proxy.corp.example:3128", true, Some(ours), true) {
            Status::Drifted(m) => assert!(m.contains("does not match"), "unexpected: {m}"),
            other => panic!("expected drift, got {other:?}"),
        }

        // No port ever bound.
        match compute_status(ours, true, None, false) {
            Status::Drifted(m) => assert!(m.contains("never bound"), "unexpected: {m}"),
            other => panic!("expected drift, got {other:?}"),
        }
    }

    #[test]
    fn domain_coverage_is_reported_but_never_drift() {
        // A subscription login whose domain is off is a real gap in what Gate
        // sees, and it is deliberately NOT a status: the config is healthy, and
        // the remedy is a switch this integration does not own, so drift would
        // mean a red pill no re-connect can clear while
        // `reconcile_unmapped_tools` retried one every pass. It rides a connect
        // note instead.
        let ours = "http://127.0.0.1:9977";
        assert_eq!(
            compute_status(ours, true, Some(ours), true),
            Status::Connected,
            "a healthy config is Connected regardless of the domain catalog"
        );

        // `coverage_note` reads the domain catalog through `app_support_dir()`, so
        // with no redirect in place it reads the *developer's* real one. Anyone who
        // has run Gate Connect with the chatgpt domain on had the note suppressed
        // and failed here, for reasons the code under test has nothing to do with.
        // The bare env this wants has to be built, not assumed.
        let note = {
            let _lock = crate::env::path_env_lock();
            let home =
                std::env::temp_dir().join(format!("gate-openclaw-note-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&home);
            let prev = std::env::var_os("GATE_CONNECT_TEST_HOME");
            std::env::set_var("GATE_CONNECT_TEST_HOME", &home);
            let note = coverage_note(OpenAiAuthMode::Bearer);
            match prev {
                Some(v) => std::env::set_var("GATE_CONNECT_TEST_HOME", v),
                None => std::env::remove_var("GATE_CONNECT_TEST_HOME"),
            }
            let _ = std::fs::remove_dir_all(&home);
            note
        }
        .expect("an empty home has no domains file, so the note applies");
        assert!(note.contains("chatgpt"), "must name the domain: {note}");
        assert!(
            note.contains("proxy domain chatgpt on"),
            "and carry the command that turns it on: {note}"
        );
        assert!(
            note.contains("row is under OpenAI"),
            "the switch is the user's, so point at where it lives: {note}"
        );

        // An API-key profile talks to api.openai.com, which the OpenAI provider
        // switch already covers. Nothing to say.
        assert_eq!(coverage_note(OpenAiAuthMode::ApiKey), None);
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

    #[test]
    fn oauth_and_api_key_openai_profiles_are_told_apart() {
        let mode = |v: Value| openai_auth_mode(v.as_object().unwrap());

        // Subscription login. The profile is keyed by account, so the `mode`
        // inside is what discriminates - not the key, which varies per user.
        assert_eq!(
            mode(json!({ "auth": { "profiles": {
                "openai:someone@example.com": { "provider": "openai", "mode": "oauth" }
            } } })),
            OpenAiAuthMode::Bearer
        );
        // Pasted key: the shape a real install writes alongside
        // `auth-profiles.json`.
        assert_eq!(
            mode(json!({ "auth": { "profiles": {
                "openai:default": { "provider": "openai", "mode": "api_key" }
            } } })),
            OpenAiAuthMode::ApiKey
        );
        // An oauth profile for some OTHER provider must not drag chatgpt.com
        // interception in behind it.
        assert_eq!(
            mode(json!({ "auth": { "profiles": {
                "anthropic:default": { "provider": "anthropic", "mode": "oauth" }
            } } })),
            OpenAiAuthMode::ApiKey
        );
        // Both configured. The agent picks per run, so the oauth one can still
        // reach chatgpt.com and the domain has to be on.
        assert_eq!(
            mode(json!({ "auth": { "profiles": {
                "openai:default": { "provider": "openai", "mode": "api_key" },
                "openai:someone@example.com": { "provider": "openai", "mode": "oauth" }
            } } })),
            OpenAiAuthMode::Bearer
        );
        // No auth block at all - every config written before the user ran
        // `openclaw models auth login`.
        assert_eq!(
            mode(json!({ "proxy": { "enabled": true } })),
            OpenAiAuthMode::ApiKey
        );
        // Junk in the profile slot must not panic or read as bearer.
        assert_eq!(
            mode(json!({ "auth": { "profiles": { "openai:default": "oauth" } } })),
            OpenAiAuthMode::ApiKey
        );
        // `token` counts too. Only `oauth` is the subscription login, but the
        // honest signal is the model's api, not the auth mode, and the two
        // errors are not symmetric - see BEARER_MODES.
        assert_eq!(
            mode(json!({ "auth": { "profiles": {
                "openai:default": { "provider": "openai", "mode": "token" }
            } } })),
            OpenAiAuthMode::Bearer
        );
        // The other two modes openclaw declares stay on the API-key side.
        for m in ["api_key", "aws-sdk"] {
            assert_eq!(
                mode(json!({ "auth": { "profiles": {
                    "openai:default": { "provider": "openai", "mode": m }
                } } })),
                OpenAiAuthMode::ApiKey,
                "mode {m:?} must not pull in chatgpt.com"
            );
        }
    }

    #[test]
    fn the_chatgpt_domain_is_what_actually_routes_oauth_mode_openclaw() {
        use crate::proxy::{decide, default_domains, should_intercept_host, Decision, ProxyDomain};

        // The URL OpenClaw builds in subscription mode: openclaw's
        // `resolveCodexUrl` over DEFAULT_CODEX_BASE_URL
        // (packages/ai/src/providers/openai-chatgpt-responses.ts).
        const MODEL_CALL: &str = "/backend-api/codex/responses";

        let catalog = |on: &[&str]| -> Vec<ProxyDomain> {
            default_domains()
                .into_iter()
                .map(|mut d| {
                    d.enabled = on.contains(&d.slug.as_str());
                    d
                })
                .collect()
        };

        // Why the note exists: with only what the OpenAI provider switch
        // enables, no domain claims chatgpt.com, so the CONNECT is
        // blind-tunnelled and Gate never sees the request. Nothing is broken for
        // the user - OpenClaw still reaches its provider - but Gate is blind to
        // it, and only the `chatgpt` row can change that.
        assert!(!should_intercept_host(&catalog(&["openai"]), "chatgpt.com"));

        // With that row on: intercepted, and rewritten to the gateway carrying
        // the upstream Gate reassembles back into the real URL. `decide` and
        // `apply_rewrite` both strip the `/backend-api` the upstream already
        // carries, so the entry's relay-shaped split works unchanged on the MITM
        // route OpenClaw takes.
        let routed = catalog(&[CHATGPT_DOMAIN_SLUG]);
        assert!(should_intercept_host(&routed, "chatgpt.com"));
        assert_eq!(
            decide(&routed, "chatgpt.com", MODEL_CALL),
            Decision::Rewrite {
                upstream_url: "https://chatgpt.com/backend-api".into()
            }
        );

        // And with the neighbouring row on as well - a state the ledger now lets
        // the user reach, since both are switches under OpenAI. `decide` gives
        // every enabled entry a look, so `chatgpt-apps` no longer swallows the
        // model call it deliberately does not claim.
        assert_eq!(
            decide(
                &catalog(&["chatgpt-apps", CHATGPT_DOMAIN_SLUG]),
                "chatgpt.com",
                MODEL_CALL
            ),
            Decision::Rewrite {
                upstream_url: "https://chatgpt.com/backend-api".into()
            }
        );
    }
}
