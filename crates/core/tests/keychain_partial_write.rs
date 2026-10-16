//! Hermetic test for chunk entries no manifest names: what a chunked
//! `keychain::set` leaves when it fails or is cut short after some chunks and
//! before the manifest. Those are pieces of the secret - for the OAuth bundle,
//! of its refresh token - and sign-out used to leave them in the store for
//! good, because `delete` only found chunks through a manifest.
//!
//! Backed by the file seam (`GATE_CONNECT_TEST_SECRETS`), so it never touches
//! the real OS secret store. One test function because that seam is a
//! process-global env var, and its own binary so it cannot race
//! `keychain_chunking`'s.

use gate_connect_core::keychain;

const SERVICE: &str = "ai.constellation.gate-connect.test.partial-write";
const ACCOUNT: &str = "acct";

fn temp_secrets_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gate-connect-keychain-partial-write-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp secrets dir");
    dir
}

/// The seam's file for one entry, named as `keychain::secret_file` names it.
fn entry_path(dir: &std::path::Path, account: &str) -> std::path::PathBuf {
    dir.join(format!("{SERVICE}__{account}").replace(['/', '\\', ':'], "_"))
}

fn chunk_path(dir: &std::path::Path, i: usize) -> std::path::PathBuf {
    entry_path(dir, &format!("{ACCOUNT}#gck-chunk#{i}"))
}

/// Entry files for `ACCOUNT`: the base entry plus any chunk siblings. Files
/// only, so a directory planted to make a write fail is not counted.
fn entry_files(dir: &std::path::Path) -> usize {
    let base = entry_path(dir, ACCOUNT);
    let base = base.file_name().unwrap().to_string_lossy().into_owned();
    let chunk_prefix = format!("{base}#gck-chunk#");
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name == base || name.starts_with(&chunk_prefix)
        })
        .count()
}

/// Chunks written straight to the seam with no manifest: the store as a write
/// killed between its chunks and its manifest leaves it.
fn plant_orphans(dir: &std::path::Path, range: std::ops::Range<usize>) {
    for i in range {
        std::fs::write(chunk_path(dir, i), "piece-of-a-secret").unwrap();
    }
}

#[test]
fn chunks_no_manifest_names_are_never_left_behind() {
    let dir = temp_secrets_dir();
    std::env::set_var("GATE_CONNECT_TEST_SECRETS", &dir);
    let large = "abcd".repeat(1000); // 4000 chars -> 4 chunks + 1 manifest

    // A write that fails partway takes back the chunks it wrote. A directory
    // where chunk 2's file goes makes that write fail after chunks 0 and 1.
    std::fs::create_dir(chunk_path(&dir, 2)).unwrap();
    assert!(keychain::set(SERVICE, ACCOUNT, &large).is_err());
    assert_eq!(keychain::get(SERVICE, ACCOUNT).unwrap(), None);
    assert_eq!(
        entry_files(&dir),
        0,
        "a failed chunked write must not leave the chunks it wrote"
    );
    std::fs::remove_dir(chunk_path(&dir, 2)).unwrap();

    // And only those. Chunk 3 stands in for the chunks of a write that
    // completed meanwhile, which a cleanup sweeping on past its own writes
    // would delete. Chunk 2 is a dangling symlink rather than a directory:
    // writing through it fails, but it deletes cleanly, so nothing but the
    // cleanup's own bound stops a sweep from reaching chunk 3.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.join("no-such-dir").join("x"), chunk_path(&dir, 2)).unwrap();
        plant_orphans(&dir, 3..4);
        assert!(keychain::set(SERVICE, ACCOUNT, &large).is_err());
        assert!(!chunk_path(&dir, 0).exists() && !chunk_path(&dir, 1).exists());
        assert!(
            chunk_path(&dir, 3).exists(),
            "a failed write's cleanup must not reach past the chunks it wrote"
        );
        std::fs::remove_file(chunk_path(&dir, 2)).unwrap();
        std::fs::remove_file(chunk_path(&dir, 3)).unwrap();
        assert_eq!(entry_files(&dir), 0);
    }

    // A sweep that errors does not fail the delete it is part of. A directory
    // where chunk 0's file goes makes the sweep's first lookup an error.
    keychain::set(SERVICE, ACCOUNT, "small").unwrap();
    std::fs::create_dir(chunk_path(&dir, 0)).unwrap();
    assert!(
        keychain::delete(SERVICE, ACCOUNT).unwrap(),
        "an erroring sweep must not fail a sign-out"
    );
    assert_eq!(keychain::get(SERVICE, ACCOUNT).unwrap(), None);
    std::fs::remove_dir(chunk_path(&dir, 0)).unwrap();

    // A write killed outright cannot clean up after itself, so sign-out must.
    plant_orphans(&dir, 0..3);
    assert!(
        !keychain::delete(SERVICE, ACCOUNT).unwrap(),
        "orphaned chunks are not a stored secret"
    );
    assert_eq!(
        entry_files(&dir),
        0,
        "delete must remove chunks no manifest names"
    );

    // And so must the next write, whatever its size.
    plant_orphans(&dir, 0..3);
    keychain::set(SERVICE, ACCOUNT, "small").unwrap();
    assert_eq!(
        keychain::get(SERVICE, ACCOUNT).unwrap().as_deref(),
        Some("small")
    );
    assert_eq!(
        entry_files(&dir),
        1,
        "a small write must sweep orphans it does not overwrite"
    );

    // Orphans past the end of a live manifest go too. A build without this fix
    // leaves exactly that when it writes a shorter value over a longer write
    // that failed: it overwrites the low chunks and never looks at the rest.
    keychain::set(SERVICE, ACCOUNT, &large).unwrap();
    plant_orphans(&dir, 4..6);
    assert_eq!(
        keychain::get(SERVICE, ACCOUNT).unwrap().as_deref(),
        Some(large.as_str()),
        "orphans past the manifest do not disturb the read"
    );
    assert!(keychain::delete(SERVICE, ACCOUNT).unwrap());
    assert_eq!(
        entry_files(&dir),
        0,
        "delete must remove orphans past the manifest's last chunk"
    );

    // A delete cut short among the chunks leaves what the next one sweeps. The
    // chunks go last to first, so a directory where chunk 1's file goes stops
    // the delete with only chunk 0 left: a run from 0 the sweep reaches. Going
    // first to last would leave chunks 2 and 3 behind a gap at 0.
    keychain::set(SERVICE, ACCOUNT, &large).unwrap();
    std::fs::remove_file(chunk_path(&dir, 1)).unwrap();
    std::fs::create_dir(chunk_path(&dir, 1)).unwrap();
    assert!(keychain::delete(SERVICE, ACCOUNT).is_err());
    std::fs::remove_dir(chunk_path(&dir, 1)).unwrap();
    assert!(!keychain::delete(SERVICE, ACCOUNT).unwrap());
    assert_eq!(
        entry_files(&dir),
        0,
        "a delete cut short among the chunks must leave a run the next one sweeps"
    );

    std::env::remove_var("GATE_CONNECT_TEST_SECRETS");
    let _ = std::fs::remove_dir_all(&dir);
}
