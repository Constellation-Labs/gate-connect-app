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
//!       "baseUrl": "<gateway>/v1",
//!       "headers": {
//!         "X-Gate-Api-Key": "...",
//!         "X-Gate-Upstream-Url": "<original provider host>",
//!         // ...any pre-existing headers the user had
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! The provider's `apiKey`, `api`, model list, and any other options the
//! user set survive the merge.
//!
//! Discovery: a provider is gated if it appears in `models.providers` and is
//! in our well-known list. OpenClaw resolves provider credentials from env
//! vars rather than an auth file, so (unlike OpenCode) there is no auth.json
//! to consult — a provider the user drives purely via `<PROVIDER>_API_KEY`
//! with no `models.providers.<id>` block isn't auto-discovered.
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

use crate::account;
use crate::env;
use crate::registry::{ConnectInput, Integration, Status, ToolId};

const UPSTREAM_PROVIDER_NAME: &str = "your existing providers";
const DEFAULT_UPSTREAM_URL: &str = "https://api.anthropic.com";

const GATE_KEY_HEADER: &str = "X-Gate-Api-Key";
const UPSTREAM_URL_HEADER: &str = "X-Gate-Upstream-Url";

/// Sidecar state file. We keep state out of the user-owned `openclaw.json`.
const STATE_FILENAME: &str = "openclaw-state.json";

/// Path suffix added to the user's gateway URL when we override per-provider
/// baseUrls. Upstream provider APIs sit under `/v1`, and Gate forwards the
/// full path verbatim to upstream, so the `/v1` segment has to live in the
/// client-side baseUrl. Matches OpenClaw's own documented provider examples
/// (e.g. `baseUrl: "https://api.moonshot.ai/v1"`).
const GATEWAY_PATH_SUFFIX: &str = "/v1";

/// Providers we know how to redirect through Gate. For each, the
/// `upstream_url` is the bare-host form expected by `X-Gate-Upstream-Url` —
/// Gate concatenates the inbound request path onto it, so trailing `/v1`
/// lives on the client side (in baseUrl), never here.
struct KnownProvider {
    id: &'static str,
    upstream_url: &'static str,
}

