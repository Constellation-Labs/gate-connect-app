//! Codex's provider block under each billing mode.
//!
//! Codex is the one config integration whose file depends on who pays. In BYOK
//! it carries `requires_openai_auth = true`, which is what makes Codex attach
//! its own `codex login` session as the upstream bearer. In PAYG that flag has
//! to be ABSENT: the Codex docs define a provider with neither
//! `requires_openai_auth` nor `env_key` as one that "doesn't require
//! authentication", and sending no `Authorization` at all is the only shape the
//! gateway cannot misread - any non-`sk-gw-` token in that slot is classified as
//! a passthrough credential, which forces BYOK and is then refused for want of
//! an upstream URL.
//!
//! Hermetic: a temp `$HOME` (so `~/.codex/config.toml` is a throwaway) plus the
//! `GATE_CONNECT_TEST_HOME` seam for the account file, and an in-memory
//! keychain. Its own test binary, since both are process-wide.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use gate_connect_core::account::{self, BillingMode};
use gate_connect_core::registry::{find, ConnectInput, Status, ToolId};
use gate_connect_core::{env, keychain};

static LOCK: Mutex<()> = Mutex::new(());

struct TempHome {
    dir: PathBuf,
    prev_home: Option<String>,
}

impl TempHome {
    fn set() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "gate-connect-codex-billing-test-{}-{}",
            std::process::id(),
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &dir);
        // Routes `account.json` (and the rest of the app-support tree) into the
        // same throwaway dir on every OS.
        std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);
        TempHome { dir, prev_home }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        match self.prev_home.take() {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        std::env::remove_var("GATE_CONNECT_TEST_HOME");
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Make `detect()` report Codex as installed: it falls back to checking for
/// `~/.codex`, which is the only hook a test can rely on across platforms.
fn fake_codex_install() {
    fs::create_dir_all(env::home().unwrap().join(".codex")).unwrap();
}

/// `auth.json` with `auth_mode = "apikey"`. BYOK reads this to decide the base
/// URL shape and hard-fails without it; PAYG must never need it.
fn write_auth_json(mode: &str) {
    let path = env::codex_auth_json_path().unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, format!(r#"{{"auth_mode":"{mode}"}}"#)).unwrap();
}

/// Persist the relay port `status()` expects the config to point at. Without
/// it, status stops at "the proxy has not been enabled yet" before it ever
/// looks at the block.
fn seed_relay_port() {
    let path = env::app_support_dir()
        .unwrap()
        .join("proxy")
        .join("relay-port");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, RELAY_PORT.to_string()).unwrap();
}

const RELAY_PORT: u16 = 45981;

fn connect_input(billing_mode: BillingMode) -> ConnectInput {
    ConnectInput {
        gateway_base_url: "https://gw.example.com".to_string(),
        upstream_url: "https://api.openai.com".to_string(),
        billing_mode,
        relay_base_url: Some(format!("http://127.0.0.1:{RELAY_PORT}")),
        engine_proxy_url: Some("http://127.0.0.1:45999".to_string()),
    }
}

fn config_toml() -> String {
    fs::read_to_string(env::codex_config_toml_path().unwrap()).unwrap()
}

/// The account is what `status()` reads the mode from, so it has to exist.
fn sign_in(billing_mode: BillingMode) {
    keychain::use_in_memory_backend();
    account::save("https://gw.example.com", Some("sk-gw-test")).unwrap();
    account::set_billing_mode(billing_mode).unwrap();
}

#[test]
fn byok_writes_requires_openai_auth() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    fake_codex_install();
    write_auth_json("apikey");
    sign_in(BillingMode::Byok);

    find(ToolId::Codex)
        .unwrap()
        .connect(&connect_input(BillingMode::Byok))
        .unwrap();

    let toml = config_toml();
    assert!(
        toml.contains("requires_openai_auth = true"),
        "BYOK relies on Codex attaching its own login: {toml}"
    );
}

/// The flag's absence is the whole PAYG change here, and nothing may replace it:
/// no `env_key`, and no credential of ours written into the user's config.
#[test]
fn payg_writes_an_unauthenticated_provider_block() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    fake_codex_install();
    write_auth_json("apikey");
    sign_in(BillingMode::Payg);

    find(ToolId::Codex)
        .unwrap()
        .connect(&connect_input(BillingMode::Payg))
        .unwrap();

    let toml = config_toml();
    assert!(
        !toml.contains("requires_openai_auth"),
        "PAYG needs Codex to send no Authorization at all: {toml}"
    );
    assert!(
        !toml.contains("env_key"),
        "env_key would make Codex send a provider key instead: {toml}"
    );
    assert!(
        !toml.contains("sk-gw-"),
        "no Gate credential belongs in a tool config: {toml}"
    );
    // The apikey path shape, since the ChatGPT route is a subscription and so
    // never pay-as-you-go.
    assert!(
        toml.contains("base_url = \"http://127.0.0.1:45981/openai/v1\""),
        "PAYG pins the OpenAI /v1 shape: {toml}"
    );
}

/// The reason PAYG cannot go through `read_auth_mode`: a user who never ran
/// `codex login` has no `auth.json`, and BYOK's read of it hard-fails. PAYG has
/// no login to read, so it must connect anyway.
#[test]
fn payg_connects_with_no_auth_json_where_byok_cannot() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    fake_codex_install();
    sign_in(BillingMode::Payg);
    // Deliberately no write_auth_json().

    let integ = find(ToolId::Codex).unwrap();
    assert!(
        integ.connect(&connect_input(BillingMode::Payg)).is_ok(),
        "PAYG must not require a Codex login"
    );
    let err = integ
        .connect(&connect_input(BillingMode::Byok))
        .expect_err("BYOK still needs a login to source its bearer from");
    assert!(
        format!("{err:#}").contains("codex login"),
        "the BYOK error should name the fix: {err:#}"
    );
}

/// Each mode reads the other's block as drift, which is how the app knows a
/// switch means "reconnect Codex" rather than leaving it on a shape that will be
/// refused.
#[test]
fn each_mode_reports_the_others_block_as_drift() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _home = TempHome::set();
    fake_codex_install();
    write_auth_json("apikey");
    sign_in(BillingMode::Byok);
    seed_relay_port();

    let integ = find(ToolId::Codex).unwrap();
    integ.connect(&connect_input(BillingMode::Byok)).unwrap();

    // Flip who pays without reconnecting: the BYOK block on disk now carries a
    // flag PAYG cannot use.
    account::set_billing_mode(BillingMode::Payg).unwrap();
    match integ.status().unwrap() {
        Status::Drifted(reason) => assert!(
            reason.contains("requires_openai_auth"),
            "the drift should name the flag: {reason}"
        ),
        other => panic!("expected drift after a mode switch, got {other}"),
    }

    // Reconnecting into the new mode settles it.
    integ.connect(&connect_input(BillingMode::Payg)).unwrap();
    assert_eq!(integ.status().unwrap(), Status::Connected);

    // And the reverse: a PAYG block under a BYOK account is drift too.
    account::set_billing_mode(BillingMode::Byok).unwrap();
    assert!(matches!(integ.status().unwrap(), Status::Drifted(_)));
}
