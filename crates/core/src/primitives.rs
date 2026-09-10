//! Small native primitives shared across integrations.
//!
//! `write_file` is cross-platform. Below it are the privileged-execution
//! and shell-quoting helpers the proxy subsystem uses for its elevated
//! steps (macOS/Linux), plus the cached install-id for attribution (all
//! platforms: every OS the app ships on has an activity view to feed).

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::path::PathBuf;
use std::process::Command;

/// Run a command to completion, or kill it once `timeout` has passed.
///
/// `Ok(None)` is the timeout: the child was signalled and reaped, and the caller
/// decides what a non-answer means. Everything else is the ordinary
/// [`std::process::Command::output`] contract, stdin included: it is set to
/// [`std::process::Stdio::null`] here the way `output()` sets it, so a child
/// that would prompt gets EOF rather than the caller's terminal. A caller that
/// wants otherwise sets stdin on the `Command` after this returns it - none do.
///
/// **Why this exists at all.** Two of the subprocesses this app shells out to
/// live on paths the window polls - `certutil` against an NSS database, and
/// `gsettings get` against the session's proxy schema - and neither hangs for a
/// reason the app can see. A locked database, a stalled network mount, a dbus
/// peer that does not answer: each of them blocks in the open, on a path where
/// somebody is waiting on a switch.
///
/// Three private copies of this loop had grown before it: `ca_windows`,
/// `ca_linux`, and `integrations::binaries`' version probe. Two of them are now
/// this one. `ca_windows::certutil_bounded` keeps its own on purpose and its
/// doc says why: it nulls all three stdio handles rather than piping, and it
/// deliberately does not `wait()` after the kill, because reaping is a Unix
/// concern and waiting on a `TerminateProcess` that failed would reintroduce
/// the hang it exists to remove.
///
/// **The child is reaped after the kill.** `Child::kill` signals and
/// `Child::drop` deliberately does not wait, so on Unix a killed child stays a
/// zombie for the life of the parent - harmless once, and the certutil caller
/// reaches it per database per enable. Only the direct child: a caller that
/// spawns a shell which forks leaves the grandchild behind, and every caller
/// here runs a leaf binary for that reason.
///
/// **Only for commands whose output is small.** stdout and stderr are piped and
/// not read until the child exits, so a child that writes more than the pipe
/// buffer holds blocks on the write, never exits, and is reported as a hang.
/// Nothing here caps the bytes: the deadline bounds how long a fast writer gets,
/// and a caller that puts the output somewhere a person reads truncates it
/// itself (`ca_linux`'s `one_line_capped`). Every caller emits at most a
/// certificate or a settings value. A command with unbounded output needs a
/// reader thread instead.
pub fn output_bounded(
    mut cmd: Command,
    timeout: std::time::Duration,
) -> std::io::Result<Option<std::process::Output>> {
    /// First gap between `try_wait` polls, doubling to [`POLL_MAX`].
    ///
    /// A flat 50ms - the value the `ca_*` copies used - is a floor on every
    /// call, because a freshly spawned child has essentially never exited by
    /// the first `try_wait`. That is paid six times over on `gsettings_capture`
    /// and once per database per enable on `certutil`, all on a path where
    /// somebody is waiting on a switch. Starting short and backing off costs a
    /// few extra wakeups on a call that was going to be slow anyway, and
    /// nothing on the ones that answer in single-digit milliseconds - which is
    /// every one of them on a healthy machine. `integrations::binaries` polled
    /// at 10ms flat for the same reason before it moved here.
    const POLL_FIRST: std::time::Duration = std::time::Duration::from_millis(1);
    /// Ceiling for the backoff, so a genuinely stuck child is not spun on.
    const POLL_MAX: std::time::Duration = std::time::Duration::from_millis(50);

    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let deadline = std::time::Instant::now() + timeout;
    let mut poll = POLL_FIRST;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            // Kill and reap before propagating. Returning `?` straight out left
            // the one path in here that creates the zombie the rest of this
            // function exists to avoid.
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            // Reaped, not just signalled. See the note above.
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(poll);
        poll = (poll * 2).min(POLL_MAX);
    }
    child.wait_with_output().map(Some)
}

