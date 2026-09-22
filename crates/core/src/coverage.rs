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

use serde::{Deserialize, Serialize};

/// The gap between what a tool sends and what Gate is able to look at.
///
/// Empty on both counts means full coverage, which is the ordinary case and
/// the one that needs no words.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpstreamCoverage {
    /// Hosts a catalog entry covers, but whose switch is off: `(host, slug)`.
    ///
    /// The remediable half. A switch exists and turning it on closes the gap,
    /// which is why the slug travels with the host.
    pub switched_off: Vec<(String, String)>,
    /// Hosts no catalog entry claims at all, which Gate cannot route whatever
    /// the user does - Bedrock, Vertex, a self-hosted endpoint.
    ///
    /// The irremediable half, and the one AG-932 is mostly about. Offering an
    /// action here would be a lie; the honest move is to say so and stop.
    pub unknown: Vec<String>,
}

impl UpstreamCoverage {
    /// Is there anything to report at all?
    pub fn is_covered(&self) -> bool {
        self.switched_off.is_empty() && self.unknown.is_empty()
    }
}
