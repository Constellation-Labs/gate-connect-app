//! Small native primitives shared across integrations.
//!
//! `write_file` is cross-platform. Everything below it (plist, privileged
//! writes, process scans, org-plugins install, install-id) is used only
//! by the macOS-only Cowork integration and is gated to match.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

#[cfg(target_os = "macos")]
use plist::Value;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::process::Command;

/// Write `bytes` to `path` atomically: stage to a sibling tempfile,
/// fsync, chmod, then rename into place. A crash mid-write leaves either
/// the old file intact or no file at all -- never a torn destination.
/// Creates parent dirs as needed. On Unix the file ends up with permissions
/// `mode`; on Windows `mode` is ignored (Windows uses ACLs, and the file
/// inherits its parent dir's ACL).
pub fn write_file(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    // If `path` is a symlink (e.g. ~/.claude/settings.json linked to a synced
    // location), resolve it so we rewrite the real target and leave the link
    // intact instead of replacing it with a regular file.
    let dest = match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => fs::canonicalize(path)
            .with_context(|| format!("resolving symlink {}", path.display()))?,
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
// macOS-only helpers below. Used by the Cowork integration and the
// standard-mode → 3P-mode migration. Both depend on macOS subsystems
// (Managed Preferences plist, AppleScript for elevation, the
// `lsof`/`pgrep` family of Darwin process tools).
// ---------------------------------------------------------------------

/// Write an XML plist to `path`. Returns the bytes written so the caller
/// can hand them to a privileged helper instead of writing directly.
#[cfg(target_os = "macos")]
pub fn plist_bytes(value: &Value) -> Result<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();
    value
        .to_writer_xml(&mut buf)
        .context("serializing plist to XML")?;
    Ok(buf)
}

/// Stage `bytes` to a user-owned tempfile and then `mv` it into `path`
/// under administrator privileges. CLI mode uses `sudo`; GUI mode uses
/// AppleScript's `do shell script with administrator privileges`, which
/// pops the macOS authentication panel.
#[cfg(target_os = "macos")]
pub fn write_file_privileged(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = path.parent().context("path has no parent")?;
    let tmp = stage_tempfile(bytes)?;

    let script = format!(
        "/bin/mkdir -p {parent} && /bin/mv -f {tmp} {dest} && /bin/chmod {mode:o} {dest}",
        parent = sh_quote(&parent.display().to_string()),
        tmp = sh_quote(&tmp.display().to_string()),
        dest = sh_quote(&path.display().to_string()),
        mode = mode,
    );
    run_as_admin(&script).with_context(|| format!("privileged write to {}", path.display()))?;
    // Best effort: tempfile is moved on success, but clean up if it survived.
    let _ = fs::remove_file(&tmp);
    Ok(())
}

/// Delete a file with administrator privileges. Best-effort: returns Ok
/// even if the file is already gone.
#[cfg(target_os = "macos")]
pub fn remove_file_privileged(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let script = format!("/bin/rm -f {}", sh_quote(&path.display().to_string()));
    run_as_admin(&script).with_context(|| format!("privileged rm of {}", path.display()))?;
    Ok(())
}

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
        let status = Command::new("/usr/bin/osascript")
            .args(["-e", &applescript])
            .status()
            .context("invoking osascript")?;
        if !status.success() {
            anyhow::bail!("osascript command exited non-zero (user cancelled?)");
        }
    }
    Ok(())
}

/// Run a shell command as root on Linux. In a terminal (CLI usage) we use
/// `sudo`, which caches credentials so a batch of privileged steps prompts
/// once; in a GUI session (no controlling tty) we use `pkexec`, which pops the
/// polkit authentication dialog — the Linux analogue of the macOS osascript
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

#[cfg(target_os = "macos")]
fn stage_tempfile(bytes: &[u8]) -> Result<std::path::PathBuf> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::time::{SystemTime, UNIX_EPOCH};
    // Stage inside a user-private 0o700 dir rather than world-readable /tmp:
    // the payload carries the gateway URL + X-Gate-Api-Key, and must not be
    // readable by other local users in the window before the privileged `mv`.
    let staging = crate::env::app_support_dir()?.join("staging");
    fs::create_dir_all(&staging)
        .with_context(|| format!("creating staging dir {}", staging.display()))?;
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("chmod 0o700 {}", staging.display()))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = staging.join(format!(
        "gate-connect-{pid}-{nanos}.tmp",
        pid = std::process::id(),
    ));
    // Create the file at 0o600 *before* writing payload bytes (O_EXCL), so the
    // secret never lands in a transiently world-readable file.
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("creating tempfile {}", path.display()))?;
    f.write_all(bytes)
        .with_context(|| format!("writing tempfile {}", path.display()))?;
    Ok(path)
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

