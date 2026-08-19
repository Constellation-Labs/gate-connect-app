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

export type ActivityStatus = "success" | "error";
export type ActivitySecurity = "allow" | "flagged" | "redacted" | "blocked";

export interface ActivityEntry {
  id: string;
  /** Pre-formatted upstream so this pane stays locale-agnostic. */
  time: string;
  status: ActivityStatus;
  /** What the guardrails did, or `null` when the gateway said nothing - either no
   *  decision was recorded, or the row is not this caller's and security detail is
   *  self-only for every role. The two are deliberately indistinguishable here;
   *  both draw no pill. Null is *not* `allow`: rendering it as one would report a
   *  blocked request as permitted. */
  security: ActivitySecurity | null;
  /** The model that served the request, or copy saying none was attributed.
   *
   *  This column used to hold a conversation title. It cannot: the only
   *  human-readable label the gateway holds is the user's own prompt, stored
   *  unredacted, and AG-574 forbids showing prompt text. So the row identifies
   *  the request by what served it and which conversation it belonged to. */
  model: string;
  /** Conversation identifier, rendered mono. */
  reference: string;
  onView: () => void;
}

export interface GateModel {
  /** Model vendor, e.g. "Anthropic". */
  vendor: string;
  /** Fully qualified model id, rendered mono, e.g. "gate/opus 5". */
  id: string;
  /** Vendor mark, 16px. */
  logo?: ReactNode;
}

/** Text sits at the palette's 900 level; the 100 backgrounds are inferred from
 *  that pairing rather than sampled, same as the Overview's action pills. */
const STATUS_STYLES: Record<ActivityStatus, string> = {
  success: "bg-green-100 text-green-900",
  error: "bg-red-100 text-red-900",
};

const SECURITY_STYLES: Record<ActivitySecurity, string> = {
  allow: "bg-green-100 text-green-900",
  flagged: "bg-amber-100 text-amber-900",
  redacted: "bg-purple-100 text-purple-900",
  blocked: "bg-red-100 text-red-900",
};

export function AppPane({
  name,
  isProtected,
  since,
  logo,
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
  unavailable,
  alert,
}: {
  name: string;
  isProtected: boolean;
  /** Relative age of the current status ("2m ago"). */
  since?: string;
  /** 16px brand mark for the header tile. */
  logo?: ReactNode;
  onToggleProtected: () => void;
  stats: UsageStats;
  buckets: MessagesBucket[];
  modelChoice: ModelChoice;
  onChooseModel: (choice: ModelChoice) => void;
  gateModel: GateModel;
  onChangeModel: () => void;
  /** Pre-formatted balance, e.g. "$10.25 available". */
  credits: string;
  onAddCredits: () => void;
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
    <div className="flex flex-1 flex-col gap-4 overflow-auto bg-gray-100 p-6">
      <header className="flex items-center gap-3">
        <span
          aria-hidden
          className="flex size-10 shrink-0 items-center justify-center rounded-base border border-white/[0.24] bg-black text-sm font-medium text-white"
          style={{
            backgroundImage:
              "linear-gradient(180deg, rgba(255,255,255,0.32) 0%, rgba(0,0,0,0.32) 100%)",
          }}
        >
          {logo ?? name.charAt(0)}
        </span>
        <div className="min-w-0 flex-1">
          <h1 className="truncate text-xl font-medium leading-6 tracking-heading text-neutral-900">
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
          <BaseSwitch on={isProtected} label={name} onClick={onToggleProtected} />
        </span>
      </header>

      {alert}

      <StatTiles stats={stats} pending={pending} />
      <MessagesChart buckets={buckets} pending={pending} unavailable={unavailable?.chart} />

      <ModelSelection
        appName={name}
        choice={modelChoice}
        onChoose={onChooseModel}
        gateModel={gateModel}
        onChangeModel={onChangeModel}
        credits={credits}
        onAddCredits={onAddCredits}
      />

      <RecentActivity
        activity={activity}
        pending={eventsPending}
        unavailable={unavailable?.events}
        onLoadMore={onLoadMore}
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
      <h2 className="text-sm font-medium leading-5 text-neutral-900">Model selection</h2>
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
          <p className="font-mono text-sm leading-5 text-neutral-900">{gateModel.id}</p>
        </InfoRow>

        <InfoRow
          icon={<Icon name="creditCard" size={16} />}
          action={{ label: "Add credits", onClick: onAddCredits, external: true }}
        >
          <p className="text-sm leading-5 text-neutral-900">
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
      className={`flex items-center gap-3 rounded-lg border p-3 text-left transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
        selected
          ? "border-base-primary bg-base-card"
          : "border-base-border bg-base-card hover:bg-gray-50"
      }`}
    >
      <span
        aria-hidden
        className="flex size-8 shrink-0 items-center justify-center rounded-base border border-base-border text-neutral-700"
      >
        {icon}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium leading-5 text-neutral-900">
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
    <div className="flex items-center gap-3 rounded-lg border border-base-border p-3">
      <span
        aria-hidden
        className="flex size-8 shrink-0 items-center justify-center rounded-base border border-base-border text-neutral-700"
      >
        {icon}
      </span>
      <div className="min-w-0 flex-1">{children}</div>
      <button
        type="button"
        onClick={action.onClick}
        className="flex shrink-0 items-center gap-1.5 rounded-base border border-base-border bg-base-card px-2 py-1 text-base-xs font-medium leading-4 text-base-primary shadow-base-2xs transition-colors hover:bg-gray-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
      >
        {action.label}
        {action.external && <Icon name="squareArrowOutUpRight" size={12} />}
      </button>
    </div>
  );
}

