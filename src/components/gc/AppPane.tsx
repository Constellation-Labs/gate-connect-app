import type { ReactNode } from "react";
import { BADGE_STYLES, BaseSwitch, Card, EmptyNote, Pill, Skeleton } from "./base";
import { Icon } from "./Icon";
import { MessagesChart, StatTiles } from "./metrics";
import type { MessagesBucket, UsageStats } from "./metrics";
import { STATUS_TEXT, statusDetail } from "./Sidebar";
import type { AppStatus } from "./Sidebar";

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
export type {
  ActivityEntry,
  ActivitySecurity,
  ActivityStatus,
} from "../../lib/toolEventRow";
import type { ActivityEntry } from "../../lib/toolEventRow";

export interface GateModel {
  /** Model vendor, e.g. "anthropic". */
  vendor: string;
  /**
   * Every enabled model, in the user's order, rendered mono.
   *
   * The whole set, not the first of it. Figma 228:89517 draws this card with a
   * single model row, and following that drew a heading reading "Current Gate
   * models" over exactly one id - which reads as the card having lost five of
   * them, because that is indistinguishable from what it would look like if it
   * had. A count in the heading is not worth a list the user cannot see.
   *
   * Supersedes the earlier `alsoEnabled` count, which drove only the heading's
   * plural: a number that says "and five others" without naming them answers
   * the wrong half of the question.
   *
   * Listed the way the confirmation dialog lists a set (130:48278): stacked, and
   * with no vendor mark once there is more than one, since a single glyph cannot
   * stand for several vendors and repeating it per line would claim each id
   * belongs to the first one's.
   */
  ids: string[];
  /** Vendor mark, 16px. Drawn only for a set of one. */
  logo?: ReactNode;
}


