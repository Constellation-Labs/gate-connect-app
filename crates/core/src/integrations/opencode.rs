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
//!       "baseURL": "http://127.0.0.1:<relay-port>/<slug><client-path>"
//!     }
//!   }
//! }
//! ```
//!
//! That single value is the entire write. Both parts come from
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

use crate::env;
use crate::registry::{ConnectInput, Integration, Status, ToolId};

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
    // OpenCode Zen - opencode.ai/zen/v1/{chat/completions,messages,models}
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
        Ok(state.providers.keys().any(|provider_id| {
            settings
                .get("provider")
                .and_then(|v| v.as_object())
                .and_then(|m| m.get(provider_id))
                .and_then(|v| v.as_object())
                .and_then(|b| b.get("options"))
                .and_then(|v| v.as_object())
                .and_then(|o| o.get("baseURL"))
                .and_then(|v| v.as_str())
                .is_some_and(looks_local)
        }))
    }

    fn status(&self) -> Result<Status> {
        if !self.detect()? {
            return Ok(Status::NotInstalled);
        }
        // Connected = sidecar state exists AND at least one provider
        // currently carries Gate's two headers. If state exists but no
        // provider has our headers anymore, the user edited opencode.json
        // by hand - surface that as drift.
        let Some(state) = load_state()? else {
            return Ok(Status::Detected);
        };
        if state.providers.is_empty() {
            return Ok(Status::Detected);
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
                "OpenCode is not installed on this machine - install it from https://opencode.ai first"
            );
        }
        let relay_base_url = input.relay_base_url.as_deref().context(
            "the Gate proxy relay is not running - enable the proxy before connecting OpenCode",
        )?;

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
        // non-public `baseURL`, skip it - the user pointed it at a
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

        // Load any existing state - re-connects must preserve the
        // *original* snapshots, not overwrite them with our intermediate
        // values from the previous connect.
        let mut state = load_state()?.unwrap_or_default();

        let provider_map = ensure_object(&mut settings, "provider");

        // Each provider gets its own baseURL: the path Gate must not swallow
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
                "None of the configured OpenCode providers can route through Gate yet ({}). Gate has no upstream domain for them.",
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

    fn save_upstream_credential(&self, _credential: &str) -> Result<()> {
        anyhow::bail!(
            "OpenCode does not need a separate upstream credential - Gate Connect adds its headers to whatever provider(s) you've already authenticated to via `opencode auth login`."
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
    Some(resolved.relay_base_url(relay_base_url))
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
    // 0o600 defensively (the file no longer holds the Gate key - the relay
    // injects it - but may carry other user config). Atomic-write protects
    // against partial writes corrupting JSON on crash.
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
            Some("http://127.0.0.1:9977/openrouter/v1")
        );
        // Zen and Go keep their own paths under the shared opencode.ai upstream.
        assert_eq!(
            expected_base_url("opencode", "http://127.0.0.1:9977").as_deref(),
            Some("http://127.0.0.1:9977/opencode/zen/v1")
        );
        assert_eq!(
            expected_base_url("opencode-go", "http://127.0.0.1:9977").as_deref(),
            Some("http://127.0.0.1:9977/opencode/zen/go/v1")
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
}
