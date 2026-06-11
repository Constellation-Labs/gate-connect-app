//! OpenCode integration.
//!
//! Unlike Claude Code / Codex / Cowork — which each target one upstream
//! provider — OpenCode is multi-provider by design. Users authenticate
//! to OpenAI, Anthropic, OpenRouter, etc. via `opencode auth login
//! <provider>` and each provider's request goes straight to its native
//! endpoint. Gate Connect's job here is purely to *intercept* those
//! existing requests: redirect them through Constellation Gate and add
//! the two Gate identification headers. The user's existing upstream
//! auth (API key from auth.json, OAuth bearer, etc.) is untouched —
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
//!       "baseURL": "<gateway>/v1",
//!       "headers": {
//!         "X-Gate-Api-Key": "...",
//!         "X-Gate-Upstream-Url": "<original provider host>",
//!         ...any pre-existing headers the user had
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! The provider's `apiKey` , its model list, and any
//! other options the user set survive the merge. OpenCode keeps using
//! the original provider's npm package (@ai-sdk/anthropic for anthropic,
//! @ai-sdk/openai for openai, etc.), so SDK-specific wire formats stay
//! correct — Gate just sees whatever the SDK sends and forwards.
//!
//! Discovery: a provider is gated if EITHER it appears in opencode.json's
//! provider map OR it has an entry in `~/.local/share/opencode/auth.json`.
//! That captures both built-in providers the user has logged into and
//! provider blocks the user added by hand. Local-only providers (e.g.
//! `llamacpp`) and user-custom gateway shapes (e.g. their own `gateway`
//! provider) are not in our well-known list and stay untouched.
//!
//! State tracking: OpenCode's JSON schema is strict
//! (`additionalProperties: false`), so we can't stash markers inside
//! opencode.json. Snapshots of each gated provider's original
//! `baseURL` + `headers` live in a sidecar at
//! `<app_support_dir>/opencode-state.json` so disconnect restores the
//! file byte-equivalent.

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

/// Sidecar state file. OpenCode's JSON schema rejects unknown top-level
/// keys, so we can't keep state inside opencode.json itself.
const STATE_FILENAME: &str = "opencode-state.json";

/// Path suffix added to the user's gateway URL when we override per-provider
/// baseURLs. Every AI SDK provider we target appends a specific path
/// (`/messages`, `/chat/completions`, `/responses`) onto baseURL, and all
/// of them sit under `/v1` on the upstream side. Gate forwards the full
/// path verbatim to upstream, so the `/v1` segment has to live in baseURL.
const GATEWAY_PATH_SUFFIX: &str = "/v1";

/// Providers we know how to redirect through Gate. For each, the
/// `upstream_url` is the bare-host form expected by `X-Gate-Upstream-Url`
/// — Gate concatenates the inbound request path onto it, so trailing
/// `/v1` lives on the client side (in baseURL), never here.
struct KnownProvider {
    id: &'static str,
    upstream_url: &'static str,
}

/// The list is intentionally short. Adding a new entry means knowing
/// the canonical upstream URL and that the upstream speaks a wire format
/// Gate can forward. Users with custom providers
/// can still wire them up by hand.
///
/// Upstream URLs are bare-host form — Gate concatenates the inbound
/// request path verbatim. The OpenCode Zen / Go endpoints intentionally
/// host both OpenAI- and Anthropic-shape endpoints under the same prefix
/// (`/v1/chat/completions` and `/v1/messages`), so a single base URL
/// works for either AI-SDK flavor.
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
    // OpenCode Zen — opencode.ai/zen/v1/{chat/completions,messages,models}
    KnownProvider {
        id: "opencode",
        upstream_url: "https://opencode.ai/zen",
    },
    // OpenCode Go — opencode.ai/zen/go/v1/{chat/completions,messages}
    KnownProvider {
        id: "opencode-go",
        upstream_url: "https://opencode.ai/zen/go",
    },
];

/// Skip a provider if its currently-configured `baseURL` looks
/// non-public — localhost / loopback / private network / plain HTTP.
/// Protects against the edge case where someone names a local OpenAI-
/// compatible server `openai` and would otherwise have their endpoint
/// silently swapped to the Gate URL. `llamacpp`, `gateway`, and any
/// other ID that's not in `KNOWN_PROVIDERS` is already untouched by
/// design — this guard exists only for collisions inside the allowlist.
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
    /// One entry per provider we modified, keyed by provider ID. Each
    /// entry records exactly what `options.baseURL` and `options.headers`
    /// looked like *before* we touched it, so disconnect can restore
    /// byte-identical values . BTreeMap so
    /// the on-disk JSON stays deterministic.
    #[serde(default)]
    providers: BTreeMap<String, ProviderSnapshot>,
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
}

/// Likely install locations of the `opencode` binary. Detection falls
/// back to the config dir if none match, so npm/Bun/Volta layouts that
/// land binaries in per-user prefixes still register as installed.
#[cfg(target_os = "macos")]
const CLI_BIN_PATHS: &[&str] = &["/opt/homebrew/bin/opencode", "/usr/local/bin/opencode"];
#[cfg(all(unix, not(target_os = "macos")))]
const CLI_BIN_PATHS: &[&str] = &["/usr/local/bin/opencode", "/usr/bin/opencode"];
#[cfg(windows)]
const CLI_BIN_PATHS: &[&str] = &[];

