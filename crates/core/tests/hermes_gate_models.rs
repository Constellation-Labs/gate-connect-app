//! Gate models, written into Hermes' own `config.yaml`.
//!
//! Gate adds a `providers.gate-connect` entry - the relay's Gate models route,
//! chat completions, the enabled set as a fixed list with discovery off - and
//! selects it with `model.provider` / `model.default`. What is pinned here:
//!
//! - the shape Gate writes, and that the user's comments and keys survive it;
//! - App default and disconnect leave `config.yaml` exactly as the same steps
//!   would have without Gate models, including a fresh install's `model: ""`;
//! - a model the user picks inside Hermes is Hermes' choice: another provider
//!   puts Hermes back on its own model and is kept, a pick from the set is not
//!   drift and survives a reconnect.
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

static LOCK: Mutex<()> = Mutex::new(());
const PORT: u16 = 9977;

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
            "gate-connect-hermes-gate-models-{}-{}",
            std::process::id(),
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &dir);
        std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);
        std::env::remove_var("HERMES_HOME");
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

/// Close to what Hermes ships and this machine runs: comments, the model block
/// on OpenRouter, and unrelated sections after it.
const ORIGINAL: &str = "\
# Hermes agent configuration.
model:
  # Which upstream to talk to.
  default: z-ai/glm-5.2
  provider: openrouter
  base_url: https://openrouter.ai/api/v1
  api_mode: chat_completions

terminal:
  backend: local
_config_version: 45
";

fn setup(config: &str) -> TempHome {
    let home = TempHome::set();
    let proxy = env::app_support_dir().unwrap().join("proxy");
    fs::create_dir_all(&proxy).unwrap();
    fs::write(proxy.join("relay-port"), PORT.to_string()).unwrap();
    fs::write(proxy.join("engine-port"), PORT.to_string()).unwrap();
    fs::write(
        proxy.join("ca-cert.pem"),
        "-----BEGIN CERTIFICATE-----\ngate-test-ca\n-----END CERTIFICATE-----\n",
    )
    .unwrap();
    let launcher = env::home().unwrap().join(".local/bin/hermes");
    fs::create_dir_all(launcher.parent().unwrap()).unwrap();
    fs::write(&launcher, "#!/bin/sh\n").unwrap();
    let cfg = env::hermes_config_path().unwrap();
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    fs::write(&cfg, config).unwrap();
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

fn config() -> String {
    fs::read_to_string(env::hermes_config_path().unwrap()).unwrap()
}

fn write_config(body: &str) {
    fs::write(env::hermes_config_path().unwrap(), body).unwrap();
}

fn yaml() -> serde_yaml::Value {
    serde_yaml::from_str(&config()).unwrap()
}

fn choose_gate(ids: &[&str]) {
    let ids: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
    preferences::set_tool_model("hermes", ModelSource::Gate, ids, true, vec![]).unwrap();
}

fn choose_tool() {
    let ids = preferences::gate_models_for("hermes").unwrap_or_default();
    preferences::set_tool_model("hermes", ModelSource::Tool, ids, false, vec![]).unwrap();
}

#[test]
fn gate_models_are_hermes_provider_list_and_default() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = setup(ORIGINAL);
    let hermes = find(ToolId::Hermes).unwrap();
    choose_gate(&[LUNA, OPUS]);
    hermes.connect(&input()).unwrap();

    let y = yaml();
    assert_eq!(y["model"]["provider"], "gate-connect");
    assert_eq!(y["model"]["default"], LUNA);
    let served = format!("http://127.0.0.1:{PORT}/__gate/t/hermes/gate/v1");
    assert_eq!(
        y["model"]["base_url"],
        served.as_str(),
        "pointed at our provider, not left on OpenRouter: {}",
        config()
    );
    assert_eq!(y["model"]["api_mode"], "chat_completions");
    let p = &y["providers"]["gate-connect"];
    assert_eq!(p["name"], "Gate Connect");
    assert_eq!(
        p["api"],
        format!("http://127.0.0.1:{PORT}/__gate/t/hermes/gate/v1").as_str()
    );
    assert_eq!(p["api_mode"], "chat_completions");
    assert_eq!(p["discover_models"], false);
    assert_eq!(p["models"][0], LUNA);
    assert_eq!(p["models"][1], OPUS);
    assert_eq!(p["extra_headers"]["x-gate-tool"], "hermes");
    assert!(config().starts_with("# Hermes agent configuration.\nmodel:\n"));
    assert!(config().contains("  # Which upstream to talk to.\n"));
    assert!(config().contains("terminal:\n  backend: local\n"));

    assert_eq!(
        hermes.gate_model_state().unwrap(),
        GateModelState::Applied { model: LUNA.into() }
    );
}

