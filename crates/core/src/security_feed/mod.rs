//! Live security-event feed (AG-578).
//!
//! Blocked and flagged events, pushed from the gateway over Server-Sent Events
//! while the app is running, so a request blocked *now* reaches the user without
//! them reopening a pane.
//!
//! **Why this lives in Rust and not the webview.** Three reasons, in order of
//! weight. The app's CSP (`src-tauri/tauri.conf.json`) lists no gateway origin in
//! `connect-src`, so a renderer-side `EventSource` is blocked outright; widening
//! it would let a compromised renderer talk to the cloud with no Rust-side
//! credential mediation, which is the inverse of the design [`crate::gateway_api`]
//! exists to enforce. The credential and its refresh loop are Rust-only. And the
//! push path to the window already exists as Tauri events.
//!
//! **Why not [`crate::gateway_api::call_json`].** That is `reqwest::blocking`
//! with a 15s timeout, which is right for a control-plane read and wrong for a
//! connection meant to stay open for hours. This module reuses its two *rules* -
//! `.no_proxy()`, and the credential the account is actually using - without
//! reusing its client. Those rules are not stylistic: without `.no_proxy()` the
//! app's own machine-wide `HTTPS_PROXY` export captures the call and the injected
//! `X-Gate-Api-Key` produces a spurious 401.
//!
//! **The feed is not routing.** Its state is held here, emitted on its own
//! channel, and read by nothing in [`crate::proxy`]. A dead feed must never move
//! a switch, a verdict or the master toggle, and routing being off must never
//! stop the feed - the events come from the gateway, not from local traffic.
//! `lib/groups.ts` documents what conflating observed state with intent costs.
//!
//! The wire contract is `docs/ag-578-security-events-contract.md`.

pub mod client;
pub mod notify;
pub mod sse;

use std::collections::{HashSet, VecDeque};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::account::{self, AuthMode};
use crate::gateway_api::{Failure, FailureCode};
use crate::oauth;

/// What the feed says about its own connection (AC4).
///
/// Deliberately three states and not a bool. "Reconnecting" is the one that
/// earns its place: a feed that drops for four seconds and comes back has not
/// failed, and telling the user it is Offline for those four seconds trains them
/// to ignore the indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedState {
    Live,
    Reconnecting,
    /// Also the resting state when there is no credential, no org, or the app
    /// has not started the feed. Not an error.
    Offline,
}

/// What the guardrails did. Only these two reach this stream: `redact` is
/// high-volume and low-signal, and `allow` is not a verdict. Both still appear
/// on the Overview counters and the tool-events table, which is where a complete
/// picture belongs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Block,
    Flag,
}

/// One event, as the gateway sends it and as the window renders it.
///
/// **What is absent is the point.** No prompt text, no response text, no matched
/// value, no `results` evidence blob, no conversation title and no session
/// reference. AC3 forbids them, and they are omitted from the DTO rather than
/// dropped here, so a field that never crosses the wire cannot leak through a log
/// or a crash report. If one appears in this struct, the contract was widened
/// upstream and that needs saying out loud.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityEvent {
    /// The stream position (a ULID), which is also the dedupe key. Not
    /// `requestId`: one request records a decision in each phase, so two events
    /// can share a request id and collapsing them would drop a real event.
    #[serde(default)]
    pub id: String,
    pub request_id: String,
    pub at: String,
    pub action: Action,
    /// `credential | phi | pii | injection | other`, already derived gateway-side
    /// by `deriveSecurityCategory()`. Null when nothing fired under a name its
    /// CASE chain recognises.
    #[serde(default)]
    pub category: Option<String>,
    /// Null is ordinary, not exceptional: an agent whose User-Agent is not on the
    /// gateway's allowlist is recorded as unattributed rather than guessed at.
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
}

/// The stream's opening frame.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    /// Whether `Last-Event-ID` will be honoured on reconnect. False on every
    /// deployment today: `SecurityEventBus::supportsRecovery` is a constant
    /// `false` because Redis pub/sub keeps no backlog and nothing reads the
    /// header yet.
    ///
    /// **And nothing backfills what a `false` means we missed.** The contract
    /// says this sends the client to `/v1/me/tool-events`, but that route
    /// refuses a request naming no tool ("this route reports on one tool at a
    /// time"), so no single call answers "what did I miss, across every tool".
    /// A gap - a disconnect, a gateway deploy, an app restart - is therefore
    /// permanent in the Security pane. Recorded here rather than left as the
    /// sentence that used to describe a backfill nobody wrote.
    #[serde(default)]
    pub recovery: bool,
    #[serde(default)]
    pub heartbeat_ms: Option<u64>,
}

