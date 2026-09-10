//! Finding the executable a tool actually runs, and asking it its version.
//!
//! Two jobs, one module, because the second is worthless without the first and
//! the first is worth having on its own.
//!
//! # Why the well-known paths are not enough
//!
//! Every integration carries a short list of absolute paths (`/usr/local/bin/x`,
//! `/opt/homebrew/bin/x`) and falls back to "does the config directory exist".
//! That answers *is it installed* and nothing else - a developer machine
//! routinely has the binary somewhere else entirely. Measured on one: Claude
//! Code lives at `~/.local/bin/claude`, which is on none of the lists, so
//! detection succeeds through the config-dir fallback and no path is ever known.
//!
//! # Why PATH is not enough either
//!
//! Gate Connect is a GUI application. A macOS `.app` launched from Finder, or a
//! Linux desktop entry, inherits the session's minimal environment and **not**
//! the user's shell `PATH` - no nvm, no Volta, no `~/.local/bin`. So `PATH` is
//! searched, and then the directories a shell would have added are searched
//! explicitly. That ordering is deliberate: `PATH` is what the user's own shell
//! would resolve, and it wins.
//!
//! # Why the version comes from the binary and not from a file
//!
//! Three cheaper sources exist and two of them lie. Measured on a machine
//! holding both an npm and a native install of the same tool:
//!
//! | source                                   | said     |
//! |------------------------------------------|----------|
//! | npm `package.json`                       | 2.0.13   |
//! | the resolved native path                 | 2.1.263  |
//! | the tool's own last-update record        | 2.1.263  |
//! | `claude --version`                       | 2.1.263  |
//!
//! The npm copy is stale and is not what runs, so reading `package.json` would
//! report a version the user is not running - worse than reporting none, because
//! it sends a support thread after the wrong release. Asking the executable is
//! the only answer that is about the thing that actually executes.
//!
//! It is also cheap, which it did not use to be: these tools ship native
//! binaries now rather than Node entrypoints. Measured at 80ms cold and under a
//! millisecond warm.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

/// How long a `--version` may take before it is abandoned.
///
/// A version is a nice-to-have on every surface that reads one, so the tradeoff
/// is entirely one-sided: a tool that does not answer promptly reports no
/// version, and nothing else about the app waits on it.
const VERSION_TIMEOUT: Duration = Duration::from_secs(2);

/// Directories a login shell would have added and a GUI process will not have.
///
/// Searched after `PATH`, never before: if the user's shell resolves the tool
/// somewhere, that is the copy their terminal runs, and it is the copy whose
/// version and configuration matter.
fn shell_bin_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let Some(home) = dirs_home() else {
        return dirs;
    };
    for rel in [
        ".local/bin",
        ".npm-global/bin",
        ".volta/bin",
        ".bun/bin",
        ".deno/bin",
        ".cargo/bin",
        ".yarn/bin",
    ] {
        dirs.push(home.join(rel));
    }
    dirs
}

fn dirs_home() -> Option<PathBuf> {
    dirs::home_dir()
}

/// Whether this path is a file we could execute.
///
/// Existence only. A permission check would be a second syscall to answer a
/// question the spawn answers anyway, and a binary that is present but not
/// executable is a broken install worth reporting as "found, no version" rather
/// than as "not installed".
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

