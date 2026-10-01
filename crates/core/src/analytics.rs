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
//! Nothing here touches the network. The one secret read is [`save_identity`]'s
//! check that a sub it is asked to store is the live session's, through the
//! witnessed `oauth::current`, and only for a save that names a sub.

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
    create_store_with(dir, |staging| {
        if legacy {
            write_marker(&staging.join(LEGACY_MARKER))?;
        }
        Ok(())
    })
}

/// Put a complete store at `dir` in one step, or none at all.
///
/// The store is built in a staging directory beside it, `.legacy` included,
/// and renamed into place. Creating `dir` first and writing `.legacy` into it
/// second left a window in which the directory existed without its marker: a
/// marker write that failed there (a full disk, an antivirus lock on Windows)
/// left an upgraded install judged fresh for good, because every later
/// [`init_in`] sees the directory and returns, and a claim racing between the
/// two steps saw a fresh store too. Either way the first-occurrence milestones
/// went out for a machine that had run Gate Connect for months.
///
/// Losing the rename to another window or process is not an error: the
/// winner's store is complete, and its legacy decision is the one that stands.
/// A rename only replaces an EMPTY directory (POSIX; Windows never replaces
/// one), and a store is empty only while it is fresh and unclaimed, so the
/// most a lost race can do is turn a fresh store legacy, which suppresses a
/// milestone and never duplicates one.
fn create_store_with(dir: &Path, populate: impl FnOnce(&Path) -> Result<()>) -> Result<()> {
    static STAGING: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let parent = dir
        .parent()
        .context("the milestone store has no parent dir")?;
    let staging = parent.join(format!(
        ".{STORE_DIR}.staging-{}-{}",
        std::process::id(),
        STAGING.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir(&staging).with_context(|| format!("creating {}", staging.display()))?;
    let built = populate(&staging).and_then(|()| match fs::rename(&staging, dir) {
        Ok(()) => Ok(()),
        Err(_) if dir.is_dir() => Ok(()),
        Err(e) => Err(e).with_context(|| format!("moving the store into {}", dir.display())),
    });
    // Gone already after a successful rename; otherwise the half-built store.
    let _ = fs::remove_dir_all(&staging);
    built
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
    let os = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    };
    let candidates = claude_config_candidates(
        os,
        dirs::data_dir().as_deref(),
        dirs::data_local_dir().as_deref(),
    );
    strongest_block(
        candidates
            .iter()
            .filter_map(|p| fs::read_to_string(p).ok())
            .filter_map(|raw| cowork_block_in_config(&raw)),
    )
}

/// Any-true across every config Claude might be using, with the org policy
/// outranking the user's own switch - the same precedence as within one file.
fn strongest_block(found: impl Iterator<Item = &'static str>) -> Option<&'static str> {
    let mut best = None;
    for block in found {
        if block == "org_cloud_only" {
            return Some(block);
        }
        best = Some(block);
    }
    best
}

/// Every `claude_desktop_config.json` Claude Desktop itself may read, in the
/// order Claude probes its data dirs.
///
/// Taken from Claude Desktop 2.16120.0's own resolver rather than from the one
/// path the MCP docs name, because on Windows the first-party app is an MSIX
/// package (which local Cowork requires) and its data lives in the package's
/// redirected roaming folder, not in `%APPDATA%`. The bundle's `$Re()` lists the
/// first-party dirs:
///
/// - Windows: `%LOCALAPPDATA%\Claude-Data` (used when roaming app data is
///   redirected to a network share), `%APPDATA%\Claude`, and
///   `%LOCALAPPDATA%\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude`.
/// - elsewhere: `<app data>/Claude`.
///
/// and `Gu()` the third-party deployment's dir, `Claude-3p`: under
/// `%LOCALAPPDATA%` on Windows, beside `Claude` elsewhere. Probing a dir that
/// is not in use costs one failed read, and the answer is any-true, so listing
/// one Claude does not use on this machine cannot produce a false block - only
/// a stale file left behind by an uninstalled flavour could, which is the
/// accepted limit of reading another app's state.
///
/// Empty on Linux, which has no Claude Desktop to run Cowork in. Pure, so each
/// path is testable on any host.
pub fn claude_config_candidates(
    os: &str,
    app_data: Option<&Path>,
    local_app_data: Option<&Path>,
) -> Vec<PathBuf> {
    const FILE: &str = "claude_desktop_config.json";
    let mut dirs: Vec<PathBuf> = Vec::new();
    match os {
        "windows" => {
            if let Some(local) = local_app_data {
                dirs.push(local.join("Claude-Data"));
            }
            if let Some(roaming) = app_data {
                dirs.push(roaming.join("Claude"));
            }
            if let Some(local) = local_app_data {
                dirs.push(
                    local
                        .join("Packages")
                        .join("Claude_pzs8sxrjxfjjc")
                        .join("LocalCache")
                        .join("Roaming")
                        .join("Claude"),
                );
                dirs.push(local.join("Claude-3p"));
            }
        }
        "macos" => {
            if let Some(support) = app_data {
                dirs.push(support.join("Claude"));
                dirs.push(support.join("Claude-3p"));
            }
        }
        _ => {}
    }
    dirs.into_iter().map(|d| d.join(FILE)).collect()
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

/// What this install is identified as in analytics, kept next to `install-id`
/// so every window and every launch agrees on it (AG-960).
///
/// **Why it is persisted.** posthog-js's bootstrap re-registers its distinct id
/// on every launch; bootstrapping the install id as anonymous each time forced
/// a fresh `$identify(sub, $anon = install id)` per launch, and a second account
/// signing in on the same machine would have been merged onto the first
/// person's install. With this record the webview bootstraps the identified
/// `sub` directly (`isIdentifiedID: true`, no event), merges the install id into
/// a person only the first time this install is ever identified
/// (`ever_identified`), and on sign-out or an account switch resets the client
/// instead of merging.
///
/// **Only a Constellation sign-in is ever a person.** An API-key account is
/// never identified, here or anywhere else: the key's creator is not
/// necessarily the person at this machine. So nothing about an API-key account
/// is recorded beyond the org and auth mode the windows group by.
///
/// Nothing here is a secret: `sub` is the opaque Cognito id already sent as the
/// distinct id, and the org id is already the analytics group. The webview only
/// writes a sub after the diagnostics answer allows identification, so an
/// install that never agreed has nothing here but the org it routes for.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Identity {
    /// The Cognito `sub` the analytics client is identified as right now, or
    /// `None` when it is not identified.
    #[serde(default)]
    pub identified_sub: Option<String>,
    /// Whether this install has ever been identified with anybody. Sticky: once
    /// true, a save cannot turn it back. It is also what retires the install id
    /// as a distinct id: once identified, the install id belongs to that
    /// account's person, so after a sign-out the client moves to a fresh
    /// anonymous id rather than back onto it.
    #[serde(default)]
    pub ever_identified: bool,
    /// The organization this install routes for, as the owning window last saw
    /// it. Lets a window that never reads the account (the tray, the intro)
    /// group its events, including an API-key account's, whose org only the
    /// main window's activity read learns.
    #[serde(default)]
    pub org_id: Option<String>,
    /// `"oauth"` or `"api_key"`, beside the org it describes.
    #[serde(default)]
    pub auth_mode: Option<String>,
}

const IDENTITY_FILE: &str = "analytics-identity.json";

/// The advisory lock every writer of [`IDENTITY_FILE`] holds, beside it. A
/// separate file because the record itself is replaced by rename, and a lock on
/// a file that is renamed over locks nothing the next opener sees.
const IDENTITY_LOCK_FILE: &str = "analytics-identity.lock";

/// A value from the webview that becomes a PostHog id: bounded, printable,
/// non-empty. Anything else is dropped rather than stored.
fn clean_id(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
}

/// The stored identity.
///
/// A missing file is a fresh install and reads as the default: never
/// identified. A file that exists but cannot be read or parsed **fails
/// closed**: it reads as an install that has been identified, because nothing
/// about who it belonged to can be recovered, and assuming "nobody" would
/// bootstrap an install id that may already be a person's. The cost is that
/// such an install files under a fresh anonymous id from then on.
///
/// An I/O failure that is not about the content (permission denied, a busy
/// file, a rename racing the read on Windows) also reads as fail-closed here,
/// because a reader must answer something; but the writers below refuse to
/// build on it (see [`read_identity_in`]), so a transient error can never be
/// written back as a permanent fact.
pub fn load_identity_in(support: &Path) -> Identity {
    read_identity_in(support).unwrap_or_else(|_| Identity::fail_closed())
}

/// [`load_identity_in`] for a writer: `Err` when the file could not be READ,
/// so the caller writes nothing, rather than persisting the fail-closed value
/// of a read that may succeed a moment later. Content that is there but not a
/// record (unparseable JSON, bytes that are not UTF-8) still fails closed: that
/// will not get better on a retry.
///
/// A record written by an earlier build of this branch may carry fields this
/// one no longer has (`api_key_org`, `install_id_retired`); serde ignores
/// them.
fn read_identity_in(support: &Path) -> Result<Identity> {
    let path = support.join(IDENTITY_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Identity::default()),
        Err(e) if e.kind() == ErrorKind::InvalidData => return Ok(Identity::fail_closed()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    Ok(serde_json::from_str::<Identity>(&raw)
        .ok()
        .map(|i| Identity {
            identified_sub: clean_id(i.identified_sub),
            org_id: clean_id(i.org_id),
            auth_mode: clean_id(i.auth_mode),
            ever_identified: i.ever_identified,
        })
        .unwrap_or_else(Identity::fail_closed))
}

impl Identity {
    /// What an unreadable record stands for: see [`load_identity_in`].
    fn fail_closed() -> Self {
        Identity {
            ever_identified: true,
            ..Identity::default()
        }
    }
}

/// Serialises every writer's read-compute-write of the record within this
/// process. The file lock below would serialise threads too (each takes its own
/// open file description), but holding the mutex first keeps a thread from
/// spinning on the Windows lock against its own process.
static IDENTITY_WRITERS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Run one writer's whole read-compute-write with the record to itself.
///
/// Two layers, because there are two kinds of concurrent writer. The app's
/// windows reach [`save_identity`] from command threads while a sign-out reaches
/// [`forget_identity`] from a blocking task, and `gate-connect logout` reaches
/// the same forget from a process of its own. Without this, a save that read
/// the record before a forget and wrote after it put the signed-out account
/// back: the forget was simply lost. The mutex covers the threads; the advisory
/// lock on [`IDENTITY_LOCK_FILE`] covers the CLI, and is released by the OS if
/// its holder dies.
fn with_identity_locked<T>(support: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let _threads = IDENTITY_WRITERS.lock().unwrap_or_else(|e| e.into_inner());
    fs::create_dir_all(support).with_context(|| format!("creating {}", support.display()))?;
    let _process = file_lock::exclusive(&support.join(IDENTITY_LOCK_FILE))?;
    f()
}

/// A cross-process advisory lock on one file, held until the returned handle is
/// dropped. No new dependency: `flock(2)` through `libc` on unix, and on
/// Windows an open with no sharing, which the OS refuses to any other opener
/// (`ERROR_SHARING_VIOLATION`) until the handle closes.
mod file_lock {
    use anyhow::{Context, Result};
    use std::fs::{File, OpenOptions};
    use std::path::Path;

    #[cfg(unix)]
    pub fn exclusive(path: &Path) -> Result<File> {
        use std::os::fd::AsRawFd;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        loop {
            // SAFETY: `file` owns a valid fd for the duration of the call.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                return Ok(file);
            }
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::Interrupted {
                return Err(err).with_context(|| format!("locking {}", path.display()));
            }
        }
    }

    #[cfg(windows)]
    pub fn exclusive(path: &Path) -> Result<File> {
        use std::os::windows::fs::OpenOptionsExt;
        use std::time::{Duration, Instant};
        const ERROR_SHARING_VIOLATION: i32 = 32;
        // A holder keeps it for one small read and write; ten seconds is a
        // holder that is stuck, and a stuck lock must not hang a sign-out.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .share_mode(0)
                .open(path)
            {
                Ok(file) => return Ok(file),
                Err(e)
                    if e.raw_os_error() == Some(ERROR_SHARING_VIOLATION)
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(e).with_context(|| format!("locking {}", path.display())),
            }
        }
    }
}

fn write_identity_in(support: &Path, stored: &Identity) -> Result<()> {
    let body = serde_json::to_vec_pretty(stored).context("serializing analytics identity")?;
    crate::primitives::write_file(&support.join(IDENTITY_FILE), &body, 0o600)
}

/// Store `next`, keeping `ever_identified` sticky (and implied by a sub).
///
/// **A sub is accepted only for the live session.** `live_sub` answers which
/// Constellation account is signed in right now (the stored token bundle's
/// `sub`), and a save naming any other sub, or a sub while nobody is signed in,
/// is refused with nothing written. The webview's view of the session lags the
/// core's: a save already in flight when a sign-out ran, or one a window makes
/// from a session read taken before the sign-out, would otherwise land after
/// [`forget_identity_in`] and put the account that left back on record, and the
/// next launch would bootstrap it as identified. Asked inside the lock, so a
/// forget cannot run between the check and the write; and every sign-out
/// deletes the bundle before it forgets, so a save the lock lets through after
/// the deletion finds no live sub.
///
/// `live_sub` is not called for a save with no sub, which is what keeps this
/// off the secret store for every org or auth-mode update.
pub fn save_identity_in(
    support: &Path,
    next: Identity,
    live_sub: impl FnOnce() -> Option<String>,
) -> Result<()> {
    let identified_sub = clean_id(next.identified_sub);
    with_identity_locked(support, || {
        let prev = read_identity_in(support)?;
        if let Some(sub) = identified_sub.as_deref() {
            if clean_id(live_sub()).as_deref() != Some(sub) {
                anyhow::bail!("refusing an analytics identity that is not the live session's");
            }
        }
        let stored = Identity {
            ever_identified: prev.ever_identified
                || next.ever_identified
                || identified_sub.is_some(),
            identified_sub,
            org_id: clean_id(next.org_id),
            auth_mode: clean_id(next.auth_mode),
        };
        write_identity_in(support, &stored)
    })
}

/// The account is gone: drop the identified sub, the org and the auth mode,
/// keeping `ever_identified`. Called from `oauth::clear`, which every sign-out
/// path reaches - the app's Disconnect and Reset, `gate-connect logout`, the
/// startup reconcile - so a sign-out the webview never saw still stops the next
/// launch filing under the old person.
pub fn forget_identity_in(support: &Path) -> Result<()> {
    with_identity_locked(support, || {
        let prev = read_identity_in(support)?;
        if prev.identified_sub.is_none() && prev.org_id.is_none() && prev.auth_mode.is_none() {
            return Ok(());
        }
        write_identity_in(
            support,
            &Identity {
                ever_identified: prev.ever_identified,
                ..Identity::default()
            },
        )
    })
}

/// [`load_identity_in`] against the real data dir.
pub fn load_identity() -> Identity {
    crate::env::app_support_dir()
        .map(|d| load_identity_in(&d))
        .unwrap_or_default()
}

/// [`save_identity_in`] against the real data dir and the stored OAuth session.
/// The session read is `oauth::current`, witnessed on `account.json`, so it is a
/// cache hit unless a sign-in or sign-out has moved that file.
pub fn save_identity(next: Identity) -> Result<()> {
    save_identity_in(&crate::env::app_support_dir()?, next, || {
        crate::oauth::current().ok().flatten().and_then(|t| t.sub())
    })
}

/// [`forget_identity_in`] against the real data dir.
pub fn forget_identity() -> Result<()> {
    forget_identity_in(&crate::env::app_support_dir()?)
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

    #[test]
    fn windows_probes_every_dir_claude_does() {
        let got = claude_config_candidates("windows", Some(Path::new("R")), Some(Path::new("L")));
        let want: Vec<PathBuf> = [
            "L/Claude-Data",
            "R/Claude",
            "L/Packages/Claude_pzs8sxrjxfjjc/LocalCache/Roaming/Claude",
            "L/Claude-3p",
        ]
        .iter()
        .map(|d| {
            d.split('/')
                .fold(PathBuf::new(), |p, c| p.join(c))
                .join("claude_desktop_config.json")
        })
        .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn macos_probes_first_and_third_party_dirs() {
        let got = claude_config_candidates("macos", Some(Path::new("S")), None);
        assert_eq!(
            got,
            vec![
                Path::new("S")
                    .join("Claude")
                    .join("claude_desktop_config.json"),
                Path::new("S")
                    .join("Claude-3p")
                    .join("claude_desktop_config.json"),
            ]
        );
    }

    #[test]
    fn linux_has_no_claude_desktop_to_probe() {
        assert!(
            claude_config_candidates("linux", Some(Path::new("S")), Some(Path::new("L")))
                .is_empty()
        );
    }

    /// Each candidate on its own is enough: the MSIX package dir is where the
    /// first-party Windows app keeps it, and a probe that stopped at
    /// `%APPDATA%` missed it entirely.
    #[test]
    fn any_candidate_that_says_off_wins() {
        let dir = scratch("cowork-paths");
        let roaming = dir.join("R");
        let local = dir.join("L");
        let candidates = claude_config_candidates("windows", Some(&roaming), Some(&local));
        for (i, target) in candidates.iter().enumerate() {
            let _ = fs::remove_dir_all(&roaming);
            let _ = fs::remove_dir_all(&local);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(
                target,
                r#"{"preferences":{"secureVmFeaturesEnabled":false}}"#,
            )
            .unwrap();
            let found = strongest_block(
                candidates
                    .iter()
                    .filter_map(|p| fs::read_to_string(p).ok())
                    .filter_map(|raw| cowork_block_in_config(&raw)),
            );
            assert_eq!(found, Some("user"), "candidate {i}: {}", target.display());
        }
    }

    #[test]
    fn the_org_policy_outranks_the_user_switch_across_files() {
        assert_eq!(
            strongest_block(["user", "org_cloud_only"].into_iter()),
            Some("org_cloud_only")
        );
        assert_eq!(strongest_block(["user"].into_iter()), Some("user"));
        assert_eq!(strongest_block(std::iter::empty()), None);
    }

    /// No live session: the closure a save with no sub must never need.
    fn no_session() -> Option<String> {
        None
    }

    fn live(sub: &str) -> impl FnOnce() -> Option<String> + '_ {
        move || Some(sub.to_string())
    }

    #[test]
    fn identity_round_trips_and_first_identification_is_sticky() {
        let dir = scratch("identity");
        assert_eq!(load_identity_in(&dir), Identity::default());
        save_identity_in(
            &dir,
            Identity {
                identified_sub: Some("sub-a".into()),
                org_id: Some("org-1".into()),
                auth_mode: Some("oauth".into()),
                ever_identified: false,
            },
            live("sub-a"),
        )
        .unwrap();
        let got = load_identity_in(&dir);
        assert_eq!(got.identified_sub.as_deref(), Some("sub-a"));
        assert!(
            got.ever_identified,
            "a sub implies the install has been identified"
        );

        forget_identity_in(&dir).unwrap();
        let got = load_identity_in(&dir);
        assert_eq!(got.identified_sub, None);
        assert_eq!(got.org_id, None);
        assert_eq!(got.auth_mode, None);
        assert!(
            got.ever_identified,
            "sign-out must not make the next sign-in look like the first"
        );

        // A save from the webview cannot clear it either.
        save_identity_in(&dir, Identity::default(), no_session).unwrap();
        assert!(load_identity_in(&dir).ever_identified);
    }

    #[test]
    fn a_corrupt_identity_fails_closed_and_hostile_values_are_not_stored() {
        let dir = scratch("identity-bad");
        fs::write(dir.join(IDENTITY_FILE), "{not json").unwrap();
        // Fails closed: nothing can say whose the install id was.
        let got = load_identity_in(&dir);
        assert!(got.ever_identified);
        assert_eq!(got.identified_sub, None);
        fs::remove_file(dir.join(IDENTITY_FILE)).unwrap();
        save_identity_in(
            &dir,
            Identity {
                identified_sub: Some("  ".into()),
                org_id: Some("a\nb".into()),
                auth_mode: Some("x".repeat(200)),
                ever_identified: false,
            },
            no_session,
        )
        .unwrap();
        assert_eq!(load_identity_in(&dir), Identity::default());
    }

    #[test]
    fn a_missing_identity_is_a_fresh_install_and_a_corrupt_one_fails_closed() {
        let dir = scratch("identity-fail-closed");
        assert_eq!(load_identity_in(&dir), Identity::default());
        fs::write(dir.join(IDENTITY_FILE), [0xff, 0xfe, 0x00]).unwrap();
        assert!(load_identity_in(&dir).ever_identified);
    }

    /// A record an earlier build of this branch wrote, with the API-key
    /// retirement fields it no longer has, is still a record: an unknown field
    /// is ignored, not a parse failure that would fail closed.
    #[test]
    fn a_record_with_the_retired_api_key_fields_still_reads() {
        let dir = scratch("identity-old-fields");
        fs::write(
            dir.join(IDENTITY_FILE),
            r#"{"identified_sub":null,"ever_identified":false,"org_id":"org-a","auth_mode":"api_key","api_key_org":"org-a","install_id_retired":true}"#,
        )
        .unwrap();
        let got = load_identity_in(&dir);
        assert!(!got.ever_identified);
        assert_eq!(got.org_id.as_deref(), Some("org-a"));
        assert_eq!(got.auth_mode.as_deref(), Some("api_key"));
    }

    /// Review item 2: a save naming a sub lands only while that sub is the
    /// live session's. Nothing is written when it is refused.
    #[test]
    fn a_sub_is_stored_only_for_the_live_session() {
        let dir = scratch("identity-live");
        let with_sub = |sub: &str| Identity {
            identified_sub: Some(sub.into()),
            org_id: Some("org-1".into()),
            auth_mode: Some("oauth".into()),
            ever_identified: true,
        };
        assert!(save_identity_in(&dir, with_sub("sub-a"), no_session).is_err());
        assert!(
            !dir.join(IDENTITY_FILE).exists(),
            "a refused save writes nothing"
        );
        assert!(save_identity_in(&dir, with_sub("sub-a"), live("sub-b")).is_err());
        assert!(!dir.join(IDENTITY_FILE).exists());
        save_identity_in(&dir, with_sub("sub-a"), live("sub-a")).unwrap();
        assert_eq!(
            load_identity_in(&dir).identified_sub.as_deref(),
            Some("sub-a")
        );
    }

    /// The race the reviewer described: a window's save was in flight when the
    /// sign-out ran. The sign-out deleted the bundle and forgot; the late save
    /// must not put the account back.
    #[test]
    fn a_save_that_lands_after_a_sign_out_does_not_restore_the_account() {
        let dir = scratch("identity-late-save");
        let signed_in = Identity {
            identified_sub: Some("sub-a".into()),
            org_id: Some("org-1".into()),
            auth_mode: Some("oauth".into()),
            ever_identified: true,
        };
        save_identity_in(&dir, signed_in.clone(), live("sub-a")).unwrap();
        // `oauth::clear`: the bundle goes first, then the forget.
        forget_identity_in(&dir).unwrap();
        assert!(save_identity_in(&dir, signed_in, no_session).is_err());
        let got = load_identity_in(&dir);
        assert_eq!(got.identified_sub, None);
        assert_eq!(got.org_id, None);
        assert!(got.ever_identified);
    }

    /// The session is consulted only for a save that names a sub, so an org or
    /// auth-mode update never reads the secret store.
    #[test]
    fn a_save_with_no_sub_does_not_ask_for_the_session() {
        let dir = scratch("identity-no-ask");
        save_identity_in(
            &dir,
            Identity {
                org_id: Some("org-1".into()),
                auth_mode: Some("api_key".into()),
                ..Identity::default()
            },
            || panic!("the session was read for a save with no sub"),
        )
        .unwrap();
        assert_eq!(load_identity_in(&dir).org_id.as_deref(), Some("org-1"));
    }

    /// Review item 1, the cross-process half: a writer waits while another
    /// holder (here an open of the lock file of our own, which `flock` and a
    /// no-share open both treat as a different holder) has the record.
    #[test]
    fn a_writer_waits_for_the_lock_another_process_holds() {
        let dir = scratch("identity-flock");
        save_identity_in(
            &dir,
            Identity {
                identified_sub: Some("sub-a".into()),
                ever_identified: true,
                ..Identity::default()
            },
            live("sub-a"),
        )
        .unwrap();
        let held = file_lock::exclusive(&dir.join(IDENTITY_LOCK_FILE)).unwrap();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let forget = {
            let (dir, done) = (dir.clone(), done.clone());
            std::thread::spawn(move || {
                forget_identity_in(&dir).unwrap();
                done.store(true, Ordering::SeqCst);
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(
            !done.load(Ordering::SeqCst),
            "the forget ran while another holder had the lock"
        );
        assert_eq!(
            load_identity_in(&dir).identified_sub.as_deref(),
            Some("sub-a")
        );
        drop(held);
        forget.join().unwrap();
        assert_eq!(load_identity_in(&dir).identified_sub, None);
    }

    /// Review item 1, the read-compute-write itself: two writers racing over
    /// one record, one of them identifying. Unlocked, the other read the record
    /// before the identification and wrote after it, losing the sticky bit.
    #[test]
    fn concurrent_writers_never_lose_a_sticky_fact() {
        for round in 0..200 {
            let dir = scratch(&format!("identity-race-{round}"));
            let gate = std::sync::Arc::new(std::sync::Barrier::new(3));
            let identify = {
                let (dir, gate) = (dir.clone(), gate.clone());
                std::thread::spawn(move || {
                    gate.wait();
                    save_identity_in(
                        &dir,
                        Identity {
                            identified_sub: Some("sub-a".into()),
                            ..Identity::default()
                        },
                        || Some("sub-a".into()),
                    )
                    .unwrap();
                })
            };
            let updates: Vec<_> = (0..2)
                .map(|_| {
                    let (dir, gate) = (dir.clone(), gate.clone());
                    std::thread::spawn(move || {
                        gate.wait();
                        save_identity_in(
                            &dir,
                            Identity {
                                org_id: Some("org-1".into()),
                                ..Identity::default()
                            },
                            no_session,
                        )
                        .unwrap();
                    })
                })
                .collect();
            identify.join().unwrap();
            for u in updates {
                u.join().unwrap();
            }
            assert!(
                load_identity_in(&dir).ever_identified,
                "round {round}: an identification was lost to a concurrent write"
            );
            let _ = fs::remove_dir_all(&dir);
        }
    }

    /// Round 3, M3: the forget lives in the core, on the path every sign-out
    /// takes, so `gate-connect logout` forgets too. `account::clear` is what the
    /// CLI's logout and the app's Reset call, and it reaches `oauth::clear`.
    #[test]
    fn every_sign_out_path_reaches_the_forget() {
        let oauth = include_str!("oauth.rs").replace("\r\n", "\n");
        let at = oauth
            .find("pub fn clear() -> Result<()> {")
            .expect("oauth::clear");
        let body = &oauth[at..at + oauth[at..].find("\n}\n").expect("end of clear")];
        let forget = body
            .find("crate::analytics::forget_identity()")
            .expect("clear forgets");
        // After the bundle is deleted, which is what lets `save_identity`'s
        // live-session check refuse a save that lands after the forget.
        assert!(body.find("keychain::delete(").expect("the delete") < forget);

        let account = include_str!("account.rs").replace("\r\n", "\n");
        let at = account
            .find("pub fn clear() -> Result<()> {")
            .expect("account::clear");
        let body = &account[at..at + account[at..].find("\n}\n").expect("end of clear")];
        assert!(body.contains("crate::oauth::clear()?"));

        let cli = include_str!("../../cli/src/main.rs").replace("\r\n", "\n");
        let at = cli.find("fn cmd_logout()").expect("cmd_logout");
        assert!(cli[at..].contains("account::clear()?"));
    }

    /// Round 5: an identity file that cannot be READ (here, a directory where
    /// the file should be: the read fails with an I/O error, not a parse error)
    /// makes every writer refuse, and writes nothing. Persisting the
    /// fail-closed value would turn a transient error into a permanent fact.
    #[test]
    fn an_unreadable_identity_is_not_written_over() {
        let dir = scratch("identity-io");
        fs::create_dir_all(dir.join(IDENTITY_FILE)).unwrap();
        assert!(save_identity_in(&dir, Identity::default(), no_session).is_err());
        assert!(forget_identity_in(&dir).is_err());
        assert!(dir.join(IDENTITY_FILE).is_dir(), "nothing was written");
        // A reader still answers, and answers closed.
        assert!(load_identity_in(&dir).ever_identified);
    }

    /// The case the round-5 fix is for: the record is fine, a read of it fails
    /// for a reason that passes (here, permissions), and the write path is
    /// still open - `write_file` replaces by rename, so an unreadable file does
    /// not stop it. Writing the fail-closed value over a good record would mark
    /// an install that was never identified as identified, for good.
    #[cfg(unix)]
    #[test]
    fn a_transient_read_error_does_not_change_a_good_record() {
        use std::os::unix::fs::PermissionsExt;
        // Root reads through a 000 mode, which would make this pass vacuously.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let dir = scratch("identity-eacces");
        save_identity_in(
            &dir,
            Identity {
                org_id: Some("org-a".into()),
                auth_mode: Some("api_key".into()),
                ..Identity::default()
            },
            no_session,
        )
        .unwrap();
        let file = dir.join(IDENTITY_FILE);
        fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();

        assert!(save_identity_in(&dir, Identity::default(), no_session).is_err());
        assert!(forget_identity_in(&dir).is_err());

        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        let got = load_identity_in(&dir);
        assert!(
            !got.ever_identified,
            "a good record must not be failed closed by a read error"
        );
        assert_eq!(got.org_id.as_deref(), Some("org-a"));
    }

    /// Review item 3: a store whose `.legacy` could not be written is not left
    /// in place, so the next start judges the install again instead of taking
    /// a half-built store for a fresh one.
    #[test]
    fn a_store_that_could_not_be_finished_is_not_left_behind() {
        let dir = scratch("store-half");
        fs::write(dir.join("account.json"), "{}").unwrap();
        let store = dir.join(STORE_DIR);
        assert!(create_store_with(&store, |_| anyhow::bail!("disk full")).is_err());
        assert!(!store.exists(), "a half-built store was left in place");
        // Nor its staging dir.
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("staging"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        // The next start decides again, and an upgraded install is legacy.
        assert!(!claim_in(&dir, "app_first_launched").unwrap());
        assert!(store.join(LEGACY_MARKER).is_file());
    }

    /// And no claim can see a legacy store before its marker is in it: every
    /// racer on an upgraded install is told the first launch already happened.
    #[test]
    fn concurrent_first_claims_on_an_upgraded_install_never_see_it_fresh() {
        for round in 0..50 {
            let dir = scratch(&format!("store-race-{round}"));
            fs::write(dir.join("account.json"), "{}").unwrap();
            let gate = std::sync::Arc::new(std::sync::Barrier::new(8));
            let wins = std::sync::Arc::new(AtomicUsize::new(0));
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    let (dir, gate, wins) = (dir.clone(), gate.clone(), wins.clone());
                    std::thread::spawn(move || {
                        gate.wait();
                        if claim_in(&dir, "app_first_launched").expect("no claim errors") {
                            wins.fetch_add(1, Ordering::Relaxed);
                        }
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }
            assert_eq!(wins.load(Ordering::Relaxed), 0, "round {round}");
            let _ = fs::remove_dir_all(&dir);
        }
    }
}
