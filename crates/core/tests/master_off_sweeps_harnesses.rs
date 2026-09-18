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

mod common;

use common::RelayStub;
use gate_connect_core::registry::{find, ConnectInput, Mechanism, Status, ToolId};
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
    // The quit predicate reads the persisted relay port to know which origin
    // is ours; seed it to match the base URL `connect` is given below, so the
    // address it writes reads as the relay's and not a stranger's.
    let proxy_dir = env::app_support_dir().unwrap().join("proxy");
    fs::create_dir_all(&proxy_dir).unwrap();
    fs::write(proxy_dir.join("relay-port"), "8402").unwrap();
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
            engine_proxy_url: Some("http://127.0.0.1:8403".into()),
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

/// The routing switch is the other half, and it must do the opposite.
///
/// `snapshot_and_park_everything` is what the routing toggle runs now: the
/// engine parks with its ports bound and forwarding straight through, so a
/// config naming them still reaches the tool's own provider. Rewriting it would
/// move no traffic and would tell every running process that it missed a change
/// and has to be reopened.
///
/// Asserted on the file, not on `status`: status is allowed to say "configured
/// but not routing" here, and does. What must not happen is Gate editing
/// somebody's config to say it.
#[test]
fn the_routing_switch_leaves_a_harness_config_alone() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    connect_opencode();

    let path = env::opencode_config_path().unwrap();
    let before = fs::read_to_string(&path).expect("opencode config after connect");
    assert!(
        before.contains("127.0.0.1:8402"),
        "premise: connect must have written the relay address, got {before}"
    );

    provider::snapshot_and_park_everything().expect("routing off");

    let after = fs::read_to_string(&path).expect("opencode config after routing off");
    assert_eq!(
        before, after,
        "the routing switch must not rewrite a tool's configuration"
    );
}

/// And it records nothing to restore, because it reverted nothing.
///
/// A slug left in this snapshot would be restored on master-on, which for a
/// config that was never changed means a write for nothing - and before
/// `write_file` learned to decline an identical write, that was a bumped mtime
/// and a reopen prompt per toggle.
#[test]
fn the_routing_switch_records_no_swept_tools() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    connect_opencode();

    provider::snapshot_and_park_everything().expect("routing off");

    let snapshot = env::app_support_dir()
        .unwrap()
        .join("provider")
        .join("restore-tools-snapshot.json");
    let raw = fs::read_to_string(&snapshot).unwrap_or_default();
    assert!(
        !raw.contains("opencode"),
        "nothing was swept, so nothing may be recorded for restore, got {raw}"
    );
}

/// Plain quit reverts a tool whose configured address dies with the GUI, and
/// records it so the startup restore brings it back. OpenCode's `baseURL`
/// names the relay origin, which lives in the GUI process on macOS and
/// Windows, so it is exactly the case. The other half of the rule - a tool
/// naming the forwarder is left alone - is `plain_quit_follows_the_address_rule_for_claude_code`
/// in `master_cycle_preserves_members.rs`, which has the Claude Code fixture.
#[test]
fn plain_quit_reverts_a_relay_tool_and_records_it() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    connect_opencode();
    let path = env::opencode_config_path().unwrap();
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("127.0.0.1:8402"),
        "premise: connect must have written the relay address"
    );

    let reverted = provider::revert_stranded_configs_for_quit().expect("plain quit");

    assert_eq!(reverted, vec!["OpenCode".to_string()]);
    assert!(
        !fs::read_to_string(&path)
            .unwrap()
            .contains("127.0.0.1:8402"),
        "the relay address must be gone from the config"
    );
    let snapshot = env::app_support_dir()
        .unwrap()
        .join("provider")
        .join("restore-tools-snapshot.json");
    let raw = fs::read_to_string(&snapshot).expect("snapshot written");
    assert!(
        raw.contains("opencode"),
        "must be recorded for restore, got {raw}"
    );
}

/// The declared mechanism per tool is the fact the quit teardown keys on, so it
/// is pinned against the table in `docs/routing-architecture.md` section 3.
#[test]
fn the_mechanism_table_matches_the_routing_doc() {
    let m = |id| find(id).unwrap().mechanism();
    assert_eq!(m(ToolId::Codex), Mechanism::Relay);
    assert_eq!(m(ToolId::OpenCode), Mechanism::Relay);
    assert_eq!(m(ToolId::ClaudeCode), Mechanism::ForwardProxy);
    assert_eq!(m(ToolId::OpenClaw), Mechanism::ForwardProxy);
    assert_eq!(m(ToolId::Hermes), Mechanism::ForwardProxy);
    assert_eq!(m(ToolId::EnvProxy), Mechanism::Environment);
}

