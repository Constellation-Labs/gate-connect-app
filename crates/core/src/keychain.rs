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
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Context, Result};
use keyring::Entry;

const SERVICE_PREFIX: &str = "ai.constellation.gate-connect";

/// Test seam: when `GATE_CONNECT_TEST_SECRETS` is set, back secrets with files
/// in that directory instead of the OS keychain. Unlike the in-memory backend
/// below, this works across a spawned `gate-connect` process (env vars are
/// inherited; a process-global mutex is not), which is what the CLI flow tests
/// need. Unset in production, so the real Keychain / Credential Manager /
/// Secret Service path is always used there.
fn test_secrets_dir() -> Option<PathBuf> {
    std::env::var_os("GATE_CONNECT_TEST_SECRETS")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// File-backed secret path for the test seam: one file per service/account,
/// with path-separator chars folded so the name is always a single segment.
fn secret_file(dir: &std::path::Path, service: &str, account: &str) -> PathBuf {
    let name = format!("{service}__{account}").replace(['/', '\\', ':'], "_");
    dir.join(name)
}

/// Optional process-global in-memory secret store, installed only by tests via
/// [`use_in_memory_backend`]. `None` in every normal build, so production always
/// hits the native OS secret store below. It exists because the OS keychain
/// (Secret Service / Keychain / Credential Manager) is unavailable in headless
/// CI and must never be touched by tests - and keyring's own mock store keeps
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
    if let Some(dir) = test_secrets_dir() {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating test secret store {}", dir.display()))?;
        let path = secret_file(&dir, service, account);
        return std::fs::write(&path, value)
            .with_context(|| format!("writing test secret {}", path.display()));
    }
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
    if let Some(dir) = test_secrets_dir() {
        let path = secret_file(&dir, service, account);
        return match std::fs::read_to_string(&path) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("reading test secret {}", path.display())),
        };
    }
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
    if let Some(dir) = test_secrets_dir() {
        let path = secret_file(&dir, service, account);
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e).with_context(|| format!("deleting test secret {}", path.display())),
        };
    }
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
