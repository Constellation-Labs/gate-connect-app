//! The chat surfaces (`claude-web`, `chatgpt-apps`) are offered whichever
//! gateway the account points at. They were gated to staging while the
//! gateway-side classification was staging-only; this pins the state after that
//! gate was lifted. Exercises the real `account.json` read through
//! `proxy::config`, in its own test binary because the data-dir seam is
//! process-global (same reason as `oauth_env_selection.rs`).

use std::fs;
use std::path::PathBuf;

use gate_connect_core::proxy::{config, ProxyDomain};
use gate_connect_core::{account, keychain};

/// The two chat-protocol slugs, plus the relay-only subscription endpoint that
/// was never gated - it must stay available, as it always was.
const CHAT_SURFACES: [&str; 2] = ["claude-web", "chatgpt-apps"];
const RELAY_CHAT: &str = "chatgpt";

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
            "gate-connect-chat-surface-test-{}-{}",
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
fn the_chat_surfaces_are_offered_in_both_environments() {
    let _data = TempDataDir::set();
    // `switch_gateway` deletes the stored key; keep that off the OS secret
    // store, which is absent on headless CI.
    keychain::use_in_memory_backend();

    // First run, no account: offerable, and off. Both halves matter - these
    // carry a session credential rather than a brokered key, so the row has to
    // exist without routing anything until its own switch is flipped.
    for slug in CHAT_SURFACES {
        let d = find(&config::load_domains().unwrap(), slug);
        assert!(
            d.supported,
            "{slug} must be offered with no account selected"
        );
        assert!(!d.enabled, "{slug} must default to off");
    }

    // Production, which is where the gate used to take these away.
    account::save("https://gateway.constellationgate.ai", None).unwrap();
    for slug in CHAT_SURFACES {
        assert!(
            find(&config::load_domains().unwrap(), slug).supported,
            "{slug} must be offerable on production"
        );
        let domains = config::set_enabled(slug, true)
            .unwrap_or_else(|e| panic!("enabling {slug} on production: {e:#}"));
        assert!(
            find(&domains, slug).enabled,
            "{slug} must route on production"
        );
    }

    // Staging, with both left enabled on disk from the production session: the
    // persisted flags are environment-independent too, so they carry over
    // rather than needing to be re-enabled.
    account::switch_gateway(&format!("https://{}", account::STAGING_GATEWAY_HOST)).unwrap();
    let domains = config::load_domains().unwrap();
    for slug in CHAT_SURFACES {
        let d = find(&domains, slug);
        assert!(
            d.supported && d.enabled,
            "{slug} must stay on across a gateway switch"
        );
    }

    // Untouched by any of this: the subscription endpoint that predates the
    // gate, and the default-on entry.
    assert!(find(&domains, RELAY_CHAT).supported);
    assert!(
        find(&domains, "anthropic").enabled,
        "and so does the default"
    );
}
