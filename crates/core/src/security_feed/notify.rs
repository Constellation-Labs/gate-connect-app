//! Whether a security event becomes a desktop notification, and what it says.
//!
//! Kept in `core` rather than beside the `tauri` call that fires it, because the
//! interesting part is a policy with edges worth testing and none of it needs a
//! window.
//!
//! **Why grouping is required rather than polite.** A single leaked credential in
//! a file an agent is iterating over produces one blocked request per turn, and an
//! agent takes turns quickly. Ungrouped, that is a notification per turn: the OS
//! stacks them, the user dismisses the stack, and the one notification that
//! mattered goes with it. AC5 asks for grouping rules for this reason, so the
//! rule here is the conservative one - **the first event in a bucket speaks, the
//! rest only raise its count.**

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::{Action, SecurityEvent};
use crate::preferences;

/// How long events with the same cause collapse into one notification.
///
/// A minute, because that is roughly the span over which a person reads a
/// notification as being about "what is happening now" rather than about a
/// separate incident. Short enough that a genuinely new problem still speaks.
pub const GROUP_WINDOW: Duration = Duration::from_secs(60);

/// What the caller should do about one event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notification {
    /// Say nothing: the switch is off, or this is a repeat inside the window.
    Suppress,
    /// Fire a notification with this title and body.
    Fire {
        title: String,
        body: String,
        /// Whether the preference asks for a sound.
        sound: bool,
    },
}

/// One bucket's state: when it last spoke, how many events it has swallowed, and
/// what to call them if it has to summarise.
///
/// The display strings are held here rather than read back off the map key,
/// which flattens an absent category or tool to an empty string. A summary has to
/// say "a request from codex" where the key says `("", "codex")`, and
/// reconstructing that from a sentinel is how the two drift apart.
#[derive(Debug)]
struct Bucket {
    opened: Instant,
    suppressed: u32,
    category: Option<String>,
    tool: Option<String>,
}

/// Collapses events into notifications.
///
/// Buckets by `(action, category, tool)`. Not by action alone: a blocked
/// credential and a blocked injection are different problems and collapsing them
/// would report the wrong one. Not by request either, which would not group at
/// all - that is the failure mode this exists to prevent.
#[derive(Debug, Default)]
pub struct Grouper {
    buckets: HashMap<(Action, String, String), Bucket>,
}

impl Grouper {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decide what to do about `event`.
    ///
    /// Takes `prefs` rather than reading them, and `now` rather than asking the
    /// clock, so the policy is a pure function of its inputs. Both were loads
    /// from process-global state in an earlier draft, which made every test here
    /// depend on the developer's own preferences file.
    ///
    /// Returns **zero, one or two** notifications, because expiring a bucket can
    /// itself produce one: any window that closed since the last call is
    /// summarised first, then this event either speaks or is swallowed. Sweeping
    /// here rather than leaving it to the caller means there is exactly one place
    /// a bucket can expire, so a count cannot be dropped by whichever path got
    /// there first.
    pub fn admit(
        &mut self,
        event: &SecurityEvent,
        prefs: &preferences::Preferences,
        now: Instant,
    ) -> Vec<Notification> {
        let mut out = self.sweep(prefs, now);

        let wanted = match event.action {
            Action::Block => prefs.blocked_event_notifications,
            Action::Flag => prefs.flagged_event_notifications,
        };
        if !wanted {
            // A switch the user turned off has to actually stop something, or it
            // was never a switch.
            return out;
        }

        let key = (
            event.action,
            event.category.clone().unwrap_or_default(),
            event.tool.clone().unwrap_or_default(),
        );

        if let Some(bucket) = self.buckets.get_mut(&key) {
            // `sweep` has already retired anything past its window, so a bucket
            // still here is inside one.
            bucket.suppressed = bucket.suppressed.saturating_add(1);
            return out;
        }

        self.buckets.insert(
            key,
            Bucket {
                opened: now,
                suppressed: 0,
                category: event.category.clone(),
                tool: event.tool.clone(),
            },
        );

        out.push(Notification::Fire {
            title: match event.action {
                Action::Block => "Request blocked".to_string(),
                Action::Flag => "Request flagged".to_string(),
            },
            body: body_for(event),
            sound: prefs.security_notification_sound,
        });
        out
    }

