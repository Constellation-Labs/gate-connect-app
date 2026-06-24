//! Small native primitives shared across integrations.
//!
//! `write_file` is cross-platform. Below it are the privileged-execution
//! and shell-quoting helpers the proxy subsystem uses for its elevated
//! steps (macOS/Linux), plus the cached install-id for telemetry.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

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