export function AppPane({
  name,
  isProtected,
  status,
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
  modelBusy,
  modelAttention,
  modelPending,
  credits,
  plan,
  onAddCredits,
  onManageBilling,
  activity,
  pending,
  eventsPending,
  onLoadMore,
  unavailable,
  alert,
}: {
  name: string;
  isProtected: boolean;
  /**
   * Observed status, which the header line draws in full.
   *
   * This is the pane's half of the split the rail makes: a 250px row cannot fit
   * "Not protected - Configuration update failed" and truncates the reason
   * away, so `Sidebar`'s row prints the phrase alone and the reason lands here,
   * where the header has the width for a sentence.
   *
   * Absent falls back to the intent flag below, which is the older, coarser
   * line: "Protected" or "Not protected", with no reason behind it.
   */
  status?: AppStatus;
  /** Relative age of the current status ("2m ago"). Ignored when `status`
   *  carries its own suffix. */
  since?: string;
  /** 16px brand mark for the header tile. */
  logo?: ReactNode;
  /** A routing write is in flight, so the switch refuses a second click. */
  busy?: boolean;
  onToggleProtected: () => void;
  stats: UsageStats;
  buckets: MessagesBucket[];
  /** What this app is set to, or `null` when no reading landed.
   *
   *  Null is not "App default". A control drawn from a failed read is the bug
   *  CLAUDE.md's principle 2 names: the card would show App default, and clicking
   *  Gate model would look like a change when it is the first thing anyone said.
   *  So null disables the choice and says why.
   *
   *  Omitting it and `onChooseModel` together is a third thing again: the card
   *  is withheld entirely. A multi-provider tool gets that. OpenCode, OpenClaw
   *  and Hermes route whichever of their configured providers Gate covers -
   *  `lib/groups.ts` calls them "tools that talk to several providers, not one
   *  model family" - so "what does this app use on Gate model" has no single
   *  answer for them, and `main` never poses the question at all. Per the house
   *  rule the Settings pane states: an omitted handler omits its control. No
   *  `onChooseModel`, no card. */
  modelChoice?: ModelChoice | null;
  onChooseModel?: (choice: ModelChoice) => void;
  /** The remembered model, or `null` when none has been chosen.
   *
   *  Remembered, not necessarily active: it stays on screen under App default so
   *  the user can see what they would be switching to, and the card marks it
   *  inactive rather than letting its presence imply Gate is serving it. */
  gateModel?: GateModel | null;
  onChangeModel?: () => void;
  /** A model write is in flight, so the controls refuse a second click. */
  modelBusy?: boolean;
  /** Why this app's Gate model needs attention, if it does (AG-592).
   *
   *  Highlighted in place rather than raised as a banner: the cause is about
   *  this one control, and the recovery is the control itself. Null means
   *  nothing to say - which is not the same as "all clear", since an unread
   *  catalogue or balance also yields null. See `modelAttention`. */
  modelAttention?: string | null;
  /** The model *preference* read has not landed.
   *
   *  Its own flag rather than the pane's `pending`, which tracks the activity
   *  reading. Sharing one made the card draw skeletons whenever this machine was
   *  unattributed - a fact about traffic, with nothing to say about a setting. */
  modelPending?: boolean;
  /** Pre-formatted balance, or `null` when nothing reports one. Renders "N/A":
   *  no endpoint returns a Gate credit balance yet, and a dash reads as a value.
   *  See principle 6. */
  credits?: string | null;
  /** The org's plan, or null when the gateway did not name one (AG-592).
   *
   *  Optional for the same reason as the handlers above: a tool whose card is
   *  withheld entirely has no plan line to draw. */
  plan?: string | null;
  onAddCredits?: () => void;
  /** Absent when the gateway named no billing destination, which removes the
   *  control entirely rather than drawing a dead one. */
  onManageBilling?: () => void;
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
    <div className="flex flex-1 flex-col gap-4 overflow-auto bg-base-background p-6">
      <header className="flex items-center gap-3">
        <span
          aria-hidden
          className="flex size-11 shrink-0 items-center justify-center rounded-sm border border-white/[0.24] bg-black text-sm font-medium text-white"
          style={{
            backgroundImage:
              "linear-gradient(180deg, rgba(255,255,255,0.32) 0%, rgba(0,0,0,0.32) 100%)",
          }}
        >
          {logo ?? name.charAt(0)}
        </span>
        <div className="min-w-0 flex-1">
          <h1 className="truncate text-xl font-medium leading-6 tracking-heading text-base-foreground">
            {name}
          </h1>
          <AppStatusLine isProtected={isProtected} status={status} since={since} />
        </div>
        <span className="flex shrink-0 items-center gap-2">
          <span className="text-sm font-medium leading-5 text-base-foreground">
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
      <MessagesChart
        buckets={buckets}
        pending={pending}
        unavailable={unavailable?.chart}
      />

      {onChooseModel && onChangeModel && onAddCredits && (
        <ModelSelection
          appName={name}
          choice={modelChoice ?? null}
          pending={modelPending}
          busy={modelBusy}
          attention={modelAttention}
          onChoose={onChooseModel}
          gateModel={gateModel ?? null}
          onChangeModel={onChangeModel}
          credits={credits ?? null}
          plan={plan ?? null}
          onAddCredits={onAddCredits}
          onManageBilling={onManageBilling}
        />
      )}

      <RecentActivity
        activity={activity}
        pending={eventsPending}
        unavailable={unavailable?.events}
        onLoadMore={onLoadMore}
      />
    </div>
  );
}

/**
 * The header's status line: the rail's coloured phrase, and after it the reason
 * the rail had no room for. It wraps rather than truncates - the reason is the
 * only thing on screen telling the user why their tool is not covered.
 */
function AppStatusLine({
  isProtected,
  status,
  since,
}: {
  isProtected: boolean;
  status?: AppStatus;
  since?: string;
}) {
  const text = status
    ? STATUS_TEXT[status.kind]
    : isProtected
      ? STATUS_TEXT.protected
      : STATUS_TEXT["not-protected"];
  const detail = status ? statusDetail(status) : since;

  return (
    <p className="text-base leading-6">
      <span className={text.className}>{text.label}</span>
      {detail && <span className="text-neutral-500"> - {detail}</span>}
    </p>
  );
}

