//! OpenAI Codex CLI integration.
//!
//! Configures `codex` to route through Constellation Gate by editing
//! `~/.codex/config.toml`. We add (or update) a `[model_providers.gate]`
//! block and flip top-level `model_provider = "gate"` so all requests
//! flow through that provider definition. `toml_edit` keeps the rest
//! of the file - comments, user-defined providers, profiles, etc. -
//! byte-identical.
//!
//! `base_url` points at the loopback reverse-proxy relay
//! ([`crate::proxy::relay`]), not the gateway: the relay injects the live
//! Gate credential per request and forwards to the gateway, so **no Gate
//! credential is written to config.toml**. `http_headers` carries only the
//! non-secret `X-Gate-Upstream-Url` hint. (The path suffix Codex needs -
//! `/codex` or `/v1` - is still appended to the relay base, since the relay
//! forwards the request path verbatim onto the gateway.)
//!
//! Like Claude Code, Codex brings its own upstream credentials. We set
//! `requires_openai_auth = true` on the provider so Codex attaches its
//! own `codex login` session - the ChatGPT OAuth token or the API key in
//! `~/.codex/auth.json` - as the upstream bearer. Per the Codex docs this
//! is the only provider shape that carries a ChatGPT-subscription login
//! through a custom `base_url` (a bare `[auth] command` helper works for
//! API keys but leaves ChatGPT-mode Codex falling back to its built-in
//! provider and hitting chatgpt.com directly). Gate passes the bearer
//! through and forwards to OpenAI per the `X-Gate-Upstream-Url` hint.
//! Therefore [`requires_upstream_credential`] is `false`.
//!
//! Codex reads `config.toml` at startup, so the user must restart any
//! running `codex` sessions after connecting/disconnecting.
//!
//! [`requires_upstream_credential`]: crate::Integration::requires_upstream_credential

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{value, DocumentMut, Item, Table, Value};

use crate::env;
use crate::primitives;
use crate::registry::{ConnectInput, Integration, Status, ToolId};

/// File name of the auth-helper script older Gate Connect versions wrote
/// and pointed Codex's `[auth] command` at. We no longer write it - Codex
/// now sources the upstream credential itself via `requires_openai_auth` -
/// but `disconnect` still deletes any leftover so an upgrade-then-disconnect
/// leaves zero residue.
#[cfg(unix)]
const HELPER_FILENAME: &str = "codex-credential-helper.sh";
#[cfg(windows)]
const HELPER_FILENAME: &str = "codex-credential-helper.cmd";

fn helper_script_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join(HELPER_FILENAME))
}

/// Auth mode Codex is currently logged in as. Determines the upstream URL
/// shape Gate Connect writes - ChatGPT bearer tokens only authenticate
/// against `chatgpt.com/backend-api/*`, API keys only against
/// `api.openai.com/v1/*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthMode {
    Chatgpt,
    Apikey,
}

impl AuthMode {
    /// Upstream URL for `X-Gate-Upstream-Url`. Gate is pure passthrough -
    /// it concatenates the incoming request path onto this base. So the
    /// upstream URL stops at the host (no `/v1` or `/codex`); the path
    /// suffix that lands in the request comes from
    /// [`Self::gateway_path_suffix`].
    fn upstream_url(self) -> &'static str {
        match self {
            AuthMode::Chatgpt => CHATGPT_UPSTREAM_URL,
            AuthMode::Apikey => APIKEY_UPSTREAM_URL,
        }
    }

    /// Path segment appended onto the user's gateway URL to form Codex's
    /// `base_url`. Codex itself then appends `/responses` (because
    /// `wire_api = "responses"`), so the request path that hits Gate is
    /// `<suffix>/responses`. Gate forwards that verbatim onto the
    /// upstream URL, yielding e.g.
    /// `https://chatgpt.com/backend-api/codex/responses`.
    fn gateway_path_suffix(self) -> &'static str {
        match self {
            // ChatGPT-mode Codex lives at /backend-api/codex/responses on
            // the upstream side, so the path the client sends needs to
            // start with /codex.
            AuthMode::Chatgpt => "/codex",
            // API-key mode hits the standard OpenAI /v1/responses path.
            AuthMode::Apikey => "/v1",
        }
    }
}

