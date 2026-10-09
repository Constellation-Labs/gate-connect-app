//! Concurrent writers, deleters and readers of one chunked secret never tear
//! it, and a delete cut short leaves no secret rather than a broken one.
//!
//! One logical secret is several entries: `keychain::set` deletes the old ones
//! and writes a chunk at a time, manifest last. Two writers interleaving, or a
//! reader between them, used to leave or read a bundle with chunks missing -
//! the "chunk 0 of 5 missing for keyring entry ...oauth-tokens" a Mac reported
//! after two refreshes stored the renewed session at once. The same overlap is
//! what deadlocked the macOS keychain there, which no hermetic test can show;
//! this pins the lock that prevents both.
//!
//! Backed by the file seam (`GATE_CONNECT_TEST_SECRETS`). One test function,
//! because that seam is a process-global env var.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use gate_connect_core::keychain;

const SERVICE: &str = "ai.constellation.gate-connect.test.concurrent";
const ACCOUNT: &str = "someone";

/// Removes the secret store directory and the seam, pass or fail.
struct Seam {
    dir: std::path::PathBuf,
}

impl Drop for Seam {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        std::env::remove_var("GATE_CONNECT_TEST_SECRETS");
    }
}

/// The file the seam keeps chunk `i` of the test secret in (see
/// `keychain::secret_file` and `keychain::chunk_account`).
fn chunk_file(dir: &std::path::Path, i: usize) -> std::path::PathBuf {
    dir.join(format!("{SERVICE}__{ACCOUNT}#gck-chunk#{i}").replace(['/', '\\', ':'], "_"))
}

/// Run `writers` and `readers` against the secret together until the writers
/// finish. Each reader sees either a whole value from `values` or, when
/// `may_be_absent`, nothing; never an error, never a mix.
fn race(values: &Arc<Vec<String>>, writers: Vec<Box<dyn FnOnce() + Send>>, may_be_absent: bool) {
    let stop = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let (values, stop) = (values.clone(), stop.clone());
            thread::spawn(move || {
                let mut reads = 0usize;
                while !stop.load(Ordering::Relaxed) {
                    let got =
                        keychain::get(SERVICE, ACCOUNT).expect("a read never finds a torn secret");
                    match got {
                        Some(got) => assert!(
                            values.contains(&got),
                            "read a mix of two writes ({} chars)",
                            got.len()
                        ),
                        None => assert!(may_be_absent, "a secret only ever overwritten vanished"),
                    }
                    reads += 1;
                }
                reads
            })
        })
        .collect();
    let writers: Vec<_> = writers.into_iter().map(thread::spawn).collect();
    for w in writers {
        w.join().expect("a writer failed");
    }
    stop.store(true, Ordering::Relaxed);
    for r in readers {
        let reads = r.join().expect("a reader saw a torn secret");
        assert!(
            reads > 0,
            "every reader overlapped the writers at least once"
        );
    }
}

#[test]
fn concurrent_writes_and_reads_see_whole_secrets_only() {
    let dir = std::env::temp_dir().join(format!(
        "gate-connect-keychain-concurrent-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp secrets dir");
    let _seam = Seam { dir: dir.clone() };
    std::env::set_var("GATE_CONNECT_TEST_SECRETS", &dir);

    // Different lengths, so a torn read shows as a wrong chunk count as well as
    // mixed content. All past the chunk size, so every write is several entries.
    let values: Arc<Vec<String>> = Arc::new(
        [('a', 3000), ('b', 5200), ('c', 2100), ('d', 4100)]
            .iter()
            .map(|&(ch, n)| ch.to_string().repeat(n))
            .collect(),
    );

    // 1. Writers only: the secret is always there and always whole.
    keychain::set(SERVICE, ACCOUNT, &values[0]).expect("seed");
    let writers = (0..values.len())
        .map(|w| {
            let values = values.clone();
            Box::new(move || {
                for _ in 0..40 {
                    keychain::set(SERVICE, ACCOUNT, &values[w]).expect("write");
                }
            }) as Box<dyn FnOnce() + Send>
        })
        .collect();
    race(&values, writers, false);

    // 2. Sign-out and sign-in racing reads: a delete beside a write. Absent is
    //    a fair answer now; a torn secret is not.
    let writers = (0..2)
        .map(|w| {
            let values = values.clone();
            Box::new(move || {
                for _ in 0..40 {
                    keychain::delete(SERVICE, ACCOUNT).expect("delete");
                    keychain::set(SERVICE, ACCOUNT, &values[w]).expect("write");
                }
            }) as Box<dyn FnOnce() + Send>
        })
        .collect();
    race(&values, writers, true);

    keychain::delete(SERVICE, ACCOUNT).expect("delete");
    let leftover = std::fs::read_dir(&dir).expect("list").count();
    assert_eq!(leftover, 0, "no chunk outlives the delete");

    // 3. A delete cut short partway through the chunks - here a chunk the
    //    store refuses to remove; on a real machine a force-quit. It must leave
    //    no secret, which reads as signed out, rather than a manifest naming
    //    chunks that are gone, which failed every read from then on.
    keychain::set(SERVICE, ACCOUNT, &values[1]).expect("write");
    let stuck = chunk_file(&dir, 1);
    std::fs::remove_file(&stuck).expect("chunk 1 is a file");
    std::fs::create_dir_all(stuck.join("pinned")).expect("pin chunk 1 as a directory");
    assert!(
        keychain::delete(SERVICE, ACCOUNT).is_err(),
        "the delete really was cut short"
    );
    assert_eq!(
        keychain::get(SERVICE, ACCOUNT).expect("a cut-short delete leaves a readable store"),
        None
    );
}
