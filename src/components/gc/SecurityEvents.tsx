import { useState } from "react";
import type { ReactNode } from "react";
import type { SecurityEvent } from "../../lib/api";
import { BADGE_STYLES, Card, EmptyNote, Pill, Skeleton } from "./base";
import { Icon } from "./Icon";

/**
 * The live security-event feed (AG-578), as the last section of the Overview
 * pane (AG-853).
 *
 * A chronological list of the requests Gate blocked or flagged, arriving while
 * the app is open. Everything the section can show is on the row: what happened,
 * what category fired, which tool, which model, when, and a way through to the
 * dashboard. Everything it *cannot* show is the point of it - no prompt, no
 * response, no matched secret, no evidence. Those fields are omitted by the
 * gateway rather than hidden here, so there is nothing on the client to leak.
 *
 * **It was its own pane and its own rail entry until 2026-09-14.** AG-853 moved
 * it below Token savings and removed the entry, which also puts the rail back to
 * the two items the Sidenav frame (408:15625) actually draws - the third was
 * recorded as an undrawn addition at the time. What arrived from the pane is the
 * feed's own connection pill, which moved from a pane header into this card's,
 * and the partial-history notice, which still sits above the rows rather than
 * annotating each one.
 *
 * **Still undrawn.** No frame draws the section either, so it is built from the
 * component set, which is what CLAUDE.md asks for where no frame draws the thing:
 * the card is `Overview`'s own `Card`, the table is `AppPane`'s recent-activity
 * table, and the badges are the shared `BADGE_STYLES` pair. Recorded as a
 * deviation in `plans/new-app-ui-figma.md`.
 */

/** Anchor for anything that navigates *to* the feed rather than to the pane
 *  it now lives on. Nothing does today; the tray's security card did, and was
 *  removed. The Token savings section above it carries the same kind of target;
 *  see `Overview`'s `SAVINGS_SECTION_ID`. */
export const SECURITY_SECTION_ID = "security-events";

/** Rows per reveal. Ten, and the same ten the App pane's recent-activity table
 *  shows, because the two tables sit one pane apart and a person reading both
 *  should not have to work out that they count differently. */
const PAGE = 10;

/** The gateway's verb, in the section's vocabulary - the same mapping
 *  `lib/toolEvents.ts` makes, and for the same reason: the gateway records what a
 *  policy *did*, the pills read as what happened to the request. */
const ACTION_LABEL = {
  block: { label: "Blocked", badge: BADGE_STYLES.blocked },
  flag: { label: "Flagged", badge: BADGE_STYLES.flagged },
} as const;

/** When it happened, to the second.
 *
 * Seconds and a date, matching `toolEvents.ts`'s `eventTime` exactly: an agent
 * sends several requests a minute, so without seconds four rows read as one
 * moment and the order looks arbitrary. A timestamp rather than an age, because
 * an age has to be recomputed to stay true and a "2 minutes ago" written twenty
 * minutes ago is a worse lie than the age it was added to disclose. */
function eventTime(at: string): string {
  const d = new Date(at);
  const date = d.toLocaleDateString([], { month: "short", day: "numeric" });
  const time = d.toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
  return `${date}, ${time}`;
}

/** What a cell says when the gateway attributed nothing to it.
 *
 * Not an error and not a withholding: an agent whose User-Agent is not on the
 * gateway's allowlist is recorded unattributed rather than guessed at, which is
 * the honest outcome and an ordinary one. */
const UNATTRIBUTED = "-";

function Th({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <th
      scope="col"
      className={`pb-3 text-left text-base-xs font-medium text-base-muted-foreground ${className}`}
    >
      {children}
    </th>
  );
}

/** Placeholder rows while the first read is in flight.
 *
 * A skeleton rather than an empty table, because "No security events" is a claim
 * about the user's traffic and must never be made by a screen that has not
 * finished asking. */
function PendingRows() {
  return (
    <>
      {[0, 1, 2].map((i) => (
        <tr key={i} className="border-t border-base-border">
          {[0, 1, 2, 3, 4].map((c) => (
            <td key={c} className="py-3 pr-3">
              <Skeleton className="h-4 w-full" />
            </td>
          ))}
        </tr>
      ))}
    </>
  );
}

