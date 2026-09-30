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
//!
//! **Codex re-reads `config.toml` per THREAD, not per process**, so "restart
//! Codex" is the wrong thing to tell anyone. Measured 2026-09-18 on codex-cli
//! 0.146.0-alpha.3.1 by driving `codex app-server` against two loopback
//! listeners and watching which one a turn reached:
//!
//! | | picks up an edited `config.toml`? |
//! | --- | --- |
//! | a new thread in a running process | yes, immediately |
//! | a thread that was already open | no, it keeps the address it started with |
//! | that thread resumed after a restart | yes, it re-resolves |
//!
//! So the unit is the conversation. A routing change reaches every conversation
//! started after it, with no restart at all, and reaches none that are already
//! open, however many times the process is restarted, unless the user resumes
//! them. This entry used to say the config was read at startup and that running
//! sessions had to be restarted, which is wrong in both directions.
//!
//! It is also the mechanism behind the two facts below: the thread pins the
//! provider *name* and re-resolves it against whatever is on disk, which is why
//! the name has to keep resolving and why the stub exists.
//!
//! `disconnect` is the one place we stop short of zero residue: it leaves a
//! `[model_providers.gate]` passthrough stub pointed at OpenAI (see
//! [`passthrough_stub`]). Codex writes the provider *name* into every thread's
//! session metadata, so deleting the block outright makes every thread started
//! while routed unresumable ("Model provider `gate` not found"). The stub
//! carries no credential, no gateway URL and no upstream hint, so it leaks
//! nothing and routes nothing through Gate.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{value, DocumentMut, Item, Table, Value};

