//! `x-gate-model`: telling the gateway which model to serve (AG-588, AG-590).
//!
//! An integration test for the reason `tool_model_preferences.rs` gives - it
//! writes the real preferences file, and the app-support override is
//! process-global.
//!
//! What is under test is mostly what the injection must NOT do. This header
//! decides what somebody is billed for, so the load-bearing cases are a tool
//! setting it itself, a tool the app cannot identify, and a choice that is
//! stored but not active.

use std::path::PathBuf;
use std::sync::Mutex;

use gate_connect_core::env;
use gate_connect_core::preferences::{self, ModelSource};
use gate_connect_core::proxy::testing::{
    inject_attribution_for_tests, serves_gate_model, strip_tool_credential_for_tests,
    GATE_MODEL_HEADER_NAME,
};

use hyper::header::{HeaderMap, HeaderValue, USER_AGENT};

static LOCK: Mutex<()> = Mutex::new(());

struct TempHome {
    dir: PathBuf,
}

impl TempHome {
    fn set() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("gc-model-inject-{n}"));
        std::fs::create_dir_all(&dir).expect("create temp app-support dir");
        env::set_app_support_dir_for_tests(Some(dir.clone()));
        preferences::reset_cache_for_tests();
        Self { dir }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        env::set_app_support_dir_for_tests(None);
        preferences::reset_cache_for_tests();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Headers as a request from `ua` would arrive.
fn headers(ua: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(USER_AGENT, HeaderValue::from_str(ua).expect("valid ua"));
    h
}

fn model_header(h: &HeaderMap) -> Option<String> {
    h.get(GATE_MODEL_HEADER_NAME)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

#[test]
fn a_gate_choice_is_sent_for_the_tool_that_made_the_request() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();
    preferences::set_tool_model(
        "claude-code",
        ModelSource::Gate,
        vec!["anthropic/claude-opus-5".into()],
        true,
    )
    .expect("save");

    let mut h = headers("claude-cli/1.2.3 (darwin)");
    inject_attribution_for_tests(&mut h);

    assert_eq!(model_header(&h).as_deref(), Some("anthropic/claude-opus-5"));
}

#[test]
fn a_set_is_sent_whole_and_in_the_users_order() {
    // AG-590 enables several; which one a request uses is the gateway's rule.
    // Sending the set means that rule can land without changing the protocol.
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();
    preferences::set_tool_model(
        "codex",
        ModelSource::Gate,
        vec!["openai/gpt-5".into(), "anthropic/claude-haiku-4-5".into()],
        true,
    )
    .expect("save");

    let mut h = headers("codex/0.1");
    inject_attribution_for_tests(&mut h);

    assert_eq!(
        model_header(&h).as_deref(),
        Some("openai/gpt-5,anthropic/claude-haiku-4-5")
    );
}

#[test]
fn a_remembered_model_under_the_tools_own_default_is_not_sent() {
    // The pane keeps a chosen model visible while the tool picks its own, so the
    // user can see what they would switch to. Sending it would serve - and bill
    // for - a model they explicitly did not switch to.
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();
    preferences::set_tool_model(
        "claude-code",
        ModelSource::Tool,
        vec!["anthropic/claude-opus-5".into()],
        false,
    )
    .expect("save");

    let mut h = headers("claude-cli/1.2.3");
    inject_attribution_for_tests(&mut h);

    assert_eq!(model_header(&h), None);
}

#[test]
fn a_tool_cannot_choose_its_own_paid_model() {
    // The header is stripped before anything is added, so a tool that sets it
    // itself cannot pick a model the user never confirmed - and be billed for
    // it. This is the whole reason the strip is unconditional.
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();

    let mut h = headers("claude-cli/1.2.3");
    h.insert(
        GATE_MODEL_HEADER_NAME,
        HeaderValue::from_static("anthropic/claude-opus-5"),
    );
    inject_attribution_for_tests(&mut h);

    assert_eq!(
        model_header(&h),
        None,
        "a tool's own value must not survive"
    );
}

#[test]
fn an_unidentified_tool_sends_no_override() {
    // The slug is a User-Agent guess. Guessing wrong here would serve one tool's
    // chosen model to another and charge for it, so an unrecognised agent gets
    // no override at all - the same rule attribution follows, for a much larger
    // reason.
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();
    preferences::set_tool_model(
        "claude-code",
        ModelSource::Gate,
        vec!["anthropic/claude-opus-5".into()],
        true,
    )
    .expect("save");

    let mut h = headers("some-unknown-agent/9");
    inject_attribution_for_tests(&mut h);

    assert_eq!(model_header(&h), None);
}