/**
 * The upstream's mark beside the model (Figma 272:3282).
 *
 * A monogram, not a brand asset: this repo carries no provider logos - the sidebar
 * falls back to a letter for the same reason - and drawing someone else's mark
 * badly from memory is worse than not drawing it. Swap this for the real SVGs when
 * they land; the shape and size are already what the design asks for.
 *
 * Renders nothing when the provider is unknown, rather than a question mark: the
 * model name beside it already carries the row, and an empty slot keeps the column
 * aligned.
 *
 * The glyph is decorative, so it is `aria-hidden` and the name is carried by an
 * `sr-only` sibling rather than by `title` alone. A `title` on an `aria-hidden`
 * element is reachable by mouse and by nothing else, which for a one-letter
 * monogram means the provider is the one thing on the row a screen reader could
 * not get at. The tooltip stays for pointer users.
 */
function VendorMark({ provider }: { provider: string | null }) {
  if (!provider) return <span aria-hidden className="size-4 shrink-0" />;
  return (
    <>
      <span
        aria-hidden
        title={provider}
        className="flex size-4 shrink-0 items-center justify-center rounded-sm bg-neutral-200 font-mono text-[0.5rem] font-semibold uppercase leading-none text-neutral-700"
      >
        {provider.charAt(0)}
      </span>
      <span className="sr-only">{provider}</span>
    </>
  );
}

/**
 * The Model selection card (Figma `Flows / App`).
 *
 * Three things this deliberately does not do.
 *
 * **It does not default.** `choice` of `null` means no reading landed, and the
 * radios are drawn unselected and disabled rather than showing App default. A
 * switch driven by a failed read is the bug `lib/groups.ts` documents for
 * routing: the control renders one way, and clicking it turns off the setting the
 * user was trying to turn on.
 *
 * **It does not imply that a remembered model is a live one.** "Current Gate
 * model" is drawn only while Gate is the source. It once stayed visible under App
 * default, dimmed and labelled "not in use", so the user could see what they would
 * be switching to - but a section headed "Current" that describes nothing current
 * has to be read twice to learn it does not apply, and it sat directly under the
 * radio that had just said the same thing. What it was for is now carried by the
 * Gate radio itself, which names the remembered model in its own description, so
 * the answer to "what would I be switching to?" is on the control that switches.
 * `source` remains the only thing that decides what Gate serves.
 *
 * There used to be a third: a `supported` flag that withheld the control for an
 * app the gateway could not identify on a request. It went with the server-side
 * store - the choice is now a local file keyed on our own tool slug, so every
 * app this window lists can hold one.
 */
