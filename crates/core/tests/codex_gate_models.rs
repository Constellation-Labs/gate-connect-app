//! Gate models, written into Codex's own config.
//!
//! The chosen models are Codex's default `model`, its whole model picker (a
//! `model_catalog_json` Gate writes, which replaces Codex's remote catalog),
//! and a provider `base_url` on the relay's Gate models route. What is pinned
//! here:
//!
//! - the shape Gate writes, under a BYOK account (the case where it differs
//!   most from the plain connect);
//! - going back to App default, and disconnecting, leave the file exactly as
//!   the same steps would have without Gate models;
//! - a model the user picks inside Codex is Codex's choice: one outside the set
//!   puts Codex back on its own model and is kept, one inside the set survives
//!   a reconnect.
//!
//! Hermetic in the same way as `codex_billing_mode.rs`: a temp `$HOME` and
//! `GATE_CONNECT_TEST_HOME`, an in-memory keychain, and its own binary.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

mod common;

use common::RelayStub;
use gate_connect_core::account::{self, BillingMode};
use gate_connect_core::preferences::{self, GateModelMeta, ModelSource};
use gate_connect_core::registry::{find, ConnectInput, GateModelState, Status, ToolId};
use gate_connect_core::{env, keychain, tool_models};

static LOCK: Mutex<()> = Mutex::new(());

struct TempHome {
    dir: PathBuf,
    prev_home: Option<String>,
}

impl TempHome {
    fn set() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "gate-connect-codex-gate-models-{}-{}",
            std::process::id(),
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &dir);
        std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);
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

const LUNA: &str = "openai/gpt-5.6-luna";
const OPUS: &str = "anthropic/claude-opus-5";

/// The user's own config before Gate touches it: a model, a setting Gate never
/// writes, and a comment `toml_edit` must keep.
const ORIGINAL: &str =
    "# my codex setup\nmodel = \"gpt-5.6-sol\"\nmodel_reasoning_effort = \"high\"\n";

/// Codex installed, logged in with ChatGPT, the relay up, a BYOK account.
fn setup() -> (TempHome, RelayStub) {
    let home = TempHome::set();
    let codex = env::home().unwrap().join(".codex");
    fs::create_dir_all(&codex).unwrap();
    fs::write(codex.join("auth.json"), r#"{"auth_mode":"chatgpt"}"#).unwrap();
    fs::write(env::codex_config_toml_path().unwrap(), ORIGINAL).unwrap();
    let stub = RelayStub::bind(0);
    let port_file = env::app_support_dir()
        .unwrap()
        .join("proxy")
        .join("relay-port");
    fs::create_dir_all(port_file.parent().unwrap()).unwrap();
    fs::write(&port_file, stub.port().to_string()).unwrap();
    keychain::use_in_memory_backend();
    account::save("https://gw.example.com", Some("sk-gw-test")).unwrap();
    account::set_billing_mode(BillingMode::Byok).unwrap();
    (home, stub)
}

fn input(stub: &RelayStub) -> ConnectInput {
    ConnectInput {
        gateway_base_url: "https://gw.example.com".to_string(),
        billing_mode: BillingMode::Byok,
        relay_base_url: Some(format!("http://127.0.0.1:{}", stub.port())),
        engine_proxy_url: Some("http://127.0.0.1:45999".to_string()),
    }
}

fn config() -> String {
    fs::read_to_string(env::codex_config_toml_path().unwrap()).unwrap()
}

fn doc() -> toml_edit::DocumentMut {
    config().parse().unwrap()
}

fn catalog_file() -> PathBuf {
    env::app_support_dir()
        .unwrap()
        .join("codex-gate-models.json")
}

fn choose_gate(ids: &[&str]) {
    let ids: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
    let meta = vec![(
        LUNA.to_string(),
        GateModelMeta {
            name: Some("GPT-5.6 Luna".into()),
            context_window: Some(400_000),
            max_tokens: None,
        },
    )];
    preferences::set_tool_model("codex", ModelSource::Gate, ids, true, meta).unwrap();
}

fn choose_tool() {
    let ids = preferences::gate_models_for("codex").unwrap_or_default();
    preferences::set_tool_model("codex", ModelSource::Tool, ids, false, vec![]).unwrap();
}

#[test]
fn gate_models_are_written_into_codex_config_and_picker() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (_home, stub) = setup();
    let codex = find(ToolId::Codex).unwrap();
    choose_gate(&[LUNA, OPUS]);

    codex.connect(&input(&stub)).unwrap();

    let d = doc();
    assert_eq!(
        d["model"].as_str(),
        Some(LUNA),
        "the first of the set is the default"
    );
    assert_eq!(d["model_provider"].as_str(), Some("gate"));
    let block = &d["model_providers"]["gate"];
    assert_eq!(
        block["base_url"].as_str(),
        Some(format!("http://127.0.0.1:{}/__gate/t/codex/gate/v1", stub.port()).as_str()),
        "the served route, not the ChatGPT passthrough"
    );
    assert!(
        block.get("requires_openai_auth").is_none(),
        "Gate is the provider, so Codex's own login authenticates nothing: {}",
        config()
    );
    assert_eq!(
        d["model_catalog_json"].as_str(),
        Some(catalog_file().display().to_string().as_str())
    );
    assert!(config().contains("model_reasoning_effort = \"high\""));
    assert!(config().starts_with("# my codex setup"), "comments survive");

    let catalog: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(catalog_file()).unwrap()).unwrap();
    let models = catalog["models"].as_array().unwrap();
    let slugs: Vec<&str> = models.iter().map(|m| m["slug"].as_str().unwrap()).collect();
    assert_eq!(
        slugs,
        [LUNA, OPUS],
        "the picker is the set, in the user's order"
    );
    assert_eq!(models[0]["display_name"], "GPT-5.6 Luna");
    assert_eq!(models[0]["context_window"], 400_000);
    assert_eq!(models[1]["display_name"], OPUS, "no catalogue name: the id");

    assert_eq!(
        codex.gate_model_state().unwrap(),
        GateModelState::Applied { model: LUNA.into() }
    );
    assert_eq!(codex.status().unwrap(), Status::Connected, "{}", config());
}

