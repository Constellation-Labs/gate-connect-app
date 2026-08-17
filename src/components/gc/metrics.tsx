import { useState } from "react";
import { Card } from "./base";

/**
 * The usage summary shared by the Overview pane and the per-app pane: one stat
 * card of three columns, and the stacked Messages chart beneath it. Identical in
 * both places in the design, so they live here rather than in either pane.
 *
 * Presentational. The 24-hour backend is still being built, so these types
 * double as the shape that endpoint needs to satisfy.
 */

export interface UsageStats {
  messages: number;
  blockedFlagged: number;
  /** Whole percent, e.g. 38 renders as "38%". */
  tokensSavedPercent: number;
  /** Pre-formatted and currency-aware upstream, e.g. "+$3.10". */
  tokensSavedAmount: string;
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
  /** X-axis tick, e.g. an hour ("14"). */
  label: string;
  total: number;
  blocked: number;
  flagged: number;
  redacted: number;
}

const SERIES = [
  { key: "total", label: "Total messages", className: "bg-chart-messages" },
  { key: "blocked", label: "Blocked", className: "bg-chart-blocked" },
  { key: "flagged", label: "Flagged", className: "bg-chart-flagged" },
  { key: "redacted", label: "Redacted", className: "bg-chart-redacted" },
] as const;

export function StatTiles({
  stats,
  pending,
  onSelectTokensSaved,
}: {
  stats: UsageStats;
  /** First load has not landed yet. Renders em dashes rather than zeros: a
   *  zero is a real reading, and showing one while still loading tells the user
   *  their traffic was nil when we simply do not know yet. */
  pending?: boolean;
  /** Moves to the Token savings section, per AG-572. */
  onSelectTokensSaved?: () => void;
}) {
  const dash = "\u2014";
  return (
    <Card className="flex">
      <Stat label="Messages" value={pending ? dash : stats.messages.toLocaleString()} />
      <Stat
        label="Blocked/Flagged"
        value={pending ? dash : stats.blockedFlagged.toLocaleString()}
        divided
      />
      <Stat
        label="Tokens saved"
        value={pending ? dash : `${stats.tokensSavedPercent}%`}
        delta={pending ? undefined : stats.tokensSavedAmount}
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
  value: string;
  delta?: string;
  divided?: boolean;
  onSelect?: () => void;
}) {
  // A real button when it navigates, so it is focusable and announced as one.
  // `text-left` because a button centres its text by default and these tiles are
  // left-aligned.
  const Tag = onSelect ? "button" : "div";
  return (
    <Tag
      {...(onSelect ? { type: "button" as const, onClick: onSelect } : {})}
      className={`flex-1 p-4 text-left ${divided ? "border-l border-base-border" : ""}${
        onSelect ? " transition hover:bg-gray-50" : ""
      }`}
    >
      <p className="font-mono text-base-xs font-medium uppercase leading-4 tracking-eyebrow text-base-muted-foreground">
        {label}
      </p>
      <p className="mt-2 flex items-baseline gap-2">
        <span className="text-2xl font-semibold leading-8 text-neutral-900">{value}</span>
        {delta && (
          <span className="text-base-xs font-medium text-green-600">{delta}</span>
        )}
      </p>
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
export function MessagesChart({ buckets }: { buckets: MessagesBucket[] }) {
  const [hovered, setHovered] = useState<number | null>(null);
  const peak = Math.max(
    1,
    ...buckets.map((b) => b.total + b.blocked + b.flagged + b.redacted),
  );
  return (
    <Card className="p-4">
      <h2 className="text-sm font-medium leading-5 text-neutral-900">Messages</h2>

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
            key={bucket.label}
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
            key={bucket.label}
            className="w-5 text-center font-mono text-base-2xs text-base-muted-foreground"
          >
            {bucket.label}
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
            <tr key={b.label}>
              <th scope="row">{b.label}:00</th>
              <td>{b.total}</td>
              <td>{b.blocked}</td>
              <td>{b.flagged}</td>
              <td>{b.redacted}</td>
              <td>{b.total + b.blocked + b.flagged + b.redacted}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <ul className="mt-4 flex items-center justify-center gap-4">
        {SERIES.map(({ key, label, className }) => (
          <li key={key} className="flex items-center gap-1.5">
            <span aria-hidden className={`size-3 rounded-sm ${className}`} />
            <span className="text-base-xs text-neutral-600">{label}</span>
          </li>
        ))}
      </ul>
    </Card>
  );
}

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
      className="pointer-events-none absolute top-1/2 z-10 w-[12.5rem] -translate-y-1/2 rounded-lg border border-base-border bg-base-card p-2 shadow-base-md"
      style={
        side === "right"
          ? { left: `calc(${offset * 100}% + 0.75rem)` }
          : { right: `calc(${(1 - offset) * 100}% + 0.75rem)` }
      }
    >
      <p className="font-mono text-sm font-medium uppercase leading-5 tracking-eyebrow-14 text-neutral-900">
        {bucket.label}
      </p>
      <div className="mt-2 flex flex-col gap-1">
        {SERIES.map(({ key, label, className }) => (
          <div key={key} className="flex items-center justify-between gap-2">
            <span className="flex items-center gap-2">
              <span className={`size-3 rounded-sm ${className}`} />
              <span className="text-base-xs leading-4 text-neutral-900">{label}</span>
            </span>
            <span className="text-base-xs font-medium leading-4 text-neutral-900">
              {bucket[key].toLocaleString()}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
