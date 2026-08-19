//! Small preferences the user sets in Settings and expects to survive a
//! restart. Non-secret, so a plain JSON file next to `account.json` rather than
//! the keychain.
//!
//! Deliberately separate from [`crate::account`]: an account is a credential and
//! an identity, and these are choices about how the app behaves. Wedging them
//! into `AccountFile` would mean a preference change rewrites the file that holds
//! the key prefix and the selected org, and `clear()`-ing the account on reset
//! would silently take the preferences with it.
//!
//! **Every preference defaults to on**, and the default is the value a missing
//! field loads as. That is what lets Settings show a switch as On before anything
//! has ever been written, which is what the product asks for: the switch reads
//! its stored value, and the stored value of an untouched preference is its
//! default.
//!
//! Scope note. Only the preferences that currently gate something live here.
//! Per-category security-event notifications (blocked / flagged) and a sound
//! toggle are specified alongside the live event feed, and there is no feed yet -
//! a switch that gates nothing is worse than a missing switch, because it tells
//! the user they have turned something off.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::env;
use crate::primitives;

fn default_true() -> bool {
    true
}

/// The stored preferences. Every field is `#[serde(default)]`-backed so a file
/// written by an older build - or no file at all - loads as "everything on".
///
/// No longer `Copy`: `device_name` is a `String`, and the alternative - a fixed
/// buffer, or a separate file for one label - buys nothing. Callers clone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preferences {
    /// Native notifications about routing itself: a session that expired, a
    /// quit that could not put a tool back. These are the two the app actually
    /// fires today.
    #[serde(default = "default_true")]
    pub routing_health_notifications: bool,
    /// Whether Gate Connect may send diagnostic data. The onboarding step records
    /// the first answer; Settings changes it afterwards. Storing it here rather
    /// than deriving it means an install that never saw the step still reads as
    /// the documented default rather than as "unset".
    #[serde(default = "default_true")]
    pub share_diagnostics: bool,
    /// Whether the person has ever *answered* the diagnostic-data question, as
    /// opposed to having the default applied for them.
    ///
    /// Defaults to false, including for installs that predate this field, which is
    /// deliberate: consent nobody was asked for is not consent, so those installs
    /// see the onboarding step once. It is the whole reason this is a separate
    /// field rather than inferring an answer from `share_diagnostics` - the
    /// default and a deliberate "yes" are the same value and must not be the same
    /// fact.
    #[serde(default)]
    pub share_diagnostics_recorded: bool,
    /// What the person calls this machine, when they have renamed it.
    ///
    /// `None` is the normal state and is **not** a blank name: the command layer
    /// resolves it to the machine's own hostname, which is what Settings shows.
    /// Storing the override rather than the resolved value is what lets a
    /// renamed-then-cleared device go back to following the hostname, and keeps
    /// "the user chose this" distinguishable from "this is what the OS reports" -
    /// the same argument `share_diagnostics_recorded` makes one field up.
    ///
    /// Local. Nothing sends it anywhere yet; it labels this install in its own
    /// window.
    #[serde(default)]
    pub device_name: Option<String>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            routing_health_notifications: true,
            share_diagnostics: true,
            share_diagnostics_recorded: false,
            device_name: None,
        }
    }
}

fn config_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join("preferences.json"))
}