#[test]
fn app_default_and_disconnect_leave_the_file_as_if_gate_models_never_ran() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (_home, stub) = setup();
    let codex = find(ToolId::Codex).unwrap();

    // The reference: connect and disconnect with no Gate models.
    codex.connect(&input(&stub)).unwrap();
    let connected_plain = config();
    codex.disconnect().unwrap();
    let disconnected_plain = config();
    fs::write(env::codex_config_toml_path().unwrap(), ORIGINAL).unwrap();

    // App default after Gate models: back to the plain connected file.
    codex.connect(&input(&stub)).unwrap();
    choose_gate(&[LUNA]);
    codex.connect(&input(&stub)).unwrap();
    assert_ne!(config(), connected_plain);
    choose_tool();
    codex.connect(&input(&stub)).unwrap();
    assert_eq!(config(), connected_plain);
    assert!(
        !catalog_file().exists(),
        "the picker file goes with the choice"
    );
    assert_eq!(
        codex.gate_model_state().unwrap(),
        GateModelState::NotApplied
    );

    // Disconnect straight from Gate models: the plain disconnected file.
    choose_gate(&[LUNA]);
    codex.connect(&input(&stub)).unwrap();
    codex.disconnect().unwrap();
    assert_eq!(config(), disconnected_plain);
    assert!(!catalog_file().exists());
}

#[test]
fn a_model_chosen_inside_codex_outside_the_set_puts_it_back_on_its_own_model() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (_home, stub) = setup();
    let codex = find(ToolId::Codex).unwrap();
    choose_gate(&[LUNA]);
    codex.connect(&input(&stub)).unwrap();

    // The user edits Codex's model by hand, the way `/model` or a text editor
    // would leave it.
    let edited = config().replace(&format!("model = \"{LUNA}\""), "model = \"gpt-6-sol\"");
    fs::write(env::codex_config_toml_path().unwrap(), edited).unwrap();
    assert_eq!(
        codex.gate_model_state().unwrap(),
        GateModelState::Drifted {
            model: Some("gpt-6-sol".into())
        }
    );

    let view = tool_models::states()
        .remove("codex")
        .expect("codex has a card");
    assert_eq!(view.left_gate_models, Some(Some("gpt-6-sol".into())));
    assert_eq!(view.state, GateModelState::NotApplied);
    let stored = preferences::load().tool_models["codex"].clone();
    assert_eq!(
        stored.source,
        ModelSource::Tool,
        "stored, so no connect writes it back"
    );
    assert_eq!(
        stored.model_ids,
        [LUNA],
        "and the set is kept to offer again"
    );

    let d = doc();
    assert_eq!(
        d["model"].as_str(),
        Some("gpt-6-sol"),
        "the user's pick stays"
    );
    assert!(d.get("model_catalog_json").is_none(), "Gate's picker goes");
    let block = &d["model_providers"]["gate"];
    assert!(block["base_url"]
        .as_str()
        .unwrap()
        .ends_with("/__gate/t/codex/chatgpt/codex"));
    assert_eq!(block["requires_openai_auth"].as_bool(), Some(true));

    // And a later connect leaves it alone.
    codex.connect(&input(&stub)).unwrap();
    assert_eq!(doc()["model"].as_str(), Some("gpt-6-sol"));

    // Choosing Gate models again from Gate Connect applies them straight away:
    // the config is still Gate's to manage, only its model was the user's.
    assert!(codex.config_is_managed().unwrap(), "{}", config());
    let applied = tool_models::choose(
        ToolId::Codex,
        ModelSource::Gate,
        vec![LUNA.to_string()],
        true,
        vec![],
    )
    .unwrap();
    assert!(
        applied,
        "the reapply must reach a connected Codex: {}",
        config()
    );
    assert_eq!(doc()["model"].as_str(), Some(LUNA));
}

