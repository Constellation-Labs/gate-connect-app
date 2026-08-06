//! Managed edits to a tool's `.env` file.
//!
//! Shared by the two harnesses that route through the proxy engine rather than
//! the relay ([`super::hermes`], [`super::openclaw`]), because both configure
//! that the same way: set some variables in the tool's own dotenv, and take
//! exactly those away again on disconnect.
//!
//! Two rules make this safe to point at a file we do not own:
//!
//! 1. **A variable the user already set is never touched.** These files hold
//!    API keys and, in Hermes' case, whatever else the installer put there. A
//!    pre-existing `HTTPS_PROXY` may be a corporate egress proxy the rest of
//!    their setup depends on, so we leave it and record that we did - which is
//!    also what stops disconnect from deleting it later.
//! 2. **Only lines we added come back out.** [`remove_vars`] takes the exact
//!    key list [`add_vars`] reported, not the full set we would have written.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// What [`add_vars`] actually changed, for the caller's sidecar.
#[derive(Debug, Default)]
pub(crate) struct Applied {
    /// Keys we wrote. Keys the user already had are absent.
    pub added: Vec<String>,
    /// Whether the file itself did not exist before this call.
    pub file_created: bool,
}

/// True if `line` is an assignment of `key` (ignoring leading whitespace and
/// an `export ` prefix). Deliberately not a full dotenv parse - we only need to
/// recognise our own keys and avoid colliding with the user's.
fn assigns(line: &str, key: &str) -> bool {
    let l = line.trim_start();
    let l = l.strip_prefix("export ").unwrap_or(l).trim_start();
    l.strip_prefix(key)
        .is_some_and(|rest| rest.starts_with('='))
}

/// Add each `(key, value)` that the file does not already define. Returns which
/// keys were added so disconnect can remove exactly those.
pub(crate) fn add_vars(path: &Path, vars: &[(&str, String)]) -> Result<Applied> {
    let file_created = !path.exists();
    let mut body = if file_created {
        String::new()
    } else {
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?
    };

    let mut added = Vec::new();
    for (key, value) in vars {
        if body.lines().any(|l| assigns(l, key)) {
            continue;
        }
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        body.push_str(&format!("{key}={value}\n"));
        added.push((*key).to_string());
    }

    if added.is_empty() {
        return Ok(Applied {
            added,
            file_created: false,
        });
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    // 0o600: these files routinely hold the user's API keys.
    crate::primitives::write_file(path, body.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(Applied {
        added,
        file_created,
    })
}

/// Remove exactly the keys named, and the file too when we created it and
/// nothing but blank lines is left.
pub(crate) fn remove_vars(path: &Path, keys: &[String], file_created: bool) -> Result<()> {
    if !path.exists() || keys.is_empty() {
        return Ok(());
    }
    let existing =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let kept: Vec<&str> = existing
        .lines()
        .filter(|l| !keys.iter().any(|k| assigns(l, k)))
        .collect();

    if file_created && kept.iter().all(|l| l.trim().is_empty()) {
        return fs::remove_file(path).with_context(|| format!("removing {}", path.display()));
    }

    let mut body = kept.join("\n");
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    crate::primitives::write_file(path, body.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

/// The value assigned to `key`, if the file defines it. Surrounding quotes are
/// stripped so a value we wrote bare compares equal to one the user quoted.
pub(crate) fn read_var(path: &Path, key: &str) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let body = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(body
        .lines()
        .find(|l| assigns(l, key))
        .and_then(|l| l.split_once('='))
        .map(|(_, v)| v.trim().trim_matches(|c| c == '"' || c == '\'').to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "gate-dotenv-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p.join(".env")
    }

    #[test]
    fn adds_only_missing_keys_and_never_clobbers_the_users() {
        let path = tmp();
        // The user already routes through a corporate proxy and has a key.
        fs::write(
            &path,
            "OPENROUTER_API_KEY=sk-user\nHTTPS_PROXY=http://corp:3128\n",
        )
        .unwrap();

        let applied = add_vars(
            &path,
            &[
                ("HTTPS_PROXY", "http://127.0.0.1:9977".into()),
                ("NO_PROXY", "localhost,127.0.0.1".into()),
            ],
        )
        .unwrap();

        assert_eq!(applied.added, vec!["NO_PROXY".to_string()]);
        assert!(!applied.file_created);
        let body = fs::read_to_string(&path).unwrap();
        assert!(
            body.contains("HTTPS_PROXY=http://corp:3128"),
            "the user's proxy must survive: {body}"
        );
        assert!(!body.contains("127.0.0.1:9977"), "must not clobber: {body}");
        assert!(body.contains("NO_PROXY=localhost,127.0.0.1"));

        // Disconnect takes back only what we added.
        remove_vars(&path, &applied.added, applied.file_created).unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("OPENROUTER_API_KEY=sk-user"));
        assert!(after.contains("HTTPS_PROXY=http://corp:3128"));
        assert!(!after.contains("NO_PROXY"));
    }

    #[test]
    fn a_file_we_created_is_removed_again() {
        let path = tmp();
        let applied = add_vars(&path, &[("HTTPS_PROXY", "http://127.0.0.1:9977".into())]).unwrap();
        assert!(applied.file_created);
        assert!(path.exists());

        remove_vars(&path, &applied.added, applied.file_created).unwrap();
        assert!(
            !path.exists(),
            "a dotenv that only ever held our line must not be left behind"
        );
    }

    #[test]
    fn a_file_we_created_survives_if_the_user_added_to_it() {
        let path = tmp();
        let applied = add_vars(&path, &[("HTTPS_PROXY", "http://127.0.0.1:9977".into())]).unwrap();
        fs::write(
            &path,
            format!("{}USER_KEY=value\n", fs::read_to_string(&path).unwrap()),
        )
        .unwrap();

        remove_vars(&path, &applied.added, applied.file_created).unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("USER_KEY=value"));
        assert!(!after.contains("HTTPS_PROXY"));
    }

    #[test]
    fn recognises_export_and_quoted_forms() {
        let path = tmp();
        fs::write(&path, "export HTTPS_PROXY=\"http://corp:3128\"\n").unwrap();
        // `export`-prefixed counts as already set.
        let applied = add_vars(&path, &[("HTTPS_PROXY", "http://127.0.0.1:9977".into())]).unwrap();
        assert!(applied.added.is_empty());
        assert_eq!(
            read_var(&path, "HTTPS_PROXY").unwrap().as_deref(),
            Some("http://corp:3128"),
            "quotes must be stripped so values compare equal"
        );
        // A key that merely shares a prefix is a different variable.
        assert_eq!(read_var(&path, "HTTPS").unwrap(), None);
    }
}
