//! Ask the OS to bring Gate Connect back when it dies from a crash.
//!
//! The exit-time revert (`RunEvent::Exit` -> `ProxyManager::disable_quiet`) is
//! the only thing that puts the system proxy back on macOS and Windows, and an
//! abort skips it: the process is gone before any handler runs. What is left
//! behind fails open, silently unrouted: the PAC and the exported `HTTPS_PROXY`
//! both name the forwarder, which goes direct once the engine is gone, and a
//! browser that refetches the PAC and finds its port dead goes DIRECT itself
//! (`proxy::forwarder`, `engine::pac_script`). Before the forwarder fronted
//! them, the exported variables and the config-routed tools named the engine's
//! own port and failed closed instead.
//!
//! All of that heals at the next launch
//! ([`crate::proxy::ProxyManager::reconcile_on_startup`]), so the whole problem
//! is the length of the gap. [`crate::proxy::autostart_optout`] already shortens
//! it to the next boot by keeping a login item registered across the crash
//! window. This module shortens it to seconds by asking the OS to relaunch us
//! directly, and the two are deliberately complementary: that one covers "the
//! machine restarted", this one covers "the app died and the machine did not".
//!
//! Relaunching rather than merely cleaning up is the point. A silent revert
//! leaves the user unprotected with nothing on screen saying so, and coming
//! back restores the routing they asked for as well as reverting what the crash
//! stranded.
//!
//! **macOS** keeps the policy in the LaunchAgent that
//! `tauri-plugin-autostart` already writes for launch-at-login: a `KeepAlive`
//! that names only `Crashed`, so launchd restarts us after a crash signal and
//! leaves a clean exit alone. Force Quit sends `SIGKILL`, which is not one of
//! those signals, so a deliberate kill stays dead - as it must, or the app
//! would refuse to go away. The shell owns the OS call; only the file edit and
//! the give-up policy live here.
//!
//! **Windows** has no file to edit: the shell asks Windows Error Reporting
//! directly via `RegisterApplicationRestart`. WER honours it for an unhandled
//! exception or a hang and for nothing else; Task Manager and `taskkill` go
//! through `TerminateProcess`, which WER never sees, so a deliberate kill stays
//! dead here too. Only [`record_start`] and [`record_clean_exit`] are shared
//! with that path.
//!
//! **Linux** uses none of this. The engine there is a detached helper daemon
//! that outlives the GUI and drops to pass-through, so a GUI crash strands
//! nothing, and there is no exit handler to mark a clean exit with - every
//! start would look unclean.
//!
//! The give-up rule is the reason this is a module rather than two lines of
//! plist. A crash *on startup* would otherwise loop forever: launchd relaunches,
//! we crash, launchd relaunches, each turn costing a crash report and, on macOS,
//! possibly a keychain prompt. So a session that dies without reaching
//! [`MIN_HEALTHY_SECONDS`] counts towards a streak, [`MAX_UNCLEAN_STARTS`] of
//! them disarms the policy, and any clean exit clears it. A session that ran
//! longer than that and then crashed is not a loop: it resets the streak to one
//! rather than adding to it, so an app crashed once a day is still restarted
//! every time.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Consecutive unclean starts that disarm the restart policy. Three is two
/// retries: enough that a one-off crash on a cold machine still gets its
/// relaunch, few enough that a reproducible startup crash stops quickly.
pub const MAX_UNCLEAN_STARTS: u32 = 3;

/// How long a session must last to count as having got off the ground. Under
/// this, a crash is treated as a startup loop and counts towards the streak.
pub const MIN_HEALTHY_SECONDS: u64 = 60;

/// Seconds launchd waits before relaunching. Its default is 10, which is fast
/// enough to be a tight loop against a crash we have not yet given up on; this
/// keeps the throttle longer than a cold start so a wedged launch cannot spin.
pub const THROTTLE_SECONDS: u32 = 30;

/// What the last session's ending says about this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupVerdict {
    /// The previous session never recorded a clean exit: it crashed, was
    /// killed, or lost power.
    pub previous_was_unclean: bool,
    /// Consecutive unclean starts including this one. Zero after a clean exit.
    pub unclean_streak: u32,
    /// The streak reached [`MAX_UNCLEAN_STARTS`]: stop asking the OS to
    /// relaunch us, because something is failing every time we try.
    pub exhausted: bool,
}

