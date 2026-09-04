//! The last activity reading that landed, kept on disk so the Overview can open
//! on real numbers instead of on nothing (AG-576).
//!
//! Non-secret aggregates about the signed-in org, so a plain JSON file next to
//! `preferences.json` rather than the keychain. Same reasoning as
//! [`crate::preferences`]: it is a convenience, it is worthless to an attacker
//! who already has the user's disk, and a corrupt file must degrade to "no
//! cache" rather than stop the pane opening.
//!
//! **Why a cache at all.** The window is opened mid-task, for a few seconds, and
//! the endpoint is deliberately not polled (see [`crate::activity`]). Without
//! this, every open costs a network round trip before a single figure appears,
//! and an open with no connectivity shows nothing at all. Holding the previous
//! answer means the user always has the last thing that actually happened to
//! their traffic in front of them, which is the reference AG-576 asks for.
//!
//! **Why it is scoped.** The body is one org's traffic, read for one credential
//! and one installation filter. Replaying it under a different org's name would
//! be worse than showing nothing, so the file records the scope it was taken for
//! and [`load`] refuses to answer for any other. Reads never fall back to a
//! neighbouring scope, and a missing scope is a miss, not a wildcard.
//!
//! The body is stored as the raw response text, exactly as [`crate::activity`]
//! returns it: `src/lib/activity.ts` is still the only place that knows the
//! payload's shape, and a cache that parsed it would be a second model to keep
//! in step.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::account::{self, AuthMode};
use crate::env;
use crate::primitives;
use crate::registry::ToolId;

/// The file's whole contents: the readings for **one** scope, keyed by tool.
///
/// Still one scope, deliberately, and for the reason it always was: the picker's
/// other installations are a browsing affordance, not the thing the user came
/// for, and keying the file by scope would keep every org this machine has ever
/// signed into on disk for as long as the app is installed. A scope that does not
/// match is a miss and the file is replaced, exactly as before.
///
/// What is keyed now is the **tool**, so the tray's quick status can draw a
/// figure per row from one disk read instead of asking the gateway per row. The
/// map is bounded by [`crate::registry`] plus the org-wide entry - six or so -
/// and it holds counts for one org at a time, which is the property the single
/// slot was protecting.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheFile {
    /// Gateway, credential kind, org and installation filter, joined. Compared
    /// whole; the parts are never read back out.
    scope: String,
    /// Tool slug -> the `/v1/me/activity` response for it, verbatim. The
    /// org-wide reading (no `tool` filter) is held under [`ORG_WIDE`].
    ///
    /// Each body carries its own `generatedAt`, so the age of a reading needs no
    /// second timestamp here that could disagree with it - which is also what
    /// lets a caller decide for itself whether an entry is too old to use.
    ///
    /// A file written by the previous single-slot shape has no `tools` and fails
    /// to parse, which every caller already handles as a miss - and the next
    /// store replaces it. Deliberately *not* `#[serde(default)]`: a half-written
    /// entry loading as an empty map would be indistinguishable from a scope that
    /// genuinely holds nothing, and `a_mangled_file_reads_as_no_cache` pins that.
    tools: BTreeMap<String, String>,
}

/// The key the reading with no `tool` filter is held under - the Overview's own.
/// The empty string cannot collide with a slug.
const ORG_WIDE: &str = "";

fn config_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join("activity-cache.json"))
}

/// Which reading this is: everything that changes what the gateway would answer.
///
/// Returns `None` when there is no account, which is also when there is nothing
/// worth caching - a signed-out client has no reading to hold and no identity to
/// hold it under.
fn scope(install_id: Option<&str>) -> Option<String> {
    let account = account::load().ok().flatten()?;
    let mode = match account.auth_mode {
        AuthMode::OAuth => "oauth",
        AuthMode::ApiKey => "api_key",
    };
    // The org id only exists in OAuth mode. In api-key mode the org is whatever
    // the gateway resolves the *key* to, so the key has to be in the scope or two
    // different orgs share one string: `org_id_for_injection()` returns "" for
    // every key account, and `save()` rotates a key without clearing this cache.
    // Settings -> "Replace key" with a key for a different org on the same gateway
    // would then leave the scope byte-identical and `load()` would hand back the
    // previous org's body - the exact replay this module exists to prevent.
    //
    // The *prefix*, not the key: it is already on disk unhashed for the Settings
    // reveal, it changes whenever the key does, and putting a live credential in a
    // filename-adjacent string that also gets compared and logged would be a worse
    // trade for the same guarantee.
    let org = account::org_id_for_injection();
    let key = account::api_key_prefix().ok().flatten().unwrap_or_default();
    // The tool used to be part of this string. It is the map key now, which keeps
    // the property that mattered: Codex opened right after Claude Code must not
    // draw Claude Code's numbers under Codex's name while the network answers.
    // Two entries under two keys cannot be confused for each other; one entry
    // under a scope that ignored the tool could.
    Some(format!(
        "{}|{}|{}|{}|{}",
        account.gateway_base_url,
        mode,
        org,
        key,
        install_id.unwrap_or("")
    ))
}

