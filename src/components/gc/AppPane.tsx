import type { ReactNode } from "react";
import { BaseSwitch, Card, EmptyNote, Skeleton } from "./base";
import { Icon } from "./Icon";
import { MessagesChart, StatTiles } from "./metrics";
import type { MessagesBucket, UsageStats } from "./metrics";

/**
 * The per-app pane (Figma `Flows / App`), reached by selecting an app in the
 * sidebar. Same usage summary as the Overview but scoped to one app, plus the
 * model selection this app is running under and its recent traffic.
 *
 * Presentational, and the confirmation dialog that guards switching to a Gate
 * model is Phase 8 - this pane only reports the choice.
 */

export type ModelChoice = "app" | "gate";

// The feed's row type lives in `lib/` with the adapter that produces it; see
// `toolEventRow.ts`. Re-exported here so existing importers of this module keep
// working and the pane's own props stay readable.
export type { ActivityEntry, ActivitySecurity, ActivityStatus } from "../../lib/toolEventRow";
import type { ActivityEntry, ActivitySecurity } from "../../lib/toolEventRow";

export interface GateModel {
  /** Model vendor, e.g. "Anthropic". */
  vendor: string;
  /** Fully qualified model id, rendered mono, e.g. "gate/opus 5". */
  id: string;
  /** Vendor mark, 16px. */
  logo?: ReactNode;
}

/**
 * One pill per row, in the design's own set (`table/recent-activity`, 272:3150,
 * sampled from the properties panel 2026-08-21): ERROR / REDACTED / FLAGGED /
 * ALLOW / BLOCKED. These are the 100 stop with 700 text - deliberately quieter
 * than the Overview's 200/900 action pills - with ALLOW as a grey non-verdict
 * (`gray/100` over `base/muted-foreground`) and REDACTED's text at the 800.
 * ERROR and BLOCKED sample identically.
 */
const PILL_STYLES: Record<ActivitySecurity | "error", string> = {
  error: "bg-red-100 text-red-700",
  allow: "bg-gray-100 text-base-muted-foreground",
  flagged: "bg-amber-100 text-amber-700",
  redacted: "bg-violet-100 text-violet-800",
  blocked: "bg-red-100 text-red-700",
};

/**
 * Which pill a row wears. The design merged the old Status column into
 * Security, so one cell must choose: the guardrail's verdict outranks the
 * transport outcome, because a blocked request usually *also* errors from the
 * client's side, and a column of ERRORs over what Gate actually did would bury
 * the pane's story. ERROR is what remains when the gateway recorded no action.
 */
function rowVerdict(entry: ActivityEntry): ActivitySecurity | "error" | null {
  if (entry.security) return entry.security;
  return entry.status === "error" ? "error" : null;
}

