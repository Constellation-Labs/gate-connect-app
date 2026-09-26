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
//! refresh, and the caller says which those are by passing back the record from
//! its sidecar. Without that, a value Gate needs to change - the loopback port
//! is the one that moves - stayed at whatever the first connect wrote, forever.
//!
//! Ownership is of a *value*, not of a key ([`Owned`]). A key alone would be a
//! standing claim: having once written `HTTPS_PROXY` we would overwrite it on
//! every connect forever, including the hand-edit the user made last week to
//! point Hermes at their own mitmproxy. So we keep what we left on disk and
//! refresh only while the line still holds it. The moment it does not, the user
//! has been in the file and rule 1 applies again - to a line we did write.
//!
//! Edits are per line and in place. The file belongs to someone else, so a pass
//! that rebuilds it wholesale is wrong even when every value it writes is right:
//! [`str::lines`] drops the `\r` from a CRLF file, which on Windows reformats
//! every line the user owns as a side effect of correcting one of ours.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// A key a previous connect wrote, paired with the value it left on disk.
///
/// The value is the whole point: see the ownership paragraph in the module
/// docs. `value` is `None` for a key recorded before Gate kept values - ours by
/// key alone for one more connect, so the moved-port repair still works on an
/// install that predates the field, and recorded properly on the way out so the
/// window closes after a single connect and never reopens.
#[derive(Debug, Clone)]
pub(crate) struct Owned {
    pub key: String,
    pub value: Option<String>,
}

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
    /// Every key that is ours once this call returns, with the value now on
    /// disk. The caller persists this and hands it back as [`Owned::value`]
    /// next time, which is what lets the next connect tell its own line from
    /// one the user has edited since. Keys the user owns are absent, and stay
    /// absent - nothing here ever widens what we may overwrite.
    ///
    /// Populated even when nothing changed, because a key that was already
    /// correct still needs its value on record.
    pub owned_values: Vec<(String, String)>,
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

/// A line split into its content and its own terminator, so a line we rewrite
/// keeps the ending it had. Returns an empty terminator for a final line with
/// no newline at all.
fn split_terminator(line: &str) -> (&str, &str) {
    line.strip_suffix("\r\n")
        .map(|rest| (rest, "\r\n"))
        .or_else(|| line.strip_suffix('\n').map(|rest| (rest, "\n")))
        .unwrap_or((line, ""))
}

/// Strip whitespace and one layer of surrounding quotes, which is how a dotenv
/// reader sees a value. Shared with [`read_var`] on purpose: the comparison
/// that decides a line of ours is stale has to be the same one that decides the
/// tool has drifted, or a correct line gets "repaired" on every single connect.
fn unquote(raw: &str) -> String {
    raw.trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .to_string()
}

/// The value an assignment line carries, read the way [`read_var`] reads one.
fn assigned_value(line: &str) -> Option<String> {
    let (content, _) = split_terminator(line);
    content.split_once('=').map(|(_, raw)| unquote(raw))
}

/// The quote character wrapping `raw`, if it is wrapped in one.
fn surrounding_quote(raw: &str) -> Option<char> {
    let trimmed = raw.trim();
    let first = trimmed.chars().next()?;
    (matches!(first, '"' | '\'') && trimmed.len() >= 2 && trimmed.ends_with(first)).then_some(first)
}

/// Rewrite one assignment to `value`, keeping everything about the line that is
/// not the value: its indentation, any `export ` prefix, the spelling of the
/// key, the quoting the old value carried, and its own line ending.
///
/// The `export ` is not cosmetic. These files get `source`d as often as they
/// get parsed, and dropping the keyword turns an exported variable into a shell
/// local, which un-routes the tool while the line still reads correctly.
fn rewrite_assignment(line: &str, value: &str) -> String {
    let (content, terminator) = split_terminator(line);
    let Some((prefix, old)) = content.split_once('=') else {
        return line.to_string();
    };
    // A value containing the quote it would be wrapped in cannot be re-quoted
    // without an escaping rule this module deliberately does not have, so it
    // goes back bare.
    match surrounding_quote(old).filter(|q| !value.contains(*q)) {
        Some(q) => format!("{prefix}={q}{value}{q}{terminator}"),
        None => format!("{prefix}={value}{terminator}"),
    }
}

