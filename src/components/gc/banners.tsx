import { useEffect, useState } from "react";
import { BaseSwitch, StatusTile } from "./base";
import { Icon } from "./Icon";
import { routingState, showsFraction } from "../../lib/routingState";

/**
 * The banner stack that sits between the topbar and the content pane
 * (`banner/update`, `banner/routing`, `banner/partly-routing`,
 * `banner/alert/*`). Update and routing banners are full-bleed 1024px strips
 * with a hairline bottom border; the alert is an inset card.
 *
 * The component frames are live on the **Banners** canvas (`744:37738`) -
 * `banner/update` `744:37750`, `banner/routing` `744:37758`,
 * `banner/partly-routing` `744:37766`, the alert rows `744:37774` and
 * `744:37789`. An earlier note here said the Components page had been emptied
 * and these were gone; only the OLD page (`113:16762`) is empty, and believing
 * otherwise is what put a generalised icon step in `StatusTile`.
 *
 * All presentational - the shell owns dismissal and retry state.
 */

/**
 * Navy strip offering an available update. The fill is horizontal now
 * (228:85974, read 2026-08-28): blue-ribbon 800 to 900 right-to-left at 50%
 * over solid 900, replacing the old vertical pair. The design's dot matrix is
 * approximated here as a CSS radial-gradient rather than shipping the Figma
 * raster.
 *
 * **The dots are correct - do not remove them.** `dot-matrix-light` reads as
 * an EMPTY frame over MCP, and rendering both the component and the instance
 * (`228:85974`) at 1:1 and sampling the pixels finds a flat gradient with no
 * periodic variation. An audit on 2026-09-03 concluded on that evidence that
 * the pattern might be ours alone. It is not: the designer confirmed the
 * pattern is visible in Figma (2026-09-04). Whatever carries it does not
 * survive the export, so the file cannot be used to check this one - which is
 * exactly why it is written down here.
 */
export function UpdateBanner({
  version,
  onUpdate,
  onDismiss,
}: {
  version: string;
  onUpdate: () => void;
  onDismiss: () => void;
}) {
  return (
    <div className="relative flex h-12 w-full items-center justify-between border-b border-base-border bg-blue-ribbon-900 bg-gradient-to-l from-blue-ribbon-800/50 to-blue-ribbon-900/50 px-4">
      <span
        aria-hidden
        className="pointer-events-none absolute inset-0"
        style={{
          backgroundImage:
            "radial-gradient(circle, rgba(255,255,255,0.16) 1px, transparent 1px)",
          backgroundSize: "8px 8px",
        }}
      />
      <p className="relative text-sm leading-5 text-white [text-shadow:0_1px_0_rgba(0,0,0,0.05)]">
        <span className="font-medium tracking-label-14">Update available</span>{" "}
        {/* One mono run at 400, dash included, matching the design's single
         * `- v0.5.0` text node. */}
        <span className="font-mono">- {version}</span>
      </p>
      <div className="relative flex items-center gap-4">
        <button
          type="button"
          onClick={onUpdate}
          className="flex h-6 items-center rounded-control border border-base-input bg-base-card px-2.5 py-1 text-base-xs font-medium leading-4 tracking-button-xs text-base-primary shadow-base-btn-xs transition-colors hover:bg-neutral-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white"
        >
          Update
        </button>
        <button
          type="button"
          onClick={onDismiss}
          aria-label="Dismiss update notice"
          className="-m-1 rounded-sm p-1 text-base-primary-foreground transition-colors hover:bg-white/10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white"
        >
          <Icon name="x" size={16} />
        </button>
      </div>
    </div>
  );
}

/**
 * Standing summary of how much of the user's traffic is actually routed.
 *
 * Everything protected reads as "protecting you" in green; anything short of
 * that is amber. The Figma mocks label a 2-of-4 state "protecting you" and a
 * 0-of-4 state "partly routed", which cannot both be right - only two variants
 * are drawn, so the all-or-not-all split is the reading that leaves no state
 * unrepresented.
 *
 * **Which state, and what to call it, is `lib/routingState`'s** since AG-913.
 * The tray's routing card answers the same question and used to answer it in
 * different words, so the vocabulary is shared rather than written twice. Two
 * distinctions live there that this banner used to lose:
 *
 * - **An empty denominator is not a gap.** `totalCount` is what the user asked
 *   for, so zero means they asked for nothing - and "partly routing your apps ·
 *   Partly routed · 0 of 0 Apps" reported a gap where there is none, in three
 *   ways at once. Since the denominator became intent this is reachable by the
 *   master switch rather than only on a machine with no tools.
 * - **None of three is not partly.** That case used to fall into the partly
 *   branch and read "partly routing your apps" over `0 of 3`.
 *
 * The tile stays amber for all three unhappy states because there is no third
 * tone drawn and picking one by eye is the thing this repo is told not to do -
 * see question 23 in `docs/figma-questions-for-design.md`. The words are the
 * part that was making a false claim, so the words are the part that changed.
 */