export function AppPane({
  name,
  isProtected,
  since,
  logo,
  busy,
  onToggleProtected,
  stats,
  buckets,
  modelChoice,
  onChooseModel,
  gateModel,
  onChangeModel,
  credits,
  onAddCredits,
  activity,
  pending,
  eventsPending,
  onLoadMore,
  onViewEntry,
  unavailable,
  alert,
}: {
  name: string;
  isProtected: boolean;
  /** Relative age of the current status ("2m ago"). */
  since?: string;
  /** 16px brand mark for the header tile. */
  logo?: ReactNode;
  /** A routing write is in flight, so the switch refuses a second click. */
  busy?: boolean;
  onToggleProtected: () => void;
  stats: UsageStats;
  buckets: MessagesBucket[];
  /**
   * The model card, or nothing.
   *
   * Withheld for a multi-provider tool. OpenCode, OpenClaw and Hermes route
   * whichever of their configured providers Gate covers - `lib/groups.ts` calls
   * them "tools that talk to several providers, not one model family" - so
   * "what does this app use on Gate model" has no single answer for them. `main`
   * never poses the question at all: it has no model UI, and these tools appear
   * only as routing targets.
   *
   * The Figma's answer is a multi-select picker
   * (`App / Select multiple models (Opencode)`), which needs a model list no
   * gateway endpoint reports yet and a selection shape `ModelChoice` cannot
   * hold. Until that exists, matching `main` and asking nothing beats asking a
   * question whose answer the app cannot record.
   *
   * Per the house rule the Settings pane states: an omitted handler omits its
   * control. No `onChooseModel`, no card.
   */
  modelChoice?: ModelChoice;
  onChooseModel?: (choice: ModelChoice) => void;
  gateModel?: GateModel;
  onChangeModel?: () => void;
  /** Pre-formatted balance, e.g. "$10.25 available". */
  credits?: string;
  onAddCredits?: () => void;
  activity: ActivityEntry[];
  /** The first reading for this tool has not landed. Draws skeletons rather than
   *  answers, per AG-576. */
  pending?: boolean;
  /** The feed's own first page has not landed. Separate from `pending` because
   *  the counters and the feed are two reads: one can be drawn while the other
   *  is still coming. */
  eventsPending?: boolean;
  /** Fetch the next page of the feed. Absent when there is no next page, which
   *  is what removes the control rather than leaving a dead one on screen. */
  onLoadMore?: () => void;
  /** Open one request in the web dashboard. The button is drawn regardless -
   *  the design's call - and this is where its destination lands once
   *  `dashboard-web` can filter by request. */
  onViewEntry?: (entry: ActivityEntry) => void;
  /** Which sections have no reading behind them.
   *
   *  Not "this app sent nothing": a tool can be routing correctly and still be
   *  unattributed, because the slug is guessed from a User-Agent and an agent the
   *  matcher cannot place is left unlabelled. So an unread section says so, and
   *  the pane's notice names the cause. Reporting it as an empty state would tell
   *  a user who has been working all morning that their tool is idle. */
  unavailable?: { chart?: boolean; events?: boolean };
  /** Slot for an `AlertBanner` when this app has drifted. */
  alert?: ReactNode;
}) {
  return (
    <div className="flex flex-1 flex-col gap-4 overflow-auto bg-base-background p-6">
      <header className="flex items-center gap-3">
        <span
          aria-hidden
          className="flex size-10 shrink-0 items-center justify-center rounded-sm border border-white/[0.24] bg-black text-sm font-medium text-white"
          style={{
            backgroundImage:
              "linear-gradient(180deg, rgba(255,255,255,0.32) 0%, rgba(0,0,0,0.32) 100%)",
          }}
        >
          {logo ?? name.charAt(0)}
        </span>
        <div className="min-w-0 flex-1">
          <h1 className="truncate text-xl font-medium tracking-heading text-base-foreground">
            {name}
          </h1>
          <p className="text-base-xs font-medium leading-4">
            <span className={isProtected ? "text-green-600" : "text-amber-600"}>
              {isProtected ? "Protected" : "Not protected"}
            </span>
            {since && <span className="text-neutral-500"> - {since}</span>}
          </p>
        </div>
        <span className="flex shrink-0 items-center gap-2">
          <span className="text-base-xs font-medium text-neutral-600">
            {isProtected ? "On" : "Off"}
          </span>
          {/* "Route Claude Code", not "Claude Code": the sidebar row for the
            * same app is on screen with its own switch, and two switches with
            * one name is a screen reader reading the same control twice. The
            * families pane names its switches the same way. */}
          <BaseSwitch
            on={isProtected}
            label={`Route ${name}`}
            busy={busy}
            onClick={onToggleProtected}
          />
        </span>
      </header>

      {alert}

      <StatTiles stats={stats} pending={pending} />
      <MessagesChart buckets={buckets} pending={pending} unavailable={unavailable?.chart} />

      {onChooseModel && (
        <ModelSelection
          appName={name}
          choice={modelChoice ?? "app"}
          onChoose={onChooseModel}
          gateModel={gateModel ?? { vendor: "-", id: "-" }}
          onChangeModel={onChangeModel ?? (() => {})}
          credits={credits ?? "-"}
          onAddCredits={onAddCredits ?? (() => {})}
        />
      )}

      <RecentActivity
        activity={activity}
        pending={eventsPending}
        unavailable={unavailable?.events}
        onLoadMore={onLoadMore}
        onViewEntry={onViewEntry}
      />
    </div>
  );
}

