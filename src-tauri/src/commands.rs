//! The bodies of the Tauri commands, as plain functions over a [`Platform`].
//!
//! Each `#[tauri::command]` in `lib.rs` that has moved here is one call into
//! this module with [`crate::platform::TauriPlatform`], and nothing else. The
//! wiring stays in `lib.rs` (the attribute, the handler table, the async
//! runtime); what the command decides lives here, where a unit test can run it
//! against [`crate::platform::fake::FakePlatform`] (AG-903).

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
use crate::platform::App;
use crate::platform::Platform;
use crate::{
    status_for, AccountDto, BackendError, ConfiguredModelDto, OAuthStatusDto, StatusDto, ToolDto,
    ToolModelChoiceDto, ToolModelsDto,
};
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
use crate::{
    LaunchAtLoginStatus, Relaunch, RunningAgentsDto, TeardownReportDto, TeardownToolDto, VerdictDto,
};
use gate_connect_core::account::{AuthMode, BillingMode};
use gate_connect_core::analytics::{Identity, SaveOutcome};
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
use gate_connect_core::proxy::ProxyState;
use gate_connect_core::{ConnectInput, Integration, ToolId};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

// ---- Tools ----

/// Every tool the UI shows, with its status.
pub(crate) fn list_tools(platform: &impl Platform) -> Vec<ToolDto> {
    // The UI boundary is where hiding happens. The registry itself keeps every
    // integration so the sweep, restore and sign-out paths still clean up a
    // tool someone connected with an earlier build.
    platform
        .integrations()
        .iter()
        .filter(|integ| !integ.hidden_in_ui())
        .map(|integ| ToolDto {
            slug: integ.id().to_string(),
            // The row label, not the product name: this feeds the ledger, whose
            // rows sit under a heading that names the vendor. Every other reader
            // of a tool's name - the CLI, the logs, the quit notice - wants
            // `display_name`, which is why the two are separate.
            name: integ.row_label().to_string(),
            // And the product name beside it, for the readers that are a flat
            // list rather than a grouped ledger: the reopen flow's dialogs and
            // its banner. `registry` has a test asserting display
            // names are distinct precisely because such a list cannot tell two
            // "CLI"s apart.
            product_name: integ.display_name().to_string(),
            upstream_provider_name: integ.upstream_provider_name().to_string(),
            default_upstream_url: integ.default_upstream_url().to_string(),
            config_location: integ.config_location(),
            status: status_for(integ.as_ref()),
            coverage: integ.upstream_coverage(),
            client: integ.client(),
            scope: integ.scope(),
            credential: integ.credential(),
        })
        .collect()
}

/// Every visible tool's version, keyed by slug. Absent where the binary could
/// not be found; null where it was found and would not say.
pub(crate) fn tool_versions(platform: &impl Platform) -> HashMap<String, Option<String>> {
    platform
        .integrations()
        .iter()
        .filter(|integ| !integ.hidden_in_ui())
        .filter_map(|integ| {
            let (well_known, names) = integ.binary();
            if names.is_empty() {
                return None;
            }
            // Resolved here rather than inside the probe so "no binary found"
            // can drop out of the map entirely, while "found, would not say"
            // stays as an explicit null.
            let path = platform.resolve_binary(well_known, names)?;
            Some((integ.id().to_string(), platform.version_at(&path)))
        })
        .collect()
}

/// One tool's status, by slug.
pub(crate) fn tool_status(platform: &impl Platform, slug: &str) -> Result<StatusDto, String> {
    let integ = find_integration(platform, slug)?;
    Ok(status_for(integ.as_ref()))
}

/// The integration behind a slug. An unknown slug and a known tool the
/// registry lacks are two different messages.
fn find_integration(platform: &impl Platform, slug: &str) -> Result<Box<dyn Integration>, String> {
    let id = ToolId::from_slug(slug).ok_or_else(|| format!("unknown tool {slug:?}"))?;
    platform
        .integrations()
        .into_iter()
        .find(|i| i.id() == id)
        .ok_or_else(|| "integration missing from registry".to_string())
}

/// Route one tool through Gate, starting the engine its config will point at.
pub(crate) fn connect_tool(platform: &impl Platform, slug: &str) -> Result<StatusDto, String> {
    let integ = find_integration(platform, slug)?;
    let account = platform
        .account()
        .map_err(|e| format!("{e:#}"))?
        .ok_or_else(|| "Sign in to Gate AI first".to_string())?;
    // Auto-enable the proxy so the reverse-proxy relay is live: relay-routed
    // tool configs point at the loopback relay, which only exists while the
    // engine runs. Idempotent if the proxy is already on.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        // Persist the routing intent too, and before the engine comes up
        // (see `routing::enable` for the ordering): connecting a tool is
        // as deliberate a "route through Gate" as the master switch, and
        // this engine start must survive a restart - the tool's config
        // keeps pointing at the relay across a quit, so a launch that
        // doesn't bring the relay back leaves the tool dialing a dead
        // loopback port while its ledger row still reads connected.
        // Deliberately not the full `routing::enable` ceremony: a connect
        // is scoped to one tool, so it must not also restore providers a
        // master-off swept.
        if let Err(e) = platform.set_routing_intent(true) {
            eprintln!("[gate] persisting routing intent on connect failed: {e}");
            platform.report_backend_error("routing_intent", format!("{e:#}"));
        }
        platform.enable_engine().map_err(|e| format!("{e:#}"))?;
    }
    let input = ConnectInput {
        gateway_base_url: account.gateway_base_url,
        billing_mode: account.billing_mode,
        relay_base_url: platform.relay_base_url(),
        engine_proxy_url: platform.tool_proxy_url(),
    };
    integ.connect(&input).map_err(|e| format!("{e:#}"))?;
    Ok(status_for(integ.as_ref()))
}

/// Put one tool back on its own settings.
pub(crate) fn disconnect_tool(platform: &impl Platform, slug: &str) -> Result<StatusDto, String> {
    let integ = find_integration(platform, slug)?;
    integ.disconnect().map_err(|e| format!("{e:#}"))?;
    Ok(status_for(integ.as_ref()))
}

/// This install's per-tool model choices, with what each tool's own config
/// says.
pub(crate) fn tool_model_preferences(platform: &impl Platform) -> ToolModelsDto {
    // Configs first: a tool moved off Gate models from inside itself is put
    // back on its own model here, and the stored choices read below must
    // already say so.
    let configured = platform.tool_model_states();
    let prefs = platform.preferences();
    ToolModelsDto {
        tools: prefs
            .tool_models
            .into_iter()
            .map(|(slug, choice)| (slug, ToolModelChoiceDto::from(choice)))
            .collect(),
        paid_ack_unix: prefs.gate_model_paid_ack_unix,
        configured: configured
            .into_iter()
            .map(|(slug, view)| (slug.to_string(), ConfiguredModelDto::from(view)))
            .collect(),
    }
}

/// Choose the model one tool runs on. True when the tool's config changed.
pub(crate) fn set_tool_model(
    platform: &impl Platform,
    tool: String,
    source: &str,
    model_ids: Vec<String>,
    acknowledge_paid_use: bool,
) -> Result<bool, String> {
    // Parsed, not trusted: the slug has to be one this app actually configures,
    // or the pane would store a choice under a key nothing reads.
    let Some(tool) = crate::parse_tool(Some(tool))? else {
        return Err("a tool slug is required to set a model preference".into());
    };
    let source = match source {
        "tool" => gate_connect_core::preferences::ModelSource::Tool,
        "gate" => gate_connect_core::preferences::ModelSource::Gate,
        other => return Err(format!("unknown model source {other:?}")),
    };
    // The names and context windows the tool's own picker will show, read
    // now because the connect that writes them may run offline later. A
    // catalogue that cannot be read costs the picker its labels, not the
    // choice.
    let meta = match source {
        gate_connect_core::preferences::ModelSource::Gate => platform
            .gate_model_catalogue()
            .map(|json| gate_connect_core::tool_models::meta_from_catalogue(&json, &model_ids))
            .unwrap_or_default(),
        gate_connect_core::preferences::ModelSource::Tool => Vec::new(),
    };
    let applied = platform
        .choose_tool_model(tool, source, model_ids, acknowledge_paid_use, meta)
        .map_err(|e| format!("{e:#}"))?;
    // Codex's app-server daemon reads its config and model catalog only
    // when it starts; see `refresh_codex_daemon_when_idle`.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    if applied && tool == ToolId::Codex {
        platform.refresh_codex_daemon_when_idle("set model");
    }
    Ok(applied)
}

/// One row per model provider.
pub(crate) fn list_providers(
    platform: &impl Platform,
) -> Vec<gate_connect_core::provider::ProviderState> {
    platform.providers()
}

// ---- This machine ----

/// OS identifier ("macos" / "windows" / "linux") so the UI can tailor copy.
pub(crate) fn app_platform(platform: &impl Platform) -> &'static str {
    platform.os()
}

/// OS marketing name and version, for the analytics error context.
pub(crate) fn os_name(platform: &impl Platform) -> String {
    platform.os_name()
}

/// The backend half of the diagnostics report.
pub(crate) fn diagnostics(platform: &impl Platform) -> gate_connect_core::diagnostics::Diagnostics {
    platform.diagnostics()
}

/// This install's stable id.
pub(crate) fn install_id(platform: &impl Platform) -> Result<String, String> {
    platform.install_id().map_err(|e| format!("{e:#}"))
}

/// Write a webview log line at the level it names. An unknown level is
/// information, never an error (`Level::from_wire`).
pub(crate) fn log_message(platform: &impl Platform, level: &str, message: &str) {
    platform.log(gate_connect_core::logging::Level::from_wire(level), message);
}

// ---- Preferences ----

/// The user's Settings choices.
pub(crate) fn get_preferences(
    platform: &impl Platform,
) -> gate_connect_core::preferences::Preferences {
    platform.preferences()
}

/// What to call this machine.
pub(crate) fn device_name(platform: &impl Platform) -> String {
    platform.device_name()
}

/// Rename this device, or clear the name and follow the hostname again.
pub(crate) fn set_device_name(platform: &impl Platform, name: &str) -> Result<(), String> {
    platform.set_device_name(name).map_err(|e| format!("{e:#}"))
}

/// Turn native notifications on or off.
pub(crate) fn set_notifications(platform: &impl Platform, enabled: bool) -> Result<(), String> {
    platform
        .set_notifications(enabled)
        .map_err(|e| format!("{e:#}"))
}

/// Turn the sound on security notifications on or off.
pub(crate) fn set_security_notification_sound(
    platform: &impl Platform,
    enabled: bool,
) -> Result<(), String> {
    platform
        .set_security_notification_sound(enabled)
        .map_err(|e| format!("{e:#}"))
}

/// Record the provider domains Gate switched on for one tool.
pub(crate) fn record_auto_enabled_domains(
    platform: &impl Platform,
    tool: &str,
    domains: Vec<String>,
) -> Result<(), String> {
    platform
        .record_auto_enabled_domains(tool, domains)
        .map_err(|e| format!("{e:#}"))
}

/// Read what Gate switched on for one tool, without clearing it.
pub(crate) fn read_auto_enabled_domains(platform: &impl Platform, tool: &str) -> Vec<String> {
    platform.read_auto_enabled_domains(tool)
}

// ---- Account ----

/// Tell every mounted window that the signed-in session moved.
///
/// The account is the scope of nearly everything on screen: the org name, the
/// activity figures, the security feed's buffer, the installation list. Each
/// shell keys its readings on a credential string built from the account, and
/// drops them when it changes - but only the window that *made* the change knew,
/// because none of the mutating commands emitted anything. So the tray kept the
/// previous org's counts and header until something unrelated woke it, and a
/// sign-out left figures on screen for an account that could no longer read them.
///
/// Emitted after the change has landed, never before: a window that re-read on
/// the way in would read the state being replaced.
///
/// Not to be confused with `signal_session_dead` and its
/// `session-signin-required`, which says the gateway *refused* a session the user
/// did not change. This one says the user changed it on purpose.
fn signal_session_changed(platform: &impl Platform) {
    platform.emit("session-changed", Value::Null);
}

/// The stored account, reconciled against its on-disk anchor first.
pub(crate) fn get_account(platform: &impl Platform) -> Result<Option<AccountDto>, String> {
    // Reconcile the stored account against its on-disk anchor before reading it,
    // so the first-run-vs-home decision this call drives always sees a
    // consistent view. An uninstall that removed Gate Connect's files but left
    // its OS keychain entry behind (macOS drag-to-trash, or a deep uninstaller
    // that purges Application Support but can't touch the keychain). Dropping
    // that orphaned key here, rather than on a startup thread that races this
    // read, means it can't briefly route the user to a half-signed-in home. A
    // key-less account.json (URL but no key) is left intact - it's a pending-key
    // state and the read below reports has_api_key=false, so the UI routes to
    // key entry. Best-effort: a reconcile hiccup must not flip a signed-in user
    // to first-run, so we log and fall through to the read below.
    if let Err(e) = platform.reconcile_account() {
        eprintln!("account reconcile failed: {e}");
        if platform.first_reconcile_failure() {
            platform.report_backend_error("account_reconcile", format!("{e:#}"));
        }
    }
    let Some(gateway_base_url) = platform.account_base_url().map_err(|e| format!("{e:#}"))? else {
        return Ok(None);
    };
    let has_api_key = platform.has_api_key().map_err(|e| format!("{e:#}"))?;
    let auth_mode = platform.auth_mode().map_err(|e| format!("{e:#}"))?;
    let billing_mode = platform.billing_mode().map_err(|e| format!("{e:#}"))?;
    let (org_id, org_name) = match platform.selected_org().map_err(|e| format!("{e:#}"))? {
        Some((id, name)) => (Some(id), Some(name)),
        None => (None, None),
    };
    Ok(Some(AccountDto {
        gateway_base_url,
        has_api_key,
        auth_mode,
        billing_mode,
        org_id,
        org_name,
    }))
}

/// Leading characters of the stored Gate key, from `account.json`.
pub(crate) fn get_account_key_prefix(platform: &impl Platform) -> Result<Option<String>, String> {
    platform.api_key_prefix().map_err(|e| format!("{e:#}"))
}

/// The IPC-boundary checks on a gateway base URL, shared by `save_account`
/// and `switch_gateway`.
fn check_base_url(base_url: &str) -> Result<(), String> {
    if base_url.len() > 2048 {
        return Err("base url is unexpectedly long (>2048 bytes)".into());
    }
    let parsed = url::Url::parse(base_url).map_err(|e| format!("invalid base url: {e}"))?;
    if !crate::base_url_scheme_ok(&parsed) {
        return Err(crate::base_url_scheme_error());
    }
    if parsed.host_str().is_none() {
        return Err("base url is missing a host".into());
    }
    Ok(())
}

