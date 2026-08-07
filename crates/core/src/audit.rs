//! Audit event emission to the dashboard API.
//!
//! Gate Connect emits audit events whenever state changes:
//! - Proxy enable/disable
//! - Provider enable/disable
//! - API key changes
//!
//! Events are sent to `POST /v1/audit/emit` on the dashboard API.
//! Best-effort: failures are logged but don't block user operations.
//!
//! Auth: In API key mode, pass the API key as the bearer token.
//! In OAuth mode, pass the current access token (from oauth::access_token_for_injection()).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Emit an audit event to the dashboard API.
///
/// `auth_token`: The bearer token (API key in API-key mode, access token in OAuth mode).
/// Must not be empty - the endpoint requires authentication.
///
/// Returns Ok on success (event recorded).
/// On failure, logs the error but doesn't throw (best-effort).
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

    let client = reqwest::blocking::Client::new();
    let url = format!("{}/v1/audit/emit", gateway_url.trim_end_matches('/'));

    let request = json!({
        "eventType": event_type,
        "message": message,
        "data": data,
    });

    client
        .post(&url)
        .header("X-Org-Id", org_id)
        .bearer_auth(auth_token)
        .json(&request)
        .send()
        .context("sending audit emit request")?;

    Ok(())
}

/// Emit a proxy enable event.
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
                "previousState": "off",
                "newState": "on",
                "port": port,
            }
        }),
    )
}

/// Emit a proxy disable event.
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
                "previousState": "on",
                "newState": "off",
            }
        }),
    )
}

/// Emit a provider enable event.
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
                "previousState": "off",
                "newState": "on",
            }
        }),
    )
}

/// Emit a provider disable event.
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
                "previousState": "on",
                "newState": "off",
            }
        }),
    )
}

/// Emit an API key saved/updated event.
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