function ModelSelection({
  appName,
  choice,
  onChoose,
  gateModel,
  onChangeModel,
  credits,
  onAddCredits,
}: {
  appName: string;
  choice: ModelChoice;
  onChoose: (choice: ModelChoice) => void;
  gateModel: GateModel;
  onChangeModel: () => void;
  credits: string;
  onAddCredits: () => void;
}) {
  return (
    <Card className="p-4">
      <h2 className="text-sm font-medium leading-5 text-base-foreground">Model selection</h2>
      <p className="mt-1 text-sm leading-5 text-neutral-600">
        Choose whether {appName} or Gate selects the AI model for requests
      </p>

      <div role="radiogroup" aria-label="Model selection" className="mt-4 grid grid-cols-2 gap-4">
        <ModelOption
          selected={choice === "app"}
          onSelect={() => onChoose("app")}
          icon={<Icon name="cube" size={16} />}
          title="App default"
          description="Use the model configured in your app"
        />
        <ModelOption
          selected={choice === "gate"}
          onSelect={() => onChoose("gate")}
          icon={<Icon name="layers" size={16} />}
          title="Gate model"
          description="Use a model selected in Gate AI"
        />
      </div>

      {/* The design separates the choice above from the current-model rows
       * below with a full-width rule. */}
      <hr className="mt-4 border-t border-base-border" />

      {/* Only meaningful once Gate is picking the model, but the design keeps it
       * visible either way so the user can see what they would switch to. */}
      <p className="mt-4 text-base-xs text-base-muted-foreground">Current Gate model</p>

      <div className="mt-2 flex flex-col gap-2">
        <InfoRow
          icon={gateModel.logo ?? <Icon name="cube" size={16} />}
          action={{ label: "Change model", onClick: onChangeModel }}
        >
          <p className="text-base-2xs leading-4 text-base-muted-foreground">
            {gateModel.vendor}
          </p>
          <p className="font-mono text-sm leading-5 text-base-foreground">{gateModel.id}</p>
        </InfoRow>

        <InfoRow
          icon={<Icon name="creditCard" size={16} />}
          action={{ label: "Add credits", onClick: onAddCredits, external: true }}
        >
          <p className="text-sm leading-5 text-base-foreground">
            <span className="text-neutral-600">Gate credits: </span>
            {credits}
          </p>
        </InfoRow>
      </div>
    </Card>
  );
}

function ModelOption({
  selected,
  onSelect,
  icon,
  title,
  description,
}: {
  selected: boolean;
  onSelect: () => void;
  icon: ReactNode;
  title: string;
  description: string;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      onClick={onSelect}
      className={`flex items-center gap-3 rounded-md border p-3 text-left transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
        selected
          ? "border-base-primary bg-base-card"
          : "border-base-border bg-base-card hover:bg-gray-50"
      }`}
    >
      <span
        aria-hidden
        className="flex size-8 shrink-0 items-center justify-center rounded-sm border border-base-border text-neutral-700"
      >
        {icon}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium leading-5 text-base-foreground">
          {title}
        </span>
        <span className="block truncate text-base-xs leading-4 text-neutral-600">
          {description}
        </span>
      </span>
      {selected && (
        <Icon name="circleCheck" size={16} className="shrink-0 text-base-primary" />
      )}
    </button>
  );
}

function InfoRow({
  icon,
  children,
  action,
}: {
  icon: ReactNode;
  children: ReactNode;
  action: { label: string; onClick: () => void; external?: boolean };
}) {
  return (
    <div className="flex items-center gap-3 rounded-md border border-base-border p-3">
      <span
        aria-hidden
        className="flex size-8 shrink-0 items-center justify-center rounded-sm border border-base-border text-neutral-700"
      >
        {icon}
      </span>
      <div className="min-w-0 flex-1">{children}</div>
      <button
        type="button"
        onClick={action.onClick}
        className="flex shrink-0 items-center h-8 gap-1.5 rounded-control border border-base-border bg-base-card px-3 text-base-xs font-medium leading-4 tracking-button-xs text-base-primary shadow-base-btn-sm transition-colors hover:bg-gray-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
      >
        {action.label}
        {action.external && <Icon name="squareArrowOutUpRight" size={16} />}
      </button>
    </div>
  );
}

