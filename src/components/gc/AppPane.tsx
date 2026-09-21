import type { ReactNode } from "react";
import { BADGE_STYLES, BaseSwitch, Card, EmptyNote, Pill, Skeleton } from "./base";
import { Icon } from "./Icon";
import { providerMarkFor } from "./ProviderMark";
import { categoryTone } from "../../lib/toolEvents";
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
   * Every enabled model, in the user's order.
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
}


export function AppPane({
  name,
  description,
  isProtected,
  status,
  since,
  logo,
  appVendorMark,
  appFallbackMark,
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
  unattributed,
  alert,
}: {
  name: string;
  /**
   * What this app is, in one sentence - `lib/groups.ts`' `describeMember`.
   *
   * The header used to be able to skip this: a row called "Claude Code" says
   * what it is. The rows are surface kinds now ("App", "Web", "CLI"), which is
   * legible in the rail because the eyebrow above them names the vendor, and
   * not legible here at all - this h1 is the only "Anthropic" the pane has, and
   * it does not say it. So the sentence comes with the name.
   *
   * Optional, and absent means absent: a row with no copy written for it gets
   * no line rather than a placeholder under its own name.
   */
  description?: string;
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
  /**
   * The app vendor's full-colour mark, for the App-default row alone.
   *
   * Separate from `logo` because the two tiles want opposite treatments and
   * one prop cannot serve both: the pane header is a black tile with white
   * ink, so it takes the monochrome `BrandMark`; the App-default row
   * (`408:25491`) is a light tile and draws the provider's own colour -
   * `#E8704E` for Anthropic. Absent for an app with no single vendor behind
   * it, where the row falls back to `logo` and then the cube.
   */
  appVendorMark?: ReactNode;
  /**
   * The rail's section mark at the App-default row's own size, for an app with
   * no single vendor.
   *
   * Separate from `logo` only because of the size: `logo` is the pane header's,
   * drawn at 16 into a 44px black tile, and this row's tile is 36px around a
   * 20px glyph. Sharing one prop put two sizes in one slot.
   */
  appFallbackMark?: ReactNode;
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
  /** This app's traffic is never attributed to it, so neither section has a
   *  reading to show and neither ever will.
   *
   *  Deliberately separate from `unavailable`, which reports a read that failed
   *  and might succeed on a retry. Both were once the same flag, and the result
   *  was a chat-domain pane whose note explained that per-app activity does not
   *  exist directly above two cards claiming it could not be read - a fault
   *  report over a permanent, intended shape of the data. */
  unattributed?: boolean;
  /** Slot for the `AlertBanner` about this app: drift, a check error, or a
   *  whole-machine cause worded for this app. */
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
          {/* Between the name and the status line on purpose: this is what the
              row IS and the line below is what it is DOING, and the pane reads
              top-down from identity to state. Not truncated - the sentence is
              the point, and the header is the one place with room for it. */}
          {description && (
            <p className="text-base-xs leading-4 text-neutral-600">{description}</p>
          )}
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

      <StatTiles stats={stats} pending={pending} unattributed={unattributed} />
      <MessagesChart
        buckets={buckets}
        pending={pending}
        unavailable={unavailable?.chart}
        unattributed={unattributed}
      />

      {onChooseModel && onChangeModel && onAddCredits && (
        <ModelSelection
          appName={name}
          // The colour mark where the app has one vendor, the rail's
          // monochrome one where it does not (AG-879). This row used to take
          // `logo` outright, which is the set built for the header's dark tile
          // and renders flat on this one.
          appLogo={appVendorMark ?? appFallbackMark ?? logo}
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
        unattributed={unattributed}
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
 * The real brand mark now that `ProviderMark` carries them; this used to be a
 * one-letter monogram because the repo held no provider logos. Unmapped vendors
 * fall back to the cube, which is the `Icon / Boxes` the frames draw in the same
 * slot - a letter tile read as a different kind of thing entirely.
 *
 * Renders an empty slot when the provider is unknown, rather than a question
 * mark: the model name beside it already carries the row, and the spacer keeps
 * the column aligned.
 *
 * The glyph is decorative, so it is `aria-hidden` and the name is carried by an
 * `sr-only` sibling rather than by `title` alone. A `title` on an `aria-hidden`
 * element is reachable by mouse and by nothing else, which would make the
 * provider the one thing on the row a screen reader could not get at. The
 * tooltip stays for pointer users.
 */
function VendorMark({
  provider,
  vendor,
}: {
  /** What the gateway said served the request, and the only thing this row is
   *  allowed to put into words. Null when it named none. */
  provider: string | null;
  /** The namespace whose mark to draw - the provider where there is one, the
   *  model id's own namespace otherwise. See `ToolEventRow.vendor`. */
  vendor: string | null;
}) {
  if (!vendor) return <span aria-hidden className="size-4 shrink-0" />;
  return (
    <>
      <span
        aria-hidden
        // Only where the gateway named it. A derived vendor draws its mark and
        // says nothing: the mark sits beside the model id it was taken from, so
        // it identifies the model, while a tooltip would be asserting who
        // served a request that may never have reached anyone.
        title={provider ?? undefined}
        // `base.foreground`, not the muted grey the Overview's row glyphs take.
        // A brand mark is not a glyph: the colour ones carry their own fills and
        // ignore this, and the monochrome ones (openai, grok, ibm, ai21,
        // inception, relace) inherit it - so muting the wrapper rendered OpenAI
        // grey here and inked in the picker, while Moonshot, which hard-codes
        // black, stayed black in both. Same ink as `dialogs.tsx`'s row now.
        className="flex size-4 shrink-0 items-center justify-center text-base-foreground"
      >
        {providerMarkFor(vendor) ?? <Icon name="cube" size={16} />}
      </span>
      {provider && <span className="sr-only">{provider}</span>}
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
  appLogo,
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
  /**
   * The mark for the App-default row, already resolved by the caller.
   *
   * **The provider's full-colour mark where the app has one vendor**
   * (`408:25491` draws Anthropic's `#E8704E`), the rail's monochrome section
   * mark otherwise. This doc used to say the opposite - "this row is about the
   * app, not about a provider, so `ProviderMark` is the wrong family here" -
   * which is the reasoning the frame overturns, and it sat on the prop rather
   * than the call site, so it was what the next person editing this component
   * would read. Left as it was, the next "fix" to match the doc would restore
   * the flat mark.
   *
   * The distinction that IS real: the pane header's tile is black and takes the
   * monochrome family, because those marks render in `currentColor` so a dark
   * tile can ink them. This row's tile is light. One prop cannot serve both,
   * which is why the caller resolves it and `logo` stays separate.
   */
  appLogo?: ReactNode;
  choice: ModelChoice | null;
  pending?: boolean;
  busy?: boolean;
  attention?: string | null;
  onChoose: (choice: ModelChoice) => void;
  gateModel: GateModel | null;
  onChangeModel: () => void;
  credits: string | null;
  /** The org's plan, already in the user's vocabulary - the shell maps it
   *  through `formatPlan`, so this is "Pro" rather than the gateway's `paid`.
   *  Null when the gateway named none (AG-592). Title-casing used to happen
   *  here, which is how the raw value reached the screen. */
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

      {/* The App-default branch's own row (`408:25491`), below the divider the
        * frame draws at `408:25490`. The card had nothing here at all: choosing
        * App default left the radios and then the credits row, so the branch
        * that is actually serving the user said less about itself than the one
        * that was not. It is the same slot the Gate branch fills, and it
        * answers the same question - what is serving this app, and what does it
        * cost. Nothing, is the answer, and the frame says so out loud. */}
      {choice === "app" && (
        <>
          <div className="mt-4 border-t border-base-border" />
          <div className="mt-4">
            <InfoRow icon={appLogo ?? <Icon name="cube" size={20} />}>
              {/* `heading/14`, the one named heading step with no tracking at
                * all, so it overrides `text-sm`'s own -0.14px. */}
              <p className="text-sm font-medium leading-5 tracking-heading-14 text-base-foreground">
                Using {appName} model
              </p>
              {/* The frame's last sentence, "No Gate credits used", is dropped.
                * It is only true under BYOK: on a PAYG account the engine strips
                * the tool's own credential and debits the org balance for every
                * eligible slug, whatever the per-tool model source says
                * (`proxy/mod.rs` `strip_client_auth`, reached on
                * `mode == Payg`). An absolute claim about the user's money, on
                * the one surface principle 1 is about, must not be false in a
                * mode the product supports - so the row now says only what the
                * App-default branch actually decides, which is model choice.
                *
                * Raised with design rather than settled here: it is a deviation
                * from drawn copy, which CLAUDE.md reserves for design.
                * `billing_mode` is CLI-only today and the frontend never reads
                * it, which is why the balance is still gated on the Gate branch
                * alone - see the question doc. */}
              <p className="text-base-xs font-medium leading-4 text-base-muted-foreground">
                Gate protects requests, then leaves model choice to {appName}.
              </p>
            </InfoRow>
          </div>
        </>
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
                    ? (providerMarkFor(gateModel.vendor) ?? <Icon name="cube" size={16} />)
                    : undefined
                }
                actions={[{ label: "Change model", onClick: onChangeModel, disabled: busy }]}
              >
                {gateModel.ids.length === 1 ? (
                  <>
                    <p className="text-base-2xs leading-4 text-base-muted-foreground">
                      {gateModel.vendor}
                    </p>
                    <p className="text-sm leading-5 text-base-foreground">
                      {gateModel.ids[0]}
                    </p>
                  </>
                ) : (
                  <ul className="flex flex-col gap-1">
                    {gateModel.ids.map((id) => (
                      <li key={id} className="truncate text-sm leading-5 text-base-foreground">
                        {id}
                      </li>
                    ))}
                  </ul>
                )}
              </InfoRow>
            )}
          </div>

          {/* Only while Gate is the source, for the same reason as the row
            * above. A balance is what the Gate branch spends: under App default
            * this app sends Gate nothing to bill, so naming the balance here
            * describes a relationship it is not in, and "Add credits" /
            * "Manage billing" are actions on an account it is not using. The
            * card was drawing all three unconditionally, directly beneath the
            * radio that had just said Gate is not serving this app. */}
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
                  {plan} plan
                </p>
              )}
              <p className="text-sm leading-5 text-base-foreground">
                <span className="text-neutral-600">Gate credits: </span>
                {credits ?? "N/A"}
              </p>
            </InfoRow>
          </div>
        </>
      )}

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
      className={`flex items-center gap-3 rounded-control border p-3 text-left transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary disabled:cursor-not-allowed disabled:opacity-60 ${
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
    <div className="flex items-center gap-3 rounded-control border border-base-border p-3"
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
  unattributed,
  onLoadMore,
}: {
  activity: ActivityEntry[];
  /** The first page is in flight; see `AppPane`. */
  pending?: boolean;
  /** No feed was read at all; see `AppPane`. */
  unavailable?: boolean;
  /** No feed can exist for this surface; see `AppPane`. */
  unattributed?: boolean;
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
      ) : unattributed ? (
        // Ahead of the rows, not inside the empty arm. `unattributed` says
        // nothing here is attributable to this app, which is a claim about the
        // whole card - so a non-empty `activity` does not get to outvote it, and
        // nesting this under `activity.length === 0` let it. `MessagesChart`
        // resolves it the same way, its `unattributed` arm ahead of the bars,
        // and this is the same invariant one card over: the flag belongs to the
        // component that owns the reading, not to the caller's luck about
        // whether the list came back empty.
        //
        // Ahead of `unavailable` too, for the reason the chart gives: no read
        // was attempted, so none can have failed.
        <EmptyNote>Recent activity isn&apos;t attributed to this app</EmptyNote>
      ) : unavailable ? (
        <EmptyNote>Recent activity couldn&apos;t be read</EmptyNote>
      ) : activity.length === 0 ? (
        // Deliberately NOT the chart's "in the last 24hrs". The entries outlive
        // the window they were sent in - the feed keeps the last messages even
        // when they are a day or more old, because what the user came here for is
        // what this app last did - so borrowing the chart's sentence would state a
        // window this card does not use. It also put the same line twice on a pane
        // with no traffic, which is how the inaccuracy came to light.
        <EmptyNote>No recent messages</EmptyNote>
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
                {/* Type. The frame draws a 20px glyph 8px from the label, and the
                  two take DIFFERENT inks: the label is `base/foreground`, the
                  glyph is its category's colour (see `categoryTone`).
                  This comment used to claim both were `base/foreground`, citing
                  "the downloaded asset's own stroke is #030712" - which is what
                  an *export* carries, not what the frame renders. Sampling the
                  asset instead of the frame is how one ink ended up on five
                  categories.
                  Spelled as the gateway spelled it, like `SecurityEvents` does
                  with the same field: a display vocabulary for values only the
                  gateway knows would be invented here. */}
                <td className="py-[1.125rem] pr-4">
                  {entry.category ? (
                    <span
                      className="flex items-center gap-2 text-sm leading-5 text-base-foreground"
                      // Only "Regular" carries one; see `ActivityEntry.categoryTitle`.
                      title={entry.categoryTitle ?? undefined}
                    >
                      {entry.categoryIcon && (
                        // Coloured per category (`661:16450`), not inked from
                        // the span: the frame draws Injection red, PII green
                        // and Credential purple, and one ink for all of them
                        // was what this column shipped. See `categoryTone`.
                        <Icon
                          name={entry.categoryIcon}
                          size={20}
                          className={`shrink-0 ${categoryTone(entry.category)}`}
                        />
                      )}
                      <span className="truncate">{entry.category}</span>
                    </span>
                  ) : (
                    // Reached only when the gateway recorded no security action
                    // at all - a row it did not examine, or one that is not this
                    // caller's to see into. A request that WAS examined and
                    // matched nothing says "Regular" instead (`toolEvents.ts`);
                    // this is the case where we genuinely have no reading.
                    <span
                      className="text-sm leading-5 text-base-muted-foreground"
                      title="No guardrail category recorded, or not your request"
                    >
                      -
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
                      -
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
                    <VendorMark provider={entry.provider} vendor={entry.vendor} />
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

