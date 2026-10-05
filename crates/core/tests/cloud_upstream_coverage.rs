//! A tool sent to Bedrock is routed and not inspected, read from its real files.
//!
//! AG-931. The unit tests beside each integration cover the parsing; this covers
//! the path the rail actually takes - `registry::find(..).upstream_coverage()`
//! resolving `~/.claude/settings.json` and `~/.hermes/` through
//! `GATE_CONNECT_TEST_HOME`, with no catalog file on disk so the built-in one is
//! used.
//!
//! The enterprise managed settings are machine-wide and cannot be redirected, so
//! a machine that has them could change what Claude Code reports here. None of
//! the CI runners do.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use gate_connect_core::registry::{find, ToolId};

static HOME_LOCK: Mutex<()> = Mutex::new(());

/// Same guard as `config_location.rs`.
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
            "gate-connect-cloud-coverage-test-{}-{}",
            std::process::id(),
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        let prev = std::env::var("GATE_CONNECT_TEST_HOME").ok();
        std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);
        TempHome { dir, prev }
    }

    fn write(&self, rel: &str, body: &str) {
        let path = self.dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
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

#[test]
fn claude_code_on_bedrock_names_the_host_gate_cannot_see() {
    let _lock = HOME_LOCK.lock().unwrap();
    let home = TempHome::set();
    let claude = find(ToolId::ClaudeCode).expect("in the registry");

    assert_eq!(
        claude.upstream_coverage(),
        None,
        "no settings file is plain Anthropic, which its own section covers"
    );

    home.write(
        ".claude/settings.json",
        r#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"1","AWS_REGION":"us-west-2"}}"#,
    );
    let coverage = claude.upstream_coverage().expect("Bedrock is reported");
    assert_eq!(
        coverage.unknown,
        vec!["bedrock-runtime.us-west-2.amazonaws.com"]
    );
    assert!(coverage.switched_off.is_empty(), "{coverage:?}");
}

#[test]
fn hermes_on_bedrock_without_a_url_is_not_reported_as_openrouter() {
    let _lock = HOME_LOCK.lock().unwrap();
    let home = TempHome::set();
    let hermes = find(ToolId::Hermes).expect("in the registry");

    home.write(".hermes/config.yaml", "model:\n  provider: bedrock\n");
    home.write(".hermes/.env", "AWS_REGION=eu-west-2\n");
    let coverage = hermes.upstream_coverage().expect("Bedrock is reported");
    assert!(
        !coverage.defaulted,
        "read from the file, not Hermes' default"
    );
    assert_eq!(
        coverage.unknown,
        vec!["bedrock-runtime.eu-west-2.amazonaws.com"]
    );
    assert!(
        coverage.switched_off.iter().all(|s| s.slug != "openrouter"),
        "the OpenRouter default must not stand in for a Bedrock config: {coverage:?}"
    );
}