#[test]
fn a_tool_with_no_choice_is_left_alone() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();
    preferences::set_tool_model(
        "codex",
        ModelSource::Gate,
        vec!["openai/gpt-5".into()],
        true,
    )
    .expect("save");

    let mut h = headers("claude-cli/1.2.3");
    inject_attribution_for_tests(&mut h);

    assert_eq!(
        model_header(&h),
        None,
        "codex's choice is not claude-code's"
    );
}

#[test]
fn a_change_takes_effect_without_a_restart() {
    // The read is cached off the request path; a choice that only applied after
    // a restart would be its own bug report.
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();

    let mut first = headers("claude-cli/1.2.3");
    inject_attribution_for_tests(&mut first);
    assert_eq!(model_header(&first), None);

    preferences::set_tool_model(
        "claude-code",
        ModelSource::Gate,
        vec!["anthropic/claude-opus-5".into()],
        true,
    )
    .expect("save");

    let mut second = headers("claude-cli/1.2.3");
    inject_attribution_for_tests(&mut second);
    assert_eq!(
        model_header(&second).as_deref(),
        Some("anthropic/claude-opus-5")
    );
}

#[test]
fn a_choice_written_by_another_process_is_served_without_a_restart() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempHome::set();
    preferences::set_tool_model(
        "claude-code",
        ModelSource::Gate,
        vec!["anthropic/claude-opus-5".into()],
        true,
    )
    .expect("save");

    let mut h = headers("claude-cli/1.2.3 (darwin)");
    inject_attribution_for_tests(&mut h);
    assert_eq!(model_header(&h).as_deref(), Some("anthropic/claude-opus-5"));

    // What the window's write looks like from inside the Linux helper daemon:
    // the file changes and this process's `save` was never called, so nothing
    // in here had a chance to refresh a cache. The daemon owns the engine and
    // outlives the GUI, so a cache only `save` could invalidate would keep
    // serving the old model until logout.
    let path = tmp.dir.join("preferences.json");
    let raw = std::fs::read_to_string(&path).expect("read preferences");
    std::fs::write(
        &path,
        raw.replace("anthropic/claude-opus-5", "anthropic/claude-sonnet-5"),
    )
    .expect("write preferences behind the cache's back");

    let mut h = headers("claude-cli/1.2.3 (darwin)");
    inject_attribution_for_tests(&mut h);
    assert_eq!(
        model_header(&h).as_deref(),
        Some("anthropic/claude-sonnet-5")
    );
}

/// The serve decision, and what it takes with it.
///
/// A tool set to a Gate model routes differently: no upstream hint, so the
/// gateway resolves a provider and bills credits instead of forwarding to the
/// tool's own. These pin the two halves of that - the signal both proxy paths
/// branch on, and the credential a served request stops carrying.
#[test]
fn a_gate_model_marks_the_request_as_one_gate_serves() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();
    preferences::set_tool_model(
        "claude-code",
        ModelSource::Gate,
        vec!["anthropic/claude-opus-5".into()],
        true,
    )
    .expect("save");

    let mut h = headers("claude-cli/1.2.3");
    inject_attribution_for_tests(&mut h);

    assert!(
        serves_gate_model(&h),
        "a stored Gate model is what tells both proxy paths to withhold the upstream hint"
    );
}

#[test]
fn the_tools_own_default_leaves_the_request_forwarded_as_before() {
    // The default path must be untouched by any of this: no Gate model means the
    // upstream hint still goes, and the tool's own credential still pays.
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();

    let mut h = headers("claude-cli/1.2.3");
    inject_attribution_for_tests(&mut h);

    assert!(!serves_gate_model(&h));
}

#[test]
fn a_served_request_drops_the_tools_credential_from_both_slots() {
    // OpenAI-shaped APIs authenticate on `Authorization`, Anthropic on
    // `x-api-key`. A served request is paid for by Gate, so neither is needed -
    // and not sending a credential is better than trusting the far side to keep
    // discarding it.
    let mut h = headers("claude-cli/1.2.3");
    h.insert(
        hyper::header::AUTHORIZATION,
        HeaderValue::from_static("Bearer sk-ant-oat-secret"),
    );
    h.insert(
        hyper::header::HeaderName::from_static("x-api-key"),
        HeaderValue::from_static("sk-ant-secret"),
    );

    strip_tool_credential_for_tests(&mut h);

    assert_eq!(h.get(hyper::header::AUTHORIZATION), None);
    assert_eq!(h.get("x-api-key"), None);
    // The attribution headers are not credentials and stay.
    assert!(h.contains_key(hyper::header::USER_AGENT));
}
