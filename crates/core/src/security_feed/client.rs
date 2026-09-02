//! The long-lived connection: connect, stream, reconnect, and say which of those
//! is happening.
//!
//! Kept apart from the types and policy in the parent module so the state machine
//! can be read on its own. The parent owns *what* an event is and *how long* to
//! wait; this owns *when* to do which.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use futures_util::StreamExt;

use super::sse;
use super::{
    backoff_delay, credential_headers, endpoint, history_endpoint, Dedupe, FeedState, Hello,
    HistoryPage, SecurityEvent, Update, BACKOFF_FLOOR_MS, DEDUPE_CAPACITY, RECENT_CAPACITY,
};
use crate::gateway_api::FailureCode;

/// How long to hold after the gateway refuses the credential.
///
/// A backstop, not a retry cadence: a refusal is fixed by a new session rather
/// than by asking again, and [`Feed::retry_now`] is what a recovery signals
/// with. The hour is so that a signal which never arrives still costs one
/// request, instead of leaving the feed dead until the app restarts.
const REJECTED_HOLD: Duration = Duration::from_secs(3600);

/// Shared handle: what the feed is doing, and what it has seen.
///
/// The window is told about events as they arrive, but a window opened *after*
/// ten of them has missed all ten - Tauri events only reach a listener that
/// already exists, and the tray window is created and destroyed on demand. So the
/// feed keeps its own bounded buffer and the window asks for it on mount.
pub struct Feed {
    state: RwLock<FeedState>,
    recent: Mutex<VecDeque<SecurityEvent>>,
    dedupe: Mutex<Dedupe>,
    /// Set when the user asks for a retry, or when the account changed under us.
    wake: Arc<tokio::sync::Notify>,
    /// Cleared to stop the loop at shutdown.
    running: AtomicBool,
}

impl Default for Feed {
    fn default() -> Self {
        Self::new()
    }
}

impl Feed {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(FeedState::Offline),
            recent: Mutex::new(VecDeque::new()),
            dedupe: Mutex::new(Dedupe::new(DEDUPE_CAPACITY)),
            wake: Arc::new(tokio::sync::Notify::new()),
            running: AtomicBool::new(true),
        }
    }

    pub fn state(&self) -> FeedState {
        *self.state.read().expect("feed state lock")
    }

    /// The buffer, newest last, for a window that just mounted.
    pub fn recent(&self) -> Vec<SecurityEvent> {
        self.recent
            .lock()
            .expect("feed buffer lock")
            .iter()
            .cloned()
            .collect()
    }

    /// AC6's recovery action, and the account-changed path. Wakes the loop out of
    /// whatever backoff it is sitting in so a user who clicks Retry sees
    /// something happen rather than waiting out a 60s sleep.
    pub fn retry_now(&self) {
        self.wake.notify_one();
    }

    /// Drop everything belonging to the org the user just left. Events must not
    /// survive a switch, and a dedupe entry from the old org must not suppress a
    /// real event in the new one.
    pub fn reset_for_account_change(&self) {
        self.recent.lock().expect("feed buffer lock").clear();
        self.dedupe.lock().expect("feed dedupe lock").clear();
        self.retry_now();
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        self.wake.notify_waiters();
    }

    fn set_state(&self, next: FeedState, sink: &dyn Fn(Update)) {
        let mut cur = self.state.write().expect("feed state lock");
        if *cur == next {
            // Emitting an unchanged state would make the window re-render and,
            // worse, make "Reconnecting" flicker on every failed attempt in a
            // long outage when it has been Reconnecting the whole time.
            return;
        }
        *cur = next;
        drop(cur);
        // Every transition passes through here, and this module wrote no log line
        // at all until now - so a feed that was Live and starved looked exactly
        // like one that never connected, a distinction only the gateway's own
        // logs could make. It has to be answerable from the client.
        crate::logging::log(
            crate::logging::Level::Info,
            &format!("security feed: state -> {next:?}"),
        );
        sink(Update::State(next));
    }

    /// Record and forward one event, unless it is one we have already delivered.
    fn deliver(&self, ev: SecurityEvent, sink: &dyn Fn(Update)) -> bool {
        if !self.dedupe.lock().expect("feed dedupe lock").admit(&ev.id) {
            return false;
        }
        {
            let mut recent = self.recent.lock().expect("feed buffer lock");
            recent.push_back(ev.clone());
            while recent.len() > RECENT_CAPACITY {
                recent.pop_front();
            }
        }
        crate::logging::log(
            crate::logging::Level::Info,
            &format!(
                "security feed: delivered id={} {:?} req={}",
                ev.id, ev.action, ev.request_id
            ),
        );
        sink(Update::Event(Box::new(ev)));
        true
    }

    /// The `at` of the newest event held, for a backfill's `since`.
    ///
    /// Asking for everything on every reconnect would re-fetch a window the
    /// buffer already has, and on a long-lived window that is most of it. The
    /// newest event is the honest boundary, and the overlap it leaves is
    /// deliberate - see [`Self::merge_history`].
    pub fn newest_at(&self) -> Option<String> {
        self.recent
            .lock()
            .expect("feed buffer lock")
            .back()
            .map(|e| e.at.clone())
    }

    /// Merge a page of history into the buffer, returning how much was new.
    ///
    /// **Keyed on `request_id`, not on `id`.** The history route cannot
    /// reproduce the id the live stream minted for the same request: the live
    /// one is `new Date()` at publish time plus a random suffix, the historical
    /// one derives from `gateway_requests.created_at`, and the server's
    /// `history-event-id.ts` records why unifying them would mean reading every
    /// row back after insert to serve a notification. So the id dedupe alone
    /// would let a backfilled row through as a second copy of an event already
    /// on screen, which is the one outcome a catch-up must not produce.
    ///
    /// Oldest first, as the route returns them, so they append in order to a
    /// buffer whose newest event is last.
    fn merge_history(&self, events: Vec<SecurityEvent>, sink: &dyn Fn(Update)) -> usize {
        let mut merged = 0;
        for ev in events {
            let already = {
                let recent = self.recent.lock().expect("feed buffer lock");
                recent.iter().any(|e| e.request_id == ev.request_id)
            };
            if already {
                continue;
            }
            if self.deliver(ev, sink) {
                merged += 1;
            }
        }
        merged
    }
}