/// What the feed hands up to whoever is driving it. `crates/core` has no `tauri`
/// dependency, so the transport to the window is the caller's business.
#[derive(Debug, Clone)]
pub enum Update {
    State(FeedState),
    Event(Box<SecurityEvent>),
}

/// Bounded set of event ids already delivered.
///
/// Needed whichever recovery path runs: a `Last-Event-ID` replay and a
/// `tool-events` backfill would each overlap the live stream on purpose, because
/// an exclusive boundary that is off by one loses an event and a user cannot tell
/// that happened. Overlapping and deduping is the direction whose failure mode is
/// visible. Today it guards the live stream alone: neither recovery path exists.
///
/// **Not persisted across a restart.** Nothing backfills after one either, so a
/// restart starts the pane empty - see `Hello::recovery`.
#[derive(Debug)]
pub struct Dedupe {
    seen: HashSet<String>,
    order: VecDeque<String>,
    cap: usize,
}

impl Dedupe {
    pub fn new(cap: usize) -> Self {
        Self {
            seen: HashSet::new(),
            order: VecDeque::new(),
            cap: cap.max(1),
        }
    }

    /// True when this id has not been seen, recording it. False when it is a
    /// duplicate and the caller should drop the event.
    ///
    /// An empty id is always admitted: it means the server sent no `id:`, so
    /// there is nothing to deduplicate on, and silently collapsing every such
    /// event into one would be worse than showing a repeat.
    pub fn admit(&mut self, id: &str) -> bool {
        if id.is_empty() {
            return true;
        }
        if !self.seen.insert(id.to_string()) {
            return false;
        }
        self.order.push_back(id.to_string());
        while self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }

