//! Integration tests for [`provider::reconcile_enabled`]: the startup/focus
//! sweep that configures a tool installed *after* its provider was enabled
//! (e.g. Claude Code installed after Gate Connect). Anthropic ships enabled by
//! default, so a detected-but-unconfigured Claude Code should get wired up.
//!
//! These exercise real path resolution and the real integration, so each test
//! points `GATE_CONNECT_TEST_HOME` (redirects `~/.claude` and the app-support
//! dir) and `GATE_CONNECT_TEST_SECRETS` (file-backed keychain) at a throwaway
//! dir. They live in their own test binary and a `Mutex` serializes them, so
//! the env override can't race other tests.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use gate_connect_core::proxy::config;
use gate_connect_core::registry::{find, Status, ToolId};
use gate_connect_core::{account, env, provider};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Point every per-user path at a fresh temp dir for the duration of a test,
/// restoring the prior env (and deleting the dir) on drop.
struct TestEnv {
    dir: PathBuf,
    prev_home: Option<String>,
    prev_secrets: Option<String>,
}

impl TestEnv {
    fn set() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "gate-connect-reconcile-test-{}-{}",
            std::process::id(),
            n
        ));
        fs::create_dir_all(dir.join("secrets")).unwrap();
        let prev_home = std::env::var("GATE_CONNECT_TEST_HOME").ok();
        let prev_secrets = std::env::var("GATE_CONNECT_TEST_SECRETS").ok();
        std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);
        std::env::set_var("GATE_CONNECT_TEST_SECRETS", dir.join("secrets"));
        TestEnv {
            dir,
            prev_home,
            prev_secrets,
        }
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        restore("GATE_CONNECT_TEST_HOME", &self.prev_home);
        restore("GATE_CONNECT_TEST_SECRETS", &self.prev_secrets);
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn restore(key: &str, prev: &Option<String>) {
    match prev {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
}

/// Sign in: gateway URL on disk + Gate key in the (file-backed) keychain.
fn sign_in() {
    account::save("https://gw.example.com", Some("sk-gw-testkey123")).unwrap();
}

/// Seed the persisted ports and routing snapshot that a running proxy owns,
/// and bind the forward-proxy port for real. Claude Code uses that port so its
/// Anthropic base URL stays canonical; the relay port remains seeded because
/// provider reconciliation uses it as its general liveness prerequisite.///
/// The listener is returned rather than dropped because the seeded files are
/// only half of what a live proxy looks like: `engine_proxy_url()` probes the
/// port before handing it out, so a caller that lets this fall out of scope is
/// describing a crashed engine, not a running one. Bound on an ephemeral port
/// so concurrent test binaries cannot collide.
fn bind_proxy_ports() -> std::net::TcpListener {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let dir = env::app_support_dir().unwrap().join("proxy");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("relay-port"), port.to_string()).unwrap();
    fs::write(dir.join("port"), port.to_string()).unwrap();
    // An engine that has started has minted its CA, and Claude Code's connect
    // refuses without one rather than writing a proxy it could not verify.
    fs::write(
        dir.join("ca-cert.pem"),
        "-----BEGIN CERTIFICATE-----\ngate-test-ca\n-----END CERTIFICATE-----\n",
    )
    .unwrap();
    #[cfg(target_os = "macos")]
    let snapshot = "[]";
    #[cfg(target_os = "linux")]
    let snapshot = r#"{ "block_present": false }"#;
    #[cfg(target_os = "windows")]
    let snapshot = r#"{ "enable": 0, "server": "", "bypass": "", "auto_config_url": "" }"#;
    fs::write(dir.join("system-proxy.snapshot.json"), snapshot).unwrap();
    listener
}

/// Make Claude Code look installed-but-unconfigured: its config dir exists
/// (so `detect()` is true) with no Gate settings written, which is
/// [`Status::Detected`].
fn install_claude_unconfigured() {
    fs::create_dir_all(env::claude_code_config_dir().unwrap()).unwrap();
}

fn claude_status() -> Status {
    find(ToolId::ClaudeCode).unwrap().status().unwrap()
}