pub struct OpenCode;

impl Integration for OpenCode {
    fn id(&self) -> ToolId {
        ToolId::OpenCode
    }

    fn display_name(&self) -> &'static str {
        "OpenCode"
    }

    fn upstream_provider_name(&self) -> &'static str {
        UPSTREAM_PROVIDER_NAME
    }

    fn default_upstream_url(&self) -> &'static str {
        DEFAULT_UPSTREAM_URL
    }

    fn requires_upstream_credential(&self) -> bool {
        // OpenCode already has the user's upstream credentials
        // (`opencode auth login <provider>`). Gate is a pure passthrough
        // on `Authorization` / `x-api-key`, so no separate key here.
        false
    }

    fn detect(&self) -> Result<bool> {
        if CLI_BIN_PATHS.iter().any(|p| Path::new(p).exists()) {
            return Ok(true);
        }
        Ok(env::opencode_config_dir()?.exists())
    }

    fn status(&self) -> Result<Status> {
        if !self.detect()? {
            return Ok(Status::NotInstalled);
        }
        // Connected = sidecar state exists AND at least one provider
        // currently carries Gate's two headers. If state exists but no
        // provider has our headers anymore, the user edited opencode.json
        // by hand — surface that as drift.
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
                    "Gate Connect is not signed in — sign in to validate OpenCode config".into(),
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
            let headers = options
                .and_then(|o| o.get("headers"))
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
            return Ok(Status::Drifted(format!(
                "no providers carry Gate headers; expected: {}",
                state
                    .providers
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        if !drifted.is_empty() {
            return Ok(Status::Drifted(format!(
                "some providers were edited by hand and no longer route via Gate: {}",
                drifted.join(", ")
            )));
        }
        Ok(Status::Connected)
    }

    fn connect(&self, input: &ConnectInput) -> Result<()> {
        if !self.detect()? {
            anyhow::bail!(
                "OpenCode is not installed on this machine — install it from https://opencode.ai first"
            );
        }
        if !input.gateway_base_url.starts_with("https://") {
            anyhow::bail!("gateway base URL must be https://");
        }

        let acct = account::load()?
            .context("Gate Connect is not signed in (no account.json + keychain entry)")?;

        let mut settings = load_settings()?.unwrap_or_default();
        let auth = load_opencode_auth().unwrap_or_default();

        // Figure out which well-known providers to route through Gate:
        // anything the user has either pre-configured in opencode.json OR
        // logged into via `opencode auth login`.
        let configured_in_settings: Vec<&str> = settings
            .get("provider")
            .and_then(|v| v.as_object())
            .map(|m| m.keys().map(String::as_str).collect())
            .unwrap_or_default();
        let configured_in_auth: Vec<&str> = auth.keys().map(String::as_str).collect();

        let candidates: Vec<&KnownProvider> = KNOWN_PROVIDERS
            .iter()
            .filter(|p| {
                configured_in_settings.contains(&p.id) || configured_in_auth.contains(&p.id)
            })
            .collect();

        // Local-protection guard: if a candidate has an existing
        // non-public `baseURL`, skip it — the user pointed it at a
        // private endpoint on purpose and we should not redirect that
        // traffic through Gate. Local providers like `llamacpp` and
        // user-custom gateways like `gateway` are already outside
        // KNOWN_PROVIDERS, so this only ever fires on collisions
        // inside the allowlist.
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
                "No supported OpenCode providers found to route through Gate. Run `opencode auth login anthropic|openai|openrouter|opencode|opencode-go` first, then re-run connect."
            );
        }
        if !skipped_local.is_empty() {
            eprintln!(
                "note: skipping providers that look local / self-hosted: {}. Their baseURL stays as-is. Disconnect-then-reconnect after pointing them at a public endpoint to route them via Gate.",
                skipped_local.join(", ")
            );
        }

        // Load any existing state — re-connects must preserve the
        // *original* snapshots, not overwrite them with our intermediate
        // values from the previous connect.
        let mut state = load_state()?.unwrap_or_default();

        let gateway_base_url = compute_base_url(&input.gateway_base_url);
        let provider_map = ensure_object(&mut settings, "provider");

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

        if let Some(provider_map) = settings.get_mut("provider").and_then(|v| v.as_object_mut()) {
            for (provider_id, snapshot) in &state.providers {
                restore_provider(provider_map, provider_id, snapshot);
            }
            if provider_map.is_empty() {
                settings.remove("provider");
            }
        }

        if settings.is_empty() {
            let path = env::opencode_config_path()?;
            if path.exists() {
                fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
            }
            remove_state()?;
            return Ok(());
        }
        write_settings(&settings)?;
        // Only drop the sidecar once the restored config is on disk: losing
        // it before a failed write would leave Gate headers in opencode.json
        // while status reports the tool as clean and re-disconnect no-ops.
        remove_state()
    }

    fn refresh_gate_key(&self, api_key: &str) -> Result<()> {
        // Only rewrite state we own: the sidecar lists exactly the
        // providers connect() stamped with Gate headers.
        let Some(state) = load_state()? else {
            return Ok(());
        };
        let Some(mut settings) = load_settings()? else {
            return Ok(());
        };
        let mut changed = false;
        if let Some(provider_map) = settings.get_mut("provider").and_then(|v| v.as_object_mut()) {
            for provider_id in state.providers.keys() {
                let key_slot = provider_map
                    .get_mut(provider_id)
                    .and_then(|v| v.as_object_mut())
                    .and_then(|p| p.get_mut("options"))
                    .and_then(|v| v.as_object_mut())
                    .and_then(|o| o.get_mut("headers"))
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
            "OpenCode does not need a separate upstream credential — Gate Connect adds its headers to whatever provider(s) you've already authenticated to via `opencode auth login`."
        )
    }

    fn has_upstream_credential(&self) -> Result<bool> {
        Ok(true)
    }

    fn clear_upstream_credential(&self) -> Result<()> {
        Ok(())
    }
}

/// Snapshot the provider's current state, then overwrite `options.baseURL`
/// and merge our two `X-Gate-*` headers into `options.headers`. Existing
/// `options.apiKey`, model lists, and any other fields survive untouched.
fn apply_override(
    provider_map: &mut Map<String, Value>,
    state: &mut State,
    target: &KnownProvider,
    gateway_base_url: &str,
    gate_api_key: &str,
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
        Value::String(gateway_base_url.to_string()),
    );

    let headers_entry = options
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

/// Append the SDK's expected path suffix (`/v1`) to the user's gateway
/// URL. Trims trailing slash, and avoids doubling the suffix if the user
/// pasted a gateway URL that already ends in `/v1`.
fn compute_base_url(gateway: &str) -> String {
    let trimmed = gateway.trim_end_matches('/');
    if trimmed.ends_with(GATEWAY_PATH_SUFFIX) {
        trimmed.to_string()
    } else {
        format!("{trimmed}{GATEWAY_PATH_SUFFIX}")
    }
}

// --- file I/O ---------------------------------------------------------

fn settings_path() -> Result<PathBuf> {
    env::opencode_config_path()
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
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {} as JSON", path.display()))?;
    match value {
        Value::Object(m) => Ok(Some(m)),
        _ => anyhow::bail!("{} top level must be a JSON object", path.display()),
    }
}

fn write_settings(settings: &Map<String, Value>) -> Result<()> {
    let path = settings_path()?;
    let mut body = serde_json::to_string_pretty(settings).context("serializing opencode.json")?;
    body.push('\n');
    // 0o600: this file holds the Gate API key under
    // `provider.<id>.options.headers`. Atomic-write protects against
    // partial writes corrupting JSON on crash.
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
    let path = env::opencode_auth_path()?;
    if !path.exists() {
        return Ok(Map::new());
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Map::new());
    }
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {} as JSON", path.display()))?;
    match value {
        Value::Object(m) => Ok(m),
        _ => anyhow::bail!("{} top level must be a JSON object", path.display()),
    }
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
    fn apply_override_adds_gate_headers_without_clobbering_user_options() {
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
        let opts = anth["options"].as_object().unwrap();
        // baseURL overwritten to gateway
        assert_eq!(opts["baseURL"], json!("https://gw.example.com/v1"));
        // user's apiKey survived
        assert_eq!(opts["apiKey"], json!("{env:ANTHROPIC_API_KEY}"));
        // user's existing header preserved alongside Gate headers
        let hdrs = opts["headers"].as_object().unwrap();
        assert_eq!(hdrs["X-User-Custom"], json!("yes"));
        assert_eq!(hdrs[GATE_KEY_HEADER], json!("sk-gw-xxx"));
        assert_eq!(
            hdrs[UPSTREAM_URL_HEADER],
            json!("https://api.anthropic.com")
        );
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
            upstream_url: "https://api.openai.com",
        };
        apply_override(
            &mut providers,
            &mut state,
            &target,
            "https://gw.example.com/v1",
            "sk-gw-xxx",
        );

        // Connect mutated options.baseURL and added Gate headers — confirm.
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

        restore_provider(&mut providers, "openai", &snap);
        assert!(
            !providers.contains_key("openai"),
            "freshly-created provider should be removed on disconnect"
        );
    }

    #[test]
    fn looks_local_classifies_common_endpoints() {
        // Public clouds — should NOT look local.
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
            "http://54.188.228.109:3000", // plain http to a public IP — still treated as not-Gate-friendly
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
        // Bare-host form — Gate concatenates the request path.
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
            json!({ "options": { "apiKey": "user-original" }}),
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
            providers["anthropic"]["options"]["headers"][GATE_KEY_HEADER],
            json!("key2")
        );
        assert_eq!(
            providers["anthropic"]["options"]["baseURL"],
            json!("https://gw2.example/v1")
        );
    }
}