/// The line ending to give a line we append: whatever the file already uses.
fn dominant_newline(body: &str) -> &'static str {
    if body.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// Set each `(key, value)`, adding it when the file does not define it and
/// correcting it when the file defines it with a value `ours` says we wrote.
///
/// A key `ours` does not vouch for is left exactly as it is, and never appears
/// in the returned record. Reports what was added separately from what was
/// refreshed: disconnect removes the former, and only the former.
pub(crate) fn add_vars(path: &Path, vars: &[(&str, String)], ours: &[Owned]) -> Result<Applied> {
    let file_created = !path.exists();
    let body = if file_created {
        String::new()
    } else {
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?
    };

    // `split_inclusive` keeps each line's own terminator attached, so lines we
    // do not touch are re-emitted byte for byte and a CRLF file stays CRLF.
    let newline = dominant_newline(&body);
    let mut lines: Vec<String> = body.split_inclusive('\n').map(str::to_string).collect();

    let mut added = Vec::new();
    let mut refreshed = Vec::new();
    let mut owned_values = Vec::new();
    for (key, value) in vars {
        let hits: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| assigns(l, key))
            .map(|(i, _)| i)
            .collect();

        if hits.is_empty() {
            if let Some(last) = lines.last_mut() {
                if !last.ends_with('\n') {
                    last.push_str(newline);
                }
            }
            lines.push(format!("{key}={value}{newline}"));
            added.push((*key).to_string());
            owned_values.push(((*key).to_string(), value.clone()));
            continue;
        }

        // Present already. Ours to correct, or the user's to leave alone, and
        // only the sidecar can tell those apart: the line itself looks
        // identical either way.
        let Some(owned) = ours.iter().find(|o| o.key == **key) else {
            continue;
        };
        let current: Vec<String> = hits
            .iter()
            .filter_map(|&i| assigned_value(&lines[i]))
            .collect();
        // Still ours only while every assignment of the key holds what we left
        // there. A second, different assignment is someone else's hand in the
        // file, and backing off is the safe reading of an ambiguous one.
        let still_ours = owned
            .value
            .as_ref()
            .is_none_or(|left| current.iter().all(|c| c == left));
        if !still_ours {
            continue;
        }
        owned_values.push(((*key).to_string(), value.clone()));

        if current.iter().all(|c| c == value) {
            // Already right, in every position. Not reported as a change, or
            // every unattended re-connect would announce itself as a repair.
            continue;
        }

        // Correct the first assignment in place and drop any others. Collapsing
        // is what makes this converge: a duplicate assignment of a key that is
        // ours is a line we wrote, and leaving a stale second copy behind means
        // a last-wins reader keeps using it while `read_var` reports the fresh
        // one and status reads Connected.
        lines[hits[0]] = rewrite_assignment(&lines[hits[0]], value);
        for &i in hits[1..].iter().rev() {
            lines.remove(i);
        }
        refreshed.push((*key).to_string());
    }

    if added.is_empty() && refreshed.is_empty() {
        return Ok(Applied {
            added,
            refreshed,
            owned_values,
            file_created: false,
        });
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    // 0o600: these files routinely hold the user's API keys.
    crate::config_changes::write(path, lines.concat().as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(Applied {
        added,
        refreshed,
        owned_values,
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
        return crate::config_changes::remove(path);
    }

    let mut body = kept.join("\n");
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    crate::config_changes::write(path, body.as_bytes(), 0o600)
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
        .and_then(assigned_value))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Holds the app-support redirect for one test, and clears it on drop.
    ///
    /// `add_vars` and `remove_vars` stamp `config-changes.json` under the
    /// app-support dir, so without the redirect every run of these tests
    /// stamped the developer's real one. The redirect is process-global, hence
    /// `crate::env::path_env_lock`, released after the override is cleared.
    struct Store(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

    impl Drop for Store {
        fn drop(&mut self) {
            crate::env::set_app_support_dir_for_tests(None);
        }
    }

    fn tmp() -> (Store, std::path::PathBuf) {
        let lock = crate::env::path_env_lock();
        let mut p = std::env::temp_dir();
        p.push(format!(
            "gate-dotenv-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        crate::env::set_app_support_dir_for_tests(Some(p.join("app-support")));
        (Store(lock), p.join(".env"))
    }

    /// A key a previous connect wrote, with the value it left on disk.
    fn wrote(key: &str, value: &str) -> Owned {
        Owned {
            key: key.to_string(),
            value: Some(value.to_string()),
        }
    }

    #[test]
    fn adds_only_missing_keys_and_never_clobbers_the_users() {
        let (_store, path) = tmp();
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
        let (store, path) = tmp();
        fs::write(
            &path,
            "OPENROUTER_API_KEY=sk-user\nHTTPS_PROXY=http://127.0.0.1:9977\nHERMES_CA_BUNDLE=/old/ca.pem\n",
        )
        .unwrap();

        // The port moved. Both keys below are ours per the sidecar, recorded
        // with the values a previous connect left there.
        let ours = vec![
            wrote("HTTPS_PROXY", "http://127.0.0.1:9977"),
            wrote("HERMES_CA_BUNDLE", "/old/ca.pem"),
        ];
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

        // The same staleness on a key the user owns is left alone. A fresh
        // file, so the first one's hold on the store goes first.
        drop(store);
        let (_store, path) = tmp();
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
        let (_store, path) = tmp();
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
        let (_store, path) = tmp();
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
        let (_store, path) = tmp();
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

    /// A value the user has edited by hand stops being ours, even though we are
    /// the ones who put the key there.
    ///
    /// Ownership of a key alone would be a standing claim on the line: having
    /// written `HTTPS_PROXY` once, every later connect would overwrite whatever
    /// it found, including a deliberate hand-edit pointing Hermes at the user's
    /// own mitmproxy. The recorded value is what closes that: we refresh only
    /// while the line still holds what we left there.
    #[test]
    fn a_value_the_user_edited_is_no_longer_ours() {
        let (_store, path) = tmp();
        // We wrote :9977 on a previous connect. The user has since repointed it
        // at a proxy of their own, on the same key.
        fs::write(&path, "HTTPS_PROXY=http://127.0.0.1:8080\n").unwrap();

        let applied = add_vars(
            &path,
            &[("HTTPS_PROXY", "http://127.0.0.1:45981".into())],
            &[wrote("HTTPS_PROXY", "http://127.0.0.1:9977")],
        )
        .unwrap();

        assert_eq!(applied.refreshed, Vec::<String>::new());
        assert_eq!(applied.added, Vec::<String>::new());
        assert_eq!(
            applied.owned_values,
            Vec::new(),
            "a line that is no longer ours must not be recorded as ours again"
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "HTTPS_PROXY=http://127.0.0.1:8080\n",
            "the user's hand-edit survives"
        );
    }

    /// A sidecar written before values were recorded still repairs, once.
    #[test]
    fn a_key_owned_without_a_recorded_value_is_refreshed_once() {
        let (_store, path) = tmp();
        fs::write(&path, "HTTPS_PROXY=http://127.0.0.1:9977\n").unwrap();

        let legacy = Owned {
            key: "HTTPS_PROXY".to_string(),
            value: None,
        };
        let applied = add_vars(
            &path,
            &[("HTTPS_PROXY", "http://127.0.0.1:45981".into())],
            &[legacy],
        )
        .unwrap();

        assert_eq!(applied.refreshed, vec!["HTTPS_PROXY".to_string()]);
        // Recorded on the way out, so the next connect owns a value rather than
        // a bare key and the migration window closes after this one call.
        assert_eq!(
            applied.owned_values,
            vec![(
                "HTTPS_PROXY".to_string(),
                "http://127.0.0.1:45981".to_string()
            )]
        );
    }

    /// Correcting one line of ours must not reformat the rest of the file.
    ///
    /// `lines()` drops the `\r` from every line it touches, so rebuilding the
    /// body that way rewrote the user's API key line as a side effect of fixing
    /// our proxy line. On Windows that is the whole file, unattended, on every
    /// launch: rule 1 broken in a way no Unix test could see.
    #[test]
    fn a_crlf_file_keeps_its_line_endings() {
        let (_store, path) = tmp();
        fs::write(
            &path,
            "OPENROUTER_API_KEY=sk-user\r\nHTTPS_PROXY=http://127.0.0.1:9977\r\n",
        )
        .unwrap();

        let applied = add_vars(
            &path,
            &[
                ("HTTPS_PROXY", "http://127.0.0.1:45981".into()),
                ("NO_PROXY", "localhost".into()),
            ],
            &[wrote("HTTPS_PROXY", "http://127.0.0.1:9977")],
        )
        .unwrap();

        assert_eq!(applied.refreshed, vec!["HTTPS_PROXY".to_string()]);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "OPENROUTER_API_KEY=sk-user\r\nHTTPS_PROXY=http://127.0.0.1:45981\r\nNO_PROXY=localhost\r\n",
            "every line keeps CRLF, including the one we append"
        );
    }

    /// A duplicate assignment of a key of ours collapses to one correct line.
    ///
    /// This was the one case that did not converge. With the correct value
    /// first and a stale copy after it, the old exact-line check found a
    /// matching line and skipped: a last-wins reader kept using the stale copy
    /// while `read_var`, which takes the first, reported the fresh one, so
    /// status read Connected and the unattended repair never ran again.
    #[test]
    fn a_duplicate_assignment_of_ours_collapses() {
        let (_store, path) = tmp();
        fs::write(
            &path,
            "HTTPS_PROXY=http://127.0.0.1:45981\nUSER_KEY=v\nHTTPS_PROXY=http://127.0.0.1:9977\n",
        )
        .unwrap();

        let applied = add_vars(
            &path,
            &[("HTTPS_PROXY", "http://127.0.0.1:45981".into())],
            // Legacy ownership: no recorded value, so the mixed pair below is
            // still ours to tidy.
            &[Owned {
                key: "HTTPS_PROXY".to_string(),
                value: None,
            }],
        )
        .unwrap();

        assert_eq!(applied.refreshed, vec!["HTTPS_PROXY".to_string()]);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "HTTPS_PROXY=http://127.0.0.1:45981\nUSER_KEY=v\n",
            "one correct assignment survives, in the first position"
        );

        // And it converges: a second pass is a no-op.
        let again = add_vars(
            &path,
            &[("HTTPS_PROXY", "http://127.0.0.1:45981".into())],
            &[wrote("HTTPS_PROXY", "http://127.0.0.1:45981")],
        )
        .unwrap();
        assert_eq!(again.refreshed, Vec::<String>::new());
    }

    /// An `export` line of ours keeps its keyword and its quotes.
    ///
    /// These files are `source`d as often as they are parsed, so dropping the
    /// keyword turns an exported variable into a shell local: the line still
    /// reads correctly and the tool is no longer routed. The already-correct
    /// check also has to see through the quoting, or an `export KEY="right"`
    /// line is "repaired" on every connect and Hermes is told to restart for a
    /// change that never happened.
    #[test]
    fn an_export_line_of_ours_keeps_its_form() {
        let (_store, path) = tmp();
        fs::write(&path, "export HTTPS_PROXY=\"http://127.0.0.1:9977\"\n").unwrap();

        let applied = add_vars(
            &path,
            &[("HTTPS_PROXY", "http://127.0.0.1:45981".into())],
            &[wrote("HTTPS_PROXY", "http://127.0.0.1:9977")],
        )
        .unwrap();

        assert_eq!(applied.refreshed, vec!["HTTPS_PROXY".to_string()]);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "export HTTPS_PROXY=\"http://127.0.0.1:45981\"\n",
            "the keyword and the quoting are the user's formatting, not ours"
        );

        // Already correct through the quotes, so not a change at all.
        let again = add_vars(
            &path,
            &[("HTTPS_PROXY", "http://127.0.0.1:45981".into())],
            &[wrote("HTTPS_PROXY", "http://127.0.0.1:45981")],
        )
        .unwrap();
        assert_eq!(again.refreshed, Vec::<String>::new());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "export HTTPS_PROXY=\"http://127.0.0.1:45981\"\n"
        );
    }

    /// A file with no trailing newline gains one line, not a joined line.
    #[test]
    fn a_file_without_a_trailing_newline_is_appended_to_cleanly() {
        let (_store, path) = tmp();
        fs::write(&path, "USER_KEY=v").unwrap();

        add_vars(&path, &[("NO_PROXY", "localhost".into())], &[]).unwrap();

        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "USER_KEY=v\nNO_PROXY=localhost\n"
        );
    }
}
