//! The Rust half of the install funnel (AG-960): which one-time milestones this
//! install has already reported, and the typed reasons a connection failure is
//! filed under.
//!
//! The PostHog client lives in the webview (`src/lib/analytics.ts`), and there are
//! three webviews - main, tray and onboarding - each running its own copy of it.
//! "Once per install" therefore cannot be a flag in memory or in `localStorage`:
//! either would fire once per window, and `localStorage` is wiped by the same
//! storage reset that motivates tying events to the install id in the first
//! place. So the record is a file per milestone under the data dir, next to
//! `install-id`, and the claim is `create_new` on it.
//!
//! **Why a marker file per milestone and not one JSON document.** `create_new`
//! (`O_EXCL` / `CREATE_NEW`) is atomic in the filesystem itself: of any number of
//! windows, processes or threads racing to claim one name, exactly one creates
//! the file and every other gets `AlreadyExists`. A shared JSON file would need a
//! read-modify-write under a cross-process lock to promise the same, and a lock
//! is one more thing to get wrong on three platforms. The file's content is only
//! a timestamp for a human reading the directory; existence is the fact.
//!
//! Nothing here reads a secret, and nothing here touches the network.

use anyhow::{Context, Result};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

/// The directory the markers live in, under the app-support dir.
const STORE_DIR: &str = "analytics-milestones";

/// Written into a store created on a machine that had already run Gate Connect
/// before this store existed. See [`init_in`].
const LEGACY_MARKER: &str = ".legacy";

/// Milestones that record the FIRST time something happened on this install.
///
/// On an install that predates the store they are never claimable, because the
/// store cannot know whether the first time already happened under an older
/// build. `tool_connected.<slug>` is in this class too (see [`is_first_occurrence`]).
const FIRST_OCCURRENCE: [&str; 3] = [
    "app_first_launched",
    "pairing_completed",
    "first_request_proxied",
];

/// Milestones that are once per install but are not a claim about a first time,
/// so an older install can still produce them.
const ONCE_ONLY: [&str; 4] = [
    "diagnostics_opted_out",
    // One per condition, so a person who connects Claude Desktop with local
    // Cowork switched off is reported once rather than on every retry. See
    // [`cowork_setting_missing`].
    "cowork_setting_missing.user",
    "cowork_setting_missing.org_cloud_only",
    "cowork_setting_missing.enterprise",
];

/// Prefix of the per-tool milestone: `tool_connected.claude-code`.
const TOOL_PREFIX: &str = "tool_connected.";

/// The longest tool slug accepted in a milestone name. The registry's slugs are
/// all well under this; the bound exists so a name can never be a path.
const MAX_SLUG_LEN: usize = 48;

/// Whether `name` is a milestone this store knows. A closed set, because the
/// name becomes a file name and arrives from a webview: an unknown name must be
/// an error, never a file, so no string from the front end can name a path.
pub fn is_known_milestone(name: &str) -> bool {
    if FIRST_OCCURRENCE.contains(&name) || ONCE_ONLY.contains(&name) {
        return true;
    }
    match name.strip_prefix(TOOL_PREFIX) {
        Some(slug) => {
            !slug.is_empty()
                && slug.len() <= MAX_SLUG_LEN
                && slug
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        }
        None => false,
    }
}

fn is_first_occurrence(name: &str) -> bool {
    FIRST_OCCURRENCE.contains(&name) || name.starts_with(TOOL_PREFIX)
}

fn store_dir(support: &Path) -> PathBuf {
    support.join(STORE_DIR)
}

/// Files whose presence means this machine ran Gate Connect before the store
/// existed. `account.json` is written by the first sign-in and `preferences.json`
/// by the first Settings change or onboarding answer; a fresh install has
/// neither at the moment the app starts, which is when [`init`] runs.
///
/// `install-id` is deliberately not on the list: the webview reads it at startup
/// to use as the analytics distinct id, so on a fresh install it can exist
/// before the store does, and counting it would mark every new install legacy.
const LEGACY_EVIDENCE: [&str; 2] = ["account.json", "preferences.json"];

