import { useState } from "react";
import { Card, EmptyNote, Skeleton } from "./base";

/**
 * The usage summary shared by the Overview pane and the per-app pane: one stat
 * card of three columns, and the stacked Messages chart beneath it. Identical in
 * both places in the design, so they live here rather than in either pane.
 *
 * Presentational. `lib/activity.ts` adapts `GET /v1/me/activity` into these
 * types, so they double as the shape that endpoint has to satisfy.
 */

/**
 * The three counters on the stat card.
 *
 * **Every field is nullable, and null means no reading**, not zero. A counter
 * that answered `0` is a measurement and prints as `0`; a counter with no
 * reading behind it prints `N/A`. AG-576's rule, and the reason this type is
 * nullable at all: "0 blocked" is a claim about the user's traffic, and the one
 * place they would look to check it is the tile that just made it up.
 *
 * The two render identically in the mock only because the mock draws one case.
 * Figma 228:89333 is an org with no traffic, where the counts genuinely are zero
 * and Tokens saved genuinely has no figure - so it reads `0` / `0` / `N/A`, and
 * this rule reproduces that exactly. Nothing was read at all is the other case,
 * and it reads `N/A` three times, which is what the chart and the tables beneath
 * it are already saying in words.
 */
export interface UsageStats {
  messages: number | null;
  blockedFlagged: number | null;
  /** Whole percent, e.g. 38 renders as "38%". */
  tokensSavedPercent: number | null;
  /** Pre-formatted and currency-aware upstream, e.g. "+$3.10". */
  tokensSavedAmount: string | null;
}

/**
 * One column of the chart. The Figma legend labels the blue series "Total
 * messages" while stacking it under blocked/flagged/redacted, so the four are
 * additive segments and `total` means "everything not otherwise accounted for".
 *
 * The chart tooltip settles this: its four rows read 8 / 2 / 2 / 0 against a
 * bar whose heading is the bucket, and the heading carries the `mono/eyebrow`
 * style the axis ticks use - an identifier, not a figure. So "Total messages"
 * really is the remainder segment and the stack does not double-count.
 */
export interface MessagesBucket {
  /**
   * Stable identity for this bucket, used as the React key in all three lists
   * that render it. The endpoint's own UTC hour, not the label.
   *
   * `label` is a *local* hour-of-day, and on a DST fall-back two UTC buckets map
   * to the same local hour - which would mean duplicate keys in the bars, the axis
   * ticks and the sr-only table at once, twice a year, in one timezone class. The
   * contract's 24 distinct UTC hours are the thing that is actually unique.
   */
  id: string;
  /** The bucket's local hour, as a number ("14"). Display only; see `id`.
   *  Rendered through {@link hourTick} rather than printed raw. */
  label: string;
  total: number;
  blocked: number;
  flagged: number;
  redacted: number;
}

/**
 * A bucket's hour as the axis draws it: zero-padded, on the hour ("00:00").
 *
 * The redrawn chart (`706:9997`, read 2026-08-28) labels every tick this way;
 * it used to draw a bare hour, which is what `label` still carries and what the
 * accessible table used to suffix by hand. One function so the axis, the
 * tooltip heading and that table cannot phrase one bucket three ways.
 *
 * The `chart/tooltip` card is the exception the file has not caught up on: it
 * is an older node (`191:*`) than the redrawn axis around it and still heads
 * itself with a bare hour. A heading that names a column has to match the
 * column, so it follows the axis here.
 */
export function hourTick(label: string): string {
  return `${label.padStart(2, "0")}:00`;
}

const SERIES = [
  { key: "total", label: "Total messages", className: "bg-chart-messages" },
  { key: "blocked", label: "Blocked", className: "bg-chart-blocked" },
  { key: "flagged", label: "Flagged", className: "bg-chart-flagged" },
  { key: "redacted", label: "Redacted", className: "bg-chart-redacted" },
] as const;

/** What any counter reads with no figure behind it (Figma 228:89341, where
 *  Tokens saved is the one with nothing to report). */
const UNAVAILABLE = "N/A";