/**
 * Everything the section draws, which is everything `useSecurityFeed` holds plus
 * the two callbacks the shell owns.
 *
 * Named and exported because `Overview` now passes it straight through: the feed
 * is one read among the pane's several, and spreading its seven fields into
 * `Overview`'s own signature would leave nothing saying which of them belong
 * together.
 *
 * **Not `SecurityFeedView`**, which is the hook's own return type
 * (`lib/securityFeed.ts`). The two agree on five fields and differ on the rest -
 * `retry` there against `onRetry` and `onOpenEvent` here - so a name shared
 * between them is one an editor's auto-import picks wrong, with the type error
 * landing at whichever call site is furthest from the mistake.
 */
export interface SecurityEventsProps {
  /** Oldest first, as the feed buffers them. The table reverses for display. */
  events: SecurityEvent[];
  loading: boolean;
  /** The feed could not be read at all. Distinct from an empty feed, and the
   *  distinction is AC6's whole point. */
  unavailable: boolean;
  /** The catch-up read failed, so anything from before this connection is
   *  missing. A third state beside the two above, because the stream and its
   *  history fail independently: LIVE with no history is precisely the
   *  combination that rendered as "No security events". */
  historyUnavailable?: boolean;
  onRetry: () => void;
  /** Open this event in the Gate dashboard. The row's own control, and the
   *  whole of what a row leads to since 2026-09-23 - see the note on the
   *  button. */
  onOpenInDashboard: (event: SecurityEvent) => void;
}

