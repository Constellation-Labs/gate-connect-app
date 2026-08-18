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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            routing_health_notifications: true,
            share_diagnostics: true,
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

/// Record the diagnostic-data choice. Same read-modify-write reasoning.
pub fn set_share_diagnostics(enabled: bool) -> Result<()> {
    let mut prefs = load();
    prefs.share_diagnostics = enabled;
    save(&prefs)
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
        };
        let raw = serde_json::to_string(&prefs).expect("serialize");
        let back: Preferences = serde_json::from_str(&raw).expect("deserialize");
        assert_eq!(back, prefs);
    }

    /// A hand-mangled file must not stop Settings opening. `load` is infallible
    /// by design; this pins that it stays that way.
    #[test]
    fn unparseable_json_falls_back_to_defaults() {
        let prefs: Preferences = serde_json::from_str("not json").unwrap_or_default();
        assert_eq!(prefs, Preferences::default());
    }
}
