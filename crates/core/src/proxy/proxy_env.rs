//! The environment-variable half of system-proxy wiring, shared by all three
//! platforms.
//!
//! The OS proxy setting and the proxy *environment variables* are two different
//! channels reaching two different populations. GUI apps and anything built on
//! the platform HTTP stack read the OS setting (a PAC URL on macOS/Windows).
//! Command-line AI tools mostly don't: they run on Node, Bun or Python, whose
//! HTTP clients look at `HTTPS_PROXY` / `HTTP_PROXY` / `NO_PROXY` and nothing
//! else. OpenCode is the clearest case - it has no config knob for a proxy or a
//! CA at all (nothing in its config schema), so these variables are the *only*
//! way to route it.
//!
//! Linux has always wired the env channel, because `environment.d` is how the
//! system proxy is expressed there in the first place. macOS and Windows wired
//! only the OS setting, so CLI tools stayed unrouted on those platforms. This
//! module is the single source of truth for the names and values, so the three
//! implementations can't drift.
//!
//! `NODE_EXTRA_CA_CERTS` rides along because it answers the same question for
//! the same population: Node and Bun ship their own trust bundle and ignore the
//! OS trust store, so trusting our CA system-wide is not enough for them. It is
//! *additive* to the bundle, unlike Python's `cafile`, so a single cert is
//! correct here and no synthesized bundle is needed (contrast
//! [`super::ca_bundle`], which exists for tools that replace the store).
//!
//! Note these point straight at the engine, not at a PAC: env-var proxies have
//! no PAC equivalent, so every request from a tool that honours them reaches the
//! engine, which MITMs the intercepted domains and blind-tunnels the rest. That
//! is exactly what Linux has always done.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

// `EnvSnapshot` and `prior_to_record` are macOS/Windows only: Linux reverts by
// deleting a drop-in it owns outright and so has nothing to restore. Compiled
// in under `test` everywhere as well, so the re-entry guard - the one piece
// here with a subtle failure mode - is covered wherever the suite runs.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
use std::collections::BTreeMap;

/// Keep loopback off the proxy. Required, not merely polite: OpenCode's TUI
/// talks to its own local HTTP server, and routing that through the engine
/// forms a loop (their own docs call this out).
const NO_PROXY_VALUE: &str = "localhost,127.0.0.1,::1";

/// The variables we manage on platforms whose environment is case-sensitive
/// (Linux, macOS), in a stable order. Both cases of the proxy trio are set
/// because tools disagree about which they read - curl wants lower-case, most
/// Node tooling upper-case.
pub(crate) const VARS_CASE_SENSITIVE: [&str; 7] = [
    "http_proxy",
    "https_proxy",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "no_proxy",
    "NO_PROXY",
    "NODE_EXTRA_CA_CERTS",
];

/// The variables we manage on Windows. Environment names there are
/// case-insensitive, so the lower-case aliases would be the *same* value rather
/// than a second one - writing both would just fight over one registry entry.
///
/// Compiled in under `test` too, so the "Windows is a subset of the others"
/// invariant is checked on every platform's test run rather than only Windows'.
#[cfg(any(target_os = "windows", test))]
pub(crate) const VARS_CASE_INSENSITIVE: [&str; 4] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "NODE_EXTRA_CA_CERTS",
];

/// Name/value pairs for an enabled proxy at `127.0.0.1:port`, keyed by
/// [`VARS_CASE_SENSITIVE`].
pub(crate) fn case_sensitive(port: u16) -> Result<Vec<(&'static str, String)>> {
    let endpoint = format!("http://127.0.0.1:{port}");
    let ca = super::ca_cert_path()?.display().to_string();
    let values = [
        endpoint.clone(),
        endpoint.clone(),
        endpoint.clone(),
        endpoint,
        NO_PROXY_VALUE.to_string(),
        NO_PROXY_VALUE.to_string(),
        ca,
    ];
    Ok(VARS_CASE_SENSITIVE.into_iter().zip(values).collect())
}

/// Name/value pairs for an enabled proxy at `127.0.0.1:port`, keyed by
/// [`VARS_CASE_INSENSITIVE`].
#[cfg(any(target_os = "windows", test))]
pub(crate) fn case_insensitive(port: u16) -> Result<Vec<(&'static str, String)>> {
    let endpoint = format!("http://127.0.0.1:{port}");
    let ca = super::ca_cert_path()?.display().to_string();
    let values = [endpoint.clone(), endpoint, NO_PROXY_VALUE.to_string(), ca];
    Ok(VARS_CASE_INSENSITIVE.into_iter().zip(values).collect())
}