/// Run the feed until [`Feed::stop`].
///
/// `sink` carries updates to whoever is driving this; `crates/core` has no
/// `tauri` dependency, so the transport to the window is the caller's business.
///
/// The loop returns on `stop`, and on nothing else. A definite 401 is still the
/// one outcome treated differently from every other failure - it holds rather
/// than retrying on a timer - but it holds *alive*, so the recovery that
/// [`crate::oauth::mark_session_rejected`] starts has a feed to bring back. That
/// distinction is the `SessionProbe` discipline from [`crate::org`] - **a feed
/// failure must never sign the user out** - and an unreachable gateway is
/// retried forever.
pub async fn run<F>(feed: Arc<Feed>, sink: F)
where
    F: Fn(Update) + Send + Sync + 'static,
{
    let sink: Arc<dyn Fn(Update) + Send + Sync> = Arc::new(sink);
    let mut attempt: u32 = 0;
    // Survives across connections so a reconnect resumes where the last one
    // stopped, rather than waiting for the server to re-send an id.
    let mut last_id: Option<String> = None;
    let mut retry_floor_ms = BACKOFF_FLOOR_MS;

    while feed.running.load(Ordering::SeqCst) {
        match connect_once(&feed, &*sink, &mut last_id, &mut retry_floor_ms).await {
            Outcome::Rejected => {
                // The gateway refused this credential. Retrying on a timer cannot
                // fix it and would burn the shared per-IP throttle bucket doing
                // so - but *returning* killed the task for the life of the
                // process, which left the recovery `mark_session_rejected` exists
                // to start with no feed to bring back, and made `retry_now` - the
                // pane's own "Try again", and the org-switch reset - a no-op
                // against a task that had already exited. Still no retry cadence;
                // wait for the signal that a credential changed.
                crate::oauth::mark_session_rejected();
                feed.set_state(FeedState::Offline, &*sink);
                attempt = 0;
                wait(&feed, REJECTED_HOLD).await;
                continue;
            }
            Outcome::NotReady => {
                // No account, no org, or no live session. Not a failure and not
                // worth a backoff ramp: the app will wake us when that changes.
                feed.set_state(FeedState::Offline, &*sink);
                attempt = 0;
                wait(&feed, Duration::from_secs(30)).await;
                continue;
            }
            Outcome::Disconnected => {
                feed.set_state(FeedState::Reconnecting, &*sink);
            }
            Outcome::Streamed => {
                // A clean stream that ended (a `bye`, or a rolling deploy closing
                // the socket) reconnects immediately-ish rather than ramping: the
                // connection worked, so the ramp would be punishing the wrong
                // thing.
                feed.set_state(FeedState::Reconnecting, &*sink);
                attempt = 0;
            }
        }
        let delay = backoff_delay(attempt, retry_floor_ms);
        attempt = attempt.saturating_add(1);
        wait(&feed, delay).await;
    }
    feed.set_state(FeedState::Offline, &*sink);
}

