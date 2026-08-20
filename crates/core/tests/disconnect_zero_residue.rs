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
use gate_connect_core::registry::{find, ConnectInput, Status, ToolId};

static HOME_LOCK: Mutex<()> = Mutex::new(());

/// Point `$HOME` at a fresh temp dir for the duration of a test, restoring the
/// prior value (and deleting the dir) on drop.
///
/// Also pins `XDG_CONFIG_HOME` / `XDG_DATA_HOME` inside that dir. OpenCode's
/// config path honors XDG (as OpenCode itself does), so leaving the ambient
/// values in place would let a test escape its temp home and edit the
/// developer's real `~/.config/opencode/opencode.json`.
struct TempHome {
    dir: PathBuf,
    prev: Option<String>,
    prev_xdg_config: Option<String>,
    prev_xdg_data: Option<String>,
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
        let prev_xdg_config = std::env::var("XDG_CONFIG_HOME").ok();
        let prev_xdg_data = std::env::var("XDG_DATA_HOME").ok();
        std::env::set_var("HOME", &dir);
        std::env::set_var("XDG_CONFIG_HOME", dir.join(".config"));
        std::env::set_var("XDG_DATA_HOME", dir.join(".local/share"));
        TempHome {
            dir,
            prev,
            prev_xdg_config,
            prev_xdg_data,
        }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        fn restore(key: &str, prev: &Option<String>) {
            match prev {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
        restore("HOME", &self.prev);
        restore("XDG_CONFIG_HOME", &self.prev_xdg_config);
        restore("XDG_DATA_HOME", &self.prev_xdg_data);
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Persist a relay port so `relay_base_url()` answers `Some` and `connect()`
/// has a loopback base to write.
fn seed_relay_port(port: u16) {
    let path = env::app_support_dir()
        .unwrap()
        .join("proxy")
        .join("relay-port");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, port.to_string()).unwrap();
}

/// Persist an engine port so `persisted_engine_proxy_url()` answers `Some` and
/// OpenClaw's drift check can tell "pointed at us" from "pointed elsewhere".
/// Deliberately does NOT fake a running engine - no snapshot file - so status
/// still reports the honest "proxy is not running" drift.
fn seed_engine_port(port: u16) {
    let path = env::app_support_dir().unwrap().join("proxy").join("port");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, port.to_string()).unwrap();
}

/// Seed a CA cert so `ca_bundle::ensure()` has something to append. Contents
/// are never parsed here - the bundle is concatenated text - so a marker string
/// is enough to prove it made it into the output.
fn seed_ca_cert() {
    let path = env::app_support_dir()
        .unwrap()
        .join("proxy")
        .join("ca-cert.pem");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        "-----BEGIN CERTIFICATE-----\ngate-test-ca\n-----END CERTIFICATE-----\n",
    )
    .unwrap();
}

fn connect_input(relay_port: u16) -> ConnectInput {
    ConnectInput {
        gateway_base_url: "https://gw.example.com".to_string(),
        upstream_url: "https://api.anthropic.com".to_string(),
        billing_mode: Default::default(),
        relay_base_url: Some(format!("http://127.0.0.1:{relay_port}")),
        engine_proxy_url: Some(format!("http://127.0.0.1:{relay_port}")),
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

// --- The tools no provider maps -----------------------------------------
//
// These drive the real connect -> disconnect round trip rather than a
// hand-fabricated "connected" file, so they also pin the per-provider base URL
// the catalog resolves. Formatting is not asserted (both configs are
// re-serialized on write); what must hold is that no Gate value survives, the
// user's own settings do, and the sidecar is gone.

#[test]
fn opencode_disconnect_leaves_no_gate_residue() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    seed_relay_port(9977);

    let cfg = env::opencode_config_path().unwrap();
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    fs::write(
        &cfg,
        r#"{
  "provider": {
    "openrouter": { "options": { "apiKey": "user-openrouter-key" } }
  }
}
"#,
    )
    .unwrap();

    let integ = find(ToolId::OpenCode).unwrap();
    integ.connect(&connect_input(9977)).unwrap();

    // OpenRouter's API lives under /api, so `/api/v1` has to stay on the client
    // side: that keeps the forwarded path on the `/api/` inference prefix and
    // the upstream hint on the bare host the catalog knows.
    let connected = fs::read_to_string(&cfg).unwrap();
    assert!(
        connected.contains("http://127.0.0.1:9977/openrouter/v1"),
        "openrouter baseURL must keep the slug + /v1: {connected}"
    );
    assert!(
        !connected.contains("X-Gate-Upstream-Url"),
        "no Gate header may be written - the relay derives the upstream from the \
         slug in the base URL: {connected}"
    );
    assert!(matches!(integ.status().unwrap(), Status::Connected));

    integ.disconnect().unwrap();

    let after = fs::read_to_string(&cfg).unwrap_or_default();
    assert!(
        !after.contains("127.0.0.1"),
        "relay base URL must be reverted: {after}"
    );
    assert!(
        !after.contains("X-Gate-"),
        "no Gate header may survive: {after}"
    );
    assert!(
        after.contains("user-openrouter-key"),
        "the user's own apiKey must survive: {after}"
    );
    assert!(
        !env::app_support_dir()
            .unwrap()
            .join("opencode-state.json")
            .exists(),
        "sidecar must be removed"
    );
}

