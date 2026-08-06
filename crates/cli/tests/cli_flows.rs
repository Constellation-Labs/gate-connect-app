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
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use tempfile::TempDir;

const GATEWAY_URL: &str = "https://mock.gateway.test";
const API_KEY: &str = "sk-gw-ci-test";
/// A stable relay port to seed so `connect` can point tool configs at the
/// loopback relay. No engine actually runs in these tests - we only assert the
/// config the CLI writes, which uses the persisted port via `relay_base_url()`.
const RELAY_PORT: u16 = 8977;
const RELAY_URL: &str = "http://127.0.0.1:8977";

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

    /// Run the CLI and assert it exited non-zero, returning stderr. For paths
    /// where the refusal *is* the behaviour under test.
    fn run_err(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            !out.status.success(),
            "`gate-connect {}` unexpectedly succeeded\nstdout: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout),
        );
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    /// Sign in so `connect` has a gateway URL on disk + a key in the store, and
    /// seed a persisted relay port so `connect` can point tool configs at the
    /// loopback relay (the app would set this by enabling the proxy).
    fn login(&self) {
        self.run_ok(&["login", "--base-url", GATEWAY_URL, "--api-key", API_KEY]);
        let proxy_dir = self
            .home()
            .join("app-support")
            .join("Gate Connect")
            .join("proxy");
        fs::create_dir_all(&proxy_dir).unwrap();
        fs::write(proxy_dir.join("relay-port"), RELAY_PORT.to_string()).unwrap();
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
    assert!(body.contains(RELAY_URL), "relay base URL missing: {body}");
    assert!(
        !body.contains("X-Gate-Upstream-Url"),
        "no Gate header may be written - the relay derives the upstream from \
         the slug in the base URL: {body}"
    );
    // No credential is ever written - the relay injects it live.
    assert!(
        !body.contains("X-Gate-Api-Key"),
        "credential must not be written to config: {body}"
    );
    assert!(
        !body.contains(API_KEY),
        "api key value must not be written: {body}"
    );

    h.run_ok(&["disconnect", "claude-code"]);

    let after = fs::read_to_string(&settings).unwrap_or_default();
    assert!(
        !after.contains("X-Gate-Upstream-Url"),
        "gate residue left behind: {after}"
    );
    assert!(!after.contains(RELAY_URL), "relay URL left behind: {after}");
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
        body.contains(&format!("{RELAY_URL}/openai/v1")),
        "relay base URL missing: {body}"
    );
    assert!(
        body.contains("requires_openai_auth = true"),
        "requires_openai_auth not set: {body}"
    );
    // No credential is ever written - the relay injects it live.
    assert!(
        !body.contains("X-Gate-Api-Key"),
        "credential must not be written to config: {body}"
    );
    assert!(
        !body.contains(API_KEY),
        "api key value must not be written: {body}"
    );

    h.run_ok(&["disconnect", "codex"]);

    let after = fs::read_to_string(&config).unwrap_or_default();
    assert!(
        !after.contains("X-Gate-Api-Key"),
        "gate residue left behind: {after}"
    );
    assert!(!after.contains(RELAY_URL), "relay URL left behind: {after}");
    assert!(
        !after.contains(r#"model_provider = "gate""#),
        "gate pointer left behind: {after}"
    );
    // The provider name survives as a passthrough stub (api-key mode → /v1) so
    // Codex threads started while routed can still resume, going direct.
    assert!(
        after.contains(r#"base_url = "https://api.openai.com/v1""#),
        "passthrough stub missing: {after}"
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
        body.contains(&format!("{RELAY_URL}/openai/v1")),
        "managed relay base URL missing: {body}"
    );
    assert!(
        !body.contains("sk-gw-stale-manual-key"),
        "stale manual key left behind: {body}"
    );
    assert!(
        body.contains("requires_openai_auth = true"),
        "requires_openai_auth not set: {body}"
    );
    // The adopted block carries no credential - the relay injects it live.
    assert!(
        !body.contains(API_KEY),
        "api key value must not be written: {body}"
    );
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
    // The adopted block is replaced by the passthrough stub, not deleted, but
    // it must keep none of the hand-written routing.
    assert!(
        !after.contains("old.gateway.test") && !after.contains("X-Gate-Api-Key"),
        "hand-written gate routing left behind: {after}"
    );
    assert!(
        !after.contains("previous_model_provider"),
        "gate undo-log keys left behind: {after}"
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
    assert!(body.contains(RELAY_URL), "relay base URL missing: {body}");
    assert!(
        !body.contains("X-Gate-Upstream-Url"),
        "no Gate header may be written - the relay derives the upstream from \
         the slug in the base URL: {body}"
    );
    // No credential is ever written - the relay injects it live.
    assert!(
        !body.contains("X-Gate-Api-Key"),
        "credential must not be written to config: {body}"
    );
    assert!(
        !body.contains(API_KEY),
        "api key value must not be written: {body}"
    );

    h.run_ok(&["disconnect", "opencode"]);

    let after = read(&config);
    assert!(
        !after.contains("X-Gate-Upstream-Url"),
        "gate residue left behind: {after}"
    );
    assert!(!after.contains(RELAY_URL), "relay URL left behind: {after}");
}

#[test]
fn hermes_refuses_to_connect_without_a_running_proxy() {
    // Hermes now routes through the proxy engine, so connect requires a live
    // one - pointing HTTPS_PROXY at a dead address would break its requests
    // rather than merely un-routing them. That refusal is the only part of the
    // flow reachable from the CLI (a live engine needs a trusted CA, which
    // needs privileges); the round trip is covered in
    // `crates/core/tests/disconnect_zero_residue.rs`.
    let h = Harness::new();
    // detect() wants the launcher, which the installer drops in ~/.local/bin -
    // a leftover config dir alone no longer counts as installed.
    let bin = h.home().join(".local").join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("hermes"), "#!/bin/sh\n").unwrap();
    let hermes_dir = h.home().join(".hermes");
    fs::create_dir_all(&hermes_dir).unwrap();
    let config = hermes_dir.join("config.yaml");
    let seed = "model:\n  provider: custom\n  base_url: https://openrouter.ai/api/v1\n";
    fs::write(&config, seed).unwrap();
    h.login();

    let err = h.run_err(&["connect", "hermes"]);
    assert!(
        err.contains("proxy is not running"),
        "expected a refusal naming the proxy, got: {err}"
    );

    // config.yaml is no longer part of this integration at all, and a refused
    // connect must leave no .env behind either.
    assert_eq!(
        read(&config),
        seed,
        "config.yaml must never be touched by the Hermes integration"
    );
    assert!(
        !hermes_dir.join(".env").exists(),
        "a refused connect must not create .env"
    );

    // Disconnect on an unmanaged tool is a no-op, not an error.
    h.run_ok(&["disconnect", "hermes"]);
}
#[test]
fn openclaw_connect_is_a_no_op_without_routing() {
    // Complements `openclaw_refuses_to_connect_without_a_running_proxy`: the
    // refusal must also leave no sidecar, so a later `disconnect` has nothing
    // to undo and `status` does not claim the tool is managed.
    let h = Harness::new();
    let oc_dir = h.home().join(".openclaw");
    fs::create_dir_all(&oc_dir).unwrap();
    let config = oc_dir.join("openclaw.json");
    fs::write(
        &config,
        "{\n  // user config\n  models: { providers: { anthropic: {}, } },\n}\n",
    )
    .unwrap();
    h.login();

    let _ = h.run_err(&["connect", "openclaw"]);

    let sidecar = h
        .home()
        .join("app-support")
        .join("Gate Connect")
        .join("openclaw-state.json");
    assert!(
        !sidecar.exists(),
        "a refused connect must not write a sidecar"
    );

    // Disconnect on an unmanaged tool is a no-op, not an error.
    h.run_ok(&["disconnect", "openclaw"]);
    let status = h.run_ok(&["status", "openclaw"]);
    assert!(
        !status.contains("connected"),
        "must not report connected after a refused connect, got: {status}"
    );
}

#[test]
fn openclaw_refuses_to_connect_without_a_running_proxy() {
    // OpenClaw is the one integration that hands its *entire* egress to Gate:
    // managed proxy mode force-clears no_proxy and has no bypass list. Pointing
    // it at an engine that is not running would take the tool's whole network
    // down rather than degrading its inference, so connect must refuse. That
    // refusal is the safety-critical property, and it is the only part of the
    // flow reachable here - a live engine needs a trusted CA, which needs
    // privileges. The connect/disconnect round trip is covered in
    // `crates/core/tests/disconnect_zero_residue.rs`, which seeds the engine
    // port in-process.
    let h = Harness::new();
    let oc_dir = h.home().join(".openclaw");
    fs::create_dir_all(&oc_dir).unwrap();
    let config = oc_dir.join("openclaw.json");
    let seed = "{\n  // user config\n  models: { providers: { anthropic: {}, } },\n}\n";
    fs::write(&config, seed).unwrap();
    h.login();

    let err = h.run_err(&["connect", "openclaw"]);
    assert!(
        err.contains("proxy is not running"),
        "expected a refusal naming the proxy, got: {err}"
    );

    // A refused connect must leave the config byte-identical - no half-written
    // proxy block, and nothing to clean up.
    assert_eq!(
        read(&config),
        seed,
        "a refused connect must not touch the config"
    );
    assert!(
        !oc_dir.join(".env").exists(),
        "a refused connect must not create the CA env file"
    );
}

/// Serve one canned JSON token response on a loopback endpoint, standing in for
/// Cognito's `/oauth2/token`. Returns the URL to feed the token-endpoint seam.
fn spawn_token_mock() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind token mock");
    let addr = listener.local_addr().expect("token mock addr");
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut tmp = [0u8; 2048];
            let _ = stream.read(&mut tmp); // drain the request; we don't assert on it
            let body = r#"{"access_token":"at-cli","refresh_token":"rt-cli","expires_in":3600,"token_type":"Bearer"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body,
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("http://{addr}/oauth2/token")
}

