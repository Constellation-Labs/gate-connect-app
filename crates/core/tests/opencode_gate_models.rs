//! Gate models, written into OpenCode's own `opencode.json`.
//!
//! Gate adds a `provider.gate-connect` entry - the OpenAI-compatible package on
//! the relay's Gate models route, the enabled set under `models`, a placeholder
//! key the relay strips - and selects it with the top-level `model`. What is
//! pinned here:
//!
//! - the shape Gate writes, beside the providers Gate routes;
//! - App default and disconnect leave `opencode.json` exactly as the same steps
//!   would have without Gate models, down to a file that did not exist;
//! - an OpenCode on Gate models needs no provider of the user's, and one with
//!   neither is refused, on either billing mode;
//! - a default the user moves off the set is OpenCode's choice, and a pick from
//!   the set is not drift and survives a reconnect.
//!
//! Hermetic, its own binary: temp `$HOME`, XDG dirs and
//! `GATE_CONNECT_TEST_HOME`, an in-memory keychain.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use gate_connect_core::account::{self, BillingMode};
use gate_connect_core::preferences::{self, ModelSource};
use gate_connect_core::registry::{find, ConnectInput, GateModelState, ToolId};
use gate_connect_core::{env, keychain, tool_models};
use serde_json::{json, Value};

static LOCK: Mutex<()> = Mutex::new(());
const PORT: u16 = 9977;

const ENV_VARS: &[&str] = &[
    "HOME",
    "GATE_CONNECT_TEST_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "OPENCODE_CONFIG",
    "OPENCODE_CONFIG_DIR",
    "OPENCODE_CONFIG_CONTENT",
];

struct TempHome {
    dir: PathBuf,
    prev: Vec<(&'static str, Option<String>)>,
}

impl TempHome {
    fn set() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "gate-connect-opencode-gate-models-{}-{}",
            std::process::id(),
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        let prev = ENV_VARS
            .iter()
            .map(|k| (*k, std::env::var(k).ok()))
            .collect();
        for k in ENV_VARS {
            std::env::remove_var(k);
        }
        std::env::set_var("HOME", &dir);
        std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);
        std::env::set_var("XDG_CONFIG_HOME", dir.join(".config"));
        std::env::set_var("XDG_DATA_HOME", dir.join(".local/share"));
        preferences::reset_cache_for_tests();
        TempHome { dir, prev }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        for (k, v) in self.prev.drain(..) {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
        preferences::reset_cache_for_tests();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

const LUNA: &str = "openai/gpt-5.6-luna";
const OPUS: &str = "anthropic/claude-opus-5";

/// A user's config: their own default, a provider Gate routes, and settings
/// Gate has no business with.
const ORIGINAL: &str = r#"{
  "$schema": "https://opencode.ai/config.json",
  "model": "openrouter/z-ai/glm-5.2",
  "provider": {
    "openrouter": { "options": { "apiKey": "user-openrouter-key" } }
  },
  "autoupdate": false
}
"#;

fn setup(config: Option<&str>, mode: BillingMode) -> TempHome {
    let home = TempHome::set();
    let proxy = env::app_support_dir().unwrap().join("proxy");
    fs::create_dir_all(&proxy).unwrap();
    fs::write(proxy.join("relay-port"), PORT.to_string()).unwrap();
    // An installed OpenCode, so a config that names no provider still counts:
    // `~/.local/bin` off Windows, npm's shim on it.
    for bin in [
        env::home().unwrap().join(".local/bin/opencode"),
        home.dir
            .join("AppData")
            .join("Roaming")
            .join("npm")
            .join("opencode.cmd"),
    ] {
        fs::create_dir_all(bin.parent().unwrap()).unwrap();
        fs::write(&bin, "").unwrap();
    }
    if let Some(body) = config {
        let cfg = env::opencode_config_path().unwrap();
        fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        fs::write(&cfg, body).unwrap();
    }
    keychain::use_in_memory_backend();
    account::save("https://gw.example.com", Some("sk-gw-test")).unwrap();
    account::set_billing_mode(mode).unwrap();
    home
}

fn input(mode: BillingMode) -> ConnectInput {
    ConnectInput {
        gateway_base_url: "https://gw.example.com".to_string(),
        billing_mode: mode,
        relay_base_url: Some(format!("http://127.0.0.1:{PORT}")),
        engine_proxy_url: Some(format!("http://127.0.0.1:{PORT}")),
    }
}

fn config_path() -> PathBuf {
    env::opencode_config_path().unwrap()
}

fn json() -> Value {
    serde_json::from_str(&fs::read_to_string(config_path()).unwrap()).unwrap()
}

fn write_json(v: &Value) {
    fs::write(config_path(), serde_json::to_string_pretty(v).unwrap()).unwrap();
}

fn original() -> Value {
    serde_json::from_str(ORIGINAL).unwrap()
}

fn sidecar() -> PathBuf {
    env::app_support_dir().unwrap().join("opencode-state.json")
}

fn choose_gate(ids: &[&str]) {
    let ids: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
    preferences::set_tool_model("opencode", ModelSource::Gate, ids, true, vec![]).unwrap();
}

fn choose_tool() {
    let ids = preferences::gate_models_for("opencode").unwrap_or_default();
    preferences::set_tool_model("opencode", ModelSource::Tool, ids, false, vec![]).unwrap();
}

fn served() -> String {
    format!("http://127.0.0.1:{PORT}/__gate/t/opencode/gate/v1")
}

#[test]
fn gate_models_are_a_provider_of_gates_beside_the_users() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = setup(Some(ORIGINAL), BillingMode::Byok);
    let opencode = find(ToolId::OpenCode).unwrap();
    assert!(opencode.supports_gate_models());
    choose_gate(&[LUNA, OPUS]);
    opencode.connect(&input(BillingMode::Byok)).unwrap();