/// Store the gateway, and a pasted key when one is given.
pub(crate) fn save_account(
    platform: &impl Platform,
    base_url: &str,
    api_key: Option<&str>,
) -> Result<(), String> {
    check_base_url(base_url)?;
    let key = api_key.map(str::trim).filter(|s| !s.is_empty());
    if let Some(k) = key {
        crate::validate_api_key(k, "sk-")?;
    }
    platform
        .save_account(base_url, key)
        .map_err(|e| format!("{e:#}"))?;
    // A rotated key is hot-swapped into the running proxy engine below;
    // the relay injects it live per request, so no tool config embeds it.
    if let Some(k) = key {
        // Pasting a key selects the legacy path explicitly.
        platform
            .set_auth_mode(AuthMode::ApiKey)
            .map_err(|e| format!("{e:#}"))?;
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            // Drop any live OAuth bearer from the running engine now that
            // we're in ApiKey mode. The background refresh loop is gated on
            // OAuth mode, so it won't clear it; and a still-valid Cognito
            // session would otherwise keep overriding the pasted key until
            // the token expired. No-op when routing is off.
            platform.refresh_engine_token("");
            platform.refresh_engine_api_key(k);
        }
    }
    // What changed: the key, the gateway, or both.
    signal_session_changed(platform);
    Ok(())
}

/// Sign out and forget the account: tools first, then the forwarder, then the
/// account itself.
pub(crate) fn clear_account(platform: &impl Platform) -> Result<(), String> {
    // Disconnect managed tools first: clearing the account while their
    // configs still embed the key would leave them routing to the gateway
    // with a dead credential on disk. A failure aborts the sign-out.
    let done = platform
        .disconnect_all_managed()
        .map_err(|e| format!("{e:#}"))
        .and_then(|()| {
            // And stop the environment forwarder. It is deliberately left running
            // across a plain routing-off - that is exactly when the processes
            // holding our exported variables still need it - so this path and the
            // CA untrust are the only places it is retired. This is the one an
            // ordinary user reaches: it is what the Reset button runs
            // (`App.tsx`'s `forget`, which disables routing and then calls here).
            platform.stop_forwarder();
            platform.clear_account().map_err(|e| format!("{e:#}"))
        });
    // What changed: the account is gone.
    if done.is_ok() {
        // Same reason as the org switch: the events belonged to the account
        // being cleared, and the feed outlives it.
        platform.security_feed().reset_for_account_change();
        signal_session_changed(platform);
    }
    // `account::clear` forgot the analytics identity (through `oauth::clear`);
    // tell every window, so none keeps the account that went (AG-960).
    announce_stored_analytics_identity(platform);
    done
}

/// Dev-mode gateway switch. Tools out first, then the engine, then the
/// account: the engine pins the gateway URL at start, so one left up would
/// keep rewriting traffic to the old environment under the new one's token.
pub(crate) fn switch_gateway(platform: &impl Platform, base_url: &str) -> Result<(), String> {
    check_base_url(base_url)?;
    platform
        .disconnect_all_managed()
        .map_err(|e| format!("{e:#}"))?;
    // Before the account moves, so the engine can never be up against an
    // account it wasn't started from. A failure aborts the switch: routing
    // to the old gateway with the new environment's credential is the state
    // this whole command exists to avoid.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    platform.shutdown_engine().map_err(|e| format!("{e:#}"))?;
    platform
        .switch_gateway(base_url)
        .map_err(|e| format!("{e:#}"))?;
    // What changed: another gateway, and the key with it.
    signal_session_changed(platform);
    Ok(())
}

/// Explicitly set the auth mode.
pub(crate) fn set_auth_mode(platform: &impl Platform, oauth: bool) -> Result<(), String> {
    let mode = if oauth {
        AuthMode::OAuth
    } else {
        AuthMode::ApiKey
    };
    platform.set_auth_mode(mode).map_err(|e| format!("{e:#}"))?;
    // An OAuth account holds no key (`account::load`), so neither may a
    // running engine: see `oauth_begin_login`.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    if oauth {
        platform.refresh_engine_api_key("");
    }
    // What changed: which credential the gateway is given.
    signal_session_changed(platform);
    Ok(())
}

/// Switch who pays the upstream provider, and re-apply a connected Codex.
pub(crate) fn set_billing_mode(platform: &impl Platform, payg: bool) -> Result<(), String> {
    let mode = if payg {
        BillingMode::Payg
    } else {
        BillingMode::Byok
    };
    platform
        .set_billing_mode(mode)
        .map_err(|e| format!("{e:#}"))?;
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    platform.refresh_engine_mode();
    reapply_codex_for_mode(platform, mode);
    Ok(())
}

/// Rewrite Codex's provider block for `mode`, if Codex is currently routed
/// through Gate. Silent when Codex isn't installed or isn't connected - there
/// is nothing to rewrite, and a mode switch is not the place to start routing a
/// tool the user never connected.
fn reapply_codex_for_mode(platform: &impl Platform, mode: BillingMode) {
    let Some(integ) = platform
        .integrations()
        .into_iter()
        .find(|i| i.id() == ToolId::Codex)
    else {
        return;
    };
    if !matches!(integ.status(), Ok(gate_connect_core::Status::Connected)) {
        return;
    }
    let Ok(Some(account)) = platform.account() else {
        return;
    };
    let input = ConnectInput {
        gateway_base_url: account.gateway_base_url,
        billing_mode: mode,
        relay_base_url: platform.relay_base_url(),
        engine_proxy_url: platform.engine_proxy_url(),
    };
    if let Err(e) = integ.connect(&input) {
        eprintln!("re-applying Codex for the new billing mode failed: {e:#}");
    }
}

/// Persist the selected org and push it into a running engine, then clear
/// what belonged to the previous one.
pub(crate) fn set_org(
    platform: &impl Platform,
    org_id: &str,
    org_name: &str,
) -> Result<(), String> {
    platform
        .set_org(org_id, org_name)
        .map_err(|e| format!("{e:#}"))?;
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    platform.refresh_engine_org(org_id);
    // What changed: another org, so every figure on screen belongs to the previous one.
    //
    // The feed included. It is a process singleton, so without this its
    // buffer, its dedupe set and its catch-up verdict all survive the
    // switch - and `security_feed_recent` hands the previous org's events
    // straight back to a window that has just cleared its own copy. That is
    // the one thing the activity surfaces are not allowed to do: show one
    // org's traffic under another org's name.
    //
    // `reset_for_account_change` was written for exactly this and had no
    // production caller at all, only a test, so the guarantee its name makes
    // was never kept.
    platform.security_feed().reset_for_account_change();
    signal_session_changed(platform);
    Ok(())
}

// ---- OAuth ----

/// Stop an interactive login that is still waiting for the browser.
pub(crate) fn oauth_cancel_login(platform: &impl Platform) {
    platform.cancel_login();
}

/// Current OAuth sign-in status.
pub(crate) fn oauth_status(platform: &impl Platform) -> Result<OAuthStatusDto, String> {
    // Share the injector's source of truth (`live_session`): refresh a stale
    // access token so status reflects a live session, and report signed-out when
    // there's no usable session - never signed in, signed out, or the refresh
    // token is dead / unreachable. Reporting signed-out here is what routes the
    // UI back to the sign-in prompt instead of showing a signed-in home that's
    // actually riding the legacy API-key fallback. Keeping a running engine's
    // token fresh is the background refresh loop's job (see `run()`), not this
    // read's, so status stays a read that never mutates engine state.
    use gate_connect_core::oauth::SessionReading;
    Ok(match platform.session_reading() {
        SessionReading::Live(t) => OAuthStatusDto::from(&t),
        other => OAuthStatusDto {
            signed_in: false,
            email: None,
            sub: None,
            session: if matches!(other, SessionReading::Unavailable) {
                "unavailable"
            } else {
                "signed_out"
            },
            expires_at_unix: 0,
        },
    })
}

/// Forget the stored OAuth tokens (sign out). Leaves `auth_mode` at `OAuth`.
pub(crate) fn oauth_sign_out(platform: &impl Platform) -> Result<(), String> {
    let done = platform.clear_oauth_session().map_err(|e| format!("{e:#}"));
    if done.is_ok() {
        // Say that this was asked for. The state left behind is identical to an
        // expired session - no token, `auth_mode` still OAuth by design two
        // lines up - so without this the welcome pane can only guess, and it
        // guessed "Session expired" over the user's own deliberate click.
        // Best-effort and after the credential, on the same reasoning as the
        // cache clear below: a preferences write must not be the reason a
        // sign-out reports failure.
        let _ = platform.set_signed_out_deliberately(true);
        // The held activity readings belong to the org just signed out of, and
        // signing out is not a disconnect: `account.json` keeps the gateway and
        // the org, so `activity_cache`'s scope stays byte-identical and every
        // reading in it would still match and still be served. `account::clear`
        // covers the disconnect path; this covers the one that leaves the account
        // file in place. Best effort and after the credential, on the same
        // reasoning as there: a cache that will not delete must not be the reason
        // a sign-out reports failure.
        let _ = platform.clear_activity_cache();
        // Take the token out of a running engine now. The account stays in
        // OAuth mode and holds no key, so routed requests are refused as
        // signed out rather than sent under anything.
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        platform.refresh_engine_token("");
        // What changed: the OAuth session is gone, and `account.json` is not.
        signal_session_changed(platform);
    }
    // `oauth::clear` forgot the analytics identity; tell every window (AG-960).
    announce_stored_analytics_identity(platform);
    done
}

/// The orgs the signed-in user may act on. A refusal the gateway recorded is
/// news for the whole app, so it raises the dead-session signal too.
pub(crate) fn oauth_list_orgs(
    platform: &impl Platform,
) -> Result<Vec<gate_connect_core::org::Org>, String> {
    let listed = platform.list_orgs().map_err(|e| format!("{e:#}"));
    if listed.is_err() && platform.session_rejected() {
        platform.signal_session_dead();
    }
    listed
}

// ---- Activity and Gate models ----

/// The 24-hour activity overview, as the gateway's JSON. Failures cross as
/// the JSON envelope.
pub(crate) fn activity_overview(
    platform: &impl Platform,
    install_id: Option<&str>,
    tools: Option<Vec<String>>,
) -> Result<String, String> {
    let clients = gate_connect_core::activity::overview_clients(&tools.unwrap_or_default())?;
    platform
        .activity_overview(install_id, &clients)
        .map_err(crate::envelope)
}

/// The last overview that landed for this scope, or `None`.
pub(crate) fn activity_cached_overview(
    platform: &impl Platform,
    install_id: Option<&str>,
    tools: Option<Vec<String>>,
) -> Result<Option<String>, String> {
    let clients = gate_connect_core::activity::overview_clients(&tools.unwrap_or_default())?;
    Ok(platform.cached_activity_overview(install_id, &clients))
}

/// Every held per-tool reading for this installation scope, keyed by slug.
pub(crate) fn activity_cached_tool_overviews(
    platform: &impl Platform,
    install_id: Option<&str>,
) -> BTreeMap<String, String> {
    platform.cached_tool_overviews(install_id)
}

/// The first page of the named senders' recent requests.
pub(crate) fn activity_tool_events(
    platform: &impl Platform,
    install_id: Option<&str>,
    tools: &[String],
) -> Result<String, String> {
    let clients = gate_connect_core::activity::feed_clients(tools)?;
    platform
        .activity_tool_events(install_id, &clients)
        .map_err(crate::envelope)
}

/// The installations this account has sent traffic from.
pub(crate) fn activity_installations(platform: &impl Platform) -> Result<String, String> {
    platform.activity_installations().map_err(crate::envelope)
}

/// The models this gateway offers, for the picker.
pub(crate) fn gate_model_catalogue(platform: &impl Platform) -> Result<String, String> {
    platform.gate_model_catalogue().map_err(crate::envelope)
}

/// This organization's Gate credit balance and plan.
pub(crate) fn gate_credits(platform: &impl Platform) -> Result<String, String> {
    platform.gate_credits().map_err(crate::envelope)
}

// ---- Analytics ----

/// Tell every window what analytics identity is stored now. The core does the
/// forgetting (`oauth::clear`), so every caller of it, the CLI included,
/// changes the record; this is the shell's half, because only the shell has
/// windows to tell. Without it each window kept the old account for the rest
/// of the session.
fn announce_stored_analytics_identity(platform: &impl Platform) {
    announce_analytics_identity(platform, platform.analytics_identity());
}

/// Broadcast an analytics identity on the event every window's analytics seam
/// follows.
fn announce_analytics_identity(platform: &impl Platform, identity: Identity) {
    // A payload that will not serialise is not emitted, as `Emitter::emit`
    // did with the typed value.
    if let Ok(identity) = serde_json::to_value(identity) {
        platform.emit(crate::ANALYTICS_IDENTITY_EVENT, identity);
    }
}

/// What this install is identified as in analytics, after finishing a forget
/// a sign-out could not land. When it forgot, the other windows are told too.
pub(crate) fn analytics_identity(platform: &impl Platform) -> Identity {
    let forgot = platform
        .forget_identity_if_signed_out()
        .unwrap_or_else(|e| {
            eprintln!("analytics identity: finishing a forget failed: {e:#}");
            false
        });
    let identity = platform.analytics_identity();
    if forgot {
        announce_analytics_identity(platform, identity.clone());
    }
    identity
}

/// Record a change of analytics identity, and tell every window from inside
/// the core's lock, so announcements reach them in the order of the writes.
pub(crate) fn set_analytics_identity(
    platform: &impl Platform,
    identity: Identity,
) -> Result<(), String> {
    match platform.save_identity(identity, &mut |stored| {
        announce_analytics_identity(platform, stored.clone())
    }) {
        Ok(SaveOutcome::Saved) => Ok(()),
        Ok(SaveOutcome::NotLive) => Err(crate::ANALYTICS_IDENTITY_NOT_LIVE.to_string()),
        Ok(SaveOutcome::Unconfirmed) => Err(crate::ANALYTICS_IDENTITY_UNCONFIRMED.to_string()),
        Err(e) => Err(format!("{e:#}")),
    }
}

/// Claim a once-per-install analytics milestone. Any rejection reads as "do
/// not send" in the webview.
pub(crate) fn analytics_milestone_claim(
    platform: &impl Platform,
    name: &str,
) -> Result<bool, String> {
    platform.claim_milestone(name).map_err(|e| format!("{e:#}"))
}

/// Why local Cowork cannot run on this machine, or `None`.
pub(crate) fn cowork_setting_check(platform: &impl Platform) -> Option<&'static str> {
    platform.cowork_setting_missing()
}

/// Record whether Gate Connect may send diagnostic data, then tell every
/// window.
pub(crate) fn set_share_diagnostics(platform: &impl Platform, enabled: bool) -> Result<(), String> {
    platform
        .set_share_diagnostics(enabled)
        .map_err(|e| format!("{e:#}"))?;
    // Every window runs its own analytics client, and only the one the user
    // clicked in knows the answer changed. Broadcast it so the tray and the
    // intro stop (or start) too, instead of acting on the answer they read at
    // launch until the next one (AG-960).
    platform.emit(
        crate::ANALYTICS_CONSENT_EVENT,
        serde_json::json!({ "share_diagnostics": enabled, "recorded": true }),
    );
    Ok(())
}