/// The system directories a session tool is looked for in, before falling back
/// to a `PATH` search. Ordered as a distro would: `/usr/bin` holds `certutil`
/// and `gsettings` on Debian, Ubuntu, Fedora and Arch alike, and `/bin` is a
/// symlink to it on all four.
///
/// Deliberately not `/usr/local/bin`: it is the one standard directory that is
/// group-writable on a number of setups, which is the property this helper
/// exists to avoid. A tool that really lives there is still found, through the
/// `PATH` fallback.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const SYSTEM_BIN_DIRS: [&str; 2] = ["/usr/bin", "/bin"];

/// Resolve `program` to an absolute path under [`SYSTEM_BIN_DIRS`], or hand
/// back the bare name for `Command` to search `PATH` for.
///
/// `Command::new("certutil")` re-runs the `PATH` search in the child at spawn
/// time, so a directory earlier on `PATH` than `/usr/bin` decides what we
/// execute - and `~/.local/bin` and `~/bin` are on the default `PATH` on both
/// Debian and Fedora. Same user, so this is not a privilege boundary; it is the
/// standard the surrounding code already holds, `ca_linux` building its
/// escalated script out of `/bin/mkdir` and `/usr/bin/install`, and
/// `integrations::binaries` spawning the absolute path it resolved rather than
/// the name.
///
/// The fallback is deliberate rather than a hole left open: a distro that keeps
/// its tools somewhere else entirely must still work, and the alternative to
/// searching `PATH` there is reporting a tool as missing when it is installed.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn system_program(program: &str) -> PathBuf {
    SYSTEM_BIN_DIRS
        .iter()
        .map(|dir| Path::new(dir).join(program))
        .find(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(program))
}