export function RoutingBanner({
  protectedCount,
  totalCount,
  availableCount,
}: {
  /** Routed, among the apps the user switched on. Judges the tone. */
  protectedCount: number;
  /** Switched on. The denominator the TONE is judged against, and the
   *  numerator the fraction prints. */
  totalCount: number;
  /** Every app on the rail, switched on or not. The fraction's denominator -
   *  see `showsFraction`. */
  availableCount: number;
}) {
  // One vocabulary with the tray's routing card (AG-913). The fourth state is
  // new here: "you asked for three and none are routed" used to fall into the
  // `partly` branch and read "partly routing your apps" over `0 of 3`.
  const state = routingState(protectedCount, totalCount);

  return (
    <div className="flex h-12 w-full items-center justify-between border-b border-base-border bg-base-card px-4 py-2">
      <div className="flex items-center gap-3">
        {/* 32px tile with a 16px glyph - `banner/status-protected`'s
         * icon-wrapper (228:85985), between the tile's other two sizes. */}
        <StatusTile tone={state.tone} icon={state.icon} size={32} />
        <p className="text-sm font-medium leading-5 text-base-foreground">
          {state.headline}
        </p>
      </div>
      <p className="text-sm leading-5 tracking-label-14">
        <span
          className={`font-medium ${state.tone === "green" ? "text-green-600" : "text-amber-600"}`}
        >
          {/* "Routed", not "Routing": every routed frame on Flows/Overview reads
            * `Routed · 4 of 4 Apps` (re-read 2026-08-21). */}
          {state.label}
        </span>
        {/* Both greys are the drawn `base/muted-foreground` (228:85990) - the
          * separator is that list's own disc marker, same colour as its text. */}
        {/* Switched on, out of every app on the rail - coverage, not outcome.
          * The frame draws `Routed · 4 of 4 Apps`, where both halves were
          * routed-of-requested and the ratio restated the pill beside it.
          * Changed on request 2026-09-23, so it deviates from the frame and is
          * owed to design, along with the fact that it does not match the
          * rail's group counters on the numerator. `routingState` carries both.
          * "on" is load-bearing in the words: without it "2 of 8 Apps" beside a
          * "Routed" pill reads as two routed out of eight. */}
        {showsFraction(availableCount) && (
          <>
            <span className="text-base-muted-foreground"> · </span>
            <span className="text-base-muted-foreground">
              {totalCount} of {availableCount} Apps on
            </span>
          </>
        )}
      </p>
    </div>
  );
}

/**
 * Amber card raised when an app stopped being routed without the user doing it
 * (Figma `banner/alert/single-app`). When several apps are affected the card
 * pages between them, so it also takes prev/next controls that straddle the
 * card's edges.
 */
/**
 * A tool whose config is right and whose running process has not picked it up.
 *
 * The pane's version of the sidebar's "Reopen to finish": the row has 250px and
 * prints the phrase plus the program's name, and this has the width for the part
 * that matters, which is *which route the traffic is on right now*. AG-570 asks for
 * the route in use, the requested route, and the action, and the first two are
 * the whole reason this is a card rather than a sentence - "reopen required"
 * without them does not say what reopening would change.
 *
 * No switch, unlike `AlertBanner`. Nothing here is a setting: the configuration
 * already says what the user asked for, and the only thing left is a process
 * that has to end. Offering a switch would invite them to toggle routing to fix
 * a problem toggling routing causes.
 */