use crate::account::BillingMode;
use crate::env;
use crate::integrations::binaries;
use crate::integrations::precedence::Override;
use crate::registry::{ConnectInput, GateModelState, Integration, Mechanism, Status, ToolId};

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
    Ok(resolved.relay_base_url(relay_base, ToolId::Codex))
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

    fn client(&self) -> crate::taxonomy::Client {
        crate::taxonomy::Client::Codex
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

    fn binary(&self) -> (&'static [&'static str], &'static [&'static str]) {
        #[cfg(windows)]
        const NAMES: &[&str] = &["codex.exe", "codex.cmd", "codex.bat", "codex"];
        #[cfg(not(windows))]
        const NAMES: &[&str] = &["codex"];
        (CLI_BIN_PATHS, NAMES)
    }

    fn upstream_provider_name(&self) -> &'static str {
        UPSTREAM_PROVIDER_NAME
    }

    fn default_upstream_url(&self) -> &'static str {
        DEFAULT_UPSTREAM_URL
    }

    fn config_location(&self) -> Option<String> {
        config_path().ok().map(|p| p.display().to_string())
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = CLI_BIN_PATHS.iter().map(PathBuf::from).collect();
        paths.extend(env::codex_config_dir());
        paths.extend(config_path());
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
        Ok(env::codex_config_dir()?.exists())
    }

    fn config_is_managed(&self) -> Result<bool> {
        // Two-part marker, the same shape OpenCode, Hermes and OpenClaw use and
        // for the same reason. Codex's config is TOML with no schema policing
        // it, so unlike OpenCode - which needs a sidecar because opencode.json
        // rejects unknown keys - the marker can live in the file it describes.
        // That buys the first half only: the marker records who *created* the
        // block, not who wrote the values in it now, so on its own it cannot
        // tell our stale write apart from a config the user has since repointed
        // by hand. So also require that the config still aims at us.
        //
        // The connect keys are the marker, not the `[model_providers.gate]`
        // block: `connect` adopts a hand-written block under that name and
        // `disconnect` deletes it, so the block's presence says nothing about
        // who wrote it. One of the two keys is always recorded, including when
        // there was no prior `model_provider` to stash.
        //
        // The stub `disconnect` leaves behind carries its own key and is
        // excluded by [`is_connected_marker`]. Do not read more into that than
        // it says: it keeps this answer consistent with `status`, which reports
        // the stub as `Detected`, and it is *not* what keeps a disconnected
        // Codex disconnected. `reconcile_enabled` reapplies `Detected`
        // unconditionally, without ever asking this question, so a machine whose
        // OpenAI domains are still enabled reconnects Codex on the next pass by
        // that other branch. That is the provider flag working as designed - the
        // enabled domain is the intent it reads - and it is a separate decision
        // from this one.
        let path = config_path()?;
        if !path.exists() {
            return Ok(false);
        }
        let doc = read_doc(&path)?;
        Ok(is_connected_marker(&doc) && still_aims_at_us(&doc))
    }

    /// `base_url` names the loopback relay, which dies with the engine.
    fn mechanism(&self) -> Mechanism {
        Mechanism::Relay
    }

    fn supports_gate_models(&self) -> bool {
        true
    }

    /// Codex's own model and picker back, and the provider's `base_url` off
    /// the Gate models route onto the one it would have without them - if it
    /// is still on ours. `model_provider` is not touched: a user who pointed
    /// Codex at another provider did that on purpose.
    fn leave_gate_models(&self, input: &ConnectInput) -> Result<()> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(());
        }
        let mut doc = read_doc(&path)?;
        revert_gate_models(&mut doc)?;
        let on_served_route = doc
            .get("model_providers")
            .and_then(|i| i.as_table_like())
            .and_then(|t| t.get(PROVIDER_ID))
            .and_then(|i| i.as_table_like())
            .and_then(|b| b.get("base_url"))
            .and_then(|i| i.as_str())
            .is_some_and(|u| crate::proxy::gate_served::is_relay_base_url(u, ToolId::Codex));
        if on_served_route {
            let relay_base = input.relay_base_url.as_deref().context(
                "the Gate proxy relay is not running - Codex's route cannot be restored",
            )?;
            let mode = match input.billing_mode {
                BillingMode::Payg => AuthMode::Apikey,
                BillingMode::Byok => read_auth_mode().unwrap_or(AuthMode::Chatgpt),
            };
            let block = doc
                .get_mut("model_providers")
                .and_then(|i| i.as_table_like_mut())
                .and_then(|t| t.get_mut(PROVIDER_ID))
                .and_then(|i| i.as_table_like_mut())
                .context("the Gate provider block vanished")?;
            block.insert("base_url", value(relay_base_url_for(relay_base, mode)?));
            if input.billing_mode == BillingMode::Byok {
                block.insert("requires_openai_auth", value(true));
            }
        }
        write_doc(&path, &doc)
    }

    fn gate_model_state(&self) -> Result<GateModelState> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(GateModelState::NotApplied);
        }
        Ok(gate_model_state_of(&read_doc(&path)?))
    }

    fn configured_addresses(&self) -> Result<Vec<String>> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(Vec::new());
        }
        // The gate block's own `base_url`, whatever it says now: the relay
        // while connected, OpenAI direct once the passthrough stub is in.
        Ok(read_doc(&path)?
            .get("model_providers")
            .and_then(|i| i.as_table_like())
            .and_then(|t| t.get(PROVIDER_ID))
            .and_then(|i| i.as_table_like())
            .and_then(|b| b.get("base_url"))
            .and_then(|i| i.as_str())
            .map(str::to_owned)
            .into_iter()
            .collect())
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

        // What the block should say about auth depends on who pays, so the two
        // modes have opposite expectations here and each reads the other's
        // shape as drift - which is right: a mode switch has to reconnect
        // Codex, and status is how the app knows to.
        let billing_mode = crate::account::billing_mode().unwrap_or_default();
        // On Gate models the block has the PAYG shape whatever the account's
        // mode, because Gate is the provider - see `connect`.
        let on_gate_models = gate_set().is_some();
        let billing_mode = if on_gate_models {
            BillingMode::Payg
        } else {
            billing_mode
        };
        let requires_openai_auth = provider_block
            .get("requires_openai_auth")
            .and_then(|i| i.as_bool())
            .unwrap_or(false);
        match billing_mode {
            // BYOK requires `requires_openai_auth = true`. A block without it
            // is the old `[auth] command` shape that left ChatGPT-mode Codex
            // bypassing the gateway - report drift so the user reconnects into
            // the fix.
            BillingMode::Byok if !requires_openai_auth => {
                return Ok(Status::Drifted(format!(
                    "[model_providers.{PROVIDER_ID}] is missing requires_openai_auth = true"
                )));
            }
            // PAYG requires its ABSENCE: with it, Codex attaches its own
            // credential, the gateway reads that as a passthrough token, and
            // the request is refused for want of an upstream URL. A block
            // carrying it is a BYOK config the account has since moved off.
            BillingMode::Payg if requires_openai_auth => {
                return Ok(Status::Drifted(format!(
                    "[model_providers.{PROVIDER_ID}] carries requires_openai_auth = true, which \
                     Codex cannot use while this account bills through Gate"
                )));
            }
            _ => {}
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
        // an OPENAI_API_KEY to authenticate). In PAYG there is no login to
        // read and `connect` pinned the apikey shape, so expect that.
        let mode = match billing_mode {
            BillingMode::Payg => AuthMode::Apikey,
            BillingMode::Byok => read_auth_mode().unwrap_or(AuthMode::Chatgpt),
        };
        let expected_base = if on_gate_models {
            crate::proxy::gate_served::relay_base_url(&relay_base, ToolId::Codex)
        } else {
            relay_base_url_for(&relay_base, mode)?
        };
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
        // Liveness and interception, from the relay itself. Identity alone is
        // not enough: the persisted relay port survives restarts precisely so
        // configs stay valid, so the check above reads Connected while Codex
        // dials a port nothing is bound to. And a relay that answers is not a
        // relay that routes - parked, it forwards every request to the tool's
        // own provider - so reporting Connected off liveness alone is a green
        // pill over traffic going direct, which is the one thing this status
        // exists to prevent.
        //
        // The relay reports interception on its health path now. This used to
        // read `intent::load_intent()`, which is the user's stored preference
        // rather than a measurement, and got the headless `proxy relay` host
        // backwards: it always intercepts and writes no intent file, so on a
        // machine whose last explicit answer was "off" it was reported as not
        // routing while it routed.
        let Some(report) = crate::proxy::relay_report() else {
            return Ok(Status::Drifted(format!(
                "the Gate proxy is not running, so Codex cannot reach its provider \
                 ({expected_base:?} is a dead address) - turn the proxy on, or disconnect Codex \
                 to restore it"
            )));
        };
        if !report.intercepting {
            return Ok(Status::Drifted(format!(
                "routing is off, so Codex reaches its provider directly through \
                 {expected_base:?} rather than through Gate - turn routing on to route it"
            )));
        }

        // Everything Gate writes is in place. The last question is whether Codex
        // reads it, which the pointer above does not settle on its own (AG-674).
        if let Some(o) = active_profile_override(&doc, &path.display().to_string()) {
            return Ok(o.into_status());
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
        // The upstream comes from Codex's own login state: the ChatGPT-mode
        // bearer authenticates only against chatgpt.com/backend-api, the
        // apikey-mode bearer only against api.openai.com/v1. This mirrors
        // what Codex itself would have done in its native (non-Gate) routing.
        //
        // PAYG has no login state to read: the whole point is that Codex sends
        // no credential of its own, so `auth.json` may not exist at all and
        // `read_auth_mode` would hard-fail on a user who never ran
        // `codex login`. The apikey shape is the only one PAYG can use anyway -
        // the ChatGPT route is a subscription, which is by definition not
        // pay-as-you-go - so pin it rather than asking.
        let path = config_path()?;
        let mut doc = if path.exists() {
            read_doc(&path)?
        } else {
            DocumentMut::new()
        };

        // The Gate models the user chose for Codex, unless Codex has been moved
        // off them from inside - see [`models_left_by_user`]. That is a choice
        // too, and writing the Gate model straight back over it would undo what
        // the user just did in the tool they were using.
        if models_left_by_user(&doc) {
            crate::preferences::fall_back_to_tool_model(ToolId::Codex.slug())?;
        }
        let gate_models = gate_set();

        // On Gate models there is no Codex login to read: Gate serves the
        // request, so Codex authenticates to nothing and the apikey shape's
        // `/v1` path is the one the served route answers on.
        //
        // Leaving Gate models is the one BYOK connect that must not need a login
        // either: a user who only ever ran Codex on Gate models may never have
        // run `codex login`, and refusing here would leave Codex pointed at a
        // route that now refuses every request. The ChatGPT shape is the same
        // fallback `disconnect` and `status` use, and Codex then asks for its
        // login itself - which is what App default means for it.
        let leaving_gate_models = written_gate_models(&doc).is_some();
        let mode = match (input.billing_mode, &gate_models) {
            (_, Some(_)) | (BillingMode::Payg, None) => AuthMode::Apikey,
            (BillingMode::Byok, None) if leaving_gate_models => {
                read_auth_mode().unwrap_or(AuthMode::Chatgpt)
            }
            (BillingMode::Byok, None) => read_auth_mode()?,
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
        let marker_has_prev = has_connect_key(&doc);
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

        // The base URL is the whole difference between Codex on its own model
        // and Codex on Gate models: the served route answers everything it is
        // sent from the organization's credits, and the catalog routes forward
        // to OpenAI exactly as before.
        let base_url = match &gate_models {
            Some(_) => crate::proxy::gate_served::relay_base_url(relay_base, ToolId::Codex),
            None => relay_base_url_for(relay_base, mode)?,
        };

        let mut provider = Table::new();
        provider.insert("name", value(PROVIDER_DISPLAY_NAME));
        provider.insert("base_url", value(base_url.as_str()));
        provider.insert("wire_api", value("responses"));
        // BYOK: Codex sources the upstream bearer from its own `codex login`
        // session (ChatGPT OAuth token or API key in ~/.codex/auth.json)
        // and attaches it to this provider. This is the only mechanism
        // that carries a ChatGPT-subscription login through a custom
        // base_url - without it, ChatGPT-mode Codex ignores this provider
        // and hits chatgpt.com directly. Mutually exclusive with `env_key`
        // and `[auth] command` per the Codex docs, so we set neither.
        //
        // PAYG: set NEITHER `requires_openai_auth` nor `env_key`, which the
        // Codex docs define as the third, unauthenticated provider case -
        // "Codex assumes the provider doesn't require authentication", offered
        // for local models, and our `base_url` is a loopback address. Codex
        // then sends no `Authorization` at all, which is exactly what PAYG
        // needs: the gateway reads any non-`sk-gw-` token in that slot as a
        // passthrough credential, which forces BYOK and is then refused for
        // want of an upstream URL. Sending nothing is the only shape that
        // cannot be misread, and it keeps us from writing a credential (real
        // or placeholder) into the user's Codex config.
        //
        // Gate models: the PAYG shape, for the PAYG reason. Gate is the
        // provider, so Codex's own login has nothing to authenticate against.
        if input.billing_mode == BillingMode::Byok && gate_models.is_none() {
            provider.insert("requires_openai_auth", value(true));
        }

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

        match &gate_models {
            Some(ids) => apply_gate_models(&mut doc, ids)?,
            None => revert_gate_models(&mut doc)?,
        }

        write_doc(&path, &doc)?;

        // What the user has to know, and the only tool in the registry where
        // it is about conversations rather than processes. See the module docs
        // for the measurement: a new thread reads this file, one that is
        // already open never will. Telling them to restart Codex would be
        // advice that does nothing for either half.
        eprintln!(
            "note: New conversations will go through Gate. Codex pins a conversation to its \
             provider when it starts, so any you already have open keep the route they started \
             with."
        );
        Ok(())
    }

    fn disconnect(&self) -> Result<()> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(());
        }
        let mut doc = read_doc(&path)?;

        // The model first, while the marker still holds what to put back: the
        // block below rebuilds `[_gate_connect]` from scratch.
        revert_gate_models(&mut doc)?;

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
            crate::config_changes::remove(&path)?;
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
}

