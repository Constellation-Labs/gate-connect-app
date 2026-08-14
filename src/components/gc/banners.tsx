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
