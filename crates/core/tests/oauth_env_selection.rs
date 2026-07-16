//! `OAuthConfig::from_build_env` resolves the production vs staging Cognito
//! pool from the currently-selected gateway host. Exercises the real
//! `account.json` read against a throwaway data dir (via the
//! `GATE_CONNECT_TEST_HOME` seam), in its own test binary so the process-global
//! data-dir + env-var overrides can't leak into other tests.

use std::fs;
use std::path::PathBuf;

use gate_connect_core::{account, oauth::OAuthConfig};

/// Point `app_support_dir()` at a fresh temp dir for the duration of a test, so
/// `account.json` resolves there on every OS. Uses the `GATE_CONNECT_TEST_HOME`
/// seam rather than the process-global mutex override, because `app_support_dir`
/// consults that env var first: a mutex override would be silently bypassed when
/// the env var is already set in the environment (CI). Clears the seam and
/// deletes the dir on drop.
struct TempDataDir {
    dir: PathBuf,
}

impl TempDataDir {
    fn set() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "gate-connect-oauth-env-test-{}-{}",
            std::process::id(),
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);
        TempDataDir { dir }
    }
}

impl Drop for TempDataDir {
    fn drop(&mut self) {
        std::env::remove_var("GATE_CONNECT_TEST_HOME");
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// The active gateway host picks the pool; unknown/self-hosted hosts fall back
/// to production. Distinct sentinels per pool tell us which branch was taken.
#[test]
fn resolves_pool_from_gateway_host() {
    let _data = TempDataDir::set();

    std::env::set_var("GATE_COGNITO_HOSTED_DOMAIN", "prod.auth.test");
    std::env::set_var("GATE_COGNITO_CLIENT_ID", "prod-client");
    std::env::set_var("GATE_COGNITO_HOSTED_DOMAIN_STAGING", "staging.auth.test");
    std::env::set_var("GATE_COGNITO_CLIENT_ID_STAGING", "staging-client");

    // Staging gateway → staging pool.
    account::save("https://gateway-staging.constellationgate.ai", None).unwrap();
    let cfg = OAuthConfig::from_build_env().expect("config resolves for staging gateway");
    assert_eq!(cfg.hosted_domain, "staging.auth.test");
    assert_eq!(cfg.client_id, "staging-client");

    // Production gateway → prod pool.
    account::switch_gateway("https://gateway.constellationgate.ai").unwrap();
    let cfg = OAuthConfig::from_build_env().expect("config resolves for prod gateway");
    assert_eq!(cfg.hosted_domain, "prod.auth.test");
    assert_eq!(cfg.client_id, "prod-client");

    // Unknown / self-hosted host → prod pool (safe default).
    account::switch_gateway("https://gate.example.com").unwrap();
    let cfg = OAuthConfig::from_build_env().expect("config resolves for unknown gateway");
    assert_eq!(cfg.hosted_domain, "prod.auth.test");
    assert_eq!(cfg.client_id, "prod-client");
}