    let j = json();
    assert_eq!(j["model"], format!("gate-connect/{LUNA}"));
    let p = &j["provider"]["gate-connect"];
    assert_eq!(p["npm"], "@ai-sdk/openai-compatible");
    assert_eq!(p["name"], "Gate Connect");
    assert_eq!(p["options"]["baseURL"], served());
    assert_eq!(p["options"]["apiKey"], "billed-through-gate");
    let models: Vec<&String> = p["models"].as_object().unwrap().keys().collect();
    assert_eq!(models.len(), 2);
    assert!(p["models"].get(LUNA).is_some() && p["models"].get(OPUS).is_some());
    // The user's provider is still routed, and their other settings stay.
    assert_eq!(
        j["provider"]["openrouter"]["options"]["baseURL"],
        format!("http://127.0.0.1:{PORT}/__gate/t/opencode/openrouter/v1")
    );
    assert_eq!(
        j["provider"]["openrouter"]["options"]["apiKey"],
        "user-openrouter-key"
    );
    assert_eq!(j["autoupdate"], false);

    assert_eq!(
        opencode.gate_model_state().unwrap(),
        GateModelState::Applied { model: LUNA.into() }
    );
}

#[test]
fn the_catalogue_names_and_limits_reach_the_model_picker() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = setup(Some(ORIGINAL), BillingMode::Byok);
    let opencode = find(ToolId::OpenCode).unwrap();
    let meta = preferences::GateModelMeta {
        name: Some("GPT-5.6 Luna".into()),
        context_window: Some(400_000),
        max_tokens: Some(128_000),
        ..Default::default()
    };
    preferences::set_tool_model(
        "opencode",
        ModelSource::Gate,
        vec![LUNA.into(), OPUS.into()],
        true,
        vec![(LUNA.into(), meta)],
    )
    .unwrap();
    opencode.connect(&input(BillingMode::Byok)).unwrap();

    let models = &json()["provider"]["gate-connect"]["models"];
    assert_eq!(
        models[LUNA],
        json!({ "name": "GPT-5.6 Luna", "limit": { "context": 400000, "output": 128000 } })
    );
    assert_eq!(models[OPUS], json!({}), "nothing known, nothing invented");
}

#[test]
fn app_default_and_disconnect_leave_config_as_if_gate_models_never_ran() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = setup(Some(ORIGINAL), BillingMode::Byok);
    let opencode = find(ToolId::OpenCode).unwrap();

    opencode.connect(&input(BillingMode::Byok)).unwrap();
    let connected_plain = json();

    choose_gate(&[LUNA]);
    opencode.connect(&input(BillingMode::Byok)).unwrap();
    assert_ne!(json(), connected_plain);
    choose_tool();
    opencode.connect(&input(BillingMode::Byok)).unwrap();
    assert_eq!(json(), connected_plain);
    assert_eq!(
        opencode.gate_model_state().unwrap(),
        GateModelState::NotApplied
    );

    choose_gate(&[LUNA]);
    opencode.connect(&input(BillingMode::Byok)).unwrap();
    opencode.disconnect().unwrap();
    assert_eq!(json(), original(), "disconnect takes everything back out");
    assert!(!sidecar().exists());
}

#[test]
fn the_object_form_of_model_comes_back_as_written() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut before = original();
    before["model"] = json!({ "providerID": "openrouter", "model": "z-ai/glm-5.2" });
    let _home = setup(Some(&before.to_string()), BillingMode::Byok);
    let opencode = find(ToolId::OpenCode).unwrap();
    choose_gate(&[LUNA]);
    opencode.connect(&input(BillingMode::Byok)).unwrap();
    assert_eq!(json()["model"], format!("gate-connect/{LUNA}"));
    opencode.disconnect().unwrap();
    assert_eq!(json(), before);
}

