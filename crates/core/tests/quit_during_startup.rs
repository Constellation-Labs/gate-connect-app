//! Once the exit sweep has run, nothing in the same process may put Gate back
//! into a tool (AG-1118).
//!
//! The startup thread's `routing::enable` restores, starts the engine and
//! restores again, taking the master-flow lock once per pass. A quit while the
//! engine was starting swept every tool, and the second restore pass then
//! reconnected them all and cleared the swept snapshot, so the app exited with
//! tools pointed at a drained forwarder. The startup `reconcile_enabled` could
//! do the same, and held no lock at all.
//!
//! In its own test binary, because the exit sweep's mark is process-wide and
//! never cleared: any later test in the same process would see every restore
//! refuse. Same env-isolation pattern as `reconcile_enabled`.

use std::fs;
use std::path::PathBuf;

mod common;

use common::RelayStub;
use gate_connect_core::proxy::config;
use gate_connect_core::registry::{find, Status, ToolId};
use gate_connect_core::{account, env, provider};

struct TestEnv {
    dir: PathBuf,
    prev_home: Option<String>,
    prev_secrets: Option<String>,
    prev_xdg_config: Option<String>,
}

impl TestEnv {
    fn set() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "gate-connect-quit-startup-test-{}-{}",
            std::process::id(),
            n
        ));
        fs::create_dir_all(dir.join("secrets")).unwrap();
        let prev_home = std::env::var("GATE_CONNECT_TEST_HOME").ok();
        let prev_secrets = std::env::var("GATE_CONNECT_TEST_SECRETS").ok();
        let prev_xdg_config = std::env::var("XDG_CONFIG_HOME").ok();
        std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);
        std::env::set_var("GATE_CONNECT_TEST_SECRETS", dir.join("secrets"));
        std::env::set_var("XDG_CONFIG_HOME", dir.join("xdg-config"));
        TestEnv {
            dir,
            prev_home,
            prev_secrets,
            prev_xdg_config,
        }
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        restore("GATE_CONNECT_TEST_HOME", &self.prev_home);
        restore("GATE_CONNECT_TEST_SECRETS", &self.prev_secrets);
        restore("XDG_CONFIG_HOME", &self.prev_xdg_config);
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn restore(key: &str, prev: &Option<String>) {
    match prev {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
}

/// The persisted ports, CA and system-proxy snapshot of a running engine, with
/// the port bound for real. See `reconcile_enabled::bind_proxy_ports`.
fn bind_proxy_ports() -> RelayStub {
    let listener = RelayStub::bind(0);
    let port = listener.port();
    let dir = env::app_support_dir().unwrap().join("proxy");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("relay-port"), port.to_string()).unwrap();
    fs::write(dir.join("port"), port.to_string()).unwrap();
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

fn claude_status() -> Status {
    find(ToolId::ClaudeCode).unwrap().status().unwrap()
}

#[test]
fn nothing_reconnects_a_tool_once_the_exit_sweep_has_run() {
    let _env = TestEnv::set();
    account::save("https://gw.example.com", Some("sk-gw-testkey123")).unwrap();
    let _proxy = bind_proxy_ports();
    fs::create_dir_all(env::claude_code_config_dir().unwrap()).unwrap();
    provider::reconcile_enabled().unwrap();
    assert_eq!(
        claude_status(),
        Status::Connected,
        "premise: Claude Code is routed before the quit"
    );

    provider::snapshot_and_disable_everything_for_exit().expect("quit sweep");
    assert_ne!(
        claude_status(),
        Status::Connected,
        "the sweep took Gate out"
    );
    let owed = provider::pending_restore().unwrap();
    assert!(
        !owed.is_empty(),
        "premise: the sweep recorded what to restore"
    );

    // The startup thread's restore passes, landing after the sweep.
    provider::restore_all().unwrap();
    assert_ne!(
        claude_status(),
        Status::Connected,
        "a restore after the exit sweep must not reconnect the tool"
    );
    provider::restore_one("anthropic").unwrap();
    assert_ne!(claude_status(), Status::Connected);
    // And the record survives for the next launch, which is the one that
    // restores from it.
    assert_eq!(provider::pending_restore().unwrap(), owed);

    // The startup reconcile, with every reason it has to connect: the
    // provider's switch on and the tool installed and unrouted.
    config::set_enabled("anthropic", true).unwrap();
    assert_eq!(claude_status(), Status::Detected);
    provider::reconcile_enabled().unwrap();
    assert_eq!(
        claude_status(),
        Status::Detected,
        "a reconcile after the exit sweep must not connect the tool"
    );
}
