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
use std::path::PathBuf;

use crate::account::{self, AuthMode};
use crate::env;
use crate::primitives;
use crate::registry::ToolId;

/// The file's whole contents: one reading, and the scope it belongs to.
///
/// Deliberately one entry rather than a map keyed by scope. The picker's other
/// installations are a browsing affordance, not the thing the user came for, and
/// a map would keep every org this machine has ever signed into on disk for as
/// long as the app is installed.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheFile {
    /// Gateway, credential kind, org, installation filter and tool filter,
    /// joined. Compared whole; the parts are never read back out.
    scope: String,
    /// The `/v1/me/activity` response, verbatim. It carries its own
    /// `generatedAt`, so the age of this reading needs no second timestamp that
    /// could disagree with it.
    body: String,
}

fn config_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join("activity-cache.json"))
}

/// Which reading this is: everything that changes what the gateway would answer.
///
/// Returns `None` when there is no account, which is also when there is nothing
/// worth caching - a signed-out client has no reading to hold and no identity to
/// hold it under.
fn scope(install_id: Option<&str>, tool: Option<ToolId>) -> Option<String> {
    let account = account::load().ok().flatten()?;
    let mode = match account.auth_mode {
        AuthMode::OAuth => "oauth",
        AuthMode::ApiKey => "api_key",
    };
    // The org only exists in OAuth mode; a key account's org is whatever the
    // gateway resolves the key to, which the base URL and the key mode already
    // stand in for.
    let org = account::org_id_for_injection();
    // The tool filter is part of the scope for the same reason the installation
    // filter is, and the consequence of forgetting it is the sharper of the two:
    // installations sit behind a picker, but a tool is one click in the sidebar,
    // so Codex opened right after Claude Code would draw Claude Code's numbers
    // under Codex's name until the network answered.
    Some(format!(
        "{}|{}|{}|{}|{}",
        account.gateway_base_url,
        mode,
        org,
        install_id.unwrap_or(""),
        tool.map(ToolId::slug).unwrap_or("")
    ))
}

/// Hold a reading that just landed. Best effort: a cache that cannot be written
/// is not a failed fetch, and the caller has the real answer in hand either way.
pub fn store(install_id: Option<&str>, tool: Option<ToolId>, body: &str) {
    let Some(scope) = scope(install_id, tool) else {
        return;
    };
    let entry = CacheFile {
        scope,
        body: body.to_owned(),
    };
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
    let want = scope(install_id, tool)?;
    let path = config_path().ok()?;
    let raw = std::fs::read_to_string(path).ok()?;
    let entry: CacheFile = serde_json::from_str(&raw).ok()?;
    (entry.scope == want).then_some(entry.body)
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

    /// The property the whole module rests on: a body written for one scope is
    /// never handed to another. Exercised on the struct rather than through the
    /// filesystem, because the scope string is the part that decides it.
    #[test]
    fn a_body_is_only_returned_for_its_own_scope() {
        let entry = CacheFile {
            scope: "https://gw.example|oauth|org-a|install-7|claude-code".into(),
            body: r#"{"counters":{}}"#.into(),
        };
        assert_eq!(
            (entry.scope == "https://gw.example|oauth|org-a|install-7|claude-code")
                .then_some(entry.body.clone()),
            Some(entry.body.clone())
        );
        assert_eq!(
            (entry.scope == "https://gw.example|oauth|org-b|install-7|claude-code")
                .then_some(entry.body.clone()),
            None,
            "another org's reading must not be replayed"
        );
        assert_eq!(
            (entry.scope == "https://gw.example|oauth|org-a|install-7|codex")
                .then_some(entry.body.clone()),
            None,
            "another tool's reading must not be replayed"
        );
    }

    /// An installation filter changes what the gateway answers, so it has to
    /// change the scope too - otherwise selecting one machine would show the
    /// org-wide figures it just replaced.
    #[test]
    fn the_installation_filter_is_part_of_the_scope() {
        let org_wide = "https://gw.example|oauth|org-a||";
        let one_machine = "https://gw.example|oauth|org-a|install-7|";
        assert_ne!(org_wide, one_machine);
    }

    /// The same property for the tool dimension, and the likelier of the two to
    /// be noticed: a tool is one click in the sidebar, so a shared scope would
    /// draw the previous tool's numbers under the new tool's name.
    #[test]
    fn the_tool_filter_is_part_of_the_scope() {
        let every_tool = "https://gw.example|oauth|org-a|install-7|";
        let one_tool = "https://gw.example|oauth|org-a|install-7|claude-code";
        let other_tool = "https://gw.example|oauth|org-a|install-7|codex";
        assert_ne!(every_tool, one_tool);
        assert_ne!(one_tool, other_tool);
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
        let entry = CacheFile {
            scope: "s".into(),
            body: r#"{"generatedAt":"2026-08-18T09:00:00Z"}"#.into(),
        };
        let raw = serde_json::to_string(&entry).expect("serialize");
        let back: CacheFile = serde_json::from_str(&raw).expect("deserialize");
        assert_eq!(back.scope, entry.scope);
        assert_eq!(back.body, entry.body);
    }
}
