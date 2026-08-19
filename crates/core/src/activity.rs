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

use anyhow::Context;
use serde::Serialize;

use crate::account::{self, AuthMode};
use crate::oauth;

/// Why a fetch failed, in the terms AG-576 needs in order to offer an action.
///
/// Deliberately a code rather than a message: the app has to *branch* on this to
/// choose between Retry, Sign in, Switch organization and Contact support, and
/// branching on English prose is how a copy edit becomes a bug.
///
/// Note what is absent. None of these has anything to do with whether routing is
/// switched on. AG-576 calls that conflation out by name: "the gateway is
/// unreachable" and "the user turned routing off" are different facts with
/// different remedies, and reporting one as the other sends the user to fix
/// something that was never broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    /// This machine could not reach the gateway at all: DNS, TCP, TLS, timeout.
    Offline,
    /// There is no live credential to send, so the user has to sign in again.
    SignedOut,
    /// OAuth session is live but no org is selected, so there is nothing to
    /// scope the reading to.
    NoOrg,
    /// A credential was sent and the gateway refused it (401 or 403).
    Rejected,
    /// The gateway answered, unhappily. Any other non-2xx.
    Gateway,
    /// Anything else, including a response body we could not read.
    Unknown,
}

/// A failed fetch: the code the UI branches on, plus the detail a support report
/// needs. Both travel to the front end, which shows the code's copy and keeps
/// the message for diagnostics.
#[derive(Debug, Clone, Serialize)]
pub struct Failure {
    pub code: FailureCode,
    pub message: String,
}

impl Failure {
    fn new(code: FailureCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

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
pub fn overview_json(install_id: Option<&str>) -> Result<String, Failure> {
    let query: Vec<(&str, &str)> = install_id
        .filter(|s| !s.is_empty())
        .map(|id| vec![("installId", id)])
        .unwrap_or_default();
    let body = get_json(Endpoint::Activity, &query)?;
    crate::activity_cache::store(install_id.filter(|s| !s.is_empty()), &body);
    Ok(body)
}

/// The last overview that landed for this scope, if there is one.
///
/// Deliberately not a fallback inside [`overview_json`]. A held reading and a
/// fresh one are different claims - one is what happened, the other is what is
/// happening - and folding them into one return value would leave the pane
/// unable to tell which it is showing. The caller asks for both and decides.
pub fn cached_overview_json(install_id: Option<&str>) -> Option<String> {
    crate::activity_cache::load(install_id.filter(|s| !s.is_empty()))
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
}

/// One authenticated control-plane GET, shared by both reads above.
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
    };
    // Built through `Url` rather than by string concatenation so a value the user
    // never typed - an install id read back from the gateway's own list - cannot
    // smuggle a second parameter into the request.
    let url = match reqwest::Url::parse(&url).context("parsing the activity endpoint URL") {
        Ok(mut u) => {
            for (k, v) in query {
                u.query_pairs_mut().append_pair(k, v);
            }
            u
        }
        Err(e) => return Err(Failure::new(FailureCode::Unknown, format!("{e:#}"))),
    };

    let client = match reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .context("building the activity HTTP client")
    {
        Ok(c) => c,
        Err(e) => return Err(Failure::new(FailureCode::Unknown, format!("{e:#}"))),
    };

    let mut req = client.get(url.clone());

    // Name this installation on the control plane too, so the gateway can mark
    // which entry in the list is the machine asking. Absent when the id could
    // not be resolved: the reading is still correct, it just cannot say "this
    // one is you".
    if let Some(id) = crate::primitives::install_id_cached() {
        req = req.header("x-gate-install-id", id);
    }

    // Mirror the credential the engine and relay would send for this account,
    // so the pane cannot show numbers for an identity the user's traffic is
    // not actually using.
    if account.auth_mode == AuthMode::OAuth {
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
        req = req
            .header("x-gate-authorization", format!("Bearer {token}"))
            .header("x-gate-org-id", org);
    } else {
        if account.api_key.is_empty() {
            return Err(Failure::new(
                FailureCode::SignedOut,
                "no API key stored for this account",
            ));
        }
        req = req.header("x-gate-api-key", account.api_key);
    }

    let resp = match req.send() {
        Ok(r) => r,
        Err(e) => {
            // `is_connect` covers DNS and TCP, `is_timeout` the 15s ceiling
            // above. Both mean the same thing to the user - the gateway is not
            // answering this machine - and neither implicates their credential.
            let code = if e.is_connect() || e.is_timeout() {
                FailureCode::Offline
            } else {
                FailureCode::Unknown
            };
            return Err(Failure::new(
                code,
                format!("calling the gateway endpoint {url}: {e}"),
            ));
        }
    };

    let status = resp.status();
    let body = match resp.text() {
        Ok(b) => b,
        Err(e) => {
            return Err(Failure::new(
                FailureCode::Unknown,
                format!("reading the {url} response body: {e}"),
            ))
        }
    };
    if !status.is_success() {
        let code = if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            FailureCode::Rejected
        } else {
            FailureCode::Gateway
        };
        return Err(Failure::new(
            code,
            format!("gateway {url} returned {status}: {body}"),
        ));
    }
    Ok(body)
}