#[test]
fn tool_installed_after_enable_is_configured() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    let proxy = bind_proxy_ports();
    install_claude_unconfigured();
    // Precondition: Anthropic is on by default, and Claude is present but not
    // yet routed through Gate.
    assert_eq!(claude_status(), Status::Detected);

    provider::reconcile_enabled().unwrap();

    // The sweep wired it up without any explicit toggle.
    assert_eq!(claude_status(), Status::Connected);
    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(env::claude_code_settings_path().unwrap()).unwrap(),
    )
    .unwrap();
    let env_block = settings.get("env").and_then(|v| v.as_object()).unwrap();
    let expected_proxy = format!(
        "http://gate-claude-code:route@127.0.0.1:{}",
        proxy.local_addr().unwrap().port()
    );
    assert_eq!(
        env_block.get("HTTPS_PROXY").and_then(|v| v.as_str()),
        Some(expected_proxy.as_str())
    );
    // The proxy variable is inherited by everything `claude` spawns, so the
    // loopback bypass travels with it or a local MCP server goes through the
    // engine.
    assert_eq!(
        env_block.get("NO_PROXY").and_then(|v| v.as_str()),
        Some("localhost,127.0.0.1,::1")
    );
    assert!(
        !env_block.contains_key("ANTHROPIC_BASE_URL"),
        "the canonical Anthropic base URL must remain implicit"
    );
    assert!(
        !env_block.contains_key("ANTHROPIC_BETAS"),
        "Gate Connect must leave context-window selection to Claude Code's selected model variant"
    );
}

/// An engine that died without running its revert must not read as Connected.
///
/// The snapshot outlives the process that wrote it - that is the whole point
/// of `reconcile_on_startup`, which clears it at the *next* launch - so between
/// a SIGKILL and that launch the on-disk state is indistinguishable from a
/// healthy one: snapshot present, port persisted, tool config pointing at it.
/// Only the port itself can tell the two apart, which is why
/// `engine_proxy_url()` probes it rather than trusting the snapshot alone.
///
/// Reported by whoever asks first, and that is usually not the app: the GUI
/// heals this on launch, so the process that sees the stale window is the CLI
/// running beside a menubar app that is no longer there.
#[test]
fn a_crashed_engine_reads_as_drift_not_connected() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    let proxy = bind_proxy_ports();
    install_claude_unconfigured();
    provider::reconcile_enabled().unwrap();
    assert_eq!(claude_status(), Status::Connected);

    // The engine goes away without reverting anything: the port stops
    // accepting, while the snapshot and the tool's config stay exactly as it
    // left them.
    drop(proxy);

    assert!(
        matches!(claude_status(), Status::Drifted(_)),
        "a tool pointed at a dead proxy port is drifted, not connected"
    );
}

#[test]
fn user_owned_anthropic_betas_are_preserved_on_connect() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    let _proxy = bind_proxy_ports();
    install_claude_unconfigured();
    fs::write(
        env::claude_code_settings_path().unwrap(),
        r#"{ "env": { "ANTHROPIC_BETAS": "user-owned-beta" } }"#,
    )
    .unwrap();
    assert_eq!(claude_status(), Status::Detected);

    provider::reconcile_enabled().unwrap();

    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(env::claude_code_settings_path().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        settings
            .get("env")
            .and_then(|v| v.get("ANTHROPIC_BETAS"))
            .and_then(|v| v.as_str()),
        Some("user-owned-beta"),
        "Gate Connect must not overwrite or manage user-owned Anthropic betas"
    );
}

#[test]
fn disabled_provider_is_skipped() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    // The user turned Anthropic off; that intent is persisted.
    config::set_enabled("anthropic", false).unwrap();
    install_claude_unconfigured();

    provider::reconcile_enabled().unwrap();

    // Left alone - the sweep must not re-enable a provider the user disabled.
    assert_eq!(claude_status(), Status::Detected);
}

#[test]
fn no_account_is_noop() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    // No sign-in: nothing to point the tool at.
    install_claude_unconfigured();

    provider::reconcile_enabled().unwrap();

    assert_eq!(claude_status(), Status::Detected);
}

#[test]
fn already_connected_tool_is_left_untouched() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    let _proxy = bind_proxy_ports();
    install_claude_unconfigured();

    // First sweep connects it.
    provider::reconcile_enabled().unwrap();
    assert_eq!(claude_status(), Status::Connected);
    let after_first = fs::read(env::claude_code_settings_path().unwrap()).unwrap();

    // Second sweep is a no-op: a Connected tool is skipped, so the config is
    // byte-for-byte unchanged (no needless rewrite).
    provider::reconcile_enabled().unwrap();
    assert_eq!(claude_status(), Status::Connected);
    let after_second = fs::read(env::claude_code_settings_path().unwrap()).unwrap();
    assert_eq!(after_first, after_second);
}

