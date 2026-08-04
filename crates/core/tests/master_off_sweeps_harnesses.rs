//! The master routing switch must disconnect *everything* it manages, not
//! just the tools some provider happens to map.
//!
//! The provider catalog claims Claude Code and Codex; it deliberately claims
//! none of the multi-provider agent harnesses (OpenCode and friends). A
//! master-off that only walks the catalog leaves those harnesses pointed at
//! the loopback relay, which dies with the engine moments later - so the tool
//! breaks while the popover reports "not routing". `proxy_disable` therefore
//! calls `snapshot_and_disable_everything`, whose registry sweep covers them,
//! and the swept tools are recorded so master-on brings them back.
//!
//! Uses the real path resolution (`GATE_CONNECT_TEST_HOME` redirects both the
//! config dirs and the app-support dir), so it lives in its own test binary
//! and serializes the env-mutating tests within it.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use gate_connect_core::registry::{find, ConnectInput, Status, ToolId};
use gate_connect_core::{env, provider};

static HOME_LOCK: Mutex<()> = Mutex::new(());

/// Point `GATE_CONNECT_TEST_HOME` at a fresh temp dir for the duration of a
/// test, restoring the prior value (and deleting the dir) on drop.
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
            "gate-connect-master-off-test-{}-{}",
            std::process::id(),
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        let prev = std::env::var("GATE_CONNECT_TEST_HOME").ok();
        std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);
        TempHome { dir, prev }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var("GATE_CONNECT_TEST_HOME", v),
            None => std::env::remove_var("GATE_CONNECT_TEST_HOME"),
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Put OpenCode on disk with one known provider block, then route it through
/// Gate for real. Returns once the tool is in a Gate-managed state.
///
/// The status lands on `Drifted` rather than `Connected` here because no proxy
/// engine is running under the test, and OpenCode reports "the Gate proxy has
/// not been enabled yet". That is the same managed set the sweep walks
/// (`Connected | Drifted`), and it is the state the harness is actually in at
/// the moment routing is switched off, so it exercises the right path.
fn connect_opencode() {
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

    let integ = find(ToolId::OpenCode).expect("opencode integration registered");
    integ
        .connect(&ConnectInput {
            gateway_base_url: "https://gateway.example.com".into(),
            upstream_url: integ.default_upstream_url().to_string(),
            relay_base_url: Some("http://127.0.0.1:8402".into()),
        })
        .expect("connect opencode");
    let status = integ.status().unwrap();
    assert!(
        matches!(status, Status::Connected | Status::Drifted(_)),
        "precondition: opencode must be Gate-managed before master-off, got {status:?}"
    );
}

#[test]
fn master_off_disconnects_a_harness_no_provider_maps() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    connect_opencode();

    // No provider in the catalog lists OpenCode, so the provider-only pass
    // would be a no-op here. This is the regression: it used to be.
    assert!(
        !provider::providers()
            .iter()
            .any(|p| p.tool_ids.contains(&ToolId::OpenCode)),
        "premise: the catalog must not claim OpenCode"
    );

    provider::snapshot_and_disable_everything().expect("master off");

    let status = find(ToolId::OpenCode).unwrap().status().unwrap();
    assert!(
        !matches!(status, Status::Connected | Status::Drifted(_)),
        "master-off must leave the harness unmanaged, got {status:?}"
    );
}

#[test]
fn master_off_records_the_swept_harness_so_master_on_can_restore_it() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    connect_opencode();

    provider::snapshot_and_disable_everything().expect("master off");

    let snapshot = env::app_support_dir()
        .unwrap()
        .join("provider")
        .join("restore-tools-snapshot.json");
    let raw = fs::read_to_string(&snapshot)
        .unwrap_or_else(|e| panic!("reading {}: {e}", snapshot.display()));
    assert!(
        raw.contains("opencode"),
        "swept tools must be recorded for restore, got {raw}"
    );
}