#[test]
fn a_model_chosen_inside_codex_from_the_set_survives_a_reconnect() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (_home, stub) = setup();
    let codex = find(ToolId::Codex).unwrap();
    choose_gate(&[LUNA, OPUS]);
    codex.connect(&input(&stub)).unwrap();

    let edited = config().replace(
        &format!("model = \"{LUNA}\""),
        &format!("model = \"{OPUS}\""),
    );
    fs::write(env::codex_config_toml_path().unwrap(), edited).unwrap();
    codex.connect(&input(&stub)).unwrap();

    assert_eq!(
        codex.gate_model_state().unwrap(),
        GateModelState::Applied { model: OPUS.into() }
    );
    let view = tool_models::states().remove("codex").unwrap();
    assert_eq!(
        view.left_gate_models, None,
        "a pick from the set is not drift"
    );

    // A new set from Gate Connect does move the default to its first entry.
    choose_gate(&[LUNA]);
    codex.connect(&input(&stub)).unwrap();
    assert_eq!(doc()["model"].as_str(), Some(LUNA));
}

#[test]
fn catalog_entries_are_cloned_from_what_codex_already_knows() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (_home, stub) = setup();
    let cache = env::home()
        .unwrap()
        .join(".codex")
        .join("models_cache.json");
    fs::write(
        &cache,
        r#"{"models":[
            {"slug":"gpt-hidden","visibility":"hide","priority":0,"base_instructions":"hidden"},
            {"slug":"gpt-6-sol","display_name":"Sol","visibility":"list","priority":1,
             "base_instructions":"You are Codex.","shell_type":"shell_command",
             "upgrade":{"model":"x"},"service_tiers":[],"context_window":272000}
        ]}"#,
    )
    .unwrap();
    choose_gate(&[OPUS]);
    find(ToolId::Codex).unwrap().connect(&input(&stub)).unwrap();

    let catalog: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(catalog_file()).unwrap()).unwrap();
    let entry = &catalog["models"][0];
    assert_eq!(entry["slug"], OPUS);
    assert_eq!(
        entry["base_instructions"], "You are Codex.",
        "Codex's own agent setup"
    );
    assert_eq!(entry["shell_type"], "shell_command");
    assert!(
        entry.get("upgrade").is_none(),
        "OpenAI-only offers are dropped"
    );
    assert!(entry.get("service_tiers").is_none());
    assert!(
        entry.get("context_window").is_none(),
        "the template's window belongs to another model"
    );
}

/// A user who only ever ran Codex on Gate models may never have logged Codex
/// in. Going back to App default must still take Gate's route out, or Codex is
/// left pointed at a route that refuses every request.
#[test]
fn app_default_does_not_need_a_codex_login() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (_home, stub) = setup();
    fs::remove_file(env::codex_auth_json_path().unwrap()).unwrap();
    let codex = find(ToolId::Codex).unwrap();
    choose_gate(&[LUNA]);
    codex.connect(&input(&stub)).unwrap();

    choose_tool();
    codex.connect(&input(&stub)).unwrap();

    let d = doc();
    assert_eq!(
        d["model"].as_str(),
        Some("gpt-5.6-sol"),
        "Codex's own model is back"
    );
    assert!(d.get("model_catalog_json").is_none());
    assert!(!d["model_providers"]["gate"]["base_url"]
        .as_str()
        .unwrap()
        .contains("/gate/v1"));
    assert_eq!(
        codex.gate_model_state().unwrap(),
        GateModelState::NotApplied
    );
}
