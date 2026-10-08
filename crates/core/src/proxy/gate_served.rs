//! The relay route a tool's own config points at when the user puts it on Gate
//! models.
//!
//! Gate Models used to be applied here in the proxy: the chosen models rode on
//! every request as a header and the gateway rewrote the body's `model`, so the
//! tool went on showing - and reasoning about - a model it was not being served.
//! The choice now lives in the tool's config: its default model is a Gate id,
//! its model picker lists the enabled set, and its base URL is this route. What
//! the tool asks for is what it gets.
//!
//! **The URL is the whole signal.** A request that arrives here is served by
//! Gate on the organization's credits whatever the account's billing mode, and a
//! request that arrives anywhere else is routed exactly as it was before. Only
//! Gate Connect writes this URL. The route itself also refuses a tool that does
//! not support Gate models, and serves nothing until this install has accepted
//! paid Gate model use - once per install, not per tool
//! (`preferences::gate_models_served_for`).
//!
//! **What is refused, and why a refusal rather than a substitution.** The model
//! must be one the user enabled for this tool. Anything else - a `-m` flag, a
//! hand-edited config, another tool borrowing the URL - gets a readable error
//! naming the model, and nothing is billed. Serving it would spend credits on a
//! model the user never confirmed; rewriting it to one they did is the silent
//! substitution this route replaced. Gate Connect sees the same drift from the
//! config side and puts the tool back on its own default.
//!
//! Shape: `<relay>/__gate/t/<tool>/gate/v1/<path>`. The tool marker is
//! required here, where it is optional on the catalog routes, because it is
//! what the enabled set is looked up by.

use crate::registry::ToolId;

/// The leading path segment that selects this route, in the slot a catalog
/// slug occupies on the other relay routes. No catalog entry may use it.
pub(crate) const SLUG: &str = gate_connect_paths::RELAY_GATE_SERVED_SLUG;

/// What the tool appends its own paths to, after the slug: every served wire
/// format lives under `/v1` on the gateway.
const CLIENT_PATH: &str = "/v1";

/// The base URL to write into a tool's config for this route.
///
/// `relay_base` is the relay's origin, `http://127.0.0.1:<port>`, as
/// [`super::relay::base_url`] builds it.
pub fn relay_base_url(relay_base: &str, tool: ToolId) -> String {
    route_url(relay_base, tool.slug())
}

fn route_url(relay_base: &str, marker: &str) -> String {
    format!(
        "{}{}{marker}/{SLUG}{CLIENT_PATH}",
        relay_base.trim_end_matches('/'),
        super::relay::TOOL_PATH_PREFIX,
    )
}

/// The route's root for Claude Desktop, which has no config to write it into:
/// the engine moves the app's Code tab requests here itself
/// (`code_tab_gate_models` in `engine.rs`). Marked with the app's own client
/// slug, which is what the relay attributes it to and looks its models up by.
pub fn desktop_app_root_url(relay_base: &str) -> String {
    route_url(relay_base, crate::tool_models::DESKTOP_APP)
        .strip_suffix(CLIENT_PATH)
        .expect("route_url ends in CLIENT_PATH")
        .to_string()
}

/// The same route without its `/v1`, for a tool whose SDK appends the version
/// itself: the Anthropic SDK's base URL ends before `/v1/messages`.
pub fn relay_root_url(relay_base: &str, tool: ToolId) -> String {
    relay_base_url(relay_base, tool)
        .strip_suffix(CLIENT_PATH)
        .expect("relay_base_url ends in CLIENT_PATH")
        .to_string()
}

/// Is `base_url` this route, for `tool`, on any relay port?
///
/// Port-blind on purpose: the relay port is persisted but can move, and a
/// config written before a move still means "on Gate models" - reconcile
/// rewrites the port, and until then the question being asked is about intent.
pub fn is_relay_base_url(base_url: &str, tool: ToolId) -> bool {
    let suffix = format!(
        "{}{}/{SLUG}{CLIENT_PATH}",
        super::relay::TOOL_PATH_PREFIX,
        tool.slug()
    );
    let trimmed = base_url.trim_end_matches('/');
    // The root form counts too: it is the same route, with the version left to
    // the tool's SDK (see [`relay_root_url`]).
    let with_version;
    let trimmed = if trimmed.ends_with(&suffix) {
        trimmed
    } else {
        with_version = format!("{trimmed}{CLIENT_PATH}");
        with_version.as_str()
    };
    trimmed.ends_with(&suffix)
        && trimmed.strip_suffix(&suffix).is_some_and(|origin| {
            origin.starts_with("http://127.0.0.1:") || origin.starts_with("http://localhost:")
        })
}

