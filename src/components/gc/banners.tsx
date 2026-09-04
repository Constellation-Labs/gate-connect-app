import { useEffect, useState } from "react";
import type { RecoveryNextStep } from "../../lib/api";
import type { RecoveryRow } from "../../lib/recovery";
import { BaseSwitch, StatusTile } from "./base";
import { Icon } from "./Icon";

/**
 * The banner stack that sits between the topbar and the content pane
 * (`banner/update`, `banner/routing`, `banner/partly-routing`,
 * `banner/alert/*`). Update and routing banners are full-bleed 1024px strips
 * with a hairline bottom border; the alert is an inset card.
 *
 * Those component frames lived on the Components page, which the file has since
 * emptied; the live sources are the banner instances inside the flow frames,
 * which is where the 2026-08-28 re-validation read them.
 *
 * All presentational - the shell owns dismissal and retry state.
 */

/**
 * Navy strip offering an available update. The fill is horizontal now
 * (228:85974, read 2026-08-28): blue-ribbon 800 to 900 right-to-left at 50%
 * over solid 900, replacing the old vertical pair. The design's dot matrix is
 * approximated here as a CSS radial-gradient rather than shipping the Figma
 * raster.
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
        <span className="font-medium">Update available</span>{" "}
        {/* One mono run at 400, dash included, matching the design's single
         * `- v0.5.0` text node. */}
        <span className="font-mono">- {version}</span>
      </p>
      <div className="relative flex items-center gap-4">
        <button
          type="button"
          onClick={onUpdate}
          className="flex h-6 items-center rounded-control border border-base-input bg-base-card px-2.5 py-1 text-base-xs font-medium leading-4 tracking-button-xs text-base-primary shadow-base-btn-sm transition-colors hover:bg-neutral-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white"
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
 * that reads as "partly routing" in amber. The Figma mocks label a 2-of-4 state
 * "protecting you" and a 0-of-4 state "partly routed", which cannot both be
 * right - only two variants are drawn, so the all-or-not-all split is the
 * reading that leaves no state unrepresented.
 */
