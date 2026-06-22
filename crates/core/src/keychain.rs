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

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{Context, Result};
use keyring::Entry;

const SERVICE_PREFIX: &str = "ai.constellation.gate-connect";

/// Optional process-global in-memory secret store, installed only by tests via
/// [`use_in_memory_backend`]. `None` in every normal build, so production always
/// hits the native OS secret store below. It exists because the OS keychain
/// (Secret Service / Keychain / Credential Manager) is unavailable in headless
/// CI and must never be touched by tests — and keyring's own mock store keeps
/// state in the `Entry`, not across `Entry::new` calls, so it can't model
/// "a secret already exists" the way our get-after-set code paths need.
static IN_MEMORY: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// Route all keychain operations through a fresh process-global in-memory map
/// for the rest of this process. Test-only seam; never called in production.
/// Calling it again resets the map, so each test can start from empty.
#[doc(hidden)]
pub fn use_in_memory_backend() {
    *IN_MEMORY.lock().expect("in-memory keychain mutex poisoned") = Some(HashMap::new());
}

fn mem_key(service: &str, account: &str) -> String {
    format!("{service}\u{0}{account}")
}

pub fn set(service: &str, account: &str, value: &str) -> Result<()> {
    {
        let mut guard = IN_MEMORY.lock().expect("in-memory keychain mutex poisoned");
        if let Some(map) = guard.as_mut() {
            map.insert(mem_key(service, account), value.to_string());
            return Ok(());
        }
    }
    Entry::new(service, account)
        .with_context(|| format!("opening keyring entry for {service}/{account}"))?
        .set_password(value)
        .with_context(|| format!("writing keyring entry for {service}/{account}"))
}

pub fn get(service: &str, account: &str) -> Result<Option<String>> {
    {
        let guard = IN_MEMORY.lock().expect("in-memory keychain mutex poisoned");
        if let Some(map) = guard.as_ref() {
            return Ok(map.get(&mem_key(service, account)).cloned());
        }
    }
    let entry = Entry::new(service, account)
        .with_context(|| format!("opening keyring entry for {service}/{account}"))?;
    match entry.get_password() {
        Ok(s) => Ok(Some(s)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading keyring entry for {service}/{account}")),
    }
}

pub fn delete(service: &str, account: &str) -> Result<bool> {
    {
        let mut guard = IN_MEMORY.lock().expect("in-memory keychain mutex poisoned");
        if let Some(map) = guard.as_mut() {
            return Ok(map.remove(&mem_key(service, account)).is_some());
        }
    }
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