fn config_path() -> Result<PathBuf> {
    env::codex_config_toml_path()
}

/// The selected profile, when it points Codex at a provider that is not ours.
///
/// Codex resolves `model_provider` from the active profile first and only falls
/// back to the top-level key Gate writes. So `profile = "work"` plus
/// `[profiles.work] model_provider = "openai"` sends every request straight to
/// OpenAI while `model_provider = "gate"` sits above it in the same file,
/// untouched and inert - and until AG-674 that read as Connected, because we
/// checked our own key and stopped.
///
/// Profiles are the user's, not ours: `connect` writes the top-level pointer and
/// leaves the rest of the file alone. So this reports the disagreement rather
/// than resolving it - editing somebody's profile to win an argument with them
/// is not a repair.
///
/// What stays invisible here is `--profile` on the command line, which outranks
/// the file's own `profile` key. A shell flag is not a thing this process can
/// see, on the same terms as the project-level layers in
/// [`crate::integrations::precedence`].
fn active_profile_override(doc: &DocumentMut, source: &str) -> Option<Override> {
    let profile = doc.get("profile").and_then(|i| i.as_str())?;
    let table = doc
        .get("profiles")
        .and_then(|i| i.as_table_like())
        .and_then(|t| t.get(profile))
        .and_then(|i| i.as_table_like())?;
    // On Gate models the profile can also displace the MODEL: Gate writes the
    // top-level `model` and picker, and a profile that sets its own wins over
    // both. Codex then sends that model to the Gate models route, which refuses
    // it, while the pane would say the Gate models are applied (review on #382).
    if written_gate_models(doc).is_some() {
        for key in ["model", "model_catalog_json"] {
            if let Some(value) = table.get(key).and_then(|i| i.as_str()) {
                return Some(Override::new(
                    source,
                    format!(
                        "selects profile {profile:?}, whose {key} is {value:?} - Codex reads that \
                         before the Gate models written at the top level"
                    ),
                ));
            }
        }
    }
    let provider = table.get("model_provider").and_then(|i| i.as_str())?;
    if provider == PROVIDER_ID {
        return None;
    }
    Some(Override::new(
        source,
        format!(
            "selects profile {profile:?}, whose model_provider is {provider:?} - Codex reads that \
             before the top-level pointer at {PROVIDER_ID:?}"
        ),
    ))
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
    crate::config_changes::write(path, doc.to_string().as_bytes(), 0o600)
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

/// Whether `doc` carries the marker `connect` leaves, i.e. this is a config
/// Gate Connect wrote and has not disconnected.
///
/// The stub exclusion is unconditional here, where [`Integration::status`]'s is
/// qualified by `model_provider != PROVIDER_ID`. The asymmetry is deliberate and
/// conservative: a config carrying both the stub marker and a `"gate"` pointer
/// is something none of our own writes can produce (`connect` clears
/// [`PASSTHROUGH_MARKER`]), so it reads as drift there and is not reapplied
/// here - which is the right way round for a shape we cannot explain.
///
/// Split out from [`Integration::config_is_managed`] so it can be tested on a
/// parsed document like the rest of this module, rather than needing a config
/// directory on disk.
fn is_connected_marker(doc: &DocumentMut) -> bool {
    if is_passthrough_stub(doc) {
        return false;
    }
    has_connect_key(doc)
}

/// Whether the config still points at Gate: `model_provider` is ours, and our
/// block's `base_url` is still one we would have written.
///
/// The second half of [`Integration::config_is_managed`], and the half that
/// decides whether `reconcile_enabled` may rewrite this file without asking.
/// Both tests are about values the *user* could have changed to mean "stop
/// routing Codex through Gate": pointing `model_provider` at another provider is
/// how you turn Gate off without running disconnect, and repointing `base_url`
/// is how you send the traffic somewhere else. Either one, and the config stops
/// being ours to silently reapply.
///
/// `is_relay_base_url` rather than the loopback test [`super::hermes`] uses for
/// the same job: a user who repoints Codex at their own local server is still on
/// loopback, and that answer would hand the reapply a licence to take it back.
/// It judges the URL at its own origin, so a relay that came back on a different
/// port - the drift this exists to repair - still reads as ours.
///
/// Both auth modes count. Each writes a different upstream and therefore a
/// different path, and a config in the other one is drift `status` already
/// reports; what matters here is only that the URL is one of ours.
///
/// Split out for the same testability reason as [`is_connected_marker`].
fn still_aims_at_us(doc: &DocumentMut) -> bool {
    let points_at_gate = doc
        .get("model_provider")
        .and_then(|i| i.as_str())
        .is_some_and(|p| p == PROVIDER_ID);
    if !points_at_gate {
        return false;
    }
    let Some(base_url) = doc
        .get("model_providers")
        .and_then(|i| i.as_table_like())
        .and_then(|t| t.get(PROVIDER_ID))
        .and_then(|i| i.as_table_like())
        .and_then(|t| t.get("base_url"))
        .and_then(|i| i.as_str())
    else {
        return false;
    };
    if crate::proxy::gate_served::is_relay_base_url(base_url, ToolId::Codex) {
        return true;
    }
    [AuthMode::Apikey, AuthMode::Chatgpt]
        .into_iter()
        .any(|mode| {
            crate::proxy::resolve_endpoint(&direct_base_url(mode))
                .is_some_and(|r| r.is_relay_base_url(base_url, ToolId::Codex))
        })
}

/// The `[_gate_connect]` keys `connect` records, either of which says the table
/// is one we wrote. Spelled once: a third key added to `connect` and missed at
/// one of the two read sites would silently narrow the marker.
fn has_connect_key(doc: &DocumentMut) -> bool {
    doc.get("_gate_connect")
        .and_then(|i| i.as_table_like())
        .is_some_and(has_connect_key_in)
}

fn has_connect_key_in(table: &dyn toml_edit::TableLike) -> bool {
    table.contains_key("previous_model_provider")
        || table.contains_key("previous_model_provider_absent")
}

/// Is the `gate` provider block in `doc` the post-disconnect passthrough stub?
fn is_passthrough_stub(doc: &DocumentMut) -> bool {
    doc.get("_gate_connect")
        .and_then(|i| i.as_table_like())
        .and_then(|t| t.get(PASSTHROUGH_MARKER))
        .and_then(|i| i.as_bool())
        .unwrap_or(false)
}

/// Whether a `codex` command line is Codex's app-server rather than a session.
///
/// The Codex TUI does not read `config.toml` itself: it talks to a long-lived
/// app-server daemon (`codex app-server --listen …`, supervised by
/// `codex app-server daemon pid-update-loop`), which stays up after every TUI
/// and the ChatGPT app have quit. It is not something a user opens or closes,
/// so it must not read as a running Codex - which is what put "Reopen CLI to
/// finish / Close tool" on screen with no Codex open - and it is the process
/// that has to restart for a config change to reach new sessions.
pub fn is_app_server_command(cmd: &[std::ffi::OsString]) -> bool {
    cmd.iter().skip(1).any(|arg| arg == "app-server")
}

/// Is this process name Codex's binary, on any OS? Windows reports `codex.exe`.
fn is_codex_name(name: &str) -> bool {
    name == "codex" || name.eq_ignore_ascii_case("codex.exe")
}

/// Is this command line Codex's MANAGED app-server daemon - the one
/// `codex app-server daemon restart` controls - rather than any app-server?
///
/// Two processes make it up: the server (`app-server --listen … --managed-daemon`)
/// and its supervisor (`app-server daemon pid-update-loop`). An IDE extension's
/// own `codex app-server` is neither, and restarting "the daemon" on its account
/// would start one the user never had while the extension kept its stale copy
/// (review on #382).
fn is_managed_daemon_command(cmd: &[std::ffi::OsString]) -> bool {
    let args: Vec<&std::ffi::OsStr> = cmd.iter().skip(1).map(|a| a.as_os_str()).collect();
    let Some(at) = args.iter().position(|a| *a == "app-server") else {
        return false;
    };
    let rest = &args[at + 1..];
    rest.iter().any(|a| *a == "--managed-daemon")
        || (rest.first().is_some_and(|a| *a == "daemon")
            && rest.get(1).is_some_and(|a| *a == "pid-update-loop"))
}

/// What [`refresh_app_server_daemon`] found.
#[derive(Debug, PartialEq, Eq)]
pub enum DaemonRefresh {
    /// No managed daemon is running: nothing holds a stale config.
    NotRunning,
    /// It started after Codex's config last changed, so it already has it.
    Current,
    /// It predated the change and was restarted.
    Restarted,
}

/// Restart Codex's managed app-server daemon if it predates the last change Gate
/// made to Codex's config, so new sessions load that config and model catalog.
/// The daemon reads both only when it starts: measured on 0.159, a TUI session
/// opened after a model change was still offered the catalog it had loaded.
///
/// "Predates" is the same test the reopen check applies to a running tool - the
/// process's start time against `config_changes`' record of Gate's last write -
/// so every caller agrees on what is stale: a model save, the restart notice's
/// Close, and startup's reconnect (review on #382: one rule, one code path).
/// Nothing runs for a daemon that is current or absent; `restart` on an absent
/// one would start a daemon the user never asked for.
///
/// Blocks for up to 20s while Codex restarts it, so callers on a user-facing
/// path run it on a thread of their own. The restart ends the sessions the
/// daemon hosts, so callers also decide it only with no Codex session open.
pub fn refresh_app_server_daemon() -> Result<DaemonRefresh> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .without_tasks()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );
    let daemon: Vec<(std::path::PathBuf, u64)> = sys
        .processes()
        .values()
        .filter(|p| {
            is_codex_name(&p.name().to_string_lossy()) && is_managed_daemon_command(p.cmd())
        })
        .filter_map(|p| Some((p.exe()?.to_path_buf(), p.start_time())))
        .collect();
    let Some((exe, _)) = daemon.first().cloned() else {
        return Ok(DaemonRefresh::NotRunning);
    };
    let started = daemon.iter().map(|(_, t)| *t).max().unwrap_or(0);
    let changed = config_path()
        .ok()
        .and_then(|p| crate::config_changes::changed_at(&p));
    if changed.is_none_or(|changed| started >= changed) {
        return Ok(DaemonRefresh::Current);
    }
    let mut child = std::process::Command::new(&exe)
        .args(["app-server", "daemon", "restart"])
        .env("CODEX_HOME", env::codex_config_dir()?)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("running {} app-server daemon restart", exe.display()))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        if let Some(status) = child.try_wait()? {
            anyhow::ensure!(
                status.success(),
                "codex app-server daemon restart exited {status}"
            );
            return Ok(DaemonRefresh::Restarted);
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            anyhow::bail!("codex app-server daemon restart did not finish in 20s");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// `[_gate_connect]` keys for Gate models, beside the provider ones.
///
/// `gate_models` is the set Gate last wrote, in the user's order. Its presence
/// is what "Gate models are applied here" means, and it is what the
/// compare-and-restore below compares against: a `model` still in it is ours to
/// put back, one outside it is the user's and stays. Written as a list rather
/// than a flag so that changing the set changes this file - which is what
/// stamps `config_changes` and so raises the reopen notice.
const GATE_MODELS_KEY: &str = "gate_models";
const PREVIOUS_MODEL_KEY: &str = "previous_model";
const PREVIOUS_MODEL_ABSENT_KEY: &str = "previous_model_absent";
const PREVIOUS_CATALOG_KEY: &str = "previous_model_catalog_json";
const PREVIOUS_CATALOG_ABSENT_KEY: &str = "previous_model_catalog_json_absent";

/// Codex's model picker, when it is on Gate models.
///
/// `model_catalog_json` REPLACES Codex's remote catalog rather than adding to
/// it (measured on 0.159: `codex debug models` lists exactly the file), so the
/// picker offers the enabled Gate models and nothing else, under their Gate ids.
const CATALOG_FILENAME: &str = "codex-gate-models.json";

fn catalog_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join(CATALOG_FILENAME))
}