export function SecurityEvents({
  events,
  loading,
  unavailable,
  historyUnavailable,
  onRetry,
  onOpenInDashboard,
}: SecurityEventsProps) {
  // Newest first on screen: a feed is read from the top, and the event a user
  // scrolled down here for is the one that just happened.
  const all = [...events].reverse();
  /**
   * How many rows are on screen. Ten to start, ten more per click, matching
   * the App pane's recent-activity table - the surface product named as the
   * pattern for this (2026-09-23).
   *
   * Client-side here, unlike that table: this feed arrives over a stream and
   * the whole session is already in memory, so there is no page to fetch and
   * "load more" is purely revealing what is held. A session that has been open
   * a while was drawing every event it had ever seen.
   *
   * Not reset when `events` grows. New events arrive at the top and the count
   * is a floor, not a window, so a reveal the person asked for is not undone
   * by traffic arriving after it.
   */
  const [visible, setVisible] = useState(PAGE);
  const rows = all.slice(0, visible);
  const more = all.length - rows.length;

  return (
    // The pane's own `gap-4` separated the notice from the card while this was a
    // screen; as a section it has to carry that itself, so the two arrive as one
    // child of the pane. The id and `scroll-mt-6` are on the wrapper rather than
    // the card so a jump to the section lands above the notice, not past it -
    // that notice is the one thing on screen saying the list is incomplete.
    <div id={SECURITY_SECTION_ID} className="flex scroll-mt-6 flex-col gap-4">
      {/* Rows on screen and a failed catch-up is not the empty case, so it does
        * not belong in the table's empty cell - but the list is still partial
        * and nothing else in the app would mention it. Said once, above the
        * rows, rather than annotating each one. */}
      {historyUnavailable && !loading && events.length > 0 && (
        // No action here either, for the reason spelled out in the empty case
        // below: nothing the window can call re-runs the catch-up while the
        // stream is Live.
        <div
          role="status"
          className="rounded-md border border-amber-300 bg-amber-50 px-4 py-3"
        >
          <p className="text-sm leading-5 text-amber-900">
            Showing events from this session only. Earlier events couldn’t be
            loaded.
          </p>
        </div>
      )}

      <Card className="p-4" busy={loading}>
        {loading && <span className="sr-only">Loading security events</span>}
        {/* `heading/16`, the same line the Policies and Token savings cards
          * above draw. */}
        {/* The feed's connection pill - Live / Reconnecting / Offline - sat
          * beside this heading until 2026-09-23, when product asked for it to
          * go. "Live" was true on every healthy launch and said nothing; the
          * cost is that the two unhappy states no longer have a surface
          * either, the tray's security card having been removed in #334. An
          * offline feed and a quiet machine now look the same here. Raised
          * with the decision, not overlooked. */}
        <h2 className="text-base font-medium leading-6 tracking-heading-16 text-base-foreground">
          Security events
        </h2>
        {/* 20px under the heading, as on both cards above. */}
        <table className="mt-5 w-full">
          <thead>
            <tr>
              <Th>Time</Th>
              <Th>Security</Th>
              <Th>Category</Th>
              <Th>Tool</Th>
              <Th>Model</Th>
              <Th className="sr-only">Action</Th>
            </tr>
          </thead>
          <tbody>
            {loading ? (
              <PendingRows />
            ) : rows.length === 0 ? (
              <tr>
                <td colSpan={6}>
                  {unavailable ? (
                    // AC6: cannot load. Says so, and offers the way out. Never
                    // "No security events", which would be a claim about the
                    // user's traffic made by a screen that failed to ask.
                    <EmptyNote icon="triangleAlert">
                      <span className="flex flex-col items-center gap-2">
                        <span>Unavailable</span>
                        <button
                          type="button"
                          onClick={onRetry}
                          className="text-base-primary underline underline-offset-2"
                        >
                          Try again
                        </button>
                      </span>
                    </EmptyNote>
                  ) : historyUnavailable ? (
                    // Live, empty, and unable to say the feed is empty: the
                    // catch-up read is what would have answered that, and it
                    // failed. Saying "No security events" here is the same
                    // mistake `unavailable` above exists to prevent, one layer
                    // down - a claim about the user's traffic made by a screen
                    // whose question was refused.
                    //
                    // No Try again, deliberately, unlike the case above. That
                    // one recovers because `retry` re-seeds and the read can
                    // succeed. This one cannot: `Feed::retry_now` only wakes the
                    // backoff between connection attempts, and the catch-up runs
                    // once per connection off `hello` - so while the stream is
                    // Live, which is exactly when this renders, a retry issues
                    // no request and changes nothing. A button that reliably
                    // does nothing is worse than no button; the next reconnect
                    // is what fixes this, and the sentence says what is true
                    // meanwhile. Giving the backfill a forced re-run is the real
                    // fix and is a backend change, tracked separately.
                    <EmptyNote icon="triangleAlert">
                      Earlier events couldn’t be loaded
                    </EmptyNote>
                  ) : (
                    // AC6: loaded, and there is nothing. A real answer.
                    <EmptyNote icon="shieldCheck">No security events</EmptyNote>
                  )}
                </td>
              </tr>
            ) : (
              rows.map((e) => {
                const action = ACTION_LABEL[e.action];
                return (
                  <tr key={e.id} className="border-t border-base-border">
                    <td className="py-3 pr-3 font-mono text-base-xs text-base-foreground">
                      {eventTime(e.at)}
                    </td>
                    <td className="py-3 pr-3">
                      <Pill className={action.badge}>{action.label}</Pill>
                    </td>
                    <td className="py-3 pr-3 text-base-xs text-base-foreground">
                      {e.category ?? UNATTRIBUTED}
                    </td>
                    <td className="py-3 pr-3 text-base-xs text-base-foreground">
                      {e.tool ?? UNATTRIBUTED}
                    </td>
                    <td className="max-w-0 truncate py-3 pr-3 text-base-xs text-base-foreground">
                      {e.model ?? UNATTRIBUTED}
                    </td>
                    <td className="py-3 text-right">
                      {/* Straight to the dashboard. This opened
                          `SecurityEventDialog` - a summary of the same six
                          fields the row already draws, with an "Open in
                          dashboard" button under it - until product removed
                          that step on 2026-09-23. The external-link icon was
                          always here and was misleading while it opened a
                          dialog; it is accurate now. */}
                      <button
                        type="button"
                        onClick={() => onOpenInDashboard(e)}
                        className="inline-flex h-8 items-center gap-1.5 rounded-control border border-base-border bg-base-card px-3 text-base-xs text-base-foreground shadow-base-btn-sm"
                      >
                        View
                        <Icon name="squareArrowOutUpRight" size={16} />
                      </button>
                    </td>
                  </tr>
                );
              })
            )}
          </tbody>
        </table>
        {/* Hidden once there is nothing left to reveal, rather than left on
          * screen doing nothing - the same rule the App pane's copy of this
          * control follows, and the reason its own test is called "offers Load
          * more only when there is another page". A control that reliably does
          * nothing is the thing the empty-state comment above already argues
          * against. */}
        {more > 0 && (
          <div className="mt-4 flex justify-center">
            <button
              type="button"
              onClick={() => setVisible((n) => n + PAGE)}
              className="h-8 rounded-control border border-base-border bg-base-card px-3 text-base-xs font-medium leading-4 tracking-button-xs text-base-primary shadow-base-btn-sm transition-colors hover:bg-gray-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
            >
              Load more
            </button>
          </div>
        )}
      </Card>
    </div>
  );
}
