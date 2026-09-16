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
//!
//! Rule 1 is about *ownership*, not about never writing twice, and conflating
//! the two is what made re-connect a no-op: a key we wrote ourselves is ours to
//! refresh, and the caller says which those are by passing back the list from
//! its sidecar. Without that, a value Gate needs to change - the loopback port
//! is the one that moves - stayed at whatever the first connect wrote, forever.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// What [`add_vars`] actually changed, for the caller's sidecar.
#[derive(Debug, Default)]
pub(crate) struct Applied {
    /// Keys we wrote for the first time. Keys the user already had are absent,
    /// and so are keys we merely refreshed - the sidecar already lists those,
    /// and re-recording them would let a later connect claim credit for a line
    /// an earlier one put there.
    pub added: Vec<String>,
    /// Keys that were already ours and whose value we brought up to date.
    /// Reported so a caller can tell a real repair from a no-op.
    pub refreshed: Vec<String>,
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
pub(crate) fn add_vars(path: &Path, vars: &[(&str, String)], ours: &[String]) -> Result<Applied> {
    let file_created = !path.exists();
    let mut body = if file_created {
        String::new()
    } else {
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?
    };

    let mut added = Vec::new();
    let mut refreshed = Vec::new();
    for (key, value) in vars {
        let line = format!("{key}={value}");
        if body.lines().any(|l| assigns(l, key)) {
            // Present already. Ours to correct, or the user's to leave alone -
            // and `ours` is the only thing that can tell those apart, since the
            // line itself looks identical either way.
            if !ours.iter().any(|k| k == key) {
                continue;
            }
            if body.lines().any(|l| l == line) {
                continue;
            }
            body = body
                .lines()
                .map(|l| if assigns(l, key) { line.as_str() } else { l })
                .collect::<Vec<_>>()
                .join("\n");
            body.push('\n');
            refreshed.push((*key).to_string());
            continue;
        }
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        body.push_str(&format!("{line}\n"));
        added.push((*key).to_string());
    }

    if added.is_empty() && refreshed.is_empty() {
        return Ok(Applied {
            added,
            refreshed,
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
        refreshed,
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
            // Nothing is ours yet: a first connect over a file we have never
            // touched, which is the case rule 1 exists for.
            &[],
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

    /// A value of ours that has gone stale is brought up to date; the same
    /// value belonging to the user is still not.
    ///
    /// This is the migration case, and it was broken in the quiet way. The
    /// loopback port moves, so the `HTTPS_PROXY` a first connect wrote stops
    /// being the right one - and `add_vars` could only ever add, so a
    /// re-connect found the key present, skipped it, and reported success
    /// having changed nothing. `status` went on saying Drifted, and
    /// `reconcile_unmapped_tools` re-ran the same no-op on every launch. Only
    /// toggling the master switch repaired it, because disconnect removes the
    /// lines first.
    ///
    /// Ownership is the whole distinction, and it cannot be read off the file:
    /// `HTTPS_PROXY=http://127.0.0.1:9977` looks the same whether Gate wrote it
    /// or the user did. The caller's sidecar is the only record of which, which
    /// is why it has to be passed in rather than inferred here.
    #[test]
    fn a_stale_value_of_ours_is_refreshed_and_the_users_is_not() {
        let path = tmp();
        fs::write(
            &path,
            "OPENROUTER_API_KEY=sk-user\nHTTPS_PROXY=http://127.0.0.1:9977\nHERMES_CA_BUNDLE=/old/ca.pem\n",
        )
        .unwrap();

        // The port moved. Both keys below are ours per the sidecar.
        let ours = vec!["HTTPS_PROXY".to_string(), "HERMES_CA_BUNDLE".to_string()];
        let applied = add_vars(
            &path,
            &[
                ("HTTPS_PROXY", "http://127.0.0.1:45981".into()),
                ("HERMES_CA_BUNDLE", "/old/ca.pem".into()),
            ],
            &ours,
        )
        .unwrap();

        // Refreshed, not added: re-recording it would let this connect claim
        // credit for a line the first one wrote, and disconnect reads that list.
        assert_eq!(applied.added, Vec::<String>::new());
        assert_eq!(applied.refreshed, vec!["HTTPS_PROXY".to_string()]);

        let body = fs::read_to_string(&path).unwrap();
        assert!(
            body.contains("HTTPS_PROXY=http://127.0.0.1:45981"),
            "the stale port must be corrected: {body}"
        );
        assert!(!body.contains("9977"), "the old value must be gone: {body}");
        assert!(
            body.contains("OPENROUTER_API_KEY=sk-user"),
            "unrelated lines must survive: {body}"
        );
        // Already correct, so not reported as a change - otherwise every
        // unattended re-connect would look like a repair.
        assert!(!applied.refreshed.contains(&"HERMES_CA_BUNDLE".to_string()));

        // The same staleness on a key the user owns is left alone.
        let path = tmp();
        fs::write(&path, "HTTPS_PROXY=http://corp:3128\n").unwrap();
        let applied = add_vars(
            &path,
            &[("HTTPS_PROXY", "http://127.0.0.1:45981".into())],
            &[],
        )
        .unwrap();
        assert_eq!(applied.added, Vec::<String>::new());
        assert_eq!(applied.refreshed, Vec::<String>::new());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "HTTPS_PROXY=http://corp:3128\n",
            "a corporate proxy the user set is never ours to correct"
        );
    }

    #[test]
    fn a_file_we_created_is_removed_again() {
        let path = tmp();
        let applied = add_vars(
            &path,
            &[("HTTPS_PROXY", "http://127.0.0.1:9977".into())],
            &[],
        )
        .unwrap();
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
        let applied = add_vars(
            &path,
            &[("HTTPS_PROXY", "http://127.0.0.1:9977".into())],
            &[],
        )
        .unwrap();
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
        let applied = add_vars(
            &path,
            &[("HTTPS_PROXY", "http://127.0.0.1:9977".into())],
            &[],
        )
        .unwrap();
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
