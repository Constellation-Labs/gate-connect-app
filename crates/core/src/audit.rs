//! Audit event emission to the Gate dashboard API.
//!
//! Gate Connect emits an immutable audit event whenever an operator changes
//! configuration - the proxy master switch, a provider or per-domain toggle, the
//! auth mode, the selected org, or the stored Gate key. The point is that an
//! operator cannot silently suppress Gate's own logging without leaving a
//! tamper-evident trace, so a *missing* event is the failure mode that matters.
//!
//! Events go to `POST /audit/emit`, which is served by **dashboard-api**
//! (`apps/dashboard-api/src/audit/audit.controller.ts`), not by the gateway
//! proxy. Both sit behind the same ALB, and a listener rule forwards `/audit/*`
//! to dashboard-api at a higher precedence than the gateway's `/*` catch-all
//! (`terraform/aws/compute.tf`, `aws_lb_listener_rule.dashboard_api_master_admin`),
//! so posting to the account's gateway base URL reaches it. The path is
//! deliberately un-versioned: dashboard-api sets no global prefix, and `/v1/...`
//! misses that listener rule and falls through to the gateway catch-all - which
//! answers a misleading 401 about passthrough tokens rather than a 404.
//!
//! The request shape is fixed by that controller and its `AuditAuthGuard`:
//! `eventType` must be in `GATE_CONNECT_EMITTABLE_EVENT_TYPES`, `message` is
//! capped at 512 chars, `data` at 64KB, and both credential kinds ride the
//! same bearer slot (see [`Credential`]). The org is per-mode: OAuth sends the
//! selected org on `x-org-id`; ApiKey sends no org header and the endpoint
//! derives it from the key's own scope.
//!
//! Two rules the call sites depend on:
//!
//! - **Best-effort.** A failed emit never fails the operation the user asked
//!   for. Every wrapper here logs and swallows, so no caller writes `let _ =`.
//!   Failures are logged loudly on purpose: an audit pipeline that is 100%
//!   failing must not be indistinguishable from one that works.
//! - **Never elevates.** `ProxyManager::disable` and `provider::disable` are
//!   documented as needing no admin, and an elevation dialog the user can cancel
//!   while they are turning routing *off* is worse than a missing event. Nothing
//!   here elevates. Note that this rule is about admin, not about the keychain:
//!   the key read in [`credential`] is the same one `enable` already makes, so
//!   coverage does not depend on the auth mode.
//!
//! Deliberately not instrumented: `ProxyManager::disable_quiet`, the app-exit
//! path. Process shutdown is not an operator action, and a network call with a
//! 5s ceiling on the quit path is exactly the hang that function exists to
//! avoid.

use anyhow::{Context, Result};
use serde_json::json;
use std::time::Duration;

/// Where to POST the event, or `None` to skip emitting entirely.
///
/// Test seam mirroring [`crate::org`]'s `GATE_CONNECT_TEST_ORGS_ENDPOINT`:
/// `GATE_CONNECT_TEST_AUDIT_ENDPOINT` points the emit at a loopback mock (http)
/// so the header contract is assertable without a real https gateway - an
/// account's base URL must be https, so it can never name a mock itself.
///
/// The `None` arm keeps every other test hermetic. Once `GATE_CONNECT_TEST_HOME`
/// is set, the account on disk is a fixture pointing at a domain nobody serves
/// (`https://gw.example.com`), and the emits that instrumenting `account::save`
/// and friends now triggers would be real outbound requests from unit tests -
/// each one a DNS lookup and, behind a wildcard resolver, a 5s timeout. So in a
/// test home the rule is: emit only where the test said where to.
fn audit_endpoint(gateway_base_url: &str) -> Option<String> {
    if let Some(o) = std::env::var_os("GATE_CONNECT_TEST_AUDIT_ENDPOINT") {
        return Some(o.to_string_lossy().into_owned());
    }
    if std::env::var_os("GATE_CONNECT_TEST_HOME").is_some_and(|v| !v.is_empty()) {
        return None;
    }
    Some(format!(
        // No `/v1`: see the module doc - dashboard-api has no global prefix, and
        // a versioned path misses the ALB rule that routes `/audit/*` to it.
        "{}/audit/emit",
        gateway_base_url.trim_end_matches('/')
    ))
}

