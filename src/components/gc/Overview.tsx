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

/**
 * What the policy does when a criterion trips, as the gateway stores it.
 *
 * `allow` is one of them: the criterion runs and records what it finds, it just
 * does not act. It reads oddly next to the other three and is deliberately not
 * folded into "off", because a guardrail that is watching is not the same as one
 * that is not there.
 */
export type PolicyAction = "block" | "flag" | "redact" | "allow";

/** Anchor for the Tokens saved counter's jump target (AG-572). */
const SAVINGS_SECTION_ID = "token-savings";

export interface Policy {
  id: string;
  name: string;
  icon: IconName;
  /** Null when the policy states no single action for this guardrail, which is
   *  the common case: the gateway then acts per entity or per confidence tier,
   *  and no one verb describes it. Rendered as no pill rather than a guess. */
  action: PolicyAction | null;
  enabled: boolean;
}

export interface Saving {
  id: string;
  name: string;
  icon: IconName;
  enabled: boolean;
}

/**
 * Action pills. Text sits at the palette's 900 level; the 100 backgrounds are
 * inferred from that pairing rather than sampled from Figma.
 */
const ACTION_STYLES: Record<PolicyAction, string> = {
  block: "bg-red-100 text-red-900",
  flag: "bg-amber-100 text-amber-900",
  redact: "bg-purple-100 text-purple-900",
  // Not in the Figma, which draws only the three enforcing actions. Neutral
  // rather than a fourth colour: `allow` is the one that does nothing, and
  // giving it a hue would read as a severity it does not have.
  allow: "bg-neutral-100 text-neutral-900",
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
  scope,
  pending,
}: {
  stats: UsageStats;
  buckets: MessagesBucket[];
  policies: Policy[];
  savings: Saving[];
  onManagePolicies: () => void;
  onManageSavings: () => void;
  /** First load has not landed. Passed to the tiles so they show dashes rather
   *  than zeros; see `StatTiles`. */
  pending?: boolean;
  /** Slot for an `AlertBanner`, which the design places above the stat tiles. */
  alert?: ReactNode;
  period?: string;
  /** Slot for the installation picker, beside the period label: both say what
   *  the numbers below cover, so they belong on the same line. */
  scope?: ReactNode;
}) {
  return (
    <div className="flex flex-1 flex-col gap-4 overflow-auto bg-gray-100 p-6">
      <header className="flex items-baseline justify-between">
        <h1 className="text-xl font-medium leading-6 tracking-heading text-neutral-900">
          Overview
        </h1>
        <div className="flex items-center gap-3">
          {scope}
          <span className="text-base-xs text-base-muted-foreground">{period}</span>
        </div>
      </header>

      {alert}

      <StatTiles
        stats={stats}
        pending={pending}
        // AG-572: selecting the counter moves to the Token savings section.
        // `scrollIntoView` on the section rather than a hash link, which would
        // put a fragment in the webview's URL for a window that has no address.
        onSelectTokensSaved={() =>
          document.getElementById(SAVINGS_SECTION_ID)?.scrollIntoView({
            behavior: "smooth",
            block: "start",
          })
        }
      />
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
                {policy.action ? (
                  <span
                    className={`inline-block rounded-base px-1.5 py-0.5 font-mono text-base-xs font-medium uppercase leading-4 tracking-label ${ACTION_STYLES[policy.action]}`}
                  >
                    {policy.action}
                  </span>
                ) : (
                  // The policy names no single action, so neither does this. The
                  // Status column still says whether the guardrail is running,
                  // which is the part that would be a lie to leave blank.
                  <span className="text-base-xs text-base-muted-foreground" title="This policy sets no single action">
                    Not set
                  </span>
                )}
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
    // `scroll-mt-6` so the smooth scroll from the Tokens saved counter leaves
    // the same gutter the pane's padding gives every other card, rather than
    // butting the heading against the top edge.
    <Card id={SAVINGS_SECTION_ID} className="scroll-mt-6 p-4">
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