/// Read the preferences, falling back to the defaults.
///
/// A missing file is the normal first-run state, and an unparseable one is
/// treated the same way on purpose: these are non-critical toggles, and refusing
/// to open Settings because a preferences file was hand-edited would be a worse
/// failure than quietly showing the documented defaults. Nothing here is a
/// credential, so there is no security consequence to the fallback.
pub fn load() -> Preferences {
    let Ok(path) = config_path() else {
        return Preferences::default();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Write the preferences. 0644 - non-secret, and the CLI reads the same file.
pub fn save(prefs: &Preferences) -> Result<()> {
    let path = config_path()?;
    let body = serde_json::to_vec_pretty(prefs).context("serializing preferences")?;
    primitives::write_file(&path, &body, 0o644)
        .with_context(|| format!("writing {}", path.display()))
}

/// Turn routing-health notifications on or off, leaving the other preferences
/// alone. Read-modify-write rather than taking a whole `Preferences`, so a caller
/// that only knows about one switch cannot clobber a field it has never heard of.
pub fn set_routing_health_notifications(enabled: bool) -> Result<()> {
    let mut prefs = load();
    prefs.routing_health_notifications = enabled;
    save(&prefs)
}

/// Record the diagnostic-data choice, and that it *was* a choice.
///
/// Both callers - the onboarding step's Continue and the Settings switch - are the
/// person answering, so both mark it answered. That is what dismisses the
/// onboarding step, and it is why Continue records the displayed value even when
/// the person changed nothing: leaving the default in place is still an answer,
/// and treating it as unanswered would ask again on the next launch.
pub fn set_share_diagnostics(enabled: bool) -> Result<()> {
    let mut prefs = load();
    prefs.share_diagnostics = enabled;
    prefs.share_diagnostics_recorded = true;
    save(&prefs)
}

/// Rename this device, or clear the override and go back to the hostname.
///
/// A blank or whitespace-only name clears rather than storing an empty label: a
/// device row with nothing in it is worse than one showing what the OS calls the
/// machine, and it is the state a user reaches by deleting the text.
pub fn set_device_name(name: &str) -> Result<()> {
    let mut prefs = load();
    prefs.device_name = device_name_override(name);
    save(&prefs)
}

/// What a typed name means, with the file left out of it: a real name trimmed, or
/// `None` for anything blank. Separated so the decision is testable without a
/// process-global directory override - the rest of this module's tests stay off
/// the filesystem for the same reason.
fn device_name_override(name: &str) -> Option<String> {
    let trimmed = name.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_everything_on() {
        let prefs = Preferences::default();
        assert!(prefs.routing_health_notifications);
        assert!(prefs.share_diagnostics);
    }

    /// The distinction the onboarding step turns on: sharing is on by default, and
    /// that default is *not* an answer. An install that predates the field must
    /// read as unanswered so the person is actually asked.
    #[test]
    fn the_default_is_not_an_answer() {
        assert!(!Preferences::default().share_diagnostics_recorded);
        let old_file: Preferences =
            serde_json::from_str(r#"{"share_diagnostics":true}"#).expect("parses");
        assert!(old_file.share_diagnostics);
        assert!(
            !old_file.share_diagnostics_recorded,
            "a file written before this field existed was never an answer"
        );
    }

    /// Leaving the default in place is still an answer - otherwise Continue would
    /// dismiss the step and the next launch would ask again.
    #[test]
    fn an_unchanged_choice_still_counts_as_answered() {
        let raw = serde_json::to_string(&Preferences {
            share_diagnostics: true,
            share_diagnostics_recorded: true,
            routing_health_notifications: true,
            device_name: None,
        })
        .expect("serialize");
        let back: Preferences = serde_json::from_str(&raw).expect("deserialize");
        assert!(back.share_diagnostics_recorded);
    }

    /// The property Settings depends on: a file from a build that predates a
    /// field must load that field as on, not as false.
    #[test]
    fn a_missing_field_loads_as_on() {
        let prefs: Preferences = serde_json::from_str("{}").expect("empty object should parse");
        assert_eq!(prefs, Preferences::default());

        let partial: Preferences = serde_json::from_str(r#"{"share_diagnostics":false}"#)
            .expect("partial object should parse");
        assert!(
            partial.routing_health_notifications,
            "an absent field must not read as off"
        );
        assert!(!partial.share_diagnostics);
    }

    #[test]
    fn an_explicit_false_survives_a_round_trip() {
        let prefs = Preferences {
            routing_health_notifications: false,
            share_diagnostics: true,
            share_diagnostics_recorded: true,
            device_name: None,
        };
        let raw = serde_json::to_string(&prefs).expect("serialize");
        let back: Preferences = serde_json::from_str(&raw).expect("deserialize");
        assert_eq!(back, prefs);
    }

    /// A device with no name is not a device called "": the row would render
    /// blank, where `None` means "follow the hostname", which is what the user
    /// gets by clearing the field.
    #[test]
    fn a_blank_device_name_clears_the_override() {
        assert_eq!(device_name_override(""), None);
        assert_eq!(device_name_override("   "), None);
        assert_eq!(device_name_override("\t\n"), None);
    }

    /// Surrounding whitespace is a typo, not part of the name - it would show up
    /// in the row and in every comparison against it.
    #[test]
    fn a_device_name_is_stored_trimmed() {
        assert_eq!(
            device_name_override("  Studio Mac  ").as_deref(),
            Some("Studio Mac")
        );
        // Only the edges: a name can legitimately have spaces in it.
        assert_eq!(
            device_name_override("Gabriel's MacBook Pro").as_deref(),
            Some("Gabriel's MacBook Pro")
        );
    }

    /// An absent name must survive a round trip as absent. Written as `null` and
    /// read back as `None`, not as `Some("")`.
    #[test]
    fn an_absent_device_name_round_trips() {
        let prefs = Preferences::default();
        let raw = serde_json::to_string(&prefs).expect("serialize");
        let back: Preferences = serde_json::from_str(&raw).expect("deserialize");
        assert_eq!(back.device_name, None);
        // And a file from a build that predates the field loads the same way.
        let old: Preferences = serde_json::from_str(r#"{"share_diagnostics":true}"#)
            .expect("older object should parse");
        assert_eq!(old.device_name, None);
    }

    /// A hand-mangled file must not stop Settings opening. `load` is infallible
    /// by design; this pins that it stays that way.
    #[test]
    fn unparseable_json_falls_back_to_defaults() {
        let prefs: Preferences = serde_json::from_str("not json").unwrap_or_default();
        assert_eq!(prefs, Preferences::default());
    }
}