function ModelSelection({
  appName,
  choice,
  pending,
  busy,
  attention,
  onChoose,
  gateModel,
  onChangeModel,
  credits,
  plan,
  onAddCredits,
  onManageBilling,
}: {
  appName: string;
  choice: ModelChoice | null;
  pending?: boolean;
  busy?: boolean;
  attention?: string | null;
  onChoose: (choice: ModelChoice) => void;
  gateModel: GateModel | null;
  onChangeModel: () => void;
  credits: string | null;
  /** The org's plan, or null when the gateway did not name one (AG-592). */
  plan: string | null;
  onAddCredits: () => void;
  /** Absent when the gateway named no billing destination, which removes the
   *  control entirely rather than drawing a dead one. */
  onManageBilling?: () => void;
}) {
  // Under App default a chosen model is remembered, not served. Kept as one
  // named value because three places below depend on it and they must agree.
  const gateActive = choice === "gate";

  return (
    <Card className="p-4">
      <h2 className="text-base font-medium leading-6 tracking-heading-16 text-base-foreground">
        Model selection
      </h2>
      <p className="mt-1 text-sm leading-5 text-base-muted-foreground">
        Choose whether {appName} or Gate selects the AI model for requests
      </p>

      {pending ? (
        <div className="mt-4 grid grid-cols-2 gap-4">
          <Skeleton className="h-[3.75rem]" />
          <Skeleton className="h-[3.75rem]" />
        </div>
      ) : (
        <>
          <div
            role="radiogroup"
            aria-label="Model selection"
            className="mt-4 grid grid-cols-2 gap-4"
          >
            <ModelOption
              selected={choice === "app"}
              disabled={choice === null || busy}
              onSelect={() => onChoose("app")}
              icon={<Icon name="cube" size={20} />}
              title="App default"
              description="Use the model configured in your app"
            />
            <ModelOption
              selected={gateActive}
              disabled={choice === null || busy}
              onSelect={() => onChoose("gate")}
              icon={<Icon name="layers" size={20} />}
              title="Gate model"
              // Names the chosen model once there is one, rather than the
              // generic line the frame draws.
              //
              // A deliberate deviation, for a reason worth keeping: choosing a
              // model from "Change model" while on App default only *remembers*
              // it, and the sole feedback was "not in use" in small grey text on
              // a row that otherwise looked identical. Saving a model therefore
              // appeared to do nothing at all. Naming it here makes the save
              // visible and points at the switch that would actually use it.
              description={
                !gateModel
                  ? "Use a model selected in Gate AI"
                  : gateModel.ids.length === 1
                    ? `Use ${gateModel.ids[0]}`
                    : `Use any of ${gateModel.ids.length} Gate models`
              }
            />
          </div>
          {choice === null && (
            <EmptyNote className="mt-4" icon="cube">
              Gate could not read this app's model setting, so it is not shown.
              The setting itself is unchanged.
            </EmptyNote>
          )}
        </>
      )}

      {attention && (
        // AG-592's Needs attention, as a highlight rather than a dialog: the
        // cause concerns this one control and the recovery is the control
        // itself, so interrupting the pane would put the explanation further
        // from the fix.
        <p
          role="status"
          className="mt-4 flex items-start gap-2 rounded-lg border border-amber-300 bg-amber-50 p-3 text-sm leading-5 text-amber-900"
        >
          <Icon name="triangleAlert" size={16} className="mt-0.5 shrink-0" />
          <span>{attention}</span>
        </p>
      )}

      {/* Only while Gate is the source. Under App default there is no current
       * Gate model to report: the row would be a section headed "Current"
       * describing nothing current, sitting directly beneath the radio that had
       * just said so. What it was for - seeing what you would switch to - is
       * carried by the Gate radio, which names the remembered model itself.
       * Choosing one is still reachable from App default: that radio opens the
       * picker when no model is enabled yet. */}
      {gateActive && (
        <>
          <p className="mt-4 text-base-xs text-base-muted-foreground">
            {(gateModel?.ids.length ?? 0) > 1 ? "Current Gate models" : "Current Gate model"}
          </p>

          <div className="mt-2">
            {gateModel === null ? (
              <EmptyNote icon="cube">
                No Gate model chosen yet. Choose one to see what Gate would serve.
              </EmptyNote>
            ) : (
              <InfoRow
                // No mark for a set: see `GateModel.ids`.
                icon={
                  gateModel.ids.length === 1
                    ? (gateModel.logo ?? <Icon name="cube" size={16} />)
                    : undefined
                }
                actions={[{ label: "Change model", onClick: onChangeModel, disabled: busy }]}
              >
                {gateModel.ids.length === 1 ? (
                  <>
                    <p className="text-base-2xs leading-4 text-base-muted-foreground">
                      {gateModel.vendor}
                    </p>
                    <p className="font-mono text-sm leading-5 text-base-foreground">
                      {gateModel.ids[0]}
                    </p>
                  </>
                ) : (
                  <ul className="flex flex-col gap-1">
                    {gateModel.ids.map((id) => (
                      <li key={id} className="truncate font-mono text-sm leading-5 text-base-foreground">
                        {id}
                      </li>
                    ))}
                  </ul>
                )}
              </InfoRow>
            )}
          </div>
        </>
      )}

      <div className="mt-2 flex flex-col gap-2">
        <InfoRow
          icon={<Icon name="creditCard" size={20} />}
          actions={[
            // AG-592 asks for Manage billing "when available to the account".
            // Availability is answered by the gateway naming a destination: no
            // URL, no button. A disabled one would be a control the user has to
            // click to learn is not for them.
            ...(onManageBilling
              ? [{ label: "Manage billing", onClick: onManageBilling, external: true }]
              : []),
            { label: "Add credits", onClick: onAddCredits, external: true },
          ]}
        >
          {/* AG-592 asks the tool detail to show the plan alongside the
           *  balance. Drawn only when the gateway named one: a plan is the
           *  thing a reader would act on, by upgrading, and naming the wrong
           *  one sends them to change something they may already have. */}
          {plan && (
            <p className="text-base-2xs leading-4 text-base-muted-foreground">
              {plan.charAt(0).toUpperCase() + plan.slice(1)} plan
            </p>
          )}
          <p className="text-sm leading-5 text-base-foreground">
            <span className="text-neutral-600">Gate credits: </span>
            {credits ?? "N/A"}
          </p>
        </InfoRow>
      </div>
    </Card>
  );
}