#[test]
fn openclaw_disconnect_leaves_no_gate_residue() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    seed_relay_port(9977);
    seed_engine_port(9977);

    let cfg = env::openclaw_config_path().unwrap();
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    // A user-set `loopbackMode` and a provider block: connect must leave both
    // exactly as it found them. OpenClaw routes via the proxy, so there is no
    // per-provider rewriting and no discovery step at all.
    fs::write(
        &cfg,
        r#"{"proxy":{"loopbackMode":"gateway-only"},"models":{"providers":{"openrouter":{"baseUrl":"https://openrouter.ai/api/v1","apiKey":"user-openrouter-key"}}}}"#,
    )
    .unwrap();

    let integ = find(ToolId::OpenClaw).unwrap();
    integ.connect(&connect_input(9977)).unwrap();

    let connected = fs::read_to_string(&cfg).unwrap();
    assert!(
        connected.contains(r#""proxyUrl": "http://127.0.0.1:9977""#),
        "proxy.proxyUrl must point at the engine: {connected}"
    );
    assert!(
        connected.contains(r#""enabled": true"#),
        "proxy.enabled is what actually switches managed proxy mode on - without it \
         OpenClaw ignores the URL and reaches providers directly: {connected}"
    );
    assert!(
        connected.contains("https://openrouter.ai/api/v1"),
        "the provider baseUrl must stay canonical - redirecting it is what made \
         OpenClaw drop its implicit beta headers: {connected}"
    );
    assert!(
        connected.contains(r#""loopbackMode": "gateway-only""#),
        "loopbackMode is the user's local-provider bypass and must survive: {connected}"
    );
    // Not Connected here, and correctly so: no engine is running against this
    // temp HOME, and OpenClaw hands *all* its egress to the proxy. Drift is the
    // truthful answer, and it still counts as Gate-managed for the master-off
    // sweep. The Connected path is covered by `compute_status` unit tests.
    match integ.status().unwrap() {
        Status::Drifted(m) => assert!(
            m.contains("no route out") || m.contains("does not match"),
            "unexpected status message: {m}"
        ),
        other => panic!("expected drift with no engine running, got {other:?}"),
    }

    integ.disconnect().unwrap();

    let after = fs::read_to_string(&cfg).unwrap_or_default();
    assert!(
        !after.contains("127.0.0.1"),
        "the proxy URL must be reverted: {after}"
    );
    assert!(
        !after.contains("proxyUrl"),
        "a proxyUrl the user never had must not survive: {after}"
    );
    assert!(
        !after.contains("enabled"),
        "an enabled flag the user never had must not survive either - leaving it \
         behind switches managed proxy mode on with no proxy to point at: {after}"
    );
    assert!(
        after.contains(r#""loopbackMode": "gateway-only""#),
        "the user's own proxy settings must be left behind intact: {after}"
    );
    assert!(
        after.contains("https://openrouter.ai/api/v1"),
        "the user's original baseUrl must be untouched throughout: {after}"
    );
    assert!(
        !env::openclaw_config_dir().unwrap().join(".env").exists(),
        "the NODE_EXTRA_CA_CERTS file we created must be removed"
    );
    assert!(
        !env::app_support_dir()
            .unwrap()
            .join("openclaw-state.json")
            .exists(),
        "sidecar must be removed"
    );
}

#[test]
fn hermes_disconnect_leaves_no_gate_residue() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    seed_relay_port(9977);
    seed_engine_port(9977);
    seed_ca_cert();

    // detect() wants the launcher, not just the config dir - the installer drops
    // it in ~/.local/bin.
    let launcher = env::home().unwrap().join(".local/bin/hermes");
    fs::create_dir_all(launcher.parent().unwrap()).unwrap();
    fs::write(&launcher, "#!/bin/sh\n").unwrap();

    // A config.yaml and an .env holding the user's own key. Neither the model
    // block nor the key may be touched: Hermes routes via the proxy now, so
    // there is no base_url to rewrite and no provider to discover.
    let cfg = env::hermes_config_dir().unwrap().join("config.yaml");
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    let original_cfg =
        "model:\n  provider: custom\n  base_url: https://openrouter.ai/api/v1\n  api_key: user-key\n";
    fs::write(&cfg, original_cfg).unwrap();
    let envfile = env::hermes_config_dir().unwrap().join(".env");
    fs::write(&envfile, "OPENROUTER_API_KEY=sk-user\n").unwrap();

    let integ = find(ToolId::Hermes).unwrap();
    integ.connect(&connect_input(9977)).unwrap();

    let env_body = fs::read_to_string(&envfile).unwrap();
    assert!(
        env_body.contains("HTTPS_PROXY=http://127.0.0.1:9977"),
        "the proxy must be set in .env: {env_body}"
    );
    assert!(
        env_body.contains("NO_PROXY=localhost,127.0.0.1,::1"),
        "loopback must stay off the proxy so local providers keep working: {env_body}"
    );
    assert!(
        env_body.contains("HERMES_CA_BUNDLE="),
        "a full CA bundle is required - venv certifi does not see the OS store: {env_body}"
    );
    assert_eq!(
        fs::read_to_string(&cfg).unwrap(),
        original_cfg,
        "config.yaml must not be touched at all"
    );

    // A re-connect has to be a no-op that succeeds, not a refusal: it is how a
    // drifted Hermes is repaired, including unattended by `reconcile_enabled`.
    // It used to fail here, objecting to .env variables that were Gate's own.
    integ
        .connect(&connect_input(9977))
        .expect("a re-connect must be idempotent");

    // Not Connected here: no engine is running against this temp HOME. Drift is
    // the truthful answer and still counts as managed for the master-off sweep.
    match integ.status().unwrap() {
        Status::Drifted(m) => assert!(
            m.contains("not running") || m.contains("does not match"),
            "unexpected status message: {m}"
        ),
        other => panic!("expected drift with no engine running, got {other:?}"),
    }

    integ.disconnect().unwrap();

    let after = fs::read_to_string(&envfile).unwrap();
    assert!(
        !after.contains("127.0.0.1"),
        "the proxy must be reverted: {after}"
    );
    assert!(
        !after.contains("HERMES_CA_BUNDLE"),
        "the CA bundle line must be reverted: {after}"
    );
    assert!(
        after.contains("OPENROUTER_API_KEY=sk-user"),
        "the user's own key must survive: {after}"
    );
    assert_eq!(
        fs::read_to_string(&cfg).unwrap(),
        original_cfg,
        "config.yaml must still be untouched after disconnect"
    );
    assert!(
        !env::app_support_dir()
            .unwrap()
            .join("hermes-state.json")
            .exists(),
        "sidecar must be removed"
    );
}

#[test]
fn hermes_leaves_a_user_owned_proxy_alone() {
    // A pre-existing HTTPS_PROXY is likely a corporate egress proxy the rest of
    // the user's setup depends on. Clobbering it would break far more than Gate,
    // so connect refuses rather than taking it over.
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    seed_relay_port(9977);
    seed_engine_port(9977);
    seed_ca_cert();

    let launcher = env::home().unwrap().join(".local/bin/hermes");
    fs::create_dir_all(launcher.parent().unwrap()).unwrap();
    fs::write(&launcher, "#!/bin/sh\n").unwrap();

    let envfile = env::hermes_config_dir().unwrap().join(".env");
    fs::create_dir_all(envfile.parent().unwrap()).unwrap();
    let original = "HTTPS_PROXY=http://corp.example:3128\nHTTP_PROXY=http://corp.example:3128\nNO_PROXY=corp.example\nHERMES_CA_BUNDLE=/corp/ca.pem\n";
    fs::write(&envfile, original).unwrap();

    let err = find(ToolId::Hermes)
        .unwrap()
        .connect(&connect_input(9977))
        .expect_err("a user-owned proxy must not be silently replaced");
    assert!(
        format!("{err:#}").contains("own proxy settings"),
        "error should say why: {err:#}"
    );
    assert_eq!(
        fs::read_to_string(&envfile).unwrap(),
        original,
        "the user's .env must be byte-identical after a refusal"
    );
}