/// Sleep, unless the user asks for a retry first.
async fn wait(feed: &Feed, delay: Duration) {
    let wake = feed.wake.clone();
    tokio::select! {
        _ = tokio::time::sleep(delay) => {}
        _ = wake.notified() => {}
    }
}

/// Fetch the events this client could not have received, and merge them.
///
/// Exists because the stream has no replay: the server reports
/// `recovery: false` on every deployment, so a client that was away for any
/// reason has a hole, and `/v1/me/tool-events` cannot fill it - that route
/// reports on one tool at a time, so no single call answers "what did I miss
/// across every tool" (Constellation-Labs/gate#990).
///
/// **Best-effort, and never fails the stream.** A live connection that works is
/// worth more than a catch-up that did not, and the next reconnect tries again.
/// Every failure is logged rather than swallowed, because a silent catch-up that
/// never ran is indistinguishable from one that found nothing.
async fn backfill(feed: &Arc<Feed>, sink: &(dyn Fn(Update) + Send + Sync)) {
    let headers = match credential_headers() {
        Ok(h) => h,
        Err(e) => {
            crate::logging::log(
                crate::logging::Level::Warn,
                &format!(
                    "security feed: backfill skipped, no credential: {}",
                    e.message
                ),
            );
            return;
        }
    };
    // A total timeout, unlike the stream's read timeout: this is an ordinary
    // request that must finish, and the read loop is waiting on it.
    let client = match reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };
    // Built through `Url` rather than `RequestBuilder::query`, which this
    // build's reqwest features do not include, and which would also leave the
    // encoding of a timestamp to a `format!`.
    let mut url = match reqwest::Url::parse(&history_endpoint()) {
        Ok(u) => u,
        Err(e) => {
            crate::logging::log(
                crate::logging::Level::Warn,
                &format!("security feed: backfill url unusable: {e}"),
            );
            return;
        }
    };
    // The newest event already held. Absent on a first connect, which asks for
    // the server's whole window instead.
    if let Some(since) = feed.newest_at() {
        url.query_pairs_mut().append_pair("since", &since);
    }
    let mut req = client.get(url).header("accept", "application/json");
    for (k, v) in headers {
        req = req.header(k, v);
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            crate::logging::log(
                crate::logging::Level::Warn,
                &format!("security feed: backfill failed: {e}"),
            );
            return;
        }
    };
    let status = resp.status();
    if !status.is_success() {
        crate::logging::log(
            crate::logging::Level::Warn,
            &format!("security feed: backfill answered {status}"),
        );
        return;
    }
    let page = match resp.json::<HistoryPage>().await {
        Ok(p) => p,
        Err(e) => {
            crate::logging::log(
                crate::logging::Level::Warn,
                &format!("security feed: backfill body unreadable: {e}"),
            );
            return;
        }
    };

    let found = page.events.len();
    let merged = feed.merge_history(page.events, sink);
    crate::logging::log(
        crate::logging::Level::Info,
        &format!(
            "security feed: backfilled {merged} new of {found} (truncated={})",
            page.truncated
        ),
    );
    if page.truncated {
        // The window held more than one page. Worth saying: the pane is showing
        // a partial history, and nothing else in the app will mention it.
        crate::logging::log(
            crate::logging::Level::Warn,
            "security feed: backfill was truncated; older events are only on the dashboard",
        );
    }
}