function ModelOption({
  selected,
  disabled,
  onSelect,
  icon,
  title,
  description,
}: {
  selected: boolean;
  /** No reading landed, or a write is in flight. Disabled rather than hidden so
   *  the card keeps its shape and the user can see the choice exists. */
  disabled?: boolean;
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
      disabled={disabled}
      onClick={onSelect}
      className={`flex items-center gap-3 rounded-lg border p-3 text-left transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary disabled:cursor-not-allowed disabled:opacity-60 ${
        selected
          ? "border-base-primary bg-base-card"
          : "border-base-border bg-base-card enabled:hover:bg-gray-50"
      }`}
    >
      <span
        aria-hidden
        className="flex size-9 shrink-0 items-center justify-center rounded-sm border border-base-border text-base-foreground"
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
        <Icon
          name="circleCheck"
          size={16}
          className="shrink-0 text-base-primary"
        />
      )}
    </button>
  );
}

function InfoRow({
  icon,
  children,
  actions,
}: {
  /** Omitted for a row about a set, where no one mark could stand for it. */
  icon?: ReactNode;
  children: ReactNode;
  /** A list because the credits row carries two once a billing destination
   *  exists. Rightmost is the primary one, which is the order the eye lands in.
   *  An empty list draws nothing, which is how an absent destination removes
   *  its button rather than disabling it. */
  actions?: ReadonlyArray<{
    label: string;
    onClick: () => void;
    external?: boolean;
    disabled?: boolean;
  }>;
  // `muted` went with the row it dimmed. It existed to draw a remembered model
  // under App default as "not in use"; that row is now shown only while Gate is
  // the source, so nothing is left to dim.
}) {
  return (
    <div className="flex items-center gap-3 rounded-lg border border-base-border p-3"
    >
      <span
        aria-hidden
        className="flex size-9 shrink-0 items-center justify-center rounded-sm border border-base-border text-base-foreground"
      >
        {icon}
      </span>
      <div className="min-w-0 flex-1">{children}</div>
      {/* Base's button geometry from the Figma audit (h-8, rounded-control,
        * the moulded shadow), applied over our list of actions. */}
      {actions?.map((action) => (
        <button
          key={action.label}
          type="button"
          onClick={action.onClick}
          disabled={action.disabled}
          className="flex h-8 shrink-0 items-center gap-1.5 rounded-control border border-base-border bg-base-card px-3 text-base-xs font-medium leading-4 tracking-button-xs text-base-primary shadow-base-btn-sm transition-colors enabled:hover:bg-gray-50 disabled:cursor-not-allowed disabled:opacity-60 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
        >
          {action.label}
          {action.external && <Icon name="squareArrowOutUpRight" size={16} />}
        </button>
      ))}
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
  /** See `AppPane`. */
}) {
  return (
    <Card className="p-4" busy={pending}>
      <h2 className="text-base font-medium leading-6 tracking-heading-16 text-base-foreground">
        Recent activity
      </h2>

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
          {unavailable
            ? "Recent activity couldn't be read"
            : "No recent messages"}
        </EmptyNote>
      ) : (
        <table className="mt-5 w-full">
          <thead>
            <tr className="text-base-xs text-base-muted-foreground">
              {/* Column shares taken off `table/recent-activity`, whose body cells
                sit at 0/148/288/408 and whose Action button starts at 620 inside
                688 - so 148, 140, 120, 212 and 68 wide once each 16px gutter is
                counted in, which is the 21.5/20.5/17.5/30.5/10 below. Shares
                rather than pixel counts, so they hold at both window sizes. */}
              <th scope="col" className="w-[21.5%] pb-3 text-left font-normal">
                Time
              </th>
              {/* The frame's second column, drawn with a 20px glyph beside it. */}
              <th scope="col" className="w-[20.5%] pb-3 text-left font-normal">
                Type
              </th>
              {/* One column, not two: the design merged status into security, so a
                failed request reads ERROR and every other row reads what the
                guardrails did. */}
              <th scope="col" className="w-[17.5%] pb-3 text-left font-normal">
                Security
              </th>
              <th scope="col" className="w-[30.5%] pb-3 text-left font-normal">
                Model
              </th>
              <th scope="col" className="w-[10%] pb-3 text-right font-normal">
                Action
              </th>
            </tr>
          </thead>
          <tbody>
            {activity.map((entry) => (
              <tr key={entry.id} className="border-t border-base-border">
                <td className="whitespace-nowrap py-[1.125rem] pr-4 text-sm leading-5 text-base-foreground">
                  {entry.time}
                </td>
                {/* Type. The frame draws a 20px glyph 8px from the label, both at
                  `base/foreground` - the downloaded asset's own stroke is
                  #030712, and the glyph takes it from this span. Spelled as the
                  gateway spelled it, like `SecurityPane` does with the same
                  field: a display vocabulary for values only the gateway knows
                  would be invented here. */}
                <td className="py-[1.125rem] pr-4">
                  {entry.category ? (
                    <span className="flex items-center gap-2 text-sm leading-5 text-base-foreground">
                      {entry.categoryIcon && (
                        <Icon name={entry.categoryIcon} size={20} />
                      )}
                      <span className="truncate">{entry.category}</span>
                    </span>
                  ) : (
                    // The same withholding the Security cell draws, for the same
                    // reason: the gateway named no category, or this row is not
                    // this caller's to see into.
                    <span
                      className="text-sm leading-5 text-base-muted-foreground"
                      title="No guardrail category recorded, or not your request"
                    >
                      &#8212;
                    </span>
                  )}
                </td>
                <td className="py-[1.125rem] pr-4">
                  {/* Error outranks the guardrail verdict, which is the design's
                    call and the defensible one: a request that did not complete
                    is the thing the reader needs first. It does cost information -
                    a failed request that was also flagged now shows only ERROR -
                    so the row keeps the quieter fact rather than dropping it.
                    In the tooltip for pointer users, and in `sr-only` text beside
                    the badge for everyone else: a `title` is the whole mitigation
                    here, and a mitigation only a mouse can reach is not one. */}
                  {entry.status === "error" ? (
                    <>
                      <Pill
                        className={BADGE_STYLES.error}
                        title={
                          entry.security
                            ? `Request failed. Guardrails: ${entry.security}.`
                            : "Request failed."
                        }
                      >
                        error
                      </Pill>
                      {entry.security && (
                        <span className="sr-only">
                          Guardrails: {entry.security}.
                        </span>
                      )}
                    </>
                  ) : entry.security ? (
                    <Pill className={BADGE_STYLES[entry.security]}>
                      {entry.security}
                    </Pill>
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
                <td className="min-w-0 py-[1.125rem] pr-4">
                  {/* Truncated, not `nowrap`. A model id is unbounded - the
                    canonical ones run to `anthropic/claude-opus-4-5-20260514` -
                    and an un-truncated cell makes that string the table's minimum
                    width. Every column now carries a share of the table, so the
                    id cannot push the floor past the card at the window's 1024px
                    minimum. The cap lives on the `th` as a share rather than a
                    pixel count, so it holds at every size above that. */}
                  <span className="flex items-center gap-2">
                    <VendorMark provider={entry.provider} />
                    <span className="truncate text-sm leading-5 text-base-foreground">
                      {entry.model}
                    </span>
                  </span>
                </td>
                <td className="py-[1.125rem] text-right">
                  {entry.onView ? (
                    <button
                      type="button"
                      onClick={entry.onView}
                      className="inline-flex h-8 items-center gap-1.5 rounded-control border border-base-border bg-base-card px-3 text-base-xs font-medium leading-4 tracking-button-xs text-base-primary shadow-base-btn-sm transition-colors hover:bg-gray-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
                    >
                      View
                      <Icon name="squareArrowOutUpRight" size={16} />
                    </button>
                  ) : null}
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

