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
//! ([`crate::proxy::relay`]), not the gateway, and is the only thing written:
//! `http://127.0.0.1:<port>/<slug><suffix>`, where `<slug>` names the catalog
//! domain the relay routes on and `<suffix>` is the path Codex appends
//! `/responses` to (`/codex` in ChatGPT mode, `/v1` in API-key mode), since the
//! relay forwards the request path verbatim onto the gateway. The relay injects
//! the live Gate credential *and* the upstream hint per request, so **neither a
//! credential nor an `http_headers` table is written to config.toml**.
//!
//! Like Claude Code, Codex brings its own upstream credentials. We set
//! `requires_openai_auth = true` on the provider so Codex attaches its
//! own `codex login` session - the ChatGPT OAuth token or the API key in
//! `~/.codex/auth.json` - as the upstream bearer. Per the Codex docs this
//! is the only provider shape that carries a ChatGPT-subscription login
//! through a custom `base_url` (a bare `[auth] command` helper works for
//! API keys but leaves ChatGPT-mode Codex falling back to its built-in
//! provider and hitting chatgpt.com directly). Gate passes the bearer
//! through and forwards to OpenAI per the upstream hint the relay injects.
//! Therefore [`requires_upstream_credential`] is `false`.
//!
//! Codex reads `config.toml` at startup, so the user must restart any
//! running `codex` sessions after connecting/disconnecting.
//!
//! `disconnect` is the one place we stop short of zero residue: it leaves a
//! `[model_providers.gate]` passthrough stub pointed at OpenAI (see
//! [`passthrough_stub`]). Codex writes the provider *name* into every thread's
//! session metadata, so deleting the block outright makes every thread started
//! while routed unresumable ("Model provider `gate` not found"). The stub
//! carries no credential, no gateway URL and no upstream hint, so it leaks
//! nothing and routes nothing through Gate.
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

/// Codex `base_url` - the relay loopback origin, the catalog slug the relay
/// routes on, and the auth-mode path suffix Codex appends `/responses` to.
///
/// The suffix has to match the upstream's path layout
/// (`chatgpt.com/backend-api/codex` vs. `api.openai.com/v1`) because the relay
/// and Gate forward the path verbatim. Both halves come from
/// [`crate::proxy::resolve_endpoint`] rather than being spliced by hand: the
/// mode's canonical endpoint is its upstream plus its suffix, and the catalog
/// says where to cut it.
///
/// Errors when the mode's endpoint is off-catalog, which would mean the relay
/// could not forward it - better to refuse than to write a config that 403s.
fn relay_base_url_for(relay_base: &str, mode: AuthMode) -> Result<String> {
    let endpoint = format!("{}{}", mode.upstream_url(), mode.gateway_path_suffix());
    let resolved = crate::proxy::resolve_endpoint(&endpoint)
        .with_context(|| format!("Gate has no upstream domain for {endpoint:?}"))?;
    Ok(resolved.relay_base_url(relay_base))
}

