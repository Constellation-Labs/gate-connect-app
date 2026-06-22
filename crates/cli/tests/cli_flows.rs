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
