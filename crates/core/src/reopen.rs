//! Is a running tool still on the route it started with, and what can Gate
//! honestly say about that route?
//!
//! A tool reads its configuration once, when it starts. Change that
//! configuration under a running process and the process keeps whatever it
//! loaded, so it needs to be opened again before the change takes effect. That
//! is the whole of `reopen_pending`, and it turns on one comparison: is the
//! configuration this process loaded at its start the one in place now?
//!
//! **The bound has to outlive Gate.** The first version of this compared a
//! process's start time against the moment routing came up *in the current Gate
//! process*, falling back to Gate's own start time when nothing had come up
//! yet. Both of those reset on every launch, so after any restart of Gate the
//! bound was roughly "now" and every tool process that was already running was
//! classified as stale. The user could only satisfy it by closing the tool, and
//! the next restart of Gate re-armed it against whatever they opened: a
//! permanent, self-renewing "Reopen to finish" warning, reported from the field
//! against a machine whose Claude Code config had carried no Gate values for a
//! week.
//!
//! So the bound is the tool's own configuration file: Gate's history of changes
//! to it, from [`crate::config_changes`]. It is durable (it is on disk,
//! across restarts of Gate, reboots and reinstalls) and it is per tool (a
//! change to Codex does not make Claude Code stale).
//!
//! **Gate's changes, not anybody's.** This took the file's mtime until the
//! tool's own edits were found to move it: Claude Code rewrites
//! `settings.json` when the user approves a permission, and each approval made
//! every older `claude` stale with nothing Gate routes by changed. A hand edit
//! to a routing value is seen only when Gate next writes the file, which
//! records what it found there; until then the config status reports it.
//!
//! **Changed and changed back is not a change.** Every quit takes Gate out of
//! every tool's configuration and the next launch puts the same values back,
//! and a single "last changed" time moved on both: every session that outlived
//! a relaunch of Gate read as stale while it held exactly what the file says
//! again, and its row read Not protected over a tool that was routing. So the
//! record is a short history of fingerprinted changes, and the question is
//! whether the configuration a process loaded - the last change at or before
//! its start - is the one in place now.
//!
//! **No bound means no claim.** When there is no recorded change to read a
//! time from, this answers "not pending" rather than degrading to "everything
//! running is stale". Nothing on disk supports the claim that a running process
//! missed a change, and the old conservative direction is exactly what produced
//! a warning nobody could clear. Design principle 6 cuts the same way: a
//! statement on screen is a measurement, or the surface does not make it.
//!
//! Everything here is pure. The process table walk, the history read and the
//! registry lookup live in the Tauri shell; this module takes what they produce.

use crate::config_changes::Change;
use crate::routing_health::ConfigState;

/// Everything the reopen decision is made from, for one tool.
#[derive(Debug, Clone, Copy)]
pub struct ReopenEvidence<'a> {
    /// Does Gate know what this tool's process is called?
    ///
    /// False for the tools that ship no fixed process name (OpenClaw, Hermes)
    /// and for the environment channel, which is not a process at all. For
    /// those, staleness is *unobservable* rather than false, and the decision
    /// below says so by declining to raise it: reporting them unverifiable
    /// forever would bury the route probe under a permanent warning.
    pub process_names_known: bool,
    /// Unix seconds at which each running process of this tool started. Empty
    /// when nothing is running, which is the ordinary case and the one where
    /// there is nothing to reopen.
    pub process_starts: &'a [u64],
    /// Gate's changes to this tool's configuration file, oldest first, from
    /// [`crate::config_changes`]. Empty when the tool has no configuration
    /// file, or Gate has no recorded change to it.
    pub config_changes: &'a [Change],
    /// Unix seconds at which Gate's CA certificate was last written.
    ///
    /// **The second thing a tool reads once at startup**, and the one this
    /// module was blind to. The bound above is the tool's own config file, so
    /// a re-mint moved nothing `reopen_pending` looked at, and a process
    /// holding the old certificate went on failing every intercepted host
    /// while its row read Protected. AG-933, observed three times in one
    /// afternoon.
    ///
    /// **`ca-cert.pem`, not `ca-bundle.pem`**, and the distinction is the
    /// whole correctness of this field. The tools with process names - the
    /// CLIs and the desktop apps - are pointed at the certificate through
    /// `NODE_EXTRA_CA_CERTS`, which appends to their built-in roots. The
    /// bundle is a *synthesized* file for tools that take a single CA file
    /// and replace their trust store with it, it has exactly one consumer
    /// (Hermes), and `ca_bundle` regenerates it on every connect. Reading the
    /// bundle here would have missed the re-mint - which rewrites the
    /// certificate and leaves the bundle until Hermes next connects - and
    /// raised the alert on every Hermes connect for tools that never read it.
    /// Caught in review on #329.
    ///
    /// Not per tool, because the certificate is not: every tool Gate points
    /// at it reads the same file. `None` when there is none on disk, which
    /// reads as no claim the same way a missing config does.
    pub ca_cert_changed_at: Option<u64>,
}