/// The map key for a tool filter, or [`ORG_WIDE`] for none.
fn key(tool: Option<ToolId>) -> String {
    tool.map(ToolId::slug).unwrap_or(ORG_WIDE).to_owned()
}

/// The file, if it holds this scope. `None` covers every failure and a scope
/// that does not match, which mean the same thing: nothing usable on disk.
fn held(scope: &str) -> Option<CacheFile> {
    let path = config_path().ok()?;
    let raw = std::fs::read_to_string(path).ok()?;
    let entry: CacheFile = serde_json::from_str(&raw).ok()?;
    (entry.scope == scope).then_some(entry)
}

/// Hold a reading that just landed. Best effort: a cache that cannot be written
/// is not a failed fetch, and the caller has the real answer in hand either way.
pub fn store(install_id: Option<&str>, tool: Option<ToolId>, body: &str) {
    let Some(scope) = scope(install_id) else {
        return;
    };
    // Merged into whatever this scope already holds, so one tool's read does not
    // evict the others - the whole point of the map. A file from another scope is
    // replaced rather than merged into: its counts belong to an org the user has
    // left.
    let mut entry = held(&scope).unwrap_or_else(|| CacheFile {
        scope: scope.clone(),
        tools: BTreeMap::new(),
    });
    entry.scope = scope;
    entry.tools.insert(key(tool), body.to_owned());
    let Ok(path) = config_path() else {
        return;
    };
    let Ok(bytes) = serde_json::to_vec(&entry) else {
        return;
    };
    // 0600 rather than the preferences' 0644: these are aggregate counts of one
    // org's traffic, which is nobody else's business on a shared machine.
    let _ = primitives::write_file(&path, &bytes, 0o600);
}

/// The last reading for this scope, or `None`.
///
/// Infallible by design, like [`crate::preferences::load`]: every failure here -
/// no file, unreadable, unparseable, a different org - means the same thing to
/// the caller, which is that it has to wait for the network like it always did.
pub fn load(install_id: Option<&str>, tool: Option<ToolId>) -> Option<String> {
    let want = scope(install_id)?;
    held(&want)?.tools.remove(&key(tool))
}

/// Every per-tool reading held for this scope, keyed by slug.
///
/// One disk read for the whole tray, rather than one per row: the popover draws a
/// figure on each app row and a call per row - even a local one - is a file parse
/// per row for a file that already holds them all.
///
/// The org-wide entry is left out. It is the Overview's reading, it is not
/// attributable to any row, and a caller iterating rows would have to know to
/// skip a key that looks like every other one.
pub fn load_tools(install_id: Option<&str>) -> BTreeMap<String, String> {
    let Some(want) = scope(install_id) else {
        return BTreeMap::new();
    };
    let Some(mut entry) = held(&want) else {
        return BTreeMap::new();
    };
    entry.tools.remove(ORG_WIDE);
    entry.tools
}

