//! Org selection for the OAuth flow.
//!
//! After a Cognito sign-in the gateway requires the caller to pick one of the
//! user's organizations and send its id on `X-Gate-Org-Id` with every
//! OAuth-authenticated request (a request with a Cognito token but no org is
//! rejected). This module lists the pickable orgs from the gateway's
//! `GET /v1/me/orgs` endpoint; the selection itself is persisted by the
//! [`account`](crate::account) layer and injected alongside the token by the
//! proxy engine / relay.
//!
//! The list call authenticates with the same custom `x-gate-authorization`
//! header the gateway uses for inference (NOT the standard `Authorization`
//! slot, which is reserved for `sk-gw-*` keys) and needs no org header itself.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// One organization the signed-in user may act on, from `GET /v1/me/orgs`.
/// Serialized straight to the frontend, so the field names double as the DTO.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Org {
    /// Org UUID - the exact value sent back as `X-Gate-Org-Id`. The gateway
    /// matches it against the user's active memberships (not the slug).
    #[serde(rename = "orgId")]
    pub org_id: String,
    pub name: String,
    pub slug: String,
    pub role: String,
}

/// Shape of the `GET /v1/me/orgs` response: `{ user, orgs: [...] }`. We only
/// need the `orgs` array.
#[derive(Deserialize)]
struct OrgsResponse {
    orgs: Vec<Org>,
}

/// The orgs endpoint URL. Test seam mirroring [`crate::oauth`]'s
/// `GATE_CONNECT_TEST_TOKEN_ENDPOINT`: point the list call at a loopback mock
/// (http) so it's hermetically testable without a real https gateway. Unset in
/// real builds, where it's `<gateway_base_url>/v1/me/orgs`.
fn orgs_endpoint(gateway_base_url: &str) -> String {
    if let Some(o) = crate::env::test_seam("GATE_CONNECT_TEST_ORGS_ENDPOINT") {
        return o.to_string_lossy().into_owned();
    }
    format!("{}/v1/me/orgs", gateway_base_url.trim_end_matches('/'))
}

/// One orgs call's outcome, keeping the gateway's refusal apart from every
/// other way the call can fail. A 401 is the one status worth acting on rather
/// than reporting: it is the gateway saying the bearer is dead, which
/// [`list_current`] can often fix by itself.
enum OrgsCall {
    Listed(Vec<Org>),
    /// HTTP 401. Carries the body, so a caller that gives up still reports
    /// what the gateway said.
    Unauthorized(String),
    Failed(anyhow::Error),
}

/// One `GET /v1/me/orgs` with `access_token`, no retry and no interpretation.
fn call_orgs(gateway_base_url: &str, access_token: &str) -> OrgsCall {
    // Control-plane call: talk straight to the gateway, never through the
    // app's own data-plane proxy (which injects `X-Gate-Api-Key`, not the
    // OAuth token). `.no_proxy()` ignores any `HTTP(S)_PROXY` the app set.
    let call = || -> Result<(reqwest::StatusCode, String)> {
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            // Bounded like `probe_session`, and for a sharper reason here:
            // `list_current` can make two of these plus a token refresh
            // inside one Tauri command, and a black-holed gateway would
            // otherwise hang the org picker until the OS gives up on the
            // socket, with nothing to cancel it.
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .context("building the orgs HTTP client")?;
        let resp = client
            .get(orgs_endpoint(gateway_base_url))
            .header("x-gate-authorization", format!("Bearer {access_token}"))
            .send()
            .context("calling the gateway /v1/me/orgs endpoint")?;
        let status = resp.status();
        let body = resp.text().context("reading /v1/me/orgs response body")?;
        Ok((status, body))
    };
    let (status, body) = match call() {
        Ok(pair) => pair,
        Err(e) => return OrgsCall::Failed(e),
    };
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return OrgsCall::Unauthorized(body);
    }
    if !status.is_success() {
        return OrgsCall::Failed(anyhow::anyhow!(
            "gateway /v1/me/orgs returned {status}: {body}"
        ));
    }
    match parse_orgs(&body) {
        Ok(orgs) => OrgsCall::Listed(orgs),
        Err(e) => OrgsCall::Failed(e),
    }
}

/// List the orgs the OAuth user may act on. `access_token` is the Cognito
/// access token, sent on `x-gate-authorization` (the gateway's custom slot).
/// Takes the token as given - see [`list_current`] for the app's path, which
/// recovers a refusable token instead of reporting it.
pub fn list(gateway_base_url: &str, access_token: &str) -> Result<Vec<Org>> {
    match call_orgs(gateway_base_url, access_token) {
        OrgsCall::Listed(orgs) => Ok(orgs),
        OrgsCall::Unauthorized(body) => {
            bail!("gateway /v1/me/orgs returned 401 Unauthorized: {body}")
        }
        OrgsCall::Failed(e) => Err(e),
    }
}