/// A base URL the user repointed by hand names a port nothing of ours is bound
/// to, so nothing dies and the file is not touched - even though `status` reads
/// it as drifted and the disconnect sweep would take it. Plain quit runs every
/// day and promises less than "disconnect".
#[test]
fn plain_quit_leaves_a_hand_repointed_config_alone() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    connect_opencode();
    let path = env::opencode_config_path().unwrap();
    let repointed = fs::read_to_string(&path)
        .unwrap()
        .replace("http://127.0.0.1:8402", "https://gateway.elsewhere.example");
    assert_ne!(
        repointed,
        fs::read_to_string(&path).unwrap(),
        "premise: the edit took"
    );
    fs::write(&path, &repointed).unwrap();
    assert!(
        matches!(
            find(ToolId::OpenCode).unwrap().status().unwrap(),
            Status::Drifted(_)
        ),
        "premise: a repointed base URL reads as drift"
    );

    let reverted = provider::revert_stranded_configs_for_quit().expect("plain quit");

    assert!(
        reverted.is_empty(),
        "nothing of ours dies, so nothing is reverted: {reverted:?}"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        repointed,
        "the user's edit must survive"
    );
}

/// The swept-tools snapshot is a union: a pending restore from an earlier sweep
/// must not be shortened by a plain quit that records one more tool.
#[test]
fn plain_quit_unions_into_a_pending_snapshot() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    connect_opencode();
    let snapshot = env::app_support_dir()
        .unwrap()
        .join("provider")
        .join("restore-tools-snapshot.json");
    fs::create_dir_all(snapshot.parent().unwrap()).unwrap();
    fs::write(&snapshot, r#"["codex"]"#).unwrap();

    provider::revert_stranded_configs_for_quit().expect("plain quit");

    let raw = fs::read_to_string(&snapshot).unwrap();
    assert!(
        raw.contains("codex"),
        "the pending entry must survive: {raw}"
    );
    assert!(
        raw.contains("opencode"),
        "the new entry must be added: {raw}"
    );
}

/// Bind a port and persist it as the relay's, so `relay_listening()` finds
/// something. Returned so the caller keeps it alive: a relay tool's status asks
/// whether the port answers, and a persisted port file on its own is exactly
/// the state that used to read Connected over a dead address.
fn bind_relay_port() -> (RelayStub, u16) {
    bind_relay_port_with(true)
}

/// A relay on a fresh port, reporting whether it is routing.
fn bind_relay_port_with(intercepting: bool) -> (RelayStub, u16) {
    let stub = RelayStub::with_interception(0, intercepting);
    let port = stub.port();
    let dir = env::app_support_dir().unwrap().join("proxy");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("relay-port"), port.to_string()).unwrap();
    (stub, port)
}

/// Record the user's last explicit routing answer, which is what tells a parked
/// relay from a routing one.
fn seed_routing_intent(enabled: bool) {
    let path = env::app_support_dir()
        .unwrap()
        .join("proxy")
        .join("routing-intent.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, format!(r#"{{"enabled":{enabled}}}"#)).unwrap();
}

/// Route Codex through a live relay, the way `connect` does.
fn connect_codex(port: u16) {
    let dir = env::codex_config_dir().unwrap();
    fs::create_dir_all(&dir).unwrap();
    // `connect` reads the login mode from here; without it the mode probe is
    // what fails, and the test would be about something else entirely.
    fs::write(dir.join("auth.json"), r#"{"auth_mode":"apikey"}"#).unwrap();

    let integ = find(ToolId::Codex).expect("codex integration registered");
    integ
        .connect(&ConnectInput {
            gateway_base_url: "https://gateway.example.com".into(),
            upstream_url: integ.default_upstream_url().to_string(),
            relay_base_url: Some(format!("http://127.0.0.1:{port}")),
            engine_proxy_url: None,
        })
        .expect("connect codex");
}

/// A relay that answers is not a relay that routes. With the engine parked the
/// port stays bound and forwards straight through, so the probe alone reads
/// Connected over traffic going direct - a green pill over the one thing this
/// status exists to catch. The user's last explicit answer is the signal.
///
/// Both directions asserted, because a check that only ever says "not routing"
/// would pass this test by being uniformly wrong.
#[test]
fn codex_is_not_connected_while_routing_is_off() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    let (_relay, port) = bind_relay_port();
    connect_codex(port);

    assert!(
        matches!(
            find(ToolId::Codex).unwrap().status().unwrap(),
            Status::Connected
        ),
        "a relay that reports it is routing is Connected"
    );

    // The same relay, now parked: it still proves itself and still answers, and
    // Codex reaches OpenAI through it, just not through Gate.
    _relay.set_intercepting(false);
    match find(ToolId::Codex).unwrap().status().unwrap() {
        Status::Drifted(m) => {
            assert!(m.contains("routing is off"), "unexpected message: {m}");
            assert!(m.contains("directly"), "must say where traffic goes: {m}");
        }
        other => panic!("a parked relay must not read as Connected, got {other:?}"),
    }
}

/// The same for OpenCode, which had neither this check nor the liveness one
/// until the two relay tools were found disagreeing about a single parked
/// engine. Invisible before, because routing-off used to sweep OpenCode to
/// Detected; keeping the config is what made it the steady state.
#[test]
fn opencode_is_not_connected_while_routing_is_off() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    connect_opencode();
    // `connect_opencode` persists 8402 and writes base URLs naming it, so the
    // listener has to be on that port for the probe to reach it.
    let relay = RelayStub::bind(8402);

    assert!(
        matches!(
            find(ToolId::OpenCode).unwrap().status().unwrap(),
            Status::Connected
        ),
        "a relay that reports it is routing is Connected"
    );

    relay.set_intercepting(false);
    match find(ToolId::OpenCode).unwrap().status().unwrap() {
        Status::Drifted(m) => {
            assert!(m.contains("routing is off"), "unexpected message: {m}");
            assert!(m.contains("directly"), "must say where traffic goes: {m}");
        }
        other => panic!("a parked relay must not read as Connected, got {other:?}"),
    }
}