/// Whether the gateway can answer `method path` itself.
///
/// The three inference wire formats and their token counters, plus the model
/// list a tool may fetch to fill its picker. Anything else is refused before it
/// leaves the machine: with no upstream to forward to, the gateway would hold
/// the socket open on a path it does not implement and the tool would hang.
pub(crate) fn serves(method: &hyper::Method, path: &str) -> bool {
    match *method {
        hyper::Method::GET => path == "/v1/models",
        hyper::Method::POST => matches!(
            path,
            "/v1/responses" | "/v1/chat/completions" | "/v1/messages" | "/v1/messages/count_tokens"
        ),
        _ => false,
    }
}

/// Why a request on this route was refused, in words the tool will show.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Refusal {
    pub code: &'static str,
    pub message: String,
}

/// Check the body's `model` against what the user enabled for `tool`.
///
/// `enabled` is the stored set, `None` when the tool is not on Gate models at
/// all. Exact string comparison: Gate Connect writes the catalogue id verbatim,
/// so anything that differs did not come from the picker.
pub(crate) fn check_model(
    tool: &str,
    display_name: &str,
    enabled: Option<&[String]>,
    body: &[u8],
) -> Result<(), Refusal> {
    let Some(enabled) = enabled else {
        return Err(Refusal {
            code: "gate_models_off",
            message: format!(
                "{display_name} is not set to use Gate models in Gate Connect. \
                 Choose a Gate model for {display_name} in Gate Connect, or switch \
                 {display_name} back to its own provider."
            ),
        });
    };
    let model = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("model").and_then(|m| m.as_str()).map(str::to_owned));
    let Some(model) = model else {
        return Err(Refusal {
            code: "gate_model_missing",
            message: format!("the request from {tool} names no model"),
        });
    };
    if enabled.contains(&model) {
        return Ok(());
    }
    Err(Refusal {
        code: "gate_model_not_enabled",
        message: format!(
            "{model} is not one of the Gate models enabled for {display_name}. \
             Enabled: {}. Pick one of those, or enable {model} in Gate Connect.",
            enabled.join(", ")
        ),
    })
}

/// Top-level body fields that steer how a request is routed rather than what
/// it asks for, all dropped on this route (review on #382):
///
/// - `provider` picks which of the organization's accounts serves it - the
///   org pays here, so a local caller must not choose which account does;
/// - `models` (an OpenRouter-style fallback list) and `route` let a request be
///   served as a model OTHER than the `model` [`check_model`] checked, which
///   would put a model outside the enabled set on the org's credits.
///
/// Dropped rather than refused: the checked `model` is still exactly what is
/// served, and a tool configured with fallbacks keeps working on it.
const ROUTING_OVERRIDES: &[&str] = &["provider", "models", "route"];

/// The body without any [`ROUTING_OVERRIDES`], or `None` when it has none.
pub(crate) fn without_routing_overrides(body: &[u8]) -> Option<Vec<u8>> {
    let mut v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let obj = v.as_object_mut()?;
    let removed = ROUTING_OVERRIDES
        .iter()
        .filter(|k| obj.remove(**k).is_some())
        .count();
    if removed == 0 {
        return None;
    }
    serde_json::to_vec(&v).ok()
}

