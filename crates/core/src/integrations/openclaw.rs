//! OpenClaw integration.
//!
//! Like OpenCode — and unlike Claude Code / Codex — OpenClaw is
//! multi-provider by design. Users configure providers (Anthropic, OpenAI,
//! OpenRouter, …) in `~/.openclaw/openclaw.json` and authenticate to each
//! with its own credential (env vars like `ANTHROPIC_API_KEY`). Gate
//! Connect's job here is purely to *intercept* those existing requests:
//! redirect them through Constellation Gate and add the two Gate
//! identification headers. The user's upstream auth is untouched — Gate is a
//! pure passthrough on `Authorization` / `x-api-key`.
//!
//! Mechanism: OpenClaw configures providers under `models.providers.<id>`
//! (see <https://docs.openclaw.ai/concepts/model-providers>). For each
//! well-known provider the user already has configured, we write:
//!
//! ```json5
//! "models": {
//!   "providers": {
//!     "<id>": {
//!       "baseUrl": "http://127.0.0.1:<relay-port>/<slug><client-path>"
//!     }
//!   }
//! }
//! ```
//!
//! That single value is the entire write. Both parts come from
//! [`crate::proxy::resolve_endpoint`]: `<slug>` names the catalog domain, which
//! is how the relay knows where to forward, and `<client-path>` is whatever sits
//! between the upstream host and the SDK's own suffix - `/v1` for Anthropic and
//! OpenAI, `/api/v1` for OpenRouter, since Gate forwards the path verbatim.
//!
//! The base URL points at the loopback reverse-proxy relay
//! ([`crate::proxy::relay`]), not the gateway: the relay injects the live Gate
//! credential *and* the upstream hint per request, so **neither a credential nor
//! a header is written to openclaw.json** - `headers` is never touched at all.
//! The provider's `apiKey`, `api`, model list, and any other options the user set
//! survive the merge.
//!
//! Discovery: a provider is gated if it is in our well-known list and either
//! has a `models.providers.<id>` block or shows up in the auth-profile signal.
//! Caveat on that second signal: it reads `auth.profiles` out of
//! `openclaw.json`, but current OpenClaw keeps auth profiles in a per-agent
//! store (`~/.openclaw/agents/<id>/agent/…`), so a provider configured purely
//! via `openclaw models auth login` is likely still missed. See
//! `docs/harness-integration-validation.md` H2 - it is one of the reasons this
//! integration is hidden from the popover.
//!
//! Config format: `openclaw.json` is JSON5 (comments + trailing commas
//! allowed), so we parse with the `json5` crate. We write back with
//! `serde_json` (valid JSON5). NOTE: a connect rewrites the file and so
//! **drops any comments / JSON5 formatting** the user had elsewhere in it.
//! The gated `baseUrl` / `headers` values themselves are snapshotted and
//! restored byte-for-byte on disconnect.
//!
//! State tracking: snapshots of each gated provider's original `baseUrl` +
//! `headers` live in a sidecar at `<app_support_dir>/openclaw-state.json`,
//! keeping our markers out of the user-owned config file (same reasoning as
//! OpenCode's sidecar).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::env;
use crate::registry::{ConnectInput, Integration, Status, ToolId};

const UPSTREAM_PROVIDER_NAME: &str = "your existing providers";
const DEFAULT_UPSTREAM_URL: &str = "https://api.anthropic.com";

/// Sidecar state file. We keep state out of the user-owned `openclaw.json`.
const STATE_FILENAME: &str = "openclaw-state.json";

/// Providers we know how to redirect through Gate. For each, the
/// `upstream_url` is the bare-host form expected by `X-Gate-Upstream-Url` —
/// Gate concatenates the inbound request path onto it, so trailing `/v1`
/// lives on the client side (in baseUrl), never here.
struct KnownProvider {
    id: &'static str,
    endpoint: &'static str,
}

/// The list is intentionally short — matching the set Gate already forwards
/// (cf. `provider.rs` proxy domains). Adding a new entry means knowing the
/// canonical upstream URL and that the upstream speaks a wire format Gate can
/// forward. The IDs are OpenClaw's provider IDs. Users with custom providers
/// can still wire them up by hand.
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
];