/// Create the store if it does not exist, deciding once whether it belongs to a
/// legacy install.
///
/// Called by the desktop shell at the very start of `setup`, before any window
/// loads and before anything else in the app writes to the data dir, so a fresh
/// install is judged fresh. Every claim calls it too, so a store that is somehow
/// missing (deleted by hand) is recreated rather than failing - and recreated as
/// legacy whenever there is evidence of an earlier run, which is the direction
/// that can only suppress a milestone, never duplicate one.
///
/// A store path that exists but is not a directory is a corrupt store. It is
/// moved aside and replaced with a legacy store: nothing about what was claimed
/// can be read from it, and assuming nothing was claimed is how a first-launch
/// event would be sent twice.
pub fn init_in(support: &Path) -> Result<()> {
    fs::create_dir_all(support).with_context(|| format!("creating {}", support.display()))?;
    let dir = store_dir(support);
    match fs::symlink_metadata(&dir) {
        Ok(meta) if meta.is_dir() => return Ok(()),
        Ok(_) => {
            let aside = support.join(format!("{STORE_DIR}.corrupt"));
            let _ = fs::remove_file(&aside);
            fs::rename(&dir, &aside)
                .with_context(|| format!("moving the corrupt {} aside", dir.display()))?;
            return create_store(&dir, true);
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("reading {}", dir.display())),
    }
    let legacy = LEGACY_EVIDENCE.iter().any(|f| support.join(f).exists());
    create_store(&dir, legacy)
}

fn create_store(dir: &Path, legacy: bool) -> Result<()> {
    match fs::create_dir(dir) {
        Ok(()) => {}
        // Another window or process created it between our check and here. It
        // made the legacy decision; ours would be the same one.
        Err(e) if e.kind() == ErrorKind::AlreadyExists => return Ok(()),
        Err(e) => return Err(e).with_context(|| format!("creating {}", dir.display())),
    }
    if legacy {
        write_marker(&dir.join(LEGACY_MARKER))?;
    }
    Ok(())
}

fn write_marker(path: &Path) -> Result<bool> {
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(e).with_context(|| format!("creating {}", path.display())),
    };
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    // The file already exists at this point, which is the claim. A failed write
    // of the timestamp leaves an empty marker, and an empty marker is still a
    // claimed one.
    let _ = writeln!(file, "{now}");
    Ok(true)
}

/// Claim `name` for this install. `Ok(true)` exactly once per install across
/// every window and process; `Ok(false)` once it has been claimed, and always
/// for a first-occurrence milestone on a legacy install.
///
/// An `Err` means the store could not answer (an unwritable data dir, an unknown
/// name). The caller must treat it as "do not send": an event that cannot be
/// recorded as sent may be sent again on the next launch, and a duplicate first
/// launch is worse than a missing one.
pub fn claim_in(support: &Path, name: &str) -> Result<bool> {
    if !is_known_milestone(name) {
        anyhow::bail!("unknown analytics milestone");
    }
    init_in(support)?;
    let dir = store_dir(support);
    if is_first_occurrence(name) && dir.join(LEGACY_MARKER).exists() {
        return Ok(false);
    }
    write_marker(&dir.join(name))
}

/// [`init_in`] against the real data dir.
pub fn init() -> Result<()> {
    init_in(&crate::env::app_support_dir()?)
}

/// [`claim_in`] against the real data dir.
pub fn claim(name: &str) -> Result<bool> {
    claim_in(&crate::env::app_support_dir()?, name)
}

/// The closed set of reasons a connection failure is filed under, as far as the
/// Rust side can tell them apart by TYPE rather than by reading a message.
///
/// Only `port_in_use` is decided here: an `io::Error` of kind `AddrInUse`
/// anywhere in the chain. The OS words for it differ ("Address already in use
/// (os error 48)" on macOS, 98 on Linux, a sentence about socket addresses and
/// error 10048 on Windows), and the synthetic one the engine raises for a live
/// listener prints as "address in use", so the type is the one reliable answer.
/// Every other reason is decided in the webview from typed codes it already has
/// (`gateway_api::FailureCode`) or from the classified error.
pub fn failure_reason(err: &anyhow::Error) -> Option<&'static str> {
    err.chain()
        .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
        .any(|io| io.kind() == ErrorKind::AddrInUse)
        .then_some("port_in_use")
}

