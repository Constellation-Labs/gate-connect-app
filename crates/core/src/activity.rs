//! Activity overview for Gate Connect's Overview pane (AG-572).
//!
//! Reads `GET /v1/me/activity` on the gateway: counters, an hourly request
//! series, and policy / token-savings state for the signed-in org. The gateway
//! side of this contract lives in `apps/gateway-proxy/src/activity/` in the
//! `gate` repo.
//!
//! Modelled on [`crate::org`], deliberately and line for line, because the same
//! two rules apply to every control-plane call this app makes:
//!
//! 1. **Talk straight to the gateway, never through our own data-plane proxy.**
//!    `.no_proxy()` ignores the `HTTP(S)_PROXY` variables the app itself exports
//!    machine-wide. A control call captured by our own engine would arrive with
//!    `X-Gate-Api-Key` injected instead of the caller's credential and 401.
//! 2. **Send the credential the account is actually using.** OAuth accounts send
//!    the Cognito access token on `x-gate-authorization` plus the selected org on
//!    `x-gate-org-id`; key accounts send `x-gate-api-key`. The gateway resolves
//!    either into one org.
//!
//! The response is returned as a raw JSON string rather than a typed DTO. This
//! is a deliberate choice while the contract is still moving: `src/lib/activity.ts`
//! is the single place that knows the shape, so it cannot drift from a second
//! model here and no field has to be agreed twice. Failures, by contrast, *are*
//! typed - see [`FailureCode`].

use crate::account;
use crate::gateway_api;
use crate::registry::ToolId;

/// The failure taxonomy, and the authenticated call itself, now live in
/// [`crate::gateway_api`] - a second feature needed the same credential rules
/// (AG-588), and two copies of "which header does this account send" is the one
/// duplication that returns a plausible answer for the wrong org.
///
/// Re-exported rather than relocated in the callers: `activity::Failure` is the
/// path the IPC layer already names.
pub use crate::gateway_api::{Failure, FailureCode};

/// Endpoint URL. Test seam mirroring [`crate::org`]'s
/// `GATE_CONNECT_TEST_ORGS_ENDPOINT`, so the fetch can be pointed at a loopback
/// mock over plain http. Unset in real builds, where it is
/// `<gateway_base_url>/v1/me/activity`.
fn activity_endpoint(gateway_base_url: &str) -> String {
    if let Some(o) = std::env::var_os("GATE_CONNECT_TEST_ACTIVITY_ENDPOINT") {
        return o.to_string_lossy().into_owned();
    }
    format!("{}/v1/me/activity", gateway_base_url.trim_end_matches('/'))
}

/// One tool's recent-request feed, with its own test seam.
/// `<gateway_base_url>/v1/me/tool-events` in real builds.
fn tool_events_endpoint(gateway_base_url: &str) -> String {
    if let Some(o) = std::env::var_os("GATE_CONNECT_TEST_TOOL_EVENTS_ENDPOINT") {
        return o.to_string_lossy().into_owned();
    }
    format!(
        "{}/v1/me/tool-events",
        gateway_base_url.trim_end_matches('/')
    )
}

/// Discovery endpoint for the installation picker, with its own test seam.
/// `<gateway_base_url>/v1/me/installations` in real builds.
fn installations_endpoint(gateway_base_url: &str) -> String {
    if let Some(o) = std::env::var_os("GATE_CONNECT_TEST_INSTALLATIONS_ENDPOINT") {
        return o.to_string_lossy().into_owned();
    }
    format!(
        "{}/v1/me/installations",
        gateway_base_url.trim_end_matches('/')
    )
}

/// Fetch the overview for the current account, as raw JSON.
///
/// `install_id` scopes the whole reading to one installation (AG-572 AC 1);
/// `None` is the org-wide default. The gateway narrows every section or none, so
/// the client never has to reason about a half-scoped payload.
///
/// Every failure carries a [`FailureCode`]; see that type for why. The gateway's
/// own error body is kept in the message rather than replaced by a generic
/// failure, because it is the only place a 4xx explains itself.
///
/// A reading that lands is held by [`crate::activity_cache`], so the next open
/// has something real to draw before this call returns. Nothing else changes:
/// the caller still gets the fresh body, and a cache write that fails is not a
/// failed fetch.
pub fn overview_json(install_id: Option<&str>, tool: Option<ToolId>) -> Result<String, Failure> {
    let install_id = install_id.filter(|s| !s.is_empty());
    let mut query: Vec<(&str, &str)> = Vec::new();
    if let Some(id) = install_id {
        query.push(("installId", id));
    }
    if let Some(t) = tool {
        query.push(("tool", t.slug()));
    }
    let body = get_json(Endpoint::Activity, &query)?;
    crate::activity_cache::store(install_id, tool, &body);
    Ok(body)
}

