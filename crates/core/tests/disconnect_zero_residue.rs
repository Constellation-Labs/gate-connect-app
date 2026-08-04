//! Integration test for the "zero residue" disconnect contract:
//! `disconnect()` must revert the Gate edits out of the tool's config and
//! remove the in-file marker, leaving nothing of Gate behind. The
//! key-scoped marker inside the config is the undo log - no separate
//! backup file is ever written.
//!
//! Codex is the one documented exception: it keeps a `[model_providers.gate]`
//! passthrough stub pointed at OpenAI, because Codex stores the provider name
//! in each thread and would otherwise refuse to resume any thread started
//! while routing was on. No credential, gateway URL or Gate header survives
//! there either - see `crates/core/src/integrations/codex.rs`.
//!
//! These exercise the real path resolution, which reads `$HOME` via `dirs`, so
//! each test overrides `HOME` to a throwaway dir. It lives in its own test
//! binary so that override can't race the in-crate unit tests, and a `Mutex`
//! serializes the HOME-mutating tests within this binary.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use gate_connect_core::env;
use gate_connect_core::registry::{find, Status, ToolId};

static HOME_LOCK: Mutex<()> = Mutex::new(());

/// Point `$HOME` at a fresh temp dir for the duration of a test, restoring the
/// prior value (and deleting the dir) on drop.
struct TempHome {
    dir: PathBuf,
    prev: Option<String>,
}

impl TempHome {
    fn set() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "gate-connect-disconnect-test-{}-{}",
            std::process::id(),
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        let prev = std::env::var("HOME").ok();
        std::env::set_var("HOME", &dir);
        TempHome { dir, prev }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn claude_code_disconnect_leaves_no_gate_residue() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();

    let settings = env::claude_code_settings_path().unwrap();
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    // A connected state: Gate env block + our marker.
    fs::write(
        &settings,
        r#"{
  "env": {
    "ANTHROPIC_BASE_URL": "https://gw.example.com",
    "ANTHROPIC_CUSTOM_HEADERS": "X-Gate-Api-Key: sk-gw-xxx\nX-Gate-Upstream-Url: https://api.anthropic.com"
  },
  "_gateConnect": { "previousEnv": {}, "managed": ["ANTHROPIC_BASE_URL", "ANTHROPIC_CUSTOM_HEADERS"] }
}
"#,
    )
    .unwrap();

    find(ToolId::ClaudeCode).unwrap().disconnect().unwrap();

    // The user had nothing but Gate's entries, so disconnect removes the
    // file entirely (codex parity); either way no Gate residue may remain.
    let after = fs::read_to_string(&settings).unwrap_or_default();
    assert!(
        !after.contains("ANTHROPIC_BASE_URL"),
        "Gate env must be reverted out of settings.json"
    );
    assert!(
        !after.contains("_gateConnect"),
        "Gate marker must be removed from settings.json"
    );
}

#[test]
fn claude_code_disconnect_restores_previous_env() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();

    let settings = env::claude_code_settings_path().unwrap();
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    // A connected state where the user had their own ANTHROPIC_BASE_URL
    // before Gate (stashed in previousEnv), no prior custom headers, and
    // an unrelated env var Gate must not touch.
    fs::write(
        &settings,
        r#"{
  "env": {
    "ANTHROPIC_BASE_URL": "https://gw.example.com",
    "ANTHROPIC_CUSTOM_HEADERS": "X-Gate-Api-Key: sk-gw-xxx\nX-Gate-Upstream-Url: https://api.anthropic.com",
    "FOO": "bar"
  },
  "_gateConnect": { "previousEnv": { "ANTHROPIC_BASE_URL": "https://my-proxy.example.com" }, "managed": ["ANTHROPIC_BASE_URL", "ANTHROPIC_CUSTOM_HEADERS"] }
}
"#,
    )
    .unwrap();

    find(ToolId::ClaudeCode).unwrap().disconnect().unwrap();

    let after: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
    let env_block = after.get("env").and_then(|v| v.as_object()).unwrap();
    assert_eq!(
        env_block.get("ANTHROPIC_BASE_URL").and_then(|v| v.as_str()),
        Some("https://my-proxy.example.com"),
        "the user's prior ANTHROPIC_BASE_URL must be restored verbatim"
    );
    assert!(
        !env_block.contains_key("ANTHROPIC_CUSTOM_HEADERS"),
        "a key with no prior value must be removed, not left as Gate's"
    );
    assert_eq!(
        env_block.get("FOO").and_then(|v| v.as_str()),
        Some("bar"),
        "unrelated env vars must survive disconnect untouched"
    );
    assert!(
        after.get("_gateConnect").is_none(),
        "Gate marker must be removed from settings.json"
    );
}

