import type { ReactNode } from "react";
import { Card } from "./base";
import { Icon } from "./Icon";
import type { IconName } from "./Icon";

/**
 * The Overview pane (Figma `Flows / Overview`): a 24-hour summary of what Gate
 * actually did with the user's traffic. Sits right of the sidebar in the
 * 1024x720 window, on a gray/100 ground, and scrolls internally.
 *
 * Presentational. The 24-hour backend is still being built, so every number
 * arrives as a prop and nothing here talks to `lib/api`.
 */

export interface OverviewStats {
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

export type PolicyAction = "block" | "flag" | "redact";

export interface Policy {
  id: string;
  name: string;
  icon: IconName;
  action: PolicyAction;
  enabled: boolean;
}

export interface Saving {
  id: string;
  name: string;
  icon: IconName;
  enabled: boolean;
}

const SERIES = [
  { key: "total", label: "Total messages", className: "bg-chart-messages" },
  { key: "blocked", label: "Blocked", className: "bg-chart-blocked" },
  { key: "flagged", label: "Flagged", className: "bg-chart-flagged" },
  { key: "redacted", label: "Redacted", className: "bg-chart-redacted" },
] as const;

/**
 * Action pills. Text sits at the palette's 900 level; the 100 backgrounds are
 * inferred from that pairing rather than sampled from Figma.
 */
const ACTION_STYLES: Record<PolicyAction, string> = {
  block: "bg-red-100 text-red-900",
  flag: "bg-amber-100 text-amber-900",
  redact: "bg-purple-100 text-purple-900",
};

export function Overview({
  stats,
  buckets,
  policies,
  savings,
  onManagePolicies,
  onManageSavings,
  alert,
  period = "Last 24 hours",
}: {
  stats: OverviewStats;
  buckets: MessagesBucket[];
  policies: Policy[];
  savings: Saving[];
  onManagePolicies: () => void;
  onManageSavings: () => void;
  /** Slot for an `AlertBanner`, which the design places above the stat tiles. */
  alert?: ReactNode;
  period?: string;
}) {
  return (
    <div className="flex flex-1 flex-col gap-4 overflow-auto bg-gray-100 p-6">
      <header className="flex items-baseline justify-between">
        <h1 className="text-xl font-medium leading-6 tracking-heading text-neutral-900">
          Overview
        </h1>
        <span className="text-base-xs text-base-muted-foreground">{period}</span>
      </header>

      {alert}

      <StatTiles stats={stats} />
      <MessagesChart buckets={buckets} />

      <PolicyTable
        policies={policies}
        onManage={onManagePolicies}
      />
      <SavingsTable savings={savings} onManage={onManageSavings} />
    </div>
  );
}

function StatTiles({ stats }: { stats: OverviewStats }) {
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
        value={`${stats.tokensSavedPercent}%`}
        delta={stats.tokensSavedAmount}
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
function MessagesChart({ buckets }: { buckets: MessagesBucket[] }) {
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

function PolicyTable({
  policies,
  onManage,
}: {
  policies: Policy[];
  onManage: () => void;
}) {
  return (
    <Card className="p-4">
      <h2 className="text-sm font-medium leading-5 text-neutral-900">Policies</h2>

      <table className="mt-4 w-full">
        <thead>
          <tr className="text-base-xs text-base-muted-foreground">
            <th scope="col" className="pb-2 text-left font-normal">
              Policy type
            </th>
            <th scope="col" className="pb-2 text-right font-normal">
              Action
            </th>
            <th scope="col" className="w-24 pb-2 text-right font-normal">
              Status
            </th>
          </tr>
        </thead>
        <tbody>
          {policies.map((policy) => (
            <tr key={policy.id} className="border-t border-base-border">
              <td className="py-3">
                <span className="flex items-center gap-3 text-sm leading-5 text-neutral-900">
                  <Icon name={policy.icon} size={16} className="text-neutral-500" />
                  {policy.name}
                </span>
              </td>
              <td className="py-3 text-right">
                <span
                  className={`inline-block rounded-base px-1.5 py-0.5 font-mono text-base-xs font-medium uppercase leading-4 tracking-label ${ACTION_STYLES[policy.action]}`}
                >
                  {policy.action}
                </span>
              </td>
              <td className="py-3 text-right">
                <StatusPill on={policy.enabled} />
              </td>
            </tr>
          ))}
        </tbody>
      </table>

      <ManageLink label="Manage policies" onClick={onManage} />
    </Card>
  );
}

function SavingsTable({
  savings,
  onManage,
}: {
  savings: Saving[];
  onManage: () => void;
}) {
  return (
    <Card className="p-4">
      <h2 className="text-sm font-medium leading-5 text-neutral-900">Token savings</h2>

      <table className="mt-4 w-full">
        <thead>
          <tr className="text-base-xs text-base-muted-foreground">
            <th scope="col" className="pb-2 text-left font-normal">
              Savings type
            </th>
            <th scope="col" className="w-24 pb-2 text-right font-normal">
              Status
            </th>
          </tr>
        </thead>
        <tbody>
          {savings.map((saving) => (
            <tr key={saving.id} className="border-t border-base-border">
              <td className="py-3">
                <span className="flex items-center gap-3 text-sm leading-5 text-neutral-900">
                  <Icon name={saving.icon} size={16} className="text-neutral-500" />
                  {saving.name}
                </span>
              </td>
              <td className="py-3 text-right">
                <StatusPill on={saving.enabled} />
              </td>
            </tr>
          ))}
        </tbody>
      </table>

      <ManageLink label="Manage savings" onClick={onManage} />
    </Card>
  );
}

function StatusPill({ on }: { on: boolean }) {
  return (
    <span
      className={`inline-flex items-center gap-1 rounded-base px-1.5 py-0.5 text-base-xs font-medium leading-4 ${
        on ? "bg-green-100 text-green-900" : "bg-neutral-100 text-neutral-600"
      }`}
    >
      {on && <Icon name="check" size={12} />}
      {on ? "On" : "Off"}
    </span>
  );
}

/** Right-aligned footer link. Both destinations open the web dashboard. */
function ManageLink({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <div className="mt-4 flex justify-end">
      <button
        type="button"
        onClick={onClick}
        className="flex items-center gap-1.5 rounded-base px-1.5 py-1 text-base-xs font-medium text-base-primary transition-colors hover:bg-gray-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
      >
        {label}
        <Icon name="squareArrowOutUpRight" size={12} />
      </button>
    </div>
  );
}
