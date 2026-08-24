//! Master routing ON/OFF as one policy, shared by every host.
//!
//! The enable sequence is "persist the intent, restore providers, start the
//! engine, restore again for domain-only providers"; the disable sequence is
//! "snapshot-and-disable everything, stop the engine, clear the intent".
//! Each step's ordering encodes a correctness invariant (documented inline),
//! and the sequence used to be hand-copied across the app's `proxy_enable` /
//! `proxy_disable` commands, its startup auto-enable thread, and the CLI's
//! `proxy enable` - so an ordering fix had to land in three places. This
//! module is now the only copy; the shell and CLI are one-line callers that
//! differ only in how they surface [`Warning`]s.

use anyhow::Result;

use crate::provider;
use crate::proxy::{self, intent, ProxyState};

/// A best-effort step that failed inside [`enable`] or [`disable`]. The
/// sequences deliberately keep going - a restore hiccup must never block the
/// proxy from coming up, and a snapshot hiccup must never block the kill
/// switch. `component` is the stable tag the app's backend-error buffer keys
/// on; the CLI prints the error as a note instead.
pub struct Warning {
    pub component: &'static str,
    pub error: anyhow::Error,
}

impl Warning {
    fn new(component: &'static str, error: anyhow::Error) -> Self {
        Self { component, error }
    }
}

/// Master ON. Returns the engine state from a successful enable, plus any
/// best-effort warnings; a failed enable is the only hard error.
///
/// The intent is persisted *first*, not after the engine is up: it is the
/// user's choice, known before anything runs, and writing it late left a
/// crash window where routing was live with nothing recorded - a reboot then
/// came up silently passthrough. Persisting first means a failed enable
/// leaves the intent true, which is also right: the startup auto-enable
/// retries it on the next launch and already degrades quietly when it cannot
/// complete unattended.
pub fn enable() -> Result<(ProxyState, Vec<Warning>)> {
    let mut warnings = Vec::new();
    if let Err(e) = intent::set_intent(true) {
        warnings.push(Warning::new("routing_intent", e));
    }
    // Restore every provider that was on when routing was last turned off -
    // *before* enabling - so the engine comes back up routing the user's
    // prior selection rather than bare. A no-op (no snapshot) on a first
    // enable, where the engine simply starts with zero domains and passes
    // through until a provider is enabled.
    if let Err(e) = provider::restore_all() {
        warnings.push(Warning::new("provider_restore", e));
    }
    let state = proxy::manager().enable()?;
    // Second restore pass now that the proxy is up: domain-only providers
    // (no installed tool) have nothing to configure before the engine is
    // running, so the pre-enable pass leaves them in the snapshot.
    if let Err(e) = provider::restore_all() {
        warnings.push(Warning::new("provider_restore", e));
    }
    Ok((state, warnings))
}

/// Master OFF. Returns the engine state from a successful disable, plus any
/// best-effort warnings; a failed disable is the only hard error.
///
/// The provider sweep runs BEFORE the proxy stops, so config-based tools
/// (Codex) also stop and their domains are still flippable. The full sweep,
/// not the provider-only pass: the catalog maps no provider to OpenCode and
/// friends, and leaving them pointed at the relay we are about to kill would
/// strand them while the UI reports "not routing". The intent clears last -
/// explicit "off" is sticky across restarts, so the startup auto-enable
/// leaves the machine in passthrough.
pub fn disable() -> Result<(ProxyState, Vec<Warning>)> {
    let mut warnings = Vec::new();
    if let Err(e) = provider::snapshot_and_disable_everything() {
        warnings.push(Warning::new("provider_disable", e));
    }
    let state = proxy::manager().disable()?;
    if let Err(e) = intent::set_intent(false) {
        warnings.push(Warning::new("routing_intent", e));
    }
    Ok((state, warnings))
}
