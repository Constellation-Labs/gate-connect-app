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

/// The chat surfaces that only exist while the account points at staging.
///
/// Both intercept a session credential rather than a brokered key, and the
/// gateway-side classification they depend on (`CLAUDE_WEB_CHAT_COMPLETION_RE`
/// and the ChatGPT anchors beside it) is deployed to staging ahead of
/// production. Offering the row where the gateway would mis-classify the turn
/// is worse than not offering it: the user flips a switch that says Gate
/// records and inspects this traffic, and gets traffic tagged as something
/// else.
///
/// These are slugs, not hosts, so the gate covers exactly the two entries this
/// branch added and leaves `chatgpt` - the relay-only subscription endpoint,
/// which shipped before it - alone.
const STAGING_ONLY_SLUGS: &[&str] = &["claude-web", "chatgpt-apps"];

/// The built-in catalog with the staging gate applied, which is the catalog
/// every path through this module reads.
///
/// The gate rides `supported`, deliberately, rather than being a third state:
/// that flag already means "Gate cannot upstream this today", already forces
/// `enabled` off in [`load_domains`], already makes [`set_enabled`] refuse, and
/// is already what `buildGroups` (src/lib/groups.ts) filters a row out on. So a
/// user who enabled one of these on staging and switched to production gets the
/// domain switched off and the row gone, rather than a hidden row still
/// intercepting claude.ai - the one outcome a UI-level gate could not give.
///
/// Nothing re-reads this while an engine is up, and nothing needs to: the
/// watcher in `manager` reloads on the DOMAINS file's mtime, and an account
/// switch does not touch that file. The `switch_gateway` command shuts the
/// engine down before the account moves (it pins the gateway URL at start) and
/// the app relaunches, so the next `load_domains` is the first one that could
/// see the new environment.
fn gated_catalog() -> Vec<ProxyDomain> {
    let staging = crate::account::gateway_is_staging();
    let mut domains = default_domains();
    if !staging {
        for d in &mut domains {
            if STAGING_ONLY_SLUGS.contains(&d.slug.as_str()) {
                d.supported = false;
            }
        }
    }
    domains
}

