//! Which Gate model a tool should run on (AG-588).
//!
//! Three calls, all on the gateway:
//!
//! - `GET /v1/me/tool-models` - what this org has chosen, per platform.
//! - `PUT /v1/me/tool-models` - choose, for one platform.
//! - `GET /v1/models` - the catalogue the picker lists.
//!
//! The gateway side lives in `apps/gateway-proxy/src/tool-models/`, and its
//! controller doc carries the rules this module only relays: a write needs a
//! user credential and at least Member, and a first paid selection needs an
//! acknowledgement.
//!
//! **The preference is keyed on the gateway's platform id, not on our tool
//! slug.** [`ToolId::platform_id`] explains why they are different namespaces
//! and why one of our tools has no platform id at all. A tool that has none
//! cannot hold a preference, so [`platform_id_for`] refuses before a request is
//! sent rather than letting the gateway reject it - the message names the tool,
//! which a 400 from the far side would not.
//!
//! Bodies cross as raw JSON strings, as in [`crate::activity`]: `lib/toolModels.ts`
//! is the single place that models the shape.

use serde::Serialize;

use crate::account;
use crate::gateway_api::{self, Failure, FailureCode};
use crate::registry::ToolId;

/// Preferences endpoint, with a test seam mirroring [`crate::activity`]'s.
/// `<gateway_base_url>/v1/me/tool-models` in real builds.
fn prefs_endpoint(gateway_base_url: &str) -> String {
    if let Some(o) = std::env::var_os("GATE_CONNECT_TEST_TOOL_MODELS_ENDPOINT") {
        return o.to_string_lossy().into_owned();
    }
    format!(
        "{}/v1/me/tool-models",
        gateway_base_url.trim_end_matches('/')
    )
}

/// Public model catalogue, with its own seam.
/// `<gateway_base_url>/v1/models` in real builds.
///
/// Its own seam rather than sharing the preferences one, so a test can serve an
/// empty catalogue alongside a populated set of preferences - which is the state
/// a real deployment with no platform provider accounts is in, and the one where
/// a selected model has nothing to look itself up against.
fn catalogue_endpoint(gateway_base_url: &str) -> String {
    if let Some(o) = std::env::var_os("GATE_CONNECT_TEST_GATE_MODELS_ENDPOINT") {
        return o.to_string_lossy().into_owned();
    }
    format!("{}/v1/models", gateway_base_url.trim_end_matches('/'))
}

/// What Gate should serve for a tool. Mirrors the gateway's `source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The app picks its own model and Gate does not intervene.
    Tool,
    /// Gate serves the models named in the selection, overriding the app.
    Gate,
}

impl Source {
    const fn wire(self) -> &'static str {
        match self {
            Source::Tool => "tool",
            Source::Gate => "gate",
        }
    }

    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "tool" => Some(Source::Tool),
            "gate" => Some(Source::Gate),
            _ => None,
        }
    }
}

/// The gateway's platform id for a tool, or a failure that says why there is
/// none.
///
/// A refusal rather than a silent no-op: the caller is about to store a choice,
/// and "we quietly did nothing" is the outcome principle 2 in `CLAUDE.md` warns
/// about - the control would read as set while no request was affected.
pub fn platform_id_for(tool: ToolId) -> Result<&'static str, Failure> {
    tool.platform_id().ok_or_else(|| {
        Failure::new(
            FailureCode::Gateway,
            format!(
                "the gateway cannot identify {tool} on a request, so it cannot hold a model preference for it"
            ),
        )
    })
}

/// This org's model preferences, as raw JSON.
///
/// Not scoped to a tool: the response covers every platform the org has chosen
/// for, and the panes filter it. One read serves the whole sidebar, and a
/// per-tool read would ask the same question once per app for an answer that is
/// org-wide by design.
pub fn preferences_json() -> Result<String, Failure> {
    let base = base_url()?;
    gateway_api::call_json(gateway_api::Method::Get, prefs_endpoint(&base), &[], None)
}

/// Set one tool's preference, returning the stored row as raw JSON.
///
/// `model_ids` must be empty for [`Source::Tool`] and non-empty for
/// [`Source::Gate`]; the gateway refuses either mistake and its database refuses
/// the second even if the route were bypassed, so this does not re-validate.
/// What it does do is refuse a tool the gateway cannot identify *before*
/// spending a round trip on it.
///
/// `acknowledge_paid_use` should be true only when the user has just been shown
/// and accepted the billing confirmation. Sending it when the org has already
/// acknowledged is harmless and does not move the recorded date.
pub fn set_preference_json(
    tool: ToolId,
    source: Source,
    model_ids: &[String],
    acknowledge_paid_use: bool,
) -> Result<String, Failure> {
    let platform_id = platform_id_for(tool)?;
    let base = base_url()?;
    // Serialized through `serde_json` rather than formatted, so a model id
    // carrying a quote cannot break out of the body.
    let body = serde_json::json!({
        "platformId": platform_id,
        "source": source.wire(),
        "modelIds": model_ids,
        "acknowledgePaidUse": acknowledge_paid_use,
    })
    .to_string();
    gateway_api::call_json(
        gateway_api::Method::Put,
        prefs_endpoint(&base),
        &[],
        Some(body),
    )
}

/// The models this gateway offers, as raw JSON.
///
/// An empty list is a legitimate answer, not a failure: the catalogue is built
/// from platform provider accounts, and a deployment with none has nothing to
/// offer. The picker has to say so rather than draw an empty list as though the
/// request had not finished.
pub fn catalogue_json() -> Result<String, Failure> {
    let base = base_url()?;
    gateway_api::call_json(
        gateway_api::Method::Get,
        catalogue_endpoint(&base),
        &[],
        None,
    )
}

/// The configured gateway's base URL, or a signed-out failure.
///
/// Loaded here as well as inside [`gateway_api::call_json`] because the URL has
/// to be built before the call is made. Cheap - a file read the account layer
/// caches - and it keeps each endpoint function owning its own route.
fn base_url() -> Result<String, Failure> {
    match account::load() {
        Ok(Some(a)) => Ok(a.gateway_base_url),
        Ok(None) => Err(Failure::new(
            FailureCode::SignedOut,
            "no gateway account is configured",
        )),
        Err(e) => Err(Failure::new(FailureCode::Unknown, format!("{e:#}"))),
    }
}
