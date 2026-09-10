//! What a tool is *doing*, as distinct from what its config says and from what
//! the user asked for.
//!
//! [`crate::registry::Status`] answers "what is on disk". It cannot answer "is
//! this tool routing through Gate", because a config file Gate wrote is not
//! evidence that anything is using it: the relay may be down, the session may
//! have been refused server-side, or the tool process may predate the write and
//! still hold its old settings. Every one of those reads as `Connected`.
//!
//! So this module keeps `Status` as an *input* and computes a separate verdict
//! on top of it. The two must not be merged. `groups.ts` documents the bug that
//! follows from conflating observed state with intent - the switch renders off,
//! and clicking it turns off the setting the user was trying to turn on - and
//! the same argument applies one level down: a switch is driven by intent, a
//! status line by this verdict, and neither may be derived from the other.
//!
//! **What this does not prove.** The verdict is composed of local checks plus
//! one reachability probe. It establishes that the route *can* carry traffic and
//! that nothing known is in the way. It does not observe a request from the tool
//! itself, because nothing in the proxy attributes traffic to a tool today. A
//! tool reading `On` has a live, correctly configured, freshly started route; it
//! has not necessarily sent anything through it. Proving that needs per-tool
//! attribution in the relay, which is a larger change than this.

use crate::registry::Status;

/// Config truth, narrowed from [`crate::Integration::status`] so the verdict
/// logic does not have to care about the `Result` or about `Detected` vs
/// `NotInstalled` (installation is its own field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigState {
    /// Gate's own values are in the tool's config.
    Managed,
    /// Routing values are present but are not the ones Gate would write.
    Drifted,
    /// No Gate routing values. The tool is pointed at its own upstream.
    Absent,
    /// Gate's values are on disk and a layer the tool ranks higher decides the
    /// route anyway. Separate from `Drifted` because the repair is different:
    /// nothing touched our file, so writing it again changes nothing.
    Overridden,
    /// The config could not be read or parsed, so nothing about it is known.
    /// Distinct from `Absent`: absence is a verified state, this is ignorance.
    Unreadable,
}

impl ConfigState {
    /// Narrow an integration's status report. `NotInstalled` collapses to
    /// `Absent` because installation is carried separately in
    /// [`Evidence::installed`]; a missing tool has no Gate config either way.
    pub fn from_status(status: &anyhow::Result<Status>) -> Self {
        match status {
            Ok(Status::Connected) => ConfigState::Managed,
            Ok(Status::Drifted(_)) => ConfigState::Drifted,
            Ok(Status::Overridden(_)) => ConfigState::Overridden,
            Ok(Status::Detected) | Ok(Status::NotInstalled) => ConfigState::Absent,
            Err(_) => ConfigState::Unreadable,
        }
    }
}

/// Can the local hop a config-routed tool points at actually carry traffic?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteHealth {
    Reachable,
    /// Nothing is listening, or it refused the connection. No traffic is
    /// reaching Gate through this route right now.
    Unreachable,
    /// Not determined. Never treated as evidence either way, on the same
    /// principle as [`crate::org::SessionProbe::Unavailable`].
    Unknown,
}

/// What the gateway said about the stored session. Mirrors
/// [`crate::org::SessionProbe`] minus its payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionHealth {
    Valid,
    /// A definite refusal (HTTP 401). The session is dead server-side however
    /// fresh it looks locally.
    Rejected,
    /// No verdict: offline, timed out, or an unparseable answer.
    Unknown,
}

/// Everything the verdict is computed from. Passed in rather than gathered here
/// so the decision is a pure function and testable without a live relay, a
/// gateway, or a process table.
#[derive(Debug, Clone, Copy)]
pub struct Evidence {
    pub installed: bool,
    pub config: ConfigState,
    pub route: RouteHealth,
    pub session: SessionHealth,
    /// A tool process is running that started before the last routing change,
    /// so it is still using whatever settings it loaded then.
    pub reopen_pending: bool,
}

/// Why a tool is not verifiably routing. Closed set: these are the six reasons
/// the product vocabulary allows, and a seventh would need a next action and a
/// recovery path to go with it.
///
/// It was five until AG-674. The sixth arrived with its action and its recovery
/// path, which is the price this comment always named: an override is fixed by
/// editing the layer that wins, so the action points at that file and nothing
/// Gate can do unattended is offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Routing values are present but not Gate's.
    ConfigurationChanged,
    /// Gate's values are in place and a higher-precedence configuration layer
    /// decides where the traffic goes.
    ConfigurationOverridden,
    /// The config is right; the running process has not picked it up.
    ReopenRequired,
    /// The local route is not accepting connections.
    ConnectionProblem,
    /// The gateway refused the session.
    AccessProblem,
    /// Nothing is known to be wrong and nothing could be confirmed either.
    VerificationFailed,
}

