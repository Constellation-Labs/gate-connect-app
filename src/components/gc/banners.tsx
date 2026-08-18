import { BaseSwitch, StatusTile } from "./base";
import { Icon } from "./Icon";

/**
 * The banner stack that sits between the topbar and the content pane
 * (Figma `banner/update`, `banner/routing`, `banner/partly-routing`,
 * `banner/alert/*`). Update and routing banners are full-bleed 1024px strips
 * with a hairline bottom border; the alert is an inset card.
 *
 * All presentational - the shell owns dismissal and retry state.
 */

/**
 * Navy strip offering an available update. Fill is a vertical gradient from
 * blue-ribbon 800 to 900, overlaid with the design's dot matrix - approximated
 * here as a CSS radial-gradient rather than shipping the Figma raster.
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
    <div className="relative flex h-12 w-full items-center justify-between border-b border-black/20 bg-gradient-to-b from-blue-ribbon-800 to-blue-ribbon-900 px-4">
      <span
        aria-hidden
        className="pointer-events-none absolute inset-0"
        style={{
          backgroundImage:
            "radial-gradient(circle, rgba(255,255,255,0.16) 1px, transparent 1px)",
          backgroundSize: "8px 8px",
        }}
      />
      <p className="relative text-sm font-medium leading-5 text-white">
        Update available <span className="text-white/50">-</span>{" "}
        <span className="font-mono text-white/80">{version}</span>
      </p>
      <div className="relative flex items-center gap-3">
        <button
          type="button"
          onClick={onUpdate}
          className="flex h-6 items-center rounded-base border border-white/40 px-2 text-base-xs font-medium leading-4 text-white transition-colors hover:bg-white/10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white"
        >
          Update
        </button>
        <button
          type="button"
          onClick={onDismiss}
          aria-label="Dismiss update notice"
          className="text-white/80 transition-colors hover:text-white focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white"
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
        <StatusTile
          tone={allProtected ? "green" : "amber"}
          icon={allProtected ? "shieldCheck" : "shieldBan"}
        />
        <p className="text-sm font-medium leading-5 text-neutral-900">
          {allProtected
            ? "Gate Connect is protecting you"
            : "Gate Connect is partly routing your apps"}
        </p>
      </div>
      <p className="text-sm leading-5">
        <span
          className={`font-medium ${allProtected ? "text-green-600" : "text-amber-600"}`}
        >
          {allProtected ? "Routing" : "Partly routed"}
        </span>
        <span className="text-neutral-400"> · </span>
        <span className="text-neutral-600">
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
export function AlertBanner({
  title,
  body,
  on,
  switchLabel,
  onToggle,
  onDismiss,
  paging,
}: {
  title: string;
  body: string;
  on: boolean;
  /** Accessible name for the switch, which the visible title does not supply. */
  switchLabel: string;
  onToggle: () => void;
  onDismiss: () => void;
  /** Present only in the multiple-apps variant. */
  paging?: { onPrev: () => void; onNext: () => void };
}) {
  return (
    <div className="relative flex items-center gap-6 rounded-lg border border-amber-300 bg-amber-50 py-4 pl-4 pr-5">
      <div className="flex min-w-0 flex-1 items-center gap-4">
        <StatusTile tone="amber" icon="triangleAlert" size={36} />
        <div className="min-w-0">
          <p className="text-sm font-medium leading-5 text-neutral-900">{title}</p>
          <p className="text-base-xs leading-4 text-neutral-600">{body}</p>
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-3">
        <BaseSwitch on={on} label={switchLabel} onClick={onToggle} />
        <button
          type="button"
          onClick={onDismiss}
          aria-label="Dismiss alert"
          className="text-neutral-500 transition-colors hover:text-neutral-900 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
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
      className={`absolute top-1/2 flex size-5 -translate-y-1/2 items-center justify-center rounded-full border border-base-border bg-base-card text-neutral-600 shadow-base-xs transition-colors hover:text-neutral-900 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
        side === "prev" ? "-left-2.5" : "-right-2.5"
      }`}
    >
      <Icon name={side === "prev" ? "chevronLeft" : "chevronRight"} size={12} />
    </button>
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
  onDismiss,
}: {
  title: string;
  hint: string;
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
  busy,
  onResume,
  onReviewDetails,
  onFinishLater,
}: {
  /** What is still outstanding, providers and tools together - the user does not
   * care which snapshot an entry came from. Never empty; the shell omits the
   * banner instead. */
  names: string[];
  busy?: boolean;
  onResume: () => void;
  /** Opens the read-only account of what the restore did. Omitted when there is no
   * journal to show - a restore interrupted before it wrote one leaves the
   * snapshots but no explanation, and a button onto an empty dialog is worse than
   * no button. */
  onReviewDetails?: () => void;
  onFinishLater: () => void;
}) {
  const many = names.length > 1;
  return (
    <div
      role="status"
      className="flex w-full items-start gap-3 border-b border-amber-200 bg-amber-50 px-4 py-3"
    >
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
      {onReviewDetails && (
        <button
          type="button"
          onClick={onReviewDetails}
          className="shrink-0 rounded-base px-2 py-1 text-base-xs font-medium text-amber-900 underline decoration-amber-300 transition-colors hover:bg-amber-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-amber-600"
        >
          Review details
        </button>
      )}
      <button
        type="button"
        onClick={onResume}
        disabled={busy}
        className="shrink-0 rounded-base border border-amber-300 bg-base-card px-2 py-1 text-base-xs font-medium text-amber-900 shadow-base-2xs transition-colors hover:bg-amber-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-amber-600 disabled:cursor-not-allowed disabled:opacity-50"
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
  );
}
