//! Persists which proxy domains the user has enabled. The built-in catalog
//! ([`default_domains`]) is the source of truth for the domain set; this
//! file only records per-slug enabled flags, so adding a new built-in
//! domain in a future release surfaces it automatically with its default
//! state. Structured to be swappable for a Gate-served registry later.
//!
//! [`default_domains`]: crate::proxy::default_domains

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::env;
use crate::proxy::{default_domains, ProxyDomain};

#[derive(Default, Serialize, Deserialize)]
struct DomainsFile {
    /// slug -> enabled. Only entries that differ from / pin the default are
    /// strictly needed, but we persist the full set we know about.
    #[serde(default)]
    enabled: HashMap<String, bool>,
}

fn config_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join("proxy").join("domains.json"))
}

/// Last-modified time of the domains file, or `None` when it does not exist
/// yet or cannot be stat'd.
///
/// For watchers that need to notice a write made by *another process*: a
/// second `gate-connect` invocation toggling a domain writes this file, and
/// whoever is hosting the engine has no other way to learn of it. Deliberately
/// a stat rather than a full parse, so polling it costs nothing.
/// `test` included so the generic desktop manager (`manager_core`) and its
/// tests compile on every platform.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
pub(crate) fn domains_file_mtime() -> Option<std::time::SystemTime> {
    fs::metadata(config_path().ok()?).ok()?.modified().ok()
}

fn read_file() -> Result<DomainsFile> {
    let path = config_path()?;
    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw)
            .with_context(|| format!("parsing {} as JSON", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DomainsFile::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn write_file(file: &DomainsFile) -> Result<()> {
    let path = config_path()?;
    let raw = serde_json::to_string_pretty(file).context("serializing proxy domains config")?;
    // Atomic write (handles parent dirs too): a crash mid-write must not
    // leave a corrupt domains file behind.
    crate::primitives::write_file(&path, raw.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

/// The full domain catalog with persisted enabled flags applied. An
/// unsupported domain is always returned disabled regardless of what's on
/// disk - Gate can't upstream it yet, so it must not route.
pub fn load_domains() -> Result<Vec<ProxyDomain>> {
    let overrides = read_file()?.enabled;
    let mut domains = default_domains();
    for d in &mut domains {
        if let Some(&enabled) = overrides.get(&d.slug) {
            d.enabled = enabled;
        }
        if !d.supported {
            d.enabled = false;
        }
    }
    Ok(domains)
}

/// Flip a domain's enabled flag and persist. Refuses to enable an
/// unsupported domain. Returns the updated catalog.
pub fn set_enabled(slug: &str, enabled: bool) -> Result<Vec<ProxyDomain>> {
    let catalog = default_domains();
    let target = catalog
        .iter()
        .find(|d| d.slug == slug)
        .with_context(|| format!("unknown proxy domain {slug:?}"))?;
    if enabled && !target.supported {
        anyhow::bail!("proxy domain {slug:?} is not supported yet");
    }

    let mut file = read_file()?;
    file.enabled.insert(slug.to_string(), enabled);
    write_file(&file)?;
    load_domains()
}
