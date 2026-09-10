import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  securityFeedHistoryOk,
  securityFeedRecent,
  securityFeedRetry,
  securityFeedState,
  type FeedState,
  type SecurityEvent,
} from "./api";

/**
 * The live security-event feed, as the window sees it (AG-578).
 *
 * Unlike `useActivity` and `useToolEvents`, this hook does not fetch on a
 * schedule and does not page. The backend holds one long-lived connection to the
 * gateway and pushes; this listens. The reason that split exists at all is worth
 * keeping in view: the app's CSP lists no gateway origin in `connect-src` and the
 * webview holds no token, so a renderer-side `EventSource` is not a shortcut that
 * was passed over - it is blocked, and deliberately.
 *
 * Two things arrive on mount rather than by event. The connection state, because
 * a window that opened while the feed was already Live would otherwise sit at its
 * initial value until the next transition, which on a healthy feed may be never.
 * And the buffer, because Tauri events only reach a window that is already
 * listening: a popover opened after ten blocked requests has missed all ten, and
 * showing "No security events" would be a claim about the user's traffic rather
 * than about this window's uptime.
 */

/**
 * How the pane distinguishes "loaded and empty" from "could not load" (AC6).
 *
 * `loading` is only true before the first read answers. After that the feed is
 * either showing what it has or saying it cannot, and a feed that is Reconnecting
 * with events already on screen is neither loading nor unavailable - it is a live
 * surface having a bad minute, and blanking it would lose what the user was
 * reading.
 */
/**
 * How many events the window keeps.
 *
 * Matches `RECENT_CAPACITY` in `crates/core/src/security_feed/mod.rs`, so a
 * window that has been open for a week shows the same depth of history as one
 * opened a moment ago. The pane is a live feed, not an archive: the full
 * 24-hour record is `/v1/me/tool-events`, behind the per-app activity table.
 */
const FEED_CAPACITY = 200;

export interface SecurityFeedView {
  events: SecurityEvent[];
  state: FeedState;
  loading: boolean;
  /** The mount read failed outright, so there is nothing to show and no reason
   *  to believe the list is empty. Distinct from an empty feed. */
  unavailable: boolean;
  /** The catch-up read for events from before this connection failed, so what is
   *  on screen is only what arrived live.
   *
   *  A third state, and it has to be: the stream can be Live with its history
   *  missing, which is what a gateway answering the history route with 400
   *  produced - a pane showing the LIVE pill over "No security events" while the
   *  local log recorded the refusal. `unavailable` is about this window's own
   *  read failing; this is about the backfill behind it. */
  historyUnavailable: boolean;
  /** AC6's recovery action. */
  retry: () => void;
}

export function useSecurityFeed(enabled: boolean, credential = ""): SecurityFeedView {
  const [events, setEvents] = useState<SecurityEvent[]>([]);
  const [state, setState] = useState<FeedState>("offline");
  const [loading, setLoading] = useState(true);
  const [unavailable, setUnavailable] = useState(false);
  const [historyOk, setHistoryOk] = useState(true);
  /** Which account the events on screen belong to, so a reply that lands after a
   *  switch is dropped rather than shown under the new org's name. */
  const attempt = useRef(0);

  // Events from the org the user just left must not survive the switch. The same
  // rule the Overview applies to its cached reading, and for the same reason:
  // a number - or here a security event - shown under the wrong org's name is
  // worse than no number at all.
  useEffect(() => {
    setEvents([]);
    setLoading(true);
    setUnavailable(false);
    // The old org's failed catch-up says nothing about the new one's. The
    // backend clears its copy too, in `reset_for_account_change`, which
    // `set_org` and `clear_account` now call - they did not when this line was
    // written, so the seed below would re-read the previous org's verdict and
    // the emit-on-change guard would then suppress the correction. This keeps
    // the two from disagreeing for the length of one read.
    setHistoryOk(true);
  }, [credential]);

  const seed = useCallback(() => {
    if (!enabled) return;
    const mine = ++attempt.current;
    setLoading(true);
    // The backfill runs once per connection, so a window that mounted after it
    // failed would never see the event. Seeded here for the same reason the
    // connection state and the buffer are.
    //
    // Its own read, deliberately NOT in the `Promise.all` below. This is a
    // supplementary fact about a feed that loaded, so a backend that cannot
    // answer it must not be able to fail the load - inside the `all`, one
    // rejection would take the buffer and the connection state with it and
    // render a working feed as Unavailable. Failing quietly to `true` is right
    // too: not knowing whether history is missing is not evidence that it is.
    securityFeedHistoryOk()
      .then((okHistory) => {
        if (mine === attempt.current) setHistoryOk(okHistory);
      })
      .catch(() => {});
    Promise.all([securityFeedRecent(), securityFeedState()])
      .then(([recent, feedState]) => {
        if (mine !== attempt.current) return;
        setEvents(recent);
        setState(feedState);
        setUnavailable(false);
      })
      .catch(() => {
        if (mine !== attempt.current) return;
        // The backend could not answer at all. Not the same as an empty feed,
        // and the pane must not render it as one.
        setUnavailable(true);
      })
      .finally(() => {
        if (mine === attempt.current) setLoading(false);
      });
  }, [enabled, credential]);

  useEffect(() => {
    seed();
  }, [seed]);

  useEffect(() => {
    if (!enabled) return;
    const offEvent = listen<SecurityEvent>("security-event", (e) => {
      // Newest last, matching the buffer's order and the table's. The backend
      // has already deduped, so this appends without checking - re-checking here
      // would put a second, differently-bounded dedupe set in the window and the
      // two would disagree on eviction.
      //
      // Bounded to the same capacity the backend buffers, and for a reason the
      // backend does not have: a window stays open for days, so an unbounded
      // array here grows for as long as the app runs and every render walks it.
      // Dropping the oldest matches what a reopened window would have been given
      // anyway, so the two do not disagree about how far back the feed goes.
      setEvents((prev) => {
        const next = [...prev, e.payload];
        return next.length > FEED_CAPACITY ? next.slice(next.length - FEED_CAPACITY) : next;
      });
    });
    const offState = listen<FeedState>("security-feed-state", (e) => {
      setState(e.payload);
      // A feed that reached any state answered, so whatever failed at mount is
      // no longer what is happening.
      setUnavailable(false);
    });
    // Deliberately NOT folded into the state listener: the connection and its
    // history fail independently, which is the whole reason this exists. A
    // successful reconnect that still cannot fetch the history must not clear
    // this, and a failed catch-up must not disturb the LIVE pill.
    const offHistory = listen<boolean>("security-feed-history", (e) => {
      setHistoryOk(e.payload);
    });
    return () => {
      void offEvent.then((f) => f()).catch(() => {});
      void offState.then((f) => f()).catch(() => {});
      void offHistory.then((f) => f()).catch(() => {});
    };
  }, [enabled]);

  const retry = useCallback(() => {
    // Ask the backend to leave its backoff, and re-read: the click has to do
    // something visible even when the connection itself takes a while.
    void securityFeedRetry().catch(() => {});
    seed();
  }, [seed]);

  return {
    events,
    state,
    loading,
    unavailable,
    historyUnavailable: !historyOk,
    retry,
  };
}
