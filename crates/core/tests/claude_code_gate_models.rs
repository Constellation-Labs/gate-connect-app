//! Gate models, written into Claude Code's own `settings.json`.
//!
//! Gate sets the default `model`, a `modelPicker` that replaces the built-in
//! lineup with the enabled set, every model tier Claude Code picks on its own,
//! and `ANTHROPIC_BASE_URL` on the relay's Gate models route. What is pinned:
//!
//! - the shape, including the tiers that would otherwise send `claude-haiku-*`
//!   and friends to a route that refuses them;
//! - App default and disconnect leave `settings.json` exactly as the same steps
//!   would have without Gate models, keeping the user's own keys;
//! - a model the user picks in Claude Code outside the set puts it back on its
//!   own model and is kept; the picker's Default row is not drift.
//!
//! Hermetic, its own binary: temp `$HOME` and `GATE_CONNECT_TEST_HOME`, an
//! in-memory keychain.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use gate_connect_core::account::{self, BillingMode};
use gate_connect_core::preferences::{self, ModelSource};
use gate_connect_core::registry::{find, ConnectInput, GateModelState, ToolId};
use gate_connect_core::{env, keychain, tool_models};
use serde_json::Value;

static LOCK: Mutex<()> = Mutex::new(());
const PORT: u16 = 9977;
const OPUS: &str = "anthropic/claude-opus-5";
const LUNA: &str = "openai/gpt-5.6-luna";

struct TempHome {
    dir: PathBuf,
    prev_home: Option<String>,
}

impl TempHome {
    fn set() -> Self {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "gate-connect-claude-gate-models-{}-{n}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &dir);
        std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        preferences::reset_cache_for_tests();
        TempHome { dir, prev_home }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        match self.prev_home.take() {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        std::env::remove_var("GATE_CONNECT_TEST_HOME");
        preferences::reset_cache_for_tests();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// The user's own settings: a permission rule and an env value Gate never
/// touches, plus a model they had picked.
const ORIGINAL: &str = r#"{
  "model": "opus",
  "permissions": {
    "allow": ["Bash(ls:*)"]
  },
  "env": {
    "MY_VAR": "1"
  }
}
"#;

fn setup() -> TempHome {
    let home = TempHome::set();
    let proxy = env::app_support_dir().unwrap().join("proxy");
    fs::create_dir_all(&proxy).unwrap();
    fs::write(proxy.join("relay-port"), PORT.to_string()).unwrap();
    fs::write(
        proxy.join("ca-cert.pem"),
        "-----BEGIN CERTIFICATE-----\ngate-test-ca\n-----END CERTIFICATE-----\n",
    )
    .unwrap();
    let settings = env::claude_code_settings_path().unwrap();
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, ORIGINAL).unwrap();
    keychain::use_in_memory_backend();
    account::save("https://gw.example.com", Some("sk-gw-test")).unwrap();
    account::set_billing_mode(BillingMode::Byok).unwrap();
    home
}

fn input() -> ConnectInput {
    ConnectInput {
        gateway_base_url: "https://gw.example.com".to_string(),
        billing_mode: BillingMode::Byok,
        relay_base_url: Some(format!("http://127.0.0.1:{PORT}")),
        engine_proxy_url: Some(format!("http://127.0.0.1:{PORT}")),
    }
}

fn raw() -> String {
    fs::read_to_string(env::claude_code_settings_path().unwrap()).unwrap()
}

fn json() -> Value {
    serde_json::from_str(&raw()).unwrap()
}

fn choose_gate(ids: &[&str]) {
    let ids: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
    preferences::set_tool_model("claude-code", ModelSource::Gate, ids, true, vec![]).unwrap();
}

fn choose_tool() {
    let ids = preferences::gate_models_for("claude-code").unwrap_or_default();
    preferences::set_tool_model("claude-code", ModelSource::Tool, ids, false, vec![]).unwrap();
}

#[test]
fn gate_models_are_claude_codes_model_picker_and_every_tier() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = setup();
    let claude = find(ToolId::ClaudeCode).unwrap();
    choose_gate(&[OPUS, LUNA]);
    claude.connect(&input()).unwrap();