/// The one action offered for a reason. One-to-one with [`Reason`] on purpose:
/// storing them as independent fields lets a reason and its action drift apart,
/// and the pairing is the part the user acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextAction {
    ApplyGateConfiguration,
    /// Open the layer that wins, so the person can remove the value there.
    /// Deliberately not something Gate does for them: the winning file is
    /// somebody else's - a repo the user shares, or a policy their
    /// administrator set - and editing it unasked is a larger claim on the
    /// machine than this app makes anywhere else.
    ShowConflictingConfig,
    ReopenTool,
    Reconnect,
    SignIn,
    RetryCheck,
}

impl Reason {
    pub const fn next_action(self) -> NextAction {
        match self {
            Reason::ConfigurationChanged => NextAction::ApplyGateConfiguration,
            Reason::ConfigurationOverridden => NextAction::ShowConflictingConfig,
            Reason::ReopenRequired => NextAction::ReopenTool,
            Reason::ConnectionProblem => NextAction::Reconnect,
            Reason::AccessProblem => NextAction::SignIn,
            Reason::VerificationFailed => NextAction::RetryCheck,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Reason::ConfigurationChanged => "configuration_changed",
            Reason::ConfigurationOverridden => "configuration_overridden",
            Reason::ReopenRequired => "reopen_required",
            Reason::ConnectionProblem => "connection_problem",
            Reason::AccessProblem => "access_problem",
            Reason::VerificationFailed => "verification_failed",
        }
    }
}

impl NextAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            NextAction::ApplyGateConfiguration => "apply_gate_configuration",
            NextAction::ShowConflictingConfig => "show_conflicting_config",
            NextAction::ReopenTool => "reopen_tool",
            NextAction::Reconnect => "reconnect",
            NextAction::SignIn => "sign_in",
            NextAction::RetryCheck => "retry_check",
        }
    }
}

/// One tool's routing verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingVerdict {
    /// Not on this machine. Counts as none of the three states below - an
    /// absent tool must not pad a "3 of 4 protected" count in either direction.
    NotInstalled,
    /// Routing, as far as every check can establish.
    On,
    /// Deliberately not routing, and confirmed to be pointed elsewhere.
    Off,
    NeedsAttention(Reason),
}

impl RoutingVerdict {
    pub const fn as_str(self) -> &'static str {
        match self {
            RoutingVerdict::NotInstalled => "not_installed",
            RoutingVerdict::On => "on",
            RoutingVerdict::Off => "off",
            RoutingVerdict::NeedsAttention(_) => "needs_attention",
        }
    }

    pub const fn reason(self) -> Option<Reason> {
        match self {
            RoutingVerdict::NeedsAttention(r) => Some(r),
            _ => None,
        }
    }
}

