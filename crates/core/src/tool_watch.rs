//! The filesystem watch behind the `tools-changed` event.
//!
//! Detection is the one reading the windows cannot be told about. The engine
//! emits `proxy-state-changed` when routing moves, the gateway pushes security
//! events over its own stream, and every other reading is a fetch the user's own
//! action triggers - but a tool being *installed* happens entirely outside this
//! app, and both shells therefore ran a 5s poll over [`crate::registry`] to
//! notice. This module replaces it: the OS already knows when those files
//! appear, so it says so instead of being asked twelve times a minute.
//!
//! **What is watched is directories, not files.** The interesting paths mostly
//! do not exist yet - `~/.codex/config.toml` is what appears when someone
//! installs Codex - and no backend can watch a path that is not there. So each
//! target is resolved to the deepest directory that *does* exist at or above it
//! ([`arm_dir`]), that directory is watched non-recursively, and every fire
//! re-arms: once `~/.codex` exists it is watched directly, and the events from
//! inside it stop being filtered out by their own parent.
//!
//! Two consequences worth knowing:
//!
//! - **A busy ancestor is normal.** Arming `~` or `/usr/local/bin` means hearing
//!   about every npm install and every dotfile write. [`is_relevant`] drops what
//!   cannot matter, and the debounce collapses what is left, so the cost of a
//!   noisy directory is a filter pass rather than a re-detection.
//! - **`$PATH` is not a path.** Hermes' launcher can sit anywhere on `$PATH`
//!   (see its `launcher_on_path`), and an install there fires nothing. The
//!   window's read on the visibility edge is what still covers it, which is why
//!   dropping the poll did not drop that.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::registry;

/// How long a burst has to go quiet before it is reported.
///
/// Editors and package managers do not write a file once. A `npm i -g` touches a
/// directory dozens of times and an atomic config save is a create, a write and
/// a rename - so without this a single install would emit a dozen
/// `tools-changed` events and the window would run a dozen detection sweeps.
/// 400ms is long enough to swallow those and short enough that the rail repaints
/// while the user is still looking at the terminal they typed the install into.
const QUIET: Duration = Duration::from_millis(400);

/// Every path the registry wants watched, deduplicated.
///
/// Deduplicated because the integrations legitimately overlap - two tools whose
/// binaries live in `/usr/local/bin` resolve to the same armed directory - and
/// arming one directory twice would double every event from it.
pub fn watched_paths() -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    registry()
        .iter()
        .flat_map(|i| i.watch_paths())
        .filter(|p| seen.insert(p.clone()))
        .collect()
}

/// The deepest directory that exists at or above `path`.
///
/// `None` only when nothing above it exists either, which on a real machine means
/// the path was never resolvable - a Windows drive that is not mounted, say.
/// Watching the root as a fallback would be worse than watching nothing.
pub fn arm_dir(path: &Path) -> Option<PathBuf> {
    let mut candidate = if path.is_dir() {
        Some(path)
    } else {
        path.parent()
    };
    while let Some(dir) = candidate {
        if dir.is_dir() {
            return Some(dir.to_path_buf());
        }
        candidate = dir.parent();
    }
    None
}

/// Could an event about `changed` have changed what detection answers?
///
/// Three ways in, and the third is the one that is easy to miss:
///
/// 1. `changed` **is** a target - the settings file was edited.
/// 2. `changed` is **inside** a target - a target directory got a file, which is
///    what `~/.claude` existing is made of.
/// 3. `changed` is **above** a target - `~/.codex` was just created, so the
///    target inside it is now reachable and the watch has to re-arm even though
///    nothing about the target itself has happened yet.
///
/// Everything else is a neighbour's business: the npm install into
/// `/usr/local/bin` that armed directory also reports.
pub fn is_relevant(changed: &Path, targets: &[PathBuf]) -> bool {
    targets
        .iter()
        .any(|t| changed == t || changed.starts_with(t) || t.starts_with(changed))
}

