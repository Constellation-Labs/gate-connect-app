//! The models this gateway offers, for the picker (AG-588).
//!
//! One call: `GET /v1/models`. It is the gateway's public catalogue - platform
//! provider accounts only, `org_id IS NULL` - which is exactly what "use a Gate
//! model" means: models Gate itself can serve on PAYG credits. An org's own BYOK
//! accounts are deliberately not in it; those are models the org already pays
//! for directly, and offering them under "Use Gate credits" would misdescribe
//! who is being billed.
//!
//! There is an org-aware catalogue on the dashboard API
//! (`GET /api/v1/available-models`), and it is the wrong one here for that
//! reason, as well as living behind a different service and a different
//! credential than everything else this app calls.
//!
//! This module is what remains of a larger one that also read and wrote model
//! *preferences* on the gateway. Those now live in [`crate::preferences`], on
//! this machine; only the catalogue is still a network call, because only the
//! catalogue is something the gateway knows and the app does not.

use crate::account;
use crate::gateway_api::{self, Failure, FailureCode};

/// Catalogue endpoint, with a test seam mirroring [`crate::activity`]'s.
/// `<gateway_base_url>/v1/models` in real builds.
fn catalogue_endpoint(gateway_base_url: &str) -> String {
    if let Some(o) = std::env::var_os("GATE_CONNECT_TEST_GATE_MODELS_ENDPOINT") {
        return o.to_string_lossy().into_owned();
    }
    format!("{}/v1/models", gateway_base_url.trim_end_matches('/'))
}

/// The models this gateway offers, as raw JSON.
///
/// An empty list is a legitimate answer, not a failure: the catalogue is built
/// from platform provider accounts, and a deployment with none has nothing to
/// offer. The picker has to say so rather than draw an empty list as though the
/// request had not finished.
///
/// The route is public on the gateway, but the call still goes through
/// [`gateway_api::call_json`] so it inherits the two rules that matter: no
/// data-plane proxy, and the account's own credential when there is one. A
/// public route reached through our own MITM would answer, and answer for the
/// wrong deployment.
pub fn catalogue_json() -> Result<String, Failure> {
    let base = match account::load() {
        Ok(Some(a)) => a.gateway_base_url,
        Ok(None) => {
            return Err(Failure::new(
                FailureCode::SignedOut,
                "no gateway account is configured",
            ))
        }
        Err(e) => return Err(Failure::new(FailureCode::Unknown, format!("{e:#}"))),
    };
    gateway_api::call_json(
        gateway_api::Method::Get,
        catalogue_endpoint(&base),
        &[],
        None,
    )
}