/// Why local Cowork cannot run on this device, as Claude Desktop itself decides
/// it, or `None` when nothing Gate can read says it is off.
///
/// **What "the Cowork setting" is.** Cowork is a mode inside the Claude desktop
/// app. Gate routes it through the `anthropic` domain, which only sees a task
/// that runs on THIS machine, in Claude's local VM; a task that runs on
/// Anthropic's servers never touches the machine's network, so routing is on,
/// the CA is trusted and nothing is routed. Claude Desktop gates its local VM
/// ("yukonSilver" in its code) on three settings it stores where Gate can read
/// them, and reports each as its own `unsupportedCode`:
///
/// - `enterprise`: the managed policy `secureVmFeaturesEnabled` is false
///   (`disabled_by_enterprise`). Read from `HKLM\SOFTWARE\Policies\Claude` on
///   Windows. Not read on macOS, where it lives in a configuration profile
///   under `/Library/Managed Preferences` as a binary plist; that is a known gap.
/// - `user`: `preferences.secureVmFeaturesEnabled` is false in
///   `claude_desktop_config.json` (`disabled_by_user`).
/// - `org_cloud_only`: `preferences.coworkLocalTasksOffLatched` is true in the
///   same file, Claude's local copy of an organization policy that runs Cowork
///   in the cloud only (`disabled_by_org_policy`). A cached copy of a server
///   flag, so it can lag the server by a launch of Claude.
///
/// All three are deterministic reads of Claude's own state, not a guess from
/// traffic. They were found in Claude Desktop 2.16120.0's bundle
/// (`.vite/build/index.chunk-*.js`: the preference schema, its defaults
/// `secureVmFeaturesEnabled: true` and `coworkLocalTasksOffLatched: false`, and
/// the support check that returns the codes above), so the key names are
/// Claude's internal names rather than a documented contract and could move in
/// a later Claude release. A missing file, a missing key or unparseable JSON all
/// read as `None`: the direction that under-reports.
///
/// What this does NOT see: the per-account "Only on your computer" choice on
/// claude.ai, which is stored server-side (and which Anthropic's help center
/// says is removed on 2026-10-06, after which new Cowork tasks run in the
/// cloud), platform limits (macOS below 14, no virtualization), and Linux, which
/// has no Claude Desktop to run Cowork in.
pub fn cowork_setting_missing() -> Option<&'static str> {
    if cowork_disabled_by_policy() {
        return Some("enterprise");
    }
    let raw = fs::read_to_string(claude_desktop_config_path()?).ok()?;
    cowork_block_in_config(&raw)
}