/// Every target, in each spelling the OS may report it under: as declared, and
/// fully resolved.
///
/// **macOS is why, and it cost a CI run to find.** FSEvents reports canonical
/// paths - `/private/var/...` for anything under the `/var` symlink,
/// `/private/tmp` for `/tmp` - and notify's fsevent backend canonicalises the
/// directory it registers, so events arrive spelled differently from the path
/// they were asked for. [`is_relevant`] is prefix arithmetic, and two spellings
/// of one file share no prefix, so **every** event was dropped as a neighbour's
/// business and the watch reported nothing at all. Linux hid it completely:
/// inotify reports paths exactly as registered, and `/tmp` is not a symlink
/// there.
///
/// Precomputed rather than canonicalising each event, which would put a
/// `realpath` syscall on the path of every write in a busy armed directory - a
/// package upgrade in `/usr/bin` is thousands of them - and would block on a
/// stale network mount while holding the burst.
///
/// A target whose resolved form is identical adds nothing, which is the common
/// case: nobody's `~/.codex` is a symlink.
fn spellings(targets: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = targets.to_vec();
    for target in targets {
        // Resolved through the deepest directory that exists, because a path
        // that is not there yet cannot be canonicalised - and not existing yet
        // is the normal state of these.
        let Some(dir) = arm_dir(target) else { continue };
        let Ok(real) = dir.canonicalize() else {
            continue;
        };
        if real == dir {
            continue;
        }
        let resolved = match target.strip_prefix(&dir) {
            Ok(suffix) => real.join(suffix),
            Err(_) => continue,
        };
        if !out.contains(&resolved) {
            out.push(resolved);
        }
    }
    out
}

/// Start watching, for the life of the process.
///
/// No handle comes back on purpose, mirroring the engine's observers
/// (`proxy::set_engine_crash_observer`): there is exactly one caller, it wants
/// the watch for as long as the app is up, and a handle it had to hold somewhere
/// would only be a way to drop the watch by accident. The threads are the
/// watcher's own plus one debounce thread.
///
/// `on_change` is called after a relevant burst goes quiet - never on start-up,
/// because nothing has changed yet and the shells read at mount anyway.
pub fn start(on_change: impl Fn() + Send + 'static) -> Result<()> {
    watch_targets(watched_paths(), on_change)
}