/// Forget the held reading. Called when the account goes away: a disconnect or a
/// reset must not leave one org's figures on disk for the next person to sign in
/// on this machine to see flash past.
pub fn clear() -> Result<()> {
    let path = config_path()?;
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property the whole module rests on: a reading written for one scope is
    /// never handed to another. Exercised on the struct rather than through the
    /// filesystem, because the scope string is the part that decides it.
    ///
    /// The tool is no longer in that string - it is the map key - so the two
    /// halves are now asserted separately: the scope guards the org and the
    /// machine, `each_tool_keeps_its_own_entry` guards the tool.
    #[test]
    fn a_reading_is_only_returned_for_its_own_scope() {
        let mut tools = BTreeMap::new();
        tools.insert("claude-code".to_owned(), r#"{"counters":{}}"#.to_owned());
        let entry = CacheFile {
            scope: "https://gw.example|oauth|org-a|install-7".into(),
            tools,
        };
        let for_scope =
            |want: &str| (entry.scope == want).then(|| entry.tools.get("claude-code").cloned());
        assert_eq!(
            for_scope("https://gw.example|oauth|org-a|install-7"),
            Some(Some(r#"{"counters":{}}"#.to_owned()))
        );
        assert_eq!(
            for_scope("https://gw.example|oauth|org-b|install-7"),
            None,
            "another org's reading must not be replayed"
        );
        assert_eq!(
            for_scope("https://gw.example|oauth|org-a|install-9"),
            None,
            "another machine's reading must not be replayed"
        );
    }

    /// An installation filter changes what the gateway answers, so it has to
    /// change the scope too - otherwise selecting one machine would show the
    /// org-wide figures it just replaced.
    #[test]
    fn the_installation_filter_is_part_of_the_scope() {
        let org_wide = "https://gw.example|oauth|org-a|";
        let one_machine = "https://gw.example|oauth|org-a|install-7";
        assert_ne!(org_wide, one_machine);
    }

    /// The same property for the tool dimension, which the map key carries now
    /// rather than the scope string. Still the likelier of the two to be noticed:
    /// a tool is one click in the sidebar, so two tools sharing a slot would draw
    /// the previous tool's numbers under the new tool's name.
    #[test]
    fn each_tool_keeps_its_own_entry() {
        let mut file = CacheFile {
            scope: "s".into(),
            tools: BTreeMap::new(),
        };
        file.tools
            .insert(key(Some(ToolId::ClaudeCode)), "cc".into());
        file.tools.insert(key(Some(ToolId::Codex)), "cx".into());
        file.tools.insert(key(None), "org".into());

        assert_eq!(
            file.tools.get("claude-code").map(String::as_str),
            Some("cc")
        );
        assert_eq!(file.tools.get("codex").map(String::as_str), Some("cx"));
        assert_eq!(
            file.tools.get(ORG_WIDE).map(String::as_str),
            Some("org"),
            "the unfiltered reading has a key of its own, and the empty string \
             cannot collide with a slug",
        );
    }

    /// The api-key replay, exercised through `scope()` and the real filesystem
    /// rather than against hand-written strings.
    ///
    /// The hand-written tests above cannot catch this class: they assert that two
    /// strings differ, which says nothing about whether `scope()` puts the key in
    /// one. In api-key mode `org_id_for_injection()` is empty for every account, so
    /// before the prefix was included, two keys belonging to two different orgs on
    /// one gateway produced byte-identical scopes and `load()` replayed the first
    /// org's body under the second's name.
    #[test]
    fn replacing_an_api_key_changes_the_scope() {
        // Both seams are process-global, so another test redirecting them while
        // this one is between its `save()` and its `scope()` would have it read a
        // home with no account in it. See `crate::env::path_env_lock`.
        let _lock = crate::env::path_env_lock();
        let home = std::env::temp_dir().join(format!("gate-cache-scope-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        // Both seams: the account file lives under app support, and `save()` writes
        // the key itself through the keychain, which must not touch the real one.
        let prev_home = std::env::var_os("GATE_CONNECT_TEST_HOME");
        let prev_secrets = std::env::var_os("GATE_CONNECT_TEST_SECRETS");
        std::env::set_var("GATE_CONNECT_TEST_HOME", &home);
        std::env::set_var("GATE_CONNECT_TEST_SECRETS", home.join("secrets"));

        account::save("https://gw.example", Some("sk-gw-aaaaaaaaaaaa1111")).unwrap();
        let first = scope(None).expect("an account exists");
        account::save("https://gw.example", Some("sk-gw-bbbbbbbbbbbb2222")).unwrap();
        let second = scope(None).expect("an account exists");

        // Restore rather than clear: an ambient value belongs to whoever set it.
        let restore = |k: &str, v: Option<std::ffi::OsString>| match v {
            Some(v) => std::env::set_var(k, v),
            None => std::env::remove_var(k),
        };
        restore("GATE_CONNECT_TEST_HOME", prev_home);
        restore("GATE_CONNECT_TEST_SECRETS", prev_secrets);
        let _ = std::fs::remove_dir_all(&home);

        assert_ne!(
            first, second,
            "a different key can mean a different org, so its reading is not the same reading"
        );
    }

    /// The merge, through the real filesystem: storing one tool must not evict
    /// another. A slot that held one reading is what made a per-row figure
    /// impossible - the tray's second row would evict the first - so this is the
    /// property the whole change exists for.
    #[test]
    fn storing_one_tool_leaves_the_others_alone() {
        // Both seams are process-global; see `replacing_an_api_key_changes_the_scope`.
        let _lock = crate::env::path_env_lock();
        let home = std::env::temp_dir().join(format!("gate-cache-merge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let prev_home = std::env::var_os("GATE_CONNECT_TEST_HOME");
        let prev_secrets = std::env::var_os("GATE_CONNECT_TEST_SECRETS");
        std::env::set_var("GATE_CONNECT_TEST_HOME", &home);
        std::env::set_var("GATE_CONNECT_TEST_SECRETS", home.join("secrets"));
        account::save("https://gw.example", Some("sk-gw-aaaaaaaaaaaa1111")).unwrap();

        store(None, Some(ToolId::ClaudeCode), r#"{"tool":"cc"}"#);
        store(None, Some(ToolId::Codex), r#"{"tool":"cx"}"#);
        store(None, None, r#"{"tool":"org"}"#);

        let cc = load(None, Some(ToolId::ClaudeCode));
        let cx = load(None, Some(ToolId::Codex));
        let org = load(None, None);
        let rows = load_tools(None);
        // A different machine is a different scope, so the file is replaced -
        // which must not leave the org-wide slot answering for it either.
        store(Some("install-7"), Some(ToolId::Codex), r#"{"tool":"cx-7"}"#);
        let after_scope_change = load(None, Some(ToolId::ClaudeCode));

        let restore = |k: &str, v: Option<std::ffi::OsString>| match v {
            Some(v) => std::env::set_var(k, v),
            None => std::env::remove_var(k),
        };
        restore("GATE_CONNECT_TEST_HOME", prev_home);
        restore("GATE_CONNECT_TEST_SECRETS", prev_secrets);
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(cc.as_deref(), Some(r#"{"tool":"cc"}"#));
        assert_eq!(
            cx.as_deref(),
            Some(r#"{"tool":"cx"}"#),
            "the second tool's store must not have evicted the first"
        );
        assert_eq!(org.as_deref(), Some(r#"{"tool":"org"}"#));
        assert_eq!(
            rows.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["claude-code", "codex"],
            "the org-wide reading is not a row and must not be handed out as one"
        );
        assert_eq!(
            after_scope_change, None,
            "a scope change drops the file rather than merging another machine's \
             readings into it"
        );
    }

    #[test]
    fn a_mangled_file_reads_as_no_cache() {
        assert!(serde_json::from_str::<CacheFile>("not json").is_err());
        assert!(
            serde_json::from_str::<CacheFile>(r#"{"scope":"a"}"#).is_err(),
            "a half-written entry must not load as an empty body"
        );
    }

    #[test]
    fn an_entry_survives_a_round_trip() {
        let mut tools = BTreeMap::new();
        tools.insert(
            "claude-code".to_owned(),
            r#"{"generatedAt":"2026-08-18T09:00:00Z"}"#.to_owned(),
        );
        let entry = CacheFile {
            scope: "s".into(),
            tools,
        };
        let raw = serde_json::to_string(&entry).expect("serialize");
        let back: CacheFile = serde_json::from_str(&raw).expect("deserialize");
        assert_eq!(back.scope, entry.scope);
        assert_eq!(back.tools, entry.tools);
    }

    /// A file the previous shape wrote. It has no `tools`, so it does not load -
    /// which is a miss, the state every caller already waits out, rather than a
    /// reading of nothing.
    #[test]
    fn the_previous_single_slot_file_reads_as_no_cache() {
        let old = r#"{"scope":"https://gw.example|oauth|org-a||","body":"{}"}"#;
        assert!(serde_json::from_str::<CacheFile>(old).is_err());
    }
}