    /// Retire every window that has closed, summarising the ones that swallowed
    /// something.
    ///
    /// **This is why the count exists.** Grouping without it means thirty blocked
    /// requests in a minute produce one notification that says a request was
    /// blocked - true, and a wild understatement of what is happening. The
    /// trailing summary is what makes the volume honest, and it arrives after the
    /// storm rather than before it because the first notification's job is to be
    /// immediate and this one's job is to be complete.
    ///
    /// **Must be called on a timer, not only from [`admit`].** A storm ends by
    /// events stopping, so the moment worth reporting is precisely the moment
    /// nothing arrives to drive an `admit`.
    ///
    /// Re-reads the switches rather than trusting the ones in force when the
    /// bucket opened: a user who turns blocked notifications off mid-storm has
    /// asked for silence, and a summary landing afterwards would be the switch
    /// failing to stop something.
    pub fn sweep(&mut self, prefs: &preferences::Preferences, now: Instant) -> Vec<Notification> {
        let mut due = Vec::new();
        let mut expired = Vec::new();
        for (key, bucket) in &self.buckets {
            if now.duration_since(bucket.opened) >= GROUP_WINDOW {
                expired.push(key.clone());
            }
        }
        for key in expired {
            let Some(bucket) = self.buckets.remove(&key) else {
                continue;
            };
            if bucket.suppressed == 0 {
                // The window closed having swallowed nothing. The one
                // notification it fired already told the whole story.
                continue;
            }
            let wanted = match key.0 {
                Action::Block => prefs.blocked_event_notifications,
                Action::Flag => prefs.flagged_event_notifications,
            };
            if !wanted {
                continue;
            }
            due.push(Notification::Fire {
                title: match key.0 {
                    Action::Block => "More requests blocked".to_string(),
                    Action::Flag => "More requests flagged".to_string(),
                },
                body: summary_body(
                    key.0,
                    bucket.category.as_deref(),
                    bucket.tool.as_deref(),
                    bucket.suppressed,
                ),
                sound: prefs.security_notification_sound,
            });
        }
        due
    }

    /// How many events a bucket has swallowed since it spoke. For a caller that
    /// wants to update a notification in place; nothing does yet.
    pub fn suppressed(&self, event: &SecurityEvent) -> u32 {
        let key = (
            event.action,
            event.category.clone().unwrap_or_default(),
            event.tool.clone().unwrap_or_default(),
        );
        self.buckets.get(&key).map(|b| b.suppressed).unwrap_or(0)
    }
}

/// The one sentence a notification gets.
///
/// Names the category and the tool when the gateway attributed them, and says
/// less when it did not rather than inventing either. **No content**: this string
/// is rendered by the OS, may sit on a lock screen, and AC3's list of things that
/// must never appear applies with more force here than anywhere in the window.
fn body_for(event: &SecurityEvent) -> String {
    let verb = match event.action {
        Action::Block => "blocked",
        Action::Flag => "flagged",
    };
    match (event.category.as_deref(), event.tool.as_deref()) {
        (Some(cat), Some(tool)) => format!("Gate {verb} a {cat} match in {tool}."),
        (Some(cat), None) => format!("Gate {verb} a {cat} match."),
        (None, Some(tool)) => format!("Gate {verb} a request from {tool}."),
        (None, None) => format!("Gate {verb} a request."),
    }
}

