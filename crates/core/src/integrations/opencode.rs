//! OpenCode integration.
//!
//! Unlike Claude Code / Codex / Cowork - which each target one upstream
//! provider - OpenCode is multi-provider by design. Users authenticate
//! to OpenAI, Anthropic, OpenRouter, etc. via `opencode auth login
//! <provider>` and each provider's request goes straight to its native
//! endpoint. Gate Connect's job here is purely to *intercept* those
//! existing requests: redirect them through Constellation Gate and add
//! the two Gate identification headers. The user's existing upstream
//! auth (API key from auth.json, OAuth bearer, etc.) is untouched -
//! Gate is a pure passthrough on `Authorization` / `x-api-key`.
//!
//! Mechanism: opencode.json supports per-provider option overrides via
//! deep merge (see <https://opencode.ai/docs/config/>). For each
//! well-known provider the user already has configured, we write:
//!
//! ```json
//! "provider": {
//!   "<id>": {
//!     "options": {
//!       "baseURL": "http://127.0.0.1:<relay-port>/__gate/t/opencode/<slug><client-path>"
//!     }
//!   }
//! }
//! ```
//!
//! That single value is the entire write. The `/__gate/t/opencode` marker names
//! the tool to the relay, which is what lets `x-gate-client` be read off the
//! route we wrote rather than guessed from OpenCode's `User-Agent`; see
//! `proxy::relay`'s `TOOL_PATH_PREFIX`. The rest comes from
//! [`crate::proxy::resolve_endpoint`]: `<slug>` names the catalog domain, which
//! is how the relay knows where to forward, and `<client-path>` is whatever sits
//! between the upstream host and the SDK's own suffix - `/v1` for Anthropic and
//! OpenAI, `/api/v1` for OpenRouter, `/zen/v1` and `/zen/go/v1` for OpenCode Zen
//! and Go, since Gate forwards the request path verbatim.
//!
//! The base URL points at the loopback reverse-proxy relay
//! ([`crate::proxy::relay`]), not the gateway: the relay injects the live Gate
//! credential *and* the upstream hint per request, so **neither a credential nor
//! a header is written to opencode.json**. `options.headers` is not in
//! OpenCode's config schema, so not writing it also drops a dependency on an
//! undocumented key - the key is never touched at all.
//!
//! The provider's `apiKey` , its model list, and any
//! other options the user set survive the merge. OpenCode keeps using
//! the original provider's npm package (@ai-sdk/anthropic for anthropic,
//! @ai-sdk/openai for openai, etc.), so SDK-specific wire formats stay
//! correct - Gate just sees whatever the SDK sends and forwards.
//!
//! Discovery: a provider is gated if EITHER it appears in opencode.json's
//! provider map OR it has an entry in `~/.local/share/opencode/auth.json`.
//! That captures both built-in providers the user has logged into and
//! provider blocks the user added by hand. Local-only providers (e.g.
//! `llamacpp`) and user-custom gateway shapes (e.g. their own `gateway`
//! provider) are not in our well-known list and stay untouched.
//!
//! An OpenCode that names no provider at all - a fresh Desktop install, which
//! sends Zen's free models anonymously - gets Anthropic and OpenAI added on a
//! pay-as-you-go account, billed through Gate, and is refused under BYOK. Zen's
//! free tier is never added: it refuses traffic routed through Gate. See
//! `seed_providers`. The sidecar marks what was added, and every connect takes
//! it back out before deciding again.
//!
//! **Gate models** are a provider of Gate's own, `provider.gate-connect`, beside
//! the user's: the AI SDK's OpenAI-compatible package pointed at the relay's
//! Gate models route, the chosen set under `models`, and the placeholder key the
//! relay strips. The top-level `model` names one of them. Gate serves that route
//! on the organization's credits whatever the billing mode, so an OpenCode on
//! Gate models needs no provider of the user's and gets no PAYG seeds: the Gate
//! provider is already what the seeds are for. See `apply_gate_models`.
//!
//! State tracking: OpenCode's JSON schema is strict
//! (`additionalProperties: false`), so we can't stash markers inside
//! opencode.json. Snapshots of each gated provider's original
//! `baseURL` + `headers` live in a sidecar at
//! `<app_support_dir>/opencode-state.json` so disconnect restores the
//! file byte-equivalent.

//! **Config granularity: per process.** Measured 2026-09-18 on OpenCode
//! 1.18.27, driving a headless `opencode serve` against two loopback listeners
//! with a custom provider whose `baseURL` was repointed underneath it. A second
//! message on the same session still went to the original address. So a running
//! OpenCode has to be restarted; `opencode run` is a fresh process and needs
//! nothing.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crate::env;
use crate::integrations::binaries;
use crate::integrations::precedence::Override;
use crate::registry::{ConnectInput, GateModelState, Integration, Mechanism, Status, ToolId};

const UPSTREAM_PROVIDER_NAME: &str = "your existing providers";
const DEFAULT_UPSTREAM_URL: &str = "https://api.anthropic.com";

/// Sidecar state file. OpenCode's JSON schema rejects unknown top-level
/// keys, so we can't keep state inside opencode.json itself.
const STATE_FILENAME: &str = "opencode-state.json";

/// Providers we know how to redirect through Gate.
///
/// `endpoint` is the provider's **canonical endpoint** - the URL OpenCode would
/// call if Gate were not in the picture, including the `/v1`-style suffix.
/// [`crate::proxy::resolve_endpoint`] splits it into the upstream Gate forwards
/// to and the path that must stay in `baseURL`; nothing here hardcodes that
/// split, because doing it by hand is what broke OpenRouter.
struct KnownProvider {
    id: &'static str,
    endpoint: &'static str,
}

/// The list is intentionally short. Adding a new entry means knowing the
/// canonical endpoint and that the upstream speaks a wire format Gate can
/// forward. Users with custom providers can still wire them up by hand.
///
/// Every entry must resolve against the proxy catalog - an endpoint no domain
/// covers is skipped at connect time rather than repointed, because the relay
/// would 403 every request. `known_provider_endpoints_all_resolve_against_the_catalog`
/// holds the whole list to that.
const KNOWN_PROVIDERS: &[KnownProvider] = &[
    KnownProvider {
        id: "anthropic",
        endpoint: "https://api.anthropic.com/v1",
    },
    KnownProvider {
        id: "openai",
        endpoint: "https://api.openai.com/v1",
    },
    KnownProvider {
        id: "openrouter",
        endpoint: "https://openrouter.ai/api/v1",
    },
    // OpenCode Zen - opencode.ai/zen/v1/{chat/completions,responses,messages,models}
    KnownProvider {
        id: "opencode",
        endpoint: "https://opencode.ai/zen/v1",
    },
    // OpenCode Go - opencode.ai/zen/go/v1/{chat/completions,messages}
    KnownProvider {
        id: "opencode-go",
        endpoint: "https://opencode.ai/zen/go/v1",
    },
];

/// Skip a provider if its currently-configured `baseURL` looks
/// non-public - localhost / loopback / private network / plain HTTP.
/// Protects against the edge case where someone names a local OpenAI-
/// compatible server `openai` and would otherwise have their endpoint
/// silently swapped to the Gate URL. `llamacpp`, `gateway`, and any
/// other ID that's not in `KNOWN_PROVIDERS` is already untouched by
/// design - this guard exists only for collisions inside the allowlist.
fn looks_local(base_url: &str) -> bool {
    let lc = base_url.trim().to_ascii_lowercase();
    if lc.is_empty() {
        return false;
    }
    if !lc.starts_with("https://") {
        // http://, ws://, plain hostname, file:// - all not the public
        // cloud endpoint Gate expects to forward to. Treat as local.
        return true;
    }
    // Strip scheme to get host[:port]/...
    let rest = &lc["https://".len()..];
    let authority = rest.split('/').next().unwrap_or("");
    // Handle bracketed IPv6 literals like `[fd12::1]:443` so the colon split
    // below doesn't mangle the address into `[fd12`.
    let host = authority
        .strip_prefix('[')
        .and_then(|a| a.split(']').next())
        .unwrap_or_else(|| authority.split(':').next().unwrap_or(""));
    matches!(
        host,
        "localhost" | "127.0.0.1" | "0.0.0.0" | "::1" | "metadata.google.internal"
    ) || host.ends_with(".local")
        || host.ends_with(".lan")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("169.254.")
        || host.ends_with(".internal")
        || (host.contains(':')
            && (host.starts_with("fc") || host.starts_with("fd") || host.starts_with("fe80")))
        || (host.starts_with("172.")
            && host
                .split('.')
                .nth(1)
                .and_then(|s| s.parse::<u8>().ok())
                .map(|n| (16..=31).contains(&n))
                .unwrap_or(false))
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    /// One entry per provider we modified, keyed by provider ID. Each
    /// entry records exactly what `options.baseURL` and `options.headers`
    /// looked like *before* we touched it, so disconnect can restore
    /// byte-identical values . BTreeMap so
    /// the on-disk JSON stays deterministic.
    #[serde(default)]
    providers: BTreeMap<String, ProviderSnapshot>,
    /// What Gate models changed in `opencode.json`, while they are applied.
    /// `None` is OpenCode on its own models as far as Gate is concerned.
    #[serde(default)]
    gate_models: Option<GateModels>,
}

impl State {
    /// Nothing of Gate's left to restore: the sidecar can go.
    fn is_empty(&self) -> bool {
        self.providers.is_empty() && self.gate_models.is_none()
    }
}

