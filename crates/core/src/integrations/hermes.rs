//! Hermes integration.
//!
//! Hermes routes through Gate's MITM proxy engine, not the reverse-proxy relay,
//! and the entire integration is a few variables in `~/.hermes/.env`:
//!
//! ```text
//! HTTPS_PROXY=http://127.0.0.1:<engine-port>
//! HTTP_PROXY=http://127.0.0.1:<engine-port>
//! NO_PROXY=localhost,127.0.0.1,::1
//! HERMES_CA_BUNDLE=<app-support>/proxy/ca-bundle.pem
//! ```
//!
//! **Routing is never written to `config.yaml`.** Hermes loads `$HERMES_HOME/.env` at CLI
//! startup (`hermes_cli/env_loader.py`, called from `cli.py`) before any client
//! is constructed, and `agent/process_bootstrap.py` reads `HTTPS_PROXY` /
//! `HTTP_PROXY` / `ALL_PROXY` (plus lower-case) from the environment, honouring
//! `NO_PROXY` via `proxy_bypass_environment`. Which traffic then reaches Gate is
//! decided by the enabled catalog domains - the engine MITMs those and
//! blind-tunnels everything else.
//!
//! Why this replaced rewriting `model.base_url`: that redirect was per-endpoint,
//! so anything changing which endpoint is live routed around us while `status()`
//! still said Connected. A process-level proxy catches the socket regardless of
//! which provider config won, which retires that whole class - H1 (the
//! native-Anthropic wire) and H6 (`custom_providers` overriding `model.base_url`)
//! both stop being reachable. It also means a fresh install with no `config.yaml`
//! is no longer a special case: nothing below refuses for want of a model block.
//! See `docs/harness-integration-validation.md`.
//!
//! `NO_PROXY` is set for loopback so a locally-hosted provider keeps talking to
//! itself directly instead of being tunnelled through the engine - the same
//! protection the old `is_local_url` guard gave, expressed where Hermes can
//! actually act on it.
//!
//! **`config.yaml` is read to say what Gate will see, and written only to name
//! Hermes on the wire.** The distinction is the one the paragraph above draws.
//! `model.base_url` is a routing directive, and writing it was per-endpoint, so
//! a config change routed around Gate while status said Connected - that is
//! what `.env` replaced and it stays replaced. `model.extra_headers` decides
//! nothing about where a request goes: it adds `x-gate-tool: hermes`, which the
//! engine reads and consumes, and if it is missing the request routes exactly
//! the same and arrives unattributed. So the rule that banned the base_url
//! write does not reach this one, and the header is the only signal the engine
//! can have for Hermes - no base URL means no path marker, and its User-Agent
//! is `python-httpx/...` because it is a Python program.
//!
//! The write is surgical (`yaml_block`) rather than a `serde_yaml` round-trip,
//! because a round-trip returns a semantically equal document with every
//! comment gone, and this is a file the user wrote. A shape that editor will
//! not touch is left alone, costing the label and nothing else.
//! A correct `.env` is only half of being visible: the engine MITMs a host
//! only while an enabled catalog domain claims it, and Hermes' documented default
//! upstream (`openrouter.ai`) ships off, so the traffic can be routed through
//! Gate and blind-tunnelled past it at the same time. Connecting Hermes must not
//! quietly fix that by flipping the domain - that would widen what Gate
//! intercepts for every other client on the machine, and for a domain some
//! provider lists it would flip that provider's state too, which
//! `provider::reconcile_enabled` reads as licence to configure that provider's
//! tools. Whose traffic Gate inspects is the user's axis; whether Hermes points
//! at the proxy is this one. So `connect` prints what it found ([`Coverage`]) and
//! leaves the switch alone.
//!
//! Certificates: `HERMES_CA_BUNDLE` points at a bundle carrying the platform's
//! trust roots *plus* Gate's CA ([`crate::proxy::ca_bundle`]). The OS trust
//! store alone is not enough here. Stdlib Python does read it, but Hermes
//! installs into a venv (`setup-hermes.sh` runs `uv venv` then pip), so its
//! `httpx` / `requests` clients fall back to a **pip-installed** certifi that
//! knows nothing about the CA - measured: `certifi.where()` in that venv is
//! `…/site-packages/certifi/cacert.pem`, not the system bundle. `ssl_verify.py`
//! feeds this value to `create_default_context(cafile=…)`, which *replaces* the
//! trust store rather than adding to it, which is why it must be a full bundle
//! and never our single cert. One variable covers both client libraries -
//! `agent/model_metadata.py` reads the same key for its `requests` callsites.
//!
//! State tracking: a sidecar at `<app_support_dir>/hermes-state.json` records
//! which variables we added, so disconnect removes exactly those and leaves a
//! pre-existing `HTTPS_PROXY` (a corporate egress proxy, say) alone.

//! **Config granularity: per process.** Measured 2026-09-18 by running a
//! `hermes chat` session against a fake proxy named by `HTTPS_PROXY` in an
//! isolated `HERMES_HOME`, repointing that variable at a second proxy
//! mid-session, and sending another turn. Nine fresh requests followed the
//! repoint and every one still went to the *old* proxy. The `.env` is loaded
//! into the environment once at startup, so this matches the mechanism, and
//! "restart it" is the right advice - which is what `connect` already prints.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::integrations::binaries;
use crate::integrations::dotenv;
use crate::integrations::precedence::Override;
use crate::integrations::yaml_block;
use crate::registry::{ConnectInput, Integration, Mechanism, Status, ToolId};

const DISPLAY_NAME: &str = "Hermes";
/// The row label. Hermes is its own family on the ledger, so the heading
/// says "Hermes" and this says which of its surfaces the row is.
const ROW_LABEL: &str = "CLI";
const UPSTREAM_PROVIDER_NAME: &str = "your existing providers";
const DEFAULT_UPSTREAM_URL: &str = "https://openrouter.ai/api/v1";
const STATE_FILENAME: &str = "hermes-state.json";

/// Keep loopback off the proxy so a self-hosted provider is reached directly.
use crate::proxy::NO_PROXY_VALUE;

/// The variable status compares against; the others move with it.
const PRIMARY_VAR: &str = "HTTPS_PROXY";

#[cfg(unix)]
const CLI_BIN_PATHS: &[&str] = &["/usr/local/bin/hermes", "/usr/bin/hermes"];
#[cfg(not(unix))]
const CLI_BIN_PATHS: &[&str] = &[];

