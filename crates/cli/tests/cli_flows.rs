//! Black-box tests that drive the real `gate-connect` binary through the
//! `login → connect → disconnect` lifecycle for each supported tool, then
//! assert the per-tool config it wrote (and that disconnect leaves no Gate
//! residue). Because the CLI and the menubar app are both thin wrappers over
//! the same `gate-connect-core` functions, this exercises the exact paths the
//! GUI runs.
//!
//! Hermetic on every OS via two core test seams: `GATE_CONNECT_TEST_HOME`
//! roots all per-user config paths under a temp dir (portable where `$HOME`
//! isn't, e.g. Windows), and `GATE_CONNECT_TEST_SECRETS` file-backs the Gate
//! key instead of the OS keychain. No network, no elevation, no real gateway.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

const GATEWAY_URL: &str = "https://mock.gateway.test";
const API_KEY: &str = "sk-gw-ci-test";

/// A throwaway home + secret store and a helper to run the CLI against them.
struct Harness {
    home: TempDir,
    secrets: TempDir,
}

impl Harness {
    fn new() -> Self {
        Harness {
            home: TempDir::new().unwrap(),
            secrets: TempDir::new().unwrap(),
        }
    }

    fn home(&self) -> &Path {
        self.home.path()
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_gate-connect"))
            .args(args)
            .env("GATE_CONNECT_TEST_HOME", self.home.path())
            .env("GATE_CONNECT_TEST_SECRETS", self.secrets.path())
            .output()
            .expect("failed to spawn gate-connect")
    }