export function ReopenAlert({
  name,
  routeInUse,
  requestedRoute,
  onReopen,
}: {
  name: string;
  /** Where the traffic is going now. Omitted when the backend could not say,
   *  and the card degrades to naming the action rather than inventing a route -
   *  a guessed endpoint here would be a claim about the user's traffic. */
  routeInUse?: string | null;
  requestedRoute?: string | null;
  onReopen: () => void;
}) {
  return (
    // `role="status"`, which a pane card does not normally take: this one is
    // raised by a background sweep rather than by anything the user just did,
    // and the shell banner that used to carry the same fact announced it. With
    // that banner gone this is the window's only voice for it, so a card
    // appearing under a reader's cursor says so instead of arriving in silence.
    <div
      role="status"
      className="flex items-center gap-6 rounded-control border border-amber-300 bg-amber-50 py-4 pl-4 pr-5"
    >
      <div className="flex min-w-0 flex-1 items-center gap-4">
        <StatusTile tone="amber" icon="refresh" size={36} />
        <div className="min-w-0">
          <p className="text-sm font-medium leading-5 text-base-foreground">
            Reopen {name} to finish
          </p>
          <p className="text-base-xs leading-4 text-gray-600">
            It was already running when its configuration changed, so it is still
            using the route it started with.
          </p>
          {routeInUse && requestedRoute && (
            // Sans, weighted rather than set in mono: identifier *values* are
            // sans here (design, 2026-09-04), and an endpoint is an identifier
            // rather than machine output. This card set them in mono until that
            // question came back answered.
            <p className="mt-1 break-all text-base-xs leading-4 text-gray-600">
              In use:{" "}
              <span className="font-medium text-base-foreground">{routeInUse}</span>
              {" · "}
              Requested:{" "}
              <span className="font-medium text-base-foreground">{requestedRoute}</span>
            </p>
          )}
        </div>
      </div>
      <button
        type="button"
        onClick={onReopen}
        className="shrink-0 rounded-control border border-base-border bg-base-card px-3 py-2 text-base-xs font-medium leading-4 text-base-foreground shadow-base-btn-sm transition-colors hover:bg-neutral-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
      >
        {/* "Close", not "Reopen". The button opens the close confirmation,
            because a CLI is a shell session Gate does not own and cannot start
            again - `RunningAgent.can_reopen` is false for every one of them.
            Labelling it "Reopen Claude Code" promised the one thing this flow
            never does. The heading above still says "Reopen ... to finish",
            which is the true sentence: Gate closes it, the user opens it. */}
        Close tool
      </button>
    </div>
  );
}

/**
 * The shell-width counterpart of [`PaneNote`]: advice that is about the machine
 * rather than about the pane that happens to be open, dismissible because the
 * user is the only one who knows when they have acted on it.
 *
 * Neutral, for the reason `PaneNote` gives below and one more. The three banners
 * above it are amber or red and each names something the shell is waiting to
 * have fixed; this one is told once, on a transition, and there is nothing here
 * for Gate to re-check afterwards. Drawn in that palette it would read as a
 * fourth fault, and it would sit above a rail whose rows all say Protected.
 *
 * Not in the Figma. The file draws `banner/update`, `banner/routing`,
 * `banner/partly-routing` and the alert rows, and nothing neutral at this width,
 * so the frame geometry is borrowed from the routing banner (full-bleed strip,
 * hairline bottom border, 16/12 padding, a 16px tile beside a two-line stack)
 * with `base/*` inks in place of the amber.
 */
export function NoteBanner({
  title,
  body,
  onDismiss,
}: {
  title: string;
  body: string;
  onDismiss: () => void;
}) {
  return (
    <div
      role="status"
      className="w-full border-b border-base-border bg-base-card px-4 py-3"
    >
      <div className="flex w-full items-start gap-3">
        <Icon
          name="info"
          size={16}
          className="mt-0.5 shrink-0 text-neutral-500"
        />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium leading-5 text-base-foreground">
            {title}
          </p>
          <p className="text-base-xs leading-4 text-neutral-600">{body}</p>
        </div>
        <button
          type="button"
          onClick={onDismiss}
          aria-label="Dismiss"
          className="shrink-0 text-neutral-500 transition-colors hover:text-base-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
        >
          <Icon name="x" size={16} />
        </button>
      </div>
    </div>
  );
}