/// Sidecar that records what `connect` changed so `disconnect` can undo exactly
/// that and nothing else.
#[derive(Debug, Serialize, Deserialize, Default)]
struct State {
    #[serde(default = "default_version")]
    version: u8,
    /// Variables connect added to `~/.hermes/.env`. Anything the user had
    /// already set is absent, so disconnect leaves it be.
    #[serde(default)]
    added_vars: Vec<String>,
    /// The value connect last wrote for each variable that is ours. A refresh
    /// is gated on the line still holding it, so a value the user has edited by
    /// hand since stops being ours and is left alone. Absent on a sidecar
    /// written before this field existed, which `add_vars` reads as ownership
    /// by key for one connect and then records properly.
    #[serde(default)]
    written_vars: BTreeMap<String, String>,
    /// Whether connect created `.env` itself.
    #[serde(default)]
    env_file_created: bool,
    /// What the `config.yaml` header edit had to create, so disconnect takes
    /// back exactly that. Absent on a state file written before the header
    /// existed, which reads as "we wrote nothing there" - correct, because we
    /// had not.
    #[serde(default)]
    header_created: Option<yaml_block::Created>,
}

fn default_version() -> u8 {
    2
}

pub struct Hermes;

impl Integration for Hermes {
    fn id(&self) -> ToolId {
        ToolId::Hermes
    }