/// Every event this module emits is the same kind: an operator changed the
/// configuration. The `data.action` field distinguishes *which* change.
const EVENT_TYPE: &str = "admin.config.changed";

/// How an audit emit authenticates.
///
/// Both credential kinds ride the *same* bearer slot, so this is a struct rather
/// than a per-mode enum. `AuditAuthGuard` (dashboard-api,
/// `src/audit/guards/audit-auth.guard.ts`) reads one place - the standard
/// `Authorization: Bearer …` - and sniffs the `sk-gw-` prefix itself to decide
/// between validating an API key and verifying a Cognito access token. It never
/// looks at `x-gate-authorization`; sending the token there instead is a 401
/// ("Missing authorization header"), which is an empty audit trail.
///
/// This is *not* the data-plane convention. [`crate::proxy`] and
/// [`crate::org`] use the gateway's own `x-gate-*` slots because they talk to
/// gateway-proxy, which reserves `Authorization` for the upstream provider. This
/// call goes to dashboard-api, which has no such conflict.
///
/// `org_id` mirrors the data plane's tenancy rules per mode. In OAuth mode it is
/// the selected org, sent on `x-org-id` - required, because a Cognito user may
/// belong to many orgs and the controller has nothing safe to derive. In ApiKey
/// mode it is `None` and no header is sent: the controller derives the org from
/// the key's own scope, exactly as the gateway does for the key's data-plane
/// traffic. Sending a selected org here instead would 403 whenever it differs
/// from the key's scope - the one org the key can actually affect.
#[derive(Debug, Clone)]
pub struct Credential {
    /// Cognito access token, or a legacy `sk-gw-*` workspace key.
    pub token: String,
    /// Selected org UUID (OAuth mode). `None` in ApiKey mode - the endpoint
    /// derives the org from the key's scope.
    pub org_id: Option<String>,
}

/// Fit `message` inside the controller's `@MaxLength(512)`, which counts UTF-16
/// code units. Most messages are built from fixed catalog strings and are nowhere
/// near it; the one caller-shaped field is the org name in [`org_selected`],
/// which an operator picks. Truncating beats letting an over-long name 400 the
/// request, because a rejected emit is a silently missing audit event - exactly
/// what the operator would have been trying to arrange.
fn clamp_message(message: &str) -> std::borrow::Cow<'_, str> {
    const MAX_UTF16: usize = 512;
    if message.encode_utf16().count() <= MAX_UTF16 {
        return std::borrow::Cow::Borrowed(message);
    }
    // Room for the ellipsis, and cut on a char boundary so the string stays
    // valid UTF-8 (a surrogate pair may land us a unit under the budget).
    let mut units = 0;
    let mut end = 0;
    for (i, c) in message.char_indices() {
        let next = units + c.len_utf16();
        if next > MAX_UTF16 - 1 {
            break;
        }
        units = next;
        end = i + c.len_utf8();
    }
    std::borrow::Cow::Owned(format!("{}\u{2026}", &message[..end]))
}

