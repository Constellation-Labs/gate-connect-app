//! Gate models, as a choice written into each tool's own config.
//!
//! The pane's Gate model card stores a choice ([`crate::preferences`]) and this
//! module turns it into the tool's config: the integration's `connect` writes
//! the chosen models - the default model, the tool's own model picker, and a
//! base URL on the relay's Gate models route ([`crate::proxy::gate_served`]) -
//! and reads them back through [`Integration::gate_model_state`].
//!
//! Two directions, and the second is the one that is easy to get wrong:
//!
//! - **Gate Connect to the tool.** [`choose`] stores the choice and rewrites the
//!   tool's config when Gate manages it. A tool reads its config at startup, so
//!   the change reaches the next session; the window offers the restart notice
//!   (the write stamps `config_changes`, which is what the reopen check reads).
//! - **The tool to Gate Connect.** A user can change the model inside the tool.
//!   Its config is the source of truth for what it will run, so [`states`]
//!   reads every config, and one that has moved off the Gate models Gate wrote
//!   puts that tool back on its own model - stored as such, so no later connect
//!   writes the Gate model back over the user's choice - and reports it, so the
//!   pane can say what happened.
//!
//! [`Integration::gate_model_state`]: crate::registry::Integration::gate_model_state

use anyhow::Result;
use std::collections::BTreeMap;

use crate::preferences::{self, GateModelMeta, ModelSource};
use crate::registry::{self, GateModelState, ToolId};

/// Serialises [`choose`] and [`states`] with each other.
///
/// Both read a tool's config, decide, and write the stored choice and the
/// config back, and the window runs them in separate blocking tasks: a focus
/// re-read folding drift while a save is in flight must not interleave with
/// it. Taken before the master-flow lock (`provider::reapply_tool_config`,
/// `provider::leave_gate_models`), never after, and nothing under that lock
/// calls back in here.
static FLOW_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The most Gate models one tool can be put on at once.
///
/// A product limit, not a technical one (design, 2026-09-30): the picker stops
/// at this many, and [`choose`] refuses more, so the CLI and anything else that
/// stores a choice cannot go past what the window allows. Only a Gate choice is
/// held to it. App default keeps whatever set it remembers, so a set stored
/// before the limit existed can still be put back on the tool's own model.
pub const MAX_GATE_MODELS: usize = 4;

fn flow_guard() -> std::sync::MutexGuard<'static, ()> {
    FLOW_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// What one tool's Gate model card should show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolModelView {
    /// Whether the card applies: the tool supports Gate models.
    pub supported: bool,
    /// What the tool's config says now, after any drift has been folded back
    /// into the stored choice.
    pub state: GateModelState,
    /// Set when this read found the tool moved off Gate models from inside it,
    /// and put it back on its own model. The model it names now, if any.
    pub left_gate_models: Option<Option<String>>,
    /// Set when the tool's config could not be read, or the tool could not be
    /// put back on its own model after it drifted. Either way the card must
    /// say so: a stored App default over a config still on the Gate models
    /// route means every request is refused (review on #382).
    pub problem: Option<String>,
}

/// Store one tool's model choice and write it into the tool's config.
///
/// Returns whether the config was rewritten. `false` is not a failure: a tool
/// Gate does not manage right now keeps the choice for its next connect.
///
/// `meta` is what the catalogue says about the chosen models, remembered so
/// the tool's picker can show names and context windows on an offline
/// reconnect ([`preferences::Preferences::gate_model_meta`]).
pub fn choose(
    tool: ToolId,
    source: ModelSource,
    model_ids: Vec<String>,
    acknowledge_paid_use: bool,
    meta: Vec<(String, GateModelMeta)>,
) -> Result<bool> {
    if source == ModelSource::Gate && model_ids.len() > MAX_GATE_MODELS {
        anyhow::bail!(
            "{} Gate models were chosen, and a tool can use at most {MAX_GATE_MODELS}",
            model_ids.len()
        );
    }
    let _guard = flow_guard();
    let integ = registry::find(tool);
    let supported = integ.as_ref().is_some_and(|i| i.supports_gate_models());
    // Settle drift left by the PREVIOUS choice before storing this one. A config
    // the user moved off Gate models that no read has folded yet (CLI use, a
    // window that never lost focus, a fix-up that failed) would otherwise be
    // folded by the `connect` below, against the new choice: `connect` sees the
    // old record drifted and falls back to App default, undoing what the user
    // just picked while this call reported success (review on #382).
    if let Some(integ) = integ.as_ref().filter(|_| supported) {
        if matches!(integ.gate_model_state(), Ok(GateModelState::Drifted { .. })) {
            leave_gate_models(tool)?;
        }
    }
    preferences::set_tool_model(tool.slug(), source, model_ids, acknowledge_paid_use, meta)?;
    if !supported {
        return Ok(false);
    }
    match crate::provider::reapply_tool_config(tool.slug()) {
        Ok(applied) => Ok(applied),
        // Going back to the tool's own model cannot be left half done: the
        // stored choice already says App default, and a config still on the Gate
        // models route would have every request refused. So if the full rewrite
        // fails, the Gate models still come out, and the error is reported
        // either way.
        Err(e) if source == ModelSource::Tool => {
            let _ = crate::provider::leave_gate_models(tool.slug());
            Err(e)
        }
        Err(e) => Err(e),
    }
}