/// Read `~/.codex/auth.json` and report which auth mode Codex is in.
/// Missing/malformed file is an error here - connect() needs to know.
/// Anything other than `"apikey"` falls through to `chatgpt` (matches
/// Codex's own treatment in the credential helper).
fn read_auth_mode() -> Result<AuthMode> {
    let path = env::codex_auth_json_path()?;
    if !path.exists() {
        anyhow::bail!(
            "Codex isn't logged in yet - run `codex login` first, then retry the Gate Connect connect"
        );
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let parsed: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {} as JSON", path.display()))?;
    let mode = parsed
        .get("auth_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match mode {
        "apikey" => Ok(AuthMode::Apikey),
        _ => Ok(AuthMode::Chatgpt),
    }
}

/// Codex `base_url` - the relay loopback origin plus the auth-mode-specific
/// path suffix Codex appends `/responses` to. The relay (and Gate behind it)
/// forward the path verbatim, so this suffix has to match the upstream's path
/// layout (`chatgpt.com/backend-api/codex` vs. `api.openai.com/v1`). Trims any
/// trailing slash, and avoids doubling the suffix if `base` already includes it.
fn compute_base_url(base: &str, mode: AuthMode) -> String {
    let trimmed = base.trim_end_matches('/');
    let suffix = mode.gateway_path_suffix();
    if trimmed.ends_with(suffix) {
        trimmed.to_string()
    } else {
        format!("{trimmed}{suffix}")
    }
}

const UPSTREAM_PROVIDER_NAME: &str = "OpenAI";

/// Shown in the UI's "Advanced → Upstream URL" field. Codex actually
/// ignores whatever the user types here and recomputes the upstream URL
/// from `~/.codex/auth.json`'s `auth_mode` at connect time, since the
/// two auth modes have incompatible upstream URL shapes (api.openai.com
/// vs. chatgpt.com/backend-api). This default just matches the
/// API-key-mode case.
const DEFAULT_UPSTREAM_URL: &str = "https://api.openai.com/v1";

/// `auth_mode == "chatgpt"` → ChatGPT subscription login. The Responses
/// API for Codex lives at `https://chatgpt.com/backend-api/codex/responses`.
/// Gate concatenates the incoming request path onto this base, so the
/// upstream URL has NO `/codex` segment here - that comes from the
/// client-side path suffix below.
const CHATGPT_UPSTREAM_URL: &str = "https://chatgpt.com/backend-api";

/// `auth_mode == "apikey"` → user pasted an `sk-…` key. The Responses
/// API for API-key callers lives at `https://api.openai.com/v1/responses`.
/// Same passthrough rule: no `/v1` here, that lives in the path suffix.
const APIKEY_UPSTREAM_URL: &str = "https://api.openai.com";

/// Name of the provider block we own inside `[model_providers.*]`.
const PROVIDER_ID: &str = "gate";
const PROVIDER_DISPLAY_NAME: &str = "Constellation Gate";

const UPSTREAM_URL_HEADER: &str = "X-Gate-Upstream-Url";

/// Common install locations for the `codex` binary. Detection also falls
/// back to checking `~/.codex/` so Volta / asdf / npx layouts still flag
/// as installed even when none of these hard-coded paths match -- that
/// fallback is what Windows relies on entirely (Codex installs to a
/// per-user npm prefix that's effectively unguessable).
#[cfg(target_os = "macos")]
const CLI_BIN_PATHS: &[&str] = &["/opt/homebrew/bin/codex", "/usr/local/bin/codex"];
#[cfg(all(unix, not(target_os = "macos")))]
const CLI_BIN_PATHS: &[&str] = &["/usr/local/bin/codex", "/usr/bin/codex"];
#[cfg(windows)]
const CLI_BIN_PATHS: &[&str] = &[];

pub struct Codex;

impl Integration for Codex {
    fn id(&self) -> ToolId {
        ToolId::Codex
    }

    fn display_name(&self) -> &'static str {
        "Codex"
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
        Ok(env::codex_config_dir()?.exists())
    }

    fn status(&self) -> Result<Status> {
        if !self.detect()? {
            return Ok(Status::NotInstalled);
        }
        let path = config_path()?;
        if !path.exists() {
            return Ok(Status::Detected);
        }
        let doc = read_doc(&path)?;

        let provider_block = doc
            .get("model_providers")
            .and_then(|i| i.as_table_like())
            .and_then(|t| t.get(PROVIDER_ID))
            .and_then(|i| i.as_table_like());
        let model_provider = doc
            .get("model_provider")
            .and_then(|i| i.as_str())
            .unwrap_or("");

        let Some(provider_block) = provider_block else {
            return Ok(Status::Detected);
        };
        // Our provider block (with the embedded Gate key) is still present
        // even though the pointer was changed - that's drift, not a clean
        // machine; reporting Detected here would make sign-out skip the
        // residue.
        if model_provider != PROVIDER_ID {
            return Ok(Status::Drifted(format!(
                "[model_providers.{PROVIDER_ID}] is present but model_provider is {model_provider:?}"
            )));
        }

        // Require `requires_openai_auth = true`. A block without it is the
        // old `[auth] command` shape that left ChatGPT-mode Codex bypassing
        // the gateway - report drift so the user reconnects into the fix.
        let requires_openai_auth = provider_block
            .get("requires_openai_auth")
            .and_then(|i| i.as_bool())
            .unwrap_or(false);
        if !requires_openai_auth {
            return Ok(Status::Drifted(format!(
                "[model_providers.{PROVIDER_ID}] is missing requires_openai_auth = true"
            )));
        }

        // The provider points at the relay's loopback base; the relay only
        // exists once the proxy has been enabled.
        let relay_base = match crate::proxy::relay_base_url() {
            Some(u) => u,
            None => {
                return Ok(Status::Drifted(
                    "the Gate proxy has not been enabled yet - turn it on to route Codex".into(),
                ));
            }
        };
        // Accept whichever auth-mode shape is currently written. If auth.json
        // can't be read, fall back to ChatGPT (the only mode where the bug
        // bites - wrong base_url shape causes 404s; API-key mode just needs
        // an OPENAI_API_KEY to authenticate).
        let mode = read_auth_mode().unwrap_or(AuthMode::Chatgpt);
        let expected_base = compute_base_url(&relay_base, mode);
        let base_url = provider_block
            .get("base_url")
            .and_then(|i| i.as_str())
            .unwrap_or("");
        if base_url != expected_base {
            return Ok(Status::Drifted(format!(
                "[model_providers.{PROVIDER_ID}] base_url is {base_url:?}, expected {expected_base:?}"
            )));
        }

        // Only the non-secret upstream hint is written; the relay injects the
        // Gate credential live, so there is deliberately no key header here.
        let headers_table = provider_block
            .get("http_headers")
            .and_then(|i| i.as_table_like());
        let upstream = headers_table
            .and_then(|h| h.get(UPSTREAM_URL_HEADER))
            .and_then(|i| i.as_str())
            .unwrap_or("");
        if upstream.is_empty() {
            return Ok(Status::Drifted(format!(
                "[model_providers.{PROVIDER_ID}.http_headers] missing {UPSTREAM_URL_HEADER}"
            )));
        }
        let expected_upstream = mode.upstream_url();
        if upstream != expected_upstream {
            return Ok(Status::Drifted(format!(
                "{UPSTREAM_URL_HEADER} is {upstream:?}, expected {expected_upstream:?} for auth_mode {:?}",
                match mode { AuthMode::Chatgpt => "chatgpt", AuthMode::Apikey => "apikey" }
            )));
        }

        Ok(Status::Connected)
    }

    fn connect(&self, input: &ConnectInput) -> Result<()> {
        if !self.detect()? {
            anyhow::bail!(
                "Codex is not installed on this machine - install it from https://developers.openai.com/codex first"
            );
        }
        let relay_base = input.relay_base_url.as_deref().context(
            "the Gate proxy relay is not running - enable the proxy before connecting Codex",
        )?;
        // We intentionally ignore `input.upstream_url`. The ChatGPT-mode
        // bearer authenticates only against chatgpt.com/backend-api, the
        // apikey-mode bearer only against api.openai.com/v1, so we
        // compute both URLs from the current Codex login state instead
        // of trusting whatever value flowed through the UI/Advanced
        // field. This mirrors what Codex itself would have done in its
        // native (non-Gate) routing.
        let mode = read_auth_mode()?;

        let path = config_path()?;
        let mut doc = if path.exists() {
            read_doc(&path)?
        } else {
            DocumentMut::new()
        };

        // A [model_providers.gate] block without our `_gate_connect` marker
        // is a hand-written setup (the manual PAYG instructions had users
        // author exactly this block). Adopt it: the insert below replaces it
        // with the managed shape, and disconnect deletes it like any block
        // we wrote. Anything under that name targets our provider id, so
        // overwriting is the migration the user is asking for.

        // Stash the prior `model_provider` so disconnect can restore it.
        // Skip if we've already done this (re-connect mustn't clobber the
        // original snapshot with our own intermediate value). Check both
        // marker keys: a first connect over a config with no
        // `model_provider` records only `previous_model_provider_absent`,
        // and a re-connect that ignored it would re-snapshot our own
        // `"gate"` pointer - disconnect would then "restore" `model_provider
        // = "gate"` after deleting the provider block.
        let marker_has_prev = doc
            .get("_gate_connect")
            .and_then(|i| i.as_table_like())
            .map(|t| {
                t.contains_key("previous_model_provider")
                    || t.contains_key("previous_model_provider_absent")
            })
            .unwrap_or(false);
        // A pre-existing `"gate"` pointer is never worth restoring: it came
        // from a hand-written setup whose block we adopt and later delete,
        // so treat it like no prior value.
        let previous_model_provider = doc
            .get("model_provider")
            .and_then(|i| i.as_str())
            .filter(|s| *s != PROVIDER_ID)
            .map(|s| s.to_string());

        // Ensure [model_providers] table exists, then write/replace
        // [model_providers.gate]. If the user has it as an inline table
        // (`model_providers = { ... }`), upgrade to a regular table so we
        // can use `set_implicit` and uniform table operations - a bare
        // `as_table_mut` returns None for inline.
        let entry = doc
            .entry("model_providers")
            .or_insert_with(|| Item::Table(new_table()));
        upgrade_inline_to_table(entry);
        let model_providers = entry
            .as_table_mut()
            .context("`model_providers` must be a TOML table")?;
        model_providers.set_implicit(true);

        let base_url = compute_base_url(relay_base, mode);
        let upstream_url = mode.upstream_url();

        let mut provider = Table::new();
        provider.insert("name", value(PROVIDER_DISPLAY_NAME));
        provider.insert("base_url", value(base_url.as_str()));
        provider.insert("wire_api", value("responses"));
        // Codex sources the upstream bearer from its own `codex login`
        // session (ChatGPT OAuth token or API key in ~/.codex/auth.json)
        // and attaches it to this provider. This is the only mechanism
        // that carries a ChatGPT-subscription login through a custom
        // base_url - without it, ChatGPT-mode Codex ignores this provider
        // and hits chatgpt.com directly. Mutually exclusive with `env_key`
        // and `[auth] command` per the Codex docs, so we set neither.
        provider.insert("requires_openai_auth", value(true));

        // Only the non-secret upstream hint. The Gate credential is injected
        // live by the relay, never written here.
        let mut headers = Table::new();
        headers.set_implicit(false);
        headers.insert(UPSTREAM_URL_HEADER, value(upstream_url));
        provider.insert("http_headers", Item::Table(headers));

        model_providers.insert(PROVIDER_ID, Item::Table(provider));

        doc["model_provider"] = value(PROVIDER_ID);

        let marker_entry = doc
            .entry("_gate_connect")
            .or_insert_with(|| Item::Table(new_table()));
        upgrade_inline_to_table(marker_entry);
        let marker = marker_entry
            .as_table_mut()
            .context("`_gate_connect` must be a TOML table")?;
        if !marker_has_prev {
            match previous_model_provider {
                Some(s) => {
                    marker.insert("previous_model_provider", value(s));
                }
                None => {
                    // Sentinel for "no prior value" so disconnect knows
                    // to delete `model_provider` entirely rather than
                    // restoring an empty string.
                    marker.insert("previous_model_provider_absent", value(true));
                }
            }
        }
        // No Gate-managed provider list is recorded; the marker above is
        // sufficient.

        write_doc(&path, &doc)
    }

    fn disconnect(&self) -> Result<()> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(());
        }
        let mut doc = read_doc(&path)?;

        // Remove our model_providers.gate block. Use `as_table_like_mut`
        // so inline-table forms (`model_providers = { ... }`) are also
        // handled - `as_table_mut` would silently skip them and leave
        // our entry behind.
        let mut model_providers_empty = false;
        if let Some(model_providers) = doc
            .get_mut("model_providers")
            .and_then(|i| i.as_table_like_mut())
        {
            model_providers.remove(PROVIDER_ID);
            model_providers_empty = model_providers.is_empty();
        }
        if model_providers_empty {
            // Drop the table so the file doesn't end up with an orphan
            // `[model_providers]` header / empty inline value.
            doc.remove("model_providers");
        }

        // Restore the prior `model_provider` (or remove the key entirely
        // if there was none before).
        let (prev, absent) = doc
            .get("_gate_connect")
            .and_then(|i| i.as_table_like())
            .map(|t| {
                (
                    t.get("previous_model_provider")
                        .and_then(|i| i.as_str())
                        .map(|s| s.to_string()),
                    t.get("previous_model_provider_absent")
                        .and_then(|i| i.as_bool())
                        .unwrap_or(false),
                )
            })
            .unwrap_or((None, false));
        match (prev, absent) {
            (Some(s), _) => doc["model_provider"] = value(s),
            (None, true) => {
                doc.remove("model_provider");
            }
            (None, false) => {
                // No marker (or partial state) - best-effort: only remove
                // our pointer, don't risk clobbering an unrelated value.
                if doc.get("model_provider").and_then(|i| i.as_str()) == Some(PROVIDER_ID) {
                    doc.remove("model_provider");
                }
            }
        }
        doc.remove("_gate_connect");

        // Write the restored config before removing the helper script: a
        // failed write must not leave config.toml pointing `[auth] command`
        // at a script that no longer exists.
        if doc.as_table().is_empty() {
            // The user had nothing but our config; remove the file so we
            // truly leave no residue.
            fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        } else {
            write_doc(&path, &doc)?;
        }

        // Remove the legacy auth-helper script if an older Gate Connect
        // version left one behind - keep "zero residue" on disconnect.
        let helper = helper_script_path()?;
        if helper.exists() {
            fs::remove_file(&helper).with_context(|| format!("removing {}", helper.display()))?;
        }
        Ok(())
    }

    fn save_upstream_credential(&self, _credential: &str) -> Result<()> {
        anyhow::bail!(
            "Codex does not need a separate upstream credential - it reuses your `codex login` session"
        );
    }

    fn has_upstream_credential(&self) -> Result<bool> {
        Ok(true)
    }

    fn clear_upstream_credential(&self) -> Result<()> {
        Ok(())
    }
}

