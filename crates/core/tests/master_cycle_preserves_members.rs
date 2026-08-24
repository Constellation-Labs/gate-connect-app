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

/// Persist a relay port so `relay_base_url()` answers `Some` and `connect()`
/// has a loopback base to write. Binds a real listener on the port (kept
/// alive by the caller): Claude Code's status check probes relay liveness, so
/// a dead seeded port would read as the honest "proxy is not running" drift
/// instead of Connected.
fn set_relay_port() -> std::net::TcpListener {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let dir = env::app_support_dir().unwrap().join("proxy");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("relay-port"), port.to_string()).unwrap();
    listener
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

/// The restore snapshots live beside the proxy's own state, and `provider`
/// keeps their paths private. Seeded and read here directly so a test can stage
/// the on-disk state a master-off leaves behind.
fn snapshot_file(name: &str) -> PathBuf {
    env::app_support_dir().unwrap().join("provider").join(name)
}

fn seed_snapshot(name: &str, slugs: &[&str]) {
    let path = snapshot_file(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, serde_json::to_string(slugs).unwrap()).unwrap();
}

fn read_snapshot(name: &str) -> Option<Vec<String>> {
    let raw = fs::read_to_string(snapshot_file(name)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// The bug, directly: Claude Code routing, Claude Desktop switched off, one
/// master cycle.
#[test]
fn a_member_switched_off_stays_off_across_a_master_cycle() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    let _relay = set_relay_port();
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
    let _relay = set_relay_port();
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
    let _relay = set_relay_port();
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
    let _relay = set_relay_port();
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

/// The two restore passes, and the one that runs too early.
///
/// `routing::enable` calls `restore_all` twice: once before the engine is up
/// (config-file tools) and once after (domain-only providers, which have no
/// tool to configure and need a running engine to flip their domain in). The
/// first pass therefore *cannot* finish a domain-only provider - `enable_inner`
/// takes its `plan.nothing` path and returns `Ok` having done nothing - and
/// counting that `Ok` as a completed restore cleared the snapshot before the
/// second pass could read it. Every domain-only provider then stayed off for
/// the rest of the session with nothing reporting a failure.
///
/// Seeded on disk rather than arranged through the API on purpose: the shape
/// only occurs with a live engine at master-off and none at restore, which this
/// harness has no way to stage.
#[test]
fn a_restore_that_could_not_run_yet_keeps_its_snapshot() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = TestEnv::set();
    sign_in();
    // Claude Code deliberately not installed and no engine running, so
    // `anthropic` is the domain-only shape: nothing to do until the engine is
    // up. The domain is off because the master-off sweep just turned it off -
    // arranging that is what makes this the state a restore actually starts
    // from, rather than one where the provider already reads on. The skip list
    // is non-empty because that is the case that lost the snapshot: with it
    // empty, `enable_inner` bails and the error already kept the entry.
    config::set_enabled("anthropic", false).unwrap();
    seed_snapshot("restore-snapshot.json", &["anthropic"]);
    seed_snapshot("restore-skip-members.json", &["claude-code"]);

    provider::restore_all().unwrap();

    assert_eq!(
        read_snapshot("restore-snapshot.json").as_deref(),
        Some(["anthropic".to_string()].as_slice()),
        "the pre-engine pass consumed the snapshot the post-engine pass needs"
    );
    assert!(
        read_snapshot("restore-skip-members.json").is_some(),
        "the skip list has to outlive an unfinished restore too, or the retry \
         turns on the members the user had switched off"
    );
}