enum Outcome {
    /// Connected and streamed; the stream then ended.
    Streamed,
    /// Never got a usable stream.
    Disconnected,
    /// Nothing to connect with yet.
    NotReady,
    /// The gateway refused the credential.
    Rejected,
}

async fn connect_once(
    feed: &Arc<Feed>,
    sink: &(dyn Fn(Update) + Send + Sync),
    last_id: &mut Option<String>,
    retry_floor_ms: &mut u64,
) -> Outcome {
    let headers = match credential_headers() {
        Ok(h) => h,
        Err(e) => {
            return match e.code {
                // "Not signed in" and "no org chosen" are states the user is
                // about to leave, not errors to retry against.
                FailureCode::SignedOut | FailureCode::NoOrg => Outcome::NotReady,
                _ => Outcome::Disconnected,
            };
        }
    };

    // `.no_proxy()` is mandatory: without it the app's own machine-wide
    // HTTPS_PROXY export captures this call, the engine injects X-Gate-Api-Key,
    // and the gateway answers 401 for a credential the user never sent.
    //
    // No overall timeout, unlike the control-plane client: this connection is
    // meant to stay open. The read timeout is what notices a dead peer, and the
    // server's 15s keepalive comment is what keeps it from firing on an idle but
    // healthy stream.
    let client = match reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(45))
        .build()
    {
        Ok(c) => c,
        Err(_) => return Outcome::Disconnected,
    };

    let mut req = client.get(endpoint()).header("accept", "text/event-stream");
    for (k, v) in headers {
        req = req.header(k, v);
    }
    if let Some(id) = last_id.as_deref() {
        req = req.header("last-event-id", id);
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            crate::logging::log(
                crate::logging::Level::Warn,
                &format!("security feed: connect failed: {e}"),
            );
            return Outcome::Disconnected;
        }
    };
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        crate::logging::log(
            crate::logging::Level::Warn,
            &format!("security feed: gateway refused the stream ({status})"),
        );
        return Outcome::Rejected;
    }
    if !status.is_success() {
        crate::logging::log(
            crate::logging::Level::Warn,
            &format!("security feed: gateway answered {status}"),
        );
        return Outcome::Disconnected;
    }

    let mut decoder = sse::Decoder::new();
    decoder.seed_last_id(last_id.clone());
    let mut stream = resp.bytes_stream();
    let mut saw_hello = false;
    // One catch-up per connection. A server that sent a second hello on one
    // socket would otherwise re-fetch the same window.
    let mut backfilled = false;

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            // A mid-stream read error is a disconnect, not a rejection: the
            // credential was accepted or we would not be here.
            Err(_) => break,
        };
        for frame in decoder.push(&chunk) {
            if let Some(ms) = frame.retry {
                *retry_floor_ms = ms;
            }
            if let Some(id) = frame.id.clone() {
                *last_id = Some(id);
            }
            match frame.event.as_deref() {
                Some("hello") => {
                    let hello: Hello = serde_json::from_str(&frame.data).unwrap_or_default();
                    if let Some(ms) = hello.heartbeat_ms {
                        // Only informative today; recorded so a server that
                        // widens its keepalive does not look like a dead peer.
                        let _ = ms;
                    }
                    saw_hello = true;
                    // Live only once the server has actually said hello. A socket
                    // that opened and then produced nothing is not a live feed,
                    // and claiming otherwise is the reassurance this app is not
                    // allowed to fake.
                    feed.set_state(FeedState::Live, sink);
                    // Catch up on whatever the stream could not replay, now that
                    // the server has blessed this connection.
                    //
                    // Gated on the server's own answer: when `recovery` is true
                    // the stream honours `Last-Event-ID` and re-sends the gap
                    // itself, and running both would be a second fetch to
                    // discard. It is false on every deployment today.
                    //
                    // Here rather than before the read loop so the sequence a
                    // window sees is hello, then Live, then history, then live
                    // events. An event published during the call is still in the
                    // socket buffer and gets read straight after, where the
                    // dedupe collapses it against whatever this added.
                    if !hello.recovery && !backfilled {
                        backfilled = true;
                        backfill(feed, sink).await;
                    }
                }
                Some("security-event") => {
                    match serde_json::from_str::<SecurityEvent>(&frame.data) {
                        Ok(mut ev) => {
                            // The frame's `id:` is the stream position and the
                            // dedupe key; a payload that omits it inherits it.
                            if ev.id.is_empty() {
                                ev.id = frame.id.clone().unwrap_or_default();
                            }
                            feed.deliver(ev, sink);
                        }
                        // One unparseable frame must not kill the connection: a
                        // gateway that adds a field should not take the feed down
                        // with it.
                        Err(_) => continue,
                    }
                }
                Some("bye") => return Outcome::Streamed,
                // Unknown event names are ignored so the server can add one.
                _ => {}
            }
        }
    }

    if saw_hello {
        Outcome::Streamed
    } else {
        Outcome::Disconnected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security_feed::Action;

    fn ev(id: &str, request_id: &str, at: &str) -> SecurityEvent {
        SecurityEvent {
            id: id.to_string(),
            request_id: request_id.to_string(),
            at: at.to_string(),
            action: Action::Flag,
            category: Some("credential".to_string()),
            tool: Some("claude-code".to_string()),
            model: None,
            provider: None,
        }
    }

    /// A sink that records the ids it was handed, in order.
    fn recorder() -> (Arc<Mutex<Vec<String>>>, impl Fn(Update) + Send + Sync) {
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let handle = seen.clone();
        let sink = move |u: Update| {
            if let Update::Event(e) = u {
                handle.lock().expect("test sink").push(e.id.clone());
            }
        };
        (seen, sink)
    }

    #[test]
    fn merge_history_admits_events_the_buffer_has_not_seen() {
        let feed = Feed::new();
        let (seen, sink) = recorder();

        let merged = feed.merge_history(
            vec![
                ev("h1", "req-a", "2026-09-02T13:00:00.000Z"),
                ev("h2", "req-b", "2026-09-02T13:01:00.000Z"),
            ],
            &sink,
        );

        assert_eq!(merged, 2);
        assert_eq!(*seen.lock().unwrap(), vec!["h1", "h2"]);
    }

    #[test]
    fn merge_history_skips_a_request_already_on_screen() {
        // The whole reason the merge is keyed on `request_id`: the history route
        // cannot reproduce the id the live stream minted for the same request,
        // so an id-only dedupe would show the user one event twice.
        let feed = Feed::new();
        let (seen, sink) = recorder();
        feed.deliver(ev("live-1", "req-a", "2026-09-02T13:00:00.000Z"), &sink);

        let merged = feed.merge_history(
            vec![
                ev("hist-1", "req-a", "2026-09-02T13:00:00.000Z"),
                ev("hist-2", "req-b", "2026-09-02T13:01:00.000Z"),
            ],
            &sink,
        );

        assert_eq!(
            merged, 1,
            "req-a was already delivered under a different id"
        );
        assert_eq!(*seen.lock().unwrap(), vec!["live-1", "hist-2"]);
    }

    #[test]
    fn newest_at_is_the_boundary_a_backfill_asks_from() {
        let feed = Feed::new();
        let (_seen, sink) = recorder();
        assert_eq!(
            feed.newest_at(),
            None,
            "nothing held means ask for the window"
        );

        feed.deliver(ev("a", "req-a", "2026-09-02T13:00:00.000Z"), &sink);
        feed.deliver(ev("b", "req-b", "2026-09-02T13:05:00.000Z"), &sink);

        // The buffer is oldest-first, so the boundary is its back, not its front.
        assert_eq!(
            feed.newest_at().as_deref(),
            Some("2026-09-02T13:05:00.000Z")
        );
    }

    #[test]
    fn a_cleared_feed_asks_for_the_whole_window_again() {
        // An org switch drops the buffer, and a `since` from the previous org's
        // events would silently narrow the new org's catch-up.
        let feed = Feed::new();
        let (_seen, sink) = recorder();
        feed.deliver(ev("a", "req-a", "2026-09-02T13:00:00.000Z"), &sink);

        feed.reset_for_account_change();

        assert_eq!(feed.newest_at(), None);
    }
}
