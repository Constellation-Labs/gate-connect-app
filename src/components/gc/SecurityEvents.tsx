import type { ReactNode } from "react";
import type { FeedState, SecurityEvent } from "../../lib/api";
import { BADGE_STYLES, Card, EmptyNote, Pill, Skeleton } from "./base";
import { Modal } from "./Modal";
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
 *  it now lives on - today the tray's security card, through
 *  `security-events-requested`. The
 *  Token savings section above it carries the same kind of target for the same
 *  kind of caller; see `Overview`'s `SAVINGS_SECTION_ID`. */
export const SECURITY_SECTION_ID = "security-events";

/** What the feed's own connection is doing, in the design's words.
 *
 * Green / amber / neutral rather than green / amber / red: an offline feed is not
 * an error, it is a feed that is not running, and painting it red next to a
 * perfectly healthy routing switch invites the reading that routing broke too.
 * That is the whole distinction AC4 asks the screen to hold. */
const FEED_LABEL: Record<FeedState, { label: string; className: string }> = {
  live: { label: "Live", className: "bg-green-100 text-green-900" },
  reconnecting: { label: "Reconnecting", className: "bg-amber-100 text-amber-900" },
  offline: { label: "Offline", className: "bg-gray-100 text-neutral-700" },
};

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
  state: FeedState;
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
  onOpenEvent: (event: SecurityEvent) => void;
}

export function SecurityEvents({
  events,
  state,
  loading,
  unavailable,
  historyUnavailable,
  onRetry,
  onOpenEvent,
}: SecurityEventsProps) {
  const feed = FEED_LABEL[state];
  // Newest first on screen: a feed is read from the top, and the event a user
  // scrolled down here for is the one that just happened.
  const rows = [...events].reverse();

  return (
    // The pane's own `gap-4` separated the notice from the card while this was a
    // screen; as a section it has to carry that itself, so the two arrive as one
    // child of the pane. The id and `scroll-mt-6` are on the wrapper rather than
    // the card so a jump from the tray lands above the notice, not past it -
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
          * above draw. The pill sits on it because it qualifies the rows
          * underneath - it was in the pane header for the same reason, and the
          * card header is where that header's job went. `items-baseline` so the
          * uppercase pill sits on the heading's baseline rather than centring
          * against a taller line box. */}
        <div className="flex items-baseline justify-between gap-3">
          <h2 className="text-base font-medium leading-6 tracking-heading-16 text-base-foreground">
            Security events
          </h2>
          {/* The feed's own connection, never routing's - and never the
              Overview's activity read either, which is a different question
              answered by a different backend. `role="status"` so a screen
              reader hears the transition without the table moving.
            *
            * **A pill is a reading too**, which is why it waits. `state` is
            * seeded `"offline"` and replaced when the mount read answers
            * (`useSecurityFeed`), so an unguarded pill reports a connection
            * nobody has checked yet - and since the feed moved onto the pane the
            * window opens on, that is every cold launch rather than the rare
            * glimpse it was behind a rail entry. Principle 6's "a value still in
            * flight draws a `Skeleton`", applied to the one figure on this card
            * that is not a row. The tray refuses the same claim by hiding its
            * card outright while loading; this card is the section, so it cannot
            * leave, and draws the placeholder instead. */}
          {loading ? (
            <Skeleton className="h-6 w-20 shrink-0" />
          ) : (
            <span role="status" aria-label={`Event feed ${feed.label}`}>
              <Pill className={feed.className}>{feed.label}</Pill>
            </span>
          )}
        </div>
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
                      <button
                        type="button"
                        onClick={() => onOpenEvent(e)}
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
      </Card>
    </div>
  );
}

/**
 * One event's summary, and the way through to the dashboard (AC2, AC7).
 *
 * The dialog does **not** close when Open in dashboard is clicked. AC7 asks for
 * the summary to stay visible "until the matching dashboard detail opens", and
 * opening a browser is a thing that can fail: the opener returns a classified
 * error rather than throwing, so on a failure the user is left looking at the
 * event they asked about instead of at the pane behind a banner. The caller
 * closes it once the open succeeded.
 *
 * Nothing here is content. The fields are the same six the row draws, which is
 * all the payload carries - there is no "show more" behind this, deliberately,
 * because the evidence it would show is what AC3 forbids.
 */
export function SecurityEventDialog({
  event,
  onClose,
  onOpenDashboard,
}: {
  event: SecurityEvent;
  onClose: () => void;
  onOpenDashboard: () => void;
}) {
  const action = ACTION_LABEL[event.action];
  return (
    <Modal
      icon={event.action === "block" ? "shieldBan" : "triangleAlert"}
      tone={event.action === "block" ? "danger" : "warning"}
      title={`${action.label} request`}
      subtitle={eventTime(event.at)}
      width={512}
      closeButton
      onDismiss={onClose}
      secondary={{ label: "Close", onClick: onClose }}
      primary={{ label: "Open in dashboard", onClick: onOpenDashboard }}
    >
      <dl className="flex flex-col gap-2 text-base-xs">
        {[
          ["Category", event.category],
          ["Tool", event.tool],
          ["Model", event.model],
          ["Provider", event.provider],
        ].map(([label, value]) => (
          <div key={label} className="flex items-baseline justify-between gap-4">
            <dt className="text-base-muted-foreground">{label}</dt>
            <dd className="text-base-foreground">{value ?? UNATTRIBUTED}</dd>
          </div>
        ))}
        <div className="flex items-baseline justify-between gap-4">
          <dt className="text-base-muted-foreground">Request</dt>
          {/* Mono: an identifier, and the one thing on this dialog the user
              might read back to support. */}
          <dd className="font-mono text-base-foreground">{event.requestId}</dd>
        </div>
      </dl>
    </Modal>
  );
}
