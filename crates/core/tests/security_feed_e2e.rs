//! End-to-end coverage of the live security-event feed's connection behaviour
//! (AG-578).
//!
//! The state machine is the contract under test, not an implementation detail.
//! Three acceptance criteria are decided entirely by what this loop does when the
//! network misbehaves: the feed reports Live / Reconnecting / Offline
//! independently of routing (AC4), it retrieves missed events after reconnecting
//! *without duplicates* (AC5), and a feed that cannot load says so rather than
//! failing silently (AC6).
//!
//! The one behaviour worth stating plainly, because getting it wrong logs the
//! user out of a working app: **only a definite 401 ends the loop.** Everything
//! else retries. `org.rs`'s `SessionProbe` makes the same distinction and
//! `docs/ag-572-activity-api-contract.md` §11 requires it of every consumer.
//!
//! Each test runs its own loopback mock, but the account lives behind the
//! process-global `GATE_CONNECT_TEST_HOME` seam plus an in-memory keychain, so a
//! `Mutex` serializes them within this binary.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gate_connect_core::security_feed::client::{run, Feed};
use gate_connect_core::security_feed::{FeedState, Update};
use gate_connect_core::{account, keychain};

/// Serializes these tests against each other: the account lives behind the
/// process-global `GATE_CONNECT_TEST_HOME` seam, so two running at once would
/// fight over one temp dir.
///
/// A `tokio::sync::Mutex`, not a `std` one, because every test here holds the
/// guard across `.await`. A `std` guard held across an await can park the runtime
/// thread while still holding the lock, which is a deadlock rather than a style
/// preference - clippy's `await_holding_lock` is right about it.
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
            "gate-connect-feed-test-{}-{}",
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
        std::env::remove_var("GATE_CONNECT_TEST_SECURITY_EVENTS_ENDPOINT");
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn with_key_account() {
    keychain::use_in_memory_backend();
    account::save("https://gw.example.com", Some("sk-gw-test")).unwrap();
}

fn drain_request_head(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Serve a scripted SSE stream per connection, and record the request heads so a
/// test can assert what the client sent on reconnect.
///
/// Each entry is one connection's body. When the script runs out the listener
/// stops answering, which the client sees as a connection failure and retries -
/// exactly what a stopped gateway looks like.
fn mock_stream(bodies: Vec<&'static str>) -> Arc<Mutex<Vec<String>>> {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
    let url = format!(
        "http://{}/v1/me/security-events/stream",
        listener.local_addr().expect("mock addr")
    );
    let heads: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = heads.clone();
    thread::spawn(move || {
        for body in bodies {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let head = drain_request_head(&mut stream);
            sink.lock().unwrap().push(head);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                 Cache-Control: no-cache\r\nConnection: close\r\n\r\n{body}"
            );
            stream.write_all(response.as_bytes()).ok();
            stream.flush().ok();
            // Closing ends the stream, which the client treats as a disconnect
            // and reconnects from - the rolling-deploy case.
        }
    });
    std::env::set_var("GATE_CONNECT_TEST_SECURITY_EVENTS_ENDPOINT", url);
    heads
}

/// Answer every connection with one status and no stream.
fn mock_status(status_line: &'static str) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
    let url = format!(
        "http://{}/v1/me/security-events/stream",
        listener.local_addr().expect("mock addr")
    );
    thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            drain_request_head(&mut stream);
            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(response.as_bytes()).ok();
        }
    });
    std::env::set_var("GATE_CONNECT_TEST_SECURITY_EVENTS_ENDPOINT", url);
}