export function StatTiles({
  stats,
  pending,
  onSelectTokensSaved,
}: {
  stats: UsageStats;
  /** First load has not landed yet. Renders skeletons rather than figures: a
   *  zero is a real reading, and `N/A` says there is none. Neither is true
   *  while we are still asking. */
  pending?: boolean;
  /** Moves to the Token savings section, per AG-572. */
  onSelectTokensSaved?: () => void;
}) {
  // Three states, and they are three: `null` here means "still loading" and
  // draws a skeleton, `N/A` means there is no reading behind this counter, and
  // a number - including zero - is a reading and prints as one.
  const count = (value: number | null) =>
    pending ? null : value === null ? UNAVAILABLE : value.toLocaleString();
  return (
    <Card className="flex" busy={pending}>
      {pending && <span className="sr-only">Loading your activity</span>}
      <Stat label="Messages" value={count(stats.messages)} />
      <Stat label="Blocked/Flagged" value={count(stats.blockedFlagged)} divided />
      <Stat
        label="Tokens saved"
        value={
          pending
            ? null
            : stats.tokensSavedPercent === null
              ? UNAVAILABLE
              : `${stats.tokensSavedPercent}%`
        }
        delta={pending ? undefined : (stats.tokensSavedAmount ?? undefined)}
        divided
        onSelect={onSelectTokensSaved}
      />
    </Card>
  );
}

function Stat({
  label,
  value,
  delta,
  divided,
  onSelect,
}: {
  label: string;
  /** Null while the reading is in flight; see `StatTiles`. */
  value: string | null;
  delta?: string;
  divided?: boolean;
  onSelect?: () => void;
}) {
  // A real button when it navigates, so it is focusable and announced as one.
  // `text-left` because a button centres its text by default and these tiles are
  // left-aligned.
  const Tag = onSelect ? "button" : "div";
  // `span.block`, not `<p>`. When `Tag` is a button these are inside it, and `<p>`
  // is not valid phrasing content there - the parser is then free to restructure
  // the tile, which is the sort of thing that renders correctly until it does not.
  // Renders identically.
  return (
    <Tag
      {...(onSelect ? { type: "button" as const, onClick: onSelect } : {})}
      className={`flex-1 p-4 text-left ${divided ? "border-l border-base-border" : ""}${
        onSelect ? " transition hover:bg-gray-50" : ""
      }`}
    >
      <span className="block font-mono text-base-xs font-medium uppercase leading-4 tracking-eyebrow text-base-muted-foreground">
        {label}
      </span>
      <span className="mt-2 flex items-baseline gap-2">
        {value === null ? (
          // The height of the figure it stands in for, so the card does not
          // resize under the user when the reading lands.
          <Skeleton className="my-1 h-6 w-16" />
        ) : (
          <span className="text-2xl font-semibold leading-8 text-base-foreground">{value}</span>
        )}
        {delta && (
          <span className="text-base-xs font-medium text-green-600">{delta}</span>
        )}
      </span>
    </Tag>
  );
}

/**
 * Stacked bars, hand-rolled rather than pulling in a chart library: the mark is
 * four stacked rectangles, and hand-rolling keeps the series on design tokens
 * instead of theming around a library.
 *
 * Bars are the design's 20px wide and distribute across the card, so the same
 * markup holds whether the backend returns 24 buckets or fewer.
 */
