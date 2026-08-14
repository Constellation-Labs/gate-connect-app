//! Activity overview for the Gate Connect popover (AG-572).
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
//! is a deliberate choice while the contract is still moving: the popover's
//! temporary viewer renders whatever the gateway sends, so the two cannot drift
//! and no field has to be modelled twice before it is agreed.

use anyhow::{bail, Context, Result};

use crate::account::{self, AuthMode};
use crate::oauth;

/// Endpoint URL. Test seam mirroring [`crate::org`]'s
/// `GATE_CONNECT_TEST_ORGS_ENDPOINT`, so the fetch can be pointed at a loopback
/// mock over plain http. Unset in real builds, where it is
/// `<gateway_base_url>/v1/me/activity`.
fn activity_endpoint(gateway_base_url: &str) -> String {
    if let Some(o) = std::env::var_os("GATE_CONNECT_TEST_ACTIVITY_ENDPOINT") {
        return o.to_string_lossy().into_owned();
    }
    format!(
        "{}/v1/me/activity",
        gateway_base_url.trim_end_matches('/')
    )
}

/// Fetch the overview for the current account, as raw JSON.
///
/// Errors when no account is configured, when no usable credential exists, or
/// when the gateway answers with a non-success status (the body is included so
/// the viewer can show the gateway's own error envelope rather than a generic
/// failure).
pub fn overview_json() -> Result<String> {
    let account = account::load()?.context("no gateway account is configured")?;

    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .context("building the activity HTTP client")?;

    let mut req = client.get(activity_endpoint(&account.gateway_base_url));

    // Mirror the credential the engine and relay would send for this account,
    // so the popover cannot show numbers for an identity the user's traffic is
    // not actually using.
    if account.auth_mode == AuthMode::OAuth {
        let token = oauth::access_token_for_injection();
        if token.is_empty() {
            bail!("signed out: no live OAuth session for this account");
        }
        let org = account::org_id_for_injection();
        if org.is_empty() {
            bail!("no organization selected");
        }
        req = req
            .header("x-gate-authorization", format!("Bearer {token}"))
            .header("x-gate-org-id", org);
    } else {
        if account.api_key.is_empty() {
            bail!("no API key stored for this account");
        }
        req = req.header("x-gate-api-key", account.api_key);
    }

    let resp = req.send().context("calling the gateway /v1/me/activity endpoint")?;
    let status = resp.status();
    let body = resp
        .text()
        .context("reading /v1/me/activity response body")?;
    if !status.is_success() {
        bail!("gateway /v1/me/activity returned {status}: {body}");
    }
    Ok(body)
}