/// A fresh OpenCode Desktop names no provider. On Gate models it needs none:
/// the Gate provider is the whole config, on either billing mode.
#[test]
fn gate_models_need_no_provider_of_the_users() {
    for mode in [BillingMode::Byok, BillingMode::Payg] {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _home = setup(None, mode);
        let opencode = find(ToolId::OpenCode).unwrap();
        choose_gate(&[OPUS]);
        opencode.connect(&input(mode)).unwrap();

        let j = json();
        let providers: Vec<&String> = j["provider"].as_object().unwrap().keys().collect();
        assert_eq!(providers, ["gate-connect"], "{mode:?}: {j}");
        assert_eq!(j["model"], format!("gate-connect/{OPUS}"));

        opencode.disconnect().unwrap();
        assert!(
            !config_path().exists(),
            "{mode:?}: a file Gate created is removed"
        );
        assert!(!sidecar().exists());
    }
}

/// Off Gate models, an OpenCode with no provider of the user's has nothing to
/// route on either billing mode: pay-as-you-go adds no providers of Gate's own.
/// Connect refuses with the remedy, and writes nothing.
#[test]
fn no_provider_and_no_gate_models_is_refused_on_either_billing_mode() {
    for mode in [BillingMode::Byok, BillingMode::Payg] {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _home = setup(None, mode);
        let opencode = find(ToolId::OpenCode).unwrap();
        let err = opencode.connect(&input(mode)).unwrap_err();
        assert!(
            format!("{err:#}").starts_with("Sign in to a provider in OpenCode first"),
            "{mode:?}: {err:#}"
        );
        assert!(!config_path().exists(), "{mode:?}");
        assert!(!sidecar().exists(), "{mode:?}");
    }
}

/// BYOK with nothing of the user's and Gate models just left: connect refuses,
/// and Gate's provider still comes off the disk.
#[test]
fn leaving_gate_models_with_nothing_else_to_route_leaves_no_residue() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = setup(None, BillingMode::Byok);
    let opencode = find(ToolId::OpenCode).unwrap();
    choose_gate(&[LUNA]);
    opencode.connect(&input(BillingMode::Byok)).unwrap();

    choose_tool();
    assert!(opencode.connect(&input(BillingMode::Byok)).is_err());
    assert!(!config_path().exists());
    assert!(!sidecar().exists());
}

#[test]
fn moving_the_default_off_the_set_puts_opencode_back_on_its_own_model() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = setup(Some(ORIGINAL), BillingMode::Byok);
    let opencode = find(ToolId::OpenCode).unwrap();
    choose_gate(&[LUNA]);
    opencode.connect(&input(BillingMode::Byok)).unwrap();

    let mut edited = json();
    edited["model"] = json!("anthropic/claude-sonnet-5");
    write_json(&edited);
    assert_eq!(
        opencode.gate_model_state().unwrap(),
        GateModelState::Drifted {
            model: Some("anthropic/claude-sonnet-5".into())
        }
    );

    let view = tool_models::states()
        .remove("opencode")
        .expect("opencode has a card");
    assert_eq!(
        view.left_gate_models,
        Some(Some("anthropic/claude-sonnet-5".into()))
    );
    assert_eq!(
        preferences::load().tool_models["opencode"].source,
        ModelSource::Tool
    );

    let j = json();
    assert_eq!(
        j["model"], "anthropic/claude-sonnet-5",
        "the user's pick stays"
    );
    assert!(j["provider"].get("gate-connect").is_none(), "{j}");
    assert!(
        j["provider"]["openrouter"]["options"]["baseURL"]
            .as_str()
            .unwrap()
            .contains("/__gate/t/opencode/openrouter/v1"),
        "routing stays: {j}"
    );

    // A reconnect writes nothing of Gate models back.
    opencode.connect(&input(BillingMode::Byok)).unwrap();
    assert!(json()["provider"].get("gate-connect").is_none());
    assert_eq!(json()["model"], "anthropic/claude-sonnet-5");
}

#[test]
fn a_pick_from_the_set_is_not_drift_and_survives_a_reconnect() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = setup(Some(ORIGINAL), BillingMode::Byok);
    let opencode = find(ToolId::OpenCode).unwrap();
    choose_gate(&[LUNA, OPUS]);
    opencode.connect(&input(BillingMode::Byok)).unwrap();

    let mut edited = json();
    edited["model"] = json!(format!("gate-connect/{OPUS}"));
    write_json(&edited);
    assert_eq!(
        opencode.gate_model_state().unwrap(),
        GateModelState::Applied { model: OPUS.into() }
    );
    opencode.connect(&input(BillingMode::Byok)).unwrap();
    assert_eq!(json()["model"], format!("gate-connect/{OPUS}"));
}

/// A `gate-connect` provider of the user's own is not Gate's to replace: the
/// choice is refused and the file is left exactly as it was.
#[test]
fn a_users_own_gate_connect_provider_is_refused_and_left_alone() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut before = original();
    before["provider"]["gate-connect"] =
        json!({ "options": { "baseURL": "https://gate.example.com/v1" } });
    let _home = setup(Some(&before.to_string()), BillingMode::Byok);
    let opencode = find(ToolId::OpenCode).unwrap();
    choose_gate(&[LUNA]);
    let err = opencode.connect(&input(BillingMode::Byok)).unwrap_err();
    assert!(err.to_string().contains("gate-connect"), "{err:#}");
    assert_eq!(json(), before);
}