/// An OpenAI-shaped error body. Codex, Hermes and the OpenAI SDKs all surface
/// `error.message`, so this is what makes a refusal readable in the tool rather
/// than a bare status.
pub(crate) fn error_body(code: &str, message: &str) -> String {
    serde_json::json!({
        "error": { "message": message, "type": "gate_connect", "code": code }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_base_url_names_the_tool_and_the_route() {
        assert_eq!(
            relay_base_url("http://127.0.0.1:47174", ToolId::Codex),
            "http://127.0.0.1:47174/__gate/t/codex/gate/v1"
        );
        assert_eq!(
            relay_base_url("http://127.0.0.1:47174/", ToolId::Hermes),
            "http://127.0.0.1:47174/__gate/t/hermes/gate/v1"
        );
    }

    #[test]
    fn recognising_the_base_url_ignores_the_port_but_not_the_tool() {
        let url = relay_base_url("http://127.0.0.1:1", ToolId::Codex);
        assert!(is_relay_base_url(&url, ToolId::Codex));
        assert!(is_relay_base_url(
            "http://127.0.0.1:9999/__gate/t/codex/gate/v1/",
            ToolId::Codex
        ));
        assert!(!is_relay_base_url(&url, ToolId::Hermes));
        let root = relay_root_url("http://127.0.0.1:1", ToolId::ClaudeCode);
        assert_eq!(root, "http://127.0.0.1:1/__gate/t/claude-code/gate");
        assert!(is_relay_base_url(&root, ToolId::ClaudeCode));
        assert!(!is_relay_base_url(
            "http://127.0.0.1:1/__gate/t/codex/openai/v1",
            ToolId::Codex
        ));
        // Not ours if it is not loopback.
        assert!(!is_relay_base_url(
            "https://evil.example/__gate/t/codex/gate/v1",
            ToolId::Codex
        ));
    }

    #[test]
    fn only_paths_the_gateway_answers_are_served() {
        use hyper::Method;
        assert!(serves(&Method::POST, "/v1/responses"));
        assert!(serves(&Method::POST, "/v1/chat/completions"));
        assert!(serves(&Method::POST, "/v1/messages"));
        assert!(serves(&Method::POST, "/v1/messages/count_tokens"));
        assert!(serves(&Method::GET, "/v1/models"));
        assert!(!serves(&Method::GET, "/v1/responses"));
        assert!(!serves(&Method::POST, "/v1/models"));
        assert!(!serves(&Method::POST, "/codex/responses"));
        assert!(!serves(&Method::POST, "/v1/responses/compact"));
        assert!(!serves(&Method::POST, "/api/oauth/usage"));
    }

    fn set(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn an_enabled_model_passes_untouched() {
        let enabled = set(&["openai/gpt-5.6-luna", "anthropic/claude-opus-5"]);
        let body = br#"{"model":"anthropic/claude-opus-5","input":"hi"}"#;
        assert_eq!(check_model("codex", "Codex", Some(&enabled), body), Ok(()));
    }

    #[test]
    fn a_model_outside_the_set_is_refused_by_name_not_substituted() {
        let enabled = set(&["openai/gpt-5.6-luna"]);
        let body = br#"{"model":"gpt-5.6-sol"}"#;
        let refusal = check_model("codex", "Codex", Some(&enabled), body).unwrap_err();
        assert_eq!(refusal.code, "gate_model_not_enabled");
        assert!(refusal.message.contains("gpt-5.6-sol"));
        assert!(refusal.message.contains("openai/gpt-5.6-luna"));
    }

    #[test]
    fn a_tool_that_is_not_on_gate_models_is_refused() {
        let body = br#"{"model":"openai/gpt-5.6-luna"}"#;
        let refusal = check_model("hermes", "Hermes", None, body).unwrap_err();
        assert_eq!(refusal.code, "gate_models_off");
    }

    #[test]
    fn a_body_with_no_model_is_refused() {
        let enabled = set(&["openai/gpt-5.6-luna"]);
        for body in [&b"{}"[..], b"not json", b""] {
            let refusal = check_model("codex", "Codex", Some(&enabled), body).unwrap_err();
            assert_eq!(refusal.code, "gate_model_missing");
        }
    }

    #[test]
    fn routing_overrides_are_dropped_and_nothing_else_is() {
        let body = br#"{"model":"a/b","provider":{"order":["x"]},"models":["c/d"],"route":"fallback","input":"hi"}"#;
        let out: serde_json::Value =
            serde_json::from_slice(&without_routing_overrides(body).expect("had some")).unwrap();
        assert_eq!(out, serde_json::json!({"model":"a/b","input":"hi"}));
        assert!(without_routing_overrides(br#"{"model":"a/b"}"#).is_none());
        assert!(without_routing_overrides(b"not json").is_none());
    }

    #[test]
    fn the_error_body_is_openai_shaped() {
        let v: serde_json::Value = serde_json::from_str(&error_body("c", "m")).expect("valid json");
        assert_eq!(v["error"]["message"], "m");
        assert_eq!(v["error"]["code"], "c");
    }
}