export function RoutingBanner({
  protectedCount,
  totalCount,
}: {
  protectedCount: number;
  totalCount: number;
}) {
  const allProtected = totalCount > 0 && protectedCount === totalCount;

  return (
    <div className="flex h-12 w-full items-center justify-between border-b border-base-border bg-base-card px-4 py-2">
      <div className="flex items-center gap-3">
        {/* 32px tile with a 16px glyph - `banner/status-protected`'s
         * icon-wrapper (228:85985), between the tile's other two sizes. */}
        <StatusTile
          tone={allProtected ? "green" : "amber"}
          icon={allProtected ? "shieldCheck" : "shieldBan"}
          size={32}
        />
        <p className="text-sm font-medium leading-5 text-base-foreground">
          {allProtected
            ? "Gate Connect is protecting you"
            : "Gate Connect is partly routing your apps"}
        </p>
      </div>
      <p className="text-sm leading-5">
        <span
          className={`font-medium ${allProtected ? "text-green-600" : "text-amber-600"}`}
        >
          {/* "Routed", not "Routing": every routed frame on Flows/Overview reads
            * `Routed · 4 of 4 Apps` (re-read 2026-08-21). */}
          {allProtected ? "Routed" : "Partly routed"}
        </span>
        {/* Both greys are the drawn `base/muted-foreground` (228:85990) - the
          * separator is that list's own disc marker, same colour as its text. */}
        <span className="text-base-muted-foreground"> · </span>
        <span className="text-base-muted-foreground">
          {protectedCount} of {totalCount} Apps
        </span>
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
 * The pane's version of the sidebar's "Not protected - Reopen required": the row
 * has 250px and prints the phrase, and this has the width for the part that
 * matters, which is *which route the traffic is on right now*. AG-570 asks for
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
    <div className="flex items-center gap-6 rounded-control border border-amber-300 bg-amber-50 py-4 pl-4 pr-5">
      <div className="flex min-w-0 flex-1 items-center gap-4">
        <StatusTile tone="amber" icon="refresh" size={36} />
        <div className="min-w-0">
          <p className="text-sm font-medium leading-5 text-base-foreground">
            Reopen {name} to apply its route
          </p>
          <p className="text-base-xs leading-4 text-gray-600">
            It was already running when its configuration changed, so it is still
            using the route it started with.
          </p>
          {routeInUse && requestedRoute && (
            // Mono, because both are endpoints - identity, not prose.
            <p className="mt-1 text-base-xs leading-4 text-gray-600">
              In use: <span className="font-mono text-base-foreground">{routeInUse}</span>
              {" · "}
              Requested:{" "}
              <span className="font-mono text-base-foreground">{requestedRoute}</span>
            </p>
          )}
        </div>
      </div>
      <button
        type="button"
        onClick={onReopen}
        className="shrink-0 rounded-control border border-base-border bg-base-card px-3 py-2 text-base-xs font-medium leading-4 text-base-foreground shadow-base-btn-sm transition-colors hover:bg-neutral-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
      >
        Reopen tool
      </button>
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
    <div className="relative flex items-center gap-6 rounded-control border border-amber-300 bg-amber-50 py-4 pl-4 pr-5">
      <div className="flex min-w-0 flex-1 items-center gap-4">
        <StatusTile tone="amber" icon="triangleAlert" size={36} />
        <div className="min-w-0">
          <p className="text-sm font-medium leading-5 text-base-foreground">{title}</p>
          <p className="text-base-xs leading-4 text-gray-600">{body}</p>
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-3">
        <BaseSwitch on={on} label={switchLabel} busy={busy} onClick={onToggle} />
        <button
          type="button"
          onClick={onDismiss}
          aria-label="Dismiss alert"
          className="text-neutral-500 transition-colors hover:text-base-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
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

/**
 * Routing did not fully come back, and Gate knows which parts.
 *
 * The provider snapshots have always recorded unfinished work - `restore_all`
 * keeps failures in the file and clears it only once everything is back - but
 * nothing read them for display. So a half-finished restore left some tools
 * routing and some not, with no statement anywhere that Gate was aware of it.
 * That is the state this names.
 *
 * **Amber, not red.** Nothing is broken beyond repair and nothing was lost: the
 * work is recorded and resuming retries exactly what failed. Red would be
 * claiming worse than is true.
 *
 * Dismissing is session-only on purpose. The pending state lives on disk, so the
 * notice returns on the next launch or refresh until the work actually finishes -
 * which is what AG-570 asks for when it says the recovery action persists.
 *
 * Provisional layout: the Figma draws no recovery notice (AG-569 is To Do).
 */
export function RecoveryBanner({
  names,
  rows,
  progress,
  busy,
  onResume,
  onAction,
  onReviewDetails,
  onFinishLater,
}: {
  /** What is still outstanding, providers and tools together - the user does not
   * care which snapshot an entry came from. Never empty; the shell omits the
   * banner instead. */
  names: string[];
  /** Every tool the interrupted operation touched, resolved and unresolved
   * alike. Omitted while the summary is still loading, and the per-tool section
   * is omitted with it: a row list that renders before the readings land would
   * show every tool as "Never checked", which is a claim.
   *
   * The resolved ones are listed too, on purpose. AG-570 asks the summary to
   * account for every tool, and "Codex: configuration written, routing" is the
   * half of the picture that tells the user what they are *not* being asked to
   * fix. */
  rows?: RecoveryRow[];
  /** Which slug is being worked on right now, and what has been attempted this
   * pass. The notice drives the entries one at a time so it can say so - a
   * single "Resuming…" over the whole set cannot answer "which tool is it on",
   * which is the question a user watching a stuck resume actually has. */
  progress?: { active: string | null; done: string[] };
  busy?: boolean;
  onResume: () => void;
  /** Act on one row. The step is passed back rather than resolved here because
   * only one of the three - `retry` - belongs to this notice; the shell decides
   * where the other two go. Absent leaves the rows read-only. */
  onAction?: (slug: string, step: RecoveryNextStep) => void;
  /** Opens the read-only account of what the operation did. Omitted when there is
   * no journal to show - an operation interrupted before it wrote one leaves the
   * snapshots but no explanation, and a button onto an empty dialog is worse than
   * no button. */
  onReviewDetails?: () => void;
  onFinishLater: () => void;
}) {
  const many = names.length > 1;
  const [open, setOpen] = useState(false);
  return (
    <div
      role="status"
      className="w-full border-b border-amber-200 bg-amber-50 px-4 py-3"
    >
      <div className="flex w-full items-start gap-3">
        <StatusTile tone="amber" icon="refresh" />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium leading-5 text-amber-900">
            Routing didn’t finish coming back
          </p>
          <p className="text-base-xs leading-4 text-amber-900/80">
            {names.join(", ")} {many ? "are" : "is"} still waiting. Gate recorded what
            was left, so resuming picks up where it stopped.
          </p>
        </div>
        {rows && rows.length > 0 && (
          <button
            type="button"
            onClick={() => setOpen((v) => !v)}
            aria-expanded={open}
            className="flex shrink-0 items-center gap-1 rounded-sm px-2 py-1 text-base-xs font-medium text-amber-900 transition-colors hover:bg-amber-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-amber-600"
          >
            {open ? "Hide tools" : `Show tools (${rows.length})`}
            <Icon name={open ? "chevronDown" : "chevronRight"} size={14} />
          </button>
        )}
        {onReviewDetails && (
          <button
            type="button"
            onClick={onReviewDetails}
            className="shrink-0 rounded-sm px-2 py-1 text-base-xs font-medium text-amber-900 underline decoration-amber-300 transition-colors hover:bg-amber-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-amber-600"
          >
            Review details
          </button>
        )}
        <button
          type="button"
          onClick={onResume}
          disabled={busy}
          className="shrink-0 rounded-sm border border-amber-300 bg-base-card px-2 py-1 text-base-xs font-medium text-amber-900 shadow-base-2xs transition-colors hover:bg-amber-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-amber-600 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {busy ? "Resuming…" : "Resume now"}
        </button>
        <button
          type="button"
          onClick={onFinishLater}
          disabled={busy}
          aria-label="Finish later"
          className="shrink-0 text-amber-900/70 transition-colors hover:text-amber-900 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-amber-600 disabled:opacity-50"
        >
          <Icon name="x" size={16} />
        </button>
      </div>

      {open && rows && (
        <ul className="mt-3 flex flex-col gap-1 border-t border-amber-200 pt-2">
          {rows.map((row) => (
            <RecoveryRowLine
              key={`${row.kind}:${row.slug}`}
              row={row}
              progress={progress}
              busy={busy}
              onAction={onAction}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * One tool inside the notice: what its write reached, what the last check saw,
 * and the one thing left to do about it.
 *
 * The progress words outrank the recorded stage while a resume is running, and
 * only then. A row that has just been attempted says so from `progress.done`
 * rather than from its stage, because the stage the caller is holding was read
 * before the attempt - re-reading the summary between every entry would make the
 * whole list flicker to catch one line up.
 */
function RecoveryRowLine({
  row,
  progress,
  busy,
  onAction,
}: {
  row: RecoveryRow;
  progress?: { active: string | null; done: string[] };
  busy?: boolean;
  onAction?: (slug: string, step: RecoveryNextStep) => void;
}) {
  const active = progress?.active === row.slug;
  const attempted = progress?.done.includes(row.slug) ?? false;
  const stage = active
    ? "Working on it now…"
    : attempted
      ? "Just attempted"
      : row.stageLine;
  return (
    <li className="flex items-start gap-2 py-1">
      <span className="mt-0.5 shrink-0">
        <Icon
          name={active ? "refresh" : row.stageComplete ? "circleCheck" : "info"}
          size={14}
          className={
            active
              ? "animate-spin text-amber-700"
              : row.stageComplete
                ? "text-green-700"
                : "text-amber-700"
          }
        />
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex flex-wrap items-baseline gap-x-2">
          <span className="text-base-xs font-medium leading-4 text-amber-900">
            {row.name}
          </span>
          <span className="text-base-xs leading-4 text-amber-900/80">{stage}</span>
        </span>
        {/* The three readings the stage cannot carry. Printed on one line
         * because they are read together - a stale process under a written
         * config is the case where "the write worked" and "it is not routing"
         * are both true, and splitting them across lines hides the pairing. */}
        <span className="block text-base-xs leading-4 text-amber-900/70">
          Last verified: {row.lastVerified ?? "no reading yet"} · Check:{" "}
          {row.checkResult} · {row.runningState}
        </span>
      </span>
      {row.action && onAction && (
        <button
          type="button"
          onClick={() => onAction(row.slug, row.nextStep)}
          // Sign-in is a different screen, so the row names the step and offers
          // no control for it rather than a button that leaves the flow. Retry
          // and reopen both act.
          disabled={busy || row.nextStep === "sign_in"}
          title={row.nextStep === "sign_in" ? `${row.action} to finish this one` : undefined}
          className="shrink-0 rounded-sm border border-amber-300 bg-base-card px-2 py-0.5 text-base-xs font-medium text-amber-900 shadow-base-2xs transition-colors hover:bg-amber-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-amber-600 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {row.action}
        </button>
      )}
    </li>
  );
}
