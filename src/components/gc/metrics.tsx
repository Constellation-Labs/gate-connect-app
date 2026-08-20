import { Card } from "./base";
import { Icon } from "./Icon";
import type { IconName } from "./Icon";

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
  /** Whole percent, e.g. 38 renders as "38%". **Null when there is nothing to
   * divide by**, which the design draws as `N/A` rather than `0%` - a period
   * with no traffic saved no tokens, but it did not save none of them either. */
  tokensSavedPercent: number | null;
  /** Pre-formatted and currency-aware upstream, e.g. "+$3.10". Omitted with a
   * null percent, since there is no amount to report. */
  tokensSavedAmount?: string;
}

/**
 * One column of the chart. The Figma legend labels the blue series "Total
 * messages" while stacking it under blocked/flagged/redacted, so the four are
 * treated as additive segments and `total` means "everything not otherwise
 * accounted for". Worth pinning down while the backend is still in flight - if
 * `total` really is the grand total, the stack double-counts.
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

export function StatTiles({ stats }: { stats: UsageStats }) {
  return (
    <Card className="flex">
      <Stat label="Messages" value={stats.messages.toLocaleString()} />
      <Stat
        label="Blocked/Flagged"
        value={stats.blockedFlagged.toLocaleString()}
        divided
      />
      <Stat
        label="Tokens saved"
        value={stats.tokensSavedPercent === null ? "N/A" : `${stats.tokensSavedPercent}%`}
        delta={stats.tokensSavedPercent === null ? undefined : stats.tokensSavedAmount}
        divided
      />
    </Card>
  );
}

function Stat({
  label,
  value,
  delta,
  divided,
}: {
  label: string;
  value: string;
  delta?: string;
  divided?: boolean;
}) {
  return (
    <div className={`flex-1 p-4 ${divided ? "border-l border-base-border" : ""}`}>
      <p className="font-mono text-base-xs font-medium uppercase leading-4 tracking-eyebrow text-base-muted-foreground">
        {label}
      </p>
      <p className="mt-2 flex items-baseline gap-2">
        <span className="text-2xl font-semibold leading-8 text-neutral-900">{value}</span>
        {delta && (
          <span className="text-base-xs font-medium text-green-600">{delta}</span>
        )}
      </p>
    </div>
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
/**
 * What a metrics card shows when the period it covers is genuinely empty, as
 * opposed to still loading. Centred in the card's body, below its heading.
 *
 * The design draws a small mark above the line in both places it appears. Only
 * the Messages one is identifiable (a column chart); the Recent activity mark
 * could not be read at the resolution the frame gave up, so that call site
 * passes no icon rather than inventing one.
 */
export function EmptyNote({ children, icon }: { children: string; icon?: IconName }) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 py-10 text-center">
      {icon && (
        <span aria-hidden className="text-base-muted-foreground">
          <Icon name={icon} size={20} />
        </span>
      )}
      <p className="text-sm leading-5 text-base-muted-foreground">{children}</p>
    </div>
  );
}

export function MessagesChart({ buckets }: { buckets: MessagesBucket[] }) {
  // Empty means "no traffic in the period", which is not the same as no
  // buckets: an hour-by-hour response for a quiet day is 24 buckets of zero,
  // and drawing 24 invisible bars over a labelled axis reads as a broken chart.
  const empty = buckets.every(
    (b) => b.total + b.blocked + b.flagged + b.redacted === 0,
  );

  const peak = Math.max(
    1,
    ...buckets.map((b) => b.total + b.blocked + b.flagged + b.redacted),
  );
  const totals = buckets.reduce(
    (acc, b) => ({
      total: acc.total + b.total,
      blocked: acc.blocked + b.blocked,
      flagged: acc.flagged + b.flagged,
      redacted: acc.redacted + b.redacted,
    }),
    { total: 0, blocked: 0, flagged: 0, redacted: 0 },
  );

  if (empty) {
    return (
      <Card className="p-4">
        <h2 className="text-sm font-medium leading-5 text-neutral-900">Messages</h2>
        <EmptyNote icon="chartColumn">No messages sent in the last 24hrs</EmptyNote>
      </Card>
    );
  }

  return (
    <Card className="p-4">
      <h2 className="text-sm font-medium leading-5 text-neutral-900">Messages</h2>

      <div
        role="img"
        aria-label={
          `Messages over the period: ${totals.total} total, ` +
          `${totals.blocked} blocked, ${totals.flagged} flagged, ` +
          `${totals.redacted} redacted.`
        }
        className="mt-4 flex h-28 items-end justify-between gap-1"
      >
        {buckets.map((bucket) => (
          // `flex-col-reverse` so the first series renders at the *bottom* of
          // the stack: the design bases each bar on the blue total and piles
          // blocked, flagged and redacted on top of it.
          <div key={bucket.label} className="flex h-full w-5 flex-col-reverse">
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