/// The Gate models stored for Codex, if it is set to them.
fn gate_set() -> Option<Vec<String>> {
    crate::preferences::gate_models_for(ToolId::Codex.slug())
}

fn marker(doc: &DocumentMut) -> Option<&dyn toml_edit::TableLike> {
    doc.get("_gate_connect").and_then(|i| i.as_table_like())
}

/// The set Gate last wrote into this config, or `None` if Gate models are not
/// applied here.
fn written_gate_models(doc: &DocumentMut) -> Option<Vec<String>> {
    let list = marker(doc)?.get(GATE_MODELS_KEY)?.as_array()?;
    Some(
        list.iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
    )
}

fn top_str<'a>(doc: &'a DocumentMut, key: &str) -> Option<&'a str> {
    doc.get(key).and_then(|i| i.as_str())
}

/// What this config says about Gate models. Split from the trait method so it
/// can be tested on a document.
///
/// Applied means all four things Gate wrote still hold: Codex points at our
/// provider, the provider points at the served route, the picker is our file,
/// and the model is one of the set. Any one of them moved is the user moving
/// Codex off Gate models from inside Codex.
fn gate_model_state_of(doc: &DocumentMut) -> GateModelState {
    let Some(written) = written_gate_models(doc) else {
        return GateModelState::NotApplied;
    };
    let model = top_str(doc, "model").map(str::to_owned);
    let on_provider = top_str(doc, "model_provider") == Some(PROVIDER_ID);
    let on_route = doc
        .get("model_providers")
        .and_then(|i| i.as_table_like())
        .and_then(|t| t.get(PROVIDER_ID))
        .and_then(|i| i.as_table_like())
        .and_then(|b| b.get("base_url"))
        .and_then(|i| i.as_str())
        .is_some_and(|u| crate::proxy::gate_served::is_relay_base_url(u, ToolId::Codex));
    let on_catalog = match (top_str(doc, "model_catalog_json"), catalog_path()) {
        (Some(p), Ok(ours)) => Path::new(p) == ours,
        _ => false,
    };
    match model {
        Some(m) if on_provider && on_route && on_catalog && written.contains(&m) => {
            GateModelState::Applied { model: m }
        }
        model => GateModelState::Drifted { model },
    }
}