/// Skip a provider if its currently-configured `baseUrl` looks non-public —
/// localhost / loopback / private network / plain HTTP. Protects against the
/// edge case where someone points a well-known provider ID at a local
/// endpoint and would otherwise have it silently swapped to the Gate URL.
fn looks_local(base_url: &str) -> bool {
    let lc = base_url.trim().to_ascii_lowercase();
    if lc.is_empty() {
        return false;
    }
    if !lc.starts_with("https://") {
        // http://, ws://, plain hostname, file:// — all not the public
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
    /// One entry per provider we modified, keyed by provider ID. Each entry
    /// records exactly what `baseUrl` and `headers` looked like *before* we
    /// touched it, so disconnect can restore byte-identical values. BTreeMap
    /// so the on-disk JSON stays deterministic.
    #[serde(default)]
    providers: BTreeMap<String, ProviderSnapshot>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct ProviderSnapshot {
    /// Whether the provider entry existed at all before connect. If false,
    /// disconnect must remove the entry entirely.
    #[serde(default)]
    provider_existed: bool,
    /// Original `baseUrl`. `None` means the field was absent.
    #[serde(default)]
    previous_base_url: Option<Value>,
    /// Original `headers`. `None` means the field was absent.
    #[serde(default)]
    previous_headers: Option<Value>,
}

/// Likely install locations of the `openclaw` binary. Detection falls back to
/// the config dir, which is the primary signal for OpenClaw (it always writes
/// `~/.openclaw`), so this stays empty rather than guessing per-user prefixes.
#[cfg(unix)]
const CLI_BIN_PATHS: &[&str] = &[];
#[cfg(windows)]
const CLI_BIN_PATHS: &[&str] = &[];

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
        // OpenClaw already has the user's upstream credentials (provider env
        // vars). Gate is a pure passthrough on `Authorization` / `x-api-key`,
        // so no separate key here.
        false
    }

    fn detect(&self) -> Result<bool> {
        if CLI_BIN_PATHS.iter().any(|p| Path::new(p).exists()) {
            return Ok(true);
        }
        Ok(env::openclaw_config_dir()?.exists())
    }

    fn config_is_managed(&self) -> Result<bool> {
        // Two-part marker, mirroring OpenCode. The sidecar only exists because
        // connect() wrote it, but on its own it can't tell our stale write apart
        // from a baseUrl the user has since repointed by hand - so also require
        // that at least one recorded provider still aims at loopback, which is
        // only ever us.
        let Some(state) = load_state()? else {
            return Ok(false);
        };
        let settings = load_settings()?.unwrap_or_default();
        Ok(state.providers.keys().any(|provider_id| {
            settings
                .get("models")
                .and_then(|v| v.as_object())
                .and_then(|m| m.get("providers"))
                .and_then(|v| v.as_object())
                .and_then(|p| p.get(provider_id))
                .and_then(|v| v.as_object())
                .and_then(|b| b.get("baseUrl"))
                .and_then(|v| v.as_str())
                .is_some_and(looks_local)
        }))
    }

    fn status(&self) -> Result<Status> {
        if !self.detect()? {
            return Ok(Status::NotInstalled);
        }
        // Connected = sidecar state exists AND at least one provider currently
        // carries Gate's two headers. If state exists but no provider has our
        // headers anymore, the user edited openclaw.json by hand — surface
        // that as drift.
        let Some(state) = load_state()? else {
            return Ok(Status::Detected);
        };
        if state.providers.is_empty() {
            return Ok(Status::Detected);
        }

        let settings = load_settings()?.unwrap_or_default();

        let relay_base = match crate::proxy::relay_base_url() {
            Some(u) => u,
            None => {
                return Ok(Status::Drifted(
                    "the Gate proxy has not been enabled yet - turn it on to route OpenClaw".into(),
                ));
            }
        };

        Ok(compute_status(&state, &settings, &relay_base))
    }

    fn connect(&self, input: &ConnectInput) -> Result<()> {
        if !self.detect()? {
            anyhow::bail!(
                "OpenClaw is not installed on this machine — install it from https://docs.openclaw.ai first"
            );
        }
        let relay_base_url = input.relay_base_url.as_deref().context(
            "the Gate proxy relay is not running - enable the proxy before connecting OpenClaw",
        )?;

        let mut settings = load_settings()?.unwrap_or_default();

        // Figure out which well-known providers to route through Gate. Two
        // independent signals, because OpenClaw configures providers two ways:
        //   - an explicit `models.providers.<id>` block (custom baseUrl, etc.),
        //   - an `auth.profiles.*` entry written by `openclaw models auth login`
        //     for a built-in/plugin provider (e.g. OpenRouter, Anthropic).
        // A provider set up purely via auth login has NO `models.providers`
        // entry yet — `apply_override` creates one, and the snapshot's
        // `provider_existed = false` makes disconnect remove it again.
        let mut configured: Vec<&str> = provider_map_ref(&settings)
            .map(|m| m.keys().map(String::as_str).collect())
            .unwrap_or_default();
        for id in auth_profile_providers(&settings) {
            if !configured.contains(&id) {
                configured.push(id);
            }
        }

        let candidates: Vec<&KnownProvider> = KNOWN_PROVIDERS
            .iter()
            .filter(|p| configured.contains(&p.id))
            .collect();

        // Local-protection guard: if a candidate has an existing non-public
        // `baseUrl`, skip it — the user pointed it at a private endpoint on
        // purpose and we should not redirect that traffic through Gate.
        let mut skipped_local: Vec<&str> = Vec::new();
        let targets: Vec<&KnownProvider> = candidates
            .into_iter()
            .filter(|p| {
                let existing = provider_map_ref(&settings)
                    .and_then(|m| m.get(p.id))
                    .and_then(|v| v.as_object())
                    .and_then(|b| b.get("baseUrl"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !existing.is_empty() && looks_local(existing) {
                    skipped_local.push(p.id);
                    false
                } else {
                    true
                }
            })
            .collect();

        if targets.is_empty() {
            anyhow::bail!(
                "No supported OpenClaw providers found to route through Gate. Authenticate one of anthropic / openai / openrouter (e.g. `openclaw models auth login`) or configure it under `models.providers` in ~/.openclaw/openclaw.json first, then re-run connect."
            );
        }
        if !skipped_local.is_empty() {
            eprintln!(
                "note: skipping providers that look local / self-hosted: {}. Their baseUrl stays as-is. Disconnect-then-reconnect after pointing them at a public endpoint to route them via Gate.",
                skipped_local.join(", ")
            );
        }

        // Load any existing state — re-connects must preserve the *original*
        // snapshots, not overwrite them with our intermediate values from the
        // previous connect.
        let mut state = load_state()?.unwrap_or_default();

        let provider_map = ensure_provider_map(&mut settings);

        // Each provider gets its own baseUrl: the path Gate must not swallow
        // differs per upstream (`/v1` for Anthropic and OpenAI, `/api/v1` for
        // OpenRouter), and the split between "upstream Gate forwards to" and
        // "path the client keeps" is decided in one place, by the catalog.
        let mut skipped_off_catalog: Vec<&str> = Vec::new();
        let mut applied = 0;
        for target in &targets {
            let Some(resolved) = crate::proxy::resolve_endpoint(target.endpoint) else {
                // No catalog domain covers this upstream, so the relay would
                // 403 every request. Leaving the config alone is strictly
                // better than repointing it at a dead end.
                skipped_off_catalog.push(target.id);
                continue;
            };
            let base_url = resolved.relay_base_url(relay_base_url);
            apply_override(provider_map, &mut state, target, &base_url);
            applied += 1;
        }

        if applied == 0 {
            anyhow::bail!(
                "None of the configured OpenClaw providers can route through Gate yet ({}). Gate has no upstream domain for them.",
                skipped_off_catalog.join(", ")
            );
        }

        save_state(&state)?;
        write_settings(&settings)
    }

    fn disconnect(&self) -> Result<()> {
        let Some(state) = load_state()? else {
            return Ok(());
        };
        let Some(mut settings) = load_settings()? else {
            remove_state()?;
            return Ok(());
        };

        if let Some(provider_map) = provider_map_mut(&mut settings) {
            for (provider_id, snapshot) in &state.providers {
                restore_provider(provider_map, provider_id, snapshot);
            }
            if provider_map.is_empty() {
                // Drop empty `models.providers`, then `models` if it's now bare.
                if let Some(models) = settings.get_mut("models").and_then(|v| v.as_object_mut()) {
                    models.remove("providers");
                    if models.is_empty() {
                        settings.remove("models");
                    }
                }
            }
        }

        if settings.is_empty() {
            let path = env::openclaw_config_path()?;
            if path.exists() {
                fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
            }
            remove_state()?;
            return Ok(());
        }
        write_settings(&settings)?;
        // Only drop the sidecar once the restored config is on disk: losing it
        // before a failed write would leave Gate headers in openclaw.json while
        // status reports the tool as clean and re-disconnect no-ops.
        remove_state()
    }

    fn save_upstream_credential(&self, _credential: &str) -> Result<()> {
        anyhow::bail!(
            "OpenClaw does not need a separate upstream credential — Gate Connect adds its headers to whatever provider(s) you've already configured in ~/.openclaw/openclaw.json."
        )
    }

    /// Hidden from the popover: provider discovery reads `auth.profiles` from
    /// `openclaw.json`, but OpenClaw keeps auth profiles in a per-agent store,
    /// so a provider configured purely via `openclaw models auth login` is
    /// likely never gated. Redirecting `anthropic` also makes OpenClaw suppress
    /// its implicit beta headers (interleaved thinking, claude-code) with no
    /// warning from us. See docs/harness-integration-validation.md H2, H3.
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

/// Pure drift evaluation: given the providers we own (`state`), the parsed
/// `openclaw.json` (`settings`), and the gateway baseUrl we expect, decide the
/// connection status. A provider is healthy when its `baseUrl` matches and it
/// carries both Gate headers; any mismatch is drift. No providers healthy →
/// the user wiped our edits; some unhealthy → a partial hand-edit. Split out of
/// [`OpenClaw::status`] so the comparison logic is testable without touching
/// the filesystem, account, or env.
fn compute_status(state: &State, settings: &Map<String, Value>, relay_base_url: &str) -> Status {
    let mut healthy = 0;
    let mut drifted = Vec::new();
    for provider_id in state.providers.keys() {
        let block = provider_map_ref(settings)
            .and_then(|m| m.get(provider_id))
            .and_then(|v| v.as_object());
        let base_url = block
            .and_then(|b| b.get("baseUrl"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        // The baseUrl is the whole test now: it carries the relay origin, the
        // catalog slug the relay routes on, and the provider's own path. No
        // credential and no header is written, so there is nothing else to
        // check. Per provider, because OpenRouter's path is `/api/v1` where
        // Anthropic's is `/v1`.
        let expected = expected_base_url(provider_id, relay_base_url);
        if expected.as_deref() == Some(base_url) {
            healthy += 1;
        } else {
            drifted.push(provider_id.clone());
        }
    }
    if healthy == 0 {
        return Status::Drifted(format!(
            "no providers carry Gate headers; expected: {}",
            state
                .providers
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !drifted.is_empty() {
        return Status::Drifted(format!(
            "some providers were edited by hand and no longer route via Gate: {}",
            drifted.join(", ")
        ));
    }
    Status::Connected
}

/// Snapshot the provider's current state, then overwrite `baseUrl` (to the
/// relay) and merge the non-secret `X-Gate-Upstream-Url` header into `headers`.
/// No Gate credential is written - the relay injects it live per request.
/// Existing `apiKey`, `api`, model lists, and any other fields survive
/// untouched.
fn apply_override(
    provider_map: &mut Map<String, Value>,
    state: &mut State,
    target: &KnownProvider,
    relay_base_url: &str,
) {
    // Only snapshot the first time we touch this provider. Re-connect must
    // preserve the user's original values, not record our own intermediate
    // state.
    if !state.providers.contains_key(target.id) {
        let block = provider_map.get(target.id).and_then(|v| v.as_object());
        let provider_existed = block.is_some();
        let previous_base_url = block.and_then(|b| b.get("baseUrl")).cloned();
        let previous_headers = block.and_then(|b| b.get("headers")).cloned();
        state.providers.insert(
            target.id.to_string(),
            ProviderSnapshot {
                provider_existed,
                previous_base_url,
                previous_headers,
            },
        );
    }

    // Did the user own this provider entry before we first touched it? The
    // snapshot written above is the source of truth, so this stays correct
    // across re-connects (where our own entry is already present on disk).
    let created_by_us = state
        .providers
        .get(target.id)
        .map(|s| !s.provider_existed)
        .unwrap_or(false);

    // Ensure models.providers.<id> exists as an object, then patch it.
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

    provider.insert(
        "baseUrl".to_string(),
        Value::String(relay_base_url.to_string()),
    );

    // OpenClaw's schema requires a `models` array on every
    // `models.providers.<id>` block. When we *create* the entry for a
    // built-in / auth-profile provider (it had no `models.providers` block
    // before), seed an empty array: OpenClaw still merges the provider's
    // built-in catalog models, so `<provider>/...` model refs keep
    // resolving, and the config validates instead of erroring with
    // "models.providers.<id>.models: Invalid input". We never add it to a
    // user-owned entry — a custom provider definition owns its own list,
    // and disconnect must restore it byte-for-byte.
    if created_by_us {
        provider
            .entry("models".to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
    }

    // `headers` is left alone entirely: the relay reads the upstream off the slug
    // segment in `baseUrl` and injects the hint itself.
}

/// Reverse `apply_override` for one provider. Restores baseUrl / headers to
/// whatever the user had, and drops a provider entry we created entirely.
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

    match &snapshot.previous_base_url {
        Some(v) => {
            provider.insert("baseUrl".to_string(), v.clone());
        }
        None => {
            provider.remove("baseUrl");
        }
    }
    match &snapshot.previous_headers {
        Some(v) => {
            provider.insert("headers".to_string(), v.clone());
        }
        None => {
            provider.remove("headers");
        }
    }
}

/// The `baseUrl` connect writes for `provider_id`, given the relay's loopback
/// base. `None` when the provider is not one we redirect (unknown id, or an
/// endpoint no catalog domain covers) - `compute_status` treats that as drift
/// rather than pretending it matches.
///
/// The suffix is per provider and comes from the catalog, not a constant: the
/// relay and Gate forward the path verbatim, so whatever sits between the
/// upstream host and the SDK's own suffix (`/v1` for Anthropic and OpenAI,
/// `/api/v1` for OpenRouter) has to live on this side.
fn expected_base_url(provider_id: &str, relay_base_url: &str) -> Option<String> {
    let target = KNOWN_PROVIDERS.iter().find(|p| p.id == provider_id)?;
    let resolved = crate::proxy::resolve_endpoint(target.endpoint)?;
    Some(resolved.relay_base_url(relay_base_url))
}

// --- file I/O ---------------------------------------------------------

/// Borrow `models.providers` from a parsed settings map, if present.
fn provider_map_ref(settings: &Map<String, Value>) -> Option<&Map<String, Value>> {
    settings
        .get("models")
        .and_then(|v| v.as_object())
        .and_then(|m| m.get("providers"))
        .and_then(|v| v.as_object())
}

/// Provider ids the user has an auth profile for. `openclaw models auth login`
/// writes `auth.profiles.<provider>:<name>` entries (each with a `provider`
/// field) but does *not* add a `models.providers` block — so for built-in /
/// plugin providers (OpenRouter, Anthropic, …) this is the only discovery
/// signal that the user actually has credentials to route. May contain
/// duplicates when a provider has multiple profiles; callers dedupe.
fn auth_profile_providers(settings: &Map<String, Value>) -> Vec<&str> {
    settings
        .get("auth")
        .and_then(|v| v.as_object())
        .and_then(|m| m.get("profiles"))
        .and_then(|v| v.as_object())
        .map(|profiles| {
            profiles
                .values()
                .filter_map(|p| p.as_object()?.get("provider")?.as_str())
                .collect()
        })
        .unwrap_or_default()
}

/// Mutably borrow `models.providers` from a settings map, if both levels are
/// objects. Used by disconnect / refresh, which never create the path.
fn provider_map_mut(settings: &mut Map<String, Value>) -> Option<&mut Map<String, Value>> {
    settings
        .get_mut("models")
        .and_then(|v| v.as_object_mut())
        .and_then(|m| m.get_mut("providers"))
        .and_then(|v| v.as_object_mut())
}

/// Ensure `models.providers` exists as nested objects and return it mutably.
fn ensure_provider_map(settings: &mut Map<String, Value>) -> &mut Map<String, Value> {
    let models = ensure_object(settings, "models");
    ensure_object(models, "providers")
}

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
    // 0o600 defensively (the file no longer holds the Gate key - the relay
    // injects it - but may carry other user config). Atomic-write protects
    // against partial writes corrupting the config on crash.
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
    fn expected_base_url_carries_the_per_provider_path() {
        // Each base URL names the catalog slug the relay routes on, then the
        // provider's own path: `/v1` for Anthropic and OpenAI, `/api/v1` for
        // OpenRouter, whose API lives under `/api`.
        assert_eq!(
            expected_base_url("anthropic", "http://127.0.0.1:9977").as_deref(),
            Some("http://127.0.0.1:9977/anthropic/v1")
        );
        assert_eq!(
            expected_base_url("openai", "http://127.0.0.1:9977/").as_deref(),
            Some("http://127.0.0.1:9977/openai/v1")
        );
        assert_eq!(
            expected_base_url("openrouter", "http://127.0.0.1:9977").as_deref(),
            Some("http://127.0.0.1:9977/openrouter/api/v1")
        );
        assert_eq!(expected_base_url("nope", "http://127.0.0.1:9977"), None);
    }

    #[test]
    fn apply_override_rewrites_base_url_without_clobbering_user_fields() {
        let mut providers = Map::new();
        providers.insert(
            "anthropic".to_string(),
            json!({
                "apiKey": "${ANTHROPIC_API_KEY}",
                "api": "anthropic-messages",
                "headers": { "X-User-Custom": "yes" },
                "models": [{ "id": "claude-opus-4-6" }]
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
        // baseUrl overwritten to the relay
        assert_eq!(anth["baseUrl"], json!("http://127.0.0.1:9977/v1"));
        // user's apiKey + api survived
        assert_eq!(anth["apiKey"], json!("${ANTHROPIC_API_KEY}"));
        assert_eq!(anth["api"], json!("anthropic-messages"));
        // The user's own header survives, and no Gate header joins it: the relay
        // reads the upstream off the slug in baseUrl and injects both the hint
        // and the credential itself.
        let hdrs = anth["headers"].as_object().unwrap();
        assert_eq!(hdrs["X-User-Custom"], json!("yes"));
        assert!(!hdrs.contains_key("X-Gate-Api-Key"));
        assert!(!hdrs.contains_key("X-Gate-Upstream-Url"));
        // models block untouched
        assert!(anth["models"].is_array());
        // snapshot captured
        let snap = &state.providers["anthropic"];
        assert!(snap.provider_existed);
        assert!(snap.previous_base_url.is_none());
        assert_eq!(
            snap.previous_headers,
            Some(json!({ "X-User-Custom": "yes" }))
        );
    }

    #[test]
    fn apply_override_then_restore_round_trips_to_original() {
        let original = json!({
            "apiKey": "sk-real",
            "baseUrl": "https://my-self-hosted.example/v1",
            "headers": { "X-User-Custom": "yes" },
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

        // Connect mutated baseUrl and added the upstream header — confirm.
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
        // Provider didn't exist before connect — disconnect should drop it.
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

        // A created block must carry a `models` array — OpenClaw's schema
        // requires it (`models.providers.<id>.models`). Empty is valid and
        // lets the built-in catalog models merge in.
        let created = providers["openai"].as_object().unwrap();
        assert_eq!(
            created.get("models"),
            Some(&json!([])),
            "created provider must seed an empty models array for schema validity"
        );

        restore_provider(&mut providers, "openai", &snap);
        assert!(
            !providers.contains_key("openai"),
            "freshly-created provider should be removed on disconnect"
        );
    }

    #[test]
    fn looks_local_classifies_common_endpoints() {
        for u in [
            "https://api.openai.com",
            "https://api.anthropic.com/v1",
            "https://openrouter.ai/api/v1",
        ] {
            assert!(!looks_local(u), "expected public: {u}");
        }
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
        ] {
            assert!(looks_local(u), "expected local: {u}");
        }
        for u in [
            "https://172.15.0.1", // outside 16-31 range
            "https://172.32.0.1", // outside 16-31 range
            "https://api.10.com", // starts with digit but not 10.
        ] {
            assert!(!looks_local(u), "expected public: {u}");
        }
    }

    #[test]
    fn known_provider_endpoints_all_resolve_against_the_catalog() {
        // Every OpenClaw provider we redirect must be one the relay can forward.
        // This is the assertion the old "bare https host" check could not make:
        // `https://openrouter.ai/api` satisfied every shape rule while matching
        // no catalog entry, so connect happily pointed OpenRouter at a relay
        // that 403'd every request.
        for p in KNOWN_PROVIDERS {
            assert!(p.endpoint.starts_with("https://"), "{} must be https", p.id);
            assert!(
                !p.endpoint.ends_with('/'),
                "{} endpoint must not end in /",
                p.id
            );
            let resolved = crate::proxy::resolve_endpoint(p.endpoint)
                .unwrap_or_else(|| panic!("{} endpoint {} is off-catalog", p.id, p.endpoint));
            let domain = crate::proxy::default_domains()
                .into_iter()
                .find(|d| d.slug == resolved.slug)
                .expect("resolved slug is a catalog domain");
            assert!(
                domain.rewrite_prefixes.iter().any(|pre| {
                    // The client path is only the *start* of what the tool
                    // sends - the SDK appends its own leaf (`/messages`,
                    // `/chat/completions`). So the two must sit on the same
                    // path branch, in either direction: `/v1` vs
                    // `/v1/messages` (prefix extends it) and `/api/v1` vs
                    // `/api/` (path extends the prefix) are both fine.
                    pre.starts_with(resolved.client_path.as_str())
                        || resolved.client_path.starts_with(pre.as_str())
                }),
                "{}: client path {:?} is on no branch of {:?}, so its traffic would \
                 pass through to the user's own account instead of routing via Gate",
                p.id,
                resolved.client_path,
                domain.rewrite_prefixes
            );
        }
    }

    #[test]
    fn reconnect_preserves_original_snapshot() {
        let mut providers = Map::new();
        providers.insert(
            "anthropic".to_string(),
            json!({ "apiKey": "user-original" }),
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

        // Second connect with a different relay port — snapshot must NOT
        // change to reflect our own intermediate (post-first-connect) state.
        apply_override(
            &mut providers,
            &mut state,
            &target,
            "http://127.0.0.1:9002/v1",
        );
        let second_snapshot = state.providers["anthropic"].previous_headers.clone();
        assert_eq!(first_snapshot, second_snapshot);
        // baseUrl on disk reflects the latest connect.
        assert_eq!(
            providers["anthropic"]["baseUrl"],
            json!("http://127.0.0.1:9002/v1")
        );
    }

    /// Build a `State` whose `providers` map carries the given IDs (snapshot
    /// contents are irrelevant to `compute_status`, which only reads the keys).
    fn state_with(ids: &[&str]) -> State {
        let mut state = State::default();
        for id in ids {
            state
                .providers
                .insert((*id).to_string(), ProviderSnapshot::default());
        }
        state
    }

    /// A `models.providers` settings map with the given providers, each given a
    /// (baseUrl, has_gate_headers) shape. "Gate headers" now means just the
    /// non-secret upstream hint - the relay injects the credential live.
    fn settings_with(providers: &[(&str, &str, bool)]) -> Map<String, Value> {
        let mut provider_map = Map::new();
        for (id, base_url, gate_headers) in providers {
            let mut block = json!({ "baseUrl": base_url });
            if *gate_headers {
                block["headers"] = json!({
                    "X-Gate-Upstream-Url": "https://api.anthropic.com",
                });
            }
            provider_map.insert((*id).to_string(), block);
        }
        let mut settings = Map::new();
        settings.insert("models".to_string(), json!({ "providers": provider_map }));
        settings
    }

    #[test]
    fn compute_status_connected_when_all_providers_carry_gate_headers() {
        let state = state_with(&["anthropic", "openai"]);
        let settings = settings_with(&[
            ("anthropic", "https://gw.example.com/anthropic/v1", true),
            ("openai", "https://gw.example.com/openai/v1", true),
        ]);
        assert_eq!(
            compute_status(&state, &settings, "https://gw.example.com"),
            Status::Connected
        );
    }

    #[test]
    fn compute_status_partial_drift_when_one_provider_hand_edited() {
        // openai still routes via Gate; anthropic's baseUrl was changed back.
        let state = state_with(&["anthropic", "openai"]);
        let settings = settings_with(&[
            ("anthropic", "https://api.anthropic.com", true),
            ("openai", "https://gw.example.com/openai/v1", true),
        ]);
        match compute_status(&state, &settings, "https://gw.example.com") {
            Status::Drifted(msg) => {
                assert!(msg.contains("edited by hand"), "unexpected message: {msg}");
                assert!(
                    msg.contains("anthropic"),
                    "should name the drifted provider: {msg}"
                );
                assert!(
                    !msg.contains("openai"),
                    "healthy provider should not be listed: {msg}"
                );
            }
            other => panic!("expected partial drift, got {other:?}"),
        }
    }

    #[test]
    fn compute_status_full_drift_when_no_provider_carries_headers() {
        // User wiped our edits: provider exists but has no Gate headers.
        let state = state_with(&["anthropic"]);
        let settings = settings_with(&[("anthropic", "https://api.anthropic.com", false)]);
        match compute_status(&state, &settings, "https://gw.example.com") {
            Status::Drifted(msg) => {
                assert!(
                    msg.contains("no providers carry Gate headers"),
                    "unexpected message: {msg}"
                );
                assert!(
                    msg.contains("anthropic"),
                    "should list expected provider: {msg}"
                );
            }
            other => panic!("expected full drift, got {other:?}"),
        }
    }

    #[test]
    fn auth_profile_providers_discovers_plugin_providers() {
        // Real-world shape: `models.providers` is empty and providers are set
        // up via `openclaw models auth login`, which only writes auth profiles.
        // Discovery must still find anthropic + openrouter here.
        let settings = json!({
            "models": { "providers": {} },
            "auth": {
                "profiles": {
                    "anthropic:default": { "mode": "token", "provider": "anthropic" },
                    "google:default": { "provider": "google", "mode": "api_key" },
                    "openrouter:default": { "provider": "openrouter", "mode": "api_key" },
                }
            }
        });
        let settings = settings.as_object().unwrap().clone();

        let found = auth_profile_providers(&settings);
        assert!(found.contains(&"anthropic"), "got {found:?}");
        assert!(found.contains(&"openrouter"), "got {found:?}");
        // `google` is surfaced by the helper but filtered out later by
        // KNOWN_PROVIDERS — the helper itself stays generic.
        assert!(found.contains(&"google"), "got {found:?}");

        // Connect's candidate filter intersects with KNOWN_PROVIDERS.
        let candidates: Vec<&str> = KNOWN_PROVIDERS
            .iter()
            .map(|p| p.id)
            .filter(|id| found.contains(id))
            .collect();
        assert!(candidates.contains(&"anthropic"));
        assert!(candidates.contains(&"openrouter"));
        assert!(!candidates.contains(&"google"));
    }

    #[test]
    fn auth_profile_providers_empty_when_no_auth_block() {
        let settings = Map::new();
        assert!(auth_profile_providers(&settings).is_empty());
    }
}