/// The record behind Gate models in `opencode.json`: the set Gate wrote, and
/// what it replaced, so going back to OpenCode's own models restores the user's
/// values rather than a guess at them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct GateModels {
    /// The set Gate last wrote, in the user's order.
    written: Vec<String>,
    /// The top-level `model` before Gate models, in whichever form the user
    /// wrote it. `None` for a file that named no model.
    #[serde(default)]
    previous_model: Option<Value>,
    /// Whether the file had a `provider` map before Gate models, so taking
    /// Gate's provider out does not leave an empty one the user never wrote.
    #[serde(default)]
    provider_map_existed: bool,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct ProviderSnapshot {
    /// Whether the provider entry existed at all before connect. If
    /// false, disconnect must remove the entry entirely.
    #[serde(default)]
    provider_existed: bool,
    /// Whether the provider had an `options` object before connect.
    /// If false, disconnect drops the whole `options` block (otherwise
    /// we'd leave an empty `options: {}` orphan).
    #[serde(default)]
    options_existed: bool,
    /// Original `options.baseURL`. `None` means the field was absent.
    #[serde(default)]
    previous_base_url: Option<Value>,
    /// Original `options.headers`. `None` means the field was absent.
    #[serde(default)]
    previous_headers: Option<Value>,
    /// Connect added this provider itself, because OpenCode named none of its
    /// own: see [`seed_providers`]. Every connect takes these back out before
    /// deciding again, so a later login or a billing-mode switch replaces them.
    #[serde(default)]
    seeded: bool,
    /// Connect also wrote [`PAYG_PLACEHOLDER_API_KEY`] as `options.apiKey`.
    #[serde(default)]
    placeholder_key: bool,
}

/// What a pay-as-you-go account adds to an OpenCode that names no provider:
/// the PAYG-eligible upstreams, billed through Gate. OpenRouter is eligible
/// too and left out, because it would fill the model picker with hundreds of
/// entries the user did not ask for.
const PAYG_SEED_PROVIDERS: &[&str] = &["anthropic", "openai"];

/// `options.apiKey` for a [`PAYG_SEED_PROVIDERS`] entry. OpenCode only lists a
/// provider it holds a key for, and the AI SDK refuses to send without one, so
/// something has to be there. It is not a credential: the relay strips the
/// tool's own key on a pay-as-you-go request. The value says so to anyone who
/// reads the file.
const PAYG_PLACEHOLDER_API_KEY: &str = "billed-through-gate";

/// Likely install locations of the `opencode` binary. Detection falls
/// back to the config dir if none match, so npm/Bun/Volta layouts that
/// land binaries in per-user prefixes still register as installed.
/// Windows has no fixed locations, so it adds per-user ones at runtime through
/// [`windows_install_paths`].
#[cfg(target_os = "macos")]
const CLI_BIN_PATHS: &[&str] = &["/opt/homebrew/bin/opencode", "/usr/local/bin/opencode"];
#[cfg(all(unix, not(target_os = "macos")))]
const CLI_BIN_PATHS: &[&str] = &["/usr/local/bin/opencode", "/usr/bin/opencode"];
#[cfg(windows)]
const CLI_BIN_PATHS: &[&str] = &[];

/// The per-user install locations [`env::opencode_cli_candidates`] names, or
/// none off Windows. Checked by `detect` only, never by `binary()`: one of them
/// is the Desktop app's GUI executable, which a `--version` probe would launch.
///
/// They also cover a gap `PATH` cannot: a Gate Connect started before the user
/// ran `npm i -g` or `scoop install` holds the `PATH` it was launched with.
fn windows_install_paths() -> Vec<PathBuf> {
    #[cfg(windows)]
    return env::opencode_cli_candidates().unwrap_or_default();
    #[cfg(not(windows))]
    Vec::new()
}

pub struct OpenCode;

impl Integration for OpenCode {
    fn id(&self) -> ToolId {
        ToolId::OpenCode
    }