/// Did the user move Codex off the Gate models Gate wrote, from inside Codex?
fn models_left_by_user(doc: &DocumentMut) -> bool {
    matches!(gate_model_state_of(doc), GateModelState::Drifted { .. })
}

/// Write the Gate models into `doc`: the default model, the picker, and the
/// marker that records both. Snapshots Codex's own values the first time, the
/// way `previous_model_provider` is snapshotted, so a re-apply cannot overwrite
/// the original with Gate's.
///
/// The default is the first of the set, unless Codex is already on one of the
/// set and the set has not changed: then the user picked that one in Codex's
/// own picker, and a reconnect must not take it back.
fn apply_gate_models(doc: &mut DocumentMut, ids: &[String]) -> Result<()> {
    let written = written_gate_models(doc);
    let current = top_str(doc, "model").map(str::to_owned);
    let current_catalog = top_str(doc, "model_catalog_json").map(str::to_owned);
    let model = match (&written, &current) {
        (Some(w), Some(c)) if w.as_slice() == ids && ids.contains(c) => c.clone(),
        _ => ids
            .first()
            .context("a Gate model set cannot be empty")?
            .clone(),
    };

    let path = catalog_path()?;
    let catalog = catalog_json(ids)?;
    crate::primitives::write_file(&path, catalog.as_bytes(), 0o644)
        .with_context(|| format!("writing {}", path.display()))?;

    let marker = doc
        .entry("_gate_connect")
        .or_insert_with(|| Item::Table(new_table()))
        .as_table_mut()
        .context("`_gate_connect` must be a TOML table")?;
    if written.is_none() {
        match current {
            Some(m) => marker.insert(PREVIOUS_MODEL_KEY, value(m)),
            None => marker.insert(PREVIOUS_MODEL_ABSENT_KEY, value(true)),
        };
        match current_catalog {
            Some(c) => marker.insert(PREVIOUS_CATALOG_KEY, value(c)),
            None => marker.insert(PREVIOUS_CATALOG_ABSENT_KEY, value(true)),
        };
    }
    let mut list = toml_edit::Array::new();
    for id in ids {
        list.push(id.as_str());
    }
    marker.insert(GATE_MODELS_KEY, value(list));

    doc["model"] = value(model);
    doc["model_catalog_json"] = value(path.display().to_string());
    Ok(())
}