/// Persisted answer to "may Gate put its proxy into your environment?".
///
/// These variables are machine-wide: `HTTPS_PROXY` redirects git, curl, npm and
/// everything else, not only the AI tools we care about. That is a big enough
/// change to be something the user holds an opinion about, so it is a choice
/// with a home on disk rather than a silent side effect of the routing switch.
///
/// Owned by the `env-proxy` integration; read by the platform managers so a user
/// who turned it off does not get it back on the next enable.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExportChoice {
    enabled: bool,
}

fn choice_path() -> Result<PathBuf> {
    Ok(crate::env::app_support_dir()?
        .join("proxy")
        .join("env-routing.json"))
}

/// Whether the user wants the proxy exported into their environment.
///
/// Defaults to **true**: the export is what routes the command-line tools, and
/// the routing switch has always implied it. Only an explicit disconnect turns
/// it off. An unreadable file reads as `true` for the same reason - the failure
/// that matters is a tool silently not routing, not one routing when asked.
pub(crate) fn export_opted_in() -> bool {
    let Ok(path) = choice_path() else {
        return true;
    };
    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str::<ExportChoice>(&raw)
            .map(|c| c.enabled)
            .unwrap_or(true),
        Err(_) => true,
    }
}

/// Record the user's choice. Persisted so it survives a routing toggle, an app
/// restart, and a reboot.
pub(crate) fn set_export_opted_in(enabled: bool) -> Result<()> {
    let path = choice_path()?;
    let raw = serde_json::to_string_pretty(&ExportChoice { enabled })
        .context("serializing the proxy env choice")?;
    crate::primitives::write_file(&path, raw.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

/// What each managed variable held before we first set it, so disable can put
/// the user's own value back rather than simply deleting ours.
///
/// This matters more here than it does for the Linux drop-in. There, our
/// variables live in a file we own outright, and "off" is a plain delete that
/// uncovers whatever else the session set. macOS `launchctl` and the Windows
/// `HKCU\Environment` key are *shared* stores: writing into them overwrites the
/// user's own value in place, and deleting on disable would destroy a corporate
/// egress proxy the rest of their machine depends on. Hence macOS/Windows only.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct EnvSnapshot {
    /// Prior value per variable. `None` records "was not set", which restores
    /// as a delete rather than as an empty string.
    #[serde(default)]
    pub vars: BTreeMap<String, Option<String>>,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn snapshot_path() -> Result<PathBuf> {
    Ok(crate::env::app_support_dir()?
        .join("proxy")
        .join("env-snapshot.json"))
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn save_snapshot(snapshot: &EnvSnapshot) -> Result<()> {
    let path = snapshot_path()?;
    let raw = serde_json::to_string_pretty(snapshot).context("serializing proxy env snapshot")?;
    crate::primitives::write_file(&path, raw.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn load_snapshot() -> Result<Option<EnvSnapshot>> {
    let path = snapshot_path()?;
    match fs::read_to_string(&path) {
        Ok(raw) => {
            Ok(Some(serde_json::from_str(&raw).with_context(|| {
                format!("parsing {} as JSON", path.display())
            })?))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn clear_snapshot() -> Result<()> {
    let path = snapshot_path()?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

/// Record the current values of `keys` as the pre-Gate state, unless a snapshot
/// already exists.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn snapshot_prior(keys: &[&'static str], read: impl Fn(&str) -> Option<String>) {
    let existing = load_snapshot().unwrap_or_else(|e| {
        // An unreadable snapshot is not a reason to skip the write: without one,
        // disable falls back to deleting our variables outright, which is the
        // safe direction but loses a user value. Overwrite it with a good one.
        eprintln!("gate proxy: unreadable proxy env snapshot ({e}); re-recording");
        None
    });
    let Some(snapshot) = prior_to_record(existing, keys, read) else {
        return;
    };
    if let Err(e) = save_snapshot(&snapshot) {
        eprintln!("gate proxy: could not record prior proxy environment ({e})");
    }
}

/// What [`snapshot_prior`] should write, or `None` to keep the existing record.
///
/// Split out as a pure function so the re-entry guard is testable without any
/// disk or process-global environment - the guard is the whole point of this
/// module and the one piece with a subtle failure mode.
///
/// The guard: enable is idempotent and can run twice (a re-enable, or a
/// reconcile racing a manual toggle). Without it the second pass would snapshot
/// the values *we* wrote on the first, and disable would then "restore" the user
/// onto our own - by then dead - proxy instead of their real one.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
fn prior_to_record(
    existing: Option<EnvSnapshot>,
    keys: &[&'static str],
    read: impl Fn(&str) -> Option<String>,
) -> Option<EnvSnapshot> {
    if existing.is_some() {
        return None;
    }
    Some(EnvSnapshot {
        vars: keys
            .iter()
            .map(|key| ((*key).to_string(), read(key)))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn the_two_var_sets_agree_on_values() {
        // No temp home needed: `app_support_dir` is pure path construction, so
        // nothing here touches the disk or the process environment.
        let sensitive: BTreeMap<_, _> = case_sensitive(4321).unwrap().into_iter().collect();
        let insensitive: BTreeMap<_, _> = case_insensitive(4321).unwrap().into_iter().collect();

        // Windows drops the lower-case aliases, but must not drift on what the
        // remaining four mean. Checked on every platform, not just Windows.
        for (key, value) in &insensitive {
            assert_eq!(
                sensitive.get(key),
                Some(value),
                "{key} must carry the same value on every platform"
            );
        }
        assert_eq!(
            sensitive.get("http_proxy"),
            sensitive.get("HTTP_PROXY"),
            "the case aliases must carry the same value"
        );
        assert_eq!(
            sensitive.get("HTTPS_PROXY").map(String::as_str),
            Some("http://127.0.0.1:4321")
        );
        assert_eq!(
            sensitive.get("NO_PROXY").map(String::as_str),
            Some(NO_PROXY_VALUE)
        );
        assert!(
            sensitive
                .get("NODE_EXTRA_CA_CERTS")
                .is_some_and(|p| p.ends_with("ca-cert.pem")),
            "the CA variable must point at our cert"
        );
    }

    /// Windows environment names are case-insensitive, so writing the aliases
    /// there would be two writes fighting over one entry.
    #[test]
    fn the_windows_set_carries_no_case_aliases() {
        for key in VARS_CASE_INSENSITIVE {
            assert_eq!(
                key.to_ascii_uppercase(),
                key,
                "{key} must be upper-case on Windows"
            );
        }
        assert!(
            VARS_CASE_INSENSITIVE
                .iter()
                .all(|k| VARS_CASE_SENSITIVE.contains(k)),
            "the Windows set must be a subset of the case-sensitive set"
        );
    }

    /// Enabling twice must not overwrite the record of what the user had.
    ///
    /// Pure, so it needs no temp home and cannot race the other tests in this
    /// process over `GATE_CONNECT_TEST_HOME`.
    #[test]
    fn a_second_enable_does_not_overwrite_the_users_recorded_value() {
        let corporate = "http://proxy.corp.example:3128";
        let first = prior_to_record(None, &VARS_CASE_SENSITIVE, |key| {
            (key == "HTTPS_PROXY").then(|| corporate.to_string())
        })
        .expect("the first enable must record a snapshot");

        assert_eq!(
            first.vars.get("HTTPS_PROXY"),
            Some(&Some(corporate.to_string())),
            "the user's own proxy must be recorded"
        );
        assert_eq!(
            first.vars.get("NO_PROXY"),
            Some(&None),
            "a variable the user never set must be recorded as unset, so it \
             restores as a delete rather than an empty value"
        );

        // Second enable: the environment now reports *our* proxy back. Recording
        // that would make disable "restore" the user onto our dead port.
        let second = prior_to_record(Some(first), &VARS_CASE_SENSITIVE, |_| {
            Some("http://127.0.0.1:9977".to_string())
        });
        assert!(
            second.is_none(),
            "a re-enable must keep the original record, not overwrite it"
        );
    }

    #[test]
    fn loopback_is_always_bypassed() {
        // OpenCode's TUI reaches its own local server; proxying that loops.
        for host in ["localhost", "127.0.0.1", "::1"] {
            assert!(
                NO_PROXY_VALUE.split(',').any(|h| h == host),
                "{host} must be in NO_PROXY"
            );
        }
    }
}
