//! What Gate can and cannot see of a tool's upstream.
//!
//! A tool being *routed* and Gate being able to *inspect* it are two different
//! things, and the rail conflated them: a row read Protected whenever its
//! config named Gate, whatever happened to the traffic afterwards. For most
//! tools that is harmless, because their section bundles the provider domain
//! with the tool and one switch does both.
//!
//! Hermes and OpenClaw are the exceptions. Their upstream is whatever the user
//! configured, and the row that would intercept it lives in somebody else's
//! section - so "routed" and "inspected" come apart, and the app claimed the
//! second while delivering only the first. AG-932.

/// The gap between what a tool sends and what Gate is able to look at.
///
/// Shaped by #327 and shared since AG-932: `switched_off` is keyed by catalog
/// slug rather than by host, because one row can claim several hosts and it is
/// one switch either way - a caller that names rows must not name the same row
/// twice or flip the same switch twice.
#[derive(Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct UpstreamCoverage {
    /// Whether the endpoints are Hermes' documented default rather than read
    /// from `config.yaml` - because the file is missing, does not parse, or
    /// names no endpoint. A caller's copy has to say which: "your config uses
    /// OpenRouter" is false about a file that was never read, and an install
    /// with no config really will call OpenRouter.
    pub defaulted: bool,
    /// Provider rows the catalog covers whose switch is off, one per slug.
    pub switched_off: Vec<SwitchedOff>,
    /// Hosts no catalog entry claims, which Gate cannot route at all.
    pub unknown: Vec<String>,
}

/// One provider row Hermes points at that Gate is not inspecting.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SwitchedOff {
    /// The catalog slug, which is what `proxy domain` and `proxy_set_domain`
    /// take.
    pub slug: String,
    /// Every host in `config.yaml` this row claims. One row can claim several,
    /// and it is one switch either way, so a caller that names rows must not
    /// name the same row twice or flip the same switch twice.
    pub hosts: Vec<String>,
    /// Display names of the tools whose provider this domain switches on.
    /// `provider::reconcile_enabled` reads an enabled cascade domain as licence
    /// to connect that provider's detected tools at the next launch, so turning
    /// `anthropic` on for Hermes' sake also reaches Claude Code. Empty for a
    /// proxy-only provider such as OpenRouter. A caller that asks has to say
    /// this, because the switch itself does not.
    pub tools: Vec<String>,
}

impl UpstreamCoverage {
    /// Is there anything to report at all?
    pub fn is_covered(&self) -> bool {
        self.switched_off.is_empty() && self.unknown.is_empty()
    }
}