function RecentActivity({
  activity,
  pending,
  unavailable,
  onLoadMore,
  onViewEntry,
}: {
  activity: ActivityEntry[];
  /** The first page is in flight; see `AppPane`. */
  pending?: boolean;
  /** No feed was read at all; see `AppPane`. */
  unavailable?: boolean;
  /** Absent when there is no next page. */
  onLoadMore?: () => void;
  /** See `AppPane`. */
  onViewEntry?: (entry: ActivityEntry) => void;
}) {
  return (
    <Card className="p-4" busy={pending}>
      <h2 className="text-sm font-medium leading-5 text-base-foreground">Recent activity</h2>

      {pending ? (
        <PendingRows />
      ) : activity.length === 0 ? (
        // Deliberately NOT the chart's "in the last 24hrs". The entries outlive
        // the window they were sent in - the feed keeps the last messages even
        // when they are a day or more old, because what the user came here for is
        // what this app last did - so borrowing the chart's sentence would state a
        // window this card does not use. It also put the same line twice on a pane
        // with no traffic, which is how the inaccuracy came to light.
        <EmptyNote>
          {unavailable ? "Recent activity couldn't be read" : "No recent messages"}
        </EmptyNote>
      ) : (
      <table className="mt-4 w-full">
        <thead>
          <tr className="text-base-xs text-base-muted-foreground">
            <th scope="col" className="pb-2 text-left font-normal">
              Time
            </th>
            <th scope="col" className="pb-2 text-left font-normal">
              Security
            </th>
            <th scope="col" className="pb-2 text-left font-normal">
              Model
            </th>
            <th scope="col" className="pb-2 text-left font-normal">
              Message
            </th>
            <th scope="col" className="pb-2 text-right font-normal">
              Action
            </th>
          </tr>
        </thead>
        <tbody>
          {activity.map((entry) => {
            const verdict = rowVerdict(entry);
            return (
            <tr key={entry.id} className="border-t border-base-border">
              <td className="whitespace-nowrap py-3 pr-4 text-sm leading-5 text-base-foreground">
                {entry.time}
              </td>
              <td className="py-3 pr-4">
                {verdict ? (
                  <Pill className={PILL_STYLES[verdict]}>{verdict}</Pill>
                ) : (
                  // Withheld, not permitted. A pill here would read as a verdict.
                  <span
                    className="text-sm leading-5 text-base-muted-foreground"
                    title="No security action recorded, or not your request"
                  >
                    &#8212;
                  </span>
                )}
              </td>
              <td className="min-w-0 py-3 pr-4">
                {/* Name only - the design pairs it with a vendor mark, but no
                  * marks are exported yet (open question 2), and the sidebar's
                  * tiles already fall back the same way. */}
                <p className="truncate text-sm leading-5 text-base-foreground">
                  {entry.model}
                </p>
              </td>
              <td className="min-w-0 py-3 pr-4">
                {/* The design draws a message title over this identifier. There
                  * is no title to draw: AG-574 excludes prompt text, and the only
                  * human-readable label the gateway holds for a conversation is
                  * the user's own prompt, stored unredacted. So the cell carries
                  * what the row can truthfully be identified by. */}
                <p className="truncate font-mono text-base-xs leading-4 text-base-muted-foreground">
                  {entry.reference}
                </p>
              </td>
              <td className="py-3 text-right">
                {/* Opens the request in the web dashboard - once there is a URL
                  * for it. The dashboard has no tool/machine/time filter yet, so
                  * the shell has nothing to pass for `onViewEntry`; the button is
                  * drawn ahead of its wiring by decision (2026-08-21). */}
                <button
                  type="button"
                  onClick={() => onViewEntry?.(entry)}
                  className="inline-flex items-center h-8 gap-1.5 rounded-control border border-base-border bg-base-card px-3 text-base-xs font-medium leading-4 tracking-button-xs text-base-primary shadow-base-btn-sm transition-colors hover:bg-gray-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
                >
                  View
                  <Icon name="squareArrowOutUpRight" size={16} />
                </button>
              </td>
            </tr>
            );
          })}
        </tbody>
      </table>
      )}

      {onLoadMore && (
        <div className="mt-4 flex justify-center">
          <button
            type="button"
            onClick={onLoadMore}
            className="h-8 rounded-control border border-base-border bg-base-card px-3 text-base-xs font-medium leading-4 tracking-button-xs text-base-primary shadow-base-btn-sm transition-colors hover:bg-gray-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
          >
            Load more
          </button>
        </div>
      )}
    </Card>
  );
}

/**
 * The feed before it exists: five rows, fixed widths, one line each.
 *
 * Five because that is what the design draws, and a placeholder that guesses the
 * count high leaves the card collapsing when the real answer lands. Mirrors
 * `Overview`'s `PendingRows`, which does the same job for its two tables.
 */
function PendingRows() {
  return (
    <div className="mt-4 flex flex-col gap-3">
      {[0, 1, 2, 3, 4].map((i) => (
        <div
          key={i}
          className="flex items-center justify-between gap-3 border-t border-base-border pt-3"
        >
          <Skeleton className="h-4 w-16" />
          <Skeleton className="h-4 w-14" />
          <Skeleton className="h-4 w-24" />
          <Skeleton className="h-4 w-32" />
          <Skeleton className="h-4 w-12" />
        </div>
      ))}
    </div>
  );
}

function Pill({ className, children }: { className: string; children: ReactNode }) {
  // Drawn at 4px; the radius scale names no 4px stop and its comment maps the
  // drawn 4 onto `sm`, the same call the controls made.
  return (
    <span
      className={`inline-block rounded-sm px-2 py-1 font-mono text-base-xs font-medium uppercase leading-4 tracking-label ${className}`}
    >
      {children}
    </span>
  );
}