    let s = json();
    assert_eq!(s["model"], OPUS);
    let env = &s["env"];
    assert_eq!(
        env["ANTHROPIC_BASE_URL"],
        format!("http://127.0.0.1:{PORT}/__gate/t/claude-code/gate").as_str(),
        "the SDK appends /v1 itself"
    );
    for tier in [
        "ANTHROPIC_DEFAULT_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_DEFAULT_FABLE_MODEL",
        "ANTHROPIC_SMALL_FAST_MODEL",
    ] {
        assert_eq!(env[tier], OPUS, "{tier} must be an enabled model");
    }
    assert_eq!(env["CLAUDE_CODE_SUBAGENT_MODEL"], "inherit");
    assert_eq!(env["CLAUDE_CODE_SUBAGENT_MODEL_FORCE"], "1");
    assert_eq!(env["CLAUDE_CODE_NO_MODEL_FALLBACK"], "1");
    assert!(env.get("ANTHROPIC_MODEL").is_none(), "it would mask /model");
    assert_eq!(env["MY_VAR"], "1");
    assert!(env["HTTPS_PROXY"]
        .as_str()
        .unwrap()
        .contains("gate-claude-code"));
    let picker = &s["modelPicker"];
    assert_eq!(picker["replaceBuiltInOptions"], true);
    assert_eq!(picker["options"][0]["model"], OPUS);
    assert_eq!(picker["options"][1]["model"], LUNA);
    assert_eq!(s["permissions"]["allow"][0], "Bash(ls:*)");

    assert_eq!(
        claude.gate_model_state().unwrap(),
        GateModelState::Applied { model: OPUS.into() }
    );
}

#[test]
fn app_default_and_disconnect_leave_settings_as_if_gate_models_never_ran() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = setup();
    let claude = find(ToolId::ClaudeCode).unwrap();

    claude.connect(&input()).unwrap();
    let connected_plain = raw();
    claude.disconnect().unwrap();
    let disconnected_plain = raw();
    fs::write(env::claude_code_settings_path().unwrap(), ORIGINAL).unwrap();

    claude.connect(&input()).unwrap();
    choose_gate(&[OPUS]);
    claude.connect(&input()).unwrap();
    choose_tool();
    claude.connect(&input()).unwrap();
    assert_eq!(raw(), connected_plain);
    assert_eq!(
        claude.gate_model_state().unwrap(),
        GateModelState::NotApplied
    );

    choose_gate(&[OPUS]);
    claude.connect(&input()).unwrap();
    claude.disconnect().unwrap();
    assert_eq!(raw(), disconnected_plain);
    assert_eq!(json()["model"], "opus", "the user's own model is back");
}

#[test]
fn a_model_typed_into_claude_code_outside_the_set_is_kept() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = setup();
    let claude = find(ToolId::ClaudeCode).unwrap();
    choose_gate(&[OPUS]);
    claude.connect(&input()).unwrap();

    // What `/model claude-sonnet-5` persists.
    let mut s = json();
    s["model"] = Value::String("claude-sonnet-5".into());
    fs::write(
        env::claude_code_settings_path().unwrap(),
        serde_json::to_string_pretty(&s).unwrap(),
    )
    .unwrap();

    let view = tool_models::states().remove("claude-code").unwrap();
    assert_eq!(view.left_gate_models, Some(Some("claude-sonnet-5".into())));
    assert_eq!(
        preferences::load().tool_models["claude-code"].source,
        ModelSource::Tool
    );
    let s = json();
    assert_eq!(s["model"], "claude-sonnet-5", "the user's pick stays");
    assert!(s.get("modelPicker").is_none(), "Gate's lineup goes");
    assert!(
        s["env"].get("ANTHROPIC_BASE_URL").is_none(),
        "back on Anthropic's URL"
    );
    assert!(s["env"].get("ANTHROPIC_DEFAULT_HAIKU_MODEL").is_none());
    assert!(s["env"]["HTTPS_PROXY"].as_str().is_some(), "routing stays");
}

#[test]
fn the_default_row_is_not_drift() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = setup();
    let claude = find(ToolId::ClaudeCode).unwrap();
    choose_gate(&[OPUS, LUNA]);
    claude.connect(&input()).unwrap();

    // Picking "Default" clears `model`; it resolves to the pinned default.
    let mut s = json();
    s.as_object_mut().unwrap().remove("model");
    fs::write(
        env::claude_code_settings_path().unwrap(),
        serde_json::to_string_pretty(&s).unwrap(),
    )
    .unwrap();
    assert_eq!(
        claude.gate_model_state().unwrap(),
        GateModelState::Applied { model: OPUS.into() }
    );
}
