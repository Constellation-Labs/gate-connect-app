//! Cross-platform secret-store wrapper. Backed by the `keyring` crate,
//! which routes to:
//!
//! - macOS Keychain (via `apple-native` → Security.framework)
//! - Windows Credential Manager (via `windows-native`)
//! - Linux Secret Service (via `sync-secret-service`, vendored libdbus)
//!
//! Every secret Gate Connect writes uses the same service prefix
//! (`ai.constellation.gate-connect.*`) so the user can audit / nuke
//! them with one query in their OS's native secret manager.

use anyhow::{Context, Result};
use keyring::Entry;

const SERVICE_PREFIX: &str = "ai.constellation.gate-connect";

pub fn set(service: &str, account: &str, value: &str) -> Result<()> {
    Entry::new(service, account)
        .with_context(|| format!("opening keyring entry for {service}/{account}"))?
        .set_password(value)
        .with_context(|| format!("writing keyring entry for {service}/{account}"))
}

pub fn get(service: &str, account: &str) -> Result<Option<String>> {
    let entry = Entry::new(service, account)
        .with_context(|| format!("opening keyring entry for {service}/{account}"))?;
    match entry.get_password() {
        Ok(s) => Ok(Some(s)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading keyring entry for {service}/{account}")),
    }
}

pub fn delete(service: &str, account: &str) -> Result<bool> {
    let entry = Entry::new(service, account)
        .with_context(|| format!("opening keyring entry for {service}/{account}"))?;
    match entry.delete_credential() {
        Ok(()) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(e).with_context(|| format!("deleting keyring entry for {service}/{account}")),
    }
}

pub fn tool_service(tool: &str, label: &str) -> String {
    format!("{SERVICE_PREFIX}.{tool}.{label}")
}

pub fn account_service(label: &str) -> String {
    format!("{SERVICE_PREFIX}.account.{label}")
}
