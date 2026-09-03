import type { ReactNode } from "react";
import type { FeedState, SecurityEvent } from "../../lib/api";
import { BADGE_STYLES, Card, EmptyNote, Pill, Skeleton } from "./base";
import { Modal } from "./Modal";
import { Icon } from "./Icon";

/**
 * The live security-event feed (AG-578).
 *
 * A chronological list of the requests Gate blocked or flagged, arriving while
 * the app is open. Everything the pane can show is on the row: what happened, what
 * category fired, which tool, which model, when, and a way through to the
 * dashboard. Everything it *cannot* show is the point of the screen - no prompt,
 * no response, no matched secret, no evidence. Those fields are omitted by the
 * gateway rather than hidden here, so there is nothing on the client to leak.
 *
 * **Undrawn in Figma.** The Sidenav frame (408:15625) draws Overview and Settings
 * only, and no frame draws this pane. It is built from the component set, which
 * is what CLAUDE.md asks for where no frame draws the thing: the pane layout is
 * `Overview`'s, the table is `AppPane`'s recent-activity table, and the badges are
 * the shared `BADGE_STYLES` pair. Recorded as a deviation in
 * `plans/new-app-ui-figma.md`.
 */

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

/** The gateway's verb, in the pane's vocabulary - the same mapping
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

export function SecurityPane({
  events,
  state,
  loading,
  unavailable,
  onRetry,
  onOpenEvent,
}: {
  /** Oldest first, as the feed buffers them. The table reverses for display. */
  events: SecurityEvent[];
  state: FeedState;
  loading: boolean;
  /** The feed could not be read at all. Distinct from an empty feed, and the
   *  distinction is AC6's whole point. */
  unavailable: boolean;
  onRetry: () => void;
  onOpenEvent: (event: SecurityEvent) => void;
}) {
  const feed = FEED_LABEL[state];
  // Newest first on screen: a feed is read from the top, and the event a user
  // opened the pane for is the one that just happened.
  const rows = [...events].reverse();

  return (
    <div className="flex flex-1 flex-col gap-4 overflow-auto bg-base-background p-6">
      <header className="flex items-baseline justify-between">
        <h1 className="text-xl font-medium tracking-heading text-base-foreground">
          Security events
        </h1>
        <div className="flex items-center gap-3">
          {/* The feed's own connection, never routing's. `role="status"` so a
              screen reader hears the transition without the table moving. */}
          <span role="status" aria-label={`Event feed ${feed.label}`}>
            <Pill className={feed.className}>{feed.label}</Pill>
          </span>
        </div>
      </header>

      <Card className="p-4" busy={loading}>
        {loading && <span className="sr-only">Loading security events</span>}
        <table className="w-full">
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
 * event they asked about instead of at an empty pane behind a banner. The caller
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
