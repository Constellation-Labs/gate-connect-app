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

/// Max characters stored in a single credential entry. Windows Credential
/// Manager caps a credential blob at 2560 bytes, and `keyring` UTF-16-encodes
/// the value before that check (2 bytes/char for ASCII), so the real budget is
/// ~1280 ASCII chars. 1024 chars is ~2048 bytes, comfortably under the cap.
/// Values longer than this are transparently split across chunk entries; see
/// [`set`]. macOS Keychain and Secret Service have far larger limits, so this
/// only ever trips on Windows - but the split path runs on every platform so it
/// stays exercised in tests.
const MAX_CHUNK_CHARS: usize = 1024;

/// Sentinel prefixing the value stored at the base account when a secret is
/// split across chunk entries; the rest of the value is the chunk count. A raw
/// NUL byte never appears in JSON produced by `serde_json` (it escapes to
/// `\u0000`) nor in an API key or PEM, so a value starting with this marker
/// unambiguously identifies a chunk manifest and can't collide with a real
/// secret.
const CHUNK_MARKER: &str = "\u{0}gck-chunks\u{0}";

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

/// Write one value to one credential entry, no chunking. All three backends
/// (file-seam, in-memory, native OS store) live here.
fn set_raw(service: &str, account: &str, value: &str) -> Result<()> {
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

/// Read one credential entry, no chunk reassembly. `Ok(None)` when absent.
fn get_raw(service: &str, account: &str) -> Result<Option<String>> {
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

/// Delete one credential entry, no chunk cleanup. `Ok(false)` when absent.
fn delete_raw(service: &str, account: &str) -> Result<bool> {
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

/// Split `value` into pieces of at most `max_chars` chars each, never splitting a
/// multibyte char. Concatenating the pieces in order reproduces `value` exactly.
fn split_chunks(value: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut count = 0;
    for ch in value.chars() {
        current.push(ch);
        count += 1;
        if count == max_chars {
            chunks.push(std::mem::take(&mut current));
            count = 0;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Account name for the `i`th chunk of a chunked secret. Printable and NUL-free
/// so it's a valid native target name and folds cleanly in the file-backed test
/// seam; a collision would need a real account literally ending `#gck-chunk#N`.
fn chunk_account(account: &str, i: usize) -> String {
    format!("{account}#gck-chunk#{i}")
}

/// If `value` is a chunk manifest, return its chunk count; otherwise `None`.
fn parse_manifest(value: &str) -> Option<usize> {
    value.strip_prefix(CHUNK_MARKER)?.parse().ok()
}

/// Delete the secret at `(service, account)`, whether stored as a single entry
/// or a chunk manifest plus its chunks. Returns whether anything was present.
fn remove(service: &str, account: &str) -> Result<bool> {
    if let Some(n) = get_raw(service, account)?
        .as_deref()
        .and_then(parse_manifest)
    {
        for i in 0..n {
            delete_raw(service, &chunk_account(account, i))?;
        }
    }
    delete_raw(service, account)
}

pub fn set(service: &str, account: &str, value: &str) -> Result<()> {
    // Clear any prior value first so a shrinking chunk count - or a single->chunk
    // (or chunk->single) transition - never leaves orphaned chunk entries behind.
    remove(service, account)?;
    if value.chars().count() <= MAX_CHUNK_CHARS {
        return set_raw(service, account, value);
    }
    let chunks = split_chunks(value, MAX_CHUNK_CHARS);
    for (i, chunk) in chunks.iter().enumerate() {
        set_raw(service, &chunk_account(account, i), chunk)?;
    }
    // Write the manifest last: until it exists a torn write reads as "no secret"
    // rather than a manifest pointing at chunks that aren't all there yet.
    set_raw(service, account, &format!("{CHUNK_MARKER}{}", chunks.len()))
}

pub fn get(service: &str, account: &str) -> Result<Option<String>> {
    let Some(value) = get_raw(service, account)? else {
        return Ok(None);
    };
    let Some(n) = parse_manifest(&value) else {
        return Ok(Some(value));
    };
    let mut assembled = String::new();
    for i in 0..n {
        let chunk = get_raw(service, &chunk_account(account, i))?.with_context(|| {
            format!("chunk {i} of {n} missing for keyring entry {service}/{account}")
        })?;
        assembled.push_str(&chunk);
    }
    Ok(Some(assembled))
}

pub fn delete(service: &str, account: &str) -> Result<bool> {
    remove(service, account)
}

pub fn tool_service(tool: &str, label: &str) -> String {
    format!("{SERVICE_PREFIX}.{tool}.{label}")
}

pub fn account_service(label: &str) -> String {
    format!("{SERVICE_PREFIX}.account.{label}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_chunks_reassembles_exactly() {
        for value in [
            "",
            "a",
            "abc",
            &"x".repeat(1024),
            &"y".repeat(1025),
            &"z".repeat(3000),
        ] {
            let chunks = split_chunks(value, MAX_CHUNK_CHARS);
            assert_eq!(
                chunks.concat(),
                value,
                "round-trip failed for len {}",
                value.len()
            );
            assert!(
                chunks.iter().all(|c| c.chars().count() <= MAX_CHUNK_CHARS),
                "a chunk exceeded MAX_CHUNK_CHARS"
            );
        }
    }

    #[test]
    fn split_chunks_counts_are_right() {
        assert_eq!(split_chunks("", 4).len(), 0);
        assert_eq!(split_chunks("abcd", 4).len(), 1); // exact multiple stays one chunk
        assert_eq!(split_chunks("abcde", 4).len(), 2); // remainder spills to a second
        assert_eq!(split_chunks(&"a".repeat(12), 4).len(), 3);
    }

    #[test]
    fn split_chunks_never_splits_a_multibyte_char() {
        // 5 emoji, chunk size 2 -> 3 chunks (2, 2, 1) with every char intact.
        let value = "😀😀😀😀😀";
        let chunks = split_chunks(value, 2);
        assert_eq!(chunks.concat(), value);
        assert!(chunks.iter().all(|c| c.chars().all(|ch| ch == '😀')));
    }

    #[test]
    fn parse_manifest_recognizes_only_the_marker() {
        assert_eq!(parse_manifest(&format!("{CHUNK_MARKER}3")), Some(3));
        assert_eq!(parse_manifest(r#"{"access_token":"x"}"#), None);
        assert_eq!(parse_manifest("sk-gw-plain-key"), None);
        assert_eq!(parse_manifest(CHUNK_MARKER), None); // marker with no count
    }

    #[test]
    fn chunk_account_is_distinct_and_ordered() {
        assert_eq!(chunk_account("gclar", 0), "gclar#gck-chunk#0");
        assert_ne!(chunk_account("gclar", 0), chunk_account("gclar", 1));
    }
}
