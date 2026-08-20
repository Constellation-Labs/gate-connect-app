import type { ReactNode } from "react";
import { Card } from "./base";
import { Icon } from "./Icon";
import type { IconName } from "./Icon";
import { MessagesChart, StatTiles } from "./metrics";
import type { MessagesBucket, UsageStats } from "./metrics";

/**
 * The Overview pane (Figma `Flows / Overview`): a 24-hour summary of what Gate
 * actually did with the user's traffic. Sits right of the sidebar in the
 * 1024x720 window, on a gray/100 ground, and scrolls internally.
 *
 * Presentational. The 24-hour backend is still being built, so every number
 * arrives as a prop and nothing here talks to `lib/api`.
 */

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

/**
 * Action pills. Every fill was read off the pixels of `Overview-partly-routed-1`
 * on 2026-08-20: `red/200`, `amber/200`, `violet/200`, at a 2px radius with 8/4
 * padding and an 8px gap. Text sits at the matching 900.
 *
 * **Violet, not purple.** The drawn REDACT fill is `#ddd6fe`, which is violet/200;
 * purple/200 is `#e9d5ff`. The same mistake was in `chart.redacted`. Nothing else
 * in this palette is violet, so it is easy to "correct" back by eye - don't.
 */
const ACTION_STYLES: Record<PolicyAction, string> = {
  block: "bg-red-200 text-red-900",
  flag: "bg-amber-200 text-amber-900",
  redact: "bg-violet-200 text-violet-900",
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
  stats: UsageStats;
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
                  className={`inline-block rounded-sm px-2 py-1 font-mono text-base-xs font-medium uppercase leading-4 tracking-label ${ACTION_STYLES[policy.action]}`}
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
      className={`inline-flex items-center gap-2 rounded-sm px-2 py-1 font-mono text-base-xs font-medium uppercase leading-4 tracking-label ${
        on ? "bg-green-200 text-green-900" : "bg-neutral-100 text-neutral-600"
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