/// Every supporting tool's card, read from its config.
///
/// Drift is reconciled here, on read, because a read is when it can be seen:
/// the pane opening, the window regaining focus. See the module doc for why it
/// is stored and not only shown.
pub fn states() -> BTreeMap<&'static str, ToolModelView> {
    let _guard = flow_guard();
    let mut out = BTreeMap::new();
    for integ in registry::registry() {
        let tool = integ.id();
        if !integ.supports_gate_models() {
            continue;
        }
        // The card gets a fixed sentence and the log gets the chain. A parse
        // error can quote the offending line of a config that sits next to a
        // token, and the chain is developer text in any case (review on #382).
        let (state, problem) = match integ.gate_model_state() {
            Ok(state) => (state, None),
            Err(e) => {
                crate::logging::failure(&format!(
                    "reading {}'s config for Gate models failed: {e:#}",
                    integ.display_name()
                ));
                (
                    GateModelState::NotApplied,
                    Some(format!(
                        "Gate Connect could not read {}'s config, so it cannot tell which \
                         model it runs.",
                        integ.display_name()
                    )),
                )
            }
        };
        let mut view = ToolModelView {
            supported: true,
            state: state.clone(),
            left_gate_models: None,
            problem,
        };
        if let GateModelState::Drifted { model } = state {
            match leave_gate_models(tool) {
                Ok(()) => {
                    view.state = integ
                        .gate_model_state()
                        .unwrap_or(GateModelState::NotApplied);
                    view.left_gate_models = Some(model);
                }
                Err(e) => {
                    crate::logging::failure(&format!(
                        "putting {} back on its own model failed: {e:#}",
                        integ.display_name()
                    ));
                    // The card's title already says the requests are refused,
                    // so this is the cause and the fix only.
                    view.problem = Some(format!(
                        "{name} left Gate models and couldn’t go back to its own model. Choose \
                         a model again under Model selection.",
                        name = integ.display_name()
                    ));
                }
            }
        }
        out.insert(tool.slug(), view);
    }
    out
}

/// The user moved `tool` off Gate models inside the tool: store that, then take
/// Gate's picker and route out of the config. The model the user picked is left
/// where it is - the integrations restore only values that are still the ones
/// Gate wrote - and so is the routing.
fn leave_gate_models(tool: ToolId) -> Result<()> {
    preferences::fall_back_to_tool_model(tool.slug())?;
    crate::provider::leave_gate_models(tool.slug())
}

/// The picker fields for `ids`, read off the catalogue JSON the window already
/// fetched. Best effort by design: a model the catalogue does not describe is
/// written by id alone, and a catalogue that will not parse describes nothing.
pub fn meta_from_catalogue(catalogue_json: &str, ids: &[String]) -> Vec<(String, GateModelMeta)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(catalogue_json) else {
        return Vec::new();
    };
    let Some(rows) = v.get("data").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|row| {
            let id = row.get("id")?.as_str()?;
            if !ids.iter().any(|want| want == id) {
                return None;
            }
            let num = |k: &str| row.get(k).and_then(|n| n.as_u64());
            let freeform = row
                .get("tool_shapes")
                .and_then(|t| t.get("freeform"))
                .and_then(|f| f.get("verdict"))
                .and_then(|v| v.as_str());
            Some((
                id.to_string(),
                GateModelMeta {
                    name: row.get("name").and_then(|n| n.as_str()).map(str::to_owned),
                    context_window: num("context_window"),
                    max_tokens: num("max_tokens"),
                    freeform_tools: match freeform {
                        Some("works") => Some(true),
                        Some("fails") => Some(false),
                        _ => None,
                    },
                },
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_rows_become_picker_fields_for_chosen_ids_only() {
        let json = r#"{"object":"list","data":[
            {"id":"openai/gpt-5.6-luna","name":"GPT-5.6 Luna","context_window":400000,"max_tokens":128000,
             "tool_shapes":{"freeform":{"verdict":"works"},"function":{"verdict":"works"}}},
            {"id":"anthropic/claude-opus-5","name":"Claude Opus 5"},
            {"id":"other/model","name":"Other"}
        ]}"#;
        let ids = vec![
            "openai/gpt-5.6-luna".to_string(),
            "anthropic/claude-opus-5".to_string(),
        ];
        let meta = meta_from_catalogue(json, &ids);
        assert_eq!(meta.len(), 2);
        assert_eq!(meta[0].0, "openai/gpt-5.6-luna");
        assert_eq!(meta[0].1.context_window, Some(400_000));
        assert_eq!(meta[1].1.name.as_deref(), Some("Claude Opus 5"));
        assert_eq!(meta[1].1.context_window, None);
        assert_eq!(meta[0].1.freeform_tools, Some(true));
        assert_eq!(
            meta[1].1.freeform_tools, None,
            "nobody checked: unknown, not yes"
        );
    }

    #[test]
    fn an_unreadable_catalogue_describes_nothing() {
        assert!(meta_from_catalogue("nope", &["a/b".to_string()]).is_empty());
        assert!(meta_from_catalogue("{}", &["a/b".to_string()]).is_empty());
    }
}