// ---- Security feed ----

/// What the feed is doing: `live`, `reconnecting` or `offline`.
pub(crate) fn security_feed_state(
    platform: &impl Platform,
) -> gate_connect_core::security_feed::FeedState {
    platform.security_feed().state()
}

/// Whether the events from before this connection could be fetched.
pub(crate) fn security_feed_history_ok(platform: &impl Platform) -> bool {
    platform.security_feed().history_ok()
}

/// The events the feed has buffered, oldest first.
pub(crate) fn security_feed_recent(
    platform: &impl Platform,
) -> Vec<gate_connect_core::security_feed::SecurityEvent> {
    platform.security_feed().recent()
}

/// The "Try again" behind an Unavailable feed: wake the connection loop.
pub(crate) fn security_feed_retry(platform: &impl Platform) {
    platform.security_feed().retry_now();
}

// ---- Routing ----

/// Whether already-running routed clients may be pointing at a dead port.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn routed_clients_stale(platform: &impl Platform) -> bool {
    platform.routed_clients_may_be_stale()
}

/// Whether the startup auto-enable is still in flight.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn routing_startup_pending(platform: &impl Platform) -> bool {
    platform.startup_enable_pending()
}

/// Where the tools stand after a teardown, read back from each tool's own
/// config rather than from what the teardown believed it wrote.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn teardown_report(platform: &impl Platform) -> TeardownReportDto {
    let mut report = TeardownReportDto {
        defaults: Vec::new(),
        still_gate: Vec::new(),
        awaiting_reopen: Vec::new(),
        failed: Vec::new(),
    };
    for integ in platform.integrations() {
        if integ.hidden_in_ui() {
            continue;
        }
        let slug = integ.id().to_string();
        let status = integ.status();
        // Not on the machine: it has no configuration to put back, and listing it
        // would pad a report the user reads as a to-do list.
        if matches!(status, Ok(gate_connect_core::Status::NotInstalled)) {
            continue;
        }
        let tool = |next_action: &'static str| TeardownToolDto {
            slug: slug.clone(),
            name: integ.display_name().to_string(),
            next_action,
        };
        match status {
            Ok(gate_connect_core::Status::Connected)
            | Ok(gate_connect_core::Status::Drifted(_))
            | Ok(gate_connect_core::Status::Overridden(_)) => {
                report.still_gate.push(tool("retry_disconnect"))
            }
            Ok(_) if platform.reopen_pending(&slug) => {
                report.awaiting_reopen.push(tool("reopen_tool"))
            }
            Ok(_) => report.defaults.push(tool("none")),
            Err(_) => report.failed.push(tool("retry_check")),
        }
    }
    report
}

// ---- Proxy ----
//
// The built-in MITM proxy, on the platforms where CA trust and system-proxy
// wiring are implemented. Each adapter runs its body on the blocking pool
// except `proxy_set_domain` and the two launch-at-login commands, which were
// sync before they moved and stay sync.

/// The proxy's state.
///
/// The adapter runs this on the blocking pool, never on the thread driving the
/// event loop (bedf13a): on Windows the read shells out to `certutil` for the
/// CA-trust reading, and a `certutil` that hangs froze the window and left the
/// app unquittable. `ca_windows::certutil_bounded` caps that wait; the blocking
/// pool keeps even the capped wait off the UI thread.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn proxy_status(platform: &impl Platform) -> Result<ProxyState, String> {
    platform.proxy_status().map_err(|e| format!("{e:#}"))
}

/// Read the browser stores, once, for the note the window raises when trust
/// is granted somewhere this process could not see it happen. The window asks
/// on a transition and never on a poll - see `proxy::probe_browser_store`.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn proxy_browser_store(
    platform: &impl Platform,
) -> Option<gate_connect_core::proxy::NssTrust> {
    platform.probe_browser_store()
}

/// Turn routing on, then show it: tray first, then the crash safety net.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn proxy_enable(platform: &(impl Platform + App)) -> Result<ProxyState, String> {
    // Master ON is one policy shared with the CLI and the startup
    // auto-enable: persist the intent (so the startup auto-enable
    // re-routes after a restart - whether the app relaunches at boot is
    // governed separately by "Launch at login"), restore the provider
    // selection around the engine start, and surface best-effort hiccups
    // without blocking the proxy from coming up.
    let (_, warnings) = platform.enable_routing().map_err(|e| format!("{e:#}"))?;
    platform.refresh_codex_daemon_when_idle("routing on");
    for w in warnings {
        eprintln!("[gate] proxy enable: {} failed: {:#}", w.component, w.error);
        platform.report_backend_failure(w.component, &w.error);
    }
    // Status re-read rather than enable's own state: the post-enable
    // restore pass can flip domains, and the UI wants the settled set.
    let state = platform.proxy_status().map_err(|e| format!("{e:#}"))?;
    // Refresh the tray for the new routing state on every platform: retint the
    // mark, recolor the status dot (green on / gray off), and update the
    // tooltip where supported (macOS + Windows).
    platform.update_tray_status(state.running);
    // Crash safety net: routing is now on, but with no login item registered a
    // crash would strand the system proxy at a dead port with nothing
    // relaunching at boot to run the startup self-heal. (macOS/Windows only;
    // see `arm_crash_safety_net` in lib.rs for why Linux is excluded.)
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    platform.arm_crash_safety_net();
    Ok(state)
}

/// Turn routing off, then show it, then finish a deferred launch-at-login
/// opt-out that was waiting for exactly this.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn proxy_disable(platform: &(impl Platform + App)) -> Result<ProxyState, String> {
    // Master OFF is one policy shared with the CLI: sweep + disconnect
    // everything managed before the proxy stops, then clear the routing
    // intent so explicit "off" is sticky across restarts (whether the app
    // relaunches at boot is governed separately by "Launch at login").
    // Best-effort hiccups surface as warnings and never block the kill
    // switch.
    let (state, warnings) = platform.disable_routing().map_err(|e| format!("{e:#}"))?;
    for w in warnings {
        eprintln!(
            "[gate] proxy disable: {} failed: {:#}",
            w.component, w.error
        );
        platform.report_backend_failure(w.component, &w.error);
    }
    // Refresh the tray for the new routing state on every platform: retint the
    // mark, recolor the status dot (green on / gray off), and update the
    // tooltip where supported (macOS + Windows).
    platform.update_tray_status(state.running);
    // A deferred launch-at-login opt-out can complete now: with routing off,
    // deregistering can no longer strand the system proxy across a restart.
    platform.complete_pending_autostart_disable();
    Ok(state)
}

/// What Gate would and would not see of this Hermes install right now. Enables
/// no domain and writes no file; what the caller does with the answer is ask.
/// See the adapter in lib.rs for why it is uncached.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn hermes_upstream_coverage(
    platform: &impl Platform,
) -> gate_connect_core::integrations::hermes::Coverage {
    platform.hermes_upstream_coverage()
}

/// The operator toggling one interception domain by hand, audited.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn proxy_set_domain(
    platform: &impl Platform,
    slug: &str,
    enabled: bool,
) -> Result<ProxyState, String> {
    let state = platform
        .set_domain(slug, enabled)
        .map_err(|e| format!("{e:#}"))?;

    // Audited here rather than inside `ProxyManager::set_domain`, because
    // `provider::enable` / `provider::disable` drive that method internally -
    // instrumenting it there would turn one operator action into N+1 events.
    // This command is the operator toggling one domain by hand.
    if let Ok(Some(base_url)) = platform.account_base_url() {
        platform.audit_domain_toggled(&base_url, slug, enabled);
    }
    Ok(state)
}

/// Opt in or out of exporting the proxy to the session environment, and read
/// the state back.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn proxy_set_env_export(
    platform: &impl Platform,
    enabled: bool,
) -> Result<ProxyState, String> {
    platform
        .set_env_export(enabled)
        .map_err(|e| format!("{e:#}"))?;
    platform.proxy_status().map_err(|e| format!("{e:#}"))
}

/// Trust the CA. Pops an interactive prompt.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn proxy_trust_ca(platform: &impl Platform) -> Result<ProxyState, String> {
    platform.trust_ca().map_err(|e| format!("{e:#}"))
}

/// Untrust the CA. Can pop an interactive prompt.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn proxy_untrust_ca(platform: &impl Platform) -> Result<ProxyState, String> {
    platform.untrust_ca().map_err(|e| format!("{e:#}"))
}

/// What every config-routed tool is actually doing. The adapter in lib.rs
/// says why this is its own command and why it runs on the blocking pool.
///
/// The two probes run **once** for the whole sweep, not once per tool. They ask
/// about shared infrastructure - the relay port and the account's session - so
/// per-tool calls would be the same answer at N times the cost, and would let
/// two rows in one refresh disagree about whether the session is alive.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn routing_verdicts(platform: &impl Platform) -> Vec<VerdictDto> {
    use gate_connect_core::routing_health::{self, ConfigState, Evidence};

    let route = platform.probe_relay_route();
    let session = platform.probe_session_health();
    // One read for the whole sweep, like the two probes: every row that needs to
    // name the Gate route names the same one.
    let gate_route = platform
        .account_base_url()
        .ok()
        .flatten()
        .unwrap_or_else(|| "Constellation Gate".to_string());

    let mut recorded: Vec<(String, routing_health::RoutingVerdict)> = Vec::new();
    let integrations = platform.integrations();
    let checked: Vec<_> = integrations
        .iter()
        .filter(|integ| !integ.hidden_in_ui())
        .map(|integ| {
            let status = integ.status();
            let installed = !matches!(status, Ok(gate_connect_core::Status::NotInstalled));
            let slug = integ.id().to_string();
            let config = ConfigState::from_status(&status);
            let verdict = routing_health::verdict_for(&Evidence {
                installed,
                config,
                route,
                session,
                reopen_pending: platform.reopen_pending(&slug),
            });
            // The log keeps what the check found; only the row is held.
            recorded.push((slug.clone(), verdict));
            (integ, slug, config, verdict)
        })
        .collect();
    // One hold for the process, so both shells draw the same answer. It holds
    // by time rather than by count of checks, because both shells sweep, so
    // how many checks arrive depends on how many are open. Taken only here,
    // after every `status()` above: those read each tool's config off disk,
    // and holding the lock across them queued one shell's sweep behind the
    // other's I/O. A poisoned lock only loses the history, which reads as
    // nothing held.
    let mut hold = platform
        .verdict_hold()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let now = std::time::Instant::now();
    let verdicts: Vec<VerdictDto> = checked
        .into_iter()
        .map(|(integ, slug, config, verdict)| {
            let verdict = hold.apply(&slug, verdict, now);
            let reason = verdict.reason();
            // Only the half that was read off disk. The surfaces draw the pair
            // when both are present and omit it otherwise, so a reopen notice
            // now names the action rather than an endpoint nobody measured.
            let routes = if matches!(reason, Some(routing_health::Reason::ReopenRequired)) {
                gate_connect_core::reopen::reopen_routes(
                    config,
                    &gate_route,
                    integ.default_upstream_url(),
                )
            } else {
                gate_connect_core::reopen::ReopenRoutes {
                    in_use: None,
                    requested: None,
                }
            };
            VerdictDto {
                slug,
                state: verdict.as_str(),
                reason: reason.map(|r| r.as_str()),
                next_action: reason.map(|r| r.next_action().as_str()),
                route_in_use: routes.in_use,
                requested_route: routes.requested,
            }
        })
        .collect();
    drop(hold);
    // Persisted after the sweep, not during it: the log is what lets the recovery
    // summary report a check it did not take, and a half-written sweep would be a
    // worse record than the previous whole one.
    platform.record_verdict_sweep(&recorded);
    verdicts
}

// ---- Launch at login ----
//
// A standalone user setting (Settings screen) that owns the login item
// directly - it is no longer armed/disarmed by the routing toggle. Disabling it
// while routing is on is two-step: the opt-out marker and the
// defer-vs-deregister decision live in `gate_connect_core::proxy::
// autostart_optout` (see its module docs for the full rationale), and only the
// OS login-item calls go through the platform.

/// The user's launch-at-login choice, and whether its removal is deferred.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn launch_at_login_status(
    platform: &(impl Platform + App),
) -> Result<LaunchAtLoginStatus, String> {
    let registered = platform
        .login_item_registered()
        .map_err(|e| format!("{e:#}"))?;
    let pending = platform.autostart_opt_out_pending();
    Ok(LaunchAtLoginStatus {
        enabled: registered && !pending,
        pending_disable: pending,
    })
}

/// Turn launch at login on, or off now or at the next safe point.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn set_launch_at_login(
    platform: &(impl Platform + App),
    enabled: bool,
) -> Result<(), String> {
    // This marker/login-item read-then-write and `arm_crash_safety_net`
    // share no lock; see the accepted-race note on that function.
    if enabled {
        platform
            .set_autostart_opt_out_pending(false)
            .map_err(|e| format!("{e:#}"))?;
        platform
            .register_login_item()
            .map_err(|e| format!("{e:#}"))?;
        // The plugin has just rewritten the LaunchAgent from its template,
        // taking the crash-restart policy with it. Put it back.
        #[cfg(target_os = "macos")]
        platform.arm_crash_restart();
        Ok(())
    } else if platform
        .record_autostart_opt_out()
        .map_err(|e| format!("{e:#}"))?
    {
        platform
            .deregister_login_item()
            .map_err(|e| format!("{e:#}"))
    } else {
        // Routing is on: the opt-out is deferred and the login item stays
        // registered until the next safe point.
        Ok(())
    }
}

// ---- Processes ----

/// The running agent processes `only` names (every agent for `None`): what
/// each one is, and whether it needs a reopen.
///
/// The report the webview decides the routing-change takeover from (f2f8c6c):
/// an empty `agents` is how it learns there is nothing to offer, so this reports
/// exactly what the walk found and nothing else.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn running_agents(
    platform: &impl Platform,
    only: Option<&[String]>,
) -> RunningAgentsDto {
    let names = crate::agent_names_for(only);
    let mut agents = platform.running_agents(&names);
    // Oldest first: the ones that need a reopen are the ones being looked
    // for, and a stable order keeps two reports from the same machine
    // diffable.
    //
    // **The tie-break is load-bearing, not tidiness.** `reopen.ts`'s
    // `reopenTools` keeps the FIRST row per slug, and since the ChatGPT app and
    // the `codex` it bundles resolve to one slug (AG-947), two rows now compete
    // to speak for that app. `start_time` is whole seconds and both processes
    // can land in the same one; the walk itself is a `HashMap` iteration, and
    // `sort_by_key` is stable, so a tie would hand the row to whichever the map
    // happened to yield first. When that is the helper the app reports
    // `can_reopen: false` and the dialog offers no way to reopen it.
    //
    // So: relaunchable first among equals, then pid, which is total. Before
    // the two shared a slug there was no contest and the plain sort was right.
    agents.sort_by_key(|agent| (agent.started_at_unix, !agent.can_reopen, agent.pid));
    RunningAgentsDto {
        scanned_names: names.iter().map(|n| n.to_string()).collect(),
        agents,
    }
}