    /// Forget everything. Called on an org switch: events belonging to the org
    /// the user just left must not suppress an identical-looking one in the org
    /// they moved to.
    pub fn clear(&mut self) {
        self.seen.clear();
        self.order.clear();
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

/// Longest a reconnect will ever wait.
pub const BACKOFF_CEILING: Duration = Duration::from_secs(60);
/// Shortest, before jitter, and the floor a server `retry:` cannot undercut.
pub const BACKOFF_FLOOR_MS: u64 = 1_000;
/// How many events the window can ask for after opening mid-session.
pub const RECENT_CAPACITY: usize = 200;
/// How many ids to remember for dedupe.
pub const DEDUPE_CAPACITY: usize = 2_000;

/// Delay before reconnect attempt `attempt` (0-based), with **full jitter**.
///
/// Full jitter, not a fixed backoff and not a jittered band around the target:
/// the gateway's `/v1/me/*` throttle bucket is keyed on **source address**, not
/// on the credential, so every Gate Connect instance behind one office NAT or VPN
/// egress shares one 100-req/min budget. Lockstep reconnects after a gateway
/// deploy are therefore a self-inflicted 429 for a whole office, and a 429 raised
/// there is indistinguishable to the client from one raised by the proxy path.
/// `src/lib/activity.ts` refuses to poll for the same reason.
///
/// `floor_ms` lets the server raise the floor through an SSE `retry:` field. It
/// can raise it and never lower it: the ceiling and the jitter are ours to keep.
pub fn backoff_delay(attempt: u32, floor_ms: u64) -> Duration {
    let floor = floor_ms.max(BACKOFF_FLOOR_MS);
    let ceiling = BACKOFF_CEILING.as_millis() as u64;
    // Saturating so a long outage cannot overflow the shift into a tiny delay.
    let window = floor
        .saturating_mul(1u64 << attempt.min(16))
        .min(ceiling)
        .max(floor);
    let jittered = rand::random::<f64>() * window as f64;
    Duration::from_millis(jittered as u64)
}

/// The stream endpoint, with the test seam every other gateway call in this
/// crate carries. `env::test_seam` refuses to honour it in release builds, so a
/// shipped app cannot be pointed at an attacker's stream by an environment
/// variable.
pub fn endpoint() -> String {
    if let Some(o) = crate::env::test_seam("GATE_CONNECT_TEST_SECURITY_EVENTS_ENDPOINT") {
        return o.to_string_lossy().into_owned();
    }
    let base = account::load()
        .ok()
        .flatten()
        .map(|a| a.gateway_base_url)
        .unwrap_or_default();
    format!(
        "{}/v1/me/security-events/stream",
        base.trim_end_matches('/')
    )
}

/// The credential headers this account should send, resolved fresh.
///
/// Resolved per connect attempt rather than once at startup, so a token the
/// background refresh loop replaced mid-stream is picked up by the next reconnect
/// with no coordination between the two.
///
/// Mirrors [`crate::gateway_api::call_json`] deliberately. Two implementations of
/// "which header does this account send" is the one duplication that returns a
/// plausible answer for the wrong org.
pub fn credential_headers() -> Result<Vec<(&'static str, String)>, Failure> {
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

    let mut headers: Vec<(&'static str, String)> = Vec::new();
    if let Some(id) = crate::primitives::install_id_cached() {
        headers.push(("x-gate-install-id", id.to_string()));
    }

    if account.auth_mode == AuthMode::OAuth {
        // `live_session` refreshes; `current` is a raw keychain read and can fail
        // on a token that was only stale.
        let token = oauth::access_token_for_injection();
        if token.is_empty() {
            return Err(Failure::new(
                FailureCode::SignedOut,
                "no live OAuth session for this account",
            ));
        }
        let org = account::org_id_for_injection();
        if org.is_empty() {
            return Err(Failure::new(FailureCode::NoOrg, "no organization selected"));
        }
        headers.push(("x-gate-authorization", format!("Bearer {token}")));
        headers.push(("x-gate-org-id", org));
    } else {
        if account.api_key.is_empty() {
            return Err(Failure::new(
                FailureCode::SignedOut,
                "no API key stored for this account",
            ));
        }
        headers.push(("x-gate-api-key", account.api_key));
    }
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupe_admits_once_and_refuses_a_repeat() {
        let mut d = Dedupe::new(8);
        assert!(d.admit("01A"));
        assert!(!d.admit("01A"));
        assert!(d.admit("01B"));
    }

    #[test]
    fn dedupe_evicts_oldest_beyond_its_cap() {
        let mut d = Dedupe::new(2);
        d.admit("a");
        d.admit("b");
        d.admit("c");
        assert_eq!(d.len(), 2);
        // "a" fell out, so it is admitted again rather than wrongly suppressed.
        assert!(d.admit("a"));
        // "c" is still remembered.
        assert!(!d.admit("c"));
    }

    #[test]
    fn dedupe_always_admits_an_empty_id() {
        let mut d = Dedupe::new(8);
        assert!(d.admit(""));
        assert!(d.admit(""));
        assert!(d.is_empty(), "an empty id is not worth remembering");
    }

    #[test]
    fn dedupe_clears_for_an_org_switch() {
        let mut d = Dedupe::new(8);
        d.admit("01A");
        d.clear();
        assert!(d.admit("01A"));
    }

    #[test]
    fn backoff_never_exceeds_the_ceiling() {
        for attempt in 0..40 {
            for _ in 0..20 {
                assert!(backoff_delay(attempt, BACKOFF_FLOOR_MS) <= BACKOFF_CEILING);
            }
        }
    }

    #[test]
    fn backoff_widens_with_the_attempt_count() {
        // Full jitter makes any single draw meaningless, so compare the widest
        // draw seen over many, which tracks the window rather than the mean.
        let widest = |attempt: u32| {
            (0..400)
                .map(|_| backoff_delay(attempt, BACKOFF_FLOOR_MS))
                .max()
                .unwrap()
        };
        assert!(widest(4) > widest(0));
    }

    #[test]
    fn a_server_retry_raises_the_floor_but_cannot_lower_it() {
        // A server asking to be hit sooner than our own floor is not obeyed.
        for _ in 0..200 {
            assert!(backoff_delay(0, 1) <= Duration::from_millis(BACKOFF_FLOOR_MS));
        }
        // A server asking for longer is.
        let widest = (0..400).map(|_| backoff_delay(0, 8_000)).max().unwrap();
        assert!(widest > Duration::from_millis(BACKOFF_FLOOR_MS));
    }

    #[test]
    fn an_event_deserialises_from_the_wire_shape() {
        let raw = r#"{"id":"01J","requestId":"r-1","at":"2026-08-31T14:03:00Z",
                      "action":"block","category":"credential","tool":"claude-code",
                      "model":"claude-opus-4","provider":"anthropic"}"#;
        let ev: SecurityEvent = serde_json::from_str(raw).expect("parses");
        assert_eq!(ev.action, Action::Block);
        assert_eq!(ev.request_id, "r-1");
        assert_eq!(ev.category.as_deref(), Some("credential"));
    }

    #[test]
    fn an_event_tolerates_every_nullable_field_being_absent() {
        // The gateway records null for an unattributed agent, and a payload that
        // simply omits the key must not fail the whole frame.
        let raw = r#"{"id":"01J","requestId":"r-2","at":"t","action":"flag"}"#;
        let ev: SecurityEvent = serde_json::from_str(raw).expect("parses");
        assert_eq!(ev.action, Action::Flag);
        assert!(ev.tool.is_none() && ev.model.is_none() && ev.category.is_none());
    }

    #[test]
    fn hello_defaults_to_no_recovery() {
        // A server that says nothing about recovery is assumed not to support it,
        // so the client backfills rather than trusting a replay that never comes.
        let h: Hello = serde_json::from_str("{}").expect("parses");
        assert!(!h.recovery);
    }
}
