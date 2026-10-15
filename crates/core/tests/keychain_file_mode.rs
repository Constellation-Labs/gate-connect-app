//! The file seam (`GATE_CONNECT_TEST_SECRETS`) holds real secrets in plaintext
//! during development, so what it creates has to be owner-only: the directory
//! 0700 and each secret 0600, whatever the umask and wherever the seam points.
//!
//! Its own binary because the seam is a process-global env var.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;

use gate_connect_core::keychain;

const SERVICE: &str = "ai.constellation.gate-connect.test.file-mode";

fn mode(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn the_file_seam_creates_its_store_and_secrets_owner_only() {
    let root = std::env::temp_dir().join(format!(
        "gate-connect-keychain-file-mode-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    // Not created up front: the seam has to make the directory itself.
    let dir = root.join("secrets");
    std::env::set_var("GATE_CONNECT_TEST_SECRETS", &dir);

    keychain::set(SERVICE, "acct", "sk-gw-not-a-real-key").unwrap();
    assert_eq!(mode(&dir), 0o700, "secret store directory");
    let file = dir.join(format!("{SERVICE}__acct"));
    assert_eq!(mode(&file), 0o600, "secret file");

    // An entry that already exists at a looser mode is tightened on rewrite.
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    keychain::set(SERVICE, "acct", "sk-gw-another-key").unwrap();
    assert_eq!(mode(&file), 0o600, "rewritten secret file");
    assert_eq!(
        keychain::get(SERVICE, "acct").unwrap().as_deref(),
        Some("sk-gw-another-key")
    );

    let _ = std::fs::remove_dir_all(&root);
}