/// Write `bytes` to `path` atomically: stage to a sibling tempfile,
/// fsync, chmod, then rename into place. A crash mid-write leaves either
/// the old file intact or no file at all -- never a torn destination.
/// Creates parent dirs as needed. On Unix the file ends up with permissions
/// `mode`; on Windows `mode` is ignored (Windows uses ACLs, and the file
/// inherits its parent dir's ACL).
pub fn write_file(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    // If `path` is a symlink (e.g. ~/.claude/settings.json), resolve it so we
    // rewrite the real target and leave the link intact instead of replacing
    // it with a regular file. But refuse a link that redirects the write OUT
    // of its own directory: the payload carries the Gate key, and a config
    // path symlinked into synced storage (iCloud/Dropbox) or elsewhere would
    // send the key off the machine. A link that stays in the same directory is
    // benign and still followed.
    let dest = match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            let target = fs::canonicalize(path)
                .with_context(|| format!("resolving symlink {}", path.display()))?;
            // Canonicalize both sides so the comparison is apples-to-apples
            // (both extended-form `\\?\...` on Windows).
            let link_dir = path
                .parent()
                .map(fs::canonicalize)
                .transpose()
                .with_context(|| format!("resolving parent of {}", path.display()))?;
            if target.parent().map(Path::to_path_buf) != link_dir {
                anyhow::bail!(
                    "refusing to write Gate credentials through symlink {} which \
                     resolves outside its directory to {}; replace it with a \
                     regular file",
                    path.display(),
                    target.display()
                );
            }
            target
        }
        _ => path.to_path_buf(),
    };
    let path: &Path = &dest;
    // Always apply the requested `mode`, including on overwrite: callers pass
    // 0o600/0o700 because the payload carries the Gate key, and the target may
    // pre-exist with the tool's own looser umask (commonly 0o644).
    #[cfg(not(unix))]
    let _ = mode;
    let parent = path
        .parent()
        .with_context(|| format!("path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("create_dir_all {}", parent.display()))?;

    use std::time::{SystemTime, UNIX_EPOCH};
    let file_name = path
        .file_name()
        .with_context(|| format!("path has no file name: {}", path.display()))?
        .to_string_lossy()
        .into_owned();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = parent.join(format!(
        ".{file_name}.gate-connect.{pid}.{nanos}.tmp",
        pid = std::process::id(),
    ));

    let write_then_rename = || -> Result<()> {
        use std::io::Write;
        let mut opts = fs::OpenOptions::new();
        opts.create_new(true).write(true);
        // Create the tempfile already at the requested mode so the payload
        // (which may carry the Gate key) is never world-readable, even
        // transiently under a permissive umask. The set_permissions below
        // still runs to guarantee the exact mode regardless of umask.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(mode);
        }
        let mut f = opts
            .open(&tmp)
            .with_context(|| format!("creating tempfile {}", tmp.display()))?;
        f.write_all(bytes)
            .with_context(|| format!("writing tempfile {}", tmp.display()))?;
        f.sync_all()
            .with_context(|| format!("fsync {}", tmp.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(fs::Permissions::from_mode(mode))
                .with_context(|| format!("chmod {:o} {}", mode, tmp.display()))?;
        }
        drop(f);
        fs::rename(&tmp, path)
            .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
        Ok(())
    };
    match write_then_rename() {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------
// Privileged-execution + shell-quoting helpers for the proxy subsystem
// (macOS via sudo/osascript, Linux via sudo/pkexec), plus the cached
// install-id used for gateway telemetry attribution.
// ---------------------------------------------------------------------

#[cfg(target_os = "macos")]
pub(crate) fn run_as_admin(shell_cmd: &str) -> Result<()> {
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() {
        let status = Command::new("/usr/bin/sudo")
            .args(["/bin/sh", "-c", shell_cmd])
            .status()
            .context("invoking sudo")?;
        if !status.success() {
            anyhow::bail!("sudo command exited non-zero");
        }
    } else {
        // AppleScript's "do shell script" wants the inner command as a
        // double-quoted string with backslashes and quotes escaped.
        let inner = shell_cmd.replace('\\', "\\\\").replace('"', "\\\"");
        let applescript = format!("do shell script \"{inner}\" with administrator privileges");
        // Capture output rather than inheriting: osascript is non-interactive
        // from this process's perspective (the auth dialog is GUI), and its
        // stderr is the only way to tell a genuine cancel ("User canceled.
        // (-128)") from the inner command failing after a correct password.
        let out = Command::new("/usr/bin/osascript")
            .args(["-e", &applescript])
            .output()
            .context("invoking osascript")?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stderr = stderr.trim();
            if stderr.contains("-128") {
                anyhow::bail!("the administrator authorization dialog was cancelled");
            }
            anyhow::bail!(
                "osascript command exited non-zero: {}",
                if stderr.is_empty() {
                    "(no stderr)"
                } else {
                    stderr
                }
            );
        }
    }
    Ok(())
}

/// Run a shell command as root on Linux. In a terminal (CLI usage) we use
/// `sudo`, which caches credentials so a batch of privileged steps prompts
/// once; in a GUI session (no controlling tty) we use `pkexec`, which pops the
/// polkit authentication dialog - the Linux analogue of the macOS osascript
/// admin path. The proxy subsystem's privileged steps (writing
/// `/etc/environment`, installing the CA into the system trust store) go
/// through here. A cancelled/denied prompt makes the helper exit non-zero.
#[cfg(target_os = "linux")]
pub(crate) fn run_as_admin(shell_cmd: &str) -> Result<()> {
    use std::io::IsTerminal;
    let prog = if std::io::stdout().is_terminal() {
        "sudo"
    } else {
        "pkexec"
    };
    let status = Command::new(prog)
        .args(["/bin/sh", "-c", shell_cmd])
        .status()
        .with_context(|| format!("invoking {prog}"))?;
    if !status.success() {
        anyhow::bail!("{prog} command exited non-zero (cancelled or denied?)");
    }
    Ok(())
}

/// Run a shell command as root **without ever prompting**: directly when this
/// process is already root, and via `sudo -n` otherwise.
///
/// This is the escalation for the headless CA-trust path (`--system-trust`),
/// and it deliberately shares nothing with [`run_as_admin`]. Both of that
/// helper's branches are prompting branches - a tty `sudo` asks for a password
/// and the GUI branch (osascript / pkexec) opens a dialog - so on a build agent
/// or a headless Mac mini they hang or fail with nobody to answer. `sudo -n`
/// returns immediately instead, and the caller turns that into an error saying
/// how to re-run.
///
/// stdin is closed for the same reason: an unexpected password prompt must fail
/// fast rather than block a CI job on a read that will never be answered.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn run_as_root_noninteractive(shell_cmd: &str) -> Result<()> {
    // SAFETY: geteuid never fails.
    let already_root = unsafe { libc::geteuid() } == 0;
    let mut cmd = if already_root {
        let mut c = Command::new("/bin/sh");
        c.args(["-c", shell_cmd]);
        c
    } else {
        let mut c = Command::new("sudo");
        c.args(["-n", "/bin/sh", "-c", shell_cmd]);
        c
    };
    let out = cmd
        .stdin(std::process::Stdio::null())
        .output()
        .with_context(|| {
            if already_root {
                "invoking /bin/sh".to_string()
            } else {
                "invoking sudo -n".to_string()
            }
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stderr = stderr.trim();
        if !already_root {
            anyhow::bail!(
                "this needs root and sudo would have prompted; re-run as root (or with a passwordless sudo rule){}",
                if stderr.is_empty() {
                    String::new()
                } else {
                    format!(": {stderr}")
                }
            );
        }
        anyhow::bail!(
            "privileged command exited non-zero: {}",
            if stderr.is_empty() {
                "(no stderr)"
            } else {
                stderr
            }
        );
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn sh_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// Stable UUID identifying this install, generated once and cached at
/// `<app_support_dir>/install-id`.
///
/// Independent of consent and of any analytics client: Settings shows it as the
/// Install ID, and it is the one identifier a person can read off a machine that
/// has never sent anything anywhere. That is why it is not the PostHog distinct
/// id - that one is absent in a build with no project key, and absent again the
/// moment somebody opts out of diagnostics, which would blank a row that has
/// nothing to do with the choice they made.
///
/// **It is sent now.** `inject_attribution` stamps it on every routed request as
/// `x-gate-install-id`, so the activity view can group traffic by machine. That
/// makes it the machine's *self-asserted* identity: it groups requests and
/// authorizes nothing, anyone can forge it, and the gateway treats it
/// accordingly. Being sent independently of the diagnostics consent is the point
/// of the paragraph above, not an oversight - it is routing metadata, not
/// telemetry - but it does mean the disclosure copy has to name it.
pub fn install_id() -> Result<String> {
    let path = crate::env::app_support_dir()?.join("install-id");
    if let Ok(s) = fs::read_to_string(&path) {
        let s = s.trim().to_string();
        if !s.is_empty() {
            return Ok(s);
        }
    }
    let id = simple_uuid_v4()?;
    // Not a secret (an anonymous attribution id), written 0600 via
    // `write_file` anyway so every state file here follows one convention.
    write_file(&path, id.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(id)
}

/// The install id, resolved once per process, or `None` if it could not be.
///
/// Every caller is on the request path, so this must never be the reason a
/// request fails: an unreadable or unwritable data dir degrades to unattributed
/// traffic, which is exactly what the gateway saw before attribution existed.
/// Caching also keeps the file read off the hot path - the id cannot change
/// while the process runs.
pub fn install_id_cached() -> Option<&'static str> {
    static ID: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    ID.get_or_init(|| match install_id() {
        Ok(id) => Some(id),
        Err(e) => {
            eprintln!("[gate] install id unavailable, requests will be unattributed: {e:#}");
            None
        }
    })
    .as_deref()
}

/// Tiny inline v4 generator, so one call does not pull in `uuid`.
///
/// `rand` rather than `/dev/urandom`, which was the reason this and
/// [`install_id`] were macOS-only: Windows has no such device, so the id could
/// not exist on two of the three platforms Gate ships to. `rand` is already a
/// dependency - `oauth` mints the PKCE verifier with it.
fn simple_uuid_v4() -> Result<String> {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format!(
  "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
  bytes[0], bytes[1], bytes[2], bytes[3],
  bytes[4], bytes[5],
  bytes[6], bytes[7],
  bytes[8], bytes[9],
  bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
  ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A command that answers comes back whole.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn a_prompt_command_returns_its_output() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "printf hello"]);
        let out = output_bounded(cmd, std::time::Duration::from_secs(5))
            .expect("spawn")
            .expect("not a timeout");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout), "hello");
    }

    /// One that does not is killed at the deadline, and does not take the
    /// deadline's worth of patience plus the command's.
    ///
    /// The whole point of the helper: `certutil` against a locked database and
    /// `gsettings` against a dbus peer that never answers both block in the call
    /// rather than failing it, on a path where somebody is waiting on a switch.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn a_hanging_command_is_killed_at_the_deadline() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30"]);
        let started = std::time::Instant::now();
        let out = output_bounded(cmd, std::time::Duration::from_millis(300)).expect("spawn");
        let waited = started.elapsed();

        assert!(out.is_none(), "a killed command has no output to report");
        // Bounded well clear of the 300ms deadline but well under the child's
        // own 30s, so a loaded runner does not fail it and a deadline that grew
        // by an order of magnitude does not pass it. The earlier 10s bound
        // would have let a 3s deadline through.
        assert!(
            waited < std::time::Duration::from_secs(5),
            "waited {waited:?}, which means the deadline did not fire"
        );
    }

    /// A command that answers quickly is not held up by the poll gap.
    ///
    /// The regression this pins: a flat 50ms first poll is a floor on every
    /// call, and `gsettings_capture` makes six of them in a row on the enable
    /// path. `sh -c :` exits in single-digit milliseconds, so anything near
    /// 50ms here means the backoff went back to starting at its ceiling.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn a_quick_command_is_not_held_for_a_poll_gap() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", ":"]);
        let started = std::time::Instant::now();
        output_bounded(cmd, std::time::Duration::from_secs(5))
            .expect("spawn")
            .expect("not a timeout");
        let waited = started.elapsed();
        assert!(
            waited < std::time::Duration::from_millis(40),
            "waited {waited:?} for a command that exits immediately"
        );
    }

    /// stdin is closed, the way `Command::output` closes it.
    ///
    /// `certutil` sets this itself and its comment says why - a
    /// password-protected NSS database hands the child the caller's terminal
    /// and blocks there. The helper owes every other caller the same, and
    /// `gsettings_get` lost it silently on the way in here from `.output()`.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn stdin_is_closed_for_the_child() {
        let mut cmd = Command::new("sh");
        // Reads EOF immediately on a null stdin; blocks to the deadline on an
        // inherited one under a terminal.
        cmd.args(["-c", "cat; printf done"]);
        let out = output_bounded(cmd, std::time::Duration::from_secs(5))
            .expect("spawn")
            .expect("not a timeout");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "done");
    }

    /// And it is reaped, not just signalled.
    ///
    /// `Child::kill` signals and `Child::drop` deliberately does not wait, so
    /// the two hand-rolled copies this helper replaced left a zombie for the
    /// life of the process - once per database per enable, on the path that
    /// runs certutil.
    ///
    /// **The child reports its own pid**, rather than the test counting this
    /// process's zombies. That first version passed alone and failed in the
    /// suite: the count is process-wide, the harness runs tests in parallel, and
    /// `integrations::binaries` has a test that abandons a child on purpose. A
    /// pid is the only way to ask about *this* child.
    #[test]
    #[cfg(target_os = "linux")]
    fn a_killed_command_is_reaped() {
        // `TmpDir`, not a hand-built name: the first version interpolated
        // `ThreadId(2)` into the path, and the unquoted parens made `sh` exit on
        // a syntax error - so the child was gone before the deadline and the
        // assertion below failed for a reason that had nothing to do with the
        // reap. Quoted here as well.
        let dir = TmpDir::new("reap");
        let pidfile = dir.0.join("pid");
        let mut cmd = Command::new("sh");
        // `$$` is the shell's own pid, which is the child spawned below.
        cmd.args([
            "-c",
            &format!("echo $$ > '{}'; sleep 30", pidfile.display()),
        ]);

        assert!(output_bounded(cmd, std::time::Duration::from_millis(500))
            .expect("spawn")
            .is_none());

        let pid = fs::read_to_string(&pidfile)
            .expect("the child wrote its pid before the deadline")
            .trim()
            .to_string();
        let _ = fs::remove_file(&pidfile);

        // Reaped means gone from the table entirely. The not-a-zombie branch is
        // for the vanishingly unlikely case that the pid has been reused by the
        // time we look - it must not read as a pass for a zombie either way.
        match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Err(_) => {}
            Ok(stat) => {
                // `stat` is `pid (comm) state ...` and comm can hold spaces and
                // parens, so the state is the field after the LAST ')'.
                let state = stat
                    .rsplit_once(')')
                    .and_then(|(_, rest)| rest.split_whitespace().next().map(str::to_string));
                assert_ne!(
                    state.as_deref(),
                    Some("Z"),
                    "the killed child was left unreaped"
                );
            }
        }
    }

    /// Fresh, unique temp directory for one test; removed on drop.
    struct TmpDir(std::path::PathBuf);
    impl TmpDir {
        fn new(tag: &str) -> Self {
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("gate_pf_{}_{tag}_{n}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            TmpDir(dir)
        }
        fn path(&self, name: &str) -> std::path::PathBuf {
            self.0.join(name)
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn write_file_writes_regular_file() {
        let dir = TmpDir::new("regular");
        let path = dir.path("config.json");
        write_file(&path, b"hello", 0o600).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"hello");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    /// A symlink that stays inside its own directory is benign and still
    /// followed: the real target receives the bytes.
    #[cfg(unix)]
    #[test]
    fn write_file_follows_same_dir_symlink() {
        let dir = TmpDir::new("samedir");
        let real = dir.path("config.real.json");
        let link = dir.path("config.json");
        fs::write(&real, b"old").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        write_file(&link, b"new", 0o600).unwrap();

        assert_eq!(fs::read(&real).unwrap(), b"new");
        // The link itself is left intact, not replaced by a regular file.
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    /// A symlink that redirects the write out of its directory (the
    /// synced-folder threat) is refused before any bytes are written, and the
    /// error names the resolved target.
    #[cfg(unix)]
    #[test]
    fn write_file_refuses_escaping_symlink() {
        let cfg = TmpDir::new("escape_cfg");
        let synced = TmpDir::new("escape_synced");
        let target = synced.path("stolen.json");
        let link = cfg.path("config.json");
        fs::write(&target, b"").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let err = write_file(&link, b"sk-gw-secret", 0o600)
            .expect_err("escaping symlink must be refused");

        let msg = format!("{err:#}");
        assert!(msg.contains("refusing"), "unexpected error: {msg}");
        assert!(
            msg.contains(&target.canonicalize().unwrap().display().to_string()),
            "error should name the resolved target: {msg}"
        );
        // Nothing was written through the link.
        assert_eq!(fs::read(&target).unwrap(), b"");
    }
}