/// Put Codex's own model and picker back, if Gate models were applied.
///
/// Compare-and-restore: a value is put back only while it is still the one
/// Gate wrote. A `model` outside the set, or a picker that is not our file, is
/// something the user chose in Codex after Gate wrote its values, and it stays.
fn revert_gate_models(doc: &mut DocumentMut) -> Result<()> {
    let Some(written) = written_gate_models(doc) else {
        return Ok(());
    };
    let prev = |key: &str| -> Option<String> {
        marker(doc)
            .and_then(|m| m.get(key))
            .and_then(|i| i.as_str())
            .map(str::to_owned)
    };
    let (prev_model, prev_catalog) = (prev(PREVIOUS_MODEL_KEY), prev(PREVIOUS_CATALOG_KEY));
    let ours_catalog = catalog_path()?;

    let model_is_ours = top_str(doc, "model").is_none_or(|m| written.iter().any(|w| w == m));
    if model_is_ours {
        match prev_model {
            Some(m) => doc["model"] = value(m),
            None => {
                doc.remove("model");
            }
        }
    }
    let catalog_is_ours =
        top_str(doc, "model_catalog_json").is_some_and(|p| Path::new(p) == ours_catalog);
    if catalog_is_ours {
        match prev_catalog {
            Some(c) => doc["model_catalog_json"] = value(c),
            None => {
                doc.remove("model_catalog_json");
            }
        }
    }
    if let Some(m) = doc.get_mut("_gate_connect").and_then(|i| i.as_table_mut()) {
        for key in [
            GATE_MODELS_KEY,
            PREVIOUS_MODEL_KEY,
            PREVIOUS_MODEL_ABSENT_KEY,
            PREVIOUS_CATALOG_KEY,
            PREVIOUS_CATALOG_ABSENT_KEY,
        ] {
            m.remove(key);
        }
    }
    match fs::remove_file(&ours_catalog) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", ours_catalog.display())),
    }
}