#[test]
fn codex_disconnect_restores_previous_model_provider() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();

    let config = env::codex_config_toml_path().unwrap();
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    // A connected state where the user's model_provider was "openai"
    // before Gate flipped the pointer, plus a user setting Gate must
    // not touch.
    fs::write(
        &config,
        r#"model = "gpt-5.2"
model_provider = "gate"

[model_providers.gate]
name = "Gate"
base_url = "https://gw.example.com/codex"

[model_providers.gate.http_headers]
X-Gate-Api-Key = "sk-gw-xxx"
X-Gate-Upstream-Url = "https://chatgpt.com/backend-api"

[_gate_connect]
previous_model_provider = "openai"
"#,
    )
    .unwrap();

    find(ToolId::Codex).unwrap().disconnect().unwrap();

    let after = fs::read_to_string(&config).unwrap();
    let doc: toml_edit::DocumentMut = after.parse().unwrap();
    assert_eq!(
        doc.get("model_provider").and_then(|i| i.as_str()),
        Some("openai"),
        "the user's prior model_provider must be restored verbatim"
    );
    assert_eq!(
        doc.get("model").and_then(|i| i.as_str()),
        Some("gpt-5.2"),
        "unrelated settings must survive disconnect untouched"
    );
    // The gate block survives as a passthrough stub so Codex threads started
    // while routed can still resume - but with nothing of Gate in it.
    let stub = doc
        .get("model_providers")
        .and_then(|i| i.as_table_like())
        .and_then(|t| t.get("gate"))
        .and_then(|i| i.as_table_like())
        .expect("passthrough stub must remain under [model_providers.gate]");
    assert_eq!(
        stub.get("base_url").and_then(|i| i.as_str()),
        Some("https://chatgpt.com/backend-api/codex"),
        "the stub must point straight at the upstream"
    );
    assert!(
        stub.get("http_headers").is_none(),
        "the stub must carry no Gate headers: {after}"
    );
    assert!(
        !after.contains("X-Gate-Api-Key") && !after.contains("gw.example.com"),
        "no Gate credential or gateway URL may survive disconnect: {after}"
    );
    // Only the stub marker survives; the undo-log keys are spent.
    assert!(
        !after.contains("previous_model_provider"),
        "the undo-log keys must be removed from config.toml: {after}"
    );
}

#[test]
fn codex_disconnect_leaves_only_a_passthrough_stub() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();

    let config = env::codex_config_toml_path().unwrap();
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    // A connected state: gate provider + pointer + marker.
    fs::write(
        &config,
        r#"model_provider = "gate"

[model_providers.gate]
name = "Gate"
base_url = "https://gw.example.com/codex"

[model_providers.gate.http_headers]
X-Gate-Api-Key = "sk-gw-xxx"
X-Gate-Upstream-Url = "https://chatgpt.com/backend-api"

[_gate_connect]
previous_model_provider_absent = true
"#,
    )
    .unwrap();

    find(ToolId::Codex).unwrap().disconnect().unwrap();

    // The user had nothing but Gate's config, yet the file stays: the gate
    // provider name has to keep resolving or every Codex thread started while
    // routed becomes unresumable. What's left routes direct to OpenAI.
    let after = fs::read_to_string(&config).unwrap();
    assert!(
        !after.contains(r#"model_provider = "gate""#),
        "the pointer must not survive disconnect: {after}"
    );
    assert!(
        after.contains("[model_providers.gate]")
            && after.contains(r#"base_url = "https://chatgpt.com/backend-api/codex""#),
        "the passthrough stub must remain: {after}"
    );
    assert!(
        !after.contains("X-Gate-Api-Key")
            && !after.contains("X-Gate-Upstream-Url")
            && !after.contains("gw.example.com"),
        "no Gate credential, header or gateway URL may survive disconnect: {after}"
    );

    // And the stub must read as a disconnected machine, not as drift - drift
    // is what surfaces a config warning in the UI and blocks sign-out.
    assert!(
        matches!(find(ToolId::Codex).unwrap().status(), Ok(Status::Detected)),
        "a stubbed config must report Detected, got {:?}",
        find(ToolId::Codex).unwrap().status()
    );
}

/// Disconnecting twice must be a no-op on the stub, not an accumulation of
/// comment blocks or a resurrected pointer.
#[test]
fn codex_disconnect_is_idempotent_over_the_stub() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();

    let config = env::codex_config_toml_path().unwrap();
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(
        &config,
        r#"model_provider = "gate"

[model_providers.gate]
name = "Constellation Gate"
base_url = "http://127.0.0.1:45981/codex"
requires_openai_auth = true

[model_providers.gate.http_headers]
X-Gate-Upstream-Url = "https://chatgpt.com/backend-api"

[_gate_connect]
previous_model_provider_absent = true
"#,
    )
    .unwrap();

    let codex = find(ToolId::Codex).unwrap();
    codex.disconnect().unwrap();
    let once = fs::read_to_string(&config).unwrap();
    codex.disconnect().unwrap();
    let twice = fs::read_to_string(&config).unwrap();

    assert_eq!(once, twice, "a second disconnect must change nothing");
    assert!(
        !twice.contains("127.0.0.1"),
        "the relay URL must not survive disconnect: {twice}"
    );
}
