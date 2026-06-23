//! Single Gate AI account configured once and reused for every tool the
//! user connects. Gateway base URL lives in a small JSON file; the Gate
//! API key (sk-gw-...) lives in the macOS keychain.
//!
//! Upstream-provider auth (e.g. the Anthropic OAuth bearer Cowork
//! holds after sign-in) is *not* stored here — Cowork manages that
//! itself. Gate Connect's job is to point Cowork at Gate and supply the
//! workspace identifier via `X-Gate-Api-Key`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::env;
use crate::keychain;
use crate::primitives;

const KEYCHAIN_LABEL: &str = "gateway-api-key";

pub struct Account {
    pub gateway_base_url: String,
    pub api_key: String,
}

#[derive(Serialize, Deserialize)]
struct AccountFile {
    gateway_base_url: String,
}

fn config_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join("account.json"))
}

pub fn service() -> String {
    keychain::account_service(KEYCHAIN_LABEL)
}

/// Load the account if both halves (base URL on disk + Gate key in
/// keychain) are present. Missing either half returns None.
pub fn load() -> Result<Option<Account>> {
    let Some(base_url) = load_base_url()? else {
        return Ok(None);
    };
    let user = env::current_user()?;
    let Some(api_key) = keychain::get(&service(), &user)? else {
        return Ok(None);
    };
    Ok(Some(Account {
        gateway_base_url: base_url,
        api_key,
    }))
}

/// Same as [`load`] but returns only the gateway URL, without touching
/// the keychain. Used by the UI to show "you're signed in" state
/// without triggering an authorization prompt to read the secret.
pub fn load_base_url() -> Result<Option<String>> {
    let path = config_path()?;
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let parsed: AccountFile = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {} as JSON", path.display()))?;
    Ok(Some(parsed.gateway_base_url))
}

/// Persist account state.
///
/// `api_key = Some(value)` writes the key to keychain (creating or
/// rotating). `api_key = None` leaves any existing keychain entry
/// untouched — used by the "edit account" form so the user can update
/// only the base URL without re-entering their key.
pub fn save(gateway_base_url: &str, api_key: Option<&str>) -> Result<()> {
    if gateway_base_url.len() > 2048 {
        anyhow::bail!("gateway base URL is unexpectedly long (>2048 bytes)");
    }
    if !gateway_base_url.starts_with("https://") {
        anyhow::bail!("gateway base URL must be https://");
    }
    let path = config_path()?;
    let file = AccountFile {
        gateway_base_url: gateway_base_url.to_string(),
    };
    let mut json = serde_json::to_string_pretty(&file).context("serializing account.json")?;
    json.push('\n');
    primitives::write_file(&path, json.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))?;

    if let Some(key) = api_key {
        let user = env::current_user()?;
        keychain::set(&service(), &user, key)?;
    }
    Ok(())
}

/// Dev-mode environment switch: point the account at a different gateway and
/// forget the current Gate key so the user must enter an environment-appropriate
/// one. Managed tools are disconnected by the command layer first (same as
/// [`clear`]), since their config embeds the old gateway+key.
pub fn switch_gateway(gateway_base_url: &str) -> Result<()> {
    save(gateway_base_url, None)?; // new URL on disk, key untouched
    let user = env::current_user()?;
    keychain::delete(&service(), &user)?; // forget the old key
    Ok(())
}

pub fn has_api_key() -> Result<bool> {
    let user = env::current_user()?;
    Ok(keychain::get(&service(), &user)?.is_some())
}

pub fn clear() -> Result<()> {
    let path = config_path()?;
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    }
    let user = env::current_user()?;
    keychain::delete(&service(), &user)?;
    Ok(())
}

/// Reconcile the account's two halves at startup, dropping an orphaned Gate key
/// left in the OS keychain when an uninstall removed Gate Connect's on-disk
/// files but couldn't touch the keychain. macOS drag-to-trash — and deep
/// uninstallers that purge `~/Library/Application Support` — leave exactly this
/// kind of orphan, and a stale Gate key sitting in the keychain with no account
/// behind it is what this clears.
///
/// `account.json` (gateway URL on disk) is the anchor; the Gate key in keychain
/// is the secret. We only act on the keychain-orphan half, so a signed-in user
/// (both halves present) is never touched:
/// - URL gone, key present → orphaned Gate key → delete it.
///
/// A key-less `account.json` (URL present, no key) is *not* reconciled: it's a
/// legitimate pending-key state — a fresh [`switch_gateway`], or a reinstall
/// orphan — and the app routes it to key entry pointed at that gateway.
pub fn reconcile() -> Result<()> {
    let has_url = load_base_url()?.is_some();
    let has_key = has_api_key()?;
    if !has_url && has_key {
        let user = env::current_user()?;
        keychain::delete(&service(), &user)?;
    }
    Ok(())
}