    fn display_name(&self) -> &'static str {
        DISPLAY_NAME
    }

    fn client(&self) -> crate::taxonomy::Client {
        crate::taxonomy::Client::Hermes
    }

    fn row_label(&self) -> &'static str {
        ROW_LABEL
    }

    fn binary(&self) -> (&'static [&'static str], &'static [&'static str]) {
        // `launcher_on_path` already walks PATH for these names; this is the
        // same knowledge, shared so the version probe and the detector cannot
        // disagree about what the binary is called.
        #[cfg(windows)]
        const NAMES: &[&str] = &["hermes.exe", "hermes.cmd", "hermes.bat", "hermes"];
        #[cfg(not(windows))]
        const NAMES: &[&str] = &["hermes"];
        (CLI_BIN_PATHS, NAMES)
    }

    fn upstream_provider_name(&self) -> &'static str {
        UPSTREAM_PROVIDER_NAME
    }

    fn default_upstream_url(&self) -> &'static str {
        DEFAULT_UPSTREAM_URL
    }

    fn config_location(&self) -> Option<String> {
        env_file_path().ok().map(|p| p.display().to_string())
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        // `launcher_on_path` has no path to watch - a `$PATH` entry is not a
        // location - so a Hermes somewhere unusual is the one install this
        // cannot report. The window's read on focus is what covers it.
        let mut paths: Vec<PathBuf> = CLI_BIN_PATHS.iter().map(PathBuf::from).collect();
        paths.extend(launcher_paths().unwrap_or_default());
        paths.extend(crate::env::hermes_config_dir());
        paths.extend(crate::env::hermes_config_path());
        paths
    }

    fn detect(&self) -> Result<bool> {
        // `resolve_binary` subsumes what `launcher_on_path` did by hand - the
        // same PATH walk for the same names - and adds the packaged paths and
        // the shell bin directories a GUI process does not inherit.
        // `launcher_paths` stays: those are Hermes-specific locations no
        // generic search knows about.
        let (well_known, names) = self.binary();
        if binaries::resolve_binary(well_known, names).is_some() {
            return Ok(true);
        }
        Ok(launcher_paths()?.iter().any(|p| p.exists()))
    }

    fn config_is_managed(&self) -> Result<bool> {
        // Two-part marker. The sidecar only exists because connect() wrote it,
        // but on its own it can't tell our stale write apart from a proxy the
        // user has since repointed by hand - so also require that what's on
        // disk still aims at loopback, which is only ever us.
        if load_state()?.is_none() {
            return Ok(false);
        }
        Ok(configured_proxy()?.as_deref().is_some_and(is_loopback_url))
    }

    /// `HTTPS_PROXY` in `.env` names the forwarder's proxy address.
    fn mechanism(&self) -> Mechanism {
        Mechanism::ForwardProxy
    }

    fn configured_addresses(&self) -> Result<Vec<String>> {
        Ok(configured_proxy()?.into_iter().collect())
    }

    fn status(&self) -> Result<Status> {
        if !self.detect()? {
            return Ok(Status::NotInstalled);
        }
        if load_state()?.is_none() {
            return Ok(Status::Detected);
        }
        let configured = configured_proxy()?.unwrap_or_default();
        Ok(compute_status(
            &configured,
            &crate::proxy::tool_proxy_identity_urls(),
            crate::proxy::address_health(&configured),
            crate::proxy::exported_proxy_url().as_deref(),
        ))
    }

    fn connect(&self, input: &ConnectInput) -> Result<()> {
        if !self.detect()? {
            anyhow::bail!(
                "Hermes is not installed -- install it from https://github.com/nousresearch/hermes-agent first"
            );
        }

        // Hard requirement: Hermes sends its traffic to whatever `HTTPS_PROXY`
        // names, so pointing it at an engine that is not running would break
        // its requests rather than merely un-routing them.
        let proxy_url = input.engine_proxy_url.as_deref().context(
            "the Gate proxy is not running -- turn routing on before connecting Hermes, which \
             sends its traffic through the proxy",
        )?;

        // Built before the .env write so a failure here leaves nothing behind.
        let bundle = crate::proxy::ca_bundle::ensure()?;

        // Read before the write: the refusal below has to tell our own earlier
        // work apart from the user's.
        let mut state = load_state()?.unwrap_or_default();

        // What a previous connect wrote, and the value it left there. Passing
        // the value is what keeps this ownership rather than a standing claim
        // on the key: a line the user has since repointed at their own proxy is
        // no longer ours to correct.
        let mut ours: Vec<dotenv::Owned> = state
            .written_vars
            .iter()
            .map(|(key, value)| dotenv::Owned {
                key: key.clone(),
                value: Some(value.clone()),
            })
            .collect();
        // Keys from an install that predates `written_vars`: ours by key alone,
        // for this one connect, and recorded with a value on the way out.
        for key in &state.added_vars {
            if !state.written_vars.contains_key(key) {
                ours.push(dotenv::Owned {
                    key: key.clone(),
                    value: None,
                });
            }
        }

        let applied = dotenv::add_vars(
            &env_file_path()?,
            &[
                ("HTTPS_PROXY", proxy_url.to_string()),
                ("HTTP_PROXY", proxy_url.to_string()),
                ("NO_PROXY", NO_PROXY_VALUE.to_string()),
                ("HERMES_CA_BUNDLE", bundle.display().to_string()),
            ],
            &ours,
        )?;

        // Nothing added AND nothing we ever added: the variables are the user's
        // and `add_vars` left them alone, which is a refusal. The second half of
        // that test is what keeps a re-connect working. On a re-connect every
        // variable is already present because we wrote it, so testing `added`
        // alone failed with a message about settings that were Gate's own - and
        // re-connect is how a drifted Hermes is meant to be repaired, including
        // by `provider::reconcile_unmapped_tools`, which does it unattended.
        //
        // That last sentence was false for as long as `add_vars` could only
        // add. Every key was already present, so a re-connect wrote nothing,
        // reported success and left a stale port in place; the unattended
        // repair ran every launch and fixed nothing, and only toggling the
        // master switch - disconnect, clean file, connect - actually worked.
        // `add_vars` now refreshes the keys the sidecar says are ours, which
        // is what makes the claim true.
        if applied.added.is_empty() && state.added_vars.is_empty() {
            anyhow::bail!(
                "Hermes already has its own proxy settings in ~/.hermes/.env -- Gate left them \
                 alone. Remove them first if you want Hermes to route through Gate."
            );
        }

        // Name Hermes on its own requests. This is the one signal the engine
        // can have for it: Hermes has no base URL for us to write, so there is
        // no path marker, and its User-Agent is `python-httpx/...` because it
        // is a Python program - `client_tool`'s needle for it has never once
        // fired. A header Gate writes into a file only Hermes reads is evidence
        // of our own making, which is the same standard the relay marker meets.
        //
        // `extra_headers` rather than `default_headers`: the two are merged and
        // aliases of each other, so writing the one the user is less likely to
        // be keeping means never having to edit a block that is theirs.
        //
        // Best-effort on purpose. A config shape `yaml_block` will not edit, or
        // no write permission, costs the attribution and nothing else - the
        // routing above is what makes Hermes work, and refusing to connect over
        // a label would be the wrong trade. `status` reports it.
        let header_created = write_tool_header()
            .map_err(|e| {
                eprintln!("note: could not name Hermes in its config ({e:#}); its traffic will be recorded as unattributed.");
            })
            .ok()
            .flatten();
        // Same rule as `added_vars` below, and the same trap: on a re-connect
        // the header is already there and correct, so `write_tool_header`
        // reports creating nothing - and assigning that would erase the record
        // of what the FIRST connect created. Disconnect reads this to know how
        // much to take back out, so erasing it strands our block in the user's
        // config forever.
        if state.header_created.is_none() {
            state.header_created = header_created;
        }

        // Preserve the ORIGINAL record across re-connects: a second connect
        // must not claim credit for variables the first one added.
        if state.added_vars.is_empty() {
            state.version = 2;
            state.added_vars = applied.added;
            state.env_file_created = applied.file_created;
        }
        // The values, unlike the list above, are replaced every time: they are
        // what the file holds now, not who put it there. Recorded even when
        // nothing changed, so a sidecar that predates the field stops relying
        // on ownership by key after a single connect.
        state.written_vars = applied.owned_values.into_iter().collect();
        save_state(&state)?;

        // Naming what moved matters more on a repair than on a first connect.
        // A refreshed key means the file was pointing somewhere Gate no longer
        // listens - a moved loopback port is the case that happens - and until
        // Hermes is restarted it is still using the old value, so a bare "we
        // wrote your config" would be telling the user the half that is already
        // true and omitting the half they have to act on.
        if applied.refreshed.is_empty() {
            eprintln!(
                "note: Hermes reads ~/.hermes/.env at startup -- restart it to pick this up."
            );
        } else {
            eprintln!(
                "note: updated {} in ~/.hermes/.env -- Hermes reads that file at startup, so \
                 restart it or it keeps using the old value.",
                applied.refreshed.join(", ")
            );
        }
        // A correct `.env` is only half of being seen: the engine MITMs a host
        // only while an enabled catalog domain claims it, and Hermes' own
        // default upstream ships off. Which hosts Gate inspects is the user's
        // axis, not this integration's (see [`Coverage`]), so say it rather
        // than silently flip it.
        for line in upstream_coverage().notes() {
            eprintln!("{line}");
        }
        Ok(())
    }

    fn disconnect(&self) -> Result<()> {
        let Some(state) = load_state()? else {
            return Ok(());
        };
        dotenv::remove_vars(&env_file_path()?, &state.added_vars, state.env_file_created)?;
        // Only if we wrote one. A `None` here is a connect that predates the
        // header or one whose write was refused, and in both cases the config
        // is the user's untouched.
        if let Some(created) = state.header_created {
            remove_tool_header(created)?;
        }
        // Only drop the sidecar once the file is back: losing it first would
        // leave our variables in place while status reports the tool clean.
        clear_state()
    }

    fn save_upstream_credential(&self, _credential: &str) -> Result<()> {
        anyhow::bail!(
            "Hermes does not need a separate upstream credential -- Gate routes its traffic through the proxy and passes your provider credentials through untouched."
        )
    }

    fn has_upstream_credential(&self) -> Result<bool> {
        Ok(true)
    }

    fn clear_upstream_credential(&self) -> Result<()> {
        Ok(())
    }
}

