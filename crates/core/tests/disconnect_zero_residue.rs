//! Integration test for the "zero residue" disconnect contract: `connect()`
//! copies the tool's config to a sibling `.gate-backup`, and `disconnect()`
//! must remove that copy (regression for the bug where the backup lingered).
//!
//! These exercise the real path resolution, which reads `$HOME` via `dirs`, so
//! each test overrides `HOME` to a throwaway dir. It lives in its own test
//! binary so that override can't race the in-crate unit tests, and a `Mutex`
//! serializes the HOME-mutating tests within this binary.

use std::fs;
use std::path::{Path, PathBuf};
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

/// `<path>.gate-backup`, matching how `backup_once` names the copy.
fn backup_path(path: &Path) -> PathBuf {
    let mut b = path.as_os_str().to_owned();
    b.push(".gate-backup");
    PathBuf::from(b)
}

#[test]
fn claude_code_disconnect_removes_gate_backup() {
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
    // The copy connect() would have made.
    let backup = backup_path(&settings);
    fs::write(&backup, "{}\n").unwrap();
    assert!(backup.exists());

    find(ToolId::ClaudeCode).unwrap().disconnect().unwrap();

    assert!(
        !backup.exists(),
        "disconnect must remove the .gate-backup copy"
    );
    let after = fs::read_to_string(&settings).unwrap();
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
fn codex_disconnect_removes_gate_backup() {
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
    let backup = backup_path(&config);
    fs::write(&backup, "model_provider = \"openai\"\n").unwrap();
    assert!(backup.exists());

    find(ToolId::Codex).unwrap().disconnect().unwrap();

    assert!(
        !backup.exists(),
        "disconnect must remove the .gate-backup copy"
    );
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