/// Recursively copy `src` into `dst`, creating dirs as needed. Existing
/// files at the destination are overwritten. Symlinks are copied as
/// symlinks (not followed). No privilege escalation: caller must already
/// be able to read `src` and write `dst`.
#[cfg(target_os = "macos")]
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    if !src.exists() {
        return Ok(());
    }
    let meta = fs::symlink_metadata(src).with_context(|| format!("stat {}", src.display()))?;
    let ft = meta.file_type();

    if ft.is_symlink() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create_dir_all {}", parent.display()))?;
        }
        let target = fs::read_link(src).with_context(|| format!("readlink {}", src.display()))?;
        let _ = fs::remove_file(dst);
        std::os::unix::fs::symlink(&target, dst)
            .with_context(|| format!("symlink {} -> {}", dst.display(), target.display()))?;
        return Ok(());
    }

    if ft.is_file() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create_dir_all {}", parent.display()))?;
        }
        // Unlink first when the dest already exists: `fs::copy` errors out
        // when overwriting a read-only file (e.g. git pack-*.idx is mode
        // 0444), even though shell `cp -f` handles it transparently.
        if dst.exists() {
            let _ = fs::remove_file(dst);
        }
        fs::copy(src, dst)
            .with_context(|| format!("copy {} -> {}", src.display(), dst.display()))?;
        return Ok(());
    }

    if ft.is_dir() {
        fs::create_dir_all(dst).with_context(|| format!("create_dir_all {}", dst.display()))?;
        let entries = fs::read_dir(src).with_context(|| format!("read_dir {}", src.display()))?;
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            copy_dir_recursive(&entry.path(), &dst.join(name))?;
        }
    }
    Ok(())
}

/// Total byte size of a path. For directories, walks recursively. Returns 0
/// if `path` doesn't exist. Symlinks are not followed.
#[cfg(target_os = "macos")]
pub fn dir_size_bytes(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let meta = fs::symlink_metadata(path).with_context(|| format!("stat {}", path.display()))?;
    if meta.file_type().is_symlink() {
        return Ok(meta.len());
    }
    if meta.is_file() {
        return Ok(meta.len());
    }
    let mut total: u64 = 0;
    for entry in fs::read_dir(path).with_context(|| format!("read_dir {}", path.display()))? {
        let entry = entry?;
        total = total.saturating_add(dir_size_bytes(&entry.path())?);
    }
    Ok(total)
}

