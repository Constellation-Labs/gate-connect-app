import type { ReactNode } from "react";
import { Card, CardHeader, EmptyNote, GUARDRAIL_INK, Skeleton } from "./base";
import { Icon } from "./Icon";
import type { IconName } from "./Icon";
import { MessagesChart, StatTiles } from "./metrics";
import type { MessagesBucket, UsageStats } from "./metrics";
import { SecurityEvents } from "./SecurityEvents";
import type { SecurityEventsProps } from "./SecurityEvents";

/**
 * The Overview pane (Figma `Overview/none-routed` 1390:13599 and its
 * siblings): a 24-hour summary of what Gate actually did with the user's
 * traffic. Sits right of the sidebar in the 1280x800 window as a 976px pane on
 * the `base.background` ground, and scrolls internally.
 *
 * Presentational. The 24-hour backend is still being built, so every number
 * arrives as a prop and nothing here talks to `lib/api`.
 *
 * **The live security feed is the last section of it** (AG-853), below Token
 * savings, where it used to be a pane of its own behind a third rail entry. The
 * four summaries above it and the feed answer the same question at two
 * resolutions - what Gate did with this traffic, in aggregate and event by
 * event - and the pane scrolls, so the feed costs the summaries nothing.
 *
 * It is a required prop rather than a `ReactNode` slot like `alert` below.
 * That one is genuinely optional chrome; this is the section the pane is now
 * the only home for, and a caller that could omit it could lose the feed
 * entirely with nothing failing to say so.
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
 * Action pills. `red/100`, `amber/100`, `violet/100` fills with text at the
 * matching 900, at a 2px radius with 8/4 padding and an 8px gap
 * (`1402:18182`, `1402:18209`, `1402:18236`). The fills were 200 until the
 * 2026-09-29 redraw moved them one step lighter for contrast - design's own
 * note on the change - so a 200 here is the old value, not a sample error.
 *
 * **Violet, not purple.** The drawn REDACT fill is `#ddd6fe`, which is violet/200;
 * purple/200 is `#e9d5ff`. Nothing else in this palette is violet, so it is easy
 * to "correct" back by eye - don't.
 *
 * This note used to add "the same mistake was in `chart.redacted`", which
 * conflated two different elements. The chart series is its own question: the
 * legend swatch (`706:10096`) draws violet/500 and the newer tooltip component
 * (`744:37728`) draws purple/500, so the file disagrees with itself there and
 * `chart.redacted` stays as it is pending design. This pill is not affected.
 */
const ACTION_STYLES: Record<PolicyAction, string> = {
  block: "bg-red-100 text-red-900",
  flag: "bg-amber-100 text-amber-900",
  redact: "bg-violet-100 text-violet-900",
  // Not in the Figma, which draws only the three enforcing actions. Neutral
  // rather than a fourth colour: `allow` is the one that does nothing, and
  // giving it a hue would read as a severity it does not have.
  allow: "bg-neutral-100 text-base-foreground",
};

