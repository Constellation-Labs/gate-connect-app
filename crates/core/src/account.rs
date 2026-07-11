//! Single Gate AI account configured once and reused for every tool the
//! user connects. Gateway base URL lives in a small JSON file; the Gate
//! API key (sk-gw-...) lives in the macOS keychain.
//!
//! Upstream-provider auth (e.g. the Anthropic OAuth bearer Cowork
//! holds after sign-in) is *not* stored here - Cowork manages that
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

/// How Gate Connect authenticates to the gateway. `OAuth` sends a Cognito
/// access token on `x-gate-authorization`; `ApiKey` sends the pasted
/// `sk-gw-...` on `x-gate-api-key` (the legacy path). Defaults to `ApiKey` so
/// installs predating this field load unchanged (mirrors the PAC back-compat
/// default in `system_proxy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    #[default]
    ApiKey,
    // `rename_all` would snake-case this to `o_auth`; pin the wire value the
    // frontend and CLI expect.
    #[serde(rename = "oauth")]
    OAuth,
}

pub struct Account {
    pub gateway_base_url: String,
    pub api_key: String,
    pub auth_mode: AuthMode,
}

#[derive(Serialize, Deserialize)]
struct AccountFile {
    gateway_base_url: String,
    /// Leading characters of the stored Gate key - enough to identify *which*
    /// key is in use without revealing the secret. Stored here so the Settings
    /// reveal can read it from disk instead of touching the keychain. Absent in
    /// files written before this field existed, and while the account is in a
    /// key-less pending state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    api_key_prefix: Option<String>,
    /// Which credential Gate Connect sends to the gateway. Absent in files
    /// written before OAuth support existed; `#[serde(default)]` loads those
    /// as `ApiKey`, preserving the legacy behavior.
    #[serde(default)]
    auth_mode: AuthMode,
    /// Selected organization (OAuth mode only). `org_id` is the UUID injected
    /// on `X-Gate-Org-Id`; `org_name` is cached for display. Non-secret. Absent
    /// until the user picks an org after signing in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    org_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    org_name: Option<String>,
}

fn config_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join("account.json"))
}

pub fn service() -> String {
    keychain::account_service(KEYCHAIN_LABEL)
}

/// Load the account if it's usable. The gateway URL (on disk) is always
/// required. The key requirement depends on the auth mode: in the legacy
/// `ApiKey` mode a Gate key in the keychain is required; in `OAuth` mode the
/// Cognito token is the credential, so a missing key is expected and loads as
/// an empty string . Missing what's required returns None.
pub fn load() -> Result<Option<Account>> {
    let Some(file) = read_account_file()? else {
        return Ok(None);
    };
    let user = env::current_user()?;
    let stored_key = keychain::get(&service(), &user)?;
    let api_key = match (file.auth_mode, stored_key) {
        (_, Some(key)) => key,
        (AuthMode::OAuth, None) => String::new(),
        (AuthMode::ApiKey, None) => return Ok(None),
    };
    Ok(Some(Account {
        gateway_base_url: file.gateway_base_url,
        api_key,
        auth_mode: file.auth_mode,
    }))
}

/// Same as [`load`] but returns only the gateway URL, without touching
/// the keychain. Used by the UI to show "you're signed in" state
/// without triggering an authorization prompt to read the secret.
pub fn load_base_url() -> Result<Option<String>> {
    Ok(read_account_file()?.map(|f| f.gateway_base_url))
}

/// Read and parse `account.json`, or `None` when no account is on disk. The
/// on-disk half of the account (gateway URL + key prefix) that both the UI
/// state helpers and [`save`] read without touching the keychain.
fn read_account_file() -> Result<Option<AccountFile>> {
    let path = config_path()?;
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let parsed: AccountFile = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {} as JSON", path.display()))?;
    Ok(Some(parsed))
}

