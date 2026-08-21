import type { ReactNode } from "react";
import { Card, EmptyNote, Skeleton } from "./base";
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
  unavailable,
}: {
  stats: UsageStats;
  buckets: MessagesBucket[];
  policies: Policy[];
  savings: Saving[];
  onManagePolicies: () => void;
  onManageSavings: () => void;
  /** First load has not landed. Passed down so every card draws a placeholder
   *  rather than an answer it does not have yet; see `Skeleton`. */
  pending?: boolean;
  /** Which sections were not read at all, so they say nothing about the user's
   *  traffic instead of reporting it as empty. `ActivityView.missing` is where
   *  this comes from and why the two are separate facts. */
  unavailable?: { chart?: boolean; policies?: boolean; savings?: boolean };
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
        <h1 className="text-xl font-medium tracking-heading text-neutral-900">
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
      <MessagesChart buckets={buckets} pending={pending} unavailable={unavailable?.chart} />

      <PolicyTable
        policies={policies}
        pending={pending}
        unavailable={unavailable?.policies}
        onManage={onManagePolicies}
      />
      <SavingsTable
        savings={savings}
        pending={pending}
        unavailable={unavailable?.savings}
        onManage={onManageSavings}
      />
    </div>
  );
}

function PolicyTable({
  policies,
  pending,
  unavailable,
  onManage,
}: {
  policies: Policy[];
  pending?: boolean;
  unavailable?: boolean;
  onManage: () => void;
}) {
  return (
    <Card className="p-4" busy={pending}>
      <h2 className="text-sm font-medium leading-5 text-neutral-900">Policies</h2>

      {pending ? (
        <PendingRows columns={3} />
      ) : policies.length === 0 ? (
        // Two different sentences, because they are two different facts: an org
        // that has configured no guardrails, and a list the gateway would not
        // give us. The pane's gap notice supplies the cause and the action for
        // the second; what this must not do is report it as the first.
        <EmptyNote icon="shieldCheck">
          {unavailable ? "Policies couldn't be read" : "No policies configured"}
        </EmptyNote>
      ) : (
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
                    className={`inline-block rounded-xs px-2 py-1 font-mono text-base-xs font-medium uppercase leading-4 tracking-label ${ACTION_STYLES[policy.action]}`}
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
      )}

      <ManageLink label="Manage policies" onClick={onManage} />
    </Card>
  );
}

function SavingsTable({
  savings,
  pending,
  unavailable,
  onManage,
}: {
  savings: Saving[];
  pending?: boolean;
  unavailable?: boolean;
  onManage: () => void;
}) {
  return (
    // `scroll-mt-6` so the smooth scroll from the Tokens saved counter leaves
    // the same gutter the pane's padding gives every other card, rather than
    // butting the heading against the top edge.
    <Card id={SAVINGS_SECTION_ID} className="scroll-mt-6 p-4" busy={pending}>
      <h2 className="text-sm font-medium leading-5 text-neutral-900">Token savings</h2>

      {pending ? (
        <PendingRows columns={2} />
      ) : savings.length === 0 ? (
        // Same split as the policies card, for the same reason.
        <EmptyNote icon="layers">
          {unavailable ? "Token savings couldn't be read" : "No savings configured"}
        </EmptyNote>
      ) : (
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
      )}

      <ManageLink label="Manage savings" onClick={onManage} />
    </Card>
  );
}

/**
 * A card's rows before they exist: fixed count, fixed widths, one line each.
 *
 * Three rows because both tables draw three-ish, and a placeholder that guesses
 * the count high leaves the card collapsing when the real answer lands. The
 * status column keeps its own narrow shape so the row reads as a row.
 */
function PendingRows({ columns }: { columns: 2 | 3 }) {
  return (
    <div className="mt-4 flex flex-col gap-3">
      {[0, 1, 2].map((i) => (
        <div key={i} className="flex items-center justify-between gap-3 border-t border-base-border pt-3">
          <Skeleton className="h-4 w-40" />
          {columns === 3 && <Skeleton className="h-4 w-14" />}
          <Skeleton className="h-4 w-10" />
        </div>
      ))}
    </div>
  );
}

function StatusPill({ on }: { on: boolean }) {
  return (
    <span
      className={`inline-flex items-center gap-2 rounded-xs px-2 py-1 font-mono text-base-xs font-medium uppercase leading-4 tracking-label ${
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
        className="flex items-center gap-1.5 rounded-sm px-1.5 py-1 text-base-xs font-medium text-base-primary transition-colors hover:bg-gray-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
      >
        {label}
        <Icon name="squareArrowOutUpRight" size={12} />
      </button>
    </div>
  );
}
