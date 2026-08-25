//! The chat surfaces (`claude-web`, `chatgpt-apps`) are gated on the account
//! pointing at the staging gateway. Exercises the real `account.json` read
//! through `proxy::config`, in its own test binary because the data-dir seam is
//! process-global (same reason as `oauth_env_selection.rs`).

use std::fs;
use std::path::PathBuf;

use gate_connect_core::proxy::{config, ProxyDomain};
use gate_connect_core::{account, keychain};

/// The two gated slugs, plus one that must not move: `chatgpt` is the
/// relay-only subscription endpoint, which shipped before the gate and is
/// available in both environments.
const GATED: [&str; 2] = ["claude-web", "chatgpt-apps"];
const UNGATED_CHAT: &str = "chatgpt";

/// Point `app_support_dir()` at a fresh temp dir, so `account.json` and
/// `proxy/domains.json` both resolve there on every OS. Uses the
/// `GATE_CONNECT_TEST_HOME` seam rather than the process-global mutex override,
/// because `app_support_dir` consults that env var first.
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
            "gate-connect-chat-gate-test-{}-{}",
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

fn find(domains: &[ProxyDomain], slug: &str) -> ProxyDomain {
    domains
        .iter()
        .find(|d| d.slug == slug)
        .unwrap_or_else(|| panic!("{slug} is in the catalog"))
        .clone()
}

/// One test, not several: they all move the same process-global data-dir seam,
/// so splitting them would let cargo's threads race over which gateway is on
/// disk. The phases run in the order a user meets them.
#[test]
fn the_chat_surfaces_are_staging_only() {
    let _data = TempDataDir::set();
    // `switch_gateway` deletes the stored key; keep that off the OS secret
    // store, which is absent on headless CI.
    keychain::use_in_memory_backend();

    // First run, no account: nothing is selected, so the fallback is
    // production, i.e. offer less rather than more.
    for slug in GATED {
        assert!(
            !find(&config::load_domains().unwrap(), slug).supported,
            "{slug} must not be offered with no account selected"
        );
    }

    // Staging: both surfaces are offerable, and the switch takes.
    account::save(&format!("https://{}", account::STAGING_GATEWAY_HOST), None).unwrap();
    for slug in GATED {
        assert!(
            find(&config::load_domains().unwrap(), slug).supported,
            "{slug} must be offerable on staging"
        );
        let domains = config::set_enabled(slug, true)
            .unwrap_or_else(|e| panic!("enabling {slug} on staging: {e:#}"));
        assert!(find(&domains, slug).enabled, "{slug} must route on staging");
    }

    // Production, with both left enabled on disk from the staging session. The
    // row is gone AND the domain is off: a hidden row still intercepting
    // claude.ai is the state this gate exists to prevent.
    account::switch_gateway("https://gateway.constellationgate.ai").unwrap();
    let domains = config::load_domains().unwrap();
    for slug in GATED {
        let d = find(&domains, slug);
        assert!(!d.supported, "{slug} must not be offered on production");
        assert!(
            !d.enabled,
            "{slug} was enabled on staging and must go inert on production"
        );
        let err = config::set_enabled(slug, true)
            .expect_err("enabling a gated domain on production must fail")
            .to_string();
        assert!(
            err.contains(account::STAGING_GATEWAY_HOST),
            "the refusal must name the environment that has it, got: {err}"
        );
    }

    // The gate is per-slug, not "all chat rows": the subscription endpoint
    // predates it and stays available in both environments.
    assert!(find(&domains, UNGATED_CHAT).supported);
    assert!(
        find(&domains, "anthropic").enabled,
        "and so does the default"
    );

    // Back to staging: the persisted flags were never rewritten, so the
    // surfaces come back on rather than needing to be re-enabled.
    account::switch_gateway(&format!("https://{}", account::STAGING_GATEWAY_HOST)).unwrap();
    let domains = config::load_domains().unwrap();
    for slug in GATED {
        let d = find(&domains, slug);
        assert!(d.supported && d.enabled, "{slug} must come back on staging");
    }
}