/// Close running agents so their next launch picks up the routing change, and
/// queue the apps among them for [`reopen_running_agents`]. Returns how many
/// processes closed.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn close_running_agents(platform: &impl Platform, only: Option<&[String]>) -> u32 {
    let closed = platform.close_agents(only);
    if only.is_none_or(|slugs| slugs.iter().any(|s| s == "codex")) {
        platform.refresh_codex_daemon_when_idle("close agents");
    }
    let mut reopen: Vec<(String, Relaunch)> = Vec::new();
    for target in &closed {
        if let (Some(slug), Some(relaunch)) = (target.slug, &target.relaunch) {
            // One launch per app, however many of its processes were closed.
            if !reopen.iter().any(|(s, _)| s == slug) {
                reopen.push((slug.to_string(), relaunch.clone()));
            }
        }
    }
    if !reopen.is_empty() {
        let mut guard = platform
            .pending_reopen()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Replace rather than append for the slugs in hand: a second close of
        // the same app must not queue a second launch of it.
        guard.retain(|(slug, _)| !reopen.iter().any(|(s, _)| s == slug));
        guard.extend(reopen);
    }
    closed.len() as u32
}

/// Put back the apps [`close_running_agents`] closed, once their old
/// instances are gone (bounded). Returns how many were launched.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn reopen_running_agents(platform: &impl Platform, only: Option<&[String]>) -> u32 {
    let pending: Vec<(String, Relaunch)> = {
        let mut guard = platform
            .pending_reopen()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (take, keep): (Vec<_>, Vec<_>) = guard
            .drain(..)
            .partition(|(slug, _)| only.is_none_or(|slugs| slugs.iter().any(|s| s == slug)));
        *guard = keep;
        take
    };
    if pending.is_empty() {
        return 0;
    }

    // Bounded wait for the old instances to go. 5s is long enough for an app
    // asked to quit gracefully and short enough that a user watching a button
    // does not conclude it is broken.
    let names: Vec<&str> = crate::AGENT_PROCESSES
        .iter()
        .filter(|(slug, _, _, _)| pending.iter().any(|(s, _)| s == slug))
        .map(|(_, name, _, _)| *name)
        .collect();
    platform.wait_for_agents_to_exit(&names, std::time::Duration::from_secs(5));

    pending
        .iter()
        .filter(|(_, relaunch)| platform.relaunch(relaunch))
        .count() as u32
}

// ---- Exit ----

/// What `RunEvent::Exit` does, on macOS and Windows: put every tool back,
/// revert the system proxy, and finish a deferred launch-at-login opt-out.
///
/// On app exit, revert the system proxy so traffic is never stranded at the
/// now-dead engine port. The engine lives in a process-global static whose
/// Drop is bypassed at normal exit, so without this the system proxy stays
/// pointed at a dead listener and kills connectivity until the next launch's
/// self-heal. `disable_quiet` is promptless and leaves the CA trusted.
///
/// **It never touches the routing intent** (7fc3d16). Exit reverts the
/// *system proxy* only; the intent records what is true while the app runs,
/// and the next launch enables routing however it happens - see "Routing
/// follows the app" in CLAUDE.md.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn exit(platform: &(impl Platform + App)) {
    // Put every tool back on its own settings on *every* exit, not
    // only the one that goes through `quit_app`: the same teardown
    // (`disconnect_tools_for_quit`), so a quit means one thing
    // however it arrives.
    //
    // `quit_app` is reached by the tray's Quit, the window menu's
    // and the crash screen's, and by nothing else: macOS Cmd+Q
    // comes from Tauri's default app menu, and a logout or a
    // shutdown comes from the OS, and both land here having touched
    // none of our own code. This used to revert only the tools
    // whose address dies with this process, leaving the forwarder's
    // tools pointed at Gate and working with nothing reading their
    // traffic. After `quit_app` it is skipped: the teardown runs
    // once per process (`claim_quit_teardown`), because a second
    // pass after a partial first one is not a no-op.
    //
    // Before `disable_quiet` below, deliberately: the provider half
    // turns domains off in the engine, which has to still be up.
    //
    // Not on an updater relaunch: the app is coming straight back,
    // so the sweep would rewrite every tool's config and the
    // restore would undo it, costing each of them a restart for
    // nothing.
    //
    // Deliberately does not *veto* the exit. `ExitRequested` can be
    // prevented - `code` is `None` exactly when something outside
    // our own code asked to quit, so Cmd+Q could be routed into a
    // dialog. It is not, because that event also carries a logout
    // and a shutdown, and an app that puts a dialog in front of
    // those is an app that hangs the user's logout. For the same
    // reason the sweep gives up rather than waiting on another
    // routing operation.
    //
    // Unwound rather than trusted not to panic: everything below it
    // has to run, `disable_quiet` above all, and the sweep is the
    // largest block on this path.
    //
    // No notification either. `quit_app` can fire one because it
    // runs before the exit; by the time this runs the process is
    // going away and a notification would be a promise we cannot
    // keep.
    if !platform.updater_relaunching() && platform.claim_quit_teardown() {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            platform.disconnect_tools_for_quit()
        })) {
            Ok(Ok(teardown)) if teardown.failed.is_empty() => {}
            Ok(Ok(teardown)) => eprintln!(
                "[gate] {} could not be put back on their own settings on exit",
                crate::join_names(&teardown.failed)
            ),
            Ok(Err(e)) => {
                eprintln!("[gate] putting tools back on exit failed: {e}")
            }
            Err(_) => eprintln!("[gate] putting tools back on exit panicked"),
        }
    }
    // Reaching this event at all is what
    // makes the exit clean. Recording it clears the unclean streak,
    // so a user who quits normally after a crash gets the restart
    // policy back rather than carrying the streak forever.
    if let Err(e) = platform.record_clean_exit() {
        eprintln!("[gate] recording clean exit failed: {e:#}");
    }
    if let Err(e) = platform.disable_engine_quietly() {
        eprintln!("[gate] reverting proxy on exit failed: {e}");
    }
    // The engine's relay goes with this process, parked or not, so
    // the forwarder should stop asking its port whether it is up.
    platform.forget_engine_relay_port();
    // The login item is now a standalone "Launch at login" setting,
    // decoupled from routing. A deferred opt-out (toggled off while
    // routing was on) completes here: disable_quiet() above has
    // reverted the system proxy, so deregistering can no longer
    // strand it. An updater-driven relaunch is exempt - the app
    // comes right back, so the pending opt-out stays armed and
    // routing is restored exactly as the user left it.
    if !platform.updater_relaunching() {
        platform.complete_pending_autostart_disable();
    }
    // Exit reverts the *system proxy* only. The routing intent is
    // not touched here, and must not be: see the doc above.
}

// ---- Windows ----

/// Stop pinning the popover open.
pub(crate) fn unpin_popover(platform: &impl Platform) {
    platform.set_popover_pinned(false);
}

/// Pin the popover open across a call that raises a system dialog.
pub(crate) fn pin_popover(platform: &impl Platform) {
    platform.set_popover_pinned(true);
}

/// Hand one window its buffered backend failures and clear its copy only.
pub(crate) fn drain_backend_errors(platform: &impl Platform, label: &str) -> Vec<BackendError> {
    platform.drain_backend_errors(label)
}

// ---- Updater ----