export function AlertBanner({
  title,
  body,
  on,
  switchLabel,
  busy,
  onToggle,
  onDismiss,
  paging,
}: {
  title: string;
  body: string;
  on: boolean;
  /** Accessible name for the switch, which the visible title does not supply. */
  switchLabel: string;
  /** The action this banner offers is in flight. Threaded to the switch so the
   *  remedy cannot be started twice while the first attempt is still writing. */
  busy?: boolean;
  onToggle: () => void;
  onDismiss: () => void;
  /** Present only in the multiple-apps variant. */
  paging?: { onPrev: () => void; onNext: () => void };
}) {
  return (
    <div className="relative flex items-center gap-6 rounded-md border border-amber-300 bg-amber-50 py-4 pl-4 pr-5">
      <div className="flex min-w-0 flex-1 items-center gap-4">
        <StatusTile tone="amber" icon="triangleAlert" size={36} />
        <div className="min-w-0">
          <p className="text-sm font-medium leading-5 text-base-foreground">{title}</p>
          <p className="text-base-xs leading-4 tracking-label-12 text-gray-600">{body}</p>
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-3">
        <BaseSwitch on={on} label={switchLabel} busy={busy} onClick={onToggle} />
        <button
          type="button"
          onClick={onDismiss}
          aria-label="Dismiss alert"
          className="text-base-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
        >
          <Icon name="x" size={20} />
        </button>
      </div>

      {paging && (
        <>
          <PageButton side="prev" onClick={paging.onPrev} />
          <PageButton side="next" onClick={paging.onNext} />
        </>
      )}
    </div>
  );
}

function PageButton({ side, onClick }: { side: "prev" | "next"; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={side === "prev" ? "Previous app" : "Next app"}
      className={`absolute top-1/2 flex size-5 -translate-y-1/2 items-center justify-center rounded-full border border-base-input bg-base-card text-neutral-600 shadow-base-xs transition-colors hover:text-base-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
        side === "prev" ? "-left-2.5" : "-right-2.5"
      }`}
    >
      <Icon name={side === "prev" ? "chevronLeft" : "chevronRight"} size={12} />
    </button>
  );
}

/**
 * The underlying message, behind an expander, with a copy button.
 *
 * Two of `classifyError`'s hints end on "the details below help when reporting
 * it", and one of them is the catch-all fallback - so every context without a
 * branch of its own lands on copy that promises something below. That makes
 * this the surface for any failure nobody classified. `ErrorBanner` grew it
 * first and the setup screen still had none, which meant a first-run or
 * re-sign-in failure - the one with no shell behind it - read as a dead end.
 * Shared rather than copied so the next surface cannot forget it again.
 *
 * Renders nothing when `raw` is absent or merely repeats the title, which is
 * the same guard the popover's `ErrorNote` makes.
 */
export function ErrorDetails({ raw, title }: { raw?: string; title: string }) {
  const [copied, setCopied] = useState(false);
  // Timed reset with a cleanup rather than a bare setTimeout: the surface is
  // dismissed by whatever the user does next, and a pending timer would then
  // set state on an unmounted component.
  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => setCopied(false), 1600);
    return () => clearTimeout(t);
  }, [copied]);

  if (!raw || raw === title) return null;

  return (
    <details className="mt-1">
      <summary className="cursor-pointer py-0.5 text-base-2xs text-red-900/70">
        Details
      </summary>
      <p className="mt-1 break-all font-mono text-base-2xs leading-4 text-red-900/80">
        {raw}
      </p>
      <button
        type="button"
        onClick={() => {
          void navigator.clipboard.writeText(raw).then(() => setCopied(true));
        }}
        className="mt-1.5 inline-flex items-center gap-1 text-base-2xs font-medium text-red-900/70 transition-colors hover:text-red-900 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-red-600"
      >
        <Icon name={copied ? "check" : "copy"} size={12} />
        {copied ? "Copied" : "Copy details"}
      </button>
    </details>
  );
}

/**
 * A failed action, stated where the user acted. Deliberately not `AlertBanner`:
 * that one carries a switch because a drifted app can be re-routed from it, and
 * a failure has nothing to toggle.
 */
export function ErrorBanner({
  title,
  hint,
  raw,
  onDismiss,
}: {
  title: string;
  hint: string;
  /** The underlying message; see `ErrorDetails`. */
  raw?: string;
  onDismiss: () => void;
}) {
  return (
    <div
      role="alert"
      className="flex w-full items-start gap-3 border-b border-red-200 bg-red-50 px-4 py-3"
    >
      <StatusTile tone="red" icon="triangleAlert" />
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium leading-5 text-red-900">{title}</p>
        <p className="text-base-xs leading-4 text-red-900/80">{hint}</p>
        <ErrorDetails raw={raw} title={title} />
      </div>
      <button
        type="button"
        onClick={onDismiss}
        aria-label="Dismiss error"
        className="shrink-0 text-red-900/70 transition-colors hover:text-red-900 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-red-600"
      >
        <Icon name="x" size={16} />
      </button>
    </div>
  );
}