/// Is a process for this tool running that loaded a configuration other than
/// the one in place now, or predates Gate's CA certificate, and is therefore
/// still using whatever it loaded then?
///
/// Strictly *before*: a process that started in the same second as a change is
/// taken to have loaded it. Second granularity is all a process start time
/// carries on the platforms this runs on, and the tie is far more likely to be
/// a tool that was reopened promptly than one that missed the change.
pub fn reopen_pending(ev: &ReopenEvidence) -> bool {
    if !ev.process_names_known {
        return false;
    }
    ev.process_starts.iter().any(|&start| {
        missed_config_change(ev.config_changes, start)
            || ev.ca_cert_changed_at.is_some_and(|ca| start < ca)
    })
}

/// Did a process started at `start` load a configuration other than the one in
/// place now?
///
/// What it loaded is the last change at or before its start. A process older
/// than every change kept is stale, which is what a single stamp said about it
/// too; with no change recorded at all there is nothing to support the claim.
/// A view recorded before fingerprints existed matches nothing, so a record
/// carried over from an older build decides exactly as it used to.
fn missed_config_change(changes: &[Change], start: u64) -> bool {
    let Some(current) = changes.last() else {
        // Nothing on disk records a change, so nothing supports the claim that
        // a running process missed one. See the module docs: the old fallback
        // here was Gate's own process start, which is the defect this replaces.
        return false;
    };
    changes
        .iter()
        .rposition(|change| change.at <= start)
        .is_none_or(|loaded| {
            // The newest change is the current one, so a process that loaded it
            // is current - even when its view is `None`, a legacy stamp the
            // process started after, which is what the single stamp said too.
            loaded + 1 != changes.len()
                && (changes[loaded].view.is_none() || changes[loaded].view != current.view)
        })
}

/// The two endpoints a reopen notice can name: where the tool's traffic is
/// going now, and where its saved configuration asks it to go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReopenRoutes {
    /// Where the running process is actually pointed.
    ///
    /// Always `None` today, and deliberately. Gate cannot read another
    /// process's environment, and it does not know what the tool's
    /// configuration said at the moment that process read it, so there is no
    /// reading to report. This used to be *derived* from the current config
    /// state - an absent config was reported as "the process is on the
    /// gateway", a managed one as "the process is on its own upstream" - which
    /// is a claim about the user's traffic with no measurement behind it, and
    /// it was wrong on the machine that reported the bug: no
    /// `ANTHROPIC_BASE_URL`, no `HTTPS_PROXY`, and no Gate values in the config
    /// for a week, under a banner saying the tool was still on the gateway.
    ///
    /// Kept as a field rather than deleted so that a future change that can
    /// genuinely observe the route (per-tool attribution in the relay, or a
    /// recorded reading of the config the process actually loaded) has
    /// somewhere to put it, and so the surfaces keep their "omit the pair"
    /// path. A guess must never be put here.
    pub in_use: Option<String>,
    /// Where the configuration on disk points the tool, which is what it will
    /// use once it is opened again. This one *is* read from the file, so it is
    /// reported.
    pub requested: Option<String>,
}