#[test]
fn app_default_and_disconnect_leave_config_as_if_gate_models_never_ran() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = setup(ORIGINAL);
    let hermes = find(ToolId::Hermes).unwrap();

    hermes.connect(&input()).unwrap();
    let connected_plain = config();

    choose_gate(&[LUNA]);
    hermes.connect(&input()).unwrap();
    assert_ne!(config(), connected_plain);
    choose_tool();
    hermes.connect(&input()).unwrap();
    assert_eq!(config(), connected_plain);
    assert_eq!(
        hermes.gate_model_state().unwrap(),
        GateModelState::NotApplied
    );

    choose_gate(&[LUNA]);
    hermes.connect(&input()).unwrap();
    hermes.disconnect().unwrap();
    assert_eq!(config(), ORIGINAL, "disconnect takes everything back out");
}

#[test]
fn a_fresh_install_gets_its_empty_model_back() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let fresh = "model: \"\"\n_config_version: 45\n";
    let _home = setup(fresh);
    let hermes = find(ToolId::Hermes).unwrap();
    choose_gate(&[LUNA]);
    hermes.connect(&input()).unwrap();
    assert_eq!(yaml()["model"]["default"], LUNA);
    hermes.disconnect().unwrap();
    assert_eq!(config(), fresh);
}

#[test]
fn switching_provider_inside_hermes_puts_it_back_on_its_own_model() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = setup(ORIGINAL);
    let hermes = find(ToolId::Hermes).unwrap();
    choose_gate(&[LUNA]);
    hermes.connect(&input()).unwrap();

    // What `hermes model` writes on choosing a built-in provider.
    let served = format!("http://127.0.0.1:{PORT}/__gate/t/hermes/gate/v1");
    let edited = config()
        .replace("  provider: gate-connect\n", "  provider: anthropic\n")
        .replace(
            &format!("  base_url: {served}\n"),
            "  base_url: https://api.anthropic.com\n",
        )
        .replace(
            &format!("  default: {LUNA}\n"),
            "  default: claude-sonnet-5\n",
        );
    write_config(&edited);
    assert_eq!(
        hermes.gate_model_state().unwrap(),
        GateModelState::Drifted {
            model: Some("claude-sonnet-5".into())
        }
    );

    let view = tool_models::states()
        .remove("hermes")
        .expect("hermes has a card");
    assert_eq!(view.left_gate_models, Some(Some("claude-sonnet-5".into())));
    assert_eq!(
        preferences::load().tool_models["hermes"].source,
        ModelSource::Tool
    );

    let y = yaml();
    assert_eq!(
        y["model"]["provider"], "anthropic",
        "the user's provider stays"
    );
    assert_eq!(y["model"]["default"], "claude-sonnet-5");
    assert_eq!(y["model"]["base_url"], "https://api.anthropic.com");
    assert!(
        y.get("providers").is_none(),
        "Gate's provider goes, and the section it opened with it: {}",
        config()
    );
    hermes.connect(&input()).unwrap();
    assert_eq!(
        yaml()["model"]["default"],
        "claude-sonnet-5",
        "and stays gone"
    );
}

#[test]
fn a_pick_from_the_set_inside_hermes_is_not_drift() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = setup(ORIGINAL);
    let hermes = find(ToolId::Hermes).unwrap();
    choose_gate(&[LUNA, OPUS]);
    hermes.connect(&input()).unwrap();

    // What `hermes model` writes on choosing our provider: the slug form of
    // its name and the model. The route and wire it would write are ours already.
    let edited = config()
        .replace(
            "  provider: gate-connect\n",
            "  provider: custom:gate-connect\n",
        )
        .replace(
            &format!("  default: {LUNA}\n"),
            &format!("  default: {OPUS}\n"),
        );
    write_config(&edited);
    assert_eq!(
        hermes.gate_model_state().unwrap(),
        GateModelState::Applied { model: OPUS.into() }
    );
    assert_eq!(tool_models::states()["hermes"].left_gate_models, None);

    hermes.connect(&input()).unwrap();
    assert_eq!(
        yaml()["model"]["default"],
        OPUS,
        "a reconnect keeps the user's pick"
    );

    // Back to App default restores Hermes' own values, including the slug form
    // of our provider that Hermes wrote.
    choose_tool();
    hermes.connect(&input()).unwrap();
    let y = yaml();
    assert_eq!(y["model"]["provider"], "openrouter");
    assert_eq!(y["model"]["default"], "z-ai/glm-5.2");
    assert_eq!(y["model"]["base_url"], "https://openrouter.ai/api/v1");
    assert_eq!(y["model"]["api_mode"], "chat_completions");
}

/// Connecting again - launch, reconcile - changes nothing and stays applied.
#[test]
fn a_reconnect_keeps_hermes_on_gate_models() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = setup(ORIGINAL);
    let hermes = find(ToolId::Hermes).unwrap();
    choose_gate(&[LUNA, OPUS]);
    hermes.connect(&input()).unwrap();
    let first = config();
    hermes.connect(&input()).unwrap();
    hermes.connect(&input()).unwrap();
    assert_eq!(config(), first);
    assert_eq!(
        hermes.gate_model_state().unwrap(),
        GateModelState::Applied { model: LUNA.into() }
    );
    assert_eq!(preferences::load().tool_models["hermes"].source, ModelSource::Gate);
}
