//! Linux system HTTP/HTTPS proxy wiring via a managed block in
//! `/etc/environment`. Enabling writes `http_proxy`/`https_proxy` (+ upper-case
//! aliases) pointing at our loopback engine, a `no_proxy` that keeps loopback
//! traffic off the proxy, and `NODE_EXTRA_CA_CERTS` pointing at our CA so
//! Node-based CLIs (e.g. Claude Code) — which ship their own bundle and ignore
//! the system trust store — accept the engine's minted leaf certs. Disabling
//! strips the block again.
//!
//! `/etc/environment` is read by PAM at login, so the variables reach every new
//! login session: GUI apps started afterwards *and* command-line shells. It is
//! deliberately DE-agnostic — no GNOME `gsettings` / KDE-specific path — at the
//! cost of only affecting **new** sessions (already-running shells keep their
//! environment until restarted).
//!
//! Privilege model differs from macOS/Windows. There the proxy lives in a
//! per-user store, so enable/disable/reconcile are promptless and the revert can
//! never be cancelled. `/etc/environment` is root-owned, so every write here —
//! including the safety-revert — goes through [`crate::primitives::run_as_admin`]
//! and can prompt. We touch only our own delimited block, so a concurrent edit
//! to the rest of the file is preserved.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::env;
use crate::primitives::{run_as_admin, sh_quote};

const ENV_FILE: &str = "/etc/environment";

/// Delimiters bracketing the lines we own in `/etc/environment`. Everything
/// between them (inclusive) is ours to add/replace/remove; everything else is
/// left untouched.
const BLOCK_BEGIN: &str = "# >>> gate-connect proxy (managed) >>>";
const BLOCK_END: &str = "# <<< gate-connect proxy (managed) <<<";

/// Marker recorded on enable. The managed-block design needs no captured prior
/// state to revert (we just strip our block), so this only notes whether a
/// block was already present when we looked — and, more importantly, its
/// existence on disk is what tells [`super::manager`] a previous session left
/// the proxy on (crash reconcile).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxySnapshot {
    /// Whether our managed block was already in `/etc/environment` when we
    /// snapshotted (i.e. an earlier unclean session left it behind).
    pub block_present: bool,
}

fn snapshot_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?
        .join("proxy")
        .join("system-proxy.snapshot.json"))
}

/// Path to our CA cert, mirrored from [`super::ca`] — used for
/// `NODE_EXTRA_CA_CERTS` so Node CLIs trust the engine's leaf certs.
fn ca_cert_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join("proxy").join("ca-cert.pem"))
}

/// Current contents of `/etc/environment` (empty string if it doesn't exist).
/// World-readable, so non-privileged.
fn read_env_file() -> Result<String> {
    match fs::read_to_string(ENV_FILE) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e).with_context(|| format!("reading {ENV_FILE}")),
    }
}

/// Return `content` with our managed block (and the blank line we pad it with)
/// removed. Lines outside the delimiters are preserved verbatim.
fn strip_block(content: &str) -> String {
    let mut out = Vec::new();
    let mut inside = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == BLOCK_BEGIN {
            inside = true;
            continue;
        }
        if trimmed == BLOCK_END {
            inside = false;
            continue;
        }
        if !inside {
            out.push(line);
        }
    }
    let mut joined = out.join("\n");
    // Collapse trailing whitespace/newlines left after removal, then restore a
    // single trailing newline if there's any content.
    while joined.ends_with('\n') || joined.ends_with(' ') {
        joined.pop();
    }
    if !joined.is_empty() {
        joined.push('\n');
    }
    joined
}