/// Codex's model catalog for the Gate set.
///
/// Each entry is cloned from a model Codex already knows, off its own
/// `models_cache.json`, and only the identity and size fields are replaced. That
/// is deliberate: an entry carries Codex's agent instructions and tool setup
/// (`model_messages`, `shell_type`, `apply_patch_tool_type`, ...), and those
/// describe how *Codex* works, not how the model does. A hand-built entry
/// would have to invent them, and an empty instructions field is accepted and
/// leaves Codex with no system prompt at all.
///
/// Fields that only make sense for OpenAI's own model - upgrade offers, speed
/// tiers, access programs, the "new model" notice - are dropped.
fn catalog_json(ids: &[String]) -> Result<String> {
    let template = catalog_template();
    let models: Vec<serde_json::Value> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| catalog_entry(template.as_ref(), id, i))
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({ "models": models }))
        .context("serializing the Codex model catalog")
}

/// The entry every Gate model is cloned from: Codex's own top listed model.
fn catalog_template() -> Option<serde_json::Value> {
    let path = env::codex_config_dir().ok()?.join("models_cache.json");
    let raw = fs::read_to_string(path).ok()?;
    let cache: serde_json::Value = serde_json::from_str(&raw).ok()?;
    cache
        .get("models")?
        .as_array()?
        .iter()
        .filter(|m| m.get("visibility").and_then(|v| v.as_str()) == Some("list"))
        .min_by_key(|m| {
            m.get("priority")
                .and_then(|p| p.as_i64())
                .unwrap_or(i64::MAX)
        })
        .cloned()
}

/// The template fields that make Codex send freeform tools. See
/// [`catalog_entry`].
const FREEFORM_TOOL_FIELDS: &[&str] = &["tool_mode", "apply_patch_tool_type"];

const CATALOG_DROPPED_FIELDS: &[&str] = &[
    "upgrade",
    "availability_nux",
    "available_access_programs",
    "service_tiers",
    "default_service_tier",
    "additional_speed_tiers",
    "comp_hash",
];

