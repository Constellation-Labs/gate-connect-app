//! The per-tool model choice, as it is actually stored (AG-588).
//!
//! An integration test, not a unit test, and the reason is the same one
//! `account_reconcile.rs` gives: these touch the real preferences file, which
//! means redirecting the app-support directory - and that override is
//! process-global. Inside the lib's unit tests it is visible to every other test
//! running on a sibling thread, which is not hypothetical: it made
//! `proxy::a_snapshot_without_a_listener_is_not_a_hosted_engine` fail while
//! passing on its own. Its own binary means its own process.
//!
//! What is under test is the pair of rules that decide whether someone gets
//! billed: consent is recorded only when Gate is actually asked to serve a
//! model, and the record of when they agreed cannot be moved afterwards.

use std::path::PathBuf;
use std::sync::Mutex;

use gate_connect_core::env;
use gate_connect_core::preferences::{load, set_tool_model, ModelSource};

/// The override is process-global even here, so these take turns.
static LOCK: Mutex<()> = Mutex::new(());

/// Point the app-support dir at a fresh temp dir, and clear it afterwards -
/// otherwise one test's file is the next one's starting state.
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
        let dir = std::env::temp_dir().join(format!("gc-tool-models-{n}"));
        std::fs::create_dir_all(&dir).expect("create temp app-support dir");
        env::set_app_support_dir_for_tests(Some(dir.clone()));
        Self { dir }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        env::set_app_support_dir_for_tests(None);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The acknowledgement is a record of when someone agreed to be billed. A later
/// save must not be able to move it, or the record is worthless.
#[test]
fn the_paid_acknowledgement_is_stamped_once_and_never_moved() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();

    set_tool_model(
        "claude-code",
        ModelSource::Gate,
        vec!["anthropic/claude-opus-5".into()],
        true,
    )
    .expect("first save");
    let first = load().gate_model_paid_ack_unix.expect("stamped");

    set_tool_model(
        "codex",
        ModelSource::Gate,
        vec!["openai/gpt-5".into()],
        true,
    )
    .expect("second save");
    assert_eq!(load().gate_model_paid_ack_unix, Some(first));
}

/// Nothing is billed for remembering a model under the tool's own default, so
/// nothing there may record consent to be billed. Without this, browsing the
/// picker would silently spend the one confirmation the user gets.
#[test]
fn choosing_the_tools_own_default_never_records_consent() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();

    set_tool_model(
        "claude-code",
        ModelSource::Tool,
        vec!["anthropic/claude-opus-5".into()],
        true,
    )
    .expect("save");

    let prefs = load();
    assert_eq!(prefs.gate_model_paid_ack_unix, None);
    // The model is still remembered - declining to spend is not declining to
    // choose, and the pane shows what would be switched to.
    assert_eq!(
        prefs.tool_models["claude-code"].model_ids,
        vec!["anthropic/claude-opus-5".to_string()]
    );
}

/// One tool's choice must not disturb another's, nor an unrelated preference:
/// the setter is a read-modify-write over one shared file.
#[test]
fn setting_one_tool_leaves_the_others_alone() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();

    set_tool_model("claude-code", ModelSource::Gate, vec!["a/b".into()], true).expect("first");
    set_tool_model("codex", ModelSource::Tool, vec![], false).expect("second");

    let prefs = load();
    assert_eq!(prefs.tool_models["claude-code"].source, ModelSource::Gate);
    assert_eq!(prefs.tool_models["codex"].source, ModelSource::Tool);
    assert!(prefs.share_diagnostics);
}

/// A tool nobody has configured is absent, not defaulted. The pane reads an
/// absent key as "the tool picks its own model", which is the true default;
/// writing a placeholder would make an untouched tool indistinguishable from one
/// somebody deliberately set to its own default.
#[test]
fn an_untouched_tool_has_no_entry() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();

    set_tool_model("codex", ModelSource::Gate, vec!["a/b".into()], true).expect("save");
    assert!(!load().tool_models.contains_key("claude-code"));
}