/// What a trailing summary says.
///
/// Counts only what was *swallowed*, not the whole storm: the first notification
/// already reported one, so "29 more" plus that one is the thirty that happened.
/// Saying "30 more" would double-count the one the user already saw and read as
/// thirty-one.
///
/// Same content rule as [`body_for`], and it matters as much here: this string is
/// rendered by the OS and may sit on a lock screen.
fn summary_body(action: Action, category: Option<&str>, tool: Option<&str>, count: u32) -> String {
    let verb = match action {
        Action::Block => "blocked",
        Action::Flag => "flagged",
    };
    // "1 more match" reads wrong beside "2 more matches", and a storm of exactly
    // one is common enough to be worth the branch.
    let plural = if count == 1 { "" } else { "es" };
    match (category, tool) {
        (Some(cat), Some(tool)) => {
            format!("Gate {verb} {count} more {cat} match{plural} in {tool}.")
        }
        (Some(cat), None) => format!("Gate {verb} {count} more {cat} match{plural}."),
        (None, Some(tool)) => {
            let plural = if count == 1 { "" } else { "s" };
            format!("Gate {verb} {count} more request{plural} from {tool}.")
        }
        (None, None) => {
            let plural = if count == 1 { "" } else { "s" };
            format!("Gate {verb} {count} more request{plural}.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(action: Action, category: Option<&str>, tool: Option<&str>) -> SecurityEvent {
        SecurityEvent {
            id: "01A".into(),
            // Realistic, and deliberately not a substring of any word the body
            // can legitimately contain: a one-character id made the leak guard
            // below pass on "credential".
            request_id: "req-8f3c1e2a-4b7d".into(),
            at: "t".into(),
            action,
            category: category.map(str::to_string),
            tool: tool.map(str::to_string),
            model: None,
            provider: None,
        }
    }

    #[test]
    fn the_body_names_what_it_knows_and_no_more() {
        assert_eq!(
            body_for(&event(
                Action::Block,
                Some("credential"),
                Some("claude-code")
            )),
            "Gate blocked a credential match in claude-code."
        );
        assert_eq!(
            body_for(&event(Action::Flag, Some("pii"), None)),
            "Gate flagged a pii match."
        );
        // Unattributed on both axes still produces a true sentence rather than a
        // guess or an empty one.
        assert_eq!(
            body_for(&event(Action::Block, None, None)),
            "Gate blocked a request."
        );
    }

    /// The single notification `admit` produced, or None. Panics if it produced
    /// two, which in these tests always means a sweep fired unexpectedly.
    fn one(out: Vec<Notification>) -> Option<Notification> {
        assert!(
            out.len() <= 1,
            "expected at most one notification, got {out:?}"
        );
        out.into_iter().next()
    }

    fn fired(out: Vec<Notification>) -> bool {
        matches!(one(out), Some(Notification::Fire { .. }))
    }

    fn body_of(n: &Notification) -> &str {
        match n {
            Notification::Fire { body, .. } => body,
            _ => panic!("expected a notification"),
        }
    }

    fn prefs(blocked: bool, flagged: bool, sound: bool) -> preferences::Preferences {
        preferences::Preferences {
            blocked_event_notifications: blocked,
            flagged_event_notifications: flagged,
            security_notification_sound: sound,
            ..Default::default()
        }
    }

    #[test]
    fn the_first_event_in_a_bucket_speaks() {
        let mut g = Grouper::new();
        let e = event(Action::Block, Some("credential"), Some("codex"));
        assert!(fired(g.admit(&e, &prefs(true, true, true), Instant::now())));
    }

    #[test]
    fn a_repeat_inside_the_window_is_swallowed_and_counted() {
        // The case this exists for: one leaked credential in a file an agent is
        // iterating over is one blocked request per turn.
        let mut g = Grouper::new();
        let e = event(Action::Block, Some("credential"), Some("codex"));
        let now = Instant::now();
        let p = prefs(true, true, true);
        assert!(fired(g.admit(&e, &p, now)));
        for i in 1..=5 {
            assert!(
                g.admit(&e, &p, now).is_empty(),
                "repeat {i} should be swallowed"
            );
        }
        assert_eq!(g.suppressed(&e), 5);
    }

    #[test]
    fn a_different_cause_is_a_different_bucket_and_speaks() {
        // A blocked credential and a blocked injection are different problems;
        // collapsing them would report the wrong one.
        let mut g = Grouper::new();
        let p = prefs(true, true, true);
        let now = Instant::now();
        let cred = event(Action::Block, Some("credential"), Some("codex"));
        let inj = event(Action::Block, Some("injection"), Some("codex"));
        let other_tool = event(Action::Block, Some("credential"), Some("claude-code"));
        assert!(fired(g.admit(&cred, &p, now)));
        assert!(fired(g.admit(&inj, &p, now)));
        assert!(fired(g.admit(&other_tool, &p, now)));
    }

    #[test]
    fn the_bucket_reopens_once_the_window_has_passed() {
        let mut g = Grouper::new();
        let p = prefs(true, true, true);
        let e = event(Action::Block, Some("credential"), Some("codex"));
        let start = Instant::now();
        assert!(fired(g.admit(&e, &p, start)));
        assert!(g.admit(&e, &p, start + GROUP_WINDOW / 2).is_empty());
        // A genuinely new incident still speaks. Two notifications now: the
        // trailing summary for the closed window, then this event opening a new
        // one.
        let out = g.admit(&e, &p, start + GROUP_WINDOW + Duration::from_secs(1));
        assert_eq!(out.len(), 2, "summary then the new incident, got {out:?}");
        assert!(body_of(&out[0]).contains("1 more"));
    }

    #[test]
    fn a_switch_that_is_off_stops_something() {
        let mut g = Grouper::new();
        let now = Instant::now();
        let blocked = event(Action::Block, Some("credential"), Some("codex"));
        let flagged = event(Action::Flag, Some("pii"), Some("codex"));
        // Blocked off, flagged on: only the flag speaks.
        let p = prefs(false, true, true);
        assert!(g.admit(&blocked, &p, now).is_empty());
        assert!(fired(g.admit(&flagged, &p, now)));
        // And the reverse.
        let mut g = Grouper::new();
        let p = prefs(true, false, true);
        assert!(fired(g.admit(&blocked, &p, now)));
        assert!(g.admit(&flagged, &p, now).is_empty());
    }

    #[test]
    fn the_sound_preference_rides_on_the_notification() {
        let mut g = Grouper::new();
        let e = event(Action::Block, Some("credential"), Some("codex"));
        match one(g.admit(&e, &prefs(true, true, false), Instant::now())) {
            Some(Notification::Fire { sound, .. }) => assert!(!sound),
            other => panic!("expected a notification, got {other:?}"),
        }
    }

    #[test]
    fn a_suppressed_event_does_not_reset_the_window() {
        // Otherwise a steady stream of repeats would hold the bucket open
        // forever and the next real incident would never speak.
        let mut g = Grouper::new();
        let p = prefs(true, true, true);
        let e = event(Action::Block, Some("credential"), Some("codex"));
        let start = Instant::now();
        g.admit(&e, &p, start);
        for i in 1..30 {
            g.admit(&e, &p, start + Duration::from_secs(i));
        }
        let out = g.admit(&e, &p, start + GROUP_WINDOW + Duration::from_secs(1));
        assert_eq!(out.len(), 2);
        assert!(body_of(&out[0]).contains("29 more"));
    }

    #[test]
    fn a_closed_window_summarises_what_it_swallowed() {
        // The whole point of counting. Thirty blocks in a minute must not read as
        // one incident once the dust settles.
        let mut g = Grouper::new();
        let p = prefs(true, true, true);
        let e = event(Action::Block, Some("credential"), Some("codex"));
        let start = Instant::now();
        g.admit(&e, &p, start);
        for i in 1..=29 {
            g.admit(&e, &p, start + Duration::from_secs(i % 50));
        }

        let due = g.sweep(&p, start + GROUP_WINDOW + Duration::from_secs(1));
        assert_eq!(due.len(), 1);
        // 29, not 30: the first one already spoke, and counting it twice would
        // report thirty-one events from thirty.
        assert_eq!(
            body_of(&due[0]),
            "Gate blocked 29 more credential matches in codex."
        );
    }

    #[test]
    fn a_window_that_swallowed_nothing_stays_quiet() {
        // One blocked request is one notification, not a notification and a
        // summary saying there were no others.
        let mut g = Grouper::new();
        let p = prefs(true, true, true);
        let e = event(Action::Block, Some("credential"), Some("codex"));
        let start = Instant::now();
        g.admit(&e, &p, start);
        assert!(g
            .sweep(&p, start + GROUP_WINDOW + Duration::from_secs(1))
            .is_empty());
    }

    #[test]
    fn sweeping_early_retires_nothing() {
        let mut g = Grouper::new();
        let p = prefs(true, true, true);
        let e = event(Action::Block, Some("credential"), Some("codex"));
        let start = Instant::now();
        g.admit(&e, &p, start);
        g.admit(&e, &p, start);
        assert!(g.sweep(&p, start + GROUP_WINDOW / 2).is_empty());
        // And the bucket is still holding its count rather than having been
        // dropped by the early sweep.
        assert_eq!(g.suppressed(&e), 1);
    }

    #[test]
    fn a_summary_is_delivered_once() {
        let mut g = Grouper::new();
        let p = prefs(true, true, true);
        let e = event(Action::Block, Some("credential"), Some("codex"));
        let start = Instant::now();
        g.admit(&e, &p, start);
        g.admit(&e, &p, start);
        let after = start + GROUP_WINDOW + Duration::from_secs(1);
        assert_eq!(g.sweep(&p, after).len(), 1);
        // The timer sweeps every few seconds; a second pass must not re-announce
        // a storm the user has already been told about.
        assert!(g.sweep(&p, after).is_empty());
    }

    #[test]
    fn a_switch_turned_off_mid_storm_silences_the_summary() {
        // The switch has to stop something, and a summary landing after the user
        // asked for quiet is the switch failing to.
        let mut g = Grouper::new();
        let on = prefs(true, true, true);
        let e = event(Action::Block, Some("credential"), Some("codex"));
        let start = Instant::now();
        g.admit(&e, &on, start);
        g.admit(&e, &on, start);

        let off = prefs(false, true, true);
        assert!(g
            .sweep(&off, start + GROUP_WINDOW + Duration::from_secs(1))
            .is_empty());
    }

    #[test]
    fn the_summary_counts_each_cause_separately() {
        let mut g = Grouper::new();
        let p = prefs(true, true, true);
        let start = Instant::now();
        let cred = event(Action::Block, Some("credential"), Some("codex"));
        let pii = event(Action::Flag, Some("pii"), Some("codex"));
        for _ in 0..4 {
            g.admit(&cred, &p, start);
        }
        for _ in 0..3 {
            g.admit(&pii, &p, start);
        }

        let due = g.sweep(&p, start + GROUP_WINDOW + Duration::from_secs(1));
        let bodies: Vec<&str> = due.iter().map(body_of).collect();
        assert_eq!(due.len(), 2, "one per cause, got {bodies:?}");
        assert!(bodies
            .iter()
            .any(|b| *b == "Gate blocked 3 more credential matches in codex."));
        assert!(bodies
            .iter()
            .any(|b| *b == "Gate flagged 2 more pii matches in codex."));
    }

    #[test]
    fn the_summary_reads_correctly_for_one_and_for_the_unattributed() {
        assert_eq!(
            summary_body(Action::Block, Some("credential"), Some("codex"), 1),
            "Gate blocked 1 more credential match in codex."
        );
        assert_eq!(
            summary_body(Action::Flag, Some("pii"), None, 2),
            "Gate flagged 2 more pii matches."
        );
        // Unattributed on both axes: "requests", never "matches", because there
        // is no category to have matched.
        assert_eq!(
            summary_body(Action::Block, None, None, 1),
            "Gate blocked 1 more request."
        );
        assert_eq!(
            summary_body(Action::Block, None, Some("codex"), 4),
            "Gate blocked 4 more requests from codex."
        );
    }

    #[test]
    fn a_summary_never_carries_content_either() {
        // Same guard as `body_for`: this string reaches a lock screen too.
        let body = summary_body(Action::Block, Some("credential"), Some("codex"), 12);
        for forbidden in ["prompt", "response", "sk-", "secret", "evidence"] {
            assert!(
                !body.to_lowercase().contains(forbidden),
                "leaked {forbidden}: {body}"
            );
        }
    }

    #[test]
    fn the_body_never_carries_content() {
        // A guard against the field set widening upstream without anyone noticing
        // that this string ends up on a lock screen.
        let e = event(Action::Block, Some("credential"), Some("codex"));
        let body = body_for(&e);
        assert!(!body.contains(&e.request_id), "no identifiers in the body");
        for forbidden in ["prompt", "response", "sk-", "secret", "evidence"] {
            assert!(
                !body.to_lowercase().contains(forbidden),
                "body must not mention {forbidden}: {body}"
            );
        }
    }
}
