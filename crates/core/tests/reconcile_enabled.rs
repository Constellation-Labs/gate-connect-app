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