function RecentActivity({
  activity,
  pending,
  unavailable,
  onLoadMore,
}: {
  activity: ActivityEntry[];
  /** The first page is in flight; see `AppPane`. */
  pending?: boolean;
  /** No feed was read at all; see `AppPane`. */
  unavailable?: boolean;
  /** Absent when there is no next page. */
  onLoadMore?: () => void;
}) {
  return (
    <Card className="p-4" busy={pending}>
      <h2 className="text-sm font-medium leading-5 text-neutral-900">Recent activity</h2>

      {pending ? (
        <PendingRows />
      ) : activity.length === 0 ? (
        // The entries outlive the window they were sent in: the feed keeps the
        // last messages even when they are a day or more old, because what the
        // user came here for is what this app last did, and a table that empties
        // itself at midnight answers a question nobody asked. So this reads as
        // "nothing recent", not "nothing ever".
        <EmptyNote>
          {unavailable
            ? "Recent activity couldn't be read"
            : "No messages sent in the last 24hrs"}
        </EmptyNote>
      ) : (
      <table className="mt-4 w-full">
        <thead>
          <tr className="text-base-xs text-base-muted-foreground">
            <th scope="col" className="pb-2 text-left font-normal">
              Time
            </th>
            <th scope="col" className="pb-2 text-left font-normal">
              Status
            </th>
            <th scope="col" className="pb-2 text-left font-normal">
              Security
            </th>
            <th scope="col" className="pb-2 text-left font-normal">
              Model
            </th>
            <th scope="col" className="w-20 pb-2 text-right font-normal">
              Action
            </th>
          </tr>
        </thead>
        <tbody>
          {activity.map((entry) => (
            <tr key={entry.id} className="border-t border-base-border">
              <td className="whitespace-nowrap py-3 pr-4 text-sm leading-5 text-neutral-900">
                {entry.time}
              </td>
              <td className="py-3 pr-4">
                <Pill className={STATUS_STYLES[entry.status]}>{entry.status}</Pill>
              </td>
              <td className="py-3 pr-4">
                {entry.security ? (
                  <Pill className={SECURITY_STYLES[entry.security]}>{entry.security}</Pill>
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
                <p className="truncate text-sm leading-5 text-neutral-900">
                  {entry.model}
                </p>
                <p className="truncate font-mono text-base-2xs leading-4 text-base-muted-foreground">
                  {entry.reference}
                </p>
              </td>
              <td className="py-3 text-right">
                <button
                  type="button"
                  onClick={entry.onView}
                  className="inline-flex items-center gap-1.5 rounded-base border border-base-border bg-base-card px-2 py-1 text-base-xs font-medium leading-4 text-base-primary shadow-base-2xs transition-colors hover:bg-gray-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
                >
                  View
                  <Icon name="squareArrowOutUpRight" size={12} />
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      )}

      {onLoadMore && (
        <div className="mt-4 flex justify-center">
          <button
            type="button"
            onClick={onLoadMore}
            className="rounded-base border border-base-border bg-base-card px-3 py-1.5 text-base-xs font-medium leading-4 text-base-primary shadow-base-2xs transition-colors hover:bg-gray-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
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
          <Skeleton className="h-4 w-14" />
          <Skeleton className="h-4 w-32" />
          <Skeleton className="h-4 w-12" />
        </div>
      ))}
    </div>
  );
}

function Pill({ className, children }: { className: string; children: ReactNode }) {
  return (
    <span
      className={`inline-block rounded-base px-1.5 py-0.5 font-mono text-base-xs font-medium uppercase leading-4 tracking-label ${className}`}
    >
      {children}
    </span>
  );
}