export function MessagesChart({
  buckets,
  pending,
  unavailable,
}: {
  buckets: MessagesBucket[];
  /** The series is on its way. Draws placeholder columns rather than an empty
   *  plot, which would say "no traffic" a beat before the traffic appears. */
  pending?: boolean;
  /** No series was read at all - the gateway declined it, or the fetch failed.
   *  Kept apart from an empty series, because "we were not told" and "nothing
   *  was sent" are different sentences and only one of them is about the user's
   *  traffic. The gap notice above the pane says which. */
  unavailable?: boolean;
}) {
  const [hovered, setHovered] = useState<number | null>(null);
  const highest = buckets.reduce(
    (m, b) => Math.max(m, b.total + b.blocked + b.flagged + b.redacted),
    0,
  );
  // Floored at 1 so a series of zeroes divides rather than producing NaN heights.
  // Kept separate from `highest`, which is the honest maximum and the only thing
  // that can answer whether anything happened.
  const peak = Math.max(1, highest);
  // A dense series is what the endpoint returns - an hour with no traffic is a
  // zero bar, not a missing one - so "nothing happened" is 24 zeroes rather than
  // an empty array. Both land here as a highest of zero.
  const empty = !pending && !unavailable && highest === 0;
  return (
    <Card className="p-4" busy={pending}>
      <h2 className="text-sm font-medium leading-5 text-base-foreground">Messages</h2>

      {pending ? (
        <PendingChart />
      ) : unavailable ? (
        // Not a sentence about their traffic. The pane's gap notice carries the
        // cause and the retry; this only refuses to draw a plot for a series
        // nobody sent us.
        <EmptyNote icon="chartColumn">Messages couldn&apos;t be read</EmptyNote>
      ) : empty ? (
        <EmptyNote icon="chartColumn">No messages sent in the last 24hrs</EmptyNote>
      ) : (
        <>
      {/* The bars are decoration for assistive tech; the table below carries the
          numbers. `role="img"` with a summary label used to be the whole story,
          which meant a screen-reader user got period totals and could not reach
          any individual hour - AG-572 requires the hour, its total and its
          security count to be readable without hover. `aria-hidden` rather than
          a per-bar label so the same figures are not announced twice.

          The tooltip lives inside this subtree and is hover-only for the same
          reason: it repeats what the table already says, so exposing it twice
          would be noise. Nothing here is keyboard-reachable, and nothing needs
          to be - the table is the accessible path to the same figures. */}
      <div
        aria-hidden
        className="relative mt-4 flex h-28 items-end justify-between gap-1"
        onMouseLeave={() => setHovered(null)}
      >
        {buckets.map((bucket, i) => (
          // `flex-col-reverse` so the first series renders at the *bottom* of
          // the stack: the design bases each bar on the blue total and piles
          // blocked, flagged and redacted on top of it.
          //
          // `h-full` on the hit target, not just the bar: a quiet hour is a
          // sliver two pixels tall, and hovering it should not require aim.
          <div
            key={bucket.id}
            className="flex h-full w-5 flex-col-reverse"
            onMouseEnter={() => setHovered(i)}
          >
            {SERIES.map(({ key, className }) => {
              const value = bucket[key];
              if (!value) return null;
              return (
                <div
                  key={key}
                  className={className}
                  style={{ height: `${(value / peak) * 100}%` }}
                />
              );
            })}
          </div>
        ))}

        {hovered !== null && buckets[hovered] && (
          <ChartTooltip
            bucket={buckets[hovered]}
            // Flip to the left of the cursor over the last third, so the card
            // stays inside the chart instead of hanging off the card's edge.
            side={hovered > (buckets.length - 1) * (2 / 3) ? "left" : "right"}
            offset={buckets.length > 1 ? hovered / (buckets.length - 1) : 0}
          />
        )}
      </div>

      <div className="mt-1 flex justify-between gap-1">
        {buckets.map((bucket) => (
          <span
            key={bucket.id}
            className="w-8 text-center font-mono text-base-2xs text-base-muted-foreground"
          >
            {hourTick(bucket.label)}
          </span>
        ))}
      </div>

      {/* Visually hidden, not display:none - a table is the honest structure for
          24 rows of four figures, and it gives AT users row/column navigation
          instead of one long sentence. Keyboard users reach it in reading order
          with no hover, which is the requirement.

          Column names track the tooltip, so "Total messages" is the remainder
          series and the sum gets its own column. Naming the sum "Total messages"
          while the visible legend used the same words for the blue segment gave
          two different figures the same name. */}
      <table className="sr-only">
        <caption>Messages per hour over the period</caption>
        <thead>
          <tr>
            <th scope="col">Hour</th>
            <th scope="col">Total messages</th>
            <th scope="col">Blocked</th>
            <th scope="col">Flagged</th>
            <th scope="col">Redacted</th>
            <th scope="col">All messages</th>
          </tr>
        </thead>
        <tbody>
          {buckets.map((b) => (
            <tr key={b.id}>
              <th scope="row">{hourTick(b.label)}</th>
              <td>{b.total}</td>
              <td>{b.blocked}</td>
              <td>{b.flagged}</td>
              <td>{b.redacted}</td>
              <td>{b.total + b.blocked + b.flagged + b.redacted}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <ul className="mt-4 flex items-center gap-6 border-t border-base-border pt-4">
        {SERIES.map(({ key, label, className }) => (
          <li key={key} className="flex items-center gap-2">
            <span aria-hidden className={`size-3 rounded-xs ${className}`} />
            <span className="text-base-xs text-base-foreground">{label}</span>
          </li>
        ))}
      </ul>
        </>
      )}
    </Card>
  );
}

/**
 * The chart's placeholder, as `overview-loading` (228:85602) draws it: one
 * uniform full-height column per hour of the period, over the real numbered
 * ticks and the real legend - the card keeps its shape and only the readings
 * are missing. An earlier version drew a fixed silhouette at varied heights,
 * which read as data that had already arrived.
 */
function PendingChart() {
  return (
    <>
      <div aria-hidden className="mt-4 flex h-28 items-end justify-between gap-1">
        {PENDING_HOURS.map((hour) => (
          <Skeleton key={hour} className="h-full w-5" />
        ))}
      </div>
      <div aria-hidden className="mt-1 flex justify-between gap-1">
        {PENDING_HOURS.map((hour) => (
          <span
            key={hour}
            // The tick box is the loaded axis's, not this frame's: a narrower
            // placeholder would let the axis jump sideways the moment the
            // reading lands, which is the one thing a placeholder must not do.
            className="w-8 text-center font-mono text-base-2xs text-base-muted-foreground"
          >
            {hour}
          </span>
        ))}
      </div>
      <ul className="mt-4 flex items-center gap-6 border-t border-base-border pt-4">
        {SERIES.map(({ key, label, className }) => (
          <li key={key} className="flex items-center gap-2">
            <span aria-hidden className={`size-3 rounded-xs ${className}`} />
            <span className="text-base-xs text-base-foreground">{label}</span>
          </li>
        ))}
      </ul>
    </>
  );
}

/** Positional ticks for the placeholder, 1..24 as the frame draws them. Indexes,
 *  not hours-ago: the real axis replaces them with the data's own labels. */
const PENDING_HOURS = Array.from({ length: 24 }, (_, i) => i + 1);

/**
 * The hovered bucket's four figures (Figma `chart/tooltip`).
 *
 * The heading is the bucket label in `mono/eyebrow`, the same style the axis
 * ticks carry - it names the column, it is not a fifth number. Rows repeat the
 * legend in order so the eye maps swatch to segment without re-reading.
 *
 * Positioned by percentage across the plot area rather than by measuring the
 * bar: the bars already distribute themselves, so a ratio lands on the right
 * column at any bucket count and needs no layout read.
 */
function ChartTooltip({
  bucket,
  side,
  offset,
}: {
  bucket: MessagesBucket;
  /** Which side of the hovered column the card opens towards. */
  side: "left" | "right";
  /** 0 at the first bucket, 1 at the last. */
  offset: number;
}) {
  return (
    <div
      className="pointer-events-none absolute top-1/2 z-10 w-[12.5rem] -translate-y-1/2 rounded-md border border-base-border bg-base-card p-2 shadow-base-md"
      style={
        side === "right"
          ? { left: `calc(${offset * 100}% + 0.75rem)` }
          : { right: `calc(${(1 - offset) * 100}% + 0.75rem)` }
      }
    >
      <p className="font-mono text-sm font-medium uppercase leading-5 tracking-eyebrow-14 text-base-foreground">
        {hourTick(bucket.label)}
      </p>
      <div className="mt-2 flex flex-col gap-1">
        {SERIES.map(({ key, label, className }) => (
          <div key={key} className="flex items-center justify-between gap-2">
            <span className="flex items-center gap-2">
              <span className={`size-3 rounded-xs ${className}`} />
              <span className="text-base-xs leading-4 text-base-foreground">{label}</span>
            </span>
            <span className="text-base-xs font-medium leading-4 text-base-foreground">
              {bucket[key].toLocaleString()}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