/// The full domain catalog with persisted enabled flags applied. An
/// unsupported domain is always returned disabled regardless of what's on
/// disk - Gate can't upstream it yet, so it must not route, and that covers
/// both a provider Gate has no support for and one gated to staging (see
/// [`gated_catalog`]).
pub fn load_domains() -> Result<Vec<ProxyDomain>> {
    let overrides = read_file()?.enabled;
    let mut domains = gated_catalog();
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
/// unsupported domain, including one gated to staging. Returns the updated
/// catalog.
pub fn set_enabled(slug: &str, enabled: bool) -> Result<Vec<ProxyDomain>> {
    let catalog = gated_catalog();
    let target = catalog
        .iter()
        .find(|d| d.slug == slug)
        .with_context(|| format!("unknown proxy domain {slug:?}"))?;
    if enabled && !target.supported {
        // Two reasons a domain can be unsupported, and they send the reader
        // somewhere different: "not yet" is Gate's roadmap, while the staging
        // gate is one setting away. A CLI user who typed the slug out of the
        // docs gets told which one they hit.
        if STAGING_ONLY_SLUGS.contains(&slug) {
            anyhow::bail!(
                "proxy domain {slug:?} is only available while the account points at {} \
                 (Settings > Dev mode)",
                crate::account::STAGING_GATEWAY_HOST
            );
        }
        anyhow::bail!("proxy domain {slug:?} is not supported yet");
    }

    let mut file = read_file()?;
    file.enabled.insert(slug.to_string(), enabled);
    write_file(&file)?;
    load_domains()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Root every per-user path at a throwaway dir for the closure's duration.
    /// `config_path` resolves through `app_support_dir`, whose test seam is the
    /// process-global `GATE_CONNECT_TEST_HOME` env var, so these serialize
    /// against every other path-redirecting test via `path_env_lock` - see the
    /// twin helper in `system_proxy_linux.rs` for the full rationale.
    ///
    /// The unsupported-forced-off arm of `load_domains` / `set_enabled` is not
    /// driven here: the built-in catalog ships every entry `supported:true`,
    /// and the module reads `default_domains()` directly (no catalog seam), so
    /// there is no unsupported entry to persist against.
    fn with_temp_env<T>(f: impl FnOnce() -> T) -> T {
        let _lock = crate::env::path_env_lock();
        let tmp =
            std::env::temp_dir().join(format!("gate-proxy-config-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let prev_home = std::env::var_os("GATE_CONNECT_TEST_HOME");
        std::env::set_var("GATE_CONNECT_TEST_HOME", &tmp);
        let out = f();
        match prev_home {
            Some(v) => std::env::set_var("GATE_CONNECT_TEST_HOME", v),
            None => std::env::remove_var("GATE_CONNECT_TEST_HOME"),
        }
        let _ = fs::remove_dir_all(&tmp);
        out
    }

    #[test]
    fn missing_file_yields_the_catalog_defaults() {
        with_temp_env(|| {
            // First run: no domains file on disk is not an error, it is the
            // shipped catalog verbatim (anthropic on, the rest opt-in).
            let domains = load_domains().unwrap();
            let defaults = default_domains();
            assert_eq!(domains.len(), defaults.len());
            for (got, want) in domains.iter().zip(&defaults) {
                assert_eq!(got.slug, want.slug);
                assert_eq!(got.enabled, want.enabled, "default for {:?}", got.slug);
            }
        });
    }

    #[test]
    fn set_enabled_persists_across_a_fresh_load() {
        with_temp_env(|| {
            // The returned catalog reflects the flip immediately...
            let returned = set_enabled("openai", true).unwrap();
            assert!(
                returned
                    .iter()
                    .find(|d| d.slug == "openai")
                    .unwrap()
                    .enabled
            );
            // ...and so does a re-read from disk, including turning the
            // default-on entry off durably.
            set_enabled("anthropic", false).unwrap();
            let domains = load_domains().unwrap();
            assert!(domains.iter().find(|d| d.slug == "openai").unwrap().enabled);
            assert!(
                !domains
                    .iter()
                    .find(|d| d.slug == "anthropic")
                    .unwrap()
                    .enabled
            );
        });
    }

    #[test]
    fn unknown_slug_is_refused_either_way() {
        with_temp_env(|| {
            // The catalog is the source of truth for the domain set; a slug it
            // doesn't know can be neither enabled nor disabled.
            let err = set_enabled("no-such-provider", true).unwrap_err();
            assert!(err.to_string().contains("no-such-provider"), "{err}");
            assert!(set_enabled("no-such-provider", false).is_err());
        });
    }

    #[test]
    fn stale_persisted_slug_is_ignored() {
        with_temp_env(|| {
            // A flag persisted by a build whose catalog had (or will have) a
            // slug this one doesn't: it must neither error nor surface a
            // phantom domain, and the flags for known slugs still apply.
            let path = config_path().unwrap();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, r#"{"enabled":{"ghost":true,"openai":true}}"#).unwrap();
            let domains = load_domains().unwrap();
            assert!(domains.iter().all(|d| d.slug != "ghost"));
            assert!(domains.iter().find(|d| d.slug == "openai").unwrap().enabled);
        });
    }

    #[test]
    fn torn_file_errors_rather_than_resetting() {
        with_temp_env(|| {
            // Writes are atomic, so a half-written file means something else
            // went wrong; surfacing an error beats silently reverting the
            // user's toggles to defaults. And `set_enabled` must not "fix" it
            // by clobbering the evidence.
            let path = config_path().unwrap();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, r#"{"enabled":{"open"#).unwrap();
            assert!(load_domains().is_err());
            assert!(set_enabled("openai", true).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), r#"{"enabled":{"open"#);
        });
    }
}