/// Regression: enabling a provider while the proxy is off must persist the
/// on-intent durably, so a later [`reconcile_enabled`] re-wires a tool that has
/// since dropped back to [`Status::Detected`]. Guards the enable/disable
/// persisted-intent asymmetry - `disable` always persisted off, but `enable`
/// used to persist on only while the proxy was running, stranding the intent
/// after an off then on cycle with routing stopped.
#[test]
fn enable_while_proxy_off_persists_intent_for_reconcile() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    let _proxy = bind_proxy_ports();
    install_claude_unconfigured();

    // Off then on with the proxy stopped. `disable` persists the off-intent;
    // `enable` must re-persist the on-intent as well as configuring the tool.
    provider::disable("anthropic").unwrap();
    provider::enable("anthropic").unwrap();
    assert_eq!(claude_status(), Status::Connected);

    // Drop the tool back to `Detected` (its Gate config removed) while the
    // persisted domain flag stays intact - the state a fresh reconcile faces
    // after a reboot or when the tool's config is lost.
    fs::remove_file(env::claude_code_settings_path().unwrap()).unwrap();
    assert_eq!(claude_status(), Status::Detected);

    provider::reconcile_enabled().unwrap();

    // The persisted on-intent drove the re-wire. Without the symmetry fix,
    // `enable` left the domain persisted off from the earlier `disable`, so
    // reconcile would skip the provider and Claude would stay `Detected`.
    assert_eq!(claude_status(), Status::Connected);
}

/// Write the pre-relay scheme our older builds put in settings.json: base URL
/// pointing straight at the gateway and the Gate key baked into the custom
/// headers, with the `_gateConnect` marker showing we wrote it. This is
/// [`Status::Drifted`] under the relay scheme.
fn install_claude_with_stale_managed_config() {
    install_claude_unconfigured();
    let stale = serde_json::json!({
        "env": {
            "ANTHROPIC_BASE_URL": "https://gw.example.com",
            "ANTHROPIC_CUSTOM_HEADERS":
                "X-Gate-Api-Key: sk-gw-testkey123\nX-Gate-Upstream-Url: https://api.anthropic.com"
        },
        "_gateConnect": {
            "managed": ["ANTHROPIC_BASE_URL", "ANTHROPIC_CUSTOM_HEADERS"],
            "previousEnv": {}
        }
    });
    fs::write(
        env::claude_code_settings_path().unwrap(),
        serde_json::to_string_pretty(&stale).unwrap(),
    )
    .unwrap();
}

#[test]
fn stale_managed_config_is_reapplied() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    let proxy = bind_proxy_ports();
    install_claude_with_stale_managed_config();
    // Precondition: the old scheme reads as drift, not as connected.
    assert!(matches!(claude_status(), Status::Drifted(_)));

    provider::reconcile_enabled().unwrap();

    // The sweep migrated the legacy custom base URL to transparent proxying.
    assert_eq!(claude_status(), Status::Connected);
    let raw = fs::read_to_string(env::claude_code_settings_path().unwrap()).unwrap();
    let settings: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let env_block = settings.get("env").and_then(|v| v.as_object()).unwrap();
    let expected_proxy = format!(
        "http://gate-claude-code:route@127.0.0.1:{}",
        proxy.local_addr().unwrap().port()
    );
    assert_eq!(
        env_block.get("HTTPS_PROXY").and_then(|v| v.as_str()),
        Some(expected_proxy.as_str())
    );
    assert!(!env_block.contains_key("ANTHROPIC_BASE_URL"));
    assert!(!raw.contains("X-Gate-Api-Key"));
}

#[test]
fn managed_drift_without_relay_is_left_alone() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    // No proxy ports persisted: connect() has no live forward proxy to use.
    install_claude_with_stale_managed_config();
    let before = fs::read(env::claude_code_settings_path().unwrap()).unwrap();

    provider::reconcile_enabled().unwrap();

    // Untouched: still drifted, byte-for-byte identical config.
    assert!(matches!(claude_status(), Status::Drifted(_)));
    let after = fs::read(env::claude_code_settings_path().unwrap()).unwrap();
    assert_eq!(before, after);
}

// ---------------------------------------------------------------------------
// Codex. The provider pass is the only path that could reach it - it is mapped
// to the `openai` provider, so `reconcile_unmapped_tools` skips it by
// construction - which makes `domains_enabled_persisted(openai)` the gate on
// everything below.
// ---------------------------------------------------------------------------