    /// Run the CLI and assert it exited 0, surfacing stderr on failure.
    fn run_ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "`gate-connect {}` failed: {}\nstdout: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout),
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Sign in so `connect` has a gateway URL on disk + a key in the store.
    fn login(&self) {
        self.run_ok(&["login", "--base-url", GATEWAY_URL, "--api-key", API_KEY]);
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

#[test]
fn claude_code_connect_then_disconnect() {
    let h = Harness::new();
    // detect() falls back to the config dir existing.
    fs::create_dir_all(h.home().join(".claude")).unwrap();
    h.login();

    h.run_ok(&["connect", "claude-code"]);

    let settings: PathBuf = h.home().join(".claude").join("settings.json");
    let body = read(&settings);
    assert!(body.contains(GATEWAY_URL), "base URL missing: {body}");
    assert!(
        body.contains("X-Gate-Api-Key"),
        "gate key header missing: {body}"
    );
    assert!(
        body.contains("X-Gate-Upstream-Url"),
        "upstream header missing: {body}"
    );
    assert!(body.contains(API_KEY), "api key value missing: {body}");

    h.run_ok(&["disconnect", "claude-code"]);

    let after = fs::read_to_string(&settings).unwrap_or_default();
    assert!(
        !after.contains("X-Gate-Api-Key"),
        "gate residue left behind: {after}"
    );
    assert!(
        !after.contains(GATEWAY_URL),
        "gateway URL left behind: {after}"
    );
}

#[test]
fn codex_connect_then_disconnect() {
    let h = Harness::new();
    let codex_dir = h.home().join(".codex");
    fs::create_dir_all(&codex_dir).unwrap();
    // API-key login mode → base_url gets the `/v1` suffix, upstream api.openai.com.
    fs::write(codex_dir.join("auth.json"), r#"{"auth_mode":"apikey"}"#).unwrap();
    h.login();

    h.run_ok(&["connect", "codex"]);

    let config = codex_dir.join("config.toml");
    let body = read(&config);
    assert!(
        body.contains(r#"model_provider = "gate""#),
        "pointer not set: {body}"
    );
    assert!(
        body.contains(&format!("{GATEWAY_URL}/v1")),
        "base URL missing: {body}"
    );
    assert!(
        body.contains("requires_openai_auth = true"),
        "requires_openai_auth not set: {body}"
    );
    assert!(
        body.contains("X-Gate-Api-Key"),
        "gate key header missing: {body}"
    );
    assert!(body.contains(API_KEY), "api key value missing: {body}");

    h.run_ok(&["disconnect", "codex"]);

    let after = fs::read_to_string(&config).unwrap_or_default();
    assert!(
        !after.contains("X-Gate-Api-Key"),
        "gate residue left behind: {after}"
    );
    assert!(
        !after.contains("model_providers.gate"),
        "gate provider left behind: {after}"
    );
}

/// A hand-written [model_providers.gate] (the manual PAYG setup) must be
/// adopted on connect - replaced with the managed shape - not rejected,
/// and disconnect must remove the pointer rather than "restore" the
/// hand-written `model_provider = "gate"`.
#[test]
fn codex_connect_adopts_manual_gate_block() {
    let h = Harness::new();
    let codex_dir = h.home().join(".codex");
    fs::create_dir_all(&codex_dir).unwrap();
    fs::write(codex_dir.join("auth.json"), r#"{"auth_mode":"apikey"}"#).unwrap();
    h.login();

    let config = codex_dir.join("config.toml");
    fs::write(
        &config,
        r#"model_provider = "gate"
model = "anthropic/claude-opus-4-8"

[model_providers.gate]
name = "Constellation Gate"
base_url = "https://old.gateway.test/v1"
wire_api = "responses"

[model_providers.gate.http_headers]
"X-Gate-Api-Key" = "sk-gw-stale-manual-key"
"#,
    )
    .unwrap();

    h.run_ok(&["connect", "codex"]);

    let body = read(&config);
    assert!(
        body.contains(&format!("{GATEWAY_URL}/v1")),
        "managed base URL missing: {body}"
    );
    assert!(
        !body.contains("sk-gw-stale-manual-key"),
        "stale manual key left behind: {body}"
    );
    assert!(
        body.contains("requires_openai_auth = true"),
        "requires_openai_auth not set: {body}"
    );
    assert!(body.contains(API_KEY), "api key value missing: {body}");
    // The user's unrelated keys survive the adoption.
    assert!(
        body.contains(r#"model = "anthropic/claude-opus-4-8""#),
        "unrelated model key clobbered: {body}"
    );

    h.run_ok(&["disconnect", "codex"]);

    let after = fs::read_to_string(&config).unwrap_or_default();
    assert!(
        !after.contains(r#"model_provider = "gate""#),
        "hand-written gate pointer restored: {after}"
    );
    assert!(
        !after.contains("model_providers.gate"),
        "gate provider left behind: {after}"
    );
    assert!(
        !after.contains("_gate_connect"),
        "gate marker left behind: {after}"
    );
}

#[test]
fn opencode_connect_then_disconnect() {
    let h = Harness::new();
    let oc_dir = h.home().join(".config").join("opencode");
    fs::create_dir_all(&oc_dir).unwrap();
    // Seed a known provider so connect has something to route through Gate.
    let config = oc_dir.join("opencode.json");
    fs::write(&config, r#"{"provider":{"anthropic":{}}}"#).unwrap();
    h.login();

    h.run_ok(&["connect", "opencode"]);

    let body = read(&config);
    assert!(body.contains(GATEWAY_URL), "base URL missing: {body}");
    assert!(
        body.contains("X-Gate-Api-Key"),
        "gate key header missing: {body}"
    );
    assert!(body.contains(API_KEY), "api key value missing: {body}");

    h.run_ok(&["disconnect", "opencode"]);

    let after = read(&config);
    assert!(
        !after.contains("X-Gate-Api-Key"),
        "gate residue left behind: {after}"
    );
    assert!(
        !after.contains(GATEWAY_URL),
        "gateway URL left behind: {after}"
    );
}

#[test]
fn openclaw_connect_then_disconnect() {
    let h = Harness::new();
    let oc_dir = h.home().join(".openclaw");
    fs::create_dir_all(&oc_dir).unwrap();
    // Seed a known provider so connect has something to route through Gate.
    // JSON5 (comment + trailing comma) to confirm the tolerant parse path.
    let config = oc_dir.join("openclaw.json");
    fs::write(
        &config,
        "{\n  // user config\n  models: { providers: { anthropic: {}, } },\n}\n",
    )
    .unwrap();
    h.login();

    h.run_ok(&["connect", "openclaw"]);

    let body = read(&config);
    assert!(
        body.contains(&format!("{GATEWAY_URL}/v1")),
        "base URL missing: {body}"
    );
    assert!(
        body.contains("X-Gate-Api-Key"),
        "gate key header missing: {body}"
    );
    assert!(body.contains(API_KEY), "api key value missing: {body}");

    h.run_ok(&["disconnect", "openclaw"]);

    let after = read(&config);
    assert!(
        !after.contains("X-Gate-Api-Key"),
        "gate residue left behind: {after}"
    );
    assert!(
        !after.contains(GATEWAY_URL),
        "gateway URL left behind: {after}"
    );
}

#[test]
fn openclaw_status_reports_connected_then_drift() {
    let h = Harness::new();
    let oc_dir = h.home().join(".openclaw");
    fs::create_dir_all(&oc_dir).unwrap();
    let config = oc_dir.join("openclaw.json");
    // JSON5 with unquoted keys — the un-gated starting point.
    let seed = "{\n  models: { providers: { anthropic: {} } },\n}\n";
    fs::write(&config, seed).unwrap();
    h.login();

    h.run_ok(&["connect", "openclaw"]);
    let status = h.run_ok(&["status", "openclaw"]);
    assert!(
        status.contains("connected"),
        "expected connected after connect, got: {status}"
    );

    // Hand-edit: revert the config to the un-gated seed but leave the sidecar
    // state in place — exactly the drift a user causes by editing the provider
    // block back by hand. `status` must report drift, not connected.
    fs::write(&config, seed).unwrap();
    let status = h.run_ok(&["status", "openclaw"]);
    assert!(
        status.contains("drifted"),
        "expected drift after hand-edit, got: {status}"
    );
}