/// The body of [`start`], over an explicit target list so a test can point it at
/// a directory it owns rather than at the user's home.
fn watch_targets(targets: Vec<PathBuf>, on_change: impl Fn() + Send + 'static) -> Result<()> {
    if targets.is_empty() {
        // Nothing to watch is not a failure. It is what a machine with no home
        // directory looks like, and the caller has no better answer than the
        // window's read on focus.
        return Ok(());
    }

    let (tx, rx) = mpsc::channel::<PathBuf>();
    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        // A dropped receiver means the debounce thread is gone, which cannot
        // happen while this process lives - and if it did, there is nobody left
        // to tell. An error from the backend is not actionable either: the
        // paths it concerns are still watched, and the next event still lands.
        if let Ok(event) = res {
            for path in event.paths {
                let _ = tx.send(path);
            }
        }
    })
    .context("starting the tool config watcher")?;
    let watcher = Arc::new(Mutex::new(watcher));

    // Armed once here so a failure to arm anything at all is reported to the
    // caller, rather than discovered later as a watch that never fires. The
    // record moves into the thread with it: re-deriving there would arm every
    // directory a second time, and a doubled watch doubles every event.
    let mut arms: HashSet<PathBuf> = HashSet::new();
    if arm(&watcher, &targets, &mut arms)? == 0 {
        anyhow::bail!(
            "no directory could be watched for {} tool paths",
            targets.len()
        );
    }

    std::thread::Builder::new()
        .name("gate-tool-watch".into())
        .spawn(move || {
            let mut arms = arms;
            // Both spellings, because an event may name a path we watched under
            // its resolved form - see `spellings`.
            let mut names = spellings(&targets);
            loop {
                // Block until something happens, then keep draining until the
                // burst goes quiet. A `Disconnected` here means the watcher was
                // dropped, which ends the thread with it.
                let Ok(first) = rx.recv() else { return };
                let mut relevant = is_relevant(&first, &names);
                loop {
                    match rx.recv_timeout(QUIET) {
                        Ok(path) => relevant |= is_relevant(&path, &names),
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                if !relevant {
                    continue;
                }
                // Before reporting: a directory that appeared during the burst
                // is armed now, so the *next* change inside it is heard - and
                // re-spelled with it, in case what appeared is itself a link.
                let _ = arm(&watcher, &targets, &mut arms);
                names = spellings(&targets);
                on_change();
            }
        })
        .context("spawning the tool config watch thread")?;

    Ok(())
}

/// Arm the deepest existing directory for each target, skipping the ones already
/// armed. Returns how many are armed in total.
///
/// Failures per directory are swallowed: a path that cannot be watched (a
/// permission, a vanished directory, a backend limit) must not cost the other
/// tools their watch. The count is what tells the caller whether anything worked.
fn arm(
    watcher: &Arc<Mutex<RecommendedWatcher>>,
    targets: &[PathBuf],
    armed: &mut HashSet<PathBuf>,
) -> Result<usize> {
    let mut watcher = watcher
        .lock()
        .map_err(|_| anyhow::anyhow!("the tool config watcher is poisoned"))?;
    for dir in targets.iter().filter_map(|t| arm_dir(t)) {
        if !armed.contains(&dir) && watcher.watch(&dir, RecursiveMode::NonRecursive).is_ok() {
            armed.insert(dir);
        }
    }
    Ok(armed.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gate-tool-watch-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn arms_the_deepest_directory_that_exists() {
        let root = scratch("arm");
        // The shape every integration actually has: a config file under a
        // directory the tool has not created yet.
        let target = root.join("dot-codex/config.toml");
        assert_eq!(arm_dir(&target).as_deref(), Some(root.as_path()));

        std::fs::create_dir_all(root.join("dot-codex")).unwrap();
        assert_eq!(
            arm_dir(&target).as_deref(),
            Some(root.join("dot-codex").as_path()),
            "once the directory exists the watch moves down to it",
        );
    }

    #[test]
    fn a_directory_target_arms_itself() {
        let root = scratch("self");
        assert_eq!(arm_dir(&root).as_deref(), Some(root.as_path()));
    }

    #[test]
    fn relevance_covers_the_target_its_ancestors_and_its_directory() {
        // The shape every integration declares: the directory whose existence
        // `detect` reads, and the file inside it that `status` reads.
        let targets = vec![
            PathBuf::from("/home/u/.codex"),
            PathBuf::from("/home/u/.codex/config.toml"),
        ];

        assert!(is_relevant(
            Path::new("/home/u/.codex/config.toml"),
            &targets
        ));
        // The ancestor case: `.codex` appearing is what makes the file inside it
        // reachable, and missing it leaves the watch armed one level too high.
        assert!(is_relevant(Path::new("/home/u/.codex"), &targets));
        assert!(is_relevant(Path::new("/home/u"), &targets));
        // Anything inside the watched directory, because the directory itself is
        // a target - its existence is half of what `detect` answers.
        assert!(is_relevant(
            Path::new("/home/u/.codex/history.jsonl"),
            &targets
        ));

        // A neighbour's business, which a busy armed directory reports plenty
        // of and this has to drop.
        assert!(!is_relevant(Path::new("/home/u/.npm/_cacache"), &targets));
        assert!(!is_relevant(Path::new("/usr/local/bin/tsc"), &targets));

        // With only the file declared, a sibling really is irrelevant - which is
        // why an integration declares its directory as well as its file.
        let file_only = vec![PathBuf::from("/home/u/.codex/config.toml")];
        assert!(!is_relevant(
            Path::new("/home/u/.codex/history.jsonl"),
            &file_only
        ));
    }

    #[test]
    fn every_registered_integration_declares_its_paths() {
        // Not a count: the point is that the list is not silently empty, which
        // is what an integration inheriting a default would have produced.
        let paths = watched_paths();
        assert!(!paths.is_empty(), "no tool declared a path to watch");
        let mut seen = HashSet::new();
        for p in &paths {
            assert!(seen.insert(p), "{} is declared twice", p.display());
        }
    }

    /// The macOS failure, reproduced on any OS.
    ///
    /// A symlinked ancestor is what `/var/folders/...` (macOS `TMPDIR`, under
    /// the `/var` -> `/private/var` link) is, and FSEvents reports the resolved
    /// spelling. Without both spellings in the match list every event is dropped
    /// as a neighbour's business and the watch reports nothing - which is exactly
    /// what CI saw, and what Linux cannot see on its own, `/tmp` being a real
    /// directory there.
    ///
    /// Unix only, and not for want of trying on Windows: creating a link there
    /// needs either a privilege or a junction, and `ReadDirectoryChangesW`
    /// reports paths as registered anyway, so the case this covers cannot arise
    /// on that platform. A test that compiled there would pass vacuously.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_ancestor_is_matched_under_both_spellings() {
        let root = scratch("spell");
        let real = root.join("real");
        let link = root.join("link");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let target = link.join("dot-tool/config.toml");
        let names = spellings(&[target.clone()]);

        assert!(names.contains(&target), "the declared spelling is kept");
        assert!(
            names.contains(&real.join("dot-tool/config.toml")),
            "the resolved spelling is added: {names:?}",
        );
        // Which is what makes an event named the resolved way land.
        assert!(is_relevant(&real.join("dot-tool"), &names));
    }

    #[test]
    fn an_unlinked_target_gains_no_second_spelling() {
        let root = scratch("nolink");
        let target = root.join("dot-tool/config.toml");
        // The common case, and it must not double every entry: nobody's
        // `~/.codex` is a symlink.
        assert_eq!(spellings(&[target.clone()]), vec![target]);
    }

    /// The mechanism end to end, including the re-arm: a config file created
    /// under a directory that did not exist when the watch started.
    ///
    /// Slow by nature - FSEvents coalesces on its own latency and every burst
    /// still has to clear the 400ms debounce - so it waits generously rather
    /// than assuming a cadence. It asserts *at least one* call, never a count:
    /// how many bursts the OS reports for one write is the OS's business, and
    /// pinning it is how a watch test becomes flaky.
    #[test]
    fn reports_a_config_file_appearing_under_a_new_directory() {
        let root = scratch("e2e");
        let target = root.join("dot-tool/config.toml");
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        watch_targets(vec![target.clone()], move || {
            seen.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();

        // Let the backend's stream come up before touching anything. `watch`
        // returning is not the same as the OS delivering: FSEvents starts a
        // stream on its own run loop, and a write in that window is simply not
        // reported. It costs nothing in production - the watch starts at launch,
        // hours before anyone installs a tool - and it is the difference between
        // a test that means something and one that races.
        std::thread::sleep(Duration::from_millis(500));

        // Two steps on purpose: the directory is what the watch has to re-arm
        // on, and the file inside it is what it can only hear about afterwards.
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "[gate]\n").unwrap();

        // Re-touched on a 2s cadence, not waited out in silence: if the first
        // burst was missed anyway there has to be a second, and 2s leaves 1.6s
        // of quiet for the debounce to close. An earlier version wrote every
        // 200ms and never finished at all - each write restarted the debounce,
        // so the burst it was waiting for could not go quiet until it gave up.
        let deadline = Instant::now() + Duration::from_secs(12);
        let mut nudged = Instant::now();
        while calls.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
            if nudged.elapsed() >= Duration::from_secs(2) {
                std::fs::write(&target, "[gate]\n").unwrap();
                nudged = Instant::now();
            }
        }

        assert!(
            calls.load(Ordering::SeqCst) > 0,
            "the watch reported nothing for a file created under a watched target.\n\
             target: {target:?}\narmed: {armed:?}\nspellings: {names:?}",
            armed = arm_dir(&target),
            names = spellings(&[target.clone()]),
        );
    }
}