/// Persist account state.
///
/// `api_key = Some(value)` writes the key to keychain (creating or
/// rotating) and records its prefix in `account.json`. `api_key = None`
/// leaves any existing keychain entry - and the stored prefix - untouched,
/// used by the "edit account" form so the user can update only the base URL
/// without re-entering their key.
pub fn save(gateway_base_url: &str, api_key: Option<&str>) -> Result<()> {
    if gateway_base_url.len() > 2048 {
        anyhow::bail!("gateway base URL is unexpectedly long (>2048 bytes)");
    }
    if !gateway_base_url.starts_with("https://") {
        anyhow::bail!("gateway base URL must be https://");
    }
    // Recompute the prefix from a new key; otherwise preserve the one already
    // on disk so a URL-only edit doesn't drop it. The auth mode is likewise
    // preserved - it's chosen via [`set_auth_mode`], not by saving a URL/key.
    let existing = read_account_file()?;
    let api_key_prefix = match api_key {
        Some(key) => Some(key.chars().take(12).collect()),
        None => existing.as_ref().and_then(|f| f.api_key_prefix.clone()),
    };
    // Preserve the auth mode and selected org - both are chosen via their own
    // setters, not by saving a URL/key.
    let auth_mode = existing.as_ref().map(|f| f.auth_mode).unwrap_or_default();
    let org_id = existing.as_ref().and_then(|f| f.org_id.clone());
    let org_name = existing.and_then(|f| f.org_name.clone());
    write_account_file(&AccountFile {
        gateway_base_url: gateway_base_url.to_string(),
        api_key_prefix,
        auth_mode,
        org_id,
        org_name,
    })?;

    if let Some(key) = api_key {
        let user = env::current_user()?;
        keychain::set(&service(), &user, key)?;
    }
    Ok(())
}