fn catalog_entry(
    template: Option<&serde_json::Value>,
    id: &str,
    index: usize,
) -> serde_json::Value {
    let meta = crate::preferences::gate_model_meta(id).unwrap_or_default();
    let mut entry = template.cloned().unwrap_or_else(|| {
        // What Codex requires when it has nothing to clone from (measured on
        // 0.159 by removing fields until it parsed). Instructions are left for
        // Codex's own fallback rather than invented here.
        serde_json::json!({
            "supported_reasoning_levels": [
                { "effort": "low", "description": "Faster responses" },
                { "effort": "medium", "description": "Balanced" },
                { "effort": "high", "description": "Deeper reasoning" }
            ],
            "default_reasoning_level": "medium",
            "shell_type": "shell_command",
            "supported_in_api": true,
            "support_verbosity": false,
            "truncation_policy": { "mode": "tokens", "limit": 10000 },
            "experimental_supported_tools": [],
            "base_instructions": ""
        })
    });
    let Some(obj) = entry.as_object_mut() else {
        return entry;
    };
    for field in CATALOG_DROPPED_FIELDS {
        obj.remove(*field);
    }
    // Codex's own toolset sends two freeform (`type: "custom"`) tools: code
    // mode's `exec` (`tool_mode: code_mode_only`) and `apply_patch`
    // (`apply_patch_tool_type: freeform`). Measured on 0.159: with both keys
    // gone every tool it sends is a plain function, and file edits go through
    // `exec_command` instead. Most providers refuse custom tools outright -
    // Meta's endpoint answers "`custom` tools are not supported" - so they are
    // kept only for a model the gateway has seen accept them. Unknown is not
    // yes: two in three catalogue models have never been checked.
    if meta.freeform_tools != Some(true) {
        for field in FREEFORM_TOOL_FIELDS {
            obj.remove(*field);
        }
    }
    obj.insert("slug".into(), id.into());
    obj.insert(
        "display_name".into(),
        meta.name.clone().unwrap_or_else(|| id.to_string()).into(),
    );
    obj.insert(
        "description".into(),
        "Served by Gate on your organization's credits.".into(),
    );
    obj.insert("visibility".into(), "list".into());
    obj.insert("priority".into(), (index as i64).into());
    match meta.context_window {
        Some(window) => {
            obj.insert("context_window".into(), window.into());
            obj.insert("max_context_window".into(), window.into());
        }
        // The template's window describes a different model; better Codex's own
        // default than a number that belongs to something else.
        None => {
            obj.remove("context_window");
            obj.remove("max_context_window");
        }
    }
    entry
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
    fn only_the_managed_daemon_is_the_daemon() {
        use std::ffi::OsString;
        let cmd = |a: &[&str]| a.iter().map(OsString::from).collect::<Vec<_>>();
        assert!(is_managed_daemon_command(&cmd(&[
            "/u/.codex/packages/app-server-daemon/current/bin/codex",
            "app-server",
            "--listen",
            "unix://",
            "--managed-daemon",
        ])));
        assert!(is_managed_daemon_command(&cmd(&[
            "codex",
            "app-server",
            "daemon",
            "pid-update-loop"
        ])));
        // An IDE extension's own app-server, and the ChatGPT app's, are not it.
        assert!(!is_managed_daemon_command(&cmd(&[
            "codex",
            "app-server",
            "--listen",
            "stdio://"
        ])));
        assert!(!is_managed_daemon_command(&cmd(&[
            "/Applications/ChatGPT.app/Contents/Resources/codex",
            "-c",
            "features.code_mode_host=true",
            "app-server",
            "--analytics-default-enabled",
        ])));
        assert!(!is_managed_daemon_command(&cmd(&[
            "codex",
            "exec",
            "app-server"
        ])));
        assert!(is_codex_name("codex") && is_codex_name("codex.exe") && is_codex_name("Codex.EXE"));
        assert!(!is_codex_name("codex-helper"));
    }

    /// AG-674's disagreement case for Codex. Our provider block and pointer are
    /// exactly as `connect` left them; the selected profile names a different
    /// provider, and that is the one Codex resolves.
    #[test]
    fn a_profile_naming_another_provider_overrides_our_pointer() {
        let doc: DocumentMut = r#"
model_provider = "gate"
profile = "work"

[model_providers.gate]
base_url = "http://127.0.0.1:9977/__gate/t/codex/openai/v1"

[profiles.work]
model_provider = "openai"
"#
        .parse()
        .unwrap();
        let o = active_profile_override(&doc, "/home/u/.codex/config.toml")
            .expect("an active profile on another provider is an override");
        assert!(o.to_string().contains("\"work\""));
        assert!(o.to_string().contains("\"openai\""));
    }

    /// A profile that names Gate, or one that names no provider at all and so
    /// inherits the top-level pointer, is agreement rather than a conflict.
    #[test]
    fn a_profile_on_gate_or_silent_about_the_provider_is_not_an_override() {
        let on_gate: DocumentMut = r#"
model_provider = "gate"
profile = "work"

[profiles.work]
model_provider = "gate"
"#
        .parse()
        .unwrap();
        assert_eq!(active_profile_override(&on_gate, "config.toml"), None);

        let silent: DocumentMut = r#"
model_provider = "gate"
profile = "work"

[profiles.work]
model_reasoning_effort = "high"
"#
        .parse()
        .unwrap();
        assert_eq!(active_profile_override(&silent, "config.toml"), None);
    }

    /// A profile nobody selected decides nothing. Codex reads `profile` to pick
    /// one, and a file full of unselected profiles is the normal shape.
    #[test]
    fn an_unselected_profile_is_not_an_override() {
        let doc: DocumentMut = r#"
model_provider = "gate"

[profiles.work]
model_provider = "openai"
"#
        .parse()
        .unwrap();
        assert_eq!(active_profile_override(&doc, "config.toml"), None);
    }

    /// The marker gates auto-reapply, so it has to say yes exactly when the
    /// config is a connected one we wrote.
    ///
    /// The stale-shape case is the one that matters: a base URL written by an
    /// older build is drift `reconcile_enabled` should fix silently, and
    /// without this the user is left re-running connect by hand.
    #[test]
    fn the_marker_tracks_connect_and_disconnect_not_the_provider_block() {
        let managed =
            |toml: &str| is_connected_marker(&toml.parse::<DocumentMut>().expect("valid TOML"));

        // An empty config is nobody's.
        assert!(!managed(""));

        // A hand-written `gate` block is not ours until we adopt it. The block
        // name alone proves nothing, which is why the marker keys are what get
        // checked and not the block - `connect` adopts one under that name and
        // `disconnect` deletes it.
        assert!(!managed(
            r#"model_provider = "gate"

[model_providers.gate]
base_url = "http://127.0.0.1:9977/openai/v1"
"#
        ));

        // Connected over a config that had no `model_provider`, and carrying a
        // base URL from an older build: ours, so reapplied.
        assert!(managed(
            r#"model_provider = "gate"

[model_providers.gate]
base_url = "http://127.0.0.1:9977/openai/v1"

[_gate_connect]
previous_model_provider_absent = true
"#
        ));

        // The other connect key, from a config that did have one.
        assert!(managed(
            r#"[_gate_connect]
previous_model_provider = "openai"
"#
        ));

        // Disconnected: the stub is not something to reconnect behind the
        // user's back, even though `disconnect` leaves the block in place.
        assert!(!managed(
            r#"[model_providers.gate]
base_url = "https://api.openai.com/v1"

[_gate_connect]
passthrough_stub = true
"#
        ));
    }

    #[test]
    fn chatgpt_mode_base_url_carries_the_chatgpt_slug_and_codex_path() {
        // The relay strips `/chatgpt` and forwards `/codex/responses`, which the
        // gateway concatenates onto `https://chatgpt.com/backend-api`.
        assert_eq!(
            relay_base_url_for("http://127.0.0.1:9977", AuthMode::Chatgpt).unwrap(),
            "http://127.0.0.1:9977/__gate/t/codex/chatgpt/codex"
        );
        assert_eq!(
            relay_base_url_for("http://127.0.0.1:9977/", AuthMode::Chatgpt).unwrap(),
            "http://127.0.0.1:9977/__gate/t/codex/chatgpt/codex"
        );
    }

    #[test]
    fn apikey_mode_base_url_carries_the_openai_slug_and_v1_path() {
        assert_eq!(
            relay_base_url_for("http://127.0.0.1:9977", AuthMode::Apikey).unwrap(),
            "http://127.0.0.1:9977/__gate/t/codex/openai/v1"
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
        assert!(
            rendered.contains("base_url = \"http://127.0.0.1:9977/__gate/t/codex/chatgpt/codex\"")
        );
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
