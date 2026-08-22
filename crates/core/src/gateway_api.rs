//! One authenticated control-plane call to the gateway, and the failure
//! taxonomy every caller branches on.
//!
//! Lifted out of [`crate::activity`] when a second feature needed the same call
//! (AG-588's model preferences). The two rules in that module's doc are the
//! reason this is shared code rather than a second copy:
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
//! Both are easy to get subtly wrong and impossible to notice from the UI - a
//! call that silently used the wrong identity returns a *plausible* answer for
//! somebody else's org. Duplicating them per feature is how one copy stops
//! matching what the user's traffic actually uses.
//!
//! Bodies cross as raw JSON strings rather than typed DTOs, for the reason
//! [`crate::activity`] gives: the TypeScript side is the single place that models
//! each shape, so it cannot drift from a second model here.

use anyhow::Context;
use serde::Serialize;

use crate::account::{self, AuthMode};
use crate::oauth;

/// Why a call failed, in the terms AG-576 needs in order to offer an action.
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
    /// This org has never accepted paid Gate model use, and the write needed it
    /// (428 from `PUT /v1/me/tool-models`).
    ///
    /// Its own code because it is the one failure here that is not a failure of
    /// the *call*: the credential is entitled, a step has not happened yet, and
    /// the remedy is to show the confirmation and retry rather than to retry, to
    /// sign in, or to contact anybody. Distinguishing it is what lets the pane
    /// raise the dialog instead of reporting an error the user cannot act on.
    NeedsPaidAck,
    /// The gateway answered, unhappily. Any other non-2xx.
    Gateway,
    /// Anything else, including a response body we could not read.
    Unknown,
}

/// A failed call: the code the UI branches on, plus the detail a support report
/// needs. Both travel to the front end, which shows the code's copy and keeps
/// the message for diagnostics.
///
/// For a *write*, the message is the more useful half. The gateway's refusals on
/// that path are written to be read by a person ("Your role can view this
/// organization's model settings but not change them"), and a code cannot carry
/// which of several rules refused.
#[derive(Debug, Clone, Serialize)]
pub struct Failure {
    pub code: FailureCode,
    pub message: String,
}

impl Failure {
    pub(crate) fn new(code: FailureCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// HTTP method for [`call_json`]. Two variants because two are all the control
/// plane uses; an enum rather than a string so a typo cannot become a 405.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Put,
}

/// One authenticated control-plane call.
///
/// `url` is already resolved by the caller, which owns its own test seam (see
/// [`crate::activity`] and [`crate::tool_models`]) - this function deliberately
/// does not know the route names, so adding an endpoint does not mean editing
/// shared code.
///
/// `query` pairs are appended through `Url` rather than by string concatenation,
/// so a value the user never typed - an install id read back from the gateway's
/// own list - cannot smuggle a second parameter into the request.
///
/// `body` is sent as `application/json` when present. Note the account is loaded
/// *before* the body is sent and no retry is attempted: a write that failed
/// midway is the caller's to re-issue, and a silent retry on a non-idempotent
/// route is how one click becomes two.
pub fn call_json(
    method: Method,
    url: String,
    query: &[(&str, &str)],
    body: Option<String>,
) -> Result<String, Failure> {
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
    let url = match reqwest::Url::parse(&url).context("parsing the gateway endpoint URL") {
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
        .context("building the control-plane HTTP client")
    {
        Ok(c) => c,
        Err(e) => return Err(Failure::new(FailureCode::Unknown, format!("{e:#}"))),
    };

    let mut req = match method {
        Method::Get => client.get(url.clone()),
        Method::Put => client.put(url.clone()),
    };

    // Name this installation on the control plane too, so the gateway can mark
    // which entry in a list is the machine asking. Absent when the id could not
    // be resolved: the call is still correct, it just cannot say "this one is
    // you".
    if let Some(id) = crate::primitives::install_id_cached() {
        req = req.header("x-gate-install-id", id);
    }

    // Mirror the credential the engine and relay would send for this account, so
    // a pane cannot show numbers - or store a preference - for an identity the
    // user's traffic is not actually using.
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

    if let Some(body) = body {
        req = req.header("content-type", "application/json").body(body);
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
        } else if status == reqwest::StatusCode::PRECONDITION_REQUIRED {
            FailureCode::NeedsPaidAck
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