/// The list is intentionally short — matching the set Gate already forwards
/// (cf. `provider.rs` proxy domains). Adding a new entry means knowing the
/// canonical upstream URL and that the upstream speaks a wire format Gate can
/// forward. The IDs are OpenClaw's provider IDs. Users with custom providers
/// can still wire them up by hand.
const KNOWN_PROVIDERS: &[KnownProvider] = &[
    KnownProvider {
        id: "anthropic",
        upstream_url: "https://api.anthropic.com",
    },
    KnownProvider {
        id: "openai",
        upstream_url: "https://api.openai.com",
    },
    KnownProvider {
        id: "openrouter",
        upstream_url: "https://openrouter.ai/api",
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

        let expected_base = match account::load_base_url()? {
            Some(u) => compute_base_url(&u),
            None => {
                return Ok(Status::Drifted(
                    "Gate Connect is not signed in — sign in to validate OpenClaw config".into(),
                ));
            }
        };

        Ok(compute_status(&state, &settings, &expected_base))
    }

    fn connect(&self, input: &ConnectInput) -> Result<()> {
        if !self.detect()? {
            anyhow::bail!(
                "OpenClaw is not installed on this machine — install it from https://docs.openclaw.ai first"
            );
        }
        if !input.gateway_base_url.starts_with("https://") {
            anyhow::bail!("gateway base URL must be https://");
        }

        let acct = account::load()?
            .context("Gate Connect is not signed in (no account.json + keychain entry)")?;

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

        let gateway_base_url = compute_base_url(&input.gateway_base_url);
        let provider_map = ensure_provider_map(&mut settings);

        for target in &targets {
            apply_override(
                provider_map,
                &mut state,
                target,
                &gateway_base_url,
                &acct.api_key,
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

    fn refresh_gate_key(&self, api_key: &str) -> Result<()> {
        // Only rewrite state we own: the sidecar lists exactly the providers
        // connect() stamped with Gate headers.
        let Some(state) = load_state()? else {
            return Ok(());
        };
        let Some(mut settings) = load_settings()? else {
            return Ok(());
        };
        let mut changed = false;
        if let Some(provider_map) = provider_map_mut(&mut settings) {
            for provider_id in state.providers.keys() {
                let key_slot = provider_map
                    .get_mut(provider_id)
                    .and_then(|v| v.as_object_mut())
                    .and_then(|p| p.get_mut("headers"))
                    .and_then(|v| v.as_object_mut())
                    .and_then(|h| h.get_mut(GATE_KEY_HEADER));
                if let Some(slot) = key_slot {
                    *slot = Value::String(api_key.to_string());
                    changed = true;
                }
            }
        }
        if !changed {
            return Ok(());
        }
        write_settings(&settings)
    }

    fn save_upstream_credential(&self, _credential: &str) -> Result<()> {
        anyhow::bail!(
            "OpenClaw does not need a separate upstream credential — Gate Connect adds its headers to whatever provider(s) you've already configured in ~/.openclaw/openclaw.json."
        )
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
fn compute_status(state: &State, settings: &Map<String, Value>, expected_base: &str) -> Status {
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
        let headers = block
            .and_then(|b| b.get("headers"))
            .and_then(|v| v.as_object());
        let has_key = headers
            .and_then(|h| h.get(GATE_KEY_HEADER))
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        let has_upstream = headers
            .and_then(|h| h.get(UPSTREAM_URL_HEADER))
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        if base_url == expected_base && has_key && has_upstream {
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

/// Snapshot the provider's current state, then overwrite `baseUrl` and merge
/// our two `X-Gate-*` headers into `headers`. Existing `apiKey`, `api`, model
/// lists, and any other fields survive untouched.
fn apply_override(
    provider_map: &mut Map<String, Value>,
    state: &mut State,
    target: &KnownProvider,
    gateway_base_url: &str,
    gate_api_key: &str,
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
        Value::String(gateway_base_url.to_string()),
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

    let headers_entry = provider
        .entry("headers".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !headers_entry.is_object() {
        *headers_entry = Value::Object(Map::new());
    }
    let headers = headers_entry
        .as_object_mut()
        .expect("headers entry was just coerced to an object");
    headers.insert(
        GATE_KEY_HEADER.to_string(),
        Value::String(gate_api_key.to_string()),
    );
    headers.insert(
        UPSTREAM_URL_HEADER.to_string(),
        Value::String(target.upstream_url.to_string()),
    );
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

/// Append the SDK's expected path suffix (`/v1`) to the user's gateway URL.
/// Trims trailing slash, and avoids doubling the suffix if the user pasted a
/// gateway URL that already ends in `/v1`.
fn compute_base_url(gateway: &str) -> String {
    let trimmed = gateway.trim_end_matches('/');
    if trimmed.ends_with(GATEWAY_PATH_SUFFIX) {
        trimmed.to_string()
    } else {
        format!("{trimmed}{GATEWAY_PATH_SUFFIX}")
    }
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
    // 0o600: this file holds the Gate API key under
    // `models.providers.<id>.headers`. Atomic-write protects against partial
    // writes corrupting the config on crash.
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
    fn compute_base_url_appends_v1_suffix() {
        assert_eq!(
            compute_base_url("https://gw.example.com"),
            "https://gw.example.com/v1"
        );
        assert_eq!(
            compute_base_url("https://gw.example.com/"),
            "https://gw.example.com/v1"
        );
    }

    #[test]
    fn compute_base_url_does_not_double_v1_suffix() {
        assert_eq!(
            compute_base_url("https://gw.example.com/v1"),
            "https://gw.example.com/v1"
        );
        assert_eq!(
            compute_base_url("https://gw.example.com/v1/"),
            "https://gw.example.com/v1"
        );
    }

    #[test]
    fn apply_override_adds_gate_headers_without_clobbering_user_fields() {
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
            upstream_url: "https://api.anthropic.com",
        };
        apply_override(
            &mut providers,
            &mut state,
            &target,
            "https://gw.example.com/v1",
            "sk-gw-xxx",
        );

        let anth = providers["anthropic"].as_object().unwrap();
        // baseUrl overwritten to gateway
        assert_eq!(anth["baseUrl"], json!("https://gw.example.com/v1"));
        // user's apiKey + api survived
        assert_eq!(anth["apiKey"], json!("${ANTHROPIC_API_KEY}"));
        assert_eq!(anth["api"], json!("anthropic-messages"));
        // user's existing header preserved alongside Gate headers
        let hdrs = anth["headers"].as_object().unwrap();
        assert_eq!(hdrs["X-User-Custom"], json!("yes"));
        assert_eq!(hdrs[GATE_KEY_HEADER], json!("sk-gw-xxx"));
        assert_eq!(
            hdrs[UPSTREAM_URL_HEADER],
            json!("https://api.anthropic.com")
        );
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
            upstream_url: "https://api.openai.com",
        };
        apply_override(
            &mut providers,
            &mut state,
            &target,
            "https://gw.example.com/v1",
            "sk-gw-xxx",
        );

        // Connect mutated baseUrl and added Gate headers — confirm.
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
            upstream_url: "https://api.openai.com",
        };
        apply_override(
            &mut providers,
            &mut state,
            &target,
            "https://gw.example.com/v1",
            "sk-gw-xxx",
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
    fn known_providers_are_bare_https_hosts() {
        for p in KNOWN_PROVIDERS {
            assert!(
                !p.upstream_url.ends_with("/v1"),
                "{} upstream must be bare host",
                p.id
            );
            assert!(
                !p.upstream_url.ends_with('/'),
                "{} upstream must not end in /",
                p.id
            );
            assert!(
                p.upstream_url.starts_with("https://"),
                "{} must be https",
                p.id
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
            upstream_url: "https://api.anthropic.com",
        };

        apply_override(
            &mut providers,
            &mut state,
            &target,
            "https://gw1.example/v1",
            "key1",
        );
        let first_snapshot = state.providers["anthropic"].previous_headers.clone();

        // Second connect with a different gateway / key — snapshot must NOT
        // change to reflect our own intermediate (post-first-connect) state.
        apply_override(
            &mut providers,
            &mut state,
            &target,
            "https://gw2.example/v1",
            "key2",
        );
        let second_snapshot = state.providers["anthropic"].previous_headers.clone();
        assert_eq!(first_snapshot, second_snapshot);
        // Headers on disk reflect the latest connect.
        assert_eq!(
            providers["anthropic"]["headers"][GATE_KEY_HEADER],
            json!("key2")
        );
        assert_eq!(
            providers["anthropic"]["baseUrl"],
            json!("https://gw2.example/v1")
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
    /// (baseUrl, has_gate_headers) shape.
    fn settings_with(providers: &[(&str, &str, bool)]) -> Map<String, Value> {
        let mut provider_map = Map::new();
        for (id, base_url, gate_headers) in providers {
            let mut block = json!({ "baseUrl": base_url });
            if *gate_headers {
                block["headers"] = json!({
                    GATE_KEY_HEADER: "sk-gw-xxx",
                    UPSTREAM_URL_HEADER: "https://api.anthropic.com",
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
            ("anthropic", "https://gw.example.com/v1", true),
            ("openai", "https://gw.example.com/v1", true),
        ]);
        assert_eq!(
            compute_status(&state, &settings, "https://gw.example.com/v1"),
            Status::Connected
        );
    }

    #[test]
    fn compute_status_partial_drift_when_one_provider_hand_edited() {
        // openai still routes via Gate; anthropic's baseUrl was changed back.
        let state = state_with(&["anthropic", "openai"]);
        let settings = settings_with(&[
            ("anthropic", "https://api.anthropic.com", true),
            ("openai", "https://gw.example.com/v1", true),
        ]);
        match compute_status(&state, &settings, "https://gw.example.com/v1") {
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
        match compute_status(&state, &settings, "https://gw.example.com/v1") {
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