fn config_path() -> Result<PathBuf> {
    env::codex_config_toml_path()
}

fn read_doc(path: &Path) -> Result<DocumentMut> {
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    raw.parse::<DocumentMut>()
        .with_context(|| format!("parsing {} as TOML", path.display()))
}

fn write_doc(path: &Path, doc: &DocumentMut) -> Result<()> {
    // 0o600 defensively (the file no longer carries the Gate key - the relay
    // injects it - but may hold other user config). Atomic-write protects
    // against partial writes tearing the TOML on crash.
    primitives::write_file(path, doc.to_string().as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

fn new_table() -> Table {
    let mut t = Table::new();
    t.set_implicit(false);
    t
}

/// Upgrade `Item::Value(Value::InlineTable(_))` to `Item::Table(_)` in
/// place, preserving content. No-op for any other shape. Used when we
/// need uniform table-style operations on a field the user may have
/// authored as an inline table.
fn upgrade_inline_to_table(item: &mut Item) {
    if matches!(item, Item::Value(Value::InlineTable(_))) {
        if let Item::Value(Value::InlineTable(inline)) = std::mem::take(item) {
            *item = Item::Table(inline.into_table());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chatgpt_mode_appends_codex_path_suffix() {
        assert_eq!(
            compute_base_url("https://gw.example.com", AuthMode::Chatgpt),
            "https://gw.example.com/codex"
        );
        assert_eq!(
            compute_base_url("https://gw.example.com/", AuthMode::Chatgpt),
            "https://gw.example.com/codex"
        );
    }

    #[test]
    fn apikey_mode_appends_v1_path_suffix() {
        assert_eq!(
            compute_base_url("https://gw.example.com", AuthMode::Apikey),
            "https://gw.example.com/v1"
        );
        assert_eq!(
            compute_base_url("https://gw.example.com/", AuthMode::Apikey),
            "https://gw.example.com/v1"
        );
    }

    #[test]
    fn base_url_does_not_double_path_suffix_if_already_present() {
        assert_eq!(
            compute_base_url("https://gw.example.com/codex", AuthMode::Chatgpt),
            "https://gw.example.com/codex"
        );
        assert_eq!(
            compute_base_url("https://gw.example.com/v1", AuthMode::Apikey),
            "https://gw.example.com/v1"
        );
    }

    #[test]
    fn upstream_urls_are_bare_hosts_no_path_suffix() {
        // Gate concatenates the request path onto the upstream URL, so the
        // upstream URL itself stops at the host. The /codex or /v1 segment
        // comes from the request path (the client-side base_url suffix).
        assert_eq!(
            AuthMode::Chatgpt.upstream_url(),
            "https://chatgpt.com/backend-api"
        );
        assert_eq!(AuthMode::Apikey.upstream_url(), "https://api.openai.com");
    }

    #[test]
    fn connect_writes_provider_and_flips_pointer() {
        let mut doc = DocumentMut::new();
        // Simulate an existing user config with their own model_provider.
        doc["model_provider"] = value("openai");

        // Inline a stripped-down version of connect()'s mutation so we
        // can assert without needing keychain/account state.
        let model_providers = doc
            .entry("model_providers")
            .or_insert_with(|| Item::Table(new_table()))
            .as_table_mut()
            .unwrap();
        let mut provider = Table::new();
        provider.insert("name", value(PROVIDER_DISPLAY_NAME));
        provider.insert(
            "base_url",
            value(compute_base_url("http://127.0.0.1:9977", AuthMode::Chatgpt).as_str()),
        );
        provider.insert("requires_openai_auth", value(true));
        let mut headers = Table::new();
        headers.set_implicit(false);
        headers.insert(UPSTREAM_URL_HEADER, value(AuthMode::Chatgpt.upstream_url()));
        provider.insert("http_headers", Item::Table(headers));
        model_providers.insert(PROVIDER_ID, Item::Table(provider));
        doc["model_provider"] = value(PROVIDER_ID);

        let rendered = doc.to_string();
        assert!(rendered.contains("model_provider = \"gate\""));
        assert!(rendered.contains("[model_providers.gate]"));
        assert!(rendered.contains("requires_openai_auth = true"));
        assert!(rendered.contains("[model_providers.gate.http_headers]"));
        assert!(rendered.contains("X-Gate-Upstream-Url"));
        // No Gate credential is ever written to config.toml - the relay
        // injects it live.
        assert!(!rendered.contains("X-Gate-Api-Key"));
        // ChatGPT mode: base_url points at the relay and ends in /codex (so the
        // client sends /codex/responses); upstream is the bare backend-api host
        // (Gate concatenates the path onto it).
        assert!(rendered.contains("base_url = \"http://127.0.0.1:9977/codex\""));
        assert!(rendered.contains("\"https://chatgpt.com/backend-api\""));
        // And specifically NOT the double-codex shape that produced 404s.
        assert!(!rendered.contains("https://chatgpt.com/backend-api/codex"));
    }

    #[test]
    fn read_auth_mode_defaults_to_chatgpt_for_unknown_modes() {
        // We don't test against the real auth.json, just the parsing logic.
        let parsed: serde_json::Value = serde_json::from_str(
            r#"{"auth_mode": "something-weird", "tokens": {"access_token": "t"}}"#,
        )
        .unwrap();
        let mode = match parsed
            .get("auth_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("")
        {
            "apikey" => AuthMode::Apikey,
            _ => AuthMode::Chatgpt,
        };
        assert_eq!(mode, AuthMode::Chatgpt);
    }
}