/// Persisted across launches. One file rather than a marker plus a counter:
/// the open-session flag and the streak are written at the same two moments
/// and read at one, so splitting them only adds a way for them to disagree.
#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    /// True between [`record_start`] and [`record_clean_exit`]. Finding it
    /// still true at startup is what "the last session did not exit" means.
    session_open: bool,
    /// Unix milliseconds at which the open session started, for the
    /// [`MIN_HEALTHY_SECONDS`] test.
    started_ms: u64,
    /// Consecutive unclean starts.
    unclean_streak: u32,
}

fn state_path() -> Result<PathBuf> {
    Ok(crate::env::app_support_dir()?.join("crash-restart.json"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn load(path: &Path) -> State {
    // An unreadable or corrupt file reads as a clean slate. The failure that
    // matters is refusing to relaunch a user whose app crashed, not relaunching
    // one extra time after we lost track.
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save(path: &Path, state: &State) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let raw = serde_json::to_string(state).context("serializing crash-restart state")?;
    fs::write(path, raw).with_context(|| format!("writing {}", path.display()))
}

/// Open a session and report what the previous one's ending means. Call once
/// per launch, before anything that can itself crash.
pub fn record_start() -> Result<StartupVerdict> {
    record_start_at(&state_path()?, now_ms())
}

fn record_start_at(path: &Path, now: u64) -> Result<StartupVerdict> {
    let prior = load(path);
    let previous_was_unclean = prior.session_open;
    let unclean_streak = if !previous_was_unclean {
        0
    } else if now.saturating_sub(prior.started_ms) >= MIN_HEALTHY_SECONDS * 1000 {
        // It ran long enough to be working, so this is one crash rather than a
        // loop. Deliberately 1, not 0: the session still ended badly, and a
        // second one straight after IS a loop.
        1
    } else {
        prior.unclean_streak.saturating_add(1)
    };
    save(
        path,
        &State {
            session_open: true,
            started_ms: now,
            unclean_streak,
        },
    )?;
    Ok(StartupVerdict {
        previous_was_unclean,
        unclean_streak,
        exhausted: unclean_streak >= MAX_UNCLEAN_STARTS,
    })
}

/// Close the session cleanly and clear the streak. Call from the exit handler,
/// which is the one place that knows the process is leaving on purpose.
pub fn record_clean_exit() -> Result<()> {
    record_clean_exit_at(&state_path()?)
}

fn record_clean_exit_at(path: &Path) -> Result<()> {
    save(path, &State::default())
}

/// The LaunchAgent `tauri-plugin-autostart` writes for launch-at-login.
///
/// The name is the plugin's: it passes `package_info().name`, which is
/// `productName` from `tauri.conf.json`, and `auto-launch` writes
/// `~/Library/LaunchAgents/<name>.plist`. Passed in rather than hardcoded so
/// the caller that owns the plugin owns the name too, and so the tests can
/// point at a scratch file.
/// Ungated for the same reason as [`arm`]: it is a path join, and gating it
/// would put the one macOS-shaped line in this file beyond the reach of every
/// compiler this repo is developed against.
pub fn launch_agent_plist(app_name: &str) -> Result<PathBuf> {
    // `auto-launch` resolves this with `dirs::home_dir()`; going through
    // `env::home` instead means the test seam reaches it too.
    Ok(launch_agent_plist_in(&crate::env::home()?, app_name))
}

/// The path half, separated so a test can pin the layout without moving the
/// process-global home seam under every other test.
fn launch_agent_plist_in(home: &Path, app_name: &str) -> PathBuf {
    home.join("Library")
        .join("LaunchAgents")
        .join(format!("{app_name}.plist"))
}

/// The keys we add, as one exactly-matched block.
///
/// Matched exactly rather than parsed because there is no plist parser in this
/// crate's dependencies and adding one to edit two keys is not worth it. It is
/// safe because we are the only writer of this block: the plugin's template has
/// neither key, and its enable path truncates the file, so the block is either
/// byte-for-byte ours or absent.
///
/// Built rather than a `const` so [`THROTTLE_SECONDS`] is the only place the
/// interval is written down.
fn keep_alive_block() -> String {
    format!(
        "  <key>KeepAlive</key>\n  \
         <dict>\n    \
         <key>Crashed</key>\n    \
         <true/>\n  \
         </dict>\n  \
         <key>ThrottleInterval</key>\n  \
         <integer>{THROTTLE_SECONDS}</integer>\n  "
    )
}

/// Add the restart policy to an existing LaunchAgent. Returns whether the file
/// changed; a missing plist (launch-at-login off, nothing registered) is
/// `Ok(false)` rather than an error, because there is then nothing to keep
/// alive and no problem to report.
///
/// Idempotent, and it has to be: the plugin rewrites the plist from a fixed
/// template on every enable, so this runs again after each one and at every
/// startup to repair what an enable erased.
///
/// Not gated to macOS even though only macOS calls it: it is filesystem and
/// string work with nothing platform-specific in it, and gating it would mean
/// the tests below never run anywhere this repo is developed.
pub fn arm(plist: &Path) -> Result<bool> {
    let Ok(xml) = fs::read_to_string(plist) else {
        return Ok(false);
    };
    let block = keep_alive_block();
    if xml.contains(&block) {
        return Ok(false);
    }
    // Insert before the LAST `</dict>`, which is the top-level one: the block
    // opens a nested dict of its own, so the outer close is still last after
    // any number of arms.
    let Some(at) = xml.rfind("</dict>") else {
        anyhow::bail!("{} is not a plist we recognise", plist.display());
    };
    let mut out = String::with_capacity(xml.len() + block.len());
    out.push_str(&xml[..at]);
    out.push_str(&block);
    out.push_str(&xml[at..]);
    fs::write(plist, out).with_context(|| format!("writing {}", plist.display()))?;
    Ok(true)
}

/// Take the restart policy back out, leaving the rest of the LaunchAgent alone
/// so the user's launch-at-login choice survives. Returns whether the file
/// changed. Ungated for the same reason as [`arm`].
pub fn disarm(plist: &Path) -> Result<bool> {
    let Ok(xml) = fs::read_to_string(plist) else {
        return Ok(false);
    };
    let block = keep_alive_block();
    if !xml.contains(&block) {
        return Ok(false);
    }
    let out = xml.replace(&block, "");
    fs::write(plist, out).with_context(|| format!("writing {}", plist.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_state(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "gate-crash-restart-test-{}-{name}",
                std::process::id()
            ))
            .join("crash-restart.json")
    }

    fn fresh(name: &str) -> PathBuf {
        let path = temp_state(name);
        let _ = fs::remove_dir_all(path.parent().unwrap());
        path
    }

    const MINUTE_MS: u64 = 60_000;

    #[test]
    fn a_first_ever_start_is_clean() {
        let path = fresh("first");
        let v = record_start_at(&path, MINUTE_MS).unwrap();
        assert!(!v.previous_was_unclean);
        assert_eq!(v.unclean_streak, 0);
        assert!(!v.exhausted);
    }

    #[test]
    fn a_start_after_a_clean_exit_is_clean() {
        let path = fresh("clean");
        record_start_at(&path, MINUTE_MS).unwrap();
        record_clean_exit_at(&path).unwrap();
        let v = record_start_at(&path, 2 * MINUTE_MS).unwrap();
        assert!(!v.previous_was_unclean);
        assert_eq!(v.unclean_streak, 0);
    }

    #[test]
    fn a_start_with_the_session_still_open_is_unclean() {
        let path = fresh("unclean");
        record_start_at(&path, MINUTE_MS).unwrap();
        let v = record_start_at(&path, MINUTE_MS + 1_000).unwrap();
        assert!(v.previous_was_unclean);
        assert_eq!(v.unclean_streak, 1);
        assert!(!v.exhausted);
    }

    #[test]
    fn three_quick_crashes_exhaust_the_policy() {
        let path = fresh("exhaust");
        record_start_at(&path, 0).unwrap();
        assert_eq!(record_start_at(&path, 1_000).unwrap().unclean_streak, 1);
        assert_eq!(record_start_at(&path, 2_000).unwrap().unclean_streak, 2);
        let v = record_start_at(&path, 3_000).unwrap();
        assert_eq!(v.unclean_streak, MAX_UNCLEAN_STARTS);
        assert!(v.exhausted);
    }

    #[test]
    fn a_crash_after_a_healthy_session_does_not_add_to_the_streak() {
        // Two quick crashes, then a session that ran well past the threshold
        // and crashed. That last one is a one-off, not the third turn of a
        // loop, so it must not exhaust the policy.
        let path = fresh("healthy");
        record_start_at(&path, 0).unwrap();
        record_start_at(&path, 1_000).unwrap();
        assert_eq!(record_start_at(&path, 2_000).unwrap().unclean_streak, 2);
        let v = record_start_at(&path, 2_000 + MIN_HEALTHY_SECONDS * 1_000).unwrap();
        assert!(v.previous_was_unclean);
        assert_eq!(v.unclean_streak, 1);
        assert!(!v.exhausted);
    }

    #[test]
    fn a_clean_exit_clears_an_exhausted_streak() {
        let path = fresh("recover");
        record_start_at(&path, 0).unwrap();
        record_start_at(&path, 1_000).unwrap();
        record_start_at(&path, 2_000).unwrap();
        assert!(record_start_at(&path, 3_000).unwrap().exhausted);
        record_clean_exit_at(&path).unwrap();
        let v = record_start_at(&path, 4_000).unwrap();
        assert_eq!(v.unclean_streak, 0);
        assert!(!v.exhausted);
    }

    #[test]
    fn a_corrupt_state_file_reads_as_a_clean_slate() {
        let path = fresh("corrupt");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{not json").unwrap();
        let v = record_start_at(&path, MINUTE_MS).unwrap();
        assert!(!v.previous_was_unclean);
        assert_eq!(v.unclean_streak, 0);
    }

    #[test]
    fn the_launch_agent_path_matches_what_auto_launch_writes() {
        // `auto-launch` 0.5 writes `~/Library/LaunchAgents/{app_name}.plist`,
        // and the plugin passes `package_info().name`, which is `productName`
        // from tauri.conf.json. Miss either and we would arm a file launchd
        // never reads.
        let path = launch_agent_plist_in(Path::new("/Users/someone"), "Gate Connect");
        assert_eq!(
            path,
            Path::new("/Users/someone/Library/LaunchAgents/Gate Connect.plist")
        );
    }

    mod plist {
        use super::*;

        /// Byte-for-byte what `auto-launch` 0.5 writes, so the arm/disarm
        /// round trip is tested against the real input and not a paraphrase.
        const PLUGIN_TEMPLATE: &str = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" ",
            "\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
            "<plist version=\"1.0\">\n  <dict>\n  ",
            "<key>Label</key>\n  <string>Gate Connect</string>\n  ",
            "<key>ProgramArguments</key>\n  ",
            "<array><string>/Applications/Gate Connect.app/Contents/MacOS/gate-connect-desktop",
            "</string><string>--silent</string></array>\n  ",
            "<key>RunAtLoad</key>\n  <true/>\n  </dict>\n</plist>",
        );

        fn temp_plist(name: &str) -> PathBuf {
            let path = std::env::temp_dir()
                .join(format!("gate-plist-test-{}-{name}", std::process::id()))
                .join("Gate Connect.plist");
            let _ = fs::remove_dir_all(path.parent().unwrap());
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            path
        }

        #[test]
        fn arming_adds_the_keys_once() {
            let path = temp_plist("arm");
            fs::write(&path, PLUGIN_TEMPLATE).unwrap();
            assert!(arm(&path).unwrap());
            let armed = fs::read_to_string(&path).unwrap();
            assert!(armed.contains("<key>KeepAlive</key>"));
            assert!(armed.contains("<key>Crashed</key>"));
            assert!(armed.contains("<key>ThrottleInterval</key>"));
            // The plugin's own keys survive.
            assert!(armed.contains("<key>RunAtLoad</key>"));
            assert!(armed.contains("--silent"));
            // The block goes inside the top-level dict, not after it.
            assert!(armed.trim_end().ends_with("</dict>\n</plist>"));
            // Idempotent.
            assert!(!arm(&path).unwrap());
            assert_eq!(fs::read_to_string(&path).unwrap(), armed);
        }

        #[test]
        fn disarming_restores_the_plugin_template() {
            let path = temp_plist("disarm");
            fs::write(&path, PLUGIN_TEMPLATE).unwrap();
            arm(&path).unwrap();
            assert!(disarm(&path).unwrap());
            assert_eq!(fs::read_to_string(&path).unwrap(), PLUGIN_TEMPLATE);
            assert!(!disarm(&path).unwrap());
        }

        #[test]
        fn a_missing_plist_is_not_an_error() {
            let path = temp_plist("missing");
            fs::remove_file(&path).ok();
            assert!(!arm(&path).unwrap());
            assert!(!disarm(&path).unwrap());
        }

        #[test]
        fn an_unrecognisable_plist_is_left_alone() {
            let path = temp_plist("garbage");
            fs::write(&path, "not a plist at all").unwrap();
            assert!(arm(&path).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), "not a plist at all");
        }
    }
}