/// Pure drift evaluation, split out of [`Hermes::status`] so all four states are
/// testable without a live engine.
///
/// `ours` is every proxy address that belongs to Gate, preferred first - see
/// [`crate::proxy::tool_proxy_identity_urls`] and the note on OpenClaw's
/// equivalent, which this mirrors. `health` is whether the address the config
/// names is answering and routing, which is a question about a different
/// process now that tool configs name the forwarder.
///
/// `exported` is the login environment's own `HTTPS_PROXY`, and it is asked only
/// where the answer would otherwise be Connected - see [`environment_override`].
fn compute_status(
    configured: &str,
    ours: &[String],
    health: crate::proxy::AddressHealth,
    exported: Option<&str>,
) -> Status {
    let Some(expected) = ours.first() else {
        return Status::Drifted(
            "Gate has never bound a proxy port, so nothing can be routing yet".into(),
        );
    };
    if !ours.iter().any(|ours| ours == configured) {
        return Status::Drifted(format!(
            "Hermes config does not match Gate settings (HTTPS_PROXY: {configured:?}, expected: \
             {expected:?})"
        ));
    }
    // Not routing. Two different facts hide under that, and they need
    // different sentences: the address may still be answering (the engine is
    // parked, so it forwards straight through and the tool reaches its own
    // provider), or it may be gone (the app is not running, or the ports were
    // released). Saying "dead address" for both was right when routing off
    // meant the ports went away, and is wrong for the ordinary case now.
    // See OpenClaw's equivalent for why this is three outcomes: `Dead` is the
    // address in the file answering nothing, which the engine being up cannot
    // rule out now that tool configs name the forwarder.
    match health {
        // Routed as far as the address goes, so this is where a higher-ranked
        // configuration layer is the only thing left that can be sending the
        // traffic elsewhere.
        crate::proxy::AddressHealth::Routing => match environment_override(configured, exported) {
            Some(o) => o.into_status(),
            None => Status::Connected,
        },
        crate::proxy::AddressHealth::Parked => Status::Drifted(format!(
            "routing is off, so Hermes reaches its provider directly through {configured:?} \
             rather than through Gate -- turn routing on to route it"
        )),
        crate::proxy::AddressHealth::Dead => Status::Drifted(format!(
            "nothing is listening at {configured:?}, so Hermes cannot reach its provider -- \
             turn routing on, or disconnect Hermes to put its own settings back"
        )),
    }
}

/// The login environment, when it holds a different `HTTPS_PROXY` than the one
/// we wrote into `~/.hermes/.env`.
///
/// Hermes loads that file with python-dotenv, which does not replace a variable
/// the process already has: `load_dotenv()` defaults to `override=False`. So a
/// shell that already exports `HTTPS_PROXY` - a corporate egress proxy, another
/// tool's setup - wins, our line is inert, and Hermes' traffic goes to whatever
/// that names. The file says one thing and the wire does another, which is the
/// whole of AG-674.
///
/// Read from the OS rather than from what we last wrote
/// ([`crate::proxy::exported_proxy_url`] asks `launchctl` / the registry / the
/// drop-in), because a record of our own write cannot contradict us and this
/// check exists precisely to be contradicted. The usual case is agreement:
/// Gate's own environment export puts the same address there, and the same
/// address is not a conflict.
fn environment_override(configured: &str, exported: Option<&str>) -> Option<Override> {
    let exported = exported?;
    if exported == configured {
        return None;
    }
    Some(Override::new(
        "the HTTPS_PROXY exported into your login environment",
        format!(
            "is {exported:?}, and Hermes keeps an already-set variable over the {configured:?} in \
             its .env"
        ),
    ))
}

/// The proxy Hermes is currently pointed at, per its own `.env`.
fn configured_proxy() -> Result<Option<String>> {
    dotenv::read_var(&env_file_path()?, PRIMARY_VAR)
}

/// Whether a proxy URL points at loopback - i.e. is one of ours rather than a
/// corporate egress proxy the user configured themselves.
fn is_loopback_url(url: &str) -> bool {
    let host = url_host(url);
    host == "localhost" || host == "::1" || host.starts_with("127.")
}

/// The host part of a URL, lowercased so it compares against a catalog `hosts`
/// entry directly: `openrouter.ai` for `https://openrouter.ai/api/v1`, `::1` for
/// `http://[::1]:8080`. Deliberately not a URL parser - it also runs over values
/// a user hand-wrote into a YAML file, where a missing scheme is likelier than a
/// query string.
fn url_host(url: &str) -> String {
    let lowered = url.trim().to_ascii_lowercase();
    let rest = lowered
        .split_once("://")
        .map_or(lowered.as_str(), |(_, r)| r);
    let authority = rest.split('/').next().unwrap_or("");
    // Userinfo goes first. A hand-written `https://user:token@host/` is a shape
    // this reads, and the host is the only part that may travel any further:
    // this value crosses IPC into the window and lands in the log.
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    authority
        .strip_prefix('[')
        .and_then(|a| a.split(']').next())
        .unwrap_or_else(|| authority.split(':').next().unwrap_or(""))
        .to_string()
}

/// What Gate will and won't see of this Hermes install, for the notes `connect`
/// prints.
///
/// Read-only on purpose, and the distinction is worth stating precisely because
/// it decides where the fix for this belongs.
///
/// Which hosts the engine intercepts is the user's axis - the provider rows and
/// `proxy domain` - and this integration's axis is only whether Hermes points at
/// the proxy. Enabling a domain from *here* would widen what Gate MITMs for
/// every other client on the machine as a side effect of connecting one tool,
/// and for a domain a provider claims it would flip that provider's state too,
/// which `provider::reconcile_enabled` reads as licence to configure that
/// provider's tools. So this reports, and nothing more.
///
/// **That is an argument against doing it silently, not against asking.** A
/// surface that puts the question to the person and acts on their answer has
/// made it their axis, which is the whole objection satisfied.
/// `OpenCodeEnvDialog` is the same shape already shipped, and a broader one:
/// one switch, a second and wider effect, disclosed, confirmed. Hermes needs
/// exactly that and could not have it while this type was private to a
/// `connect` that prints to stderr - the window has never been able to see any
/// of this.
#[derive(Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Coverage {
    /// Whether the endpoints are Hermes' documented default rather than read
    /// from `config.yaml` - because the file is missing, does not parse, or
    /// names no endpoint. A caller's copy has to say which: "your config uses
    /// OpenRouter" is false about a file that was never read, and an install
    /// with no config really will call OpenRouter.
    pub defaulted: bool,
    /// Provider rows the catalog covers whose switch is off, one per slug.
    pub switched_off: Vec<SwitchedOff>,
    /// Hosts no catalog entry claims, which Gate cannot route at all.
    pub unknown: Vec<String>,
}