/// `requires_engine` is derived from `mechanism` rather than declared twice.
/// The reconcile passes ask it before re-asserting a drifted config, so a new
/// integration that declared one and forgot the other would be gated by a rule
/// that does not describe it.
#[test]
fn requires_engine_follows_the_declared_mechanism() {
    for id in [
        ToolId::ClaudeCode,
        ToolId::Codex,
        ToolId::OpenCode,
        ToolId::OpenClaw,
        ToolId::Hermes,
        ToolId::EnvProxy,
    ] {
        let integ = find(id).unwrap();
        assert_eq!(
            integ.requires_engine(),
            integ.mechanism() == Mechanism::ForwardProxy,
            "{} disagrees with its own mechanism",
            integ.display_name()
        );
    }
}

/// A persisted relay port is not a live one. The port file survives restarts
/// precisely so configs stay valid across them, so comparing a config against
/// it proves identity and says nothing about reachability: on its own it reads
/// Connected while the tool dials a port nothing is bound to.
#[test]
fn codex_with_nothing_listening_is_not_connected() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    seed_routing_intent(true);
    let (relay, port) = bind_relay_port();
    connect_codex(port);
    // Connect against a live relay, then let it go: the config is untouched and
    // still correct, which is exactly the state that must not read Connected.
    drop(relay);

    match find(ToolId::Codex).unwrap().status().unwrap() {
        Status::Drifted(m) => assert!(m.contains("dead address"), "unexpected message: {m}"),
        other => panic!("a dead relay must not read as Connected, got {other:?}"),
    }
}

/// The same for OpenCode, whose fixture persists a port that nothing binds.
#[test]
fn opencode_with_nothing_listening_is_not_connected() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    seed_routing_intent(true);
    connect_opencode();

    match find(ToolId::OpenCode).unwrap().status().unwrap() {
        Status::Drifted(m) => assert!(m.contains("dead address"), "unexpected message: {m}"),
        other => panic!("a dead relay must not read as Connected, got {other:?}"),
    }
}

/// A stranger on the relay port is not our relay. A bare TCP connect cannot
/// tell the two apart, so a status check built on one reported somebody else's
/// listener as a healthy Gate relay; the challenge is what separates "the port
/// is taken" from "the port is ours".
#[test]
fn a_stranger_on_the_relay_port_is_not_our_relay() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    seed_routing_intent(true);

    // Bound and accepting, but it cannot read the token, so it cannot answer.
    let squatter = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = squatter.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in squatter.incoming() {
            drop(stream);
        }
    });
    let dir = env::app_support_dir().unwrap().join("proxy");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("relay-port"), port.to_string()).unwrap();

    assert!(
        !gate_connect_core::proxy::relay_listening(),
        "a listener that cannot answer the challenge must not read as our relay"
    );
}

/// And the stub that can answer does read as ours, so the test above is about
/// the proof rather than about the stub being unreachable.
#[test]
fn a_listener_that_answers_the_challenge_is_our_relay() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    let (_relay, _port) = bind_relay_port();

    assert!(
        gate_connect_core::proxy::relay_listening(),
        "a listener answering the challenge is our relay"
    );
}

/// The two questions the manager asks about another instance are different, and
/// the difference is what keeps `status` honest.
///
/// A parked instance holds the ports and routes nothing, so it must not be
/// reported as hosting the proxy - that is what would put "running" on screen
/// with routing off. `enable` still has to refuse against it, which it checks
/// separately once it has released its own park.
///
/// macOS and Windows only, because the function is: Linux hosts its engine in a
/// daemon and adopts it rather than asking whether another process has one. So
/// this runs on two of the three CI runners and not on a Linux dev machine.
#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
fn a_parked_instance_holds_the_ports_without_hosting_the_proxy() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    let (relay, _port) = bind_relay_port();
    // The engine port has to answer too, or the question is decided by the
    // probe rather than by the report under test.
    let engine = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let engine_port = engine.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in engine.incoming() {
            drop(stream);
        }
    });
    let dir = env::app_support_dir().unwrap().join("proxy");
    fs::write(dir.join("port"), engine_port.to_string()).unwrap();

    assert_eq!(
        gate_connect_core::proxy::engine_hosted_elsewhere(),
        Some(engine_port),
        "a routing instance is hosting the proxy"
    );

    relay.set_intercepting(false);

    assert_eq!(
        gate_connect_core::proxy::engine_hosted_elsewhere(),
        None,
        "a parked instance routes nothing, so nothing is hosting the proxy"
    );
    assert!(
        gate_connect_core::proxy::relay_listening(),
        "but it is still there, which is what keeps a second enable from taking its ports"
    );
}