/// Make Codex look installed, logged in, and connected by an older build: the
/// base URL shape we wrote before the tool marker existed, plus the
/// `[_gate_connect]` marker saying it was ours. This is [`Status::Drifted`]
/// under the marker shape.
fn install_codex_with_stale_managed_config(relay_port: u16) {
    let dir = env::codex_config_dir().unwrap();
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        env::codex_auth_json_path().unwrap(),
        r#"{"auth_mode":"apikey"}"#,
    )
    .unwrap();
    let stale = format!(
        r#"model_provider = "gate"

[model_providers.gate]
name = "Gate"
base_url = "http://127.0.0.1:{relay_port}/openai/v1"
wire_api = "responses"
requires_openai_auth = true

[_gate_connect]
previous_model_provider_absent = true
"#
    );
    fs::write(env::codex_config_toml_path().unwrap(), stale).unwrap();
}

fn codex_status() -> Status {
    find(ToolId::Codex).unwrap().status().unwrap()
}

fn codex_config() -> String {
    fs::read_to_string(env::codex_config_toml_path().unwrap()).unwrap()
}

#[test]
fn codex_stale_managed_config_is_reapplied() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    let proxy = bind_proxy_ports();
    let port = proxy.local_addr().unwrap().port();
    install_codex_with_stale_managed_config(port);
    assert!(matches!(codex_status(), Status::Drifted(_)));
    assert!(find(ToolId::Codex).unwrap().config_is_managed().unwrap());

    provider::reconcile_enabled().unwrap();

    assert_eq!(codex_status(), Status::Connected);
    assert!(
        codex_config().contains(&format!("http://127.0.0.1:{port}/__gate/t/codex/openai/v1")),
        "the stale base URL should have been rewritten with the tool marker: {}",
        codex_config()
    );
}

/// Turning Codex off by hand is an instruction, not drift to repair.
///
/// `model_provider = "openai"` is what a user writes to stop routing Codex
/// through Gate without running disconnect, and `status` reports it as
/// `Drifted` because our block is still sitting there. The marker alone cannot
/// tell that apart from our own stale write - it records who created the block,
/// not who wrote the values in it now - which is why `config_is_managed` asks
/// the second question too.
#[test]
fn codex_hand_edited_off_gate_is_not_silently_reverted() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    let proxy = bind_proxy_ports();
    let port = proxy.local_addr().unwrap().port();
    install_codex_with_stale_managed_config(port);
    let hand_edited =
        codex_config().replace(r#"model_provider = "gate""#, r#"model_provider = "openai""#);
    fs::write(env::codex_config_toml_path().unwrap(), &hand_edited).unwrap();
    assert!(matches!(codex_status(), Status::Drifted(_)));
    assert!(
        !find(ToolId::Codex).unwrap().config_is_managed().unwrap(),
        "a config pointed away from Gate is not ours to reapply"
    );

    provider::reconcile_enabled().unwrap();

    assert_eq!(codex_config(), hand_edited, "the hand edit was reverted");
}

/// The same question about the other value the user can change: our block, our
/// marker, but `base_url` repointed at their own gateway.
#[test]
fn codex_repointed_base_url_is_not_silently_reverted() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    let proxy = bind_proxy_ports();
    let port = proxy.local_addr().unwrap().port();
    install_codex_with_stale_managed_config(port);
    let repointed = codex_config().replace(
        &format!("http://127.0.0.1:{port}/openai/v1"),
        "https://gateway.example.com/v1",
    );
    fs::write(env::codex_config_toml_path().unwrap(), &repointed).unwrap();
    assert!(matches!(codex_status(), Status::Drifted(_)));
    assert!(!find(ToolId::Codex).unwrap().config_is_managed().unwrap());

    provider::reconcile_enabled().unwrap();

    assert_eq!(codex_config(), repointed, "the hand edit was reverted");
}

/// A disconnected tool stays disconnected across the sweep's drift half.
///
/// This is the load-bearing half of letting managed drift repair itself without
/// consulting the provider switch: `disconnect` removes the marker, so the
/// question `config_is_managed` asks answers "no" for every tool the user has
/// deliberately turned off, whatever its provider's domains say.
#[test]
fn codex_disconnected_is_not_reconnected_by_the_drift_half() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    let proxy = bind_proxy_ports();
    let port = proxy.local_addr().unwrap().port();
    install_codex_with_stale_managed_config(port);
    find(ToolId::Codex).unwrap().disconnect().unwrap();
    let after_disconnect = codex_config();
    assert!(
        !find(ToolId::Codex).unwrap().config_is_managed().unwrap(),
        "the disconnect stub must not read as a config we manage"
    );

    provider::reconcile_enabled().unwrap();

    assert_eq!(
        codex_config(),
        after_disconnect,
        "a disconnected Codex was reconnected"
    );
}