/// Serialize `file` to `account.json` with owner-only permissions. The single
/// on-disk writer, paired with [`read_account_file`].
fn write_account_file(file: &AccountFile) -> Result<()> {
    let path = config_path()?;
    let mut json = serde_json::to_string_pretty(file).context("serializing account.json")?;
    json.push('\n');
    primitives::write_file(&path, json.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

/// Dev-mode environment switch: point the account at a different gateway and
/// forget the current Gate key so the user must enter an environment-appropriate
/// one. Managed tools are disconnected by the command layer first (same as
/// [`clear`]), since their config embeds the old gateway+key.
pub fn switch_gateway(gateway_base_url: &str) -> Result<()> {
    save(gateway_base_url, None)?; // new URL on disk, key untouched
    let user = env::current_user()?;
    keychain::delete(&service(), &user)?; // forget the old key
    let auth_mode = read_account_file()?
        .map(|f| f.auth_mode)
        .unwrap_or_default();
    // The stored prefix named the key we just deleted, so drop it too. The org
    // is environment-specific, so a gateway switch clears it - the user re-picks
    // against the new environment after re-authenticating.
    write_account_file(&AccountFile {
        gateway_base_url: gateway_base_url.to_string(),
        api_key_prefix: None,
        auth_mode,
        org_id: None,
        org_name: None,
    })?;
    Ok(())
}

/// Persist the selected organization (OAuth mode). `org_id` is the UUID
/// injected on `X-Gate-Org-Id`; `org_name` is cached for display. Requires an
/// existing `account.json`.
pub fn set_org(org_id: &str, org_name: &str) -> Result<()> {
    let mut file = read_account_file()?.context("no account configured")?;
    file.org_id = Some(org_id.to_string());
    file.org_name = Some(org_name.to_string());
    write_account_file(&file)
}

/// The currently selected `(org_id, org_name)`, or `None` if the user hasn't
/// picked one yet. Cheap disk read; never touches the keychain.
pub fn selected_org() -> Result<Option<(String, String)>> {
    Ok(
        read_account_file()?.and_then(|f| match (f.org_id, f.org_name) {
            (Some(id), Some(name)) => Some((id, name)),
            (Some(id), None) => Some((id.clone(), id)),
            _ => None,
        }),
    )
}

/// The org id to inject on `X-Gate-Org-Id` right now, or an empty string when
/// none is selected. The single source of truth the proxy managers seed the
/// engine/relay from (mirrors [`crate::oauth::access_token_for_injection`]).
pub fn org_id_for_injection() -> String {
    read_account_file()
        .ok()
        .flatten()
        .and_then(|f| f.org_id)
        .unwrap_or_default()
}

/// Switch the persisted auth mode, preserving the base URL and key prefix. The
/// OAuth sign-in flow sets `OAuth`; pasting a Gate key sets `ApiKey`. Requires
/// an existing `account.json` .
pub fn set_auth_mode(mode: AuthMode) -> Result<()> {
    let mut file = read_account_file()?.context("no account configured")?;
    file.auth_mode = mode;
    write_account_file(&file)
}

/// Current persisted auth mode, defaulting to `ApiKey` when no account exists
/// yet. A cheap disk read that never touches the keychain.
pub fn auth_mode() -> Result<AuthMode> {
    Ok(read_account_file()?
        .map(|f| f.auth_mode)
        .unwrap_or_default())
}

pub fn has_api_key() -> Result<bool> {
    let user = env::current_user()?;
    Ok(keychain::get(&service(), &user)?.is_some())
}

/// Leading characters of the stored Gate key - through the random part that
/// distinguishes one key from another - so the UI can show *which* key is in
/// use without revealing the secret. Reads the prefix recorded in
/// `account.json` by [`save`], so it never touches the keychain and never
/// prompts. Returns `None` when no key is stored, or when the account predates
/// the stored-prefix field .
pub fn api_key_prefix() -> Result<Option<String>> {
    Ok(read_account_file()?.and_then(|f| f.api_key_prefix))
}

/// Fallback reveal for accounts saved before the prefix was recorded on disk:
/// read the key from the keychain (which may trigger an OS authorization
/// prompt), record its prefix in `account.json` so later reveals are free, and
/// return it. Gated behind an explicit user confirmation in the UI because of
/// the keychain read. Returns `None` when no key is stored.
pub fn backfill_api_key_prefix() -> Result<Option<String>> {
    let user = env::current_user()?;
    let Some(key) = keychain::get(&service(), &user)? else {
        return Ok(None);
    };
    let prefix: String = key.chars().take(12).collect();
    if let Some(mut file) = read_account_file()? {
        file.api_key_prefix = Some(prefix.clone());
        write_account_file(&file)?;
    }
    Ok(Some(prefix))
}

pub fn clear() -> Result<()> {
    let path = config_path()?;
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    }
    let user = env::current_user()?;
    keychain::delete(&service(), &user)?;
    // A full disconnect forgets every credential, so any OAuth tokens go too -
    // nothing is left behind in the secret store .
    crate::oauth::clear()?;
    Ok(())
}

/// Reconcile the account's two halves at startup, dropping an orphaned Gate key
/// left in the OS keychain when an uninstall removed Gate Connect's on-disk
/// files but couldn't touch the keychain. macOS drag-to-trash - and deep
/// uninstallers that purge `~/Library/Application Support` - leave exactly this
/// kind of orphan, and a stale Gate key sitting in the keychain with no account
/// behind it is what this clears.
///
/// `account.json` (gateway URL on disk) is the anchor; the Gate key in keychain
/// is the secret. We only act on the keychain-orphan half, so a signed-in user
/// (both halves present) is never touched:
/// - URL gone, key present → orphaned Gate key → delete it.
/// - URL gone → any orphaned OAuth tokens are equally stranded → forget them
///   . `oauth::clear` is idempotent when none exist.
///
/// A key-less `account.json` (URL present, no key) is *not* reconciled: it's a
/// legitimate pending-key state - a fresh [`switch_gateway`], or a reinstall
/// orphan - and the app routes it to key entry pointed at that gateway.
pub fn reconcile() -> Result<()> {
    let has_url = load_base_url()?.is_some();
    let has_key = has_api_key()?;
    if !has_url && has_key {
        let user = env::current_user()?;
        keychain::delete(&service(), &user)?;
    }
    if !has_url {
        crate::oauth::clear()?;
    }
    Ok(())
}