/// Build the managed block pointing at `127.0.0.1:port`.
fn build_block(port: u16) -> Result<String> {
    let endpoint = format!("http://127.0.0.1:{port}");
    let no_proxy = "localhost,127.0.0.1,::1";
    let ca = ca_cert_path()?;
    Ok(format!(
        "{BLOCK_BEGIN}\n\
         http_proxy={endpoint}\n\
         https_proxy={endpoint}\n\
         HTTP_PROXY={endpoint}\n\
         HTTPS_PROXY={endpoint}\n\
         no_proxy={no_proxy}\n\
         NO_PROXY={no_proxy}\n\
         NODE_EXTRA_CA_CERTS={ca}\n\
         {BLOCK_END}\n",
        ca = ca.display(),
    ))
}

/// Write `content` to `/etc/environment` via a privileged copy: stage to a
/// user-owned tempfile, then `install` it into place as root:root 0644. The
/// payload is non-secret (a loopback URL + the public CA path), so the staging
/// file doesn't need the locked-down treatment the Cowork credential writes get.
fn write_env_file(content: &str) -> Result<()> {
    let staging = env::app_support_dir()?.join("staging");
    fs::create_dir_all(&staging).with_context(|| format!("creating {}", staging.display()))?;
    let tmp = staging.join("etc-environment.tmp");
    fs::write(&tmp, content).with_context(|| format!("writing {}", tmp.display()))?;

    let script = format!(
        "/usr/bin/install -m 0644 -o root -g root {src} {dst}",
        src = sh_quote(&tmp.display().to_string()),
        dst = sh_quote(ENV_FILE),
    );
    let result = run_as_admin(&script).with_context(|| format!("writing {ENV_FILE}"));
    let _ = fs::remove_file(&tmp);
    result
}

/// Note whether our managed block is currently present. Non-privileged.
pub fn snapshot() -> Result<ProxySnapshot> {
    Ok(ProxySnapshot {
        block_present: read_env_file()?.contains(BLOCK_BEGIN),
    })
}

pub fn save_snapshot(snapshot: &ProxySnapshot) -> Result<()> {
    let path = snapshot_path()?;
    let raw = serde_json::to_string_pretty(snapshot).context("serializing proxy snapshot")?;
    // Atomic write (handles parent dirs too): a torn snapshot would make
    // disable/reconcile fall back to force-off instead of an exact restore.
    crate::primitives::write_file(&path, raw.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

pub fn load_snapshot() -> Result<Option<ProxySnapshot>> {
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

pub fn clear_snapshot() -> Result<()> {
    let path = snapshot_path()?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

/// Point the system proxy at the loopback engine by writing our managed block.
/// Privileged (`/etc/environment` is root-owned). Only affects new sessions.
pub fn enable(port: u16) -> Result<()> {
    let stripped = strip_block(&read_env_file()?);
    let block = build_block(port)?;
    write_env_file(&format!("{stripped}{block}"))
}

/// Strip our managed block, restoring `/etc/environment` to its prior state.
/// For the managed-block design restore and force-off are identical — both just
/// remove our lines — so `snapshot` is unused here. Privileged; safety-critical.
pub fn restore(_snapshot: &ProxySnapshot) -> Result<()> {
    force_off()
}

/// Remove our managed block. Fail-safe used when no snapshot is available, so a
/// dead engine never strands new shells at an unreachable proxy. Privileged.
pub fn force_off() -> Result<()> {
    let content = read_env_file()?;
    if !content.contains(BLOCK_BEGIN) {
        return Ok(());
    }
    write_env_file(&strip_block(&content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_removes_only_our_block() {
        let original = "FOO=bar\n";
        let with_block =
            format!("FOO=bar\n{BLOCK_BEGIN}\nhttp_proxy=http://127.0.0.1:9\n{BLOCK_END}\n");
        assert_eq!(strip_block(&with_block), original);
    }

    #[test]
    fn strip_is_noop_without_block() {
        let original = "FOO=bar\nBAZ=qux\n";
        assert_eq!(strip_block(original), original);
    }

    #[test]
    fn strip_of_empty_is_empty() {
        assert_eq!(strip_block(""), "");
    }
}
