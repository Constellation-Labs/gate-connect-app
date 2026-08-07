//! Audit event emission to the dashboard API.
//!
//! Gate Connect emits audit events whenever state changes:
//! - Proxy enable/disable
//! - Provider enable/disable
//! - API key changes
//!
//! Events are sent to `POST /v1/audit/emit` on the dashboard API.
//! Best-effort: failures are logged but don't block user operations.
//! Audit emission must be completely silent - never prompt for credentials.
//!
//! The caller should provide gateway_url, auth_token, and org_id from sources
//! that don't require prompting (e.g., already-loaded account, config file,
//! oauth::access_token_for_injection()). If any required info is unavailable
//! without prompting, the audit event is silently skipped.

use anyhow::{Context, Result};
use serde_json::json;
use std::time::Duration;

/// Emit an audit event to the dashboard API.
///
/// All parameters must be obtained without prompting. If any required information
/// is unavailable, the caller should skip the emit silently (best-effort).
///
/// `auth_token`: The bearer token (API key or OAuth access token).
/// Returns Ok on success (event recorded with 2xx status).
/// On any failure (empty token, transport error, non-2xx status, or timeout),
/// logs the error and returns Err. Callers should ignore the error (`let _ = emit(...)`).
pub fn emit(
    gateway_url: &str,
    auth_token: &str,
    org_id: &str,
    event_type: &str,
    message: &str,
    data: serde_json::Value,
) -> Result<()> {
    if auth_token.is_empty() {
        return Err(anyhow::anyhow!("auth_token is required (cannot emit without authentication)"));
    }

    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .context("building audit HTTP client")?;

    let url = format!("{}/v1/audit/emit", gateway_url.trim_end_matches('/'));

    let request = json!({
        "eventType": event_type,
        "message": message,
        "data": data,
    });

    match client
        .post(&url)
        .header("x-org-id", org_id)
        .bearer_auth(auth_token)
        .json(&request)
        .send()
    {
        Ok(response) => {
            if !response.status().is_success() {
                eprintln!(
                    "[gate] audit emit failed: {} ({})",
                    response.status(),
                    url
                );
                return Err(anyhow::anyhow!("audit emit returned {}", response.status()));
            }
            Ok(())
        }
        Err(e) => {
            eprintln!("[gate] audit emit error: {e} ({})", url);
            Err(e).context("sending audit emit request")
        }
    }
}

/// Emit a proxy enable event.
/// Caller must provide auth_token without prompting.
pub fn proxy_enabled(
    gateway_url: &str,
    auth_token: &str,
    org_id: &str,
    port: u16,
) -> Result<()> {
    emit(
        gateway_url,
        auth_token,
        org_id,
        "admin.config.changed",
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

/// Emit a proxy disable event.
/// Caller must provide auth_token without prompting.
pub fn proxy_disabled(gateway_url: &str, auth_token: &str, org_id: &str) -> Result<()> {
    emit(
        gateway_url,
        auth_token,
        org_id,
        "admin.config.changed",
        "Proxy disabled",
        json!({
            "action": "proxy_disabled",
            "proxy": {
                "newState": "off",
            }
        }),
    )
}

/// Emit a provider enable event.
/// Caller must provide auth_token without prompting.
pub fn provider_enabled(
    gateway_url: &str,
    auth_token: &str,
    org_id: &str,
    provider_name: &str,
) -> Result<()> {
    emit(
        gateway_url,
        auth_token,
        org_id,
        "admin.config.changed",
        &format!("Provider '{}' enabled", provider_name),
        json!({
            "action": "provider_enabled",
            "provider": {
                "name": provider_name,
                "newState": "on",
            }
        }),
    )
}

/// Emit a provider disable event.
/// Caller must provide auth_token without prompting.
pub fn provider_disabled(
    gateway_url: &str,
    auth_token: &str,
    org_id: &str,
    provider_name: &str,
) -> Result<()> {
    emit(
        gateway_url,
        auth_token,
        org_id,
        "admin.config.changed",
        &format!("Provider '{}' disabled", provider_name),
        json!({
            "action": "provider_disabled",
            "provider": {
                "name": provider_name,
                "newState": "off",
            }
        }),
    )
}

/// Emit an API key saved/updated event.
/// Caller must provide auth_token without prompting.
pub fn api_key_saved(
    gateway_url: &str,
    auth_token: &str,
    org_id: &str,
    old_prefix: Option<&str>,
    new_prefix: &str,
) -> Result<()> {
    emit(
        gateway_url,
        auth_token,
        org_id,
        "admin.config.changed",
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

/// Emit an API key cleared event.
/// Caller must provide auth_token without prompting.
pub fn api_key_cleared(
    gateway_url: &str,
    auth_token: &str,
    org_id: &str,
    old_prefix: &str,
) -> Result<()> {
    emit(
        gateway_url,
        auth_token,
        org_id,
        "admin.config.changed",
        "Gateway API key cleared",
        json!({
            "action": "api_key_cleared",
            "api_key": {
                "previousPrefix": old_prefix,
            }
        }),
    )
}
