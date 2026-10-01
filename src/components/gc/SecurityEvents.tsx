import { useState } from "react";
import type { ReactNode } from "react";
import type { SecurityEvent } from "../../lib/api";
import { attributed, vendorFromModelId } from "../../lib/toolEvents";
import type { ModelLabels } from "../../lib/toolModels";
import { toolMarkFor } from "./BrandMark";
import { VendorMark } from "./ProviderMark";
import { BADGE_STYLES, Card, CardHeader, EmptyNote, GUARDRAIL_INK, OutlineButton, Pill, Skeleton } from "./base";
import type { IconName } from "./Icon";
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

/**
 * The Category cell: a 20px glyph and a short label (`1402:18010`), where the
 * cell used to print the gateway's raw category string. The frame draws three
 * of the gateway's five - `pii` as "PII" on `UserRound`, `injection` as
 * "Injection" on `ShieldAlert`, `credential` as "Credential" on `KeyRound` -
 * in the same three inks the Policies rows above take for the same guardrails.
 * `phi` and `other` are not drawn (`plans/new-app-ui-figma.md`, design sync 2026-09-30,
 * question 5); `phi` takes the PII/PHI scanner's glyph because it is that
 * policy's other half, and `other` is printed as a word with no glyph rather
 * than given one by eye. A category outside the five is printed as received.
 */
const CATEGORY: Record<string, { label: string; icon?: IconName }> = {
  pii: { label: "PII", icon: "userRound" },
  phi: { label: "PHI", icon: "userRound" },
  injection: { label: "Injection", icon: "shieldAlert" },
  credential: { label: "Credential", icon: "key" },
  other: { label: "Other" },
};

function Category({ value }: { value: string }) {
  // `hasOwn`, not a bare index: the value is the gateway's string, and a plain
  // object answers "constructor" or "toString" with a function, which would
  // render as a blank cell rather than the string received. `lib/toolEvents.ts`
  // guards its own tables the same way.
  const c = Object.hasOwn(CATEGORY, value) ? CATEGORY[value] : { label: value };
  return (
    <span className="flex items-center gap-3 text-sm font-medium leading-5 tracking-heading-14 text-base-foreground">
      {c.icon && <Icon name={c.icon} size={20} className={GUARDRAIL_INK[c.icon]} />}
      {c.label}
    </span>
  );
}

/** What a cell says when the gateway attributed nothing to it.
 *
 * Not an error and not a withholding: an agent whose User-Agent is not on the
 * gateway's allowlist is recorded unattributed rather than guessed at, which is
 * the honest outcome and an ordinary one. */
const UNATTRIBUTED = "-";

/** `label/14` Medium in `base/muted-foreground` on a 16/12 row (`1402:17995`),
 *  as on the Policies and Token savings tables above. */