export function Overview({
  stats,
  buckets,
  policies,
  savings,
  onManagePolicies,
  onManageSavings,
  security,
  alert,
  period = "Last 24 hours",
  updatedAt = null,
  pending,
  unavailable,
}: {
  stats: UsageStats;
  buckets: MessagesBucket[];
  policies: Policy[];
  savings: Saving[];
  onManagePolicies: () => void;
  onManageSavings: () => void;
  /** The live security-event feed, passed straight through to the section that
   *  draws it. Its own read, with its own loading and failure states: `pending`
   *  and `unavailable` below describe the 24-hour activity read and say nothing
   *  about the feed. */
  security: SecurityEventsProps;
  /** First load has not landed. Passed down so every card draws a placeholder
   *  rather than an answer it does not have yet; see `Skeleton`. */
  pending?: boolean;
  /** Which sections were not read at all, so they say nothing about the user's
   *  traffic instead of reporting it as empty. `ActivityView.missing` is where
   *  this comes from and why the two are separate facts. */
  unavailable?: { chart?: boolean; policies?: boolean; savings?: boolean };
  /** Slot for an `AlertBanner`, which the design places above the stat tiles.
   *  Whole-machine routing causes only; a tool's own card is on its pane. */
  alert?: ReactNode;
  /** The window the numbers cover. */
  period?: string;
  /** When the reading was taken, as `ActivityView.takenAt` formats it. Drawn
   *  after the window as "Updated 14:03" (`1390:13613`); nothing is drawn
   *  while there is no reading yet. */
  updatedAt?: string | null;
}) {
  return (
    // `relative`, because this is the scroll container. An `sr-only` node is
    // `position: absolute`, and with no positioned ancestor nearer than the
    // shell root it sits at its static position *in the root's coordinate
    // space* - so the security table's hidden "Action" header, drawn near the
    // bottom of this pane's unscrolled content, hung 241px below the window as
    // the root's own scrollable overflow. The root is `overflow-hidden`, which
    // the user cannot scroll back but `scrollIntoView` will scroll forward: the
    // Tokens saved jump pinned this pane at its maximum and then shifted the
    // whole shell up by the remainder, leaving white below the sidebar with no
    // way back (AG-883's "adds space"). Positioning the scroller makes it the
    // containing block, so hidden text scrolls with the content it describes.
    <div className="relative flex flex-1 flex-col gap-4 overflow-auto bg-base-background p-6">
      {/* `mb-2` on top of the pane's `gap-4` for a 24px drop to whatever comes
        * first below, alert or stat tiles. The frame draws the header at y0
        * 24px tall and opens its content at y48 (`864:3475` -> `864:3478`),
        * where the cards below it are 16px apart - so this one gap is not the
        * pane's rhythm and cannot come from `gap-4`. */}
      <header className="mb-2 flex items-baseline justify-between">
        {/* `heading/24` (`1390:13609`, 24/28 at -1%) since the 2026-09-29
          * redraw; it was `heading/20`. */}
        <h1 className="text-2xl font-medium leading-7 tracking-heading-24 text-base-foreground">
          Overview
        </h1>
        {/* Two runs, both muted: the window in `heading/14` Medium and the
          * reading's time in `copy/14` after a separator (`1390:13611`,
          * `1390:13613`). One `copy/14` string carried both until the redraw
          * split them. */}
        <p className="text-sm leading-5 text-base-muted-foreground">
          <span className="font-medium tracking-heading-14">{period}</span>
          {updatedAt && (
            <>
              <span> · </span>
              <span>Updated {updatedAt}</span>
            </>
          )}
        </p>
      </header>

      {alert}

      <StatTiles
        stats={stats}
        pending={pending}
        // AG-572: selecting the counter moves to the Token savings section.
        // `scrollIntoView` on the section rather than a hash link, which would
        // put a fragment in the webview's URL for a window that has no address.
        //
        // **Offered only when the section has something in it (AG-883).** Token
        // savings is the second-to-last card, so `block: "start"` cannot be
        // honoured: the pane pins at its maximum scroll instead, and with the
        // table and the feed below it both empty, the click reads as the page
        // jumping to a screen of nothing. Measured in Chromium at the drawn
        // 800px window: `scrollHeight` does not change, `scrollTop` goes from 0
        // to 504 of a possible 504.
        //
        // The two tickets want opposite things - AG-572 says the counter
        // navigates, AG-883 says clicking it must not move the page - and this
        // is the reading that keeps both: it navigates when there is somewhere
        // to land, and is an ordinary tile when there is not. `Stat` already
        // draws it as a plain `div` with no hover when no handler is passed, so
        // nothing offers a jump that would go nowhere.
        onSelectTokensSaved={
          pending || unavailable?.savings || savings.length === 0
            ? undefined
            : () =>
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
      {/* Last, and after Token savings by name: the summaries above are the
        * period's totals and this is the period's detail, so it reads in the
        * order a user asks the questions in. */}
      <SecurityEvents {...security} />
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
    // No padding on the card: the header's rule spans it edge to edge, and so
    // does every row divider since the 2026-09-29 redraw (`table/recent-activity`
    // 1402:17916 draws each `table-row` full width with its own bottom border).
    // The 16px lives on the cells. The older `card/policies` (116:26707) inset
    // the dividers at 688 inside 720; that is the gutter this used to draw.
    <Card busy={pending}>
      <CardHeader title="Policies" action={{ label: "Manage policies", onClick: onManage }} />

      {pending ? (
        <PendingRows columns={3} />
      ) : policies.length === 0 ? (
        // Two different sentences, because they are two different facts: an org
        // that has configured no guardrails, and a list the gateway would not
        // give us. The pane's gap notice supplies the cause and the action for
        // the second; what this must not do is report it as the first.
        <EmptyNote icon="shieldCheck" className="px-4 pb-4">
          {unavailable ? "Policies couldn't be read" : "No policies configured"}
        </EmptyNote>
      ) : (
        <table className="w-full">
          <thead>
            {/* `label/14` Medium in `base/muted-foreground` on a 16/12 row
              * (`1402:18259`). Was `label/12` before the redraw. */}
            <tr className="text-sm font-medium leading-5 text-base-muted-foreground">
              <th scope="col" className="py-3 pl-4 text-left">
                Policy type
              </th>
              <th scope="col" className="py-3 text-right">
                Action
              </th>
              {/* 96 (`1402:18265`), holding the 50px badge flush right with the
                * action badge 60px before it - the same 60 on all three rows
                * (`1402:18181`), which is what the 46 of slack is for. */}
              <th scope="col" className="w-24 py-3 pr-4 text-right">
                Status
              </th>
            </tr>
          </thead>
          <tbody>
            {policies.map((policy) => (
              <tr key={policy.id} className="h-14 border-t border-base-border">
                <td className="pl-4">
                  {/* `label/16` (`1402:18180`: Geist Medium 16/24 at -2%, which
                    * `text-base` carries) beside a 24px glyph, 12px apart. */}
                  <span className="flex items-center gap-3 text-base font-medium leading-6 text-base-foreground">
                    <Icon
                      name={policy.icon}
                      size={24}
                      // In colour, one per guardrail (`1402:18151` and its two
                      // siblings); the savings rows below stay muted.
                      className={GUARDRAIL_INK[policy.icon] ?? "text-base-muted-foreground"}
                    />
                    {policy.name}
                  </span>
                </td>
                <td className="text-right">
                  {policy.action ? (
                    <span
                      className={`inline-flex items-center rounded-xs px-2 py-1 font-mono text-base-xs font-medium uppercase leading-4 tracking-label ${ACTION_STYLES[policy.action]}`}
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
                <td className="pr-4 text-right">
                  <StatusPill on={policy.enabled} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
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
    // butting the heading against the top edge. No padding on the card, for
    // the reason `PolicyTable` gives.
    <Card id={SAVINGS_SECTION_ID} className="scroll-mt-6" busy={pending}>
      <CardHeader title="Token savings" action={{ label: "Manage savings", onClick: onManage }} />

      {pending ? (
        <PendingRows columns={2} />
      ) : savings.length === 0 ? (
        // Same split as the policies card, for the same reason.
        <EmptyNote icon="layers" className="px-4 pb-4">
          {unavailable ? "Token savings couldn't be read" : "No savings configured"}
        </EmptyNote>
      ) : (
        <table className="w-full">
          <thead>
            {/* `label/14`, as on the policies table above. The frame spells the
              * header "Savings tyoe" (`1402:18274`); that is a typo in the file,
              * raised, not copy to ship. */}
            <tr className="text-sm font-medium leading-5 text-base-muted-foreground">
              <th scope="col" className="py-3 pl-4 text-left">
                Savings type
              </th>
              {/* 92 (`1402:18276`). */}
              <th scope="col" className="w-[92px] py-3 pr-4 text-right">
                Status
              </th>
            </tr>
          </thead>
          <tbody>
            {savings.map((saving) => (
              <tr key={saving.id} className="h-14 border-t border-base-border">
                <td className="pl-4">
                  {/* `label/16` beside a 24px glyph, as on the policies table -
                    * but these glyphs stay `base/muted-foreground`
                    * (`1402:18323`, `1402:18333`): only the guardrail rows took
                    * a colour in the redraw. */}
                  <span className="flex items-center gap-3 text-base font-medium leading-6 text-base-foreground">
                    <Icon name={saving.icon} size={24} className="text-base-muted-foreground" />
                    {saving.name}
                  </span>
                </td>
                <td className="pr-4 text-right">
                  <StatusPill on={saving.enabled} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </Card>
  );
}

/**
 * A card's rows before they exist: fixed count, fixed widths, one line each.
 *
 * Three rows because both tables draw three-ish, and a placeholder that guesses
 * the count high leaves the card collapsing when the real answer lands. The
 * status column keeps its own narrow shape so the row reads as a row. Rows are
 * the drawn 56px with full-width dividers, so the card does not change height
 * or line pattern when the real rows land.
 */
function PendingRows({ columns }: { columns: 2 | 3 }) {
  return (
    <div className="flex flex-col">
      {[0, 1, 2].map((i) => (
        <div
          key={i}
          className={`flex h-14 items-center justify-between gap-3 px-4 ${i === 0 ? "" : "border-t border-base-border"}`}
        >
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
      className={`inline-flex items-center gap-1 rounded-xs px-2 py-1 font-mono text-base-xs font-medium uppercase leading-4 tracking-label ${
        on ? "bg-green-100 text-green-900" : "bg-neutral-100 text-base-foreground"
      }`}
    >
      {/* green/100 under green/900 since the 2026-09-29 redraw (`1402:18184`);
        * the fill was 200. The glyph stays green/800, one step lighter than the
        * label: sampled #166534 off the render of `1390:13599`.
        *
        * 14px, not 12: both current policies cards draw the glyph at 14
        * (`884:9612` in the 720px frame, `1402:18185` in the 1280 one), and 14
        * is also what makes the badge measure the drawn 50px:
        * 8 + 14 + 4 + 15.84 + 8. */}
      {on && <Icon name="circleCheck" size={14} className="text-green-800" />}
      {on ? "On" : "Off"}
    </span>
  );
}
