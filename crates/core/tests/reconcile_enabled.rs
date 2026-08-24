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

/// Persist a stable relay port so [`proxy::relay_base_url`] resolves. CLI tool
/// configs point at the loopback relay, so connecting/reconciling a tool needs
/// a bound relay port; in these tests no engine runs, so we seed the persisted
/// port file directly (the same file the manager writes after `enable`) and
/// bind a real listener on it (kept alive by the caller): Claude Code's status
/// check probes relay liveness, so a dead seeded port would read as the honest
/// "proxy is not running" drift instead of Connected.
fn set_relay_port() -> std::net::TcpListener {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let dir = env::app_support_dir().unwrap().join("proxy");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("relay-port"), port.to_string()).unwrap();
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
    let _relay = set_relay_port();
    install_claude_unconfigured();
    // Precondition: Anthropic is on by default, and Claude is present but not
    // yet routed through Gate.
    assert_eq!(claude_status(), Status::Detected);

    provider::reconcile_enabled().unwrap();

    // The sweep wired it up without any explicit toggle.
    assert_eq!(claude_status(), Status::Connected);
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
    let _relay = set_relay_port();
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
    let _relay = set_relay_port();
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
    let _relay = set_relay_port();
    install_claude_with_stale_managed_config();
    // Precondition: the old scheme reads as drift, not as connected.
    assert!(matches!(claude_status(), Status::Drifted(_)));

    provider::reconcile_enabled().unwrap();

    // The sweep reasserted the managed keys: relay base URL, no baked key.
    assert_eq!(claude_status(), Status::Connected);
    let raw = fs::read_to_string(env::claude_code_settings_path().unwrap()).unwrap();
    let relay_port = _relay.local_addr().unwrap().port();
    assert!(raw.contains(&format!("http://127.0.0.1:{relay_port}")));
    assert!(!raw.contains("X-Gate-Api-Key"));
}

#[test]
fn managed_drift_without_relay_is_left_alone() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    // No relay port persisted: connect() has no base URL to point the tool
    // at, and the drift reason may *be* "relay not enabled yet".
    install_claude_with_stale_managed_config();
    let before = fs::read(env::claude_code_settings_path().unwrap()).unwrap();

    provider::reconcile_enabled().unwrap();

    // Untouched: still drifted, byte-for-byte identical config.
    assert!(matches!(claude_status(), Status::Drifted(_)));
    let after = fs::read(env::claude_code_settings_path().unwrap()).unwrap();
    assert_eq!(before, after);
}
