//! macOS CLI proxy reach via a managed `~/.zshenv` block. The `networksetup`
//! system proxy (see [`super::system_proxy`]) covers GUI apps, but CLI tools -
//! Node-based ones especially (the Gemini CLI) - read only the `HTTP(S)_PROXY`
//! env vars and trust only `NODE_EXTRA_CA_CERTS`, both resolved at process
//! init. macOS injects neither into CLI processes, so without this module a
//! proxy-only provider silently no-ops there: the CLI connects straight to
//! the upstream, bypassing Gate. Linux already has this half via its
//! `environment.d` drop-in ([`super::system_proxy`] on Linux); this is the
//! macOS parity fix, and it benefits every Node CLI, not just Gemini.
//!
//! There is no macOS equivalent of `environment.d` that the OS injects into
//! every process; the only reliable path to CLI (Node) processes is a shell
//! startup file. `~/.zshenv` is the right one: zsh is the macOS default
//! shell, and `.zshenv` is sourced for *all* zsh invocations, including
//! non-interactive CLI launches.
//!
//! Unlike the Linux drop-in - a dedicated file we own outright - `~/.zshenv`
//! is a shared, shell-sourced file the user may have content in. So edits are
//! surgical: enabling upserts a sentinel-delimited managed block (exactly one,
//! re-enables replace it in place), disabling strips only that block and
//! leaves everything else byte-for-byte intact. Only NEW shells pick the
//! change up; already-open terminals keep their environment until restarted.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::env;

/// Sentinel lines delimiting the managed block. Grep-friendly and explicit
/// about ownership so a user editing `~/.zshenv` knows what the span is.
const BLOCK_BEGIN: &str = "# >>> gate-connect proxy (managed, do not edit) >>>";
const BLOCK_END: &str = "# <<< gate-connect proxy (managed, do not edit) <<<";

/// `~/.zshenv`, via [`env::home`] so tests redirect it with the
/// `GATE_CONNECT_TEST_HOME` seam.
fn zshenv_path() -> Result<PathBuf> {
    Ok(env::home()?.join(".zshenv"))
}

/// Path to our CA cert, mirrored from [`super::ca`] - used for
/// `NODE_EXTRA_CA_CERTS` so Node CLIs trust the engine's leaf certs.
fn ca_cert_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join("proxy").join("ca-cert.pem"))
}

/// The managed block for an engine on `127.0.0.1:port`. All values are
/// double-quoted shell exports - the CA path lives under
/// `~/Library/Application Support/...`, which contains a space.
fn build_block(port: u16) -> Result<String> {
    let endpoint = format!("http://127.0.0.1:{port}");
    let no_proxy = "localhost,127.0.0.1,::1";
    let ca = ca_cert_path()?.display().to_string();
    Ok(format!(
        "{BLOCK_BEGIN}\n\
         # Written while the Gate proxy is ON so CLI tools (Node-based: Gemini, etc.)\n\
         # route through the local engine and trust its CA. Only NEW shells read this;\n\
         # restart open terminals. Removed automatically when the proxy is OFF.\n\
         export http_proxy=\"{endpoint}\"\n\
         export https_proxy=\"{endpoint}\"\n\
         export HTTP_PROXY=\"{endpoint}\"\n\
         export HTTPS_PROXY=\"{endpoint}\"\n\
         export no_proxy=\"{no_proxy}\"\n\
         export NO_PROXY=\"{no_proxy}\"\n\
         export NODE_EXTRA_CA_CERTS=\"{ca}\"\n\
         {BLOCK_END}\n"
    ))
}

/// Byte span of the managed block in `existing` - begin sentinel through the
/// end sentinel plus its trailing newline - or `None` if no block is present.
fn block_span(existing: &str) -> Option<(usize, usize)> {
    let start = existing.find(BLOCK_BEGIN)?;
    let end = existing[start..]
        .find(BLOCK_END)
        .map(|i| start + i + BLOCK_END.len())?;
    let end = if existing[end..].starts_with('\n') {
        end + 1
    } else {
        end
    };
    Some((start, end))
}

/// Replace the managed block in place if one exists, else append `block`,
/// inserting a separating newline so we never join onto a user line.
fn upsert_block(existing: &str, block: &str) -> String {
    if let Some((start, end)) = block_span(existing) {
        return format!("{}{}{}", &existing[..start], block, &existing[end..]);
    }
    let mut out = existing.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(block);
    out
}

/// Remove the managed block span (sentinels included); everything else is
/// returned byte-for-byte. No-op if no block is present.
fn strip_block(existing: &str) -> String {
    match block_span(existing) {
        Some((start, end)) => format!("{}{}", &existing[..start], &existing[end..]),
        None => existing.to_string(),
    }
}

/// Upsert the managed block into `~/.zshenv` (created if absent) so new
/// shells export the proxy + CA vars. Idempotent: exactly one block regardless
/// of how many times it runs; a new port rewrites the existing block in place.
pub fn enable(port: u16) -> Result<()> {
    let path = zshenv_path()?;
    let existing = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let updated = upsert_block(&existing, &build_block(port)?);
    crate::primitives::write_file(&path, updated.as_bytes(), 0o644)
        .with_context(|| format!("writing {}", path.display()))
}