/// Decide what a tool is doing.
///
/// The ordering below is the whole content of this function, so it is worth
/// saying why it is that order:
///
/// 1. **Not installed** outranks everything. There is no route to judge.
/// 2. **An unreadable config** is ignorance, not health. It cannot become `Off`
///    (that would claim we verified the tool is pointed elsewhere) and it cannot
///    become `On`, so it is a failed verification.
/// 3. **Drift** outranks the liveness checks. The tool is not using Gate's route
///    regardless of whether that route is up, and `Apply Gate configuration` is
///    the action either way. Reporting a dead relay to someone whose config was
///    rewritten by hand sends them to fix the wrong thing.
/// 4. **A dead route outranks a refused session.** Both block traffic, but the
///    session verdict comes from a direct call that bypasses the local hop
///    ([`crate::org::probe_session`] sets `.no_proxy()`), so it can be a definite
///    401 while the relay is also down. When nothing is listening locally, no
///    request ever left the machine, and naming the credential first would be a
///    guess about the layer above.
/// 5. **Reopen last among the failures**, because it is the one where the route
///    and the credential are both fine.
///
/// The `Off` branch runs the same reopen check as the `On` branch: a tool that
/// was disconnected while running is still holding Gate's values in memory, so
/// it is not yet verifiably pointed at its own upstream.
pub fn verdict_for(ev: &Evidence) -> RoutingVerdict {
    if !ev.installed {
        return RoutingVerdict::NotInstalled;
    }
    if ev.config == ConfigState::Unreadable {
        return RoutingVerdict::NeedsAttention(Reason::VerificationFailed);
    }
    // Beside drift, and above the liveness checks for the same reason: the tool
    // is not on Gate's route whether or not that route is healthy, and naming a
    // dead relay to someone whose traffic never reaches it would send them to
    // fix the wrong thing. The two never arrive together - an integration
    // reports one config state - so this is a ranking of the checks, not of a
    // tool that is somehow both.
    if ev.config == ConfigState::Overridden {
        return RoutingVerdict::NeedsAttention(Reason::ConfigurationOverridden);
    }
    if ev.config == ConfigState::Drifted {
        return RoutingVerdict::NeedsAttention(Reason::ConfigurationChanged);
    }

    // Routing is not wanted. The config carries no Gate values, which is the
    // verification that the tool is pointed at its own upstream - it reads that
    // file on start. A process that predates the change has not re-read it.
    if ev.config == ConfigState::Absent {
        return if ev.reopen_pending {
            RoutingVerdict::NeedsAttention(Reason::ReopenRequired)
        } else {
            RoutingVerdict::Off
        };
    }

    // Managed: Gate's values are in place, so everything else is about whether
    // they can actually work.
    if ev.route == RouteHealth::Unreachable {
        return RoutingVerdict::NeedsAttention(Reason::ConnectionProblem);
    }
    if ev.session == SessionHealth::Rejected {
        return RoutingVerdict::NeedsAttention(Reason::AccessProblem);
    }
    if ev.reopen_pending {
        return RoutingVerdict::NeedsAttention(Reason::ReopenRequired);
    }
    if ev.route == RouteHealth::Unknown || ev.session == SessionHealth::Unknown {
        return RoutingVerdict::NeedsAttention(Reason::VerificationFailed);
    }
    RoutingVerdict::On
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A managed, live, freshly started tool - the only shape that yields `On`.
    fn healthy() -> Evidence {
        Evidence {
            installed: true,
            config: ConfigState::Managed,
            route: RouteHealth::Reachable,
            session: SessionHealth::Valid,
            reopen_pending: false,
        }
    }

    #[test]
    fn healthy_tool_is_on() {
        assert_eq!(verdict_for(&healthy()), RoutingVerdict::On);
    }

    #[test]
    fn absent_config_is_off() {
        let ev = Evidence {
            config: ConfigState::Absent,
            ..healthy()
        };
        assert_eq!(verdict_for(&ev), RoutingVerdict::Off);
    }

    #[test]
    fn uninstalled_outranks_every_other_signal() {
        let ev = Evidence {
            installed: false,
            config: ConfigState::Drifted,
            route: RouteHealth::Unreachable,
            session: SessionHealth::Rejected,
            reopen_pending: true,
        };
        assert_eq!(verdict_for(&ev), RoutingVerdict::NotInstalled);
    }

    #[test]
    fn drift_outranks_a_dead_route() {
        let ev = Evidence {
            config: ConfigState::Drifted,
            route: RouteHealth::Unreachable,
            ..healthy()
        };
        assert_eq!(
            verdict_for(&ev),
            RoutingVerdict::NeedsAttention(Reason::ConfigurationChanged)
        );
    }

    #[test]
    fn dead_route_outranks_a_refused_session() {
        let ev = Evidence {
            route: RouteHealth::Unreachable,
            session: SessionHealth::Rejected,
            ..healthy()
        };
        assert_eq!(
            verdict_for(&ev),
            RoutingVerdict::NeedsAttention(Reason::ConnectionProblem)
        );
    }

    #[test]
    fn refused_session_is_an_access_problem() {
        let ev = Evidence {
            session: SessionHealth::Rejected,
            ..healthy()
        };
        assert_eq!(
            verdict_for(&ev),
            RoutingVerdict::NeedsAttention(Reason::AccessProblem)
        );
    }

    #[test]
    fn stale_process_needs_a_reopen() {
        let ev = Evidence {
            reopen_pending: true,
            ..healthy()
        };
        assert_eq!(
            verdict_for(&ev),
            RoutingVerdict::NeedsAttention(Reason::ReopenRequired)
        );
    }

    /// The `Off` side of the reopen rule: disconnecting a running tool does not
    /// make it verifiably unrouted, because it still holds the old values.
    #[test]
    fn disconnected_but_still_running_needs_a_reopen() {
        let ev = Evidence {
            config: ConfigState::Absent,
            reopen_pending: true,
            ..healthy()
        };
        assert_eq!(
            verdict_for(&ev),
            RoutingVerdict::NeedsAttention(Reason::ReopenRequired)
        );
    }

    /// The distinction `SessionProbe::Unavailable` exists to protect: no verdict
    /// is not a refusal. An offline machine must not accuse the credential.
    #[test]
    fn unknown_session_is_verification_failed_not_access_problem() {
        let ev = Evidence {
            session: SessionHealth::Unknown,
            ..healthy()
        };
        assert_eq!(
            verdict_for(&ev),
            RoutingVerdict::NeedsAttention(Reason::VerificationFailed)
        );
    }

    #[test]
    fn unknown_route_is_verification_failed() {
        let ev = Evidence {
            route: RouteHealth::Unknown,
            ..healthy()
        };
        assert_eq!(
            verdict_for(&ev),
            RoutingVerdict::NeedsAttention(Reason::VerificationFailed)
        );
    }

    /// An unreadable config must never read as `Off`: that would claim we
    /// checked and found the tool pointed elsewhere.
    #[test]
    fn unreadable_config_is_verification_failed_not_off() {
        let ev = Evidence {
            config: ConfigState::Unreadable,
            ..healthy()
        };
        assert_eq!(
            verdict_for(&ev),
            RoutingVerdict::NeedsAttention(Reason::VerificationFailed)
        );
    }

    #[test]
    fn config_state_narrows_every_status() {
        assert_eq!(
            ConfigState::from_status(&Ok(Status::Connected)),
            ConfigState::Managed
        );
        assert_eq!(
            ConfigState::from_status(&Ok(Status::Drifted("by hand".into()))),
            ConfigState::Drifted
        );
        assert_eq!(
            ConfigState::from_status(&Ok(Status::Overridden("/etc/opencode".into()))),
            ConfigState::Overridden
        );
        assert_eq!(
            ConfigState::from_status(&Ok(Status::Detected)),
            ConfigState::Absent
        );
        assert_eq!(
            ConfigState::from_status(&Ok(Status::NotInstalled)),
            ConfigState::Absent
        );
        assert_eq!(
            ConfigState::from_status(&Err(anyhow::anyhow!("unparseable"))),
            ConfigState::Unreadable
        );
    }

    /// The whole point of AG-674: every liveness check can pass - the relay
    /// answers, the session is good, the process is fresh - and the tool is
    /// still not ours to claim, because something above our file decides where
    /// its traffic goes.
    #[test]
    fn an_overridden_config_is_never_on_however_healthy_the_route() {
        let ev = Evidence {
            config: ConfigState::Overridden,
            ..healthy()
        };
        assert_eq!(
            verdict_for(&ev),
            RoutingVerdict::NeedsAttention(Reason::ConfigurationOverridden)
        );
    }

    /// And it is not `Off` either. `Off` is a verified claim that the tool is
    /// pointed at its own upstream; an override says only that the destination
    /// is not ours to name.
    #[test]
    fn an_overridden_config_is_not_reported_as_deliberately_off() {
        let ev = Evidence {
            config: ConfigState::Overridden,
            route: RouteHealth::Unreachable,
            session: SessionHealth::Unknown,
            ..healthy()
        };
        assert!(matches!(
            verdict_for(&ev),
            RoutingVerdict::NeedsAttention(Reason::ConfigurationOverridden)
        ));
    }

    /// Every reason carries exactly one action, and the pairing is what the user
    /// acts on - so it is pinned rather than left to the call site.
    #[test]
    fn every_reason_pairs_with_its_action() {
        for (reason, action) in [
            (
                Reason::ConfigurationChanged,
                NextAction::ApplyGateConfiguration,
            ),
            (
                Reason::ConfigurationOverridden,
                NextAction::ShowConflictingConfig,
            ),
            (Reason::ReopenRequired, NextAction::ReopenTool),
            (Reason::ConnectionProblem, NextAction::Reconnect),
            (Reason::AccessProblem, NextAction::SignIn),
            (Reason::VerificationFailed, NextAction::RetryCheck),
        ] {
            assert_eq!(reason.next_action(), action, "{}", reason.as_str());
        }
    }
}
