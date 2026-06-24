//! Integration test for the "zero residue" disconnect contract:
//! `disconnect()` must revert the Gate edits out of the tool's config and
//! remove the in-file marker, leaving nothing of Gate behind. The
//! key-scoped marker inside the config is the undo log - no separate
//! backup file is ever written.
//!
//! These exercise the real path resolution, which reads `$HOME` via `dirs`, so
//! each test overrides `HOME` to a throwaway dir. It lives in its own test
//! binary so that override can't race the in-crate unit tests, and a `Mutex`
//! serializes the HOME-mutating tests within this binary.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use gate_connect_core::env;
use gate_connect_core::registry::{find, ToolId};

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
    assert!(
        doc.get("model_providers").is_none(),
        "gate provider must be removed from config.toml"
    );
    assert!(
        doc.get("_gate_connect").is_none(),
        "Gate marker must be removed from config.toml"
    );
}

#[test]
fn codex_disconnect_leaves_no_gate_residue() {
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

    // The user had nothing but Gate's config, so disconnect removes the file
    // entirely; either way no Gate residue may remain.
    let after = fs::read_to_string(&config).unwrap_or_default();
    assert!(
        !after.contains("model_providers.gate"),
        "gate provider must be removed from config.toml"
    );
    assert!(
        !after.contains("_gate_connect"),
        "Gate marker must be removed from config.toml"
    );
}
