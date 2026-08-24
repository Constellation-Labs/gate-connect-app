//! A master off/on cycle must bring a family back exactly as it was, including
//! the members inside it that were switched off.
//!
//! The restore snapshot is provider-granularity, a provider counts as enabled
//! when any one of its members is on, and `enable` turns on *every* member of a
//! provider. Those three together meant a family that was on because one member
//! was on came back with all of them on. Master-off destroys the per-member
//! state on its way out - `disable` clears every domain flag and disconnects
//! every tool - so nothing was left afterwards to say which members the user had
//! switched off, and the popover reported the app as routing again without ever
//! having been asked.
//!
//! Same env-isolation pattern as `reconcile_enabled`: real path resolution and
//! real integrations against a throwaway `GATE_CONNECT_TEST_HOME`, serialized by
//! a `Mutex` so the env override can't race.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use gate_connect_core::proxy::config;
use gate_connect_core::registry::{find, Status, ToolId};
use gate_connect_core::{account, env, provider};

static ENV_LOCK: Mutex<()> = Mutex::new(());

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
            "gate-connect-mastercycle-test-{}-{}",
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

fn sign_in() {
    account::save("https://gw.example.com", Some("sk-gw-testkey123")).unwrap();
}

fn set_relay_port(port: u16) {
    let dir = env::app_support_dir().unwrap().join("proxy");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("relay-port"), port.to_string()).unwrap();
    fs::write(dir.join("port"), port.to_string()).unwrap();
    #[cfg(target_os = "macos")]
    let snapshot = "[]";
    #[cfg(target_os = "linux")]
    let snapshot = r#"{ "block_present": false }"#;
    #[cfg(target_os = "windows")]
    let snapshot = r#"{ "enable": 0, "server": "", "bypass": "", "auto_config_url": "" }"#;
    fs::write(dir.join("system-proxy.snapshot.json"), snapshot).unwrap();
}

fn install_claude_unconfigured() {
    fs::create_dir_all(env::claude_code_config_dir().unwrap()).unwrap();
}

fn claude_status() -> Status {
    find(ToolId::ClaudeCode).unwrap().status().unwrap()
}

fn domain_enabled(slug: &str) -> bool {
    config::load_domains()
        .unwrap()
        .into_iter()
        .find(|d| d.slug == slug)
        .map(|d| d.enabled)
        .unwrap_or(false)
}

/// The bug, directly: Claude Code routing, Claude Desktop switched off, one
/// master cycle.
#[test]
fn a_member_switched_off_stays_off_across_a_master_cycle() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    set_relay_port(9977);
    install_claude_unconfigured();

    // Claude Code on, Claude Desktop (the `anthropic` proxy domain, which ships
    // enabled) switched off by hand. The family is on because the tool is.
    provider::enable("anthropic").unwrap();
    config::set_enabled("anthropic", false).unwrap();
    assert_eq!(claude_status(), Status::Connected);
    assert!(!domain_enabled("anthropic"));

    provider::snapshot_and_disable_everything().unwrap();
    provider::restore_all().unwrap();

    // The family came back. The app the user switched off did not.
    assert_eq!(claude_status(), Status::Connected);
    assert!(
        !domain_enabled("anthropic"),
        "the master cycle re-enabled an app the user had switched off"
    );
}

/// The inverse arrangement, so the fix isn't just "never enable domains".
#[test]
fn a_member_that_was_on_comes_back_on() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    set_relay_port(9977);
    install_claude_unconfigured();

    provider::enable("anthropic").unwrap();
    assert_eq!(claude_status(), Status::Connected);
    assert!(domain_enabled("anthropic"));

    provider::snapshot_and_disable_everything().unwrap();
    assert_eq!(claude_status(), Status::Detected);
    assert!(!domain_enabled("anthropic"));

    provider::restore_all().unwrap();

    assert_eq!(claude_status(), Status::Connected);
    assert!(
        domain_enabled("anthropic"),
        "a member that was on stayed off"
    );
}

/// A second off-flow (the quit teardown right after a master-off) sees every
/// provider already off. If that pass contributed to the skip list, it would
/// list every member of every family and the restore would bring nothing back.
#[test]
fn a_second_off_flow_does_not_poison_the_restore() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    set_relay_port(9977);
    install_claude_unconfigured();
    provider::enable("anthropic").unwrap();

    provider::snapshot_and_disable_everything().unwrap();
    // Quit teardown, immediately after, with nothing left enabled to snapshot.
    provider::snapshot_and_disable_everything().unwrap();

    provider::restore_all().unwrap();

    assert_eq!(claude_status(), Status::Connected);
    assert!(domain_enabled("anthropic"));
}

/// The skip list is scoped to one cycle. Turning the app back on by hand after
/// the restore must not be undone by the *next* master cycle.
#[test]
fn the_skip_list_does_not_outlive_its_cycle() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    set_relay_port(9977);
    install_claude_unconfigured();
    provider::enable("anthropic").unwrap();
    config::set_enabled("anthropic", false).unwrap();

    provider::snapshot_and_disable_everything().unwrap();
    provider::restore_all().unwrap();
    assert!(!domain_enabled("anthropic"));

    // The user changes their mind and switches the app on.
    config::set_enabled("anthropic", true).unwrap();

    // A fresh cycle must honour the new answer, not the old skip list.
    provider::snapshot_and_disable_everything().unwrap();
    provider::restore_all().unwrap();
    assert!(
        domain_enabled("anthropic"),
        "a stale skip list outlived its master cycle"
    );
}