function Th({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <th
      scope="col"
      className={`py-3 pl-4 text-left text-sm font-medium leading-5 text-base-muted-foreground ${className}`}
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
        <tr key={i} className="h-14 border-t border-base-border">
          {/* Six cells under six columns, the last one the View button's. */}
          {[0, 1, 2, 3, 4, 5].map((c) => (
            <td key={c} className="pl-4 last:pr-4">
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
  /** The catalogue as a lookup (`modelLabelsFor`), for the Model cell's name
   *  and, where the id names no vendor, its mark - the labels the dashboard's
   *  Messages list draws. A prop rather than a field on `SecurityEvent`, which
   *  is the wire contract and must not grow a client-side value; the app pane's
   *  rows are adapted, and take theirs in `labelEntries` instead.
   *
   *  The rows here carry `resolved_model`, the provider's own spelling of the
   *  id, which is why the lookup is a function and not the catalogue's map:
   *  `modelLabelsFor` says which of those spellings it can answer and which
   *  keep the id. Absent, or finding nothing, the cell keeps the id; either way
   *  the id stays on hover. */
  modelLabels?: ModelLabels;
  /** Product names by tool slug ("Claude Code" for `claude-code`), from the
   *  registry's `Tool.product_name`, for the Tool cell. The gateway sends its
   *  own platform id, which for the tools this app routes is the registry slug;
   *  a platform the registry has no row for (`claude-desktop`, `cursor`) is not
   *  in the map and prints as its id. */
  toolNames?: ReadonlyMap<string, string>;
}

export function SecurityEvents({
  events,
  loading,
  unavailable,
  historyUnavailable,
  onRetry,
  onOpenInDashboard,
  modelLabels,
  toolNames,
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

      <Card busy={loading}>
        {loading && <span className="sr-only">Loading security events</span>}
        {/* The header row the Policies and Token savings cards above draw,
          * without their action. The frame (`1402:17988`) titles this card
          * "Recent activity" and gives it a "View activity" button; whether
          * that card is this feed is open (question 4 in
          * `plans/new-app-ui-figma.md`, design sync 2026-09-30), and the dashboard
          * has no activity URL to send the button to, so the title and the
          * Load more below stay. */}
        {/* The feed's connection pill - Live / Reconnecting / Offline - sat
          * beside this heading until 2026-09-23, when product asked for it to
          * go. "Live" was true on every healthy launch and said nothing; the
          * cost is that the two unhappy states no longer have a surface
          * either, the tray's security card having been removed in #334. An
          * offline feed and a quiet machine now look the same here. Raised
          * with the decision, not overlooked. */}
        <CardHeader title="Security events" />
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
                <td colSpan={6} className="px-4 pb-4">
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
                // The security bus passes the pipeline's `unknown` sentinel
                // through on both columns, where the tool-events endpoint maps
                // it to null; `attributed` makes the same reading here.
                const provider = attributed(e.provider);
                const model = attributed(e.model);
                const label = model === null ? undefined : modelLabels?.(model);
                const toolMark = e.tool === null ? undefined : toolMarkFor(e.tool, 20);
                return (
                  // 56px rows with a full-width divider, 16px cells, `copy/14`
                  // sans throughout (`1402:18003`). The time was mono 12px until
                  // the 2026-09-29 redraw; a timestamp is a value, not machine
                  // output, and CLAUDE.md's sans rule for identifier values
                  // already said so. The frame draws a coloured tool logo
                  // beside the tool (`1402:18014`, 20px) and the provider mark
                  // beside the model (`1402:18017`, 20px); both are drawn, the
                  // first from `toolMarkFor`, the second from the provider, the
                  // id's namespace or the catalogue - see `VendorMark`.
                  <tr key={e.id} className="h-14 border-t border-base-border">
                    <td className="whitespace-nowrap pl-4 text-sm leading-5 text-base-foreground">
                      {eventTime(e.at)}
                    </td>
                    <td className="pl-4">
                      <Pill className={action.badge}>{action.label}</Pill>
                    </td>
                    <td className="pl-4 text-sm leading-5 text-base-foreground">
                      {e.category ? <Category value={e.category} /> : UNATTRIBUTED}
                    </td>
                    <td className="max-w-0 pl-4">
                      {e.tool === null ? (
                        <span className="text-sm leading-5 text-base-foreground">{UNATTRIBUTED}</span>
                      ) : (
                        <span className="flex items-center gap-2">
                          {/* The mark is decorative; the name beside it is the
                              text. A tool with no mark keeps the slot so the
                              names in the column line up. */}
                          <span
                            aria-hidden
                            className="flex size-5 shrink-0 items-center justify-center text-base-foreground"
                          >
                            {toolMark}
                          </span>
                          {/* The slug on hover, as the model cell keeps its id. */}
                          <span
                            className="truncate text-sm leading-5 text-base-foreground"
                            title={e.tool}
                          >
                            {toolNames?.get(e.tool) ?? e.tool}
                          </span>
                        </span>
                      )}
                    </td>
                    <td className="max-w-0 pl-4">
                      <span className="flex items-center gap-2">
                        <VendorMark
                          provider={provider}
                          vendor={vendorFromModelId(model) ?? label?.vendor ?? null}
                          size={20}
                        />
                        {/* Truncated inside the cell rather than on it, so the
                            mark keeps its 20px while the name gives way. The id
                            on hover: the text may be the catalogue's label. */}
                        <span
                          className="truncate text-sm leading-5 text-base-foreground"
                          title={model ?? undefined}
                        >
                          {model === null ? UNATTRIBUTED : (label?.name ?? model)}
                        </span>
                      </span>
                    </td>
                    <td className="pl-4 pr-4 text-right">
                      {/* Straight to the dashboard. This opened
                          `SecurityEventDialog` - a summary of the same six
                          fields the row already draws, with an "Open in
                          dashboard" button under it - until product removed
                          that step on 2026-09-23. The external-link icon was
                          always here and was misleading while it opened a
                          dialog; it is accurate now. */}
                      {/* The `xs` Outline variant (`1402:18021`), not the `sm`
                          the header button takes. */}
                      <OutlineButton size="xs" onClick={() => onOpenInDashboard(e)} external>
                        View
                      </OutlineButton>
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
          <div className="flex justify-center border-t border-base-border px-4 py-4">
            <OutlineButton size="sm" onClick={() => setVisible((n) => n + PAGE)}>
              Load more
            </OutlineButton>
          </div>
        )}
      </Card>
    </div>
  );
}