/// Install a set of plugins into the system-wide
/// `/Library/Application Support/Claude/org-plugins/` directory. Each entry
/// is `(plugin_name, source_dir)` where `source_dir` is the directory
/// containing `.claude-plugin/plugin.json` plus the plugin's content. The
/// destination subdir is named by the canonical plugin name (so the
/// `plugin_01XXX/` rpm dir naming convention is dropped on the way in).
///
/// Implementation: stage everything to a user-owned tempdir, then one
/// `run_as_admin` call moves the staging tree into place. This minimizes
/// admin-prompt fatigue (single password prompt for an arbitrary number
/// of plugins).
#[cfg(target_os = "macos")]
pub fn install_org_plugins_dir(plugins: &[(String, std::path::PathBuf)]) -> Result<()> {
    if plugins.is_empty() {
        return Ok(());
    }

    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let staging = std::env::temp_dir().join(format!(
        "gate-connect-org-plugins-{pid}-{nanos}",
        pid = std::process::id()
    ));
    fs::create_dir_all(&staging)
        .with_context(|| format!("create staging {}", staging.display()))?;
    let cleanup = staging.clone();
    let cleanup_guard = scopeguard_drop(move || {
        let _ = fs::remove_dir_all(&cleanup);
    });

    for (name, src) in plugins {
        if name.is_empty() || name.contains('/') || name.contains('\0') {
            anyhow::bail!("invalid plugin name {name:?}");
        }
        let dst = staging.join(name);
        copy_dir_recursive(src, &dst)
            .with_context(|| format!("stage {} -> {}", src.display(), dst.display()))?;
    }

    let target = "/Library/Application Support/Claude/org-plugins";
    let script = format!(
        // 1. Create target dir (root-owned).
        // 2. For each entry in staging, rm any existing dst entry then move
        // the staged dir into place. Bash `cp -R` would also work; mv is
        // cheaper since staging is on the same volume as /Library.
        // On rare cross-volume cases (e.g. /tmp on a different fs), the
        // inner `cp -R` fallback handles it.
        // 3. chown to root:wheel and chmod 755 so Cowork-3p (and other
        // users) can read the bundle.
        "/bin/mkdir -p {target} && \
  /bin/chmod 755 {target} && \
  for d in {staging}/*; do \
  name=$(/usr/bin/basename \"$d\"); \
  /bin/rm -rf {target}/\"$name\"; \
  /bin/mv \"$d\" {target}/\"$name\" 2>/dev/null || /bin/cp -R \"$d\" {target}/\"$name\"; \
  done && \
  /usr/sbin/chown -R root:wheel {target} && \
  /bin/chmod -R u+rwX,go+rX,go-w {target}",
        target = sh_quote(target),
        staging = sh_quote(&staging.display().to_string()),
    );
    run_as_admin(&script).with_context(|| format!("install org plugins to {target}"))?;

    drop(cleanup_guard);
    Ok(())
}

/// Minimal RAII helper so we don't pull `scopeguard`.
#[cfg(target_os = "macos")]
fn scopeguard_drop<F: FnOnce()>(f: F) -> impl Drop {
    struct Guard<F: FnOnce()>(Option<F>);
    impl<F: FnOnce()> Drop for Guard<F> {
        fn drop(&mut self) {
            if let Some(f) = self.0.take() {
                f();
            }
        }
    }
    Guard(Some(f))
}

/// Is any process currently running whose command line (argv joined) contains
/// `needle`? Uses `/usr/bin/pgrep -fl`. Returns false on any error (missing
/// pgrep, permission, etc.) — callers should treat the answer as best-effort.
///
/// Other `pgrep` instances are filtered out of the matches: a concurrent
/// caller's `pgrep -f <needle>` carries the needle in its own argv, so two
/// simultaneous checks (parallel tests, the app and the CLI at once) would
/// otherwise both see a false positive. pgrep excludes itself but not its
/// siblings.
#[cfg(target_os = "macos")]
pub fn is_process_running_matching(needle: &str) -> bool {
    Command::new("/usr/bin/pgrep")
        // -l with -f prints "PID <full argv>" per match, so we can tell
        // which matches are merely other pgrep invocations.
        .args(["-fl", needle])
        .output()
        .map(|out| {
            out.status.success()
                && String::from_utf8_lossy(&out.stdout).lines().any(|line| {
                    !line
                        .split_whitespace()
                        .nth(1)
                        .is_some_and(|cmd| cmd == "pgrep" || cmd.ends_with("/pgrep"))
                })
        })
        .unwrap_or(false)
}

/// Stable UUID we send to the gateway audit trail for telemetry attribution.
/// Generated once and cached at `<app_support_dir>/install-id`.
#[cfg(target_os = "macos")]
pub fn install_id() -> Result<String> {
    let path = crate::env::app_support_dir()?.join("install-id");
    if let Ok(s) = fs::read_to_string(&path) {
        let s = s.trim().to_string();
        if !s.is_empty() {
            return Ok(s);
        }
    }
    let id = simple_uuid_v4()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&path, &id).with_context(|| format!("writing {}", path.display()))?;
    Ok(id)
}

#[cfg(target_os = "macos")]
fn simple_uuid_v4() -> Result<String> {
    // Tiny inline v4 generator so we don't pull `uuid` for one call.
    let mut bytes = [0u8; 16];
    let mut f = fs::File::open("/dev/urandom").context("opening /dev/urandom")?;
    use std::io::Read;
    f.read_exact(&mut bytes).context("reading /dev/urandom")?;
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