/// What a reopen notice may say about routes, for a tool whose verdict is
/// [`crate::routing_health::Reason::ReopenRequired`].
///
/// `gate_route` is the gateway this install talks to; `own_upstream` is the
/// tool's own provider endpoint. Which of the two is *requested* comes from the
/// config state, which is a reading of the file: managed means Gate's values
/// are in it, so the tool is being asked to go through the gateway; anything
/// else means they are not, so it is being asked to go direct.
pub fn reopen_routes(config: ConfigState, gate_route: &str, own_upstream: &str) -> ReopenRoutes {
    let requested = match config {
        ConfigState::Managed => gate_route,
        _ => own_upstream,
    };
    ReopenRoutes {
        in_use: None,
        requested: Some(requested.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reporter's machine, in Unix seconds: a Claude Code config last
    /// written a week ago, one process older than that write and one newer.
    const CONFIG_WRITE: u64 = 1_756_998_780; // Sep 4
    const BEFORE_CONFIG: u64 = 1_756_702_980; // Sep 1
    const AFTER_CONFIG: u64 = 1_757_433_720; // Sep 9

    /// A single change with no view: what one stamp used to record. Leaked so
    /// the existing `evidence` helpers can keep returning evidence that borrows
    /// only the starts; a test process frees it on exit.
    fn stamp(at: Option<u64>) -> &'static [Change] {
        Box::leak(at.map(|at| Change { at, view: None }).into_iter().collect())
    }

    /// The config-only case, which is what every test before AG-933 was about.
    fn evidence(starts: &[u64], config_changed_at: Option<u64>) -> ReopenEvidence<'_> {
        ReopenEvidence {
            process_names_known: true,
            process_starts: starts,
            config_changes: stamp(config_changed_at),
            ca_cert_changed_at: None,
        }
    }

    /// ...and the same with a certificate bundle in the picture.
    fn evidence_with_cert(
        starts: &[u64],
        config_changed_at: Option<u64>,
        ca_cert_changed_at: Option<u64>,
    ) -> ReopenEvidence<'_> {
        ReopenEvidence {
            process_names_known: true,
            process_starts: starts,
            config_changes: stamp(config_changed_at),
            ca_cert_changed_at,
        }
    }

    #[test]
    fn a_process_older_than_the_last_config_change_is_pending() {
        assert!(reopen_pending(&evidence(
            &[BEFORE_CONFIG],
            Some(CONFIG_WRITE)
        )));
    }

    #[test]
    fn a_process_started_since_the_last_config_change_is_not_pending() {
        assert!(!reopen_pending(&evidence(
            &[AFTER_CONFIG],
            Some(CONFIG_WRITE)
        )));
    }

    #[test]
    fn a_process_started_in_the_same_second_as_the_change_is_not_pending() {
        assert!(!reopen_pending(&evidence(
            &[CONFIG_WRITE],
            Some(CONFIG_WRITE)
        )));
    }

    #[test]
    fn one_stale_process_among_fresh_ones_is_enough() {
        assert!(reopen_pending(&evidence(
            &[AFTER_CONFIG, BEFORE_CONFIG, AFTER_CONFIG + 10],
            Some(CONFIG_WRITE),
        )));
    }

    #[test]
    fn nothing_running_is_never_pending() {
        assert!(!reopen_pending(&evidence(&[], Some(CONFIG_WRITE))));
    }

    /// AG-933. A certificate bundle written after the process started makes
    /// that process stale, exactly as a config write does.
    ///
    /// The bundle is the second thing a tool reads once at startup, and this
    /// module was blind to it: the bound was the tool's own config file, so a
    /// re-mint moved nothing `reopen_pending` looked at. A process holding the
    /// old bundle failed every intercepted host while its row read Protected -
    /// observed three times in one afternoon, by three different routes.
    #[test]
    fn a_process_older_than_the_ca_certificate_is_pending() {
        // Config untouched since before the process started; only the bundle
        // moved. Before AG-933 this was `false`.
        assert!(reopen_pending(&evidence_with_cert(
            &[BEFORE_CONFIG],
            None,
            Some(CONFIG_WRITE)
        )));
    }

    /// Either bound is enough, so the later one decides.
    #[test]
    fn the_later_of_the_two_bounds_is_what_counts() {
        // Process started after the config was written but before the bundle:
        // still stale, because the bundle is the newer thing it reads.
        assert!(reopen_pending(&evidence_with_cert(
            &[CONFIG_WRITE + 1],
            Some(CONFIG_WRITE),
            Some(CONFIG_WRITE + 2)
        )));
        // And the mirror: newer than both, so nothing is stale. This is the
        // case a naive "either is set" test would get wrong.
        assert!(!reopen_pending(&evidence_with_cert(
            &[CONFIG_WRITE + 3],
            Some(CONFIG_WRITE),
            Some(CONFIG_WRITE + 2)
        )));
    }

    /// A bundle with no time still makes no claim, like a config with none.
    #[test]
    fn a_missing_certificate_is_not_an_argument_that_anything_is_stale() {
        // Neither bound readable: no claim, which is the rule the module docs
        // set and the reason the old "fall back to Gate's own start" was
        // removed.
        assert!(!reopen_pending(&evidence_with_cert(
            &[BEFORE_CONFIG],
            None,
            None
        )));
    }

    #[test]
    fn a_tool_with_no_known_process_name_is_never_pending() {
        // OpenClaw and Hermes: staleness is unobservable, and the notice is
        // declined rather than raised forever.
        let ev = ReopenEvidence {
            process_names_known: false,
            process_starts: &[BEFORE_CONFIG],
            config_changes: stamp(Some(CONFIG_WRITE)),
            ca_cert_changed_at: Some(CONFIG_WRITE),
        };
        assert!(!reopen_pending(&ev));
    }

    #[test]
    fn no_config_file_means_no_claim() {
        // A running process and no readable configuration time. Nothing
        // records a change, so nothing is asserted about the process - this is
        // the branch that used to fall back to Gate's own start time.
        assert!(!reopen_pending(&evidence(&[BEFORE_CONFIG], None)));
    }

    /// The regression. Restarting Gate Connect must not re-arm the notice for a
    /// tool whose configuration has not changed since that tool started.
    ///
    /// The reporter's timeline: `claude` started Sep 9, its `settings.json` was
    /// last written Sep 4, and Gate Connect started Sep 11 - after both.
    /// Against the old bound (the routing-enabled timestamp of the current Gate
    /// process, falling back to Gate's own start) this read as stale on every
    /// launch, forever. Nothing about when Gate started is an input here, and
    /// putting it back as one is what this test fails on.
    #[test]
    fn gate_restart_does_not_re_arm_the_notice() {
        let ev = evidence(&[AFTER_CONFIG], Some(CONFIG_WRITE));
        assert!(
            !reopen_pending(&ev),
            "a process newer than the last config change is not stale, \
             however recently Gate Connect itself started"
        );
    }

    /// The older of the reporter's two processes does still predate the config
    /// write, and is still reported - but closing it settles the matter, and a
    /// restart of Gate does not bring the notice back.
    #[test]
    fn a_process_older_than_both_stays_pending_until_it_is_closed() {
        assert!(reopen_pending(&evidence(
            &[BEFORE_CONFIG],
            Some(CONFIG_WRITE)
        )));
        assert!(!reopen_pending(&evidence(&[], Some(CONFIG_WRITE))));
    }

    /// Whether a process started at each of `starts` is pending, over changes
    /// at the given times, each with its own view.
    fn pending_over(starts: &[u64], list: &[(u64, &str)]) -> bool {
        let changes: Vec<Change> = list
            .iter()
            .map(|(at, view)| Change {
                at: *at,
                view: Some(view.to_string()),
            })
            .collect();
        reopen_pending(&ReopenEvidence {
            process_names_known: true,
            process_starts: starts,
            config_changes: &changes,
            ca_cert_changed_at: None,
        })
    }

    /// The report this history exists for: a session connected before Gate was
    /// quit, still running when it is opened again. The quit took Gate's values
    /// out and the launch put the same ones back, so what the session loaded is
    /// what the file says, and it is routing.
    #[test]
    fn a_quit_and_relaunch_does_not_strand_a_session_that_outlived_it() {
        let quit_and_relaunch = [(100, "gate"), (300, "own"), (400, "gate")];
        assert!(!pending_over(&[200], &quit_and_relaunch));
    }

    /// The other half: a session opened while Gate was closed loaded no Gate
    /// values, and the launch that put them back is news to it.
    #[test]
    fn a_session_opened_while_gate_was_closed_is_pending() {
        let quit_and_relaunch = [(100, "gate"), (300, "own"), (400, "gate")];
        assert!(pending_over(&[350], &quit_and_relaunch));
    }

    /// Values that really moved - a different port, a different `NO_PROXY` -
    /// still make an older session stale.
    #[test]
    fn a_view_that_really_changed_is_pending() {
        assert!(pending_over(&[200], &[(100, "gate-a"), (300, "gate-b")]));
    }

    /// Older than every change kept: what it loaded is not known, so it is
    /// stale, as the single stamp said.
    #[test]
    fn a_process_older_than_every_kept_change_is_pending() {
        assert!(pending_over(
            &[50],
            &[(100, "gate"), (300, "own"), (400, "gate")]
        ));
    }

    /// A record from before fingerprints matches nothing, so it decides as the
    /// stamp did rather than reading as unchanged.
    #[test]
    fn a_legacy_record_decides_as_the_stamp_did() {
        let mixed = [
            Change {
                at: 100,
                view: None,
            },
            Change {
                at: 300,
                view: Some("gate".into()),
            },
        ];
        let pending = |start| {
            reopen_pending(&ReopenEvidence {
                process_names_known: true,
                process_starts: &[start],
                config_changes: &mixed,
                ca_cert_changed_at: None,
            })
        };
        assert!(pending(200));
        assert!(!pending(300));
    }

    #[test]
    fn managed_config_requests_the_gateway_and_claims_nothing_in_use() {
        let routes = reopen_routes(
            ConfigState::Managed,
            "https://gateway-staging.constellationgate.ai",
            "https://api.anthropic.com",
        );
        assert_eq!(
            routes.requested.as_deref(),
            Some("https://gateway-staging.constellationgate.ai")
        );
        assert_eq!(routes.in_use, None, "nothing measured the running process");
    }

    #[test]
    fn absent_config_requests_the_tools_own_upstream_and_claims_nothing_in_use() {
        let routes = reopen_routes(
            ConfigState::Absent,
            "https://gateway-staging.constellationgate.ai",
            "https://api.anthropic.com",
        );
        assert_eq!(
            routes.requested.as_deref(),
            Some("https://api.anthropic.com")
        );
        assert_eq!(
            routes.in_use, None,
            "an absent config is not evidence that the process is on the gateway"
        );
    }

    /// Every config state, so a future edit cannot reintroduce a route in use
    /// for one of them without this failing.
    #[test]
    fn no_config_state_ever_names_a_route_in_use() {
        for config in [
            ConfigState::Managed,
            ConfigState::Drifted,
            ConfigState::Absent,
            ConfigState::Overridden,
            ConfigState::Unreadable,
        ] {
            let routes = reopen_routes(config, "https://gate.example", "https://own.example");
            assert_eq!(routes.in_use, None, "{config:?} claimed a route in use");
            assert!(
                routes.requested.is_some(),
                "{config:?} should still name where the file points"
            );
        }
    }
}