/// The path suffix on Codex's side of the relay, for the passthrough stub that
/// disconnect leaves pointed straight at OpenAI.
fn direct_base_url(mode: AuthMode) -> String {
    format!("{}{}", mode.upstream_url(), mode.gateway_path_suffix())
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

/// `name` on the passthrough stub [`disconnect`] leaves behind, so a user
/// reading their config sees the block is no longer routed through Gate.
const PASSTHROUGH_DISPLAY_NAME: &str = "OpenAI (direct)";

/// Key inside `[_gate_connect]` marking the `gate` provider block as the
/// post-disconnect passthrough stub rather than a routed one. Without it
/// [`status`] would read the stub as leftover Gate residue and report drift.
const PASSTHROUGH_MARKER: &str = "passthrough_stub";

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
        // The passthrough stub `disconnect` leaves behind (see
        // [`passthrough_stub`]) is a `gate` block that routes nowhere near
        // Gate, so it is not residue: the machine is disconnected. Only trust
        // the marker while the pointer is off `gate` - a config still pointing
        // at us falls through to the drift checks below.
        if is_passthrough_stub(&doc) && model_provider != PROVIDER_ID {
            return Ok(Status::Detected);
        }
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
        let expected_base = relay_base_url_for(&relay_base, mode)?;
        let base_url = provider_block
            .get("base_url")
            .and_then(|i| i.as_str())
            .unwrap_or("");
        if base_url != expected_base {
            return Ok(Status::Drifted(format!(
                "[model_providers.{PROVIDER_ID}] base_url is {base_url:?}, expected {expected_base:?}"
            )));
        }

        // Nothing else to check: `base_url` above carries the relay origin, the
        // catalog slug the relay routes on, and the auth-mode path, and no header
        // or credential is written alongside it. An `http_headers` table left by
        // an older build is not drift - the next connect rewrites the block
        // wholesale, and disconnect drops it either way.

        // Identity matched; now liveness, the way Hermes does it. The
        // persisted relay port survives restarts precisely so configs stay
        // valid, which means the identity check alone reads Connected while
        // Codex dials a dead loopback port (engine crash-reverted, or routing
        // never restored). Drift rather than Connected also keeps the
        // master-off sweep repairing it.
        if !crate::proxy::relay_listening() {
            return Ok(Status::Drifted(format!(
                "the Gate proxy is not running, so Codex cannot reach its provider \
                 ({expected_base:?} is a dead address) - turn the proxy on, or disconnect Codex \
                 to restore it"
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

        let base_url = relay_base_url_for(relay_base, mode)?;

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

        // No `http_headers` at all: the relay reads the upstream off the slug
        // segment in `base_url` and injects the hint itself, and the Gate
        // credential was never written here. An older build's table is dropped
        // with the rest of the block, since we rewrite it wholesale.

        model_providers.insert(PROVIDER_ID, Item::Table(provider));

        doc["model_provider"] = value(PROVIDER_ID);

        let marker_entry = doc
            .entry("_gate_connect")
            .or_insert_with(|| Item::Table(new_table()));
        upgrade_inline_to_table(marker_entry);
        let marker = marker_entry
            .as_table_mut()
            .context("`_gate_connect` must be a TOML table")?;
        // The block we just wrote is the managed one again; a passthrough
        // marker left by an earlier disconnect would make `status` report this
        // connected config as merely Detected.
        marker.remove(PASSTHROUGH_MARKER);
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

        // Replace our model_providers.gate block with a passthrough stub
        // rather than deleting it. Codex records the provider *name* in every
        // thread's session metadata (`"model_provider":"gate"`), so a thread
        // started while routed re-resolves `gate` on resume and dies with
        // "Model provider `gate` not found" once the name is gone. The stub
        // keeps those threads resumable and sends them straight to OpenAI.
        // Read the shape before mutating so `as_table_like` also sees
        // inline-table forms (`model_providers = { ... }`), which the insert
        // below then upgrades - `as_table_mut` alone would skip them and leave
        // our routed entry behind.
        let had_gate_block = doc
            .get("model_providers")
            .and_then(|i| i.as_table_like())
            .map(|t| t.contains_key(PROVIDER_ID))
            .unwrap_or(false);
        if had_gate_block {
            let entry = doc
                .get_mut("model_providers")
                .context("`model_providers` vanished mid-disconnect")?;
            upgrade_inline_to_table(entry);
            let model_providers = entry
                .as_table_mut()
                .context("`model_providers` must be a TOML table")?;
            // auth.json may be gone by now (the user logged out of Codex);
            // fall back to ChatGPT mode for the same reason `status` does.
            let mode = read_auth_mode().unwrap_or(AuthMode::Chatgpt);
            model_providers.insert(PROVIDER_ID, Item::Table(passthrough_stub(mode)));
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
        if had_gate_block {
            // Keep exactly one marker key so `status` can tell the stub from a
            // routed block; the undo-log keys are spent and must not survive.
            let mut marker = new_table();
            marker.insert(PASSTHROUGH_MARKER, value(true));
            doc.insert("_gate_connect", Item::Table(marker));
        } else {
            doc.remove("_gate_connect");
        }

        // Write the restored config before removing the helper script: a
        // failed write must not leave config.toml pointing `[auth] command`
        // at a script that no longer exists.
        if doc.as_table().is_empty() {
            // Nothing of the user's - and no stub to preserve - is left;
            // remove the file rather than leave an empty one behind.
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

/// The `[model_providers.gate]` block [`disconnect`] leaves behind: the same
/// provider name Codex baked into every thread it started while routed, but
/// pointed straight at OpenAI. Resuming such a thread then goes direct instead
/// of failing on a missing provider. Deliberately carries no `http_headers`,
/// no gateway URL, and no credential - it mirrors what Codex's own built-in
/// `openai` provider would have done, `requires_openai_auth` included so the
/// `codex login` session still supplies the bearer.
fn passthrough_stub(mode: AuthMode) -> Table {
    let mut t = Table::new();
    t.decor_mut().set_prefix(
        "\n# Left by Constellation Gate Connect when Codex was disconnected.\n\
         # Codex stores the provider name in each thread, so threads started\n\
         # while routing was on still need `gate` to resolve; this sends them\n\
         # straight to OpenAI. Safe to delete once those threads are done.\n",
    );
    t.insert("name", value(PASSTHROUGH_DISPLAY_NAME));
    t.insert("base_url", value(direct_base_url(mode).as_str()));
    t.insert("wire_api", value("responses"));
    t.insert("requires_openai_auth", value(true));
    t
}

/// Is the `gate` provider block in `doc` the post-disconnect passthrough stub?
fn is_passthrough_stub(doc: &DocumentMut) -> bool {
    doc.get("_gate_connect")
        .and_then(|i| i.as_table_like())
        .and_then(|t| t.get(PASSTHROUGH_MARKER))
        .and_then(|i| i.as_bool())
        .unwrap_or(false)
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
    fn chatgpt_mode_base_url_carries_the_chatgpt_slug_and_codex_path() {
        // The relay strips `/chatgpt` and forwards `/codex/responses`, which the
        // gateway concatenates onto `https://chatgpt.com/backend-api`.
        assert_eq!(
            relay_base_url_for("http://127.0.0.1:9977", AuthMode::Chatgpt).unwrap(),
            "http://127.0.0.1:9977/chatgpt/codex"
        );
        assert_eq!(
            relay_base_url_for("http://127.0.0.1:9977/", AuthMode::Chatgpt).unwrap(),
            "http://127.0.0.1:9977/chatgpt/codex"
        );
    }

    #[test]
    fn apikey_mode_base_url_carries_the_openai_slug_and_v1_path() {
        assert_eq!(
            relay_base_url_for("http://127.0.0.1:9977", AuthMode::Apikey).unwrap(),
            "http://127.0.0.1:9977/openai/v1"
        );
    }

    #[test]
    fn direct_base_url_is_the_real_upstream_for_the_passthrough_stub() {
        assert_eq!(
            direct_base_url(AuthMode::Chatgpt),
            "https://chatgpt.com/backend-api/codex"
        );
        assert_eq!(
            direct_base_url(AuthMode::Apikey),
            "https://api.openai.com/v1"
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
            value(
                relay_base_url_for("http://127.0.0.1:9977", AuthMode::Chatgpt)
                    .unwrap()
                    .as_str(),
            ),
        );
        provider.insert("requires_openai_auth", value(true));
        model_providers.insert(PROVIDER_ID, Item::Table(provider));
        doc["model_provider"] = value(PROVIDER_ID);

        let rendered = doc.to_string();
        assert!(rendered.contains("model_provider = \"gate\""));
        assert!(rendered.contains("[model_providers.gate]"));
        assert!(rendered.contains("requires_openai_auth = true"));
        // ChatGPT mode: base_url points at the relay, names the `chatgpt` catalog
        // slug the relay routes on, and ends in /codex so Codex sends
        // /chatgpt/codex/responses. The relay strips the slug and forwards
        // /codex/responses.
        assert!(rendered.contains("base_url = \"http://127.0.0.1:9977/chatgpt/codex\""));
        // Nothing else is written: no header table, no upstream hint, and above
        // all no credential - the relay injects all of it live.
        assert!(!rendered.contains("http_headers"));
        assert!(!rendered.contains("X-Gate-Upstream-Url"));
        assert!(!rendered.contains("X-Gate-Api-Key"));
        // And specifically NOT the double-codex shape that produced 404s.
        assert!(!rendered.contains("https://chatgpt.com/backend-api/codex"));
    }

    #[test]
    fn passthrough_stub_points_at_the_upstream_with_no_gate_traces() {
        // ChatGPT mode: the /codex segment Codex appends /responses to has to
        // be in the base_url, since nothing rewrites the path now.
        let mut doc = DocumentMut::new();
        let model_providers = doc
            .entry("model_providers")
            .or_insert_with(|| Item::Table(new_table()))
            .as_table_mut()
            .unwrap();
        model_providers.insert(
            PROVIDER_ID,
            Item::Table(passthrough_stub(AuthMode::Chatgpt)),
        );
        let rendered = doc.to_string();
        assert!(rendered.contains("[model_providers.gate]"));
        assert!(rendered.contains(r#"base_url = "https://chatgpt.com/backend-api/codex""#));
        assert!(rendered.contains("requires_openai_auth = true"));
        // Nothing Gate-flavoured survives: no relay/gateway URL, no upstream
        // hint header, no credential.
        assert!(!rendered.contains("X-Gate-Upstream-Url"));
        assert!(!rendered.contains("http_headers"));
        assert!(!rendered.contains("X-Gate-Api-Key"));
        assert!(!rendered.contains("127.0.0.1"));

        // API-key mode lands on the standard /v1 path instead.
        let stub = passthrough_stub(AuthMode::Apikey);
        assert_eq!(
            stub.get("base_url").and_then(|i| i.as_str()),
            Some("https://api.openai.com/v1")
        );
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