/// Emit an audit event to the dashboard API's audit endpoint.
///
/// Resolve `credential` with [`credential`] rather than assembling one here.
/// Returns Ok only when the gateway accepted the event with a 2xx; a transport
/// error, timeout, or non-2xx status is an Err carrying the status/body. Ok is
/// also the answer when [`audit_endpoint`] declines to name a URL, which happens
/// only under the test-home seam.
///
/// Callers should go through the per-action wrappers below, which log failures
/// and swallow them, rather than calling this directly. It stays `Result` so the
/// header contract is assertable from a test.
pub fn emit(
    gateway_url: &str,
    credential: &Credential,
    message: &str,
    data: serde_json::Value,
) -> Result<()> {
    let Some(url) = audit_endpoint(gateway_url) else {
        return Ok(());
    };

    // Control-plane call: talk straight to the gateway, never through the app's
    // own data-plane proxy. `.no_proxy()` ignores any `HTTP(S)_PROXY` the app
    // exported - without it the POST loops back through Gate's own engine, which
    // would overwrite the credential below with `x-gate-api-key` and 401. Worst
    // on `proxy_enabled`, which fires immediately after that proxy comes up.
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        // Bounded: `provider_enable` / `proxy_enable` are synchronous Tauri
        // commands, so a hung gateway would otherwise block the toggle forever.
        .timeout(Duration::from_secs(5))
        .build()
        .context("building audit HTTP client")?;

    let mut request = client
        .post(&url)
        .bearer_auth(&credential.token)
        .json(&json!({
            "eventType": EVENT_TYPE,
            "message": clamp_message(message),
            "data": data,
        }));
    // ApiKey mode sends no org header at all - the endpoint derives the org
    // from the key's scope. See [`Credential`].
    if let Some(org_id) = &credential.org_id {
        request = request.header("x-org-id", org_id);
    }

    let response = request
        .send()
        .with_context(|| format!("calling the audit endpoint ({url})"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        anyhow::bail!("audit endpoint returned {status}: {body}");
    }
    Ok(())
}


/// The credential for an audit emit. `None` means there is nothing to
/// authenticate with, so the emit is skipped (and logged by the funnel).
///
/// In OAuth mode the token comes from [`crate::oauth::access_token_for_injection`],
/// which routes through `live_session()` and refreshes past skew-adjusted expiry
/// on every read - so it is always the live token, never a snapshot taken at some
/// earlier point in the session. The selected org rides along and is required:
/// the endpoint cannot derive an org for a Cognito user (they may belong to
/// several), and the OAuth flow forces the org picker before anything else is
/// usable, so the org-less window is sign-in-to-first-pick and nothing more.
///
/// In ApiKey mode the key alone is the credential; no org is sent and the
/// endpoint derives it from the key's scope (member keys are always org-scoped -
/// dashboard-api refuses to mint them otherwise). `api_key` is the key the
/// caller already holds, from an account it just loaded or a key it just saved;
/// when the caller has none, this falls back to [`crate::account::stored_api_key`].
///
/// That fallback is deliberate, and it reverses an earlier reading of this
/// module. The "promptless" guarantee on `ProxyManager::disable` and
/// `provider::disable` is about **admin elevation** - see the privilege-model
/// paragraph at the top of [`crate::proxy::manager`], which says the system-proxy
/// write needs no admin and only CA trust does. It says nothing about the
/// keychain, and treating it as a keychain rule cost every ApiKey-mode operator
/// five of the nine events. The read is also not new work: `enable` and the
/// startup reconcile call [`crate::account::load`], which reads the same item, so
/// by the time anything can be disabled the read has already happened.
pub fn credential(api_key: Option<&str>) -> Option<Credential> {
    match crate::account::auth_mode().unwrap_or_default() {
        crate::account::AuthMode::OAuth => {
            let org_id = crate::account::org_id_for_injection();
            if org_id.is_empty() {
                return None;
            }
            let token = crate::oauth::access_token_for_injection();
            if token.is_empty() {
                return None;
            }
            Some(Credential {
                token,
                org_id: Some(org_id),
            })
        }
        crate::account::AuthMode::ApiKey => {
            let token = match api_key {
                Some(key) if !key.is_empty() => key.to_string(),
                _ => crate::account::stored_api_key()
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
            };
            if token.is_empty() {
                return None;
            }
            Some(Credential {
                token,
                org_id: None,
            })
        }
    }
}

/// Resolve the credential, emit, and log whichever way it goes. The single
/// best-effort funnel every wrapper below goes through, so no call site has to
/// remember to discard a `Result` or to log a skip.
///
/// `api_key` is the caller's already-in-hand Gate key, if it has one - see
/// [`credential`].
fn emit_best_effort(
    gateway_url: &str,
    api_key: Option<&str>,
    action: &str,
    message: &str,
    data: serde_json::Value,
) {
    let Some(credential) = credential(api_key) else {
        // Not silent: a skipped audit event is the thing this feature exists to
        // make visible, so it says so even though nothing failed. Reachable only
        // with no credential at all: no Gate key stored (ApiKey mode) or signed
        // out / no org picked yet (OAuth mode).
        eprintln!("gate audit: skipped {action} - no credential to authenticate with");
        return;
    };
    if let Err(e) = emit(gateway_url, &credential, message, data) {
        eprintln!("gate audit: {action} not recorded ({e:#})");
    }
}

/// Record that the proxy master switch was turned on. `port` is `None` when the
/// engine's listening port could not be read - the field is then `null` rather
/// than a placeholder, because a fabricated `0` is indistinguishable from a real
/// reading to whoever reads the trail back.
pub fn proxy_enabled(gateway_url: &str, api_key: Option<&str>, port: Option<u16>) {
    emit_best_effort(
        gateway_url,
        api_key,
        "proxy_enabled",
        "Proxy enabled",
        json!({
            "action": "proxy_enabled",
            "proxy": {
                "newState": "on",
                "port": port,
            }
        }),
    )
}

/// Record that the proxy master switch was turned off.
pub fn proxy_disabled(gateway_url: &str, api_key: Option<&str>) {
    emit_best_effort(
        gateway_url,
        api_key,
        "proxy_disabled",
        "Proxy disabled",
        json!({
            "action": "proxy_disabled",
            "proxy": {
                "newState": "off",
            }
        }),
    )
}

/// Record that a provider was turned on.
pub fn provider_enabled(gateway_url: &str, api_key: Option<&str>, provider_name: &str) {
    emit_best_effort(
        gateway_url,
        api_key,
        "provider_enabled",
        &format!("Provider '{provider_name}' enabled"),
        json!({
            "action": "provider_enabled",
            "provider": {
                "name": provider_name,
                "newState": "on",
            }
        }),
    )
}

/// Record that a provider was turned off.
pub fn provider_disabled(gateway_url: &str, api_key: Option<&str>, provider_name: &str) {
    emit_best_effort(
        gateway_url,
        api_key,
        "provider_disabled",
        &format!("Provider '{provider_name}' disabled"),
        json!({
            "action": "provider_disabled",
            "provider": {
                "name": provider_name,
                "newState": "off",
            }
        }),
    )
}

/// Record that a single proxy domain was toggled. Emitted from the command layer
/// rather than from `ProxyManager::set_domain`, because `provider::enable` /
/// `provider::disable` drive that method internally - instrumenting it there
/// would turn one operator action into N+1 events.
pub fn domain_toggled(gateway_url: &str, api_key: Option<&str>, slug: &str, enabled: bool) {
    let state = if enabled { "on" } else { "off" };
    emit_best_effort(
        gateway_url,
        api_key,
        "domain_toggled",
        &format!("Proxy domain '{slug}' turned {state}"),
        json!({
            "action": "domain_toggled",
            "domain": {
                "slug": slug,
                "newState": state,
            }
        }),
    )
}

/// Record that the Gate API key was saved or rotated. Prefixes only - enough to
/// identify *which* key without putting the secret in the trail.
pub fn api_key_saved(
    gateway_url: &str,
    api_key: Option<&str>,
    old_prefix: Option<&str>,
    new_prefix: &str,
) {
    emit_best_effort(
        gateway_url,
        api_key,
        "api_key_saved",
        "Gateway API key updated",
        json!({
            "action": "api_key_saved",
            "api_key": {
                "previousPrefix": old_prefix,
                "newPrefix": new_prefix,
            }
        }),
    )
}

/// Record that the stored Gate API key was cleared. Must be emitted *before* the
/// credentials are destroyed - after the keychain delete and `oauth::clear()`
/// there is nothing left to authenticate the record of their removal with.
pub fn api_key_cleared(gateway_url: &str, api_key: Option<&str>, old_prefix: Option<&str>) {
    emit_best_effort(
        gateway_url,
        api_key,
        "api_key_cleared",
        "Gateway API key cleared",
        json!({
            "action": "api_key_cleared",
            "api_key": {
                "previousPrefix": old_prefix,
            }
        }),
    )
}

/// Record a change of auth mode. Which credential every request carries is an
/// operator-visible switch in its own right, independent of the key behind it.
pub fn auth_mode_changed(gateway_url: &str, api_key: Option<&str>, new_mode: &str) {
    emit_best_effort(
        gateway_url,
        api_key,
        "auth_mode_changed",
        &format!("Auth mode set to {new_mode}"),
        json!({
            "action": "auth_mode_changed",
            "auth": {
                "newMode": new_mode,
            }
        }),
    )
}

/// Record an operator-initiated org switch. Not emitted for the startup probe's
/// `clear_org`, which is the gateway rejecting a stale selection rather than
/// anyone choosing anything.
pub fn org_selected(gateway_url: &str, api_key: Option<&str>, org_id: &str, org_name: &str) {
    emit_best_effort(
        gateway_url,
        api_key,
        "org_selected",
        &format!("Organization switched to {org_name}"),
        json!({
            "action": "org_selected",
            "org": {
                "id": org_id,
                "name": org_name,
            }
        }),
    )
}