/// List orgs for the current account's gateway and live OAuth session.
/// Errors if no gateway is configured or no OAuth session is stored.
///
/// Goes through [`crate::oauth::live_session`], the same source the proxy
/// injects from, rather than reading the stored bundle raw: this call used to
/// be the one place that sent whatever was in the keychain, so a token that
/// lapsed since the last refresh tick 401'd here while every other path had
/// already renewed it.
///
/// A 401 is then retried once against a force-refreshed token, because the
/// local expiry check that `live_session` trusts cannot see a clock that moved
/// under it (see [`crate::oauth::force_refresh`]). If the gateway refuses that
/// too, the session really is dead: record it, so the whole app drops to the
/// sign-in state instead of this one screen showing an error while the tray
/// stays green and the engine keeps injecting a refused token.
pub fn list_current() -> Result<Vec<Org>> {
    let gateway = crate::account::load_base_url()?
        .context("no Gate account configured - sign in before listing orgs")?;
    let token = crate::oauth::live_session()
        .context("not signed in via OAuth - sign in before listing orgs")?
        .access_token;
    let refused = match call_orgs(&gateway, &token) {
        OrgsCall::Listed(orgs) => return Ok(orgs),
        OrgsCall::Failed(e) => return Err(e),
        OrgsCall::Unauthorized(body) => body,
    };
    let Some(cfg) = crate::oauth::OAuthConfig::from_build_env() else {
        bail!("gateway /v1/me/orgs returned 401 Unauthorized: {refused}");
    };
    let renewed = match crate::oauth::force_refresh(&cfg) {
        Ok(Some(tokens)) => tokens.access_token,
        // Nothing stored to renew: the gateway's refusal stands as reported.
        Ok(None) => bail!("gateway /v1/me/orgs returned 401 Unauthorized: {refused}"),
        // Cognito refused the renewal too, so the refusal is about the
        // session and only an interactive sign-in clears it.
        Err(e) if e.is_refusal() => {
            crate::oauth::mark_session_rejected();
            return Err(e).context("the gateway rejected the session and it could not be renewed");
        }
        // Cognito was unreachable. The gateway's 401 stands as an error to
        // report, but nothing here proves the session is dead - a network
        // that is down cannot testify about a credential.
        Err(e) => {
            return Err(e).context(format!(
                "gateway /v1/me/orgs returned 401 Unauthorized: {refused}; \
                 renewing the session could not reach the identity provider"
            ));
        }
    };
    match call_orgs(&gateway, &renewed) {
        OrgsCall::Listed(orgs) => Ok(orgs),
        OrgsCall::Failed(e) => Err(e),
        OrgsCall::Unauthorized(body) => {
            crate::oauth::mark_session_rejected();
            bail!("gateway /v1/me/orgs returned 401 Unauthorized: {body}")
        }
    }
}

fn parse_orgs(body: &str) -> Result<Vec<Org>> {
    let parsed: OrgsResponse =
        serde_json::from_str(body).context("parsing /v1/me/orgs response")?;
    Ok(parsed.orgs)
}

/// Verdict of [`probe_session`]: what the gateway said about a session that
/// looks valid locally.
#[derive(Debug)]
pub enum SessionProbe {
    /// Token accepted; these are the user's current org memberships.
    Accepted(Vec<Org>),
    /// The gateway explicitly refused the token (HTTP 401). The session is
    /// dead server-side however fresh it looks locally.
    Rejected,
    /// No verdict: unreachable, timed out, a non-auth error status, or an
    /// unparseable body. Never treated as evidence against the session.
    Unavailable,
}

/// Ask the gateway whether the stored session actually works: one
/// `GET /v1/me/orgs` with the access token. Run at startup, where a session
/// killed by upgrade or server-side drift (revoked user, reseeded org data)
/// would otherwise keep reading as "connected" until traffic fails - the token
/// looks fine locally, so no local check can catch it. Only a definite 401 is
/// a rejection; anything short of a verdict is [`SessionProbe::Unavailable`]
/// so an offline start never signs the user out. Short timeout so it can sit
/// on the startup path without stalling launch.
pub fn probe_session(gateway_base_url: &str, access_token: &str) -> SessionProbe {
    // Control-plane call, same rules as `list`: straight to the gateway,
    // never through the app's own data-plane proxy. A probe misrouted through
    // the proxy would 401 spuriously and kill a good session.
    let Ok(client) = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    else {
        return SessionProbe::Unavailable;
    };
    let Ok(resp) = client
        .get(orgs_endpoint(gateway_base_url))
        .header("x-gate-authorization", format!("Bearer {access_token}"))
        .send()
    else {
        return SessionProbe::Unavailable;
    };
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return SessionProbe::Rejected;
    }
    if !resp.status().is_success() {
        return SessionProbe::Unavailable;
    }
    resp.text()
        .ok()
        .and_then(|body| parse_orgs(&body).ok())
        .map(SessionProbe::Accepted)
        .unwrap_or(SessionProbe::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_orgs_response_mapping_org_id() {
        let body = r#"{
          "user": { "id": "u-1", "email": "dev@example.test" },
          "orgs": [
            { "orgId": "org-uuid-1", "name": "Acme Inc", "slug": "acme", "role": "owner" },
            { "orgId": "org-uuid-2", "name": "Side Co", "slug": "side", "role": "member" }
          ]
        }"#;
        let orgs = parse_orgs(body).unwrap();
        assert_eq!(orgs.len(), 2);
        assert_eq!(orgs[0].org_id, "org-uuid-1");
        assert_eq!(orgs[0].name, "Acme Inc");
        assert_eq!(orgs[0].slug, "acme");
        assert_eq!(orgs[0].role, "owner");
        assert_eq!(orgs[1].org_id, "org-uuid-2");
    }

    #[test]
    fn org_serializes_with_camel_case_org_id() {
        // The frontend consumes `orgId`, so the on-wire field must be camelCase.
        let org = Org {
            org_id: "org-uuid-1".into(),
            name: "Acme".into(),
            slug: "acme".into(),
            role: "owner".into(),
        };
        let json = serde_json::to_string(&org).unwrap();
        assert!(json.contains("\"orgId\":\"org-uuid-1\""), "got {json}");
        assert!(!json.contains("org_id"), "got {json}");
    }

    #[test]
    fn empty_orgs_list_parses() {
        let body = r#"{ "user": { "id": "u", "email": "e" }, "orgs": [] }"#;
        assert!(parse_orgs(body).unwrap().is_empty());
    }
}