/// Mark (or unmark) the next exit as an updater-driven relaunch.
pub(crate) fn set_updater_relaunching(platform: &impl Platform, relaunching: bool) {
    platform.set_updater_relaunching(relaunching);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::fake::{FakePlatform, FakeTool};
    use gate_connect_core::logging::Level;
    use gate_connect_core::Status;

    #[test]
    fn app_platform_reports_the_platform_os() {
        for os in ["macos", "windows", "linux"] {
            let platform = FakePlatform {
                os,
                ..Default::default()
            };
            assert_eq!(app_platform(&platform), os);
        }
    }

    #[test]
    fn set_notifications_stores_the_answer() {
        let platform = FakePlatform::default();
        assert_eq!(set_notifications(&platform, false), Ok(()));
        assert_eq!(*platform.notifications.lock().unwrap(), Some(false));
        assert!(platform.emitted().is_empty(), "nothing to announce");
    }

    /// The whole anyhow chain reaches the webview, not only the outermost
    /// context, so the reason a write failed is still in the message.
    #[test]
    fn set_notifications_reports_the_whole_error_chain() {
        let platform = FakePlatform {
            fail_writes: Some("disk full".into()),
            ..Default::default()
        };
        assert_eq!(
            set_notifications(&platform, true),
            Err("writing preferences: disk full".into())
        );
        assert_eq!(*platform.notifications.lock().unwrap(), None);
    }

    /// The payload the webview reads (`src/lib/analytics.ts`), on the event it
    /// listens on, after the answer is stored.
    #[test]
    fn set_share_diagnostics_stores_then_announces_the_answer() {
        for enabled in [true, false] {
            let platform = FakePlatform::default();
            assert_eq!(set_share_diagnostics(&platform, enabled), Ok(()));
            assert_eq!(*platform.share_diagnostics.lock().unwrap(), Some(enabled));
            assert_eq!(
                platform.emitted(),
                vec![(
                    "analytics-consent-changed".to_string(),
                    serde_json::json!({ "share_diagnostics": enabled, "recorded": true }),
                )]
            );
        }
    }

    /// A refused write announces nothing: the other windows would otherwise
    /// follow an answer that was never recorded.
    #[test]
    fn set_share_diagnostics_announces_nothing_when_the_write_fails() {
        let platform = FakePlatform {
            fail_writes: Some("read-only file system".into()),
            ..Default::default()
        };
        assert_eq!(
            set_share_diagnostics(&platform, false),
            Err("writing preferences: read-only file system".into())
        );
        assert!(platform.emitted().is_empty());
    }

    fn failing_writes() -> FakePlatform {
        FakePlatform {
            fail_writes: Some("disk full".into()),
            ..Default::default()
        }
    }

    /// The registry keeps hidden tools so cleanup still reaches them; the UI
    /// must not list them.
    #[test]
    fn list_tools_leaves_out_tools_hidden_from_the_ui() {
        let platform = FakePlatform {
            tools: vec![
                FakeTool::new(ToolId::ClaudeCode),
                FakeTool {
                    hidden: true,
                    ..FakeTool::new(ToolId::Codex)
                },
                FakeTool::new(ToolId::Hermes),
            ],
            ..Default::default()
        };
        let slugs: Vec<_> = list_tools(&platform).into_iter().map(|t| t.slug).collect();
        assert_eq!(slugs, ["claude-code", "hermes"]);
    }

    /// A status that cannot be read is a row in the error state, carrying the
    /// whole error chain, not a failed listing.
    #[test]
    fn list_tools_shows_an_unreadable_status_as_an_error_row() {
        let platform = FakePlatform {
            tools: vec![FakeTool {
                status: Err("permission denied".into()),
                ..FakeTool::new(ToolId::OpenCode)
            }],
            ..Default::default()
        };
        let tools = list_tools(&platform);
        assert_eq!(tools.len(), 1);
        assert!(matches!(
            &tools[0].status,
            StatusDto::Error { message } if message == "reading the config: permission denied"
        ));
    }

    /// Not installed drops out of the map; installed but silent stays as null;
    /// a tool with no binary and a hidden tool are never probed.
    #[test]
    fn tool_versions_tells_not_installed_from_unreadable() {
        let platform = FakePlatform {
            tools: vec![
                FakeTool {
                    binaries: &["claude"],
                    ..FakeTool::new(ToolId::ClaudeCode)
                },
                FakeTool {
                    binaries: &["codex"],
                    ..FakeTool::new(ToolId::Codex)
                },
                FakeTool {
                    binaries: &["opencode"],
                    ..FakeTool::new(ToolId::OpenCode)
                },
                FakeTool::new(ToolId::EnvProxy),
                FakeTool {
                    binaries: &["hermes"],
                    hidden: true,
                    ..FakeTool::new(ToolId::Hermes)
                },
            ],
            binaries: [
                ("claude", Some("2.1.0".to_string())),
                ("codex", None),
                ("hermes", Some("0.1".to_string())),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let versions = tool_versions(&platform);
        assert_eq!(
            versions,
            HashMap::from([
                ("claude-code".to_string(), Some("2.1.0".to_string())),
                ("codex".to_string(), None),
            ])
        );
    }

    #[test]
    fn tool_status_reads_the_named_tool() {
        let platform = FakePlatform {
            tools: vec![
                FakeTool::new(ToolId::ClaudeCode),
                FakeTool {
                    status: Ok(Status::Drifted("base URL changed".into())),
                    ..FakeTool::new(ToolId::Codex)
                },
            ],
            ..Default::default()
        };
        assert!(matches!(
            tool_status(&platform, "codex"),
            Ok(StatusDto::Drifted { reason }) if reason == "base URL changed"
        ));
    }

    /// An unknown slug and a known tool the registry lacks are two different
    /// messages, so a support thread can tell a typo from a broken build.
    #[test]
    fn tool_status_names_why_it_found_nothing() {
        let platform = FakePlatform {
            tools: vec![FakeTool::new(ToolId::ClaudeCode)],
            ..Default::default()
        };
        assert!(matches!(
            tool_status(&platform, "nope"),
            Err(e) if e == "unknown tool \"nope\""
        ));
        assert!(matches!(
            tool_status(&platform, "codex"),
            Err(e) if e == "integration missing from registry"
        ));
    }

    /// The webview's level names map onto the log's, and anything else is
    /// information rather than an invented error.
    #[test]
    fn log_message_maps_the_wire_level() {
        let platform = FakePlatform::default();
        for level in ["error", "warn", "info", "debug", "ERROR"] {
            log_message(&platform, level, level);
        }
        assert_eq!(
            *platform.logged.lock().unwrap(),
            vec![
                (Level::Error, "error".to_string()),
                (Level::Warn, "warn".to_string()),
                (Level::Info, "info".to_string()),
                (Level::Info, "debug".to_string()),
                (Level::Info, "ERROR".to_string()),
            ]
        );
    }

    #[test]
    fn install_id_reports_the_whole_error_chain() {
        let platform = FakePlatform {
            install_id: Some("abc-123".into()),
            ..Default::default()
        };
        assert_eq!(install_id(&platform), Ok("abc-123".into()));
        assert_eq!(
            install_id(&FakePlatform::default()),
            Err("reading install id: no install id".into())
        );
    }

    #[test]
    fn set_device_name_stores_the_name_or_reports_why_not() {
        let platform = FakePlatform::default();
        assert_eq!(set_device_name(&platform, "Work laptop"), Ok(()));
        assert_eq!(device_name(&platform), "Work laptop");

        let platform = failing_writes();
        assert_eq!(
            set_device_name(&platform, "Work laptop"),
            Err("writing preferences: disk full".into())
        );
        assert_eq!(device_name(&platform), "");
    }

    #[test]
    fn set_security_notification_sound_stores_the_answer_or_reports_why_not() {
        let platform = FakePlatform::default();
        assert_eq!(set_security_notification_sound(&platform, false), Ok(()));
        assert_eq!(*platform.security_sound.lock().unwrap(), Some(false));
        assert_eq!(
            set_security_notification_sound(&failing_writes(), true),
            Err("writing preferences: disk full".into())
        );
    }

    /// What one tool recorded reads back for that tool only.
    #[test]
    fn auto_enabled_domains_read_back_per_tool() {
        let platform = FakePlatform::default();
        assert_eq!(
            record_auto_enabled_domains(&platform, "codex", vec!["api.openai.com".into()]),
            Ok(())
        );
        assert_eq!(
            read_auto_enabled_domains(&platform, "codex"),
            ["api.openai.com"]
        );
        assert!(read_auto_enabled_domains(&platform, "hermes").is_empty());
        assert_eq!(
            record_auto_enabled_domains(&failing_writes(), "codex", Vec::new()),
            Err("writing preferences: disk full".into())
        );
    }

    /// True once per name, and a failure is an error the webview reads as "do
    /// not send", never a second true.
    #[test]
    fn analytics_milestone_claim_is_true_once() {
        let platform = FakePlatform::default();
        assert_eq!(
            analytics_milestone_claim(&platform, "first_route"),
            Ok(true)
        );
        assert_eq!(
            analytics_milestone_claim(&platform, "first_route"),
            Ok(false)
        );
        assert_eq!(
            analytics_milestone_claim(&failing_writes(), "first_route"),
            Err("writing preferences: disk full".into())
        );
    }

    // ---- Batch 3 ----

    fn event(id: &str) -> gate_connect_core::security_feed::SecurityEvent {
        gate_connect_core::security_feed::SecurityEvent {
            id: id.into(),
            request_id: format!("req-{id}"),
            at: "2026-10-13T12:00:00Z".into(),
            action: gate_connect_core::security_feed::Action::Block,
            category: Some("credential".into()),
            tool: None,
            model: None,
            provider: None,
        }
    }

    /// A window seeding itself on mount reads the feed it is handed, as is.
    #[test]
    fn security_feed_reads_report_the_live_feed() {
        use gate_connect_core::security_feed::FeedState;
        let platform = FakePlatform {
            feed: crate::platform::fake::FakeFeed {
                state: FeedState::Reconnecting,
                history_ok: false,
                recent: vec![event("01A"), event("01B")],
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(security_feed_state(&platform), FeedState::Reconnecting);
        assert!(!security_feed_history_ok(&platform));
        let ids: Vec<_> = security_feed_recent(&platform)
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, ["01A", "01B"], "oldest first, nothing dropped");
    }

    /// Retry wakes the loop once per click and emits nothing itself: the
    /// state change it causes arrives on the feed's own events.
    #[test]
    fn security_feed_retry_wakes_the_feed_once_per_call() {
        let platform = FakePlatform::default();
        security_feed_retry(&platform);
        security_feed_retry(&platform);
        assert_eq!(*platform.feed.retries.lock().unwrap(), 2);
        assert!(platform.emitted().is_empty());
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn routing_flags_report_what_the_platform_holds() {
        for flag in [true, false] {
            let platform = FakePlatform {
                routed_clients_stale: flag,
                startup_pending: !flag,
                ..Default::default()
            };
            assert_eq!(routed_clients_stale(&platform), flag);
            assert_eq!(routing_startup_pending(&platform), !flag);
        }
    }

    /// Each installed, visible tool lands in exactly one bucket, read from
    /// its status; not installed and hidden tools are left out.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn teardown_report_buckets_each_tool_by_what_its_config_says() {
        let platform = FakePlatform {
            tools: vec![
                FakeTool {
                    status: Ok(Status::Connected),
                    ..FakeTool::new(ToolId::ClaudeCode)
                },
                FakeTool {
                    status: Ok(Status::Drifted("base URL changed".into())),
                    ..FakeTool::new(ToolId::Codex)
                },
                FakeTool {
                    status: Ok(Status::Detected),
                    ..FakeTool::new(ToolId::OpenCode)
                },
                FakeTool {
                    status: Ok(Status::Detected),
                    ..FakeTool::new(ToolId::Hermes)
                },
                FakeTool {
                    status: Err("permission denied".into()),
                    ..FakeTool::new(ToolId::EnvProxy)
                },
                FakeTool {
                    status: Ok(Status::NotInstalled),
                    ..FakeTool::new(ToolId::OpenClaw)
                },
                FakeTool {
                    status: Ok(Status::Detected),
                    hidden: true,
                    ..FakeTool::new(ToolId::ClaudeCode)
                },
            ],
            reopen_pending: ["opencode"].into_iter().collect(),
            ..Default::default()
        };
        let report = teardown_report(&platform);
        let bucket = |tools: &[TeardownToolDto]| -> Vec<(String, &'static str)> {
            tools
                .iter()
                .map(|t| (t.slug.clone(), t.next_action))
                .collect()
        };
        assert_eq!(
            bucket(&report.still_gate),
            [
                ("claude-code".to_string(), "retry_disconnect"),
                ("codex".to_string(), "retry_disconnect"),
            ]
        );
        assert_eq!(
            bucket(&report.awaiting_reopen),
            [("opencode".to_string(), "reopen_tool")]
        );
        assert_eq!(bucket(&report.defaults), [("hermes".to_string(), "none")]);
        assert_eq!(
            bucket(&report.failed),
            [("env-proxy".to_string(), "retry_check")]
        );
        assert_eq!(report.defaults[0].name, "Fake Tool", "the product name");
    }

    /// A still-Gate tool is a disconnect to retry even when its process is
    /// stale: reopening it would read the same Gate config again.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn teardown_report_asks_for_a_disconnect_before_a_reopen() {
        let platform = FakePlatform {
            tools: vec![FakeTool {
                status: Ok(Status::Connected),
                ..FakeTool::new(ToolId::Codex)
            }],
            reopen_pending: ["codex"].into_iter().collect(),
            ..Default::default()
        };
        let report = teardown_report(&platform);
        assert_eq!(report.still_gate.len(), 1);
        assert!(report.awaiting_reopen.is_empty());
    }

    #[test]
    fn pin_and_unpin_popover_set_the_pin() {
        let platform = FakePlatform::default();
        pin_popover(&platform);
        assert_eq!(*platform.popover_pinned.lock().unwrap(), Some(true));
        unpin_popover(&platform);
        assert_eq!(*platform.popover_pinned.lock().unwrap(), Some(false));
    }

    #[test]
    fn set_updater_relaunching_stores_the_flag_both_ways() {
        let platform = FakePlatform::default();
        set_updater_relaunching(&platform, true);
        assert_eq!(*platform.updater_relaunching.lock().unwrap(), Some(true));
        set_updater_relaunching(&platform, false);
        assert_eq!(*platform.updater_relaunching.lock().unwrap(), Some(false));
    }

    /// A drain takes the calling window's failures and leaves the other
    /// window's alone.
    #[test]
    fn drain_backend_errors_takes_only_the_named_window() {
        let failure = |message: &str| BackendError {
            context: "proxy_enable",
            message: message.into(),
            reason: None,
        };
        let platform = FakePlatform::default();
        platform.backend_errors.lock().unwrap().extend([
            ("main".to_string(), vec![failure("main one")]),
            ("tray".to_string(), vec![failure("tray one")]),
        ]);
        let main: Vec<_> = drain_backend_errors(&platform, "main")
            .into_iter()
            .map(|e| e.message)
            .collect();
        assert_eq!(main, ["main one"]);
        assert!(drain_backend_errors(&platform, "main").is_empty());
        assert!(drain_backend_errors(&platform, "settings").is_empty());
        assert_eq!(drain_backend_errors(&platform, "tray").len(), 1);
    }

    // ---- Batch 4 ----

    const GATEWAY: &str = "https://gate.example.com";

    fn signed_in() -> FakePlatform {
        FakePlatform {
            account_url: std::sync::Mutex::new(Some(GATEWAY.into())),
            api_key: std::sync::Mutex::new(Some("sk-old".into())),
            ..Default::default()
        }
    }

    fn failing(name: &'static str) -> FakePlatform {
        FakePlatform {
            fail: [(name, "refused".to_string())].into_iter().collect(),
            ..signed_in()
        }
    }

    fn emits(platform: &FakePlatform) -> Vec<String> {
        platform.emitted().into_iter().map(|(e, _)| e).collect()
    }

    /// 8dce3c0. The engine pins the gateway URL when it starts, so one still
    /// up after the switch would rewrite traffic to the old environment
    /// under the new one's token. Tools out first, then the engine, and only
    /// then the account moves.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn switch_gateway_stops_the_engine_before_the_new_gateway() {
        let platform = signed_in();
        assert_eq!(
            switch_gateway(&platform, "https://staging.example.com"),
            Ok(())
        );
        assert_eq!(
            platform.calls(),
            [
                "disconnect_all_managed",
                "shutdown_engine",
                "switch_gateway https://staging.example.com",
                "emit session-changed",
            ]
        );
        assert_eq!(
            *platform.api_key.lock().unwrap(),
            None,
            "the key is forgotten"
        );
    }

    /// An engine that will not stop aborts the switch: the account stays on
    /// the gateway the engine is still pinned to, and no window is told the
    /// session moved.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn switch_gateway_keeps_the_account_when_the_engine_will_not_stop() {
        let platform = failing("shutdown_engine");
        assert_eq!(
            switch_gateway(&platform, "https://staging.example.com"),
            Err("shutdown_engine: refused".into())
        );
        assert_eq!(
            platform.calls(),
            ["disconnect_all_managed", "shutdown_engine"]
        );
        assert_eq!(
            platform.account_url.lock().unwrap().as_deref(),
            Some(GATEWAY)
        );
    }

    /// A tool that would not let go aborts before the engine or the account
    /// is touched.
    #[test]
    fn switch_gateway_stops_at_a_tool_that_will_not_disconnect() {
        let platform = failing("disconnect_all_managed");
        assert_eq!(
            switch_gateway(&platform, "https://staging.example.com"),
            Err("disconnect_all_managed: refused".into())
        );
        assert_eq!(platform.calls(), ["disconnect_all_managed"]);
    }

    /// The IPC boundary refuses a URL before anything is written, with the
    /// same messages on both commands that take one.
    #[test]
    fn gateway_urls_are_checked_before_anything_moves() {
        let long = format!("https://{}.example.com", "a".repeat(2048));
        for url in ["ftp://gate.example.com", "not a url", long.as_str()] {
            let platform = signed_in();
            let switched = switch_gateway(&platform, url);
            let saved = save_account(&platform, url, Some("sk-new"));
            assert_eq!(switched, saved, "{url}");
            assert!(platform.calls().is_empty(), "{url}");
        }
        let platform = signed_in();
        assert_eq!(
            switch_gateway(&platform, "ftp://gate.example.com"),
            Err(crate::base_url_scheme_error())
        );
        assert_eq!(
            switch_gateway(&platform, &long),
            Err("base url is unexpectedly long (>2048 bytes)".into())
        );
        assert!(matches!(
            switch_gateway(&platform, "not a url"),
            Err(e) if e.starts_with("invalid base url: ")
        ));
    }

    /// A pasted key is trimmed, selects the API-key path, and replaces
    /// whatever credential a running engine holds.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn save_account_with_a_key_hands_the_engine_the_key_alone() {
        let platform = FakePlatform::default();
        assert_eq!(save_account(&platform, GATEWAY, Some("  sk-new  ")), Ok(()));
        assert_eq!(
            platform.calls(),
            [
                format!("save_account {GATEWAY} Some(\"sk-new\")"),
                "set_auth_mode ApiKey".to_string(),
                "refresh_engine_token \"\"".to_string(),
                "refresh_engine_api_key \"sk-new\"".to_string(),
                "emit session-changed".to_string(),
            ]
        );
    }

    /// No key (or a blank one) saves the gateway and leaves the auth mode and
    /// the engine alone.
    #[test]
    fn save_account_without_a_key_only_moves_the_gateway() {
        let platform = FakePlatform::default();
        assert_eq!(save_account(&platform, GATEWAY, Some("   ")), Ok(()));
        assert_eq!(
            platform.calls(),
            [
                format!("save_account {GATEWAY} None"),
                "emit session-changed".into()
            ]
        );
    }

    #[test]
    fn save_account_refuses_a_malformed_key_or_a_failed_write() {
        let platform = FakePlatform::default();
        assert_eq!(
            save_account(&platform, GATEWAY, Some("gw-123")),
            Err("API key must start with \"sk-\"".into())
        );
        assert!(platform.calls().is_empty());

        let platform = failing("set_auth_mode");
        assert_eq!(
            save_account(&platform, GATEWAY, Some("sk-new")),
            Err("set_auth_mode: refused".into())
        );
        assert!(emits(&platform).is_empty(), "nothing changed to announce");
    }

    /// Tools first, the forwarder next, the account last; then the feed and
    /// the windows are told, and the identity the core forgot is announced.
    #[test]
    fn clear_account_disconnects_before_forgetting_and_announces_after() {
        let platform = signed_in();
        assert_eq!(clear_account(&platform), Ok(()));
        assert_eq!(
            platform.calls(),
            [
                "disconnect_all_managed",
                "stop_forwarder",
                "clear_account",
                "emit session-changed",
                "emit analytics-identity-changed",
            ]
        );
        assert_eq!(*platform.feed.resets.lock().unwrap(), 1);
        assert_eq!(*platform.account_url.lock().unwrap(), None);
    }

    /// A failed sign-out keeps the account and the feed, and still tells the
    /// windows what identity is stored.
    #[test]
    fn clear_account_that_fails_still_announces_the_stored_identity() {
        let platform = failing("disconnect_all_managed");
        assert_eq!(
            clear_account(&platform),
            Err("disconnect_all_managed: refused".into())
        );
        assert_eq!(
            platform.calls(),
            ["disconnect_all_managed", "emit analytics-identity-changed"]
        );
        assert_eq!(*platform.feed.resets.lock().unwrap(), 0);
        assert!(platform.account_url.lock().unwrap().is_some());
    }

    /// The credential goes first; the deliberate flag, the cache and the
    /// engine's token follow; the identity is announced last.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn oauth_sign_out_forgets_the_session_then_announces() {
        let platform = signed_in();
        assert_eq!(oauth_sign_out(&platform), Ok(()));
        assert_eq!(
            platform.calls(),
            [
                "clear_oauth_session",
                "set_signed_out_deliberately true",
                "clear_activity_cache",
                "refresh_engine_token \"\"",
                "emit session-changed",
                "emit analytics-identity-changed",
            ]
        );
        assert_eq!(
            *platform.signed_out_deliberately.lock().unwrap(),
            Some(true)
        );
        assert!(
            platform.account_url.lock().unwrap().is_some(),
            "account.json stays"
        );
    }

    /// The preferences write and the cache clear are best effort; only the
    /// credential can fail a sign-out.
    #[test]
    fn oauth_sign_out_fails_only_on_the_credential() {
        let platform = FakePlatform {
            fail_writes: Some("disk full".into()),
            fail: [("clear_activity_cache", "busy".to_string())]
                .into_iter()
                .collect(),
            ..signed_in()
        };
        assert_eq!(oauth_sign_out(&platform), Ok(()));

        let platform = failing("clear_oauth_session");
        assert_eq!(
            oauth_sign_out(&platform),
            Err("clear_oauth_session: refused".into())
        );
        assert_eq!(
            platform.calls(),
            ["clear_oauth_session", "emit analytics-identity-changed"]
        );
    }

    #[test]
    fn get_account_reads_every_field_or_nothing() {
        assert!(matches!(get_account(&FakePlatform::default()), Ok(None)));

        let platform = FakePlatform {
            org: std::sync::Mutex::new(Some(("org-1".into(), "Acme".into()))),
            billing_mode: std::sync::Mutex::new(BillingMode::Payg),
            ..signed_in()
        };
        let account = get_account(&platform).unwrap().expect("an account");
        assert_eq!(account.gateway_base_url, GATEWAY);
        assert!(account.has_api_key);
        assert_eq!(account.auth_mode, AuthMode::ApiKey);
        assert_eq!(account.billing_mode, BillingMode::Payg);
        assert_eq!(account.org_id.as_deref(), Some("org-1"));
        assert_eq!(account.org_name.as_deref(), Some("Acme"));

        assert!(matches!(
            get_account(&failing("read_account")),
            Err(e) if e == "read_account: refused"
        ));
    }

    /// A reconcile that fails is reported once per run, and never stops the
    /// read: a signed-in user must not be flipped to first-run.
    #[test]
    fn get_account_reports_a_failing_reconcile_once_and_reads_on() {
        let platform = failing("reconcile_account");
        assert!(get_account(&platform).unwrap().is_some());
        assert!(get_account(&platform).unwrap().is_some());
        let reports: Vec<_> = platform
            .calls()
            .into_iter()
            .filter(|c| c.starts_with("report_backend_error"))
            .collect();
        assert_eq!(
            reports,
            ["report_backend_error account_reconcile: reconcile_account: refused"]
        );
    }

    #[test]
    fn get_account_key_prefix_reads_the_recorded_prefix() {
        let platform = FakePlatform {
            key_prefix: Some("sk-gw-21".into()),
            ..Default::default()
        };
        assert_eq!(
            get_account_key_prefix(&platform),
            Ok(Some("sk-gw-21".into()))
        );
        assert_eq!(
            get_account_key_prefix(&failing("read_account")),
            Err("read_account: refused".into())
        );
    }

    /// Live, refused and unreachable are three answers; the last two are both
    /// signed out, and only `session` tells them apart.
    #[test]
    fn oauth_status_tells_a_sign_out_from_an_offline_machine() {
        let live = FakePlatform {
            session: Some(gate_connect_core::oauth::OAuthTokens {
                access_token: "a".into(),
                refresh_token: "r".into(),
                id_token: None,
                expires_at_unix: 1_900_000_000,
                client_id: "c".into(),
            }),
            ..Default::default()
        };
        let status = oauth_status(&live).unwrap();
        assert!(status.signed_in);
        assert_eq!(status.session, "live");
        assert_eq!(status.expires_at_unix, 1_900_000_000);

        let status = oauth_status(&FakePlatform::default()).unwrap();
        assert!(!status.signed_in);
        assert_eq!(status.session, "signed_out");
        assert_eq!(status.expires_at_unix, 0);

        let offline = FakePlatform {
            session_unavailable: true,
            ..Default::default()
        };
        let status = oauth_status(&offline).unwrap();
        assert!(!status.signed_in);
        assert_eq!(status.session, "unavailable");
    }

    #[test]
    fn oauth_cancel_login_cancels_the_waiting_login() {
        let platform = FakePlatform::default();
        oauth_cancel_login(&platform);
        assert_eq!(platform.calls(), ["cancel_login"]);
    }

    /// Only a refusal the gateway recorded raises the dead-session signal; an
    /// offline failure does not cry wolf.
    #[test]
    fn oauth_list_orgs_signals_a_dead_session_only_when_refused() {
        let platform = FakePlatform {
            session_rejected: true,
            ..failing("list_orgs")
        };
        assert!(matches!(
            oauth_list_orgs(&platform),
            Err(e) if e == "list_orgs: refused"
        ));
        assert_eq!(platform.calls(), ["signal_session_dead"]);

        let offline = failing("list_orgs");
        assert!(oauth_list_orgs(&offline).is_err());
        assert!(offline.calls().is_empty());

        let refused_but_listed = FakePlatform {
            session_rejected: true,
            ..Default::default()
        };
        assert_eq!(oauth_list_orgs(&refused_but_listed).unwrap().len(), 0);
        assert!(refused_but_listed.calls().is_empty());
    }

    /// Choosing OAuth takes any key out of a running engine; choosing the key
    /// path leaves the engine alone. Both tell the windows.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn set_auth_mode_takes_the_key_out_of_the_engine_for_oauth() {
        let platform = FakePlatform::default();
        assert_eq!(set_auth_mode(&platform, true), Ok(()));
        assert_eq!(
            platform.calls(),
            [
                "set_auth_mode OAuth",
                "refresh_engine_api_key \"\"",
                "emit session-changed",
            ]
        );
        let platform = FakePlatform::default();
        assert_eq!(set_auth_mode(&platform, false), Ok(()));
        assert_eq!(
            platform.calls(),
            ["set_auth_mode ApiKey", "emit session-changed"]
        );
        let platform = failing("set_auth_mode");
        assert_eq!(
            set_auth_mode(&platform, true),
            Err("set_auth_mode: refused".into())
        );
        assert_eq!(platform.calls(), ["set_auth_mode OAuth"]);
    }

    /// A connected Codex is re-applied under the new mode, pointed at the
    /// engine; any other tool, or a Codex Gate does not manage, is left alone.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn set_billing_mode_reapplies_a_connected_codex() {
        let platform = FakePlatform {
            tools: vec![
                FakeTool {
                    status: Ok(Status::Connected),
                    ..FakeTool::new(ToolId::ClaudeCode)
                },
                FakeTool {
                    status: Ok(Status::Connected),
                    ..FakeTool::new(ToolId::Codex)
                },
            ],
            relay_url: Some("http://127.0.0.1:47001".into()),
            tool_proxy: Some("http://127.0.0.1:47100".into()),
            engine_proxy: Some("http://127.0.0.1:47002".into()),
            ..signed_in()
        };
        assert_eq!(set_billing_mode(&platform, true), Ok(()));
        assert_eq!(
            platform.calls(),
            [
                "set_billing_mode Payg".to_string(),
                "refresh_engine_mode".to_string(),
                format!(
                    "connect codex {GATEWAY} Payg relay=Some(\"http://127.0.0.1:47001\") \
                     proxy=Some(\"http://127.0.0.1:47002\")"
                ),
            ]
        );
        assert!(emits(&platform).is_empty());

        let detected = FakePlatform {
            tools: vec![FakeTool::new(ToolId::Codex)],
            ..signed_in()
        };
        assert_eq!(set_billing_mode(&detected, false), Ok(()));
        assert_eq!(
            detected.calls(),
            ["set_billing_mode Byok", "refresh_engine_mode"]
        );
    }

    /// The re-apply is best effort: the mode is already stored.
    #[test]
    fn set_billing_mode_survives_a_codex_that_will_not_rewrite() {
        let platform = FakePlatform {
            tools: vec![FakeTool {
                status: Ok(Status::Connected),
                write_fails: Some("read-only".into()),
                ..FakeTool::new(ToolId::Codex)
            }],
            ..signed_in()
        };
        assert_eq!(set_billing_mode(&platform, true), Ok(()));
        assert_eq!(*platform.billing_mode.lock().unwrap(), BillingMode::Payg);
        assert_eq!(
            set_billing_mode(&failing("set_billing_mode"), true),
            Err("set_billing_mode: refused".into())
        );
    }

    /// The new org reaches the engine, the feed drops the old org's events,
    /// and the windows are told; a refused write does none of that.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn set_org_moves_the_engine_and_clears_the_previous_org() {
        let platform = signed_in();
        assert_eq!(set_org(&platform, "org-2", "Beta"), Ok(()));
        assert_eq!(
            platform.calls(),
            [
                "set_org org-2 Beta",
                "refresh_engine_org org-2",
                "emit session-changed",
            ]
        );
        assert_eq!(*platform.feed.resets.lock().unwrap(), 1);

        let platform = failing("set_org");
        assert_eq!(
            set_org(&platform, "org-2", "Beta"),
            Err("set_org: refused".into())
        );
        assert_eq!(platform.calls(), ["set_org org-2 Beta"]);
        assert_eq!(*platform.feed.resets.lock().unwrap(), 0);
    }

    fn identity(sub: &str) -> Identity {
        Identity {
            identified_sub: Some(sub.into()),
            ever_identified: true,
            ..Default::default()
        }
    }

    /// Finishing a forget tells the other windows; a plain read does not, and
    /// a forget that fails reads what is stored.
    #[test]
    fn analytics_identity_announces_only_a_forget_it_finished() {
        let platform = FakePlatform {
            identity: std::sync::Mutex::new(identity("sub-1")),
            ..Default::default()
        };
        assert_eq!(analytics_identity(&platform), identity("sub-1"));
        assert!(platform.emitted().is_empty());

        let platform = FakePlatform {
            identity: std::sync::Mutex::new(identity("sub-1")),
            forget_pending: true,
            ..Default::default()
        };
        assert_eq!(analytics_identity(&platform), Identity::default());
        assert_eq!(
            platform.emitted(),
            [(
                "analytics-identity-changed".to_string(),
                serde_json::to_value(Identity::default()).unwrap()
            )]
        );

        let platform = FakePlatform {
            identity: std::sync::Mutex::new(identity("sub-1")),
            forget_pending: true,
            ..failing("forget_identity_if_signed_out")
        };
        assert_eq!(analytics_identity(&platform), identity("sub-1"));
        assert!(platform.emitted().is_empty());
    }

    /// Saved and not-live both announce what is stored; not-live and
    /// unconfirmed reject with the names the webview reads; unconfirmed tells
    /// nobody anything.
    #[test]
    fn set_analytics_identity_announces_what_was_stored() {
        let platform = FakePlatform::default();
        assert_eq!(set_analytics_identity(&platform, identity("sub-2")), Ok(()));
        assert_eq!(
            platform.emitted(),
            [(
                "analytics-identity-changed".to_string(),
                serde_json::to_value(identity("sub-2")).unwrap()
            )]
        );

        let platform = FakePlatform {
            identity: std::sync::Mutex::new(identity("sub-1")),
            save_outcome: Some(SaveOutcome::NotLive),
            ..Default::default()
        };
        assert_eq!(
            set_analytics_identity(&platform, identity("sub-2")),
            Err("analytics-identity-not-live".into())
        );
        assert_eq!(
            platform.emitted(),
            [(
                "analytics-identity-changed".to_string(),
                serde_json::to_value(identity("sub-1")).unwrap()
            )]
        );

        let platform = FakePlatform {
            save_outcome: Some(SaveOutcome::Unconfirmed),
            ..Default::default()
        };
        assert_eq!(
            set_analytics_identity(&platform, identity("sub-2")),
            Err("analytics-identity-unconfirmed".into())
        );
        assert!(platform.emitted().is_empty());

        assert_eq!(
            set_analytics_identity(&failing("save_identity"), identity("sub-2")),
            Err("save_identity: refused".into())
        );
    }

    fn offline() -> FakePlatform {
        FakePlatform {
            gateway_failure: Some(gate_connect_core::activity::Failure {
                code: gate_connect_core::activity::FailureCode::Offline,
                message: "no route \"to\" host".into(),
            }),
            ..Default::default()
        }
    }

    /// Client names are checked before the read, and a gateway failure crosses
    /// as the JSON envelope the pane picks its action from.
    #[test]
    fn activity_reads_check_their_clients_and_envelope_failures() {
        let platform = FakePlatform::default();
        assert_eq!(
            activity_overview(&platform, Some("inst"), Some(vec!["codex".into()])),
            Ok("overview Some(\"inst\") [\"codex\"]".into())
        );
        assert_eq!(
            activity_overview(&platform, None, None),
            Ok("overview None []".into())
        );
        assert_eq!(
            activity_overview(&platform, None, Some(vec!["nope".into()])),
            Err("unknown client \"nope\"".into())
        );
        assert_eq!(
            activity_cached_overview(&platform, None, Some(vec!["nope".into()])),
            Err("unknown client \"nope\"".into())
        );
        assert_eq!(
            activity_cached_overview(&platform, Some("inst"), None),
            Ok(Some("cached overview Some(\"inst\") []".into()))
        );
        assert_eq!(
            activity_cached_tool_overviews(&platform, Some("inst")),
            BTreeMap::from([("codex".to_string(), "cached Some(\"inst\")".to_string())])
        );
        assert_eq!(
            activity_tool_events(&platform, None, &[]),
            Err("at least one client name is required to read the feed".into())
        );

        let envelope = r#"{"code":"offline","message":"no route \"to\" host"}"#.to_string();
        let platform = offline();
        assert_eq!(
            activity_overview(&platform, None, None),
            Err(envelope.clone())
        );
        assert_eq!(activity_installations(&platform), Err(envelope.clone()));
        assert_eq!(gate_model_catalogue(&platform), Err(envelope.clone()));
        assert_eq!(gate_credits(&platform), Err(envelope));
        assert_eq!(gate_credits(&FakePlatform::default()), Ok("credits".into()));
        assert_eq!(
            activity_installations(&FakePlatform::default()),
            Ok("installations".into())
        );
    }

    /// The stored choices and what each config says, read configs first.
    #[test]
    fn tool_model_preferences_reads_the_configs_then_the_choices() {
        use gate_connect_core::preferences::{ModelSource, ToolModelChoice};
        use gate_connect_core::registry::GateModelState;
        let mut prefs = gate_connect_core::preferences::Preferences::default();
        prefs.tool_models.insert(
            "codex".into(),
            ToolModelChoice {
                source: ModelSource::Gate,
                model_ids: vec!["gate/one".into()],
            },
        );
        prefs.gate_model_paid_ack_unix = Some(1_700_000_000);
        let platform = FakePlatform {
            prefs,
            model_states: BTreeMap::from([(
                "codex",
                gate_connect_core::tool_models::ToolModelView {
                    supported: true,
                    state: GateModelState::Applied {
                        model: "gate/one".into(),
                    },
                    left_gate_models: None,
                    problem: None,
                },
            )]),
            ..Default::default()
        };
        let dto = tool_model_preferences(&platform);
        assert_eq!(dto.tools["codex"].source, "gate");
        assert_eq!(dto.tools["codex"].model_ids, ["gate/one"]);
        assert_eq!(dto.paid_ack_unix, Some(1_700_000_000));
        assert_eq!(dto.configured["codex"].state, "applied");
        assert_eq!(dto.configured["codex"].model.as_deref(), Some("gate/one"));
        assert_eq!(platform.calls(), ["tool_model_states"]);
    }

    /// A Gate choice carries the catalogue's labels for the chosen models; a
    /// Codex whose config changed gets its daemon refreshed once idle.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn set_tool_model_labels_a_gate_choice_and_refreshes_codex() {
        let platform = FakePlatform {
            model_applied: true,
            ..Default::default()
        };
        assert_eq!(
            set_tool_model(
                &platform,
                "codex".into(),
                "gate",
                vec!["gate/one".into(), "gate/two".into()],
                true
            ),
            Ok(true)
        );
        assert_eq!(
            platform.calls(),
            [
                "gate_model_catalogue",
                "choose_tool_model codex Gate [\"gate/one\", \"gate/two\"] ack=true \
                 meta=[\"gate/one\"]",
                "refresh_codex_daemon_when_idle set model",
            ]
        );

        // The tool's own model needs no catalogue, and a config that did not
        // change needs no restart.
        let platform = FakePlatform::default();
        assert_eq!(
            set_tool_model(&platform, "codex".into(), "tool", Vec::new(), false),
            Ok(false)
        );
        assert_eq!(
            platform.calls(),
            ["choose_tool_model codex Tool [] ack=false meta=[]"]
        );
    }

    /// An unreadable catalogue costs the labels, not the choice.
    #[test]
    fn set_tool_model_chooses_without_a_catalogue() {
        let platform = offline();
        assert_eq!(
            set_tool_model(
                &platform,
                "opencode".into(),
                "gate",
                vec!["gate/one".into()],
                true
            ),
            Ok(false)
        );
        assert!(platform.calls().contains(
            &"choose_tool_model opencode Gate [\"gate/one\"] ack=true meta=[]".to_string()
        ));
    }

    #[test]
    fn set_tool_model_refuses_what_it_cannot_store() {
        let platform = FakePlatform::default();
        assert_eq!(
            set_tool_model(&platform, String::new(), "gate", Vec::new(), false),
            Err("a tool slug is required to set a model preference".into())
        );
        assert_eq!(
            set_tool_model(&platform, "nope".into(), "gate", Vec::new(), false),
            Err("unknown tool slug \"nope\"".into())
        );
        assert_eq!(
            set_tool_model(&platform, "codex".into(), "both", Vec::new(), false),
            Err("unknown model source \"both\"".into())
        );
        assert!(platform.calls().is_empty());
        assert_eq!(
            set_tool_model(
                &failing("choose_tool_model"),
                "codex".into(),
                "tool",
                Vec::new(),
                false
            ),
            Err("choose_tool_model: refused".into())
        );
    }

    /// The routing intent is stored before the engine starts, and the tool is
    /// pointed at the relay and the tool proxy.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn connect_tool_starts_the_engine_before_the_config_points_at_it() {
        let platform = FakePlatform {
            tools: vec![FakeTool {
                status: Ok(Status::Connected),
                ..FakeTool::new(ToolId::Codex)
            }],
            relay_url: Some("http://127.0.0.1:47001".into()),
            tool_proxy: Some("http://127.0.0.1:47100".into()),
            engine_proxy: Some("http://127.0.0.1:47002".into()),
            ..signed_in()
        };
        assert!(matches!(
            connect_tool(&platform, "codex"),
            Ok(StatusDto::Connected)
        ));
        assert_eq!(
            platform.calls(),
            [
                "set_routing_intent true".to_string(),
                "enable_engine".to_string(),
                format!(
                    "connect codex {GATEWAY} Byok relay=Some(\"http://127.0.0.1:47001\") \
                     proxy=Some(\"http://127.0.0.1:47100\")"
                ),
            ]
        );
    }

    /// An intent that will not persist is reported and the connect goes on;
    /// an engine that will not start stops it before the config is written.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn connect_tool_needs_the_engine_but_not_the_intent() {
        let platform = FakePlatform {
            tools: vec![FakeTool::new(ToolId::Codex)],
            ..failing("set_routing_intent")
        };
        assert!(connect_tool(&platform, "codex").is_ok());
        assert_eq!(
            platform.calls()[..3],
            [
                "set_routing_intent true",
                "report_backend_error routing_intent: set_routing_intent: refused",
                "enable_engine",
            ]
        );

        let platform = FakePlatform {
            tools: vec![FakeTool::new(ToolId::Codex)],
            ..failing("enable_engine")
        };
        assert!(matches!(
            connect_tool(&platform, "codex"),
            Err(e) if e == "enable_engine: refused"
        ));
        assert_eq!(
            platform.calls(),
            ["set_routing_intent true", "enable_engine"]
        );
    }

    #[test]
    fn connect_tool_needs_an_account_and_a_known_tool() {
        let platform = FakePlatform {
            tools: vec![FakeTool::new(ToolId::Codex)],
            ..Default::default()
        };
        assert!(matches!(
            connect_tool(&platform, "codex"),
            Err(e) if e == "Sign in to Gate AI first"
        ));
        assert!(matches!(
            connect_tool(&platform, "nope"),
            Err(e) if e == "unknown tool \"nope\""
        ));
        assert!(platform.calls().is_empty());
    }

    #[test]
    fn disconnect_tool_reports_the_status_or_the_whole_error() {
        let platform = FakePlatform {
            tools: vec![FakeTool::new(ToolId::OpenCode)],
            ..Default::default()
        };
        assert!(matches!(
            disconnect_tool(&platform, "opencode"),
            Ok(StatusDto::Detected)
        ));
        assert_eq!(platform.calls(), ["disconnect opencode"]);

        let platform = FakePlatform {
            tools: vec![FakeTool {
                write_fails: Some("read-only".into()),
                ..FakeTool::new(ToolId::OpenCode)
            }],
            ..Default::default()
        };
        assert!(matches!(
            disconnect_tool(&platform, "opencode"),
            Err(e) if e == "disconnect opencode: read-only"
        ));
        assert!(matches!(
            disconnect_tool(&platform, "codex"),
            Err(e) if e == "integration missing from registry"
        ));
    }

    // ---- Batch 5 ----

    /// A fake whose routing on and off each report one hiccup.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn hiccuping() -> FakePlatform {
        FakePlatform {
            routing_warnings: vec![("provider_restore", "anthropic stayed off".into())],
            ..signed_in()
        }
    }

    /// The text of one `#[tauri::command]` adapter in lib.rs, from its `fn`
    /// line to the closing brace.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn lib_body(signature: &str) -> String {
        let src = include_str!("lib.rs").replace("\r\n", "\n");
        let at = src
            .find(signature)
            .unwrap_or_else(|| panic!("{signature} not in lib.rs"));
        let end = src[at..].find("\n}\n").expect("closing brace");
        src[at..at + end].to_string()
    }

    /// bedf13a, the adapter half. `proxy_status` is polled, and on Windows the
    /// read shells out to `certutil`; run on the thread driving the event loop,
    /// a hung `certutil` froze the window and left the app unquittable. The
    /// body is one read through the platform, and the adapter runs it on the
    /// blocking pool rather than as a sync command.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn proxy_status_runs_the_status_read_through_the_blocking_seam() {
        let platform = signed_in();
        assert!(matches!(proxy_status(&platform), Ok(s) if !s.running));
        assert_eq!(platform.calls(), ["proxy_status"]);

        let platform = failing("proxy_status");
        assert_eq!(
            proxy_status(&platform).map(|s| s.running),
            Err("proxy_status: refused".into())
        );

        let adapter = lib_body("fn proxy_status()");
        assert!(
            adapter.starts_with("fn proxy_status()") && {
                let src = include_str!("lib.rs").replace("\r\n", "\n");
                src.contains("async fn proxy_status()")
            },
            "proxy_status must be an async command, not a sync one on the event loop"
        );
        let blocking = adapter
            .find("spawn_blocking(")
            .expect("proxy_status must hand its read to the blocking pool");
        let read = adapter
            .find("commands::proxy_status(&TauriPlatform)")
            .expect("proxy_status must read through commands::proxy_status");
        assert!(
            blocking < read,
            "the read runs inside spawn_blocking:\n{adapter}"
        );
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn proxy_browser_store_probes_the_stores_once() {
        let platform = FakePlatform {
            browser_store: Some(gate_connect_core::proxy::NssTrust::ToolsMissing),
            ..Default::default()
        };
        assert_eq!(
            proxy_browser_store(&platform),
            Some(gate_connect_core::proxy::NssTrust::ToolsMissing)
        );
        assert_eq!(platform.calls(), ["probe_browser_store"]);
    }

    /// 7fc3d16, superseded since: routing now comes on with every launch, so
    /// both the startup enable and this command can be the first thing to
    /// route on a machine with no login item. A crash would then strand the
    /// system proxy at a dead port with nothing relaunching at boot, so routing
    /// on arms the safety net, after the tray shows the settled state.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn proxy_enable_arms_the_crash_safety_net() {
        let platform = hiccuping();
        assert!(matches!(proxy_enable(&platform), Ok(s) if s.running));
        assert_eq!(
            platform.calls(),
            [
                "enable_routing",
                "refresh_codex_daemon_when_idle routing on",
                "report_backend_failure provider_restore: anthropic stayed off",
                "proxy_status",
                "update_tray_status true",
                "arm_crash_safety_net",
            ]
        );
    }

    /// Linux has no safety net: its engine is a detached daemon that falls back
    /// to pass-through when the GUI dies, and it has no exit-time safe point.
    #[cfg(target_os = "linux")]
    #[test]
    fn proxy_enable_on_linux_shows_the_state_and_arms_nothing() {
        let platform = hiccuping();
        assert!(matches!(proxy_enable(&platform), Ok(s) if s.running));
        assert_eq!(
            platform.calls(),
            [
                "enable_routing",
                "refresh_codex_daemon_when_idle routing on",
                "report_backend_failure provider_restore: anthropic stayed off",
                "proxy_status",
                "update_tray_status true",
            ]
        );
    }

    /// Routing that will not start, or a state that cannot be read back,
    /// stops before the tray or the net is touched.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn proxy_enable_stops_before_the_tray_when_routing_will_not_start() {
        let platform = failing("enable_routing");
        assert_eq!(
            proxy_enable(&platform).map(|s| s.running),
            Err("enable_routing: refused".into())
        );
        assert_eq!(platform.calls(), ["enable_routing"]);

        let platform = failing("proxy_status");
        assert_eq!(
            proxy_enable(&platform).map(|s| s.running),
            Err("proxy_status: refused".into())
        );
        assert_eq!(
            platform.calls(),
            [
                "enable_routing",
                "refresh_codex_daemon_when_idle routing on",
                "proxy_status"
            ]
        );
    }

    /// Off shows the state, then finishes a launch-at-login opt-out that was
    /// waiting for routing to end. A failed off touches neither.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn proxy_disable_finishes_a_deferred_opt_out_after_the_tray() {
        let platform = hiccuping();
        *platform.proxy_running.lock().unwrap() = true;
        assert!(matches!(proxy_disable(&platform), Ok(s) if !s.running));
        assert_eq!(
            platform.calls(),
            [
                "disable_routing",
                "report_backend_failure provider_restore: anthropic stayed off",
                "update_tray_status false",
                "complete_pending_autostart_disable",
            ]
        );

        let platform = failing("disable_routing");
        assert_eq!(
            proxy_disable(&platform).map(|s| s.running),
            Err("disable_routing: refused".into())
        );
        assert_eq!(platform.calls(), ["disable_routing"]);
    }

    /// One audit entry per operator toggle, and only with a gateway to file it
    /// under.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn proxy_set_domain_audits_the_toggle_under_the_stored_gateway() {
        let platform = signed_in();
        assert!(proxy_set_domain(&platform, "anthropic", true).is_ok());
        assert_eq!(
            platform.calls(),
            [
                "set_domain anthropic true".to_string(),
                format!("audit_domain_toggled {GATEWAY} anthropic true"),
            ]
        );

        let platform = FakePlatform::default();
        assert!(proxy_set_domain(&platform, "anthropic", false).is_ok());
        assert_eq!(platform.calls(), ["set_domain anthropic false"]);

        let platform = failing("set_domain");
        assert_eq!(
            proxy_set_domain(&platform, "anthropic", true).map(|s| s.running),
            Err("set_domain: refused".into())
        );
        assert_eq!(platform.calls(), ["set_domain anthropic true"]);
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn proxy_set_env_export_reads_the_state_back_after_the_change() {
        let platform = signed_in();
        assert!(proxy_set_env_export(&platform, true).is_ok());
        assert_eq!(platform.calls(), ["set_env_export true", "proxy_status"]);

        let platform = failing("set_env_export");
        assert_eq!(
            proxy_set_env_export(&platform, false).map(|s| s.running),
            Err("set_env_export: refused".into())
        );
        assert_eq!(platform.calls(), ["set_env_export false"]);
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn ca_trust_changes_report_the_whole_error_chain() {
        let platform = signed_in();
        assert!(proxy_trust_ca(&platform).is_ok());
        assert!(proxy_untrust_ca(&platform).is_ok());
        assert_eq!(platform.calls(), ["trust_ca", "untrust_ca"]);

        assert_eq!(
            proxy_trust_ca(&failing("trust_ca")).map(|s| s.running),
            Err("trust_ca: refused".into())
        );
        assert_eq!(
            proxy_untrust_ca(&failing("untrust_ca")).map(|s| s.running),
            Err("untrust_ca: refused".into())
        );
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn hermes_upstream_coverage_is_the_platform_reading() {
        let platform = FakePlatform::default();
        let coverage = hermes_upstream_coverage(&platform);
        assert!(coverage.defaulted);
        assert_eq!(coverage.unknown, ["llm.example.com"]);
        assert_eq!(platform.calls(), ["hermes_upstream_coverage"]);
    }

    /// Both probes once for the whole sweep, a row per visible tool, and the
    /// sweep recorded once, after it.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn routing_verdicts_probe_once_and_record_the_whole_sweep() {
        use gate_connect_core::routing_health::{RouteHealth, SessionHealth};
        let platform = FakePlatform {
            tools: vec![
                FakeTool {
                    status: Ok(Status::Connected),
                    ..FakeTool::new(ToolId::Codex)
                },
                FakeTool {
                    status: Ok(Status::NotInstalled),
                    ..FakeTool::new(ToolId::OpenCode)
                },
                FakeTool {
                    hidden: true,
                    ..FakeTool::new(ToolId::OpenClaw)
                },
            ],
            route: Some(RouteHealth::Reachable),
            session_health: Some(SessionHealth::Valid),
            ..signed_in()
        };
        let verdicts = routing_verdicts(&platform);
        let rows: Vec<_> = verdicts
            .iter()
            .map(|v| (v.slug.as_str(), v.state, v.reason))
            .collect();
        assert_eq!(
            rows,
            [("codex", "on", None), ("opencode", "not_installed", None)]
        );
        assert_eq!(
            platform.calls(),
            [
                "probe_relay_route",
                "probe_session_health",
                "record_verdict_sweep codex=on opencode=not_installed",
            ]
        );
    }

    /// A dead relay is a reason on the row, with its next action.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn routing_verdicts_name_why_a_connected_tool_is_not_routing() {
        use gate_connect_core::routing_health::{RouteHealth, SessionHealth};
        let platform = FakePlatform {
            tools: vec![FakeTool {
                status: Ok(Status::Connected),
                ..FakeTool::new(ToolId::Codex)
            }],
            route: Some(RouteHealth::Unreachable),
            session_health: Some(SessionHealth::Valid),
            ..signed_in()
        };
        let verdicts = routing_verdicts(&platform);
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].state, "needs_attention");
        assert!(verdicts[0].reason.is_some());
        assert!(verdicts[0].next_action.is_some());
    }

    /// A pending opt-out reads as off while the login item is still there.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn launch_at_login_status_reads_a_deferred_opt_out_as_off() {
        let platform = FakePlatform::default();
        *platform.login_item.lock().unwrap() = true;
        assert_eq!(
            launch_at_login_status(&platform),
            Ok(LaunchAtLoginStatus {
                enabled: true,
                pending_disable: false
            })
        );
        *platform.opt_out_pending.lock().unwrap() = true;
        assert_eq!(
            launch_at_login_status(&platform),
            Ok(LaunchAtLoginStatus {
                enabled: false,
                pending_disable: true
            })
        );
        assert_eq!(
            launch_at_login_status(&failing("login_item_registered")),
            Err("login_item_registered: refused".into())
        );
    }

    /// On clears a pending opt-out before registering, so the new choice is
    /// never reported as pending, and puts the crash-restart policy back.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn set_launch_at_login_on_clears_the_marker_before_registering() {
        let platform = FakePlatform::default();
        *platform.opt_out_pending.lock().unwrap() = true;
        assert_eq!(set_launch_at_login(&platform, true), Ok(()));
        let mut expected = vec!["set_autostart_opt_out_pending false", "register_login_item"];
        if cfg!(target_os = "macos") {
            expected.push("arm_crash_restart");
        }
        assert_eq!(platform.calls(), expected);
        assert!(*platform.login_item.lock().unwrap());

        let platform = failing("set_autostart_opt_out_pending");
        assert_eq!(
            set_launch_at_login(&platform, true),
            Err("set_autostart_opt_out_pending: refused".into())
        );
        assert_eq!(platform.calls(), ["set_autostart_opt_out_pending false"]);
    }

    /// Off removes the item now with routing off, and waits for a safe point
    /// with routing on.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn set_launch_at_login_off_waits_while_routing_is_on() {
        let platform = FakePlatform::default();
        *platform.login_item.lock().unwrap() = true;
        assert_eq!(set_launch_at_login(&platform, false), Ok(()));
        assert_eq!(
            platform.calls(),
            ["record_autostart_opt_out", "deregister_login_item"]
        );
        assert!(!*platform.login_item.lock().unwrap());

        let platform = FakePlatform::default();
        *platform.login_item.lock().unwrap() = true;
        *platform.proxy_running.lock().unwrap() = true;
        assert_eq!(set_launch_at_login(&platform, false), Ok(()));
        assert_eq!(platform.calls(), ["record_autostart_opt_out"]);
        assert!(*platform.login_item.lock().unwrap());
        assert!(*platform.opt_out_pending.lock().unwrap());

        let platform = failing("deregister_login_item");
        assert_eq!(
            set_launch_at_login(&platform, false),
            Err("deregister_login_item: refused".into())
        );
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn agent(
        slug: &str,
        name: &str,
        pid: u32,
        started: u64,
        can_reopen: bool,
    ) -> crate::RunningAgent {
        crate::RunningAgent {
            slug: slug.into(),
            name: name.into(),
            product_name: name.into(),
            verifiable: ToolId::from_slug(slug).is_some(),
            can_reopen,
            pid,
            started_at_unix: started,
            needs_reopen: true,
        }
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn closed(slug: Option<&'static str>, pid: u32, relaunch: Option<&str>) -> crate::CloseTarget {
        crate::CloseTarget {
            pid: sysinfo::Pid::from_u32(pid),
            started: 0,
            slug,
            name: slug.unwrap_or("?").into(),
            surface: Some(if relaunch.is_some() {
                crate::Surface::App
            } else {
                crate::Surface::Cli
            }),
            relaunch: relaunch.map(|path| Relaunch::Path(path.into())),
        }
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn app(path: &str) -> Relaunch {
        Relaunch::Path(path.into())
    }

    /// f2f8c6c, the Rust half. The routing-change takeover is skipped in the
    /// webview when `running_agents` comes back empty, so with nothing running
    /// the report must say exactly that: no rows, and the names it looked for,
    /// so "none of these" is not read as "no AI tools at all".
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn running_agents_reports_none_when_nothing_is_running() {
        let platform = FakePlatform::default();
        let report = running_agents(&platform, None);
        assert!(
            report.agents.is_empty(),
            "nothing running, nothing reported"
        );
        assert_eq!(
            report.scanned_names,
            ["claude", "codex", "opencode", "hermes", "Claude", "ChatGPT"]
        );
        assert_eq!(
            platform.calls(),
            [r#"running_agents ["claude", "codex", "opencode", "hermes", "Claude", "ChatGPT"]"#]
        );

        let report = running_agents(&platform, Some(&["codex".to_string()]));
        assert!(report.agents.is_empty());
        assert_eq!(report.scanned_names, ["codex"]);
    }

    /// Only the named tools are asked about, and a tool with no process of its
    /// own narrows the walk to nothing rather than widening it to everything.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn running_agents_asks_only_about_the_named_tools() {
        let platform = FakePlatform {
            agents: vec![
                agent("codex", "codex", 7, 100, false),
                agent("claude-code", "claude", 8, 100, false),
            ],
            ..FakePlatform::default()
        };
        let report = running_agents(&platform, Some(&["codex".to_string()]));
        assert_eq!(report.agents.len(), 1);
        assert_eq!(report.agents[0].slug, "codex");
        assert!(report.agents[0].verifiable);

        let report = running_agents(&platform, Some(&["openclaw".to_string()]));
        assert!(report.agents.is_empty());
        assert!(report.scanned_names.is_empty());
        assert_eq!(platform.calls()[1], "running_agents []");
    }

    /// Oldest first; on a tie the relaunchable row wins, then the lower pid.
    /// The ChatGPT app and its bundled `codex` share a slug and can share a
    /// start second, and the webview keeps the first row per slug.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn running_agents_puts_the_relaunchable_row_first_on_a_tie() {
        let platform = FakePlatform {
            agents: vec![
                agent("chatgpt", "ChatGPT", 40, 200, false),
                agent("chatgpt", "ChatGPT", 50, 200, true),
                agent("codex", "codex", 30, 200, false),
                agent("claude-code", "claude", 90, 100, false),
            ],
            ..FakePlatform::default()
        };
        let pids: Vec<u32> = running_agents(&platform, None)
            .agents
            .iter()
            .map(|a| a.pid)
            .collect();
        assert_eq!(pids, [90, 50, 30, 40]);
    }

    /// Each closed app is queued once, a second close replaces its entry rather
    /// than adding one, and a CLI is never queued. Codex's daemon refresh is
    /// owed only when the close could have touched Codex.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn close_running_agents_queues_each_app_once() {
        let platform = FakePlatform {
            closes: std::sync::Mutex::new(vec![
                closed(Some("chatgpt"), 1, Some("/Applications/ChatGPT.app")),
                closed(Some("chatgpt"), 2, Some("/Applications/ChatGPT.app")),
                closed(Some("codex"), 3, None),
                closed(None, 4, Some("/somewhere/else")),
            ]),
            pending_reopen: std::sync::Mutex::new(vec![
                ("chatgpt".into(), app("/old/ChatGPT.app")),
                ("anthropic".into(), app("/Applications/Claude.app")),
            ]),
            ..FakePlatform::default()
        };
        assert_eq!(close_running_agents(&platform, None), 4);
        assert_eq!(
            platform.calls(),
            [
                "close_agents None",
                "refresh_codex_daemon_when_idle close agents"
            ]
        );
        assert_eq!(
            *platform.pending_reopen.lock().unwrap(),
            [
                ("anthropic".to_string(), app("/Applications/Claude.app")),
                ("chatgpt".to_string(), app("/Applications/ChatGPT.app")),
            ]
        );

        let only = ["anthropic".to_string()];
        let platform = FakePlatform {
            closes: std::sync::Mutex::new(vec![closed(
                Some("anthropic"),
                5,
                Some("/Applications/Claude.app"),
            )]),
            ..FakePlatform::default()
        };
        assert_eq!(close_running_agents(&platform, Some(&only)), 1);
        assert_eq!(platform.calls(), [r#"close_agents Some(["anthropic"])"#]);

        let platform = FakePlatform::default();
        assert_eq!(
            close_running_agents(&platform, Some(&["codex".to_string()])),
            0
        );
        assert_eq!(
            platform.calls(),
            [
                r#"close_agents Some(["codex"])"#,
                "refresh_codex_daemon_when_idle close agents"
            ]
        );
        assert!(platform.pending_reopen.lock().unwrap().is_empty());
    }

    /// The reopen takes only the named apps off the queue, waits for their old
    /// processes before launching, and counts the launches that started. An
    /// empty queue launches nothing and waits for nothing.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn reopen_running_agents_launches_what_was_queued_after_the_wait() {
        let queued = || {
            std::sync::Mutex::new(vec![
                ("anthropic".to_string(), app("/Applications/Claude.app")),
                ("chatgpt".to_string(), app("/Applications/ChatGPT.app")),
            ])
        };
        let platform = FakePlatform {
            pending_reopen: queued(),
            relaunch_fails: vec![app("/Applications/ChatGPT.app")],
            ..FakePlatform::default()
        };
        assert_eq!(reopen_running_agents(&platform, None), 1);
        assert_eq!(
            platform.calls(),
            [
                r#"wait_for_agents_to_exit ["Claude", "ChatGPT"] 5s"#,
                r#"relaunch Path("/Applications/Claude.app")"#,
                r#"relaunch Path("/Applications/ChatGPT.app")"#,
            ]
        );
        assert!(platform.pending_reopen.lock().unwrap().is_empty());

        let platform = FakePlatform {
            pending_reopen: queued(),
            ..FakePlatform::default()
        };
        assert_eq!(
            reopen_running_agents(&platform, Some(&["anthropic".to_string()])),
            1
        );
        assert_eq!(
            *platform.pending_reopen.lock().unwrap(),
            [("chatgpt".to_string(), app("/Applications/ChatGPT.app"))]
        );
        // A second reopen of the same app finds nothing left to launch.
        assert_eq!(
            reopen_running_agents(&platform, Some(&["anthropic".to_string()])),
            0
        );
        assert_eq!(
            platform.calls().len(),
            2,
            "no second wait, no second launch"
        );
    }

    /// 7fc3d16, the exit half. Exit used to clear the routing intent unless
    /// Gate's own login item was on, so a user who relaunched any other way
    /// came up unrouted. Exit reverts the system proxy and nothing else of
    /// routing's: the journal holds no intent write, and neither the body nor
    /// the `RunEvent::Exit` arm in lib.rs names one.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn exit_never_touches_the_routing_intent() {
        let platform = signed_in();
        *platform.proxy_running.lock().unwrap() = true;
        exit(&platform);
        assert_eq!(
            platform.calls(),
            [
                "disconnect_tools_for_quit",
                "record_clean_exit",
                "disable_engine_quietly",
                "forget_engine_relay_port",
                "complete_pending_autostart_disable",
            ]
        );

        let src = include_str!("commands.rs").replace("\r\n", "\n");
        let at = src.find("pub(crate) fn exit(").expect("commands::exit");
        let body = &src[at..at + src[at..].find("\n}\n").expect("end of exit")];
        let lib = include_str!("lib.rs").replace("\r\n", "\n");
        let arm_at = lib
            .find("if let tauri::RunEvent::Exit = &event {")
            .expect("the RunEvent::Exit arm");
        let arm = &lib[arm_at..arm_at + lib[arm_at..].find("\n        });").expect("end of run")];
        for (name, text) in [("commands::exit", body), ("RunEvent::Exit", arm)] {
            for write in ["set_intent(", "set_routing_intent("] {
                assert!(!text.contains(write), "{name} must not call {write}");
            }
        }
        assert!(
            arm.contains("commands::exit("),
            "the Exit arm runs commands::exit"
        );
    }

    /// The teardown runs once per process, and never for an updater relaunch,
    /// which also keeps a pending launch-at-login opt-out armed.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn exit_tears_down_once_and_not_for_an_updater_relaunch() {
        let platform = FakePlatform {
            quit_claimed: std::sync::Mutex::new(true),
            ..signed_in()
        };
        exit(&platform);
        assert_eq!(
            platform.calls(),
            [
                "record_clean_exit",
                "disable_engine_quietly",
                "forget_engine_relay_port",
                "complete_pending_autostart_disable",
            ]
        );

        let platform = FakePlatform {
            updater_relaunching: std::sync::Mutex::new(Some(true)),
            ..signed_in()
        };
        exit(&platform);
        assert_eq!(
            platform.calls(),
            [
                "record_clean_exit",
                "disable_engine_quietly",
                "forget_engine_relay_port",
            ]
        );
        assert!(
            !*platform.quit_claimed.lock().unwrap(),
            "an updater relaunch leaves the teardown unclaimed"
        );
    }

    /// Everything after the teardown runs whatever the teardown or the revert
    /// did: the system proxy must not stay pointed at a dead port.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn exit_reverts_the_proxy_after_a_teardown_that_fails_or_panics() {
        let rest = [
            "record_clean_exit",
            "disable_engine_quietly",
            "forget_engine_relay_port",
            "complete_pending_autostart_disable",
        ];
        for platform in [
            FakePlatform {
                quit_panics: true,
                ..signed_in()
            },
            FakePlatform {
                quit_failed: vec!["Codex".into()],
                ..signed_in()
            },
            failing("disconnect_tools_for_quit"),
        ] {
            exit(&platform);
            assert_eq!(platform.calls()[0], "disconnect_tools_for_quit");
            assert_eq!(platform.calls()[1..], rest);
        }

        let platform = FakePlatform {
            fail: [
                ("record_clean_exit", "read-only".to_string()),
                ("disable_engine_quietly", "busy".to_string()),
            ]
            .into_iter()
            .collect(),
            ..signed_in()
        };
        exit(&platform);
        assert_eq!(platform.calls()[1..], rest);
    }
}
