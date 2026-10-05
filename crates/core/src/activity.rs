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
/// mock over plain http. Debug builds only, through
/// [`crate::env::test_seam`] - see it for why a release binary must refuse
/// this. In a real build it is `<gateway_base_url>/v1/me/activity`.
fn activity_endpoint(gateway_base_url: &str) -> String {
    if let Some(o) = crate::env::test_seam("GATE_CONNECT_TEST_ACTIVITY_ENDPOINT") {
        return o.to_string_lossy().into_owned();
    }
    format!("{}/v1/me/activity", gateway_base_url.trim_end_matches('/'))
}

/// The app pane's recent-request feed, with its own test seam (debug only, via
/// [`crate::env::test_seam`]). `<gateway_base_url>/v1/me/tool-events` in real
/// builds.
fn tool_events_endpoint(gateway_base_url: &str) -> String {
    if let Some(o) = crate::env::test_seam("GATE_CONNECT_TEST_TOOL_EVENTS_ENDPOINT") {
        return o.to_string_lossy().into_owned();
    }
    format!(
        "{}/v1/me/tool-events",
        gateway_base_url.trim_end_matches('/')
    )
}

/// Discovery endpoint for the installation picker, with its own test seam
/// (debug only, via [`crate::env::test_seam`]).
/// `<gateway_base_url>/v1/me/installations` in real builds.
fn installations_endpoint(gateway_base_url: &str) -> String {
    if let Some(o) = crate::env::test_seam("GATE_CONNECT_TEST_INSTALLATIONS_ENDPOINT") {
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
/// `clients` narrows it to those senders, one `tool` pair each; empty is every
/// sender. Validate it with [`overview_clients`] first.
///
/// Every failure carries a [`FailureCode`]; see that type for why. The gateway's
/// own error body is kept in the message rather than replaced by a generic
/// failure, because it is the only place a 4xx explains itself.
///
/// A reading that lands is held by [`crate::activity_cache`], so the next open
/// has something real to draw before this call returns. Nothing else changes:
/// the caller still gets the fresh body, and a cache write that fails is not a
/// failed fetch.
pub fn overview_json(install_id: Option<&str>, clients: &[&str]) -> Result<String, Failure> {
    let install_id = install_id.filter(|s| !s.is_empty());
    let query = scoped_query(install_id, clients, None);
    // The account before the request, so the reply is held under the account it
    // was asked for or not at all - see `activity_cache::store_for`.
    let taken = crate::activity_cache::scope_now();
    let body = get_json(Endpoint::Activity, &query)?;
    if let Some(taken) = taken {
        crate::activity_cache::store_for(&taken, install_id, clients, &body);
    }
    Ok(body)
}

/// The last overview that landed for this scope, if there is one.
///
/// Deliberately not a fallback inside [`overview_json`]. A held reading and a
/// fresh one are different claims - one is what happened, the other is what is
/// happening - and folding them into one return value would leave the pane
/// unable to tell which it is showing. The caller asks for both and decides.
pub fn cached_overview_json(install_id: Option<&str>, clients: &[&str]) -> Option<String> {
    crate::activity_cache::load(install_id.filter(|s| !s.is_empty()), clients)
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

/// Fetch one page of a section's recent requests, as raw JSON (AG-574).
///
/// `clients` is every sender the pane covers - the Claude pane asks for Claude
/// Code, the desktop app and claude.ai at once - and goes out as one `tool`
/// pair per name. Validate it with [`feed_clients`] first: the route requires
/// at least one, and a name the engine cannot stamp would read back empty
/// rather than fail.
///
/// `cursor` is the previous page's `nextCursor`, passed back unchanged. It is
/// opaque on purpose - the gateway owns its shape, and its keyset spans the
/// whole set - so this only forwards it.
///
/// Deliberately not cached. The held reading in [`crate::activity_cache`] is one
/// slot, and spending it on a feed that changes every request would evict the
/// overview it exists for.
pub fn tool_events_json(
    install_id: Option<&str>,
    clients: &[&str],
    cursor: Option<&str>,
) -> Result<String, Failure> {
    get_json(
        Endpoint::ToolEvents,
        &scoped_query(install_id.filter(|s| !s.is_empty()), clients, cursor),
    )
}

/// The query both scoped reads send: one `tool` pair per client, then the
/// installation and the cursor when there are any.
fn scoped_query<'a>(
    install_id: Option<&'a str>,
    clients: &[&'a str],
    cursor: Option<&'a str>,
) -> Vec<(&'static str, &'a str)> {
    let mut query: Vec<(&'static str, &'a str)> = clients.iter().map(|c| ("tool", *c)).collect();
    if let Some(id) = install_id {
        query.push(("installId", id));
    }
    if let Some(c) = cursor.filter(|s| !s.is_empty()) {
        query.push(("cursor", c));
    }
    query
}

/// The feed's client names, checked against what the engine can stamp
/// ([`crate::proxy::stamped_client`]).
///
/// An empty list is refused because the route requires a sender, and an
/// unknown name because the two sides of this boundary share one vocabulary:
/// a name that does not parse means they disagree, which is a bug worth
/// surfacing rather than a request that quietly reads back nothing.
pub fn feed_clients(names: &[String]) -> Result<Vec<&'static str>, String> {
    if names.is_empty() {
        return Err("at least one client name is required to read the feed".into());
    }
    names
        .iter()
        .map(|n| crate::proxy::stamped_client(n).ok_or_else(|| unknown_client(n)))
        .collect::<Result<_, _>>()
        .map(dedup)
}

/// The refusal for a name outside the set. The name is echoed so a mismatch
/// between the two sides can be read off the error, but only its start: the
/// list comes from the webview, and an error need not carry it back whole.
fn unknown_client(name: &str) -> String {
    let shown: String = name.chars().take(40).collect();
    let more = if shown.len() < name.len() { "..." } else { "" };
    format!("unknown client {shown:?}{more}")
}

/// Once each, in first-seen order. The set is closed, so this also bounds the
/// query to its size - well under the gateway's twenty `tool` pairs, past which
/// it refuses the request - and the query agrees with the cache key, which
/// dedups too.
fn dedup(names: Vec<&'static str>) -> Vec<&'static str> {
    let mut seen = std::collections::BTreeSet::new();
    names.into_iter().filter(|n| seen.insert(*n)).collect()
}

/// The overview's client names. Empty is allowed and means org-wide.
///
/// Wider than [`feed_clients`] by the [`ToolId`] slugs, which the tray's quick
/// status already asks for one row at a time - `env-proxy` among them, which no
/// request is ever stamped with. Refusing it would change what that row draws,
/// and that is not this read's decision to make.
pub fn overview_clients(names: &[String]) -> Result<Vec<&'static str>, String> {
    names
        .iter()
        .map(|n| {
            crate::proxy::stamped_client(n)
                .or_else(|| ToolId::from_slug(n).map(ToolId::slug))
                .ok_or_else(|| unknown_client(n))
        })
        .collect::<Result<_, _>>()
        .map(dedup)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_feed_takes_the_stamped_set_and_nothing_else() {
        assert_eq!(
            feed_clients(&names(&["claude-code", "claude-desktop", "claude-web"])),
            Ok(vec!["claude-code", "claude-desktop", "claude-web"])
        );
        assert_eq!(
            feed_clients(&names(&["codex", "chatgpt", "chatgpt-web"])),
            Ok(vec!["codex", "chatgpt", "chatgpt-web"])
        );
        assert!(feed_clients(&[]).is_err());
        assert!(feed_clients(&names(&["claude-code", "claude"])).is_err());
        // A row, not a sender; and the environment channel, which nothing stamps.
        assert!(feed_clients(&names(&["any-app"])).is_err());
        assert!(feed_clients(&names(&["env-proxy"])).is_err());
        // A refusal names the start of what it refused, not all of it.
        let long = "x".repeat(500);
        let refused = feed_clients(&[long.clone()]).unwrap_err();
        assert!(
            refused.contains(&long[..40]) && refused.len() < 80,
            "{refused}"
        );
        // Repeats go out once: twenty-one of them would otherwise be refused
        // by the gateway outright.
        assert_eq!(feed_clients(&names(&["codex"; 25])), Ok(vec!["codex"]));
        assert_eq!(
            overview_clients(&names(&["claude-web", "codex", "claude-web"])),
            Ok(vec!["claude-web", "codex"])
        );
    }

    #[test]
    fn the_overview_also_takes_tool_slugs_and_an_empty_list() {
        assert_eq!(overview_clients(&[]), Ok(vec![]));
        assert_eq!(
            overview_clients(&names(&["env-proxy"])),
            Ok(vec!["env-proxy"])
        );
        assert_eq!(
            overview_clients(&names(&["claude-web", "codex"])),
            Ok(vec!["claude-web", "codex"])
        );
        assert!(overview_clients(&names(&["any-app"])).is_err());
        assert!(overview_clients(&names(&["chatgpt", "nope"])).is_err());
    }

    #[test]
    fn one_tool_pair_per_client_then_the_installation_and_the_cursor() {
        assert_eq!(
            scoped_query(
                Some("install-7"),
                &["claude-code", "claude-desktop", "claude-web"],
                Some("c-2"),
            ),
            vec![
                ("tool", "claude-code"),
                ("tool", "claude-desktop"),
                ("tool", "claude-web"),
                ("installId", "install-7"),
                ("cursor", "c-2"),
            ]
        );
        // Page one: no cursor pair at all, and an empty one is page one too.
        assert_eq!(
            scoped_query(Some("install-7"), &["codex"], None),
            vec![("tool", "codex"), ("installId", "install-7")]
        );
        assert_eq!(
            scoped_query(None, &["codex"], Some("")),
            vec![("tool", "codex")]
        );
        // The org-wide overview sends nothing.
        assert!(scoped_query(None, &[], None).is_empty());
    }
}
