//! Concurrent writers and readers of one chunked secret never tear it.
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
    std::env::set_var("GATE_CONNECT_TEST_SECRETS", &dir);

    // Different lengths, so a torn read shows as a wrong chunk count as well as
    // mixed content. All past the chunk size, so every write is several entries.
    let values: Arc<Vec<String>> = Arc::new(
        [('a', 3000), ('b', 5200), ('c', 2100), ('d', 4100)]
            .iter()
            .map(|&(ch, n)| ch.to_string().repeat(n))
            .collect(),
    );
    keychain::set(SERVICE, ACCOUNT, &values[0]).expect("seed");

    let stop = Arc::new(AtomicBool::new(false));
    let writers: Vec<_> = (0..values.len())
        .map(|w| {
            let values = values.clone();
            thread::spawn(move || {
                for _ in 0..40 {
                    keychain::set(SERVICE, ACCOUNT, &values[w]).expect("write");
                }
            })
        })
        .collect();
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let (values, stop) = (values.clone(), stop.clone());
            thread::spawn(move || {
                let mut reads = 0;
                while !stop.load(Ordering::Relaxed) {
                    let got = keychain::get(SERVICE, ACCOUNT)
                        .expect("a read never finds a torn secret")
                        .expect("a secret that is only ever overwritten is always there");
                    assert!(
                        values.contains(&got),
                        "read a mix of two writes ({} chars)",
                        got.len()
                    );
                    reads += 1;
                }
                reads
            })
        })
        .collect();

    for w in writers {
        w.join().expect("a writer failed");
    }
    stop.store(true, Ordering::Relaxed);
    for r in readers {
        r.join().expect("a reader saw a torn secret");
    }
    let last = keychain::get(SERVICE, ACCOUNT)
        .expect("readable")
        .expect("present");
    assert!(values.contains(&last));

    keychain::delete(SERVICE, ACCOUNT).expect("delete");
    let leftover = std::fs::read_dir(&dir).expect("list").count();
    assert_eq!(leftover, 0, "no chunk outlives the delete");
    let _ = std::fs::remove_dir_all(&dir);
    std::env::remove_var("GATE_CONNECT_TEST_SECRETS");
}