/// One provider row Hermes points at that Gate is not inspecting.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SwitchedOff {
    /// The catalog slug, which is what `proxy domain` and `proxy_set_domain`
    /// take.
    pub slug: String,
    /// Every host in `config.yaml` this row claims. One row can claim several,
    /// and it is one switch either way, so a caller that names rows must not
    /// name the same row twice or flip the same switch twice.
    pub hosts: Vec<String>,
    /// Display names of the tools whose provider this domain switches on.
    /// `provider::reconcile_enabled` reads an enabled cascade domain as licence
    /// to connect that provider's detected tools at the next launch, so turning
    /// `anthropic` on for Hermes' sake also reaches Claude Code. Empty for a
    /// proxy-only provider such as OpenRouter. A caller that asks has to say
    /// this, because the switch itself does not.
    pub tools: Vec<String>,
}

impl Coverage {
    /// The lines `connect` prints, or nothing at all when every upstream is
    /// already covered - a note that says "all good" on every connect is noise,
    /// and the `proxy domains` listing is the place to confirm it.
    fn notes(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.switched_off.is_empty() {
            let list = self
                .switched_off
                .iter()
                .map(|s| {
                    format!(
                        "{} (`gate-connect proxy domain {} on`)",
                        s.hosts.join(", "),
                        s.slug
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            // Two openings, because they are two facts: a provider the person
            // wrote into `config.yaml`, or the one Hermes falls back to when
            // they wrote none.
            let subject = if self.defaulted {
                "Hermes names no provider in config.yaml, so it will call its default"
            } else {
                "Hermes is routed through Gate, but Gate is not inspecting its provider yet"
            };
            out.push(format!("note: {subject} -- {list}."));
        }
        if !self.unknown.is_empty() {
            out.push(format!(
                "note: Gate has no proxy domain for {} -- Hermes' calls there keep working, \
                 tunnelled through unseen.",
                self.unknown.join(", ")
            ));
        }
        out
    }
}

/// What Gate will and will not see of this Hermes install.
///
/// Public so the window can raise the question at the moment it matters, which
/// is the click that turns Hermes on. Reads the catalog and `config.yaml` fresh
/// on every call and holds no state, because both move underneath: the person
/// repoints Hermes at a different provider, or a domain is flipped elsewhere -
/// removing and re-trusting a certificate reset `openrouter` to off on the
/// machine that prompted this, hours after Hermes was connected.
pub fn upstream_coverage() -> Coverage {
    // Fall back to the built-in catalog rather than an empty one: on an
    // unreadable domains file the slugs are still right and only the enabled
    // flags are guesses, which beats reporting every host as unroutable.
    let catalog =
        crate::proxy::config::load_domains().unwrap_or_else(|_| crate::proxy::default_domains());
    coverage_from(&catalog, config_base_urls())
}

/// [`upstream_coverage`] over explicit inputs, so the default and the flag that
/// records it are testable without a `$HOME`.
///
/// `None` is a config that named nothing, and it is reported as Hermes'
/// documented default *and marked as such* - the default is the best guess at
/// what an unconfigured install will call, and the mark is what lets a caller
/// say "Hermes' default" rather than "your config" about it.
fn coverage_from(
    catalog: &[crate::proxy::ProxyDomain],
    configured: Option<Vec<String>>,
) -> Coverage {
    let (urls, defaulted) = configured
        .map(|urls| (urls, false))
        .unwrap_or_else(|| (vec![DEFAULT_UPSTREAM_URL.to_string()], true));
    Coverage {
        defaulted,
        ..coverage_of(catalog, &urls)
    }
}

/// The lookup behind [`coverage_from`], over an explicit catalog and URL list.
///
/// Loopback hosts are absent from both lists: `NO_PROXY` exempts them, so a
/// self-hosted provider is reached directly and never passes the engine at all.
/// Keyed by slug, not host: two hosts one row claims are one switch, and a
/// caller that named the row per host would ask about it twice.
fn coverage_of(catalog: &[crate::proxy::ProxyDomain], urls: &[String]) -> Coverage {
    let mut coverage = Coverage::default();
    for url in urls {
        if is_loopback_url(url) {
            continue;
        }
        let host = url_host(url);
        match crate::proxy::domain_claiming_host(catalog, &host) {
            Some(d) if d.enabled => {}
            Some(d) => {
                if let Some(entry) = coverage.switched_off.iter_mut().find(|s| s.slug == d.slug) {
                    if !entry.hosts.contains(&host) {
                        entry.hosts.push(host);
                    }
                } else {
                    coverage.switched_off.push(SwitchedOff {
                        slug: d.slug.clone(),
                        hosts: vec![host],
                        tools: tools_switched_on_by(&d.slug),
                    });
                }
            }
            None => {
                if !coverage.unknown.contains(&host) {
                    coverage.unknown.push(host);
                }
            }
        }
    }
    coverage
}

/// The tools an enabled `slug` licenses [`crate::provider::reconcile_enabled`]
/// to connect: every provider whose cascade includes the domain, and every tool
/// that provider maps. Names rather than ids, because the only reader is a
/// sentence put to the person.
///
/// Deduplicated, though today it cannot repeat: no two providers share a
/// cascade domain or a tool. That disjointness is a property of the catalog,
/// not of this function, and a sentence naming a tool twice is the failure a
/// reader would otherwise have to re-derive the catalog to rule out.
fn tools_switched_on_by(slug: &str) -> Vec<String> {
    let mut names = Vec::new();
    for name in crate::provider::providers()
        .iter()
        .filter(|p| crate::provider::cascade_domains(p).contains(&slug))
        .flat_map(|p| p.tool_ids.iter().copied())
        .filter_map(|id| crate::registry::find(id).map(|integ| integ.display_name().to_string()))
    {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// Keys a Hermes endpoint can be written under. `base_url`, `api` and `url` are
/// documented aliases of one another (see
/// `docs/harness-integration-validation.md`, H6), so all three have to be read.
const BASE_URL_KEYS: &[&str] = &["base_url", "api", "url"];

/// Where the tool header lives in `config.yaml`, and what it says.
///
/// `model.extra_headers` is global - "sent on every request to an
/// OpenAI-compatible endpoint" - unlike the per-provider `extra_headers`, which
/// is scoped to one named entry. That distinction is the whole choice: a
/// per-endpoint setting is what made the old `model.base_url` rewrite unsafe,
/// because adding a provider silently routed around it. A header that is missing
/// costs a label; one that is missing *only sometimes* would be worse than
/// either.
const HEADER_PARENT: &str = "model";
const HEADER_CHILD: &str = "extra_headers";

/// Write the header, returning what had to be created for it.
///
/// `None` means there was nothing to do or nothing we could safely do; the error
/// arm is reserved for I/O, so a refused shape is not reported as a failure.
fn write_tool_header() -> Result<Option<yaml_block::Created>> {
    let path = crate::env::hermes_config_path()?;
    let before = match std::fs::read_to_string(&path) {
        Ok(body) => body,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let (after, edit) = match yaml_block::set_nested(
        &before,
        HEADER_PARENT,
        HEADER_CHILD,
        crate::proxy::GATE_TOOL_HEADER,
        ToolId::Hermes.slug(),
    ) {
        Ok(result) => result,
        Err(refusal) => {
            // A shape the editor will not touch. Said once, plainly, because
            // the user's only visible symptom is otherwise a tool that routes
            // but never appears by name.
            eprintln!(
                "note: left ~/.hermes/config.yaml alone ({refusal:?}); Hermes will route through \
                 Gate but its traffic will be recorded as unattributed."
            );
            return Ok(None);
        }
    };
    match edit {
        // Already ours and already right - and on a re-connect that is the
        // usual answer, so it must not rewrite the file to say so.
        yaml_block::Edit::Unchanged => Ok(None),
        yaml_block::Edit::Refreshed => {
            write_config(&path, &after)?;
            Ok(None)
        }
        yaml_block::Edit::Inserted(created) => {
            write_config(&path, &after)?;
            Ok(Some(created))
        }
    }
}

/// Take the header back out, per what connect recorded creating.
fn remove_tool_header(created: yaml_block::Created) -> Result<()> {
    let path = crate::env::hermes_config_path()?;
    let Ok(before) = std::fs::read_to_string(&path) else {
        // Gone already; nothing of ours is left in a file that is not there.
        return Ok(());
    };
    let after = yaml_block::remove_nested(
        &before,
        HEADER_PARENT,
        HEADER_CHILD,
        crate::proxy::GATE_TOOL_HEADER,
        created,
    );
    if after != before {
        write_config(&path, &after)?;
    }
    Ok(())
}

/// 0o600 like the `.env` beside it: this file can hold `${VAR}`-interpolated
/// header values, and the upstream example uses that for access tokens.
fn write_config(path: &std::path::Path, body: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    crate::primitives::write_file(path, body.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

/// `config.yaml` as YAML, or `None` if it is absent or does not parse. Never an
/// error: nothing here is load-bearing enough to fail a connect over.
fn parsed_config() -> Option<serde_yaml::Value> {
    let path = crate::env::hermes_config_path().ok()?;
    let raw = std::fs::read_to_string(path).ok()?;
    serde_yaml::from_str(&raw).ok()
}

/// Every upstream endpoint `config.yaml` names: `model`, plus each entry of the
/// `providers:` mapping and the legacy `custom_providers:` sequence. All of them
/// and not just the live one, because which entry wins is decided inside Hermes
/// at request time - the same reason the `model.base_url` rewrite this
/// integration used to do was retired.
///
/// `None` when the file names nothing, is missing, or does not parse. The
/// caller substitutes [`DEFAULT_UPSTREAM_URL`] - Hermes' own documented default,
/// and the best available guess at what an unconfigured install will call - and
/// records that it did, because the two are different claims about the person's
/// machine.
fn config_base_urls() -> Option<Vec<String>> {
    parsed_config()
        .map(|root| base_urls_in(&root))
        .filter(|urls| !urls.is_empty())
}

/// The endpoint-collecting half of [`config_base_urls`], over an already-parsed
/// document so the shapes Hermes accepts are testable without a `$HOME`.
fn base_urls_in(root: &serde_yaml::Value) -> Vec<String> {
    let mut urls = Vec::new();
    urls.extend(endpoint_of(root.get("model")));
    if let Some(map) = root
        .get("providers")
        .and_then(serde_yaml::Value::as_mapping)
    {
        urls.extend(map.iter().filter_map(|(_, v)| endpoint_of(Some(v))));
    }
    if let Some(seq) = root
        .get("custom_providers")
        .and_then(serde_yaml::Value::as_sequence)
    {
        urls.extend(seq.iter().filter_map(|v| endpoint_of(Some(v))));
    }
    urls
}

/// The endpoint an entry names, under whichever of [`BASE_URL_KEYS`] it used.
/// `None` for anything that isn't a mapping carrying one - a fresh install's
/// `model: ""`, or a `providers` value that is just a model name.
fn endpoint_of(entry: Option<&serde_yaml::Value>) -> Option<String> {
    let entry = entry?;
    BASE_URL_KEYS
        .iter()
        .filter_map(|k| entry.get(*k).and_then(serde_yaml::Value::as_str))
        .map(str::to_string)
        .next()
}

fn env_file_path() -> Result<PathBuf> {
    Ok(crate::env::hermes_config_dir()?.join(".env"))
}

/// Where the Hermes launcher actually lands, checked instead of "the config
/// directory exists".
///
/// The installer writes `~/.hermes/` and drops the launcher in `~/.local/bin` -
/// so treating the config directory as proof of installation reported Hermes as
/// installed long after it was removed, and left the app offering to configure a
/// CLI that wasn't there.
fn launcher_paths() -> Result<Vec<PathBuf>> {
    let home = crate::env::home()?;
    Ok(vec![
        home.join(".local/bin/hermes"),
        crate::env::hermes_config_dir()?.join("bin/hermes"),
    ])
}

fn state_path() -> Result<PathBuf> {
    Ok(crate::env::app_support_dir()?.join(STATE_FILENAME))
}

fn load_state() -> Result<Option<State>> {
    let path = state_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let state: State =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(state))
}

fn save_state(state: &State) -> Result<()> {
    let path = state_path()?;
    let body = serde_json::to_string_pretty(state).context("serializing hermes-state.json")?;
    crate::primitives::write_file(&path, body.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

fn clear_state() -> Result<()> {
    let path = state_path()?;
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The catalog with one domain forced on, since only `anthropic` ships
    /// enabled and the interesting case is a switch the user has already flipped.
    fn catalog_with(enabled: &str) -> Vec<crate::proxy::ProxyDomain> {
        let mut all = crate::proxy::default_domains();
        for d in &mut all {
            d.enabled = d.slug == enabled;
        }
        all
    }

    fn urls(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_covered_upstream_prints_nothing() {
        let coverage = coverage_of(
            &catalog_with("openrouter"),
            &urls(&["https://openrouter.ai/api/v1"]),
        );
        assert_eq!(coverage, Coverage::default());
        assert!(
            coverage.notes().is_empty(),
            "an all-good note on every connect is noise"
        );
    }

    #[test]
    fn an_upstream_behind_a_switched_off_domain_names_the_switch() {
        let coverage = coverage_of(
            &catalog_with("anthropic"),
            &urls(&["https://openrouter.ai/api/v1"]),
        );
        assert_eq!(
            coverage.switched_off,
            vec![SwitchedOff {
                slug: "openrouter".to_string(),
                hosts: vec!["openrouter.ai".to_string()],
                // Proxy-only: `tool_ids` is empty, so a yes reaches no tool.
                tools: vec![],
            }]
        );

        let notes = coverage.notes();
        assert_eq!(notes.len(), 1, "one line, not one per axis: {notes:?}");
        assert!(
            notes[0].contains("gate-connect proxy domain openrouter on"),
            "the note has to carry the command that fixes it: {}",
            notes[0]
        );
        assert!(
            notes[0].contains("routed through Gate"),
            "and must not read as though Hermes failed to connect: {}",
            notes[0]
        );
    }

    #[test]
    fn an_upstream_gate_cannot_route_says_so_instead() {
        let coverage = coverage_of(
            &catalog_with("anthropic"),
            &urls(&["https://api.together.xyz/v1"]),
        );
        assert!(
            coverage.switched_off.is_empty(),
            "there is no switch to name"
        );
        assert_eq!(coverage.unknown, vec!["api.together.xyz".to_string()]);

        let notes = coverage.notes();
        assert!(
            notes[0].contains("keep working"),
            "the tool still works; only Gate's view is missing: {}",
            notes[0]
        );
    }

    #[test]
    fn loopback_and_duplicate_hosts_are_left_out() {
        let coverage = coverage_of(
            &catalog_with("anthropic"),
            &urls(&[
                // Exempted by NO_PROXY - never reaches the engine.
                "http://127.0.0.1:11434/v1",
                "http://localhost:8080",
                // The same host twice, and the same unknown host twice.
                "https://openrouter.ai/api/v1",
                "https://openrouter.ai/api",
                "https://api.together.xyz/v1",
                "https://api.together.xyz/v2",
            ]),
        );
        assert_eq!(coverage.switched_off.len(), 1, "{coverage:?}");
        assert_eq!(coverage.unknown.len(), 1, "{coverage:?}");
    }

    #[test]
    fn two_hosts_of_one_provider_are_one_switch() {
        // A row that claims two hosts is still one switch. Reported per host,
        // the window would read "Turn on OpenRouter and OpenRouter too?" and
        // flip the same domain twice.
        let mut catalog = catalog_with("anthropic");
        let row = catalog
            .iter_mut()
            .find(|d| d.slug == "openrouter")
            .expect("openrouter is in the built-in catalog");
        row.hosts.push("api.openrouter.example".to_string());
        let coverage = coverage_of(
            &catalog,
            &urls(&[
                "https://openrouter.ai/api/v1",
                "https://api.openrouter.example/v1",
            ]),
        );
        assert_eq!(coverage.switched_off.len(), 1, "{coverage:?}");
        assert_eq!(
            coverage.switched_off[0].hosts,
            vec![
                "openrouter.ai".to_string(),
                "api.openrouter.example".to_string()
            ],
            "both hosts, under the one slug"
        );
    }

    #[test]
    fn a_provider_with_tools_names_what_else_its_switch_reaches() {
        // `anthropic` is a cascade domain of the Anthropic provider, whose
        // `tool_ids` is Claude Code. Enabling it for Hermes' sake hands
        // `reconcile_enabled` licence to connect Claude Code at the next
        // launch, and the dialog has to say so - the switch does not.
        let coverage = coverage_of(
            &catalog_with("openrouter"),
            &urls(&["https://api.anthropic.com/v1"]),
        );
        assert_eq!(coverage.switched_off.len(), 1, "{coverage:?}");
        assert_eq!(coverage.switched_off[0].slug, "anthropic");
        assert_eq!(
            coverage.switched_off[0].tools,
            vec!["Claude Code".to_string()]
        );
    }

    #[test]
    fn a_config_that_names_nothing_is_reported_as_the_default_and_marked() {
        // Missing, unparseable and empty all arrive here as `None`. Hermes will
        // call OpenRouter in every one of those cases, so the row is right;
        // what would be wrong is a sentence saying "your config uses" about a
        // file nobody read, and `defaulted` is what lets the caller avoid it.
        let coverage = coverage_from(&catalog_with("anthropic"), None);
        assert!(coverage.defaulted);
        assert_eq!(coverage.switched_off.len(), 1, "{coverage:?}");
        assert_eq!(coverage.switched_off[0].slug, "openrouter");
        assert!(
            coverage.notes()[0].contains("names no provider"),
            "the CLI note says which claim it is making: {}",
            coverage.notes()[0]
        );

        let configured = coverage_from(
            &catalog_with("anthropic"),
            Some(urls(&["https://openrouter.ai/api/v1"])),
        );
        assert!(!configured.defaulted);
        assert!(configured.notes()[0].contains("routed through Gate"));
    }

    #[test]
    fn every_endpoint_shape_in_config_yaml_is_found() {
        let root: serde_yaml::Value = serde_yaml::from_str(
            "model:\n  provider: custom\n  base_url: https://openrouter.ai/api/v1\n\
             providers:\n  mine:\n    api: https://api.openai.com/v1\n  named: gpt-4o\n\
             custom_providers:\n  - url: https://api.anthropic.com\n",
        )
        .unwrap();

        assert_eq!(
            base_urls_in(&root),
            vec![
                "https://openrouter.ai/api/v1",
                "https://api.openai.com/v1",
                "https://api.anthropic.com",
            ],
            "all three aliases, in all three places, and a bare model name skipped"
        );
    }

    #[test]
    fn an_unconfigured_model_block_yields_no_endpoint() {
        // A fresh install ships `model: ""` (H5), and a provider entry may be a
        // plain model name. Neither is a mapping, so neither names a host - the
        // caller falls back to Hermes' documented default.
        for body in ["model: \"\"\n", "model: gpt-4o\n", "providers:\n  a: b\n"] {
            let root: serde_yaml::Value = serde_yaml::from_str(body).unwrap();
            assert!(
                base_urls_in(&root).is_empty(),
                "expected no endpoints from {body:?}"
            );
        }
    }

    #[test]
    fn host_extraction_survives_the_shapes_a_user_hand_writes() {
        for (url, host) in [
            ("https://openrouter.ai/api/v1", "openrouter.ai"),
            ("openrouter.ai/api/v1", "openrouter.ai"),
            ("HTTPS://OpenRouter.AI/api", "openrouter.ai"),
            ("http://192.168.1.9:8080/v1", "192.168.1.9"),
            ("http://[::1]:8080/v1", "::1"),
            ("  https://api.openai.com  ", "api.openai.com"),
            // Userinfo is a shape a hand-written URL can take, and the host is
            // the only part allowed to travel on: this value crosses IPC.
            ("https://user:s3cret@openrouter.ai/api/v1", "openrouter.ai"),
            ("https://token@[::1]:8080/v1", "::1"),
        ] {
            assert_eq!(url_host(url), host, "for {url}");
        }
    }

    #[test]
    fn compute_status_covers_the_four_states() {
        let ours = "http://127.0.0.1:9977";
        let mine = [ours.to_string()];
        let none: [String; 0] = [];

        assert_eq!(
            compute_status(
                ours,
                &mine,
                crate::proxy::AddressHealth::Routing,
                Some(ours)
            ),
            Status::Connected
        );
        // Nothing exported at all is the other agreeing shape.
        assert_eq!(
            compute_status(ours, &mine, crate::proxy::AddressHealth::Routing, None),
            Status::Connected
        );

        // Pointed at us, address answering, engine parked: Hermes reaches its
        // provider but not through Gate, so this must never read as Connected
        // and must not claim the address is dead.
        match compute_status(ours, &mine, crate::proxy::AddressHealth::Parked, None) {
            Status::Drifted(m) => {
                assert!(m.contains("routing is off"), "unexpected message: {m}");
                assert!(m.contains("directly"), "must say where traffic goes: {m}");
            }
            other => panic!("expected drift, got {other:?}"),
        }

        // Pointed at us and nothing is listening: no egress at all. Separate
        // from the parked case because the engine being up cannot rule it out
        // now that the config names the forwarder.
        match compute_status(ours, &mine, crate::proxy::AddressHealth::Dead, None) {
            Status::Drifted(m) => {
                assert!(
                    m.contains("nothing is listening"),
                    "unexpected message: {m}"
                );
                assert!(m.contains("disconnect"), "must offer a way out: {m}");
            }
            other => panic!("expected drift, got {other:?}"),
        }

        // A corporate proxy the user set by hand is not ours.
        match compute_status(
            "http://proxy.corp.example:3128",
            &mine,
            crate::proxy::AddressHealth::Routing,
            None,
        ) {
            Status::Drifted(m) => assert!(m.contains("does not match"), "unexpected: {m}"),
            other => panic!("expected drift, got {other:?}"),
        }

        match compute_status(ours, &none, crate::proxy::AddressHealth::Dead, None) {
            Status::Drifted(m) => assert!(m.contains("never bound"), "unexpected: {m}"),
            other => panic!("expected drift, got {other:?}"),
        }
    }

    /// AG-674's disagreement case for Hermes. Our four variables are in
    /// `~/.hermes/.env` and correct, the engine is up - and the shell Hermes
    /// starts from already exports a different proxy, which python-dotenv will
    /// not replace. The `.env` is right and the wire is somebody else's.
    #[test]
    fn an_exported_proxy_beats_the_env_file_we_wrote() {
        let ours = "http://127.0.0.1:9977";
        match compute_status(
            ours,
            &[ours.to_string()],
            crate::proxy::AddressHealth::Routing,
            Some("http://proxy.corp.example:3128"),
        ) {
            Status::Overridden(m) => {
                assert!(m.contains("proxy.corp.example:3128"), "unexpected: {m}");
                assert!(m.contains("HTTPS_PROXY"), "must name the variable: {m}");
            }
            other => panic!("expected an override, got {other:?}"),
        }
    }

    /// The address an install written before the forwarder repoint holds is
    /// still ours, so it reads Connected rather than sending a repair over a
    /// config that routes. Only the preferred address is named when something
    /// really has drifted.
    #[test]
    fn an_older_address_of_ours_is_not_drift() {
        let forwarder = "http://127.0.0.1:47101".to_string();
        let engine = "http://127.0.0.1:47100".to_string();
        let ours = [forwarder.clone(), engine.clone()];

        assert_eq!(
            compute_status(&engine, &ours, crate::proxy::AddressHealth::Routing, None),
            Status::Connected
        );
        assert_eq!(
            compute_status(
                &forwarder,
                &ours,
                crate::proxy::AddressHealth::Routing,
                None
            ),
            Status::Connected
        );

        match compute_status(
            "http://proxy.corp.example:3128",
            &ours,
            crate::proxy::AddressHealth::Routing,
            None,
        ) {
            Status::Drifted(m) => {
                assert!(
                    m.contains(&forwarder),
                    "must name the preferred address: {m}"
                );
                assert!(!m.contains(&engine), "must not offer the older one: {m}");
            }
            other => panic!("expected drift, got {other:?}"),
        }
    }

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
}

#[cfg(test)]
mod address_health_tests {
    use super::*;

    /// The regression this branch introduced and this round fixes: the config
    /// names the forwarder, the engine is up, and nothing is listening at the
    /// forwarder. Gating on the engine alone reported Connected over a tool
    /// whose every request failed to connect.
    #[test]
    fn a_dead_address_is_never_connected_however_the_engine_is_doing() {
        let ours = "http://127.0.0.1:9977";
        let mine = [ours.to_string()];
        match compute_status(ours, &mine, crate::proxy::AddressHealth::Dead, None) {
            Status::Drifted(m) => {
                assert!(m.contains("nothing is listening"), "unexpected: {m}");
                assert!(m.contains(ours), "must name the address: {m}");
            }
            other => panic!("expected drift, got {other:?}"),
        }
        assert_eq!(
            compute_status(ours, &mine, crate::proxy::AddressHealth::Routing, None),
            Status::Connected
        );
    }
}