/// The user-level Claude Desktop config: `~/Library/Application
/// Support/Claude/claude_desktop_config.json` on macOS and
/// `%APPDATA%\Claude\claude_desktop_config.json` on Windows, the paths
/// Anthropic documents for this file. `None` on Linux (see above).
fn claude_desktop_config_path() -> Option<PathBuf> {
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        Some(
            dirs::data_dir()?
                .join("Claude")
                .join("claude_desktop_config.json"),
        )
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn cowork_disabled_by_policy() -> bool {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    winreg::RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SOFTWARE\\Policies\\Claude")
        .and_then(|k| k.get_value::<u32, _>("secureVmFeaturesEnabled"))
        .is_ok_and(|v| v == 0)
}

#[cfg(not(target_os = "windows"))]
fn cowork_disabled_by_policy() -> bool {
    false
}

/// The pure half of [`cowork_setting_missing`]: which setting in a
/// `claude_desktop_config.json` body turns local Cowork off. The org policy
/// wins over the user's own switch because it is the one the user cannot fix.
pub fn cowork_block_in_config(raw: &str) -> Option<&'static str> {
    let doc: serde_json::Value = serde_json::from_str(raw).ok()?;
    let prefs = doc.get("preferences")?;
    if prefs.get("coworkLocalTasksOffLatched") == Some(&serde_json::Value::Bool(true)) {
        return Some("org_cloud_only");
    }
    if prefs.get("secureVmFeaturesEnabled") == Some(&serde_json::Value::Bool(false)) {
        return Some("user");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A fresh, empty data dir per test. No global override: the store takes its
    /// directory as a parameter, so parallel tests cannot see each other.
    fn scratch(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "gate-milestones-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_milestone_is_claimed_once() {
        let dir = scratch("once");
        init_in(&dir).unwrap();
        assert!(claim_in(&dir, "app_first_launched").unwrap());
        assert!(!claim_in(&dir, "app_first_launched").unwrap());
        assert!(!claim_in(&dir, "app_first_launched").unwrap());
    }

    /// Persistence is the file: a second "process" (a fresh call with nothing in
    /// memory) sees the claim.
    #[test]
    fn a_claim_survives_on_disk() {
        let dir = scratch("persist");
        init_in(&dir).unwrap();
        assert!(claim_in(&dir, "pairing_completed").unwrap());
        assert!(dir.join(STORE_DIR).join("pairing_completed").is_file());
        assert!(!claim_in(&dir, "pairing_completed").unwrap());
    }

    /// Three windows racing over one name is the case the store exists for.
    #[test]
    fn concurrent_claims_have_exactly_one_winner() {
        let dir = scratch("race");
        init_in(&dir).unwrap();
        let wins = std::sync::Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let dir = dir.clone();
                let wins = wins.clone();
                std::thread::spawn(move || {
                    if claim_in(&dir, "first_request_proxied").unwrap() {
                        wins.fetch_add(1, Ordering::Relaxed);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(wins.load(Ordering::Relaxed), 1);
    }

    /// And the same race without anyone having created the store first: the
    /// directory creation is itself a race, and losing it must not be an error.
    #[test]
    fn concurrent_first_claims_on_a_missing_store_have_one_winner() {
        let dir = scratch("race-init");
        let wins = std::sync::Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let dir = dir.clone();
                let wins = wins.clone();
                std::thread::spawn(move || {
                    if claim_in(&dir, "app_first_launched").expect("no claim errors") {
                        wins.fetch_add(1, Ordering::Relaxed);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(wins.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn milestones_are_independent() {
        let dir = scratch("indep");
        init_in(&dir).unwrap();
        assert!(claim_in(&dir, "tool_connected.codex").unwrap());
        assert!(claim_in(&dir, "tool_connected.claude-code").unwrap());
        assert!(claim_in(&dir, "diagnostics_opted_out").unwrap());
        assert!(!claim_in(&dir, "tool_connected.codex").unwrap());
    }

    /// An upgrade must not report "first launch" for a machine that has been
    /// running Gate Connect for months.
    #[test]
    fn a_store_created_on_a_used_install_suppresses_first_occurrences() {
        let dir = scratch("legacy");
        fs::write(dir.join("account.json"), "{}").unwrap();
        init_in(&dir).unwrap();
        assert!(!claim_in(&dir, "app_first_launched").unwrap());
        assert!(!claim_in(&dir, "pairing_completed").unwrap());
        assert!(!claim_in(&dir, "first_request_proxied").unwrap());
        assert!(!claim_in(&dir, "tool_connected.codex").unwrap());
        // Not a claim about a first time, so an older install still records it.
        assert!(claim_in(&dir, "diagnostics_opted_out").unwrap());
        assert!(!claim_in(&dir, "diagnostics_opted_out").unwrap());
    }

    #[test]
    fn preferences_alone_are_evidence_of_an_earlier_run() {
        let dir = scratch("legacy-prefs");
        fs::write(dir.join("preferences.json"), "{}").unwrap();
        init_in(&dir).unwrap();
        assert!(!claim_in(&dir, "app_first_launched").unwrap());
    }

    /// The webview reads the install id to bootstrap PostHog, so it can exist on
    /// a fresh install before the store does. It must not make the install legacy.
    #[test]
    fn an_install_id_alone_is_not_evidence_of_an_earlier_run() {
        let dir = scratch("fresh-id");
        fs::write(dir.join("install-id"), "abc").unwrap();
        init_in(&dir).unwrap();
        assert!(claim_in(&dir, "app_first_launched").unwrap());
    }

    /// The decision is made when the store is created, not re-made on every
    /// claim: an account written after a fresh first launch does not
    /// retroactively make the install legacy.
    #[test]
    fn a_fresh_store_stays_fresh_after_sign_in() {
        let dir = scratch("stays-fresh");
        init_in(&dir).unwrap();
        fs::write(dir.join("account.json"), "{}").unwrap();
        assert!(claim_in(&dir, "pairing_completed").unwrap());
    }

    #[test]
    fn a_corrupt_store_is_replaced_and_suppresses_first_occurrences() {
        let dir = scratch("corrupt");
        fs::write(dir.join(STORE_DIR), "not a directory").unwrap();
        assert!(!claim_in(&dir, "app_first_launched").unwrap());
        assert!(dir.join(STORE_DIR).is_dir());
        assert!(dir.join(format!("{STORE_DIR}.corrupt")).is_file());
        assert!(claim_in(&dir, "diagnostics_opted_out").unwrap());
    }

    /// A marker whose content is garbage (or empty, from a write that failed
    /// after the create) is still a claim.
    #[test]
    fn a_marker_with_unreadable_content_still_counts_as_claimed() {
        let dir = scratch("garbage");
        init_in(&dir).unwrap();
        fs::write(dir.join(STORE_DIR).join("pairing_completed"), [0xff, 0xfe]).unwrap();
        assert!(!claim_in(&dir, "pairing_completed").unwrap());
    }

    #[test]
    fn an_unwritable_store_is_an_error_not_a_claim() {
        let dir = scratch("unwritable");
        init_in(&dir).unwrap();
        // A directory where the marker file should go makes the create fail
        // with something other than NotFound on every platform... except that
        // create_new on an existing directory reports AlreadyExists, which is
        // itself the conservative answer. So the failure mode tested here is a
        // data dir that is not a directory at all.
        let file_as_dir = dir.join("not-a-dir");
        fs::write(&file_as_dir, "x").unwrap();
        assert!(claim_in(&file_as_dir, "app_first_launched").is_err());
    }

    #[test]
    fn names_are_a_closed_set() {
        let dir = scratch("names");
        for bad in [
            "",
            "app_launched",
            "../account.json",
            "tool_connected.",
            "tool_connected.../x",
            "tool_connected.Codex",
            "tool_connected.a/b",
            "tool_connected.a\\b",
            ".legacy",
        ] {
            assert!(!is_known_milestone(bad), "{bad:?} must not be a milestone");
            assert!(
                claim_in(&dir, bad).is_err(),
                "{bad:?} must not be claimable"
            );
        }
        let long = format!("tool_connected.{}", "a".repeat(MAX_SLUG_LEN + 1));
        assert!(!is_known_milestone(&long));
        assert!(is_known_milestone("tool_connected.env-proxy"));
        assert!(is_known_milestone("tool_connected.claude_desktop"));
    }

    #[test]
    fn addr_in_use_anywhere_in_the_chain_is_port_in_use() {
        let io = std::io::Error::from(ErrorKind::AddrInUse);
        let err = anyhow::Error::new(io)
            .context("the relay port 45981 is already in use")
            .context("starting the engine");
        assert_eq!(failure_reason(&err), Some("port_in_use"));
    }

    #[test]
    fn other_errors_have_no_typed_reason() {
        let io = std::io::Error::from(ErrorKind::PermissionDenied);
        assert_eq!(failure_reason(&anyhow::Error::new(io)), None);
        // Prose alone is not a type: a message that merely says the words is not
        // classified here. The webview's classifier owns prose.
        assert_eq!(
            failure_reason(&anyhow::anyhow!("address already in use")),
            None
        );
    }

    #[test]
    fn cowork_is_blocked_only_by_the_settings_claude_reads() {
        assert_eq!(
            cowork_block_in_config(r#"{"preferences":{"secureVmFeaturesEnabled":false}}"#),
            Some("user")
        );
        assert_eq!(
            cowork_block_in_config(r#"{"preferences":{"coworkLocalTasksOffLatched":true}}"#),
            Some("org_cloud_only")
        );
        // The org policy is the one the user cannot fix, so it names the cause.
        assert_eq!(
            cowork_block_in_config(
                r#"{"preferences":{"secureVmFeaturesEnabled":false,"coworkLocalTasksOffLatched":true}}"#
            ),
            Some("org_cloud_only")
        );
    }

    /// Everything that is not an explicit "off" reads as not blocked, which is
    /// the direction that can only under-report.
    #[test]
    fn cowork_defaults_and_garbage_are_not_a_block() {
        for raw in [
            "",
            "not json",
            "[]",
            "{}",
            r#"{"mcpServers":{}}"#,
            r#"{"preferences":{}}"#,
            r#"{"preferences":{"secureVmFeaturesEnabled":true,"coworkLocalTasksOffLatched":false}}"#,
            // A string is not Claude's boolean, so it is not read as one.
            r#"{"preferences":{"secureVmFeaturesEnabled":"false"}}"#,
            r#"{"preferences":{"coworkLocalTasksOffLatched":1}}"#,
            r#"{"secureVmFeaturesEnabled":false}"#,
        ] {
            assert_eq!(cowork_block_in_config(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn cowork_conditions_are_claimable_once_each() {
        let dir = scratch("cowork");
        fs::write(dir.join("account.json"), "{}").unwrap();
        init_in(&dir).unwrap();
        // Legacy installs still report them: a failure is not a first time.
        assert!(claim_in(&dir, "cowork_setting_missing.user").unwrap());
        assert!(!claim_in(&dir, "cowork_setting_missing.user").unwrap());
        assert!(claim_in(&dir, "cowork_setting_missing.org_cloud_only").unwrap());
        assert!(!is_known_milestone("cowork_setting_missing.other"));
    }
}
