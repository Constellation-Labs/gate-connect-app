//! Hermetic test for keychain chunking: an oversized secret is transparently
//! split across multiple credential entries and reassembled byte-for-byte on
//! read, and no chunk entries are left orphaned across overwrites or deletes -
//! the "nothing left behind in the secret store" contract the sign-out and
//! reconcile paths rely on.
//!
//! Backed by the file seam (`GATE_CONNECT_TEST_SECRETS`) so it never touches the
//! real OS secret store. One test function because that seam is a process-global
//! env var. The split path runs on every platform , which is
//! why this is exercisable here rather than only on Windows.

use gate_connect_core::keychain;

fn temp_secrets_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gate-connect-keychain-chunking-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp secrets dir");
    dir
}

/// Count on-disk entry files for `service`/`account` in the file seam: the base
/// entry plus any `#gck-chunk#N` siblings. The seam names files
/// `{service}__{account}` with `/ \ :` folded to `_` (see `keychain::secret_file`).
fn entry_files(dir: &std::path::Path, service: &str, account: &str) -> usize {
    let base = format!("{service}__{account}").replace(['/', '\\', ':'], "_");
    let chunk_prefix = format!("{base}#gck-chunk#");
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name == base || name.starts_with(&chunk_prefix)
        })
        .count()
}

#[test]
fn oversized_secret_chunks_reassembles_and_leaves_no_residue() {
    let dir = temp_secrets_dir();
    std::env::set_var("GATE_CONNECT_TEST_SECRETS", &dir);

    let service = "ai.constellation.gate-connect.test.chunking";
    let account = "acct";

    // Small value: single entry, round-trips unchanged.
    keychain::set(service, account, "small").unwrap();
    assert_eq!(
        keychain::get(service, account).unwrap().as_deref(),
        Some("small")
    );
    assert_eq!(
        entry_files(&dir, service, account),
        1,
        "a small value should be a single entry"
    );

    // Large value (> MAX_CHUNK_CHARS = 1024): spans multiple entries and is
    // reassembled byte-for-byte on read.
    let large = "abcd".repeat(1000); // 4000 chars -> 4 chunks + 1 manifest
    keychain::set(service, account, &large).unwrap();
    assert_eq!(
        keychain::get(service, account).unwrap().as_deref(),
        Some(large.as_str())
    );
    assert!(
        entry_files(&dir, service, account) > 1,
        "a large value should span multiple entries"
    );

    // Overwrite large -> small: the chunk siblings must be cleaned up.
    keychain::set(service, account, "tiny").unwrap();
    assert_eq!(
        keychain::get(service, account).unwrap().as_deref(),
        Some("tiny")
    );
    assert_eq!(
        entry_files(&dir, service, account),
        1,
        "shrinking to a small value must leave no orphaned chunks"
    );

    // Overwrite small -> large again, then delete: everything is removed.
    keychain::set(service, account, &large).unwrap();
    assert!(keychain::delete(service, account).unwrap());
    assert_eq!(keychain::get(service, account).unwrap(), None);
    assert_eq!(
        entry_files(&dir, service, account),
        0,
        "delete must remove the manifest and every chunk"
    );

    std::env::remove_var("GATE_CONNECT_TEST_SECRETS");
    let _ = std::fs::remove_dir_all(&dir);
}