/// Find the executable a tool would run, or `None`.
///
/// `well_known` is the integration's own absolute list, tried first because it
/// is the packaged install and the one case where a path is certain. Then
/// `PATH`, then the shell directories a GUI process does not inherit.
///
/// `names` carries the platform's spellings: one entry on Unix, several on
/// Windows where a shim may be `.cmd` or `.exe`.
pub fn resolve_binary(well_known: &[&str], names: &[&str]) -> Option<PathBuf> {
    for path in well_known {
        let path = Path::new(path);
        if is_executable_file(path) {
            return Some(path.to_path_buf());
        }
    }

    let path_dirs = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();

    for dir in path_dirs.into_iter().chain(shell_bin_dirs()) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        for name in names {
            let candidate = dir.join(name);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// Ask a resolved executable its version.
///
/// Spawns the **absolute path** rather than a bare name: a `Command::new("x")`
/// re-runs the `PATH` search at spawn time, which is both a different answer
/// from the one we resolved and a way for a directory earlier on `PATH` to
/// decide what we execute.
///
/// stdin is null so a tool that would prompt gets EOF instead of hanging on a
/// terminal it does not have; stderr is captured and dropped, because the
/// version is on stdout and an update notice on stderr is not our business.
///
/// The bound is [`crate::primitives::output_bounded`], which is this loop and
/// the two `ca_*` copies of it written once. What it adds here is the backoff:
/// the hand-rolled version polled at a flat 10ms, and a probe that answers in
/// two is now not held for the other eight.
pub fn binary_version(path: &Path) -> Option<String> {
    let mut cmd = Command::new(path);
    cmd.arg("--version");
    // Killed AND reaped inside the helper: dropping a `Child` does not wait,
    // and a zombie per probe would accumulate for the life of the app.
    let out = crate::primitives::output_bounded(cmd, VERSION_TIMEOUT).ok()??;
    parse_version(&String::from_utf8_lossy(&out.stdout))
}

/// Pull a version out of whatever the tool printed.
///
/// Every shipped tool prints something like `2.1.263 (Claude Code)`,
/// `codex-cli 0.5.1` or `opencode 1.2.3`, so the rule is: the first token that
/// starts with a digit and contains a dot. Deliberately not a semver parser -
/// a prerelease suffix, a four-part version or a `v` prefix are all things we
/// want to report verbatim rather than reject.
///
/// Bounded so a tool that ignores `--version` and prints its help cannot put a
/// paragraph into a diagnostics field.
fn parse_version(out: &str) -> Option<String> {
    out.split_whitespace()
        .map(|tok| tok.trim_start_matches('v'))
        .find(|tok| tok.starts_with(|c: char| c.is_ascii_digit()) && tok.contains('.'))
        .filter(|tok| tok.len() <= 32)
        .map(|tok| tok.trim_end_matches(&[',', ')', ';'][..]).to_string())
}

/// One probed binary: where it was, and when it was last written.
///
/// The mtime is half the key because all of these tools update *in place*, so a
/// path alone would pin the first version this process ever saw and keep serving
/// it after an upgrade.
type ProbeKey = (PathBuf, Option<std::time::SystemTime>);

/// Every tool's version, resolved once per process.
///
/// Keyed by path rather than by slug, so two integrations pointing at one binary
/// share the answer.
///
/// A `None` value means "we looked and it would not say", which is a different
/// finding from an absent key ("we have not looked"), and both differ from a
/// version. The callers print all three differently.
static VERSIONS: std::sync::RwLock<Option<std::collections::HashMap<ProbeKey, Option<String>>>> =
    std::sync::RwLock::new(None);

/// The version of the binary at `path`, or `None` if it would not say.
///
/// **Never call this on a render path.** It spawns a process on a cache miss,
/// which is the same rule `routing_verdicts` follows and for the same reason:
/// the sidebar repaints far more often than a version changes.
pub fn version_at(path: &Path) -> Option<String> {
    let stamp = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let key = (path.to_path_buf(), stamp);

    if let Ok(guard) = VERSIONS.read() {
        if let Some(found) = guard.as_ref().and_then(|m| m.get(&key)) {
            return found.clone();
        }
    }

    let version = binary_version(path);
    if let Ok(mut guard) = VERSIONS.write() {
        guard
            .get_or_insert_with(Default::default)
            .insert(key, version.clone());
    }
    version
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_common_shapes() {
        // The three the shipped tools actually print.
        assert_eq!(
            parse_version("2.1.263 (Claude Code)").as_deref(),
            Some("2.1.263")
        );
        assert_eq!(parse_version("codex-cli 0.5.1").as_deref(), Some("0.5.1"));
        assert_eq!(parse_version("opencode 1.2.3\n").as_deref(), Some("1.2.3"));
    }

    #[test]
    fn keeps_a_prerelease_and_drops_a_v() {
        // Reported verbatim rather than normalised: "which build is this" is
        // the question, and a suffix is usually the answer to it.
        assert_eq!(
            parse_version("v1.2.3-beta.4").as_deref(),
            Some("1.2.3-beta.4")
        );
        assert_eq!(parse_version("tool 10.0.0.1").as_deref(), Some("10.0.0.1"));
    }

    #[test]
    fn refuses_prose_and_bare_numbers() {
        // A tool that ignores `--version` and prints help must not put a
        // sentence into the field, and a lone integer is not a version.
        assert_eq!(parse_version("Usage: tool [options]"), None);
        assert_eq!(parse_version("build 12345"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn refuses_an_absurdly_long_token() {
        let long = format!("1.{}", "9".repeat(64));
        assert_eq!(parse_version(&long), None);
    }

    #[test]
    fn resolves_a_well_known_path_before_searching() {
        // Every platform has this one, and it is the cheapest way to prove the
        // well-known list short-circuits the PATH walk.
        #[cfg(unix)]
        {
            let found = resolve_binary(&["/bin/sh"], &["definitely-not-a-real-binary"]);
            assert_eq!(found.as_deref(), Some(Path::new("/bin/sh")));
        }
    }

    #[test]
    #[cfg(unix)]
    fn finds_a_binary_that_is_only_on_path() {
        // The case the old detection missed entirely: installed, on PATH, and
        // on none of the packaged absolute paths. Takes `path_env_lock`
        // because libtest runs these as threads of one process and PATH is
        // shared ground - see `env::path_env_lock`.
        let _guard = crate::env::path_env_lock();
        let tool = fake_tool("onpath", "echo '9.9.9'");
        let dir = tool.parent().unwrap().to_path_buf();

        let before = std::env::var_os("PATH");
        let mut dirs: Vec<PathBuf> = before
            .as_ref()
            .map(|p| std::env::split_paths(p).collect())
            .unwrap_or_default();
        dirs.push(dir.clone());
        std::env::set_var("PATH", std::env::join_paths(dirs).expect("join"));

        let found = resolve_binary(&[], &["faketool"]);

        match before {
            Some(p) => std::env::set_var("PATH", p),
            None => std::env::remove_var("PATH"),
        }

        assert_eq!(found.as_deref(), Some(tool.as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn returns_none_when_nothing_matches() {
        assert!(resolve_binary(&[], &["gate-connect-no-such-binary-xyz"]).is_none());
    }

    #[test]
    fn a_missing_binary_has_no_version() {
        assert!(binary_version(Path::new("/gate-connect/definitely/not/here")).is_none());
    }

    /// A throwaway executable that prints what we tell it to, so the spawn path
    /// is pinned against a known answer rather than against whatever the host
    /// happens to have installed.
    #[cfg(unix)]
    fn fake_tool(tag: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("gate-binaries-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("faketool");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    #[test]
    #[cfg(unix)]
    fn asks_a_real_binary_its_version() {
        let tool = fake_tool("ok", "echo '2.1.263 (Claude Code)'");
        assert_eq!(binary_version(&tool).as_deref(), Some("2.1.263"));
        let _ = std::fs::remove_dir_all(tool.parent().unwrap());
    }

    #[test]
    #[cfg(unix)]
    fn a_tool_that_prints_no_version_reports_none() {
        // Not a panic, and not an invented value: the field is absent.
        let tool = fake_tool("prose", "echo 'Usage: faketool [options]'");
        assert_eq!(binary_version(&tool), None);
        let _ = std::fs::remove_dir_all(tool.parent().unwrap());
    }

    #[test]
    #[cfg(unix)]
    fn a_hanging_tool_is_abandoned_not_waited_on() {
        // The whole reason for the timeout. `sleep 30` would outlast any
        // patience the caller has; this must come back promptly with None.
        let tool = fake_tool("hang", "sleep 30");
        let started = Instant::now();
        assert_eq!(binary_version(&tool), None);
        assert!(
            started.elapsed() < VERSION_TIMEOUT + Duration::from_secs(1),
            "took {:?}, should have given up at {VERSION_TIMEOUT:?}",
            started.elapsed()
        );
        let _ = std::fs::remove_dir_all(tool.parent().unwrap());
    }

    #[test]
    #[cfg(unix)]
    fn a_failing_tool_still_yields_what_it_printed() {
        // Several CLIs print their version and exit non-zero on an unknown
        // flag. The exit status is not the answer; the output is.
        let tool = fake_tool("rc", "echo '1.4.0'; exit 1");
        assert_eq!(binary_version(&tool).as_deref(), Some("1.4.0"));
        let _ = std::fs::remove_dir_all(tool.parent().unwrap());
    }
}