/// Drive the feed until `want` updates have arrived or the deadline passes, then
/// stop it. Returns what was collected.
async fn collect(feed: Arc<Feed>, want: usize, deadline: Duration) -> Vec<Update> {
    let seen: Arc<Mutex<Vec<Update>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let driver = feed.clone();
    let task = tokio::spawn(async move {
        run(driver, move |u| sink.lock().unwrap().push(u)).await;
    });

    let started = std::time::Instant::now();
    while started.elapsed() < deadline {
        if seen.lock().unwrap().len() >= want {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    feed.stop();
    let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
    let out = seen.lock().unwrap().clone();
    out
}

fn states(updates: &[Update]) -> Vec<FeedState> {
    updates
        .iter()
        .filter_map(|u| match u {
            Update::State(s) => Some(*s),
            _ => None,
        })
        .collect()
}

fn event_ids(updates: &[Update]) -> Vec<String> {
    updates
        .iter()
        .filter_map(|u| match u {
            Update::Event(e) => Some(e.id.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn hello_then_events_reports_live_and_delivers_each_once() {
    let _g = LOCK.lock().await;
    let _data = TempDataDir::set();
    with_key_account();
    mock_stream(vec![concat!(
        "event: hello\ndata: {\"recovery\":true}\n\n",
        "event: security-event\nid: 01A\n",
        "data: {\"requestId\":\"r1\",\"at\":\"t\",\"action\":\"block\",\"category\":\"credential\"}\n\n",
        "event: security-event\nid: 01B\n",
        "data: {\"requestId\":\"r2\",\"at\":\"t\",\"action\":\"flag\"}\n\n",
    )]);

    let feed = Arc::new(Feed::new());
    let updates = collect(feed, 3, Duration::from_secs(10)).await;

    assert!(
        states(&updates).contains(&FeedState::Live),
        "the feed must report Live once the server has said hello, got {:?}",
        states(&updates)
    );
    assert_eq!(event_ids(&updates), vec!["01A", "01B"]);
}

#[tokio::test]
async fn a_replayed_event_after_reconnect_is_not_delivered_twice() {
    let _g = LOCK.lock().await;
    let _data = TempDataDir::set();
    with_key_account();
    // Second connection replays 01A - which is what Last-Event-ID resumption
    // does at an inclusive boundary - and adds 01B.
    let heads = mock_stream(vec![
        concat!(
            "event: hello\ndata: {\"recovery\":true}\n\n",
            "event: security-event\nid: 01A\n",
            "data: {\"requestId\":\"r1\",\"at\":\"t\",\"action\":\"block\"}\n\n",
        ),
        concat!(
            "event: hello\ndata: {\"recovery\":true}\n\n",
            "event: security-event\nid: 01A\n",
            "data: {\"requestId\":\"r1\",\"at\":\"t\",\"action\":\"block\"}\n\n",
            "event: security-event\nid: 01B\n",
            "data: {\"requestId\":\"r2\",\"at\":\"t\",\"action\":\"flag\"}\n\n",
        ),
    ]);

    let feed = Arc::new(Feed::new());
    let updates = collect(feed, 6, Duration::from_secs(15)).await;

    assert_eq!(
        event_ids(&updates),
        vec!["01A", "01B"],
        "a replayed id must be dropped, not shown twice"
    );
    // And the client must have asked to resume rather than starting over.
    let heads = heads.lock().unwrap().clone();
    assert!(heads.len() >= 2, "expected a reconnect, saw {}", heads.len());
    assert!(
        heads[1].to_lowercase().contains("last-event-id: 01a"),
        "the reconnect must carry Last-Event-ID, got: {}",
        heads[1]
    );
}

#[tokio::test]
async fn a_401_ends_the_loop_rather_than_retrying() {
    let _g = LOCK.lock().await;
    let _data = TempDataDir::set();
    with_key_account();
    mock_status("401 Unauthorized");

    let feed = Arc::new(Feed::new());
    let driver = feed.clone();
    // Deliberately not driven through `collect`: the property under test is that
    // the loop returns *on its own*, and a helper that stops it cannot tell that
    // apart from a loop that was still retrying when we gave up on it.
    let task = tokio::spawn(async move {
        run(driver, |_| {}).await;
    });

    let finished = tokio::time::timeout(Duration::from_secs(10), task).await;
    assert!(
        finished.is_ok(),
        "a refused credential must end the loop; retrying cannot fix it and \
         spends a throttle bucket shared across the whole office"
    );
    assert_eq!(feed.state(), FeedState::Offline);
}

#[tokio::test]
async fn a_401_after_a_live_stream_tells_the_window_it_went_offline() {
    let _g = LOCK.lock().await;
    let _data = TempDataDir::set();
    with_key_account();
    // Live first, then the session is refused on the reconnect. This is the case
    // that has a transition to report: an expired token mid-session.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
    let url = format!(
        "http://{}/v1/me/security-events/stream",
        listener.local_addr().expect("mock addr")
    );
    thread::spawn(move || {
        let mut first = true;
        while let Ok((mut stream, _)) = listener.accept() {
            drain_request_head(&mut stream);
            let response = if first {
                first = false;
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n\
                 event: hello\ndata: {\"recovery\":false}\n\n"
                    .to_string()
            } else {
                "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_string()
            };
            stream.write_all(response.as_bytes()).ok();
        }
    });
    std::env::set_var("GATE_CONNECT_TEST_SECURITY_EVENTS_ENDPOINT", url);

    let feed = Arc::new(Feed::new());
    let updates = collect(feed, 3, Duration::from_secs(15)).await;

    let states = states(&updates);
    assert!(
        states.contains(&FeedState::Live),
        "the first connection was good; got {states:?}"
    );
    assert_eq!(
        states.last(),
        Some(&FeedState::Offline),
        "a refused reconnect must land on Offline, not sit in Reconnecting; got {states:?}"
    );
}

#[tokio::test]
async fn an_unreachable_gateway_reconnects_instead_of_giving_up() {
    let _g = LOCK.lock().await;
    let _data = TempDataDir::set();
    with_key_account();
    // A port with nothing on it: the failure a laptop on a dead wifi sees.
    let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe");
    let addr = probe.local_addr().expect("probe addr");
    drop(probe);
    std::env::set_var(
        "GATE_CONNECT_TEST_SECURITY_EVENTS_ENDPOINT",
        format!("http://{addr}/v1/me/security-events/stream"),
    );

    let feed = Arc::new(Feed::new());
    let updates = collect(feed, 1, Duration::from_secs(8)).await;

    let states = states(&updates);
    assert!(
        states.contains(&FeedState::Reconnecting),
        "an unreachable gateway is a reconnect, not a sign-out; got {states:?}"
    );
}

#[tokio::test]
async fn no_account_rests_at_offline_without_hammering() {
    let _g = LOCK.lock().await;
    let _data = TempDataDir::set();
    keychain::use_in_memory_backend();
    // Deliberately no account saved.
    mock_status("200 OK");

    let feed = Arc::new(Feed::new());
    let updates = collect(feed, 1, Duration::from_secs(4)).await;

    let states = states(&updates);
    assert!(
        states.iter().all(|s| *s == FeedState::Offline),
        "with nothing to connect with the feed rests at Offline; got {states:?}"
    );
}
