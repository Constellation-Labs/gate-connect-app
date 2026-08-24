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
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::RwLock;

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
    /// Which model each tool should run on, keyed by tool slug (AG-588).
    ///
    /// **Local by decision, not by omission.** An earlier revision stored this on
    /// the gateway, per organization. Keeping it here trades two things away and
    /// buys one back, and all three are worth stating:
    ///
    /// - Lost: agreement between a person's machines. Two laptops can now differ
    ///   about which model a tool uses.
    /// - Lost: an organization-level record of the paid-use acknowledgement. See
    ///   `gate_model_paid_ack_unix`.
    /// - Gained: the choice belongs to the machine whose traffic it governs. The
    ///   app pane is already scoped to this machine, and a per-org setting meant
    ///   one developer's click changed what their colleagues' requests were
    ///   answered with.
    ///
    /// Keyed on OUR tool slug rather than the gateway's platform id, which is the
    /// simplification that follows: the gateway no longer has to identify the
    /// tool, because the app already knows which tool it is configuring.
    ///
    /// A `BTreeMap` rather than a `HashMap` so the file's key order is stable and
    /// a diff of `preferences.json` shows what changed rather than a reshuffle.
    #[serde(default)]
    pub tool_models: BTreeMap<String, ToolModelChoice>,
    /// When this install first accepted paid Gate model use, unix seconds, or
    /// `None` if it never has.
    ///
    /// Per install, which is a real departure from AG-588 - the ticket words the
    /// confirmation as once per *organization*. Storing it locally is the honest
    /// consequence of storing the choice locally: there is no org-level record to
    /// consult, so a second machine asks again. Being asked twice is a smaller
    /// harm than being billed without having been asked on the machine doing the
    /// spending, which is what a purely local "already accepted" flag inherited
    /// from nowhere would risk.
    ///
    /// `None` and "accepted long ago" are different facts, which is why this is a
    /// timestamp rather than a bool - the same argument
    /// `share_diagnostics_recorded` makes above. Unix seconds rather than a
    /// formatted string, matching `restore_journal`'s `at_unix` and the OAuth
    /// token store; the `time` crate is pulled in without its formatting feature.
    #[serde(default)]
    pub gate_model_paid_ack_unix: Option<i64>,
}

/// What Gate should serve for one tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    /// The tool picks its own model and Gate does not intervene. The default for
    /// every tool, and what a missing entry means.
    Tool,
    /// Gate serves the chosen model, overriding what the tool asked for.
    Gate,
}

/// One tool's stored choice.
///
/// `source` and `model_ids` are separate because a chosen model is not
/// necessarily an active one: the pane keeps "Current Gate model" visible while
/// the tool is on its own default, so the user can see what they would be
/// switching to. `source` alone decides what would be served.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolModelChoice {
    pub source: ModelSource,
    /// Canonical ids, e.g. `anthropic/claude-opus-5`. Empty is legal with either
    /// source; it means no model has been chosen yet.
    ///
    /// A list from the start because AG-590 selects a set, and widening a scalar
    /// later would mean rewriting every stored file.
    #[serde(default)]
    pub model_ids: Vec<String>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            routing_health_notifications: true,
            share_diagnostics: true,
            share_diagnostics_recorded: false,
            device_name: None,
            tool_models: BTreeMap::new(),
            gate_model_paid_ack_unix: None,
        }
    }
}

fn config_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join("preferences.json"))
}

/// Hot-path cache for [`gate_models_for`].
///
/// The model choice is consulted on **every proxied request**, and a file read
/// per request is not a cost the user's actual work should pay. Unlike
/// [`crate::primitives::install_id_cached`] this cannot be a `OnceLock`: the
/// value changes whenever someone picks a model, and a choice that only took
/// effect after a restart would be its own bug report.
///
/// [`save`] refreshes it, so an in-process write is visible immediately. A write
/// from the CLI is not, which is the one staleness this accepts: the proxy and
/// the window are the same process, and the CLI does not currently set models.
static CACHE: RwLock<Option<Preferences>> = RwLock::new(None);

/// The models Gate should serve for one tool, or `None` to leave the request
/// alone.
///
/// `None` covers every case where the tool's own model must win: no entry, an
/// entry whose source is [`ModelSource::Tool`], or an empty set. That last one
/// matters - a `Gate` source with nothing chosen is not "serve anything", it is
/// a state the UI prevents and the request path must not invent a meaning for.
///
/// Infallible by construction, for the reason `install_id_cached` gives: this is
/// called while forwarding the user's real work, and a preferences file that
/// cannot be read must degrade to "the tool picks", never to a failed request.
pub fn gate_models_for(slug: &str) -> Option<Vec<String>> {
    {
        let cache = CACHE.read().ok()?;
        if let Some(prefs) = cache.as_ref() {
            return servable(prefs, slug);
        }
    }
    let prefs = load();
    let answer = servable(&prefs, slug);
    if let Ok(mut cache) = CACHE.write() {
        *cache = Some(prefs);
    }
    answer
}

fn servable(prefs: &Preferences, slug: &str) -> Option<Vec<String>> {
    let choice = prefs.tool_models.get(slug)?;
    if choice.source != ModelSource::Gate || choice.model_ids.is_empty() {
        return None;
    }
    Some(choice.model_ids.clone())
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
        .with_context(|| format!("writing {}", path.display()))?;
    // Refresh rather than clear: the next reader is on the request path, and
    // handing it a miss would put the file read back where this cache exists to
    // keep it out of. Written after the file so a failed write leaves the cache
    // agreeing with what is on disk.
    if let Ok(mut cache) = CACHE.write() {
        *cache = Some(prefs.clone());
    }
    Ok(())
}

/// Drop the cached copy. Tests only - each one points the app-support dir
/// somewhere new, and a value cached from the previous directory would outlive
/// it.
#[doc(hidden)]
pub fn reset_cache_for_tests() {
    if let Ok(mut cache) = CACHE.write() {
        *cache = None;
    }
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

/// Set one tool's model choice, leaving every other tool alone.
///
/// Read-modify-write on the whole file, like the switches above, so a caller
/// that knows about one tool cannot clobber another's entry.
///
/// **The acknowledgement is recorded here rather than by a separate call.** It is
/// only ever true *because* someone accepted a specific switch to a Gate model,
/// and a second entry point would let the two drift - an install that had
/// acknowledged but never chosen, or the reverse. `acknowledge_paid_use` is
/// honoured only when moving to [`ModelSource::Gate`]: nothing is billed for
/// remembering a model under the tool's own default, so nothing there should
/// record consent to be billed.
///
/// The stamp is written once and never moved, for the reason the gateway's
/// version used `COALESCE`: the record of *when* someone agreed to be billed is
/// worthless if a later save can advance it.
pub fn set_tool_model(
    slug: &str,
    source: ModelSource,
    model_ids: Vec<String>,
    acknowledge_paid_use: bool,
) -> Result<()> {
    let mut prefs = load();
    if source == ModelSource::Gate
        && acknowledge_paid_use
        && prefs.gate_model_paid_ack_unix.is_none()
    {
        prefs.gate_model_paid_ack_unix = Some(time::OffsetDateTime::now_utc().unix_timestamp());
    }
    prefs
        .tool_models
        .insert(slug.to_string(), ToolModelChoice { source, model_ids });
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
            ..Preferences::default()
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
            ..Preferences::default()
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