/// Serve one canned `/v1/me/orgs` response (a single org, so `login --oauth`
/// auto-selects it). Returns the URL to feed the orgs-endpoint seam.
fn spawn_orgs_mock() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind orgs mock");
    let addr = listener.local_addr().expect("orgs mock addr");
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut tmp = [0u8; 2048];
            let _ = stream.read(&mut tmp);
            let body = r#"{"user":{"id":"u-cli","email":"cli@example.test"},"orgs":[{"orgId":"org-cli-1","name":"CLI Org","slug":"cli-org","role":"owner"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body,
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("http://{addr}/v1/me/orgs")
}

/// `login --oauth`: the CLI prints the authorize URL and waits on a loopback
/// redirect; we play the browser (hit the callback with a matching state), the
/// mock token endpoint answers the exchange, and the account records OAuth mode.
/// Hermetic: temp home + file-backed secrets, the Cognito config supplied via
/// the runtime env seam, and the token endpoint redirected to the mock.
#[test]
fn login_oauth_captures_redirect_selects_org_and_records_mode() {
    let h = Harness::new();
    let token_endpoint = spawn_token_mock();
    let orgs_endpoint = spawn_orgs_mock();

    let mut child = Command::new(env!("CARGO_BIN_EXE_gate-connect"))
        .args(["login", "--base-url", GATEWAY_URL, "--oauth"])
        .env("GATE_CONNECT_TEST_HOME", h.home())
        .env("GATE_CONNECT_TEST_SECRETS", h.secrets.path())
        .env("GATE_CONNECT_TEST_TOKEN_ENDPOINT", &token_endpoint)
        .env("GATE_CONNECT_TEST_ORGS_ENDPOINT", &orgs_endpoint)
        .env("GATE_COGNITO_HOSTED_DOMAIN", "auth.example.test")
        .env("GATE_COGNITO_CLIENT_ID", "client-cli-test")
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn gate-connect login --oauth");

    // Drain the CLI's stdout to EOF on a thread (so its final "Signed in" print
    // never hits a broken pipe), forwarding the authorize URL back over a
    // channel. The CLI prints that URL, then blocks on the loopback callback.
    let stdout = child.stdout.take().expect("child stdout");
    let (tx, rx) = mpsc::channel();
    let drain = thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut tx = Some(tx);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if line.contains("/oauth2/authorize") {
                if let (Some(idx), Some(tx)) = (line.find("http"), tx.take()) {
                    let _ = tx.send(line[idx..].trim().to_string());
                }
            }
        }
    });
    let authorize_url = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("CLI did not print an authorize URL");

    // Play the browser: echo the state back on the loopback callback with a code.
    let url = reqwest::Url::parse(&authorize_url).expect("parse authorize URL");
    let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    let state = params.get("state").expect("state param").clone();
    let mut callback = reqwest::Url::parse(params.get("redirect_uri").expect("redirect_uri param"))
        .expect("parse redirect_uri");
    callback
        .query_pairs_mut()
        .append_pair("code", "auth-code-cli")
        .append_pair("state", &state);
    reqwest::blocking::get(callback).expect("hit loopback callback");

    let status = child.wait().expect("wait for CLI");
    let _ = drain.join();
    assert!(status.success(), "login --oauth exited with {status}");

    // The account records OAuth mode - proof the round-trip completed, the
    // token exchange stored a bundle, and set_auth_mode ran against the account.
    let account_json = read(
        &h.home()
            .join("app-support")
            .join("Gate Connect")
            .join("account.json"),
    );
    assert!(
        account_json.contains("\"auth_mode\": \"oauth\""),
        "account.json should record OAuth mode: {account_json}"
    );
    // The single org was auto-selected and persisted (its UUID feeds
    // X-Gate-Org-Id on every gateway request).
    assert!(
        account_json.contains("\"org_id\": \"org-cli-1\""),
        "account.json should record the selected org: {account_json}"
    );
}