/// The last overview that landed for this scope, if there is one.
///
/// Deliberately not a fallback inside [`overview_json`]. A held reading and a
/// fresh one are different claims - one is what happened, the other is what is
/// happening - and folding them into one return value would leave the pane
/// unable to tell which it is showing. The caller asks for both and decides.
pub fn cached_overview_json(install_id: Option<&str>, tool: Option<ToolId>) -> Option<String> {
    crate::activity_cache::load(install_id.filter(|s| !s.is_empty()), tool)
}

/// Every held per-tool reading for this installation scope, keyed by slug.
///
/// One disk read for a surface that draws a figure on every row. The tray's quick
/// status is that surface: it opens on what is on disk and refreshes only what has
/// gone stale, because `/v1/me/activity` answers for one tool at a time and a
/// read per row per open is the fan-out its throttle bucket cannot take.
///
/// Raw bodies, like the rest of this module - `src/lib/activity.ts` stays the only
/// place that knows the payload's shape, which is also what lets the *caller*
/// decide what "stale" means from each body's own `generatedAt` - which
/// `lib/activity.ts` surfaces as `ActivityView.takenAtMs` for exactly that, the
/// tray being a caller that holds readings it did not fetch itself.
pub fn cached_tool_overviews_json(
    install_id: Option<&str>,
) -> std::collections::BTreeMap<String, String> {
    crate::activity_cache::load_tools(install_id.filter(|s| !s.is_empty()))
}

/// Fetch one page of a tool's recent requests, as raw JSON (AG-574).
///
/// `tool` is a [`ToolId`] rather than a string so the closed set of slugs is
/// enforced here, by the compiler, instead of by the gateway rejecting a value
/// this side let through. The route requires it: the feed is always about one
/// tool.
///
/// `cursor` is the previous page's `nextCursor`, passed back unchanged. It is
/// opaque on purpose - the gateway owns its shape - so this only forwards it.
///
/// Deliberately not cached. The held reading in [`crate::activity_cache`] is one
/// slot, and spending it on a feed that changes every request would evict the
/// overview it exists for.
pub fn tool_events_json(
    install_id: Option<&str>,
    tool: ToolId,
    cursor: Option<&str>,
) -> Result<String, Failure> {
    let mut query: Vec<(&str, &str)> = vec![("tool", tool.slug())];
    if let Some(id) = install_id.filter(|s| !s.is_empty()) {
        query.push(("installId", id));
    }
    if let Some(c) = cursor.filter(|s| !s.is_empty()) {
        query.push(("cursor", c));
    }
    get_json(Endpoint::ToolEvents, &query)
}

/// Fetch the installations this account has sent traffic from, as raw JSON.
///
/// The list is derived from traffic, so it is empty until something has been
/// attributed - which is the honest answer, and what the picker renders as
/// "unattributed" rather than as a broken screen.
pub fn installations_json() -> Result<String, Failure> {
    get_json(Endpoint::Installations, &[])
}

/// Which of the two activity reads is being made. They differ only in URL: the
/// credential rules, the timeout and the failure taxonomy are identical, and
/// keeping them in one function is what stops the two drifting.
enum Endpoint {
    Activity,
    Installations,
    ToolEvents,
}

/// One authenticated control-plane GET, shared by every read above.
///
/// Nothing here but the URL: the credential rules, timeout, install-id header
/// and failure mapping are [`crate::gateway_api::call_json`]'s, so the reads and
/// AG-588's write cannot diverge on any of them.
fn get_json(which: Endpoint, query: &[(&str, &str)]) -> Result<String, Failure> {
    let account = match account::load() {
        Ok(Some(a)) => a,
        Ok(None) => {
            return Err(Failure::new(
                FailureCode::SignedOut,
                "no gateway account is configured",
            ))
        }
        Err(e) => return Err(Failure::new(FailureCode::Unknown, format!("{e:#}"))),
    };
    let url = match which {
        Endpoint::Activity => activity_endpoint(&account.gateway_base_url),
        Endpoint::Installations => installations_endpoint(&account.gateway_base_url),
        Endpoint::ToolEvents => tool_events_endpoint(&account.gateway_base_url),
    };
    gateway_api::call_json(gateway_api::Method::Get, url, query, None)
}