// ---------------------------------------------------------------------------
// OpenCode. No provider maps it, so it takes the drift half via
// `reconcile_unmapped_tools`. Its own local-protection guard is what used to
// make that unreachable.
// ---------------------------------------------------------------------------

/// Route OpenCode through Gate for real, then wind its baseURL back to the
/// shape an older build wrote - no tool marker - which is what every existing
/// install holds on the first launch after this change.
fn install_opencode_with_stale_managed_config(relay_port: u16) {
    let dir = env::opencode_config_dir().unwrap();
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        env::opencode_config_path().unwrap(),
        r#"{
  "provider": {
    "anthropic": {
      "options": { "apiKey": "{env:ANTHROPIC_API_KEY}" },
      "models": { "claude-haiku-4-5": {} }
    }
  }
}
"#,
    )
    .unwrap();

    let integ = find(ToolId::OpenCode).unwrap();
    integ
        .connect(&gate_connect_core::registry::ConnectInput {
            gateway_base_url: "https://gw.example.com".into(),
            upstream_url: integ.default_upstream_url().to_string(),
            billing_mode: Default::default(),
            relay_base_url: Some(format!("http://127.0.0.1:{relay_port}")),
            engine_proxy_url: None,
        })
        .expect("connect opencode");

    let raw = fs::read_to_string(env::opencode_config_path().unwrap()).unwrap();
    let stale = raw.replace(
        &format!("http://127.0.0.1:{relay_port}/__gate/t/opencode/"),
        &format!("http://127.0.0.1:{relay_port}/"),
    );
    assert_ne!(raw, stale, "the connect write should have carried a marker");
    fs::write(env::opencode_config_path().unwrap(), stale).unwrap();
}

fn opencode_config() -> String {
    fs::read_to_string(env::opencode_config_path().unwrap()).unwrap()
}

/// An OpenCode install written before the tool marker repairs itself.
///
/// The obstacle was its own local-protection guard: `connect` skips a provider
/// whose current baseURL `looks_local`, and Gate's relay is on 127.0.0.1, so
/// every already-connected provider was filtered out and the re-apply bailed
/// with "No supported OpenCode providers found". Not a corner case - a re-apply
/// is *always* over a baseURL of ours, so the guard fired on 100% of connected
/// installs and none of them could ever pick up a new relay port either.
#[test]
fn opencode_stale_managed_config_is_reapplied() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    let proxy = bind_proxy_ports();
    let port = proxy.local_addr().unwrap().port();
    install_opencode_with_stale_managed_config(port);
    assert!(matches!(
        find(ToolId::OpenCode).unwrap().status().unwrap(),
        Status::Drifted(_)
    ));
    assert!(find(ToolId::OpenCode).unwrap().config_is_managed().unwrap());

    provider::reconcile_enabled().unwrap();

    assert!(
        opencode_config().contains(&format!("http://127.0.0.1:{port}/__gate/t/opencode/")),
        "the stale base URL should have been rewritten with the tool marker: {}",
        opencode_config()
    );
}

/// The guard the exemption above has to keep: a provider inside the allowlist
/// that the user pointed at their own local server is still left alone, because
/// the sidecar has no record of us ever writing it.
#[test]
fn opencode_leaves_a_users_own_local_endpoint_alone() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    let proxy = bind_proxy_ports();
    let port = proxy.local_addr().unwrap().port();
    let dir = env::opencode_config_dir().unwrap();
    fs::create_dir_all(&dir).unwrap();
    let own = r#"{
  "provider": {
    "anthropic": {
      "options": { "baseURL": "http://127.0.0.1:11434/v1" }
    }
  }
}
"#;
    fs::write(env::opencode_config_path().unwrap(), own).unwrap();

    let integ = find(ToolId::OpenCode).unwrap();
    let err = integ
        .connect(&gate_connect_core::registry::ConnectInput {
            gateway_base_url: "https://gw.example.com".into(),
            upstream_url: integ.default_upstream_url().to_string(),
            billing_mode: Default::default(),
            relay_base_url: Some(format!("http://127.0.0.1:{port}")),
            engine_proxy_url: None,
        })
        .expect_err("a local endpoint we never wrote is not ours to repoint");
    assert!(
        err.to_string().contains("No supported OpenCode providers"),
        "unexpected error: {err}"
    );
    assert_eq!(
        opencode_config(),
        own,
        "the user's own endpoint was rewritten"
    );
}