/// Strip the managed block from `~/.zshenv`, preserving all user content. If
/// our block was the file's only content (we created it), the file is removed
/// rather than left empty. No-op when the block or the file is absent.
pub fn disable() -> Result<()> {
    let path = zshenv_path()?;
    let existing = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let stripped = strip_block(&existing);
    if stripped == existing {
        return Ok(()); // no block; leave the file untouched
    }
    if stripped.is_empty() {
        return match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
        };
    }
    crate::primitives::write_file(&path, stripped.as_bytes(), 0o644)
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_temp_env<T>(f: impl FnOnce() -> T) -> T {
        // Same pattern as the Linux system_proxy tests: these mutate the
        // process-global GATE_CONNECT_TEST_HOME seam and share a temp dir, so
        // serialize them and restore any ambient value on teardown.
        static GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _lock = GUARD.lock().unwrap_or_else(|e| e.into_inner());

        let tmp = std::env::temp_dir().join(format!("gate-zshenv-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let prev_home = std::env::var_os("GATE_CONNECT_TEST_HOME");
        std::env::set_var("GATE_CONNECT_TEST_HOME", &tmp);
        let out = f();
        match prev_home {
            Some(v) => std::env::set_var("GATE_CONNECT_TEST_HOME", v),
            None => std::env::remove_var("GATE_CONNECT_TEST_HOME"),
        }
        let _ = fs::remove_dir_all(&tmp);
        out
    }

    fn read_zshenv() -> String {
        fs::read_to_string(zshenv_path().unwrap()).unwrap()
    }

    #[test]
    fn enable_creates_file_with_managed_block() {
        with_temp_env(|| {
            assert!(!zshenv_path().unwrap().exists());
            enable(41234).unwrap();
            let body = read_zshenv();
            assert!(body.starts_with(BLOCK_BEGIN));
            assert!(body.contains("export HTTPS_PROXY=\"http://127.0.0.1:41234\""));
            assert!(body.contains("export no_proxy=\"localhost,127.0.0.1,::1\""));
            assert!(body.contains("export NO_PROXY=\"localhost,127.0.0.1,::1\""));
            // CA path is double-quoted so its embedded space is safe.
            assert!(body.contains("export NODE_EXTRA_CA_CERTS=\""));
            assert!(body.trim_end().ends_with(BLOCK_END));
        });
    }

    #[test]
    fn enable_preserves_user_content() {
        with_temp_env(|| {
            // No trailing newline: the upsert must add the separator itself.
            fs::write(zshenv_path().unwrap(), "export FOO=bar").unwrap();
            enable(41234).unwrap();
            let body = read_zshenv();
            assert!(body.starts_with("export FOO=bar\n"));
            assert!(body.contains(BLOCK_BEGIN));
        });
    }

    #[test]
    fn reenable_replaces_block_in_place_with_new_port() {
        with_temp_env(|| {
            fs::write(zshenv_path().unwrap(), "export FOO=bar\n").unwrap();
            enable(41234).unwrap();
            let trailer = "export BAZ=qux\n";
            fs::write(
                zshenv_path().unwrap(),
                format!("{}{}", read_zshenv(), trailer),
            )
            .unwrap();
            enable(51234).unwrap();
            let body = read_zshenv();
            assert_eq!(body.matches(BLOCK_BEGIN).count(), 1, "exactly one block");
            assert!(body.contains(":51234"));
            assert!(!body.contains(":41234"), "old port fully replaced");
            // In place: user content on both sides survives in order.
            assert!(body.starts_with("export FOO=bar\n"));
            assert!(body.ends_with(trailer));
        });
    }

    #[test]
    fn disable_strips_only_the_block() {
        with_temp_env(|| {
            fs::write(zshenv_path().unwrap(), "export FOO=bar\n").unwrap();
            enable(41234).unwrap();
            disable().unwrap();
            assert_eq!(read_zshenv(), "export FOO=bar\n");
        });
    }

    #[test]
    fn disable_removes_file_we_created() {
        with_temp_env(|| {
            enable(41234).unwrap();
            disable().unwrap();
            assert!(
                !zshenv_path().unwrap().exists(),
                "a ~/.zshenv holding only our block is removed, not left empty"
            );
        });
    }

    #[test]
    fn disable_is_noop_without_block_or_file() {
        with_temp_env(|| {
            // No file at all.
            disable().unwrap();
            // A file with no managed block is left byte-for-byte untouched.
            fs::write(zshenv_path().unwrap(), "export FOO=bar").unwrap();
            disable().unwrap();
            assert_eq!(read_zshenv(), "export FOO=bar");
        });
    }

    #[test]
    fn build_block_is_quoted_and_sentinel_delimited() {
        with_temp_env(|| {
            let block = build_block(9999).unwrap();
            assert!(block.starts_with(BLOCK_BEGIN));
            assert!(block.ends_with(&format!("{BLOCK_END}\n")));
            for key in ["http_proxy", "https_proxy", "HTTP_PROXY", "HTTPS_PROXY"] {
                assert!(block.contains(&format!("export {key}=\"http://127.0.0.1:9999\"")));
            }
            // Every export value is double-quoted (the CA path has a space).
            for line in block.lines().filter(|l| l.starts_with("export ")) {
                let value = line.split_once('=').unwrap().1;
                assert!(value.starts_with('"') && value.ends_with('"'), "{line}");
            }
        });
    }
}