    fn display_name(&self) -> &'static str {
        "OpenCode"
    }

    fn client(&self) -> crate::taxonomy::Client {
        crate::taxonomy::Client::OpenCode
    }

    fn binary(&self) -> (&'static [&'static str], &'static [&'static str]) {
        #[cfg(windows)]
        const NAMES: &[&str] = &["opencode.exe", "opencode.cmd", "opencode.bat", "opencode"];
        #[cfg(not(windows))]
        const NAMES: &[&str] = &["opencode"];
        (CLI_BIN_PATHS, NAMES)
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
        // The sidecar under app support is deliberately absent: it changes
        // because `connect` wrote it, and that path already refreshes.
        let mut paths: Vec<PathBuf> = CLI_BIN_PATHS.iter().map(PathBuf::from).collect();
        paths.extend(windows_install_paths());
        paths.extend(env::opencode_config_dir());
        paths.extend(settings_path());
        paths.extend(env::opencode_auth_path());
        paths
    }

    fn detect(&self) -> Result<bool> {
        // The packaged paths, then PATH, then the bin directories a login
        // shell adds and a GUI process does not inherit - see
        // `integrations::binaries` - and on Windows the per-user install
        // locations of `windows_install_paths`. Failing all of them, a config
        // file or a login naming a provider Gate routes still counts: see
        // `has_routable_setup`.
        let (well_known, names) = self.binary();
        if binaries::resolve_binary(well_known, names).is_some()
            || windows_install_paths().iter().any(|p| p.is_file())
        {
            return Ok(true);
        }
        has_routable_setup()
    }

    fn config_is_managed(&self) -> Result<bool> {
        // Two-part marker, since opencode.json's schema rejects unknown
        // top-level keys so we cannot leave one in the file itself. The sidecar
        // only exists because connect() wrote it, but on its own it can't tell
        // our stale write apart from a baseURL the user has since repointed by
        // hand - so also require that at least one recorded provider still aims
        // at loopback, which is only ever us.
        let Some(state) = load_state()? else {
            return Ok(false);
        };
        let settings = load_settings()?.unwrap_or_default();
        let gate_provider = state.gate_models.is_some().then_some(GATE_PROVIDER);
        Ok(state
            .providers
            .keys()
            .map(String::as_str)
            .chain(gate_provider)
            .any(|provider_id| base_url_of(&settings, provider_id).is_some_and(looks_local)))
    }

    /// `provider.<id>.options.baseURL` names the loopback relay, which dies with the engine.
    fn mechanism(&self) -> Mechanism {
        Mechanism::Relay
    }

    fn supports_gate_models(&self) -> bool {
        true
    }

    /// Gate's provider and the `model` naming it, and nothing else: the
    /// providers Gate routes stay routed, and whatever the user picked stays.
    fn leave_gate_models(&self, _input: &ConnectInput) -> Result<()> {
        let Some(mut state) = load_state()? else {
            return Ok(());
        };
        if state.gate_models.is_none() {
            return Ok(());
        }
        let mut settings = load_settings()?.unwrap_or_default();
        revert_gate_models(&mut settings, &mut state);
        save_settings_and_state(&settings, &state)
    }

    fn gate_model_state(&self) -> Result<GateModelState> {
        let Some(gm) = load_state()?.and_then(|s| s.gate_models) else {
            return Ok(GateModelState::NotApplied);
        };
        let settings = load_settings()?.unwrap_or_default();
        Ok(gate_model_state_of(&settings, &gm))
    }

    /// The PAYG seeds: their key is [`PAYG_PLACEHOLDER_API_KEY`], which only
    /// Gate makes good. Every other provider sends the user's own key.
    fn relies_on_gate_credential(&self) -> bool {
        let (Ok(Some(state)), Ok(settings)) = (load_state(), load_settings()) else {
            return false;
        };
        !placeholder_providers(&settings.unwrap_or_default(), &state).is_empty()
    }

    fn configured_addresses(&self) -> Result<Vec<String>> {
        // Every provider's `baseURL`, not only the ones the sidecar says we
        // wrote: a base URL the user has since repointed is one nothing of
        // ours is bound to, and the address rule will say so.
        Ok(load_settings()?
            .and_then(|s| s.get("provider")?.as_object().cloned())
            .map(|providers| {
                providers
                    .values()
                    .filter_map(|p| {
                        p.get("options")?
                            .get("baseURL")?
                            .as_str()
                            .map(str::to_owned)
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    fn status(&self) -> Result<Status> {
        if !self.detect()? {
            prune_orphaned_state()?;
            return Ok(Status::NotInstalled);
        }
        // Connected = sidecar state exists AND at least one provider
        // currently carries Gate's two headers. If state exists but no
        // provider has our headers anymore, the user edited opencode.json
        // by hand - surface that as drift.
        let Some(state) = load_state()? else {
            return Ok(Status::Detected);
        };
        if state.is_empty() {
            return Ok(Status::Detected);
        }
        // The stored choice and `opencode.json` must agree about Gate models,
        // or the row would read Protected over an OpenCode whose Gate provider
        // the route refuses (the choice went back to App default and the
        // fix-up failed), or over one whose chosen models never reached its
        // config. Hermes' rule, for the same reason.
        let chosen = crate::preferences::gate_models_for(ToolId::OpenCode.slug()).is_some();
        match (state.gate_models.is_some(), chosen) {
            (true, false) => {
                return Ok(Status::Drifted(
                    "OpenCode is still on Gate models in opencode.json although Gate Connect \
                     has it on its own models, so their requests are refused"
                        .into(),
                ))
            }
            (false, true) => {
                return Ok(Status::Drifted(
                    "the Gate models chosen for OpenCode are not in its opencode.json yet".into(),
                ))
            }
            _ => {}
        }

        let settings = load_settings()?.unwrap_or_default();

        let expected_base = match crate::proxy::relay_base_url() {
            Some(u) => u,
            None => {
                return Ok(Status::Drifted(
                    "the Gate proxy has not been enabled yet - turn it on to route OpenCode".into(),
                ));
            }
        };

        let mut healthy = 0;
        let mut drifted = Vec::new();
        for provider_id in state.providers.keys() {
            let block = settings
                .get("provider")
                .and_then(|v| v.as_object())
                .and_then(|m| m.get(provider_id))
                .and_then(|v| v.as_object());
            let options = block
                .and_then(|b| b.get("options"))
                .and_then(|v| v.as_object());
            let base_url = options
                .and_then(|o| o.get("baseURL"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // The baseURL is the whole test now: it carries the relay origin,
            // the catalog slug the relay routes on, and the provider's own path.
            // No credential and no header is written, so there is nothing else
            // to check. Per provider, because OpenRouter's path is `/api/v1`
            // where Anthropic's is `/v1`.
            let expected = expected_base_url(provider_id, &expected_base);
            if expected.as_deref() == Some(base_url) {
                healthy += 1;
            } else {
                drifted.push(provider_id.clone());
            }
        }
        // Gate's own provider, on the Gate models route rather than a catalog
        // slug, at the same relay.
        if state.gate_models.is_some() {
            let expected =
                crate::proxy::gate_served::relay_base_url(&expected_base, ToolId::OpenCode);
            if base_url_of(&settings, GATE_PROVIDER) == Some(expected.as_str()) {
                healthy += 1;
            } else {
                drifted.push(GATE_PROVIDER.to_string());
            }
        }
        // Neither message blames the user for the edit. A hand edit is one cause
        // of this state and the loud one, but the other is a base URL of ours
        // that has gone stale - a new relay port, or the tool marker the URL now
        // carries - and that one is ours, arrives for everybody at once on an
        // upgrade, and is repaired without the user doing anything. Saying
        // "edited by hand" there is both wrong and alarming. The first message
        // also used to name headers, which this integration stopped writing when
        // the baseURL became the whole test.
        if healthy == 0 {
            return Ok(Status::Drifted(format!(
                "no providers point at the Gate relay; expected: {}",
                drifted.join(", ")
            )));
        }
        // Liveness and routing, the same pair Codex checks and for the same
        // reason: the identity check above compares against the *persisted*
        // relay port, which survives restarts precisely so configs stay valid,
        // so on its own it reads Connected while OpenCode dials a dead port -
        // and, with the engine parked, while it reaches its providers directly.
        // This was invisible until routing-off stopped sweeping OpenCode to
        // Detected; keeping the config is what made it the steady state. Left
        // out, the two relay tools disagreed with each other about one parked
        // engine.
        // Liveness and interception, from the relay itself - see Codex's
        // equivalent for why identity alone is not enough and why this is a
        // measurement rather than the user's stored intent. Left out entirely
        // until recently, which is how the two relay tools came to disagree
        // about one parked engine.
        let Some(report) = crate::proxy::relay_report() else {
            return Ok(Status::Drifted(format!(
                "the Gate proxy is not running, so OpenCode cannot reach its providers \
                 ({expected_base:?} is a dead address) - turn the proxy on, or disconnect \
                 OpenCode to restore it"
            )));
        };
        if !report.intercepting {
            return Ok(Status::Drifted(format!(
                "routing is off, so OpenCode reaches its providers directly through \
                 {expected_base:?} rather than through Gate - turn routing on to route it"
            )));
        }
        if !drifted.is_empty() {
            return Ok(Status::Drifted(format!(
                "some providers no longer point at the Gate relay: {}",
                drifted.join(", ")
            )));
        }
        // A PAYG seed's key is a placeholder only the PAYG relay strips. Under
        // BYOK it goes upstream and is refused, so the providers Gate added for
        // PAYG have to come out - a reconnect does that through `unseed`.
        let placeholders = placeholder_providers(&settings, &state);
        if !placeholders.is_empty()
            && crate::account::billing_mode().unwrap_or_default()
                == crate::account::BillingMode::Byok
        {
            return Ok(Status::Drifted(format!(
                "Gate added {} for pay-as-you-go, which this account no longer uses - \
                 reconnect OpenCode to remove them",
                placeholders.join(", ")
            )));
        }
        // Our redirect is in the file we are allowed to write. OpenCode merges
        // five more layers over that one, and two of them are visible from here
        // (AG-674).
        if let Some(o) = overriding_layer(state.providers.keys(), &expected_base) {
            return Ok(o.into_status());
        }
        Ok(Status::Connected)
    }

    fn connect(&self, input: &ConnectInput) -> Result<()> {
        if !self.detect()? {
            anyhow::bail!(
                "OpenCode is not installed on this machine - install it from https://opencode.ai first"
            );
        }
        let relay_base_url = input.relay_base_url.as_deref().context(
            "the Gate proxy relay is not running - enable the proxy before connecting OpenCode",
        )?;

        let mut settings = load_settings()?.unwrap_or_default();
        let auth = load_opencode_auth().unwrap_or_default();
        // Load any existing state - re-connects must preserve the
        // *original* snapshots, not overwrite them with our intermediate
        // values from the previous connect.
        let mut state = load_state()?.unwrap_or_default();

        // Gate models are settled before anything else. Drift first: a config
        // the user moved off them is theirs, so no Gate models apply.
        if let Some(gm) = &state.gate_models {
            if matches!(
                gate_model_state_of(&settings, gm),
                GateModelState::Drifted { .. }
            ) {
                crate::preferences::fall_back_to_tool_model(ToolId::OpenCode.slug())?;
            }
        }
        let gate_ids = crate::preferences::gate_models_for(ToolId::OpenCode.slug());

        // Providers an earlier connect added on its own come out first, so the
        // choice below sees only what the user has. Left in, they would read as
        // the user's: a login made since would keep them beside it, and a
        // switch to BYOK would keep a placeholder key it cannot send.
        let mut changed = unseed(&mut settings, &mut state);
        // Gate models no longer chosen come out here too, so a refusal below
        // still takes them off the disk.
        if gate_ids.is_none() && state.gate_models.is_some() {
            revert_gate_models(&mut settings, &mut state);
            changed = true;
        }

        // Figure out which well-known providers to route through Gate:
        // anything the user has either pre-configured in opencode.json OR
        // logged into via `opencode auth login`. Failing that, the ones Gate
        // can bill for - see `seed_providers` - unless the user put OpenCode on
        // Gate models, whose provider already gives it models Gate pays for.
        let mut candidates = known_providers_in(&settings, &auth);
        let seeding = candidates.is_empty() && gate_ids.is_none();
        if seeding {
            candidates = seed_providers(input.billing_mode);
        }
        if candidates.is_empty() && gate_ids.is_none() {
            // BYOK with nothing of the user's: no key to route. What `unseed`
            // took out still has to reach the disk, or a switch from PAYG
            // would leave its placeholder keys going upstream.
            if changed {
                save_settings_and_state(&settings, &state)?;
            }
            anyhow::bail!(
                "Sign in to a provider in OpenCode first, or put it on Gate models, then turn \
                 this on again."
            );
        }

        // Local-protection guard: if a candidate has an existing
        // non-public `baseURL`, skip it - the user pointed it at a
        // private endpoint on purpose and we should not redirect that
        // traffic through Gate. Local providers like `llamacpp` and
        // user-custom gateways like `gateway` are already outside
        // KNOWN_PROVIDERS, so this only ever fires on collisions
        // inside the allowlist.
        //
        // Except when the private endpoint is *ours*. The relay lives on
        // `http://127.0.0.1:<port>`, which `looks_local` answers `true` for, so
        // without this exemption the guard fires on every already-connected
        // provider and a re-apply finds nothing left to target. That is not
        // theoretical: it is what a re-apply always is - the reconcile pass
        // rewriting a base URL of our own that has gone stale, which is exactly
        // how an existing install picks up a new relay port, or the tool marker
        // this file now writes.
        //
        // `is_relay_base_url` and not "is it in the sidecar", which would have
        // been the easy test and the wrong one: a provider we connected and the
        // user has since repointed at their own local server is in the sidecar
        // too, and exempting it would take their endpoint back. It asks whether
        // the string is one we could have written, at whatever port it names.
        let mut skipped_local: Vec<&str> = Vec::new();
        let targets: Vec<&KnownProvider> = candidates
            .into_iter()
            .filter(|p| {
                let existing = settings
                    .get("provider")
                    .and_then(|v| v.as_object())
                    .and_then(|m| m.get(p.id))
                    .and_then(|v| v.as_object())
                    .and_then(|b| b.get("options"))
                    .and_then(|v| v.as_object())
                    .and_then(|o| o.get("baseURL"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let ours = crate::proxy::resolve_endpoint(p.endpoint)
                    .is_some_and(|r| r.is_relay_base_url(existing, ToolId::OpenCode));
                if !existing.is_empty() && looks_local(existing) && !ours {
                    skipped_local.push(p.id);
                    false
                } else {
                    true
                }
            })
            .collect();

        // Only reachable when every provider the user has points at a local
        // endpoint: with none at all, connect has already returned above. Gate
        // models are still worth writing then; they need no provider of theirs.
        if targets.is_empty() && gate_ids.is_none() {
            if changed {
                save_settings_and_state(&settings, &state)?;
            }
            anyhow::bail!(
                "Every OpenCode provider Gate could route points at a local endpoint ({}), so there is nothing to route.",
                skipped_local.join(", ")
            );
        }
        if !skipped_local.is_empty() {
            eprintln!(
                "note: skipping providers that look local / self-hosted: {}. Their baseURL stays as-is. Disconnect-then-reconnect after pointing them at a public endpoint to route them via Gate.",
                skipped_local.join(", ")
            );
        }

        // Each provider gets its own baseURL: the path Gate must not swallow
        // differs per upstream (`/v1` for Anthropic and OpenAI, `/api/v1` for
        // OpenRouter), and the split between "upstream Gate forwards to" and
        // "path the client keeps" is decided in one place, by the catalog.
        let mut skipped_off_catalog: Vec<&str> = Vec::new();
        let mut applied = 0;
        for target in &targets {
            let provider_map = ensure_object(&mut settings, "provider");
            let Some(resolved) = crate::proxy::resolve_endpoint(target.endpoint) else {
                // No catalog domain covers this upstream, so the relay would
                // 403 every request. Leaving the config alone is strictly
                // better than repointing it at a dead end.
                skipped_off_catalog.push(target.id);
                continue;
            };
            let base_url = resolved.relay_base_url(relay_base_url, ToolId::OpenCode);
            apply_override(provider_map, &mut state, target, &base_url);
            if seeding {
                mark_seeded(provider_map, &mut state, target.id);
            }
            applied += 1;
        }

        if applied == 0 && gate_ids.is_none() {
            anyhow::bail!(
                "None of the configured OpenCode providers can route through Gate yet ({}). Gate has no upstream domain for them.",
                skipped_off_catalog.join(", ")
            );
        }

        // After the providers above, so the snapshot of whether `provider`
        // existed counts the ones Gate routes: taking Gate models out later
        // must not remove a map they still live in.
        if let Some(ids) = &gate_ids {
            apply_gate_models(&mut settings, &mut state, ids, relay_base_url)?;
        }

        save_state(&state)?;
        write_settings(&settings)
    }

    fn disconnect(&self) -> Result<()> {
        let Some(mut state) = load_state()? else {
            return Ok(());
        };
        let Some(mut settings) = load_settings()? else {
            remove_state()?;
            return Ok(());
        };

        // OpenCode's own `model` first, while the record says what it was.
        revert_gate_models(&mut settings, &mut state);
        if let Some(provider_map) = settings.get_mut("provider").and_then(|v| v.as_object_mut()) {
            for (provider_id, snapshot) in &state.providers {
                restore_provider(provider_map, provider_id, snapshot);
            }
            if provider_map.is_empty() {
                settings.remove("provider");
            }
        }

        if settings.is_empty() {
            crate::config_changes::remove(&env::opencode_config_path()?)?;
            remove_state()?;
            return Ok(());
        }
        write_settings(&settings)?;
        // Only drop the sidecar once the restored config is on disk: losing
        // it before a failed write would leave Gate headers in opencode.json
        // while status reports the tool as clean and re-disconnect no-ops.
        remove_state()
    }
}

/// Snapshot the provider's current state, then overwrite `options.baseURL`
/// (to the relay) and merge the non-secret `X-Gate-Upstream-Url` header into
/// `options.headers`. No Gate credential is written - the relay injects it live
/// per request. Existing `options.apiKey`, model lists, and any other fields
/// survive untouched.
fn apply_override(
    provider_map: &mut Map<String, Value>,
    state: &mut State,
    target: &KnownProvider,
    relay_base_url: &str,
) {
    // Only snapshot the first time we touch this provider. Re-connect
    // must preserve the user's original values, not record our own
    // intermediate state.
    if !state.providers.contains_key(target.id) {
        let block = provider_map.get(target.id).and_then(|v| v.as_object());
        let provider_existed = block.is_some();
        let options = block
            .and_then(|b| b.get("options"))
            .and_then(|v| v.as_object());
        let options_existed = options.is_some();
        let previous_base_url = options.and_then(|o| o.get("baseURL")).cloned();
        let previous_headers = options.and_then(|o| o.get("headers")).cloned();
        state.providers.insert(
            target.id.to_string(),
            ProviderSnapshot {
                provider_existed,
                options_existed,
                previous_base_url,
                previous_headers,
                ..ProviderSnapshot::default()
            },
        );
    }

    // Ensure provider.<id>.options exists, then patch it.
    let provider_entry = provider_map
        .entry(target.id.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    // A user could have set this provider to a non-object (e.g. a string);
    // coerce it so the `as_object_mut` below can never panic.
    if !provider_entry.is_object() {
        *provider_entry = Value::Object(Map::new());
    }
    let provider = provider_entry
        .as_object_mut()
        .expect("provider entry was just coerced to an object");
    let options_entry = provider
        .entry("options".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !options_entry.is_object() {
        *options_entry = Value::Object(Map::new());
    }
    let options = options_entry
        .as_object_mut()
        .expect("options entry was just coerced to an object");

    options.insert(
        "baseURL".to_string(),
        Value::String(relay_base_url.to_string()),
    );

    // `options.headers` is left alone entirely: the relay reads the upstream off
    // the slug segment in `baseURL` and injects the hint itself. That also drops
    // a dependency on a key absent from OpenCode's config schema.
}

/// Reverse `apply_override` for one provider. Restores baseURL / headers
/// to whatever the user had, and drops empty parents so the file doesn't
/// end up with orphaned `options: {}` or empty provider stubs.
fn restore_provider(
    provider_map: &mut Map<String, Value>,
    provider_id: &str,
    snapshot: &ProviderSnapshot,
) {
    if !snapshot.provider_existed {
        // We created the provider entry during connect. Remove it cleanly.
        provider_map.remove(provider_id);
        return;
    }

    let Some(provider) = provider_map
        .get_mut(provider_id)
        .and_then(|v| v.as_object_mut())
    else {
        return;
    };

    if !snapshot.options_existed {
        provider.remove("options");
        return;
    }

    let Some(options) = provider.get_mut("options").and_then(|v| v.as_object_mut()) else {
        return;
    };

    match &snapshot.previous_base_url {
        Some(v) => {
            options.insert("baseURL".to_string(), v.clone());
        }
        None => {
            options.remove("baseURL");
        }
    }
    match &snapshot.previous_headers {
        Some(v) => {
            options.insert("headers".to_string(), v.clone());
        }
        None => {
            options.remove("headers");
        }
    }
    if options.is_empty() {
        provider.remove("options");
    }
}

/// The `baseURL` connect writes for `provider_id`, given the relay's loopback
/// base. `None` when the provider is not one we redirect (unknown id, or an
/// endpoint no catalog domain covers) - `status` treats that as drift rather
/// than pretending it matches.
///
/// The suffix is per provider and comes from the catalog, not a constant: the
/// relay and Gate forward the path verbatim, so whatever sits between the
/// upstream host and the SDK's own suffix (`/v1` for Anthropic and OpenAI,
/// `/api/v1` for OpenRouter) has to live on this side.
fn expected_base_url(provider_id: &str, relay_base_url: &str) -> Option<String> {
    let target = KNOWN_PROVIDERS.iter().find(|p| p.id == provider_id)?;
    let resolved = crate::proxy::resolve_endpoint(target.endpoint)?;
    Some(resolved.relay_base_url(relay_base_url, ToolId::OpenCode))
}

// --- precedence -------------------------------------------------------

/// A layer OpenCode merges over the file Gate writes, when it repoints one of
/// the providers we redirect.
///
/// OpenCode merges its config from six sources, later winning, and scalars like
/// `options.baseURL` are overwritten rather than combined. Lowest to highest:
/// the remote org config, the global `opencode.json` Gate writes, the
/// `OPENCODE_CONFIG` file, the project's `./opencode.json`, its `.opencode/`
/// directory, and `OPENCODE_CONFIG_CONTENT` - with a managed `/etc/opencode`
/// above all of them.
///
/// Two of those are reachable from a windowed process: the managed file, whose
/// path does not move, and `OPENCODE_CONFIG_CONTENT` when it is in our own login
/// environment. `OPENCODE_CONFIG` needs no check here because
/// [`crate::env::opencode_config_path`] already writes there when it is set - we
/// are that layer, not under it.
///
/// **The project layer stays invisible, and it is the common one.** Per-repo
/// `opencode.json` is a documented, ordinary pattern (finding O1 in
/// `docs/harness-integration-validation.md`), and which repo the user is in is
/// not something this process knows. So this narrows the failure rather than
/// closing it: what it can see it now reports, and the rest is written down
/// instead of being quietly counted as connected.
fn overriding_layer<'a>(
    providers: impl Iterator<Item = &'a String>,
    relay_base_url: &str,
) -> Option<Override> {
    let providers: Vec<String> = providers.cloned().collect();

    let managed_path = env::opencode_managed_config_path();
    // An unreadable managed file is not evidence of an override. It is an
    // administrator's, we never write it, and failing `status` on somebody
    // else's malformed JSON would replace a wrong answer with a useless one.
    if let Some(managed) = super::json_config::load_object(&managed_path)
        .ok()
        .flatten()
    {
        if let Some(o) = override_in(
            &managed,
            &managed_path.display().to_string(),
            &providers,
            relay_base_url,
        ) {
            return Some(o);
        }
    }

    let inline = std::env::var("OPENCODE_CONFIG_CONTENT").ok()?;
    let parsed: Map<String, Value> = serde_json::from_str(&inline).ok()?;
    override_in(
        &parsed,
        "the OPENCODE_CONFIG_CONTENT environment variable",
        &providers,
        relay_base_url,
    )
}

/// One layer, read for the providers we redirect. Split from where the layer
/// comes from so both sources are the same check, and so a test can supply one
/// without writing to `/etc`.
fn override_in(
    layer: &Map<String, Value>,
    source: &str,
    providers: &[String],
    relay_base_url: &str,
) -> Option<Override> {
    let block = layer.get("provider").and_then(|v| v.as_object())?;
    for provider_id in providers {
        let Some(base_url) = block
            .get(provider_id)
            .and_then(|v| v.as_object())
            .and_then(|b| b.get("options"))
            .and_then(|v| v.as_object())
            .and_then(|o| o.get("baseURL"))
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        // The same value is not a conflict: a managed layer that happens to
        // name our relay leaves the traffic exactly where we put it.
        if expected_base_url(provider_id, relay_base_url).as_deref() == Some(base_url) {
            continue;
        }
        return Some(Override::new(
            source,
            format!(
                "points provider {provider_id:?} at {base_url:?}, which OpenCode merges over the \
                 Gate redirect in its global config"
            ),
        ));
    }
    None
}

// --- file I/O ---------------------------------------------------------

fn settings_path() -> Result<PathBuf> {
    env::opencode_config_path()
}

fn state_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join(STATE_FILENAME))
}

fn load_settings() -> Result<Option<Map<String, Value>>> {
    super::json_config::load_object(&settings_path()?)
}

fn write_settings(settings: &Map<String, Value>) -> Result<()> {
    super::json_config::write_object(&settings_path()?, settings)
}

/// Remove a sidecar whose `opencode.json` is gone.
///
/// Its snapshots are of that file, so without it they describe nothing: the
/// user deleted it or uninstalled OpenCode. `disconnect` always read it that
/// way, but detection no longer counts an empty config directory as an
/// install, so the sweeps stopped reaching `disconnect` to clear it. Left in
/// place, a reinstall read Drifted and its first connect kept the deleted
/// file's snapshots for a later disconnect to restore onto the new one.
///
/// Called from `status`, which is what runs when the file disappears
/// (`watch_paths` names it), and from `load_state`, so no reader can see one.
/// A config deleted and recreated while Gate is not running is not caught.
fn prune_orphaned_state() -> Result<()> {
    if !settings_path()?.exists() {
        remove_state()?;
    }
    Ok(())
}

fn load_state() -> Result<Option<State>> {
    prune_orphaned_state()?;
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
    let body = serde_json::to_string_pretty(state).context("serializing opencode-state.json")?;
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

fn load_opencode_auth() -> Result<Map<String, Value>> {
    Ok(super::json_config::load_object(&env::opencode_auth_path()?)?.unwrap_or_default())
}

use super::json_config::ensure_object;

/// The well-known providers named by the config's `provider` block or by the
/// login store: what `connect` would route, before the local-endpoint guard.
fn known_providers_in(
    settings: &Map<String, Value>,
    auth: &Map<String, Value>,
) -> Vec<&'static KnownProvider> {
    let configured = |id: &str| {
        settings
            .get("provider")
            .and_then(|v| v.as_object())
            .is_some_and(|m| m.contains_key(id))
            || auth.contains_key(id)
    };
    KNOWN_PROVIDERS
        .iter()
        .filter(|p| configured(p.id))
        .collect()
}

/// What `connect` routes for an OpenCode that names no provider of its own:
/// [`PAYG_SEED_PROVIDERS`] on pay-as-you-go, so the user has real models
/// without bringing a key - the case PAYG exists for - and nothing under BYOK,
/// where there is no key to route.
///
/// **Not OpenCode Zen**, though a fresh OpenCode Desktop uses it with no login.
/// Routed through Gate, its anonymous free tier answers "OpenCode's free tier
/// can only be used from within OpenCode" (measured on Desktop 2.0.29), so
/// adding it breaks a setup that worked. A Zen the user has signed in to is
/// theirs and is routed as before.
fn seed_providers(mode: crate::account::BillingMode) -> Vec<&'static KnownProvider> {
    if mode != crate::account::BillingMode::Payg {
        return Vec::new();
    }
    KNOWN_PROVIDERS
        .iter()
        .filter(|p| PAYG_SEED_PROVIDERS.contains(&p.id))
        .collect()
}

/// Record `provider_id`, just written by [`apply_override`], as one `connect`
/// added itself, and give it the key OpenCode needs to list it.
fn mark_seeded(provider_map: &mut Map<String, Value>, state: &mut State, provider_id: &str) {
    let Some(snapshot) = state.providers.get_mut(provider_id) else {
        return;
    };
    snapshot.seeded = true;
    snapshot.placeholder_key = true;
    if let Some(options) = provider_map
        .get_mut(provider_id)
        .and_then(|p| p.get_mut("options"))
        .and_then(|o| o.as_object_mut())
    {
        options.insert(
            "apiKey".to_string(),
            Value::String(PAYG_PLACEHOLDER_API_KEY.to_string()),
        );
    }
}

/// Take out every provider an earlier `connect` added itself, file and sidecar
/// both, and say whether there were any. A seed never existed before Gate wrote
/// it, so restoring it removes it.
fn unseed(settings: &mut Map<String, Value>, state: &mut State) -> bool {
    let seeded: Vec<String> = state
        .providers
        .iter()
        .filter(|(_, s)| s.seeded)
        .map(|(id, _)| id.clone())
        .collect();
    if seeded.is_empty() {
        return false;
    }
    if let Some(provider_map) = settings.get_mut("provider").and_then(|v| v.as_object_mut()) {
        for id in &seeded {
            restore_provider(provider_map, id, &state.providers[id]);
        }
        if provider_map.is_empty() {
            settings.remove("provider");
        }
    }
    for id in seeded {
        state.providers.remove(&id);
    }
    true
}

/// Write what [`unseed`] or [`revert_gate_models`] left, the way `disconnect`
/// does: a file or sidecar left empty is removed, not written.
fn save_settings_and_state(settings: &Map<String, Value>, state: &State) -> Result<()> {
    if settings.is_empty() {
        crate::config_changes::remove(&env::opencode_config_path()?)?;
    } else {
        write_settings(settings)?;
    }
    if state.is_empty() {
        remove_state()
    } else {
        save_state(state)
    }
}

/// `provider.<id>.options.baseURL`, if it is a string.
fn base_url_of<'a>(settings: &'a Map<String, Value>, provider_id: &str) -> Option<&'a str> {
    settings
        .get("provider")?
        .get(provider_id)?
        .get("options")?
        .get("baseURL")?
        .as_str()
}

// --- Gate models -------------------------------------------------------

/// The provider Gate adds for Gate models. Hermes' id and name for the same
/// thing, so the two read alike.
const GATE_PROVIDER: &str = "gate-connect";
const GATE_PROVIDER_NAME: &str = "Gate Connect";

/// The AI SDK package OpenCode loads for Gate's provider. Chat completions,
/// which the gateway serves for every model in its catalogue (Hermes reaches
/// the same set the same way), so one package covers Claude and GPT ids alike.
const GATE_PROVIDER_NPM: &str = "@ai-sdk/openai-compatible";

/// The top-level `model`, as `provider/model`, from either form OpenCode's
/// schema allows: the string, or `{ providerID, model }`.
fn configured_model(settings: &Map<String, Value>) -> Option<String> {
    match settings.get("model")? {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => Some(format!(
            "{}/{}",
            o.get("providerID")?.as_str()?,
            o.get("model")?.as_str()?
        )),
        _ => None,
    }
}

/// The Gate model id a `provider/model` reference names, if it names Gate's
/// provider. A `#variant` suffix is the same model.
fn gate_id_of(model: &str) -> Option<&str> {
    let model = model.split_once('#').map_or(model, |(m, _)| m);
    model.strip_prefix(GATE_PROVIDER)?.strip_prefix('/')
}

/// What `opencode.json` says, given what Gate wrote. Applied means `model`
/// names one of the set on Gate's provider, and the provider is on the Gate
/// models route; anything else is the user moving OpenCode's default off them.
///
/// The config is all this reads. A model picked in OpenCode's own picker lives
/// in its own state, not here, and does not change the default it starts with.
fn gate_model_state_of(settings: &Map<String, Value>, gm: &GateModels) -> GateModelState {
    let model = configured_model(settings);
    let on_route = base_url_of(settings, GATE_PROVIDER)
        .is_some_and(|u| crate::proxy::gate_served::is_relay_base_url(u, ToolId::OpenCode));
    match model.as_deref().and_then(gate_id_of) {
        Some(id) if on_route && gm.written.iter().any(|w| w == id) => GateModelState::Applied {
            model: id.to_string(),
        },
        _ => GateModelState::Drifted { model },
    }
}

/// Is `provider.gate-connect` one Gate wrote? Only Gate points a provider at
/// the Gate models route, so a block that does is ours even when the sidecar
/// that says so is gone.
fn gate_block_is_ours(settings: &Map<String, Value>) -> bool {
    base_url_of(settings, GATE_PROVIDER)
        .is_some_and(|u| crate::proxy::gate_served::is_relay_base_url(u, ToolId::OpenCode))
}

/// `models.<id>` for one Gate model: what the catalogue said about it when it
/// was picked, in the keys OpenCode's schema has for it. `limit` only with
/// both halves, which the schema requires together.
fn gate_model_entry(id: &str) -> Value {
    let meta = crate::preferences::gate_model_meta(id).unwrap_or_default();
    let mut entry = Map::new();
    if let Some(name) = meta.name {
        entry.insert("name".into(), Value::String(name));
    }
    if let (Some(context), Some(output)) = (meta.context_window, meta.max_tokens) {
        entry.insert(
            "limit".into(),
            serde_json::json!({ "context": context, "output": output }),
        );
    }
    Value::Object(entry)
}

/// The `provider.gate-connect` block: the Gate models route, the set under
/// `models` (which is what OpenCode's model picker lists for a provider it has
/// no catalogue entry for), and the placeholder key. The key is there because
/// OpenCode lists no provider without one; the relay strips it on this route.
fn gate_provider_block(relay: &str, ids: &[String]) -> Value {
    let models: Map<String, Value> = ids
        .iter()
        .map(|id| (id.clone(), gate_model_entry(id)))
        .collect();
    serde_json::json!({
        "npm": GATE_PROVIDER_NPM,
        "name": GATE_PROVIDER_NAME,
        "options": {
            "baseURL": crate::proxy::gate_served::relay_base_url(relay, ToolId::OpenCode),
            "apiKey": PAYG_PLACEHOLDER_API_KEY,
        },
        "models": models,
    })
}

/// Write the Gate models into `settings`, snapshotting OpenCode's own `model`
/// the first time.
///
/// The default is the first of the set, unless OpenCode is already on one of
/// the set and the set has not changed: then the user picked it, and a
/// reconnect must not take it back.
///
/// Refused, before anything is written, over a `gate-connect` provider of the
/// user's own: replacing it would lose it, and restoring it is not something
/// the record covers.
fn apply_gate_models(
    settings: &mut Map<String, Value>,
    state: &mut State,
    ids: &[String],
    relay: &str,
) -> Result<()> {
    let first = ids.first().context("a Gate model set cannot be empty")?;
    let current = configured_model(settings);
    let current_id = current.as_deref().and_then(gate_id_of);
    let default = match (&state.gate_models, current_id) {
        (Some(gm), Some(c)) if gm.written.as_slice() == ids && ids.iter().any(|i| i == c) => {
            c.to_string()
        }
        _ => first.clone(),
    };
    if state.gate_models.is_none() {
        let has_block = settings
            .get("provider")
            .and_then(|p| p.get(GATE_PROVIDER))
            .is_some();
        if has_block && !gate_block_is_ours(settings) {
            anyhow::bail!(
                "opencode.json already has a provider named \"{GATE_PROVIDER}\" that Gate did \
                 not write. Rename it in opencode.json, then choose Gate models again."
            );
        }
        // A `model` on Gate's provider with no record behind it is a Gate
        // write whose sidecar was lost: nothing of the user's to restore.
        let previous_model = settings
            .get("model")
            .filter(|_| current_id.is_none())
            .cloned();
        state.gate_models = Some(GateModels {
            written: Vec::new(),
            previous_model,
            provider_map_existed: settings.contains_key("provider"),
        });
    }
    ensure_object(settings, "provider")
        .insert(GATE_PROVIDER.into(), gate_provider_block(relay, ids));
    settings.insert(
        "model".into(),
        Value::String(format!("{GATE_PROVIDER}/{default}")),
    );
    if let Some(gm) = state.gate_models.as_mut() {
        gm.written = ids.to_vec();
    }
    Ok(())
}

/// Put OpenCode's own `model` back and take Gate's provider out, if Gate models
/// were applied.
///
/// Compare-and-restore for `model`: it is put back only while it still names
/// Gate's provider (or is gone). Anything else is the user's choice made after
/// Gate wrote, and it stays. The provider block is Gate's alone and always goes.
fn revert_gate_models(settings: &mut Map<String, Value>, state: &mut State) {
    let Some(gm) = state.gate_models.take() else {
        return;
    };
    let model_is_ours = configured_model(settings).is_none_or(|m| gate_id_of(&m).is_some());
    if model_is_ours {
        match gm.previous_model {
            Some(v) => {
                settings.insert("model".into(), v);
            }
            None => {
                settings.remove("model");
            }
        }
    }
    if let Some(map) = settings.get_mut("provider").and_then(|v| v.as_object_mut()) {
        map.remove(GATE_PROVIDER);
        if map.is_empty() && !gm.provider_map_existed {
            settings.remove("provider");
        }
    }
}

/// The PAYG seeds whose placeholder key is still in the file. A BYOK request
/// would send that key upstream, and with Gate gone nothing strips it, so these
/// are what makes OpenCode rely on Gate's credential.
fn placeholder_providers(settings: &Map<String, Value>, state: &State) -> Vec<String> {
    state
        .providers
        .iter()
        .filter(|(_, s)| s.placeholder_key)
        .filter(|(id, _)| {
            settings
                .get("provider")
                .and_then(|v| v.get(id.as_str()))
                .and_then(|b| b.get("options"))
                .and_then(|o| o.get("apiKey"))
                .and_then(|v| v.as_str())
                == Some(PAYG_PLACEHOLDER_API_KEY)
        })
        .map(|(id, _)| id.clone())
        .collect()
}

/// The fallback for an OpenCode whose binary Gate cannot find (a Volta, asdf
/// or npx install): the two files `connect` reads, and only when they name a
/// provider it could route.
///
/// **Not the config directory**, and **not the bare files either.** The
/// directory survives an uninstall, empty, and Gate's own disconnect leaves it.
/// The files can outlive an install too, holding `{}` or providers Gate does
/// not route, and reading their mere existence as an install drew an OpenCode
/// row on a machine without OpenCode (staging QA, 2026-09-30): its switch could
/// only fail, with "No supported OpenCode providers found". So the test is the
/// one `connect` applies, and a leftover that names nothing routable leaves
/// OpenCode under Not installed.
///
/// **A file Gate cannot parse still counts.** OpenCode reads `opencode.json`
/// as JSONC, so a commented config is a working install that strict JSON
/// rejects. Reading it as Not installed would hide the row with no error;
/// counting it keeps the row, and `connect` reports the parse failure.
fn has_routable_setup() -> Result<bool> {
    let (Ok(settings), Ok(auth)) = (load_settings(), load_opencode_auth()) else {
        return Ok(true);
    };
    Ok(!known_providers_in(&settings.unwrap_or_default(), &auth).is_empty())
}

/// The proxy domain for OpenCode's own Zen / Go host, `opencode.ai`. The
/// catalog's slug (`proxy::catalog`); a mismatch makes the cleanup below a
/// silent no-op, which `the_opencode_domain_is_switched_off_once_without_opencode`
/// would catch.
const DOMAIN_SLUG: &str = "opencode";

/// Present once the cleanup below has run on this install.
const DOMAIN_CLEANUP_MARKER: &str = "opencode-domain-cleaned";

/// Switch off the `opencode.ai` domain an older build left on, once, on a
/// machine without OpenCode. Returns whether it switched anything off.
///
/// An older build's OpenCode switch turned the domain on and nothing recorded
/// it, so it outlives the tool: the rail then draws an OpenCode row with a
/// routed domain and a switched-off tool, which reads "Not protected" with
/// "Partly protected: 1 of 2" beneath it.
///
/// **Once per install, not every launch.** Detection can miss a real install
/// (a project-level config, credentials in the environment), and a check that
/// ran every launch would then turn off inspection of that traffic every time,
/// and undo anybody who turned the domain back on. So the marker is written as
/// soon as the domain is seen off with OpenCode absent, and a later choice is
/// never revisited. While OpenCode is detected nothing is decided and the
/// marker waits.
///
/// Writes the flag rather than going through `ProxyManager::set_domain`: this
/// runs at startup before routing comes up, which reads the flag, and on Linux
/// `set_domain` adopts a running daemon, which leaves the enable after it with
/// nothing to do.
pub fn switch_off_orphaned_domain() -> Result<bool> {
    let marker = env::app_support_dir()?.join(DOMAIN_CLEANUP_MARKER);
    if marker.exists() || OpenCode.detect()? {
        return Ok(false);
    }
    let enabled = crate::proxy::config::load_domains()?
        .iter()
        .any(|d| d.slug == DOMAIN_SLUG && d.enabled);
    if enabled {
        crate::proxy::config::set_enabled(DOMAIN_SLUG, false)?;
        crate::logging::log(
            crate::logging::Level::Info,
            "switched off the opencode.ai domain: OpenCode is not installed",
        );
        if let Ok(Some(base_url)) = crate::account::load_base_url() {
            crate::audit::domain_toggled(&base_url, None, DOMAIN_SLUG, false);
        }
    }
    crate::primitives::write_file(&marker, b"", 0o600)?;
    Ok(enabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    fn gm(written: &[&str]) -> GateModels {
        GateModels {
            written: written.iter().map(|s| s.to_string()).collect(),
            ..GateModels::default()
        }
    }

    const SERVED: &str = "http://127.0.0.1:9977/__gate/t/opencode/gate/v1";

    #[test]
    fn a_model_reference_names_a_gate_model_only_on_gates_provider() {
        assert_eq!(
            gate_id_of("gate-connect/openai/gpt-5.6"),
            Some("openai/gpt-5.6")
        );
        assert_eq!(
            gate_id_of("gate-connect/openai/gpt-5.6#high"),
            Some("openai/gpt-5.6")
        );
        assert_eq!(gate_id_of("openai/gpt-5.6"), None);
        assert_eq!(gate_id_of("gate-connectx/a"), None);
        assert_eq!(gate_id_of("gate-connect"), None);
    }

    #[test]
    fn both_forms_of_model_are_read() {
        let s = obj(json!({ "model": "anthropic/claude-sonnet-5" }));
        assert_eq!(
            configured_model(&s).as_deref(),
            Some("anthropic/claude-sonnet-5")
        );
        let o = obj(json!({ "model": { "providerID": "gate-connect", "model": "a/b" } }));
        assert_eq!(configured_model(&o).as_deref(), Some("gate-connect/a/b"));
        assert_eq!(configured_model(&obj(json!({}))), None);
    }

    /// Applied needs all of it: `model` on Gate's provider, one of the set, and
    /// the provider on the Gate models route. Each one moved is drift.
    #[test]
    fn gate_model_state_needs_the_model_the_set_and_the_route() {
        let applied = json!({
            "model": "gate-connect/a/one",
            "provider": { "gate-connect": { "options": { "baseURL": SERVED } } }
        });
        assert_eq!(
            gate_model_state_of(&obj(applied.clone()), &gm(&["a/one", "a/two"])),
            GateModelState::Applied {
                model: "a/one".into()
            }
        );

        let mut elsewhere = applied.clone();
        elsewhere["model"] = json!("anthropic/claude-sonnet-5");
        assert_eq!(
            gate_model_state_of(&obj(elsewhere), &gm(&["a/one"])),
            GateModelState::Drifted {
                model: Some("anthropic/claude-sonnet-5".into())
            }
        );

        let mut outside_set = applied.clone();
        outside_set["model"] = json!("gate-connect/a/three");
        assert!(matches!(
            gate_model_state_of(&obj(outside_set), &gm(&["a/one"])),
            GateModelState::Drifted { .. }
        ));

        let mut off_route = applied.clone();
        off_route["provider"]["gate-connect"]["options"]["baseURL"] =
            json!("https://gate.example.com/v1");
        assert!(matches!(
            gate_model_state_of(&obj(off_route), &gm(&["a/one"])),
            GateModelState::Drifted { .. }
        ));

        let mut no_model = applied;
        no_model.as_object_mut().unwrap().remove("model");
        assert_eq!(
            gate_model_state_of(&obj(no_model), &gm(&["a/one"])),
            GateModelState::Drifted { model: None }
        );
    }

    /// Apply then revert is the identity on the file, whatever `model` and
    /// `provider` looked like before.
    #[test]
    fn apply_then_revert_restores_the_file() {
        let ids = vec!["a/one".to_string(), "a/two".to_string()];
        for before in [
            json!({}),
            json!({ "model": "openrouter/z-ai/glm-5.2", "autoupdate": false }),
            json!({ "model": { "providerID": "openrouter", "model": "x" }, "provider": {} }),
            json!({ "provider": { "openrouter": { "options": { "apiKey": "k" } } } }),
        ] {
            let mut settings = obj(before.clone());
            let mut state = State::default();
            apply_gate_models(&mut settings, &mut state, &ids, "http://127.0.0.1:9977").unwrap();
            assert_eq!(settings["model"], json!("gate-connect/a/one"));
            assert_eq!(
                settings["provider"]["gate-connect"]["options"]["baseURL"],
                json!(SERVED)
            );
            revert_gate_models(&mut settings, &mut state);
            assert_eq!(Value::Object(settings), before);
            assert!(state.is_empty());
        }
    }

    /// A model the user picked from the set survives a re-apply of the same
    /// set; a changed set starts again from its first model.
    #[test]
    fn a_reapply_keeps_a_pick_from_an_unchanged_set() {
        let relay = "http://127.0.0.1:9977";
        let ids = vec!["a/one".to_string(), "a/two".to_string()];
        let mut settings = Map::new();
        let mut state = State::default();
        apply_gate_models(&mut settings, &mut state, &ids, relay).unwrap();
        settings.insert("model".into(), json!("gate-connect/a/two"));
        apply_gate_models(&mut settings, &mut state, &ids, relay).unwrap();
        assert_eq!(settings["model"], json!("gate-connect/a/two"));

        let changed = vec!["a/two".to_string(), "a/three".to_string()];
        apply_gate_models(&mut settings, &mut state, &changed, relay).unwrap();
        assert_eq!(
            settings["model"],
            json!("gate-connect/a/two"),
            "first of the new set"
        );
        let models = settings["provider"]["gate-connect"]["models"]
            .as_object()
            .unwrap();
        assert_eq!(models.keys().collect::<Vec<_>>(), ["a/three", "a/two"]);
    }

    /// A default the user moved off Gate's provider is theirs and stays; Gate's
    /// provider goes regardless.
    #[test]
    fn revert_keeps_a_model_the_user_chose() {
        let mut settings = obj(json!({ "model": "openrouter/x" }));
        let mut state = State::default();
        apply_gate_models(
            &mut settings,
            &mut state,
            &["a/one".into()],
            "http://127.0.0.1:1",
        )
        .unwrap();
        settings.insert("model".into(), json!("anthropic/claude-sonnet-5"));
        revert_gate_models(&mut settings, &mut state);
        assert_eq!(
            Value::Object(settings),
            json!({ "model": "anthropic/claude-sonnet-5" })
        );
    }

    /// A `gate-connect` block that is on the Gate models route was written by
    /// Gate even with the record lost, and is taken over; one elsewhere is the
    /// user's and refused.
    #[test]
    fn only_a_users_own_gate_connect_provider_is_refused() {
        let ids = vec!["a/one".to_string()];
        let mut theirs = obj(json!({
            "provider": { "gate-connect": { "options": { "baseURL": "https://gate.example.com" } } }
        }));
        assert!(apply_gate_models(
            &mut theirs,
            &mut State::default(),
            &ids,
            "http://127.0.0.1:1"
        )
        .is_err());

        let mut orphan = obj(json!({
            "model": "gate-connect/a/one",
            "provider": { "gate-connect": { "options": { "baseURL": SERVED } } }
        }));
        let mut state = State::default();
        apply_gate_models(&mut orphan, &mut state, &ids, "http://127.0.0.1:9977").unwrap();
        assert_eq!(
            state.gate_models.as_ref().unwrap().previous_model,
            None,
            "a model on Gate's provider is not the user's to restore"
        );
    }

    /// AG-674's disagreement case for OpenCode. Our redirect is in the global
    /// file and correct; a layer OpenCode merges over it sends `anthropic`
    /// straight to Anthropic. Reporting that as Connected is the exact failure
    /// finding O1 describes.
    #[test]
    fn a_higher_layer_repointing_a_gated_provider_is_an_override() {
        let layer: Map<String, Value> = json!({
            "provider": {
                "anthropic": { "options": { "baseURL": "https://api.anthropic.com/v1" } }
            }
        })
        .as_object()
        .unwrap()
        .clone();
        let o = override_in(
            &layer,
            "/etc/opencode/opencode.json",
            &["anthropic".to_string()],
            "http://127.0.0.1:9977",
        )
        .expect("a repointed provider is an override");
        assert!(o.source.contains("/etc/opencode/opencode.json"));
        assert!(o.to_string().contains("api.anthropic.com"));
    }

    /// A layer that names the same relay URL we wrote changes nothing, and a
    /// layer that only touches providers we never gated is not our business.
    #[test]
    fn a_layer_agreeing_with_us_or_touching_other_providers_is_not_an_override() {
        let same: Map<String, Value> = json!({
            "provider": {
                "anthropic": {
                    "options": { "baseURL": "http://127.0.0.1:9977/__gate/t/opencode/anthropic/v1" }
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();
        assert_eq!(
            override_in(
                &same,
                "/etc/opencode/opencode.json",
                &["anthropic".to_string()],
                "http://127.0.0.1:9977"
            ),
            None
        );

        let elsewhere: Map<String, Value> = json!({
            "provider": { "llamacpp": { "options": { "baseURL": "http://localhost:8080/v1" } } }
        })
        .as_object()
        .unwrap()
        .clone();
        assert_eq!(
            override_in(
                &elsewhere,
                "/etc/opencode/opencode.json",
                &["anthropic".to_string()],
                "http://127.0.0.1:9977"
            ),
            None
        );
    }

    /// A layer can set other provider options without deciding the route. Only
    /// `options.baseURL` moves the traffic, so only it counts.
    #[test]
    fn a_layer_setting_other_options_is_not_an_override() {
        let layer: Map<String, Value> = json!({
            "provider": { "anthropic": { "options": { "timeout": 60000 } } }
        })
        .as_object()
        .unwrap()
        .clone();
        assert_eq!(
            override_in(
                &layer,
                "/etc/opencode/opencode.json",
                &["anthropic".to_string()],
                "http://127.0.0.1:9977"
            ),
            None
        );
    }

    /// An install that has never been launched still reads as installed on
    /// Windows. OpenCode Desktop puts nothing on `PATH` and writes no config
    /// until first run, and an npm or scoop install lands on a `PATH` this
    /// process may have been started before, so each showed "not installed"
    /// right after the user had installed it.
    ///
    /// `PATH` is emptied for the body so a real install on the machine running
    /// the test does not decide it.
    #[cfg(windows)]
    #[test]
    fn windows_detects_each_installer_before_first_launch() {
        let _lock = env::path_env_lock();
        let home =
            std::env::temp_dir().join(format!("gate-opencode-detect-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let prev_home = std::env::var_os("GATE_CONNECT_TEST_HOME");
        let prev_dir = std::env::var_os("OPENCODE_CONFIG_DIR");
        let prev_path = std::env::var_os("PATH");
        std::env::set_var("GATE_CONNECT_TEST_HOME", &home);
        std::env::remove_var("OPENCODE_CONFIG_DIR");
        std::env::remove_var("PATH");

        let result = std::panic::catch_unwind(|| {
            assert!(
                !OpenCode.detect().unwrap(),
                "an empty home has nothing installed"
            );
            for rel in [
                r"AppData\Local\Programs\@opencodedesktop\OpenCode.exe",
                r"AppData\Roaming\npm\opencode.cmd",
                r"scoop\shims\opencode.exe",
            ] {
                let bin = home.join(rel);
                fs::create_dir_all(bin.parent().unwrap()).unwrap();
                fs::write(&bin, b"").unwrap();
                assert!(
                    OpenCode.detect().unwrap(),
                    "{rel} alone must read as installed"
                );
                assert!(
                    !home.join(".config").join("opencode").exists(),
                    "and without the config dir that first launch creates"
                );
                fs::remove_file(&bin).unwrap();
            }
        });

        match prev_home {
            Some(v) => std::env::set_var("GATE_CONNECT_TEST_HOME", v),
            None => std::env::remove_var("GATE_CONNECT_TEST_HOME"),
        }
        if let Some(v) = prev_dir {
            std::env::set_var("OPENCODE_CONFIG_DIR", v);
        }
        if let Some(v) = prev_path {
            std::env::set_var("PATH", v);
        }
        let _ = fs::remove_dir_all(&home);
        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    }

    #[test]
    fn expected_base_url_carries_the_per_provider_path() {
        // Each base URL names the catalog slug the relay routes on, then the
        // provider's own path: `/v1` for Anthropic and OpenAI, `/api/v1` for
        // OpenRouter, whose API lives under `/api`.
        assert_eq!(
            expected_base_url("anthropic", "http://127.0.0.1:9977").as_deref(),
            Some("http://127.0.0.1:9977/__gate/t/opencode/anthropic/v1")
        );
        assert_eq!(
            expected_base_url("openai", "http://127.0.0.1:9977/").as_deref(),
            Some("http://127.0.0.1:9977/__gate/t/opencode/openai/v1")
        );
        assert_eq!(
            expected_base_url("openrouter", "http://127.0.0.1:9977").as_deref(),
            Some("http://127.0.0.1:9977/__gate/t/opencode/openrouter/v1")
        );
        // Zen and Go keep their own paths under the shared opencode.ai upstream.
        assert_eq!(
            expected_base_url("opencode", "http://127.0.0.1:9977").as_deref(),
            Some("http://127.0.0.1:9977/__gate/t/opencode/opencode/zen/v1")
        );
        assert_eq!(
            expected_base_url("opencode-go", "http://127.0.0.1:9977").as_deref(),
            Some("http://127.0.0.1:9977/__gate/t/opencode/opencode/zen/go/v1")
        );
        // An id that is not one we redirect has no expected value at all.
        assert_eq!(expected_base_url("nope", "http://127.0.0.1:9977"), None);
    }

    #[test]
    fn known_provider_endpoints_all_resolve_against_the_catalog() {
        // Every OpenCode provider we redirect must be one the relay can forward,
        // and the path we leave on the client must stay on that domain's
        // inference branch - otherwise the request either 403s off-catalog or,
        // worse, passes through to the user's own account while looking fine.
        for p in KNOWN_PROVIDERS {
            assert!(p.endpoint.starts_with("https://"), "{} must be https", p.id);
            let resolved = crate::proxy::resolve_endpoint(p.endpoint)
                .unwrap_or_else(|| panic!("{} endpoint {} is off-catalog", p.id, p.endpoint));
            let domain = crate::proxy::default_domains()
                .into_iter()
                .find(|d| d.slug == resolved.slug)
                .expect("resolved slug is a catalog domain");
            assert!(
                domain.rewrite_prefixes.iter().any(|pre| {
                    // The client path is only the start of what the tool sends -
                    // the SDK appends its own leaf - so the two must sit on the
                    // same branch, in either direction.
                    pre.starts_with(resolved.client_path.as_str())
                        || resolved.client_path.starts_with(pre.as_str())
                }),
                "{}: client path {:?} is on no branch of {:?}",
                p.id,
                resolved.client_path,
                domain.rewrite_prefixes
            );
        }
    }

    #[test]
    fn apply_override_rewrites_base_url_without_clobbering_user_options() {
        let mut providers = Map::new();
        providers.insert(
            "anthropic".to_string(),
            json!({
                "options": {
                    "apiKey": "{env:ANTHROPIC_API_KEY}",
                    "headers": { "X-User-Custom": "yes" },
                },
                "models": { "claude-haiku-4-5": {} }
            }),
        );
        let mut state = State::default();
        let target = KnownProvider {
            id: "anthropic",
            endpoint: "https://api.anthropic.com/v1",
        };
        apply_override(
            &mut providers,
            &mut state,
            &target,
            "http://127.0.0.1:9977/v1",
        );

        let anth = providers["anthropic"].as_object().unwrap();
        let opts = anth["options"].as_object().unwrap();
        // baseURL overwritten to the relay
        assert_eq!(opts["baseURL"], json!("http://127.0.0.1:9977/v1"));
        // user's apiKey survived
        assert_eq!(opts["apiKey"], json!("{env:ANTHROPIC_API_KEY}"));
        // The user's own header survives, and no Gate header joins it: the relay
        // reads the upstream off the slug in baseURL and injects both the hint
        // and the credential itself.
        let hdrs = opts["headers"].as_object().unwrap();
        assert_eq!(hdrs["X-User-Custom"], json!("yes"));
        assert!(!hdrs.contains_key("X-Gate-Api-Key"));
        assert!(!hdrs.contains_key("X-Gate-Upstream-Url"));
        // models block untouched
        assert!(anth["models"]
            .as_object()
            .unwrap()
            .contains_key("claude-haiku-4-5"));
        // snapshot captured
        let snap = &state.providers["anthropic"];
        assert!(snap.provider_existed);
        assert!(snap.options_existed);
        assert!(snap.previous_base_url.is_none());
        assert_eq!(
            snap.previous_headers,
            Some(json!({ "X-User-Custom": "yes" }))
        );
    }

    #[test]
    fn apply_override_then_restore_round_trips_to_original() {
        // Start with a provider that has options + baseURL + custom headers
        let original = json!({
            "options": {
                "apiKey": "sk-real",
                "baseURL": "https://my-self-hosted.example/v1",
                "headers": { "X-User-Custom": "yes" },
            },
        });
        let mut providers = Map::new();
        providers.insert("openai".to_string(), original.clone());

        let mut state = State::default();
        let target = KnownProvider {
            id: "openai",
            endpoint: "https://api.openai.com/v1",
        };
        apply_override(
            &mut providers,
            &mut state,
            &target,
            "http://127.0.0.1:9977/v1",
        );

        // Connect mutated options.baseURL and added the upstream header - confirm.
        assert_ne!(providers["openai"], original);

        // Now disconnect.
        let snap = state.providers["openai"].clone();
        restore_provider(&mut providers, "openai", &snap);
        assert_eq!(
            providers["openai"], original,
            "restored provider must match original byte-for-byte"
        );
    }

    #[test]
    fn apply_override_creates_entry_then_restore_removes_it() {
        // Provider didn't exist before connect - disconnect should drop it.
        let mut providers = Map::new();
        let mut state = State::default();
        let target = KnownProvider {
            id: "openai",
            endpoint: "https://api.openai.com/v1",
        };
        apply_override(
            &mut providers,
            &mut state,
            &target,
            "http://127.0.0.1:9977/v1",
        );

        assert!(providers.contains_key("openai"));
        let snap = state.providers["openai"].clone();
        assert!(!snap.provider_existed);

        restore_provider(&mut providers, "openai", &snap);
        assert!(
            !providers.contains_key("openai"),
            "freshly-created provider should be removed on disconnect"
        );
    }

    #[test]
    fn looks_local_classifies_common_endpoints() {
        // Public clouds - should NOT look local.
        for u in [
            "https://api.openai.com",
            "https://api.anthropic.com/v1",
            "https://openrouter.ai/api/v1",
            "https://opencode.ai/zen",
            "https://opencode.ai/zen/go",
        ] {
            assert!(!looks_local(u), "expected public: {u}");
        }
        // Localhost / loopback / private nets / .local / http://.
        for u in [
            "http://localhost:8080",
            "http://127.0.0.1:11434",
            "https://127.0.0.1:8443/v1",
            "https://192.168.1.50:3000",
            "https://10.0.0.7/v1",
            "https://172.16.5.4",
            "https://172.31.200.1",
            "http://my-rig.local:8080",
            "https://gpu-box.lan",
            "http://54.188.228.109:3000", // plain http to a public IP - still treated as not-Gate-friendly
        ] {
            assert!(looks_local(u), "expected local: {u}");
        }
        // Edges that should NOT trip the private-net heuristic.
        for u in [
            "https://172.15.0.1", // outside 16-31 range
            "https://172.32.0.1", // outside 16-31 range
            "https://api.10.com", // starts with digit but not 10.
        ] {
            assert!(!looks_local(u), "expected public: {u}");
        }
    }

    #[test]
    fn known_providers_includes_opencode_and_opencode_go() {
        let ids: Vec<&str> = KNOWN_PROVIDERS.iter().map(|p| p.id).collect();
        assert!(ids.contains(&"opencode"));
        assert!(ids.contains(&"opencode-go"));
        for p in KNOWN_PROVIDERS {
            assert!(p.endpoint.starts_with("https://"), "{} must be https", p.id);
            assert!(
                !p.endpoint.ends_with('/'),
                "{} endpoint must not end in /",
                p.id
            );
        }
    }

    #[test]
    fn reconnect_preserves_original_snapshot() {
        let mut providers = Map::new();
        providers.insert(
            "anthropic".to_string(),
            json!({ "options": { "apiKey": "user-original" }}),
        );
        let mut state = State::default();
        let target = KnownProvider {
            id: "anthropic",
            endpoint: "https://api.anthropic.com/v1",
        };

        apply_override(
            &mut providers,
            &mut state,
            &target,
            "http://127.0.0.1:9001/v1",
        );
        let first_snapshot = state.providers["anthropic"].previous_headers.clone();

        // Second connect with a different relay port - snapshot must NOT
        // change to reflect our own intermediate (post-first-connect) state.
        apply_override(
            &mut providers,
            &mut state,
            &target,
            "http://127.0.0.1:9002/v1",
        );
        let second_snapshot = state.providers["anthropic"].previous_headers.clone();
        assert_eq!(first_snapshot, second_snapshot);
        // baseURL on disk reflects the latest connect.
        assert_eq!(
            providers["anthropic"]["options"]["baseURL"],
            json!("http://127.0.0.1:9002/v1")
        );
    }

    fn ids(providers: &[&KnownProvider]) -> Vec<&'static str> {
        providers.iter().map(|p| p.id).collect()
    }

    /// Seed the way `connect` does when nothing is configured, into a fresh map.
    fn seed(mode: crate::account::BillingMode) -> (Map<String, Value>, State) {
        let mut settings = Map::new();
        let mut state = State::default();
        let provider_map = ensure_object(&mut settings, "provider");
        for target in seed_providers(mode) {
            apply_override(provider_map, &mut state, target, "http://127.0.0.1:9001/x");
            mark_seeded(provider_map, &mut state, target.id);
        }
        (settings, state)
    }

    /// Pay-as-you-go adds the providers Gate pays for, and BYOK adds nothing:
    /// not even the Zen a fresh OpenCode uses, whose free tier refuses traffic
    /// routed through Gate. Every seed has to be one Gate can bill.
    #[test]
    fn seeds_gate_billed_providers_on_payg_and_nothing_on_byok() {
        use crate::account::BillingMode;
        assert!(seed_providers(BillingMode::Byok).is_empty());
        assert_eq!(
            ids(&seed_providers(BillingMode::Payg)),
            ["anthropic", "openai"]
        );
        for id in PAYG_SEED_PROVIDERS {
            let p = KNOWN_PROVIDERS.iter().find(|p| p.id == *id).unwrap();
            let slug = crate::proxy::resolve_endpoint(p.endpoint).unwrap().slug;
            assert!(
                gate_connect_paths::PAYG_ELIGIBLE_SLUGS.contains(&slug.as_str()),
                "{id} is seeded for PAYG but Gate cannot bill {slug}"
            );
        }
    }

    /// Each seed points at the relay and carries the placeholder key, and the
    /// sidecar records it as added by Gate.
    #[test]
    fn payg_seeds_carry_the_placeholder_key() {
        let (settings, state) = seed(crate::account::BillingMode::Payg);
        for id in PAYG_SEED_PROVIDERS {
            let options = &settings["provider"][*id]["options"];
            assert_eq!(options["apiKey"], json!(PAYG_PLACEHOLDER_API_KEY));
            assert_eq!(options["baseURL"], json!("http://127.0.0.1:9001/x"));
        }
        assert!(state
            .providers
            .values()
            .all(|s| s.seeded && s.placeholder_key && !s.provider_existed));
        assert_eq!(
            placeholder_providers(&settings, &state),
            ["anthropic", "openai"]
        );
    }

    /// A placeholder the user has since replaced with a key of their own is
    /// not one Gate relies on.
    #[test]
    fn a_replaced_placeholder_is_not_relied_on() {
        let (mut settings, state) = seed(crate::account::BillingMode::Payg);
        settings["provider"]["anthropic"]["options"]["apiKey"] = json!("sk-ant-user");
        assert_eq!(placeholder_providers(&settings, &state), ["openai"]);
    }

    /// `unseed` takes out exactly what seeding added, and leaves a provider the
    /// user has alone - so a later login, or a switch to BYOK, starts clean.
    #[test]
    fn unseed_removes_seeds_and_keeps_the_users_providers() {
        let (mut settings, mut state) = seed(crate::account::BillingMode::Payg);
        settings["provider"].as_object_mut().unwrap().insert(
            "llamacpp".into(),
            json!({ "options": { "baseURL": "http://localhost:8080" } }),
        );
        assert!(unseed(&mut settings, &mut state));
        assert_eq!(
            Value::Object(settings),
            json!({ "provider": { "llamacpp": { "options": { "baseURL": "http://localhost:8080" } } } })
        );
        assert!(state.providers.is_empty());

        // Nothing else left: the `provider` block goes too.
        let (mut settings, mut state) = seed(crate::account::BillingMode::Payg);
        assert!(unseed(&mut settings, &mut state));
        assert!(settings.is_empty());
    }

    /// With nothing seeded, `unseed` touches nothing, including a provider Gate
    /// routes for the user.
    #[test]
    fn unseed_leaves_a_routed_user_provider_alone() {
        let mut settings: Map<String, Value> =
            json!({ "provider": { "anthropic": { "options": { "apiKey": "user" } } } })
                .as_object()
                .unwrap()
                .clone();
        let mut state = State::default();
        let target = KNOWN_PROVIDERS
            .iter()
            .find(|p| p.id == "anthropic")
            .unwrap();
        apply_override(
            ensure_object(&mut settings, "provider"),
            &mut state,
            target,
            "http://127.0.0.1:9001/x",
        );
        let before = settings.clone();
        assert!(!unseed(&mut settings, &mut state));
        assert_eq!(settings, before);
        assert!(state.providers.contains_key("anthropic"));
    }

    /// A sidecar from a build before seeding parses, with nothing marked seeded.
    #[test]
    fn an_older_sidecar_reads_as_unseeded() {
        let state: State = serde_json::from_str(
            r#"{"providers":{"anthropic":{"provider_existed":true,"options_existed":true}}}"#,
        )
        .unwrap();
        let s = &state.providers["anthropic"];
        assert!(!s.seeded && !s.placeholder_key);
    }
}
