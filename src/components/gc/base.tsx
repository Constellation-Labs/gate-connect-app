import type { CSSProperties, ReactNode } from "react";
import type { ActivitySecurity, ActivityStatus } from "../../lib/toolEventRow";
import type { IconName } from "./Icon";
import { Icon } from "./Icon";

/**
 * Shared primitives for the new app UI (Figma "Gate Connect"). Named for the
 * `base.*` token group in `tailwind.config.ts`, which mirrors the design's own
 * variable names. Distinct from `gc/ui.tsx`, which is the menu-bar popover's
 * primitive set and stays until those screens migrate.
 */

/**
 * The surface every content section sits on (Figma `card/policies` and its
 * siblings): white, 8px radius, a 1px `base/border` hairline and `shadow/sm`.
 * Shared with the Settings pane, which uses the same shape per section.
 */
export function Card({
  children,
  className = "",
  id,
  busy,
}: {
  children: ReactNode;
  /** Layout only, for callers that need to change the internal padding. */
  className?: string;
  /** Anchor, for callers that are a scroll target. */
  id?: string;
  /** The card's contents are being loaded. Announced once here rather than by
   *  each placeholder inside it, which would read as a stream of blanks. */
  busy?: boolean;
}) {
  return (
    <section
      id={id}
      aria-busy={busy || undefined}
      className={`rounded-md border border-base-border bg-base-card shadow-base-sm ${className}`}
    >
      {children}
    </section>
  );
}

/**
 * The 28x28 status chip that fronts the routing banners, and at 36px the alert
 * banner. One shape, two palettes: a vertical 50 -> 200 gradient, a 300 border
 * and a 600 icon, all on Tailwind's default ramps.
 *
 * Measured off the live **Banners** canvas (`744:37738`). An earlier note here
 * said the components had been deleted with the old Components page and read
 * the steps off flow instances instead - only `113:16762` is empty.
 *
 * **The step is per tone, which is why believing the components were gone cost
 * something.** Amber really is 600 (`tailwind colors/amber/600` #D97706, the
 * value the config records as the design's own), and 600 was generalised from
 * it to green. The green component draws **green/700** #15803D.
 *
 * Tone classes are spelled out rather than interpolated - Tailwind only sees
 * literal class names at build time.
 */
const TILE_TONES = {
  green: "from-green-50 to-green-200 border-green-300 text-green-700",
  amber: "from-amber-50 to-amber-200 border-amber-300 text-amber-600",
  // Not in the Figma, which draws no failure state. Follows the same 50 -> 200
  // gradient, 300 border, 600 icon pattern as the two that are.
  red: "from-red-50 to-red-200 border-red-300 text-red-600",
} as const;

export function StatusTile({
  tone,
  icon,
  size = 28,
}: {
  tone: keyof typeof TILE_TONES;
  icon: IconName;
  size?: 28 | 32 | 36;
}) {
  return (
    <span
      aria-hidden
      // Drawn radius is 4px on both banner tiles (228:85985 reads 3.5, the
      // alert's 228:90546 reads 4; both round to `control`, not the 6px `sm`
      // this used to carry).
      className={`flex shrink-0 items-center justify-center rounded-control border bg-gradient-to-b ${TILE_TONES[tone]} ${
        size === 36 ? "size-9" : size === 32 ? "size-8" : "size-7"
      }`}
    >
      <Icon name={icon} size={size === 36 ? 20 : 16} />
    </span>
  );
}

/**
 * 36x20 track, 16px thumb (`Switch` component set, `408:14253`). Geometry
 * differs from the popover's `gc/ui.tsx` Switch (38x22 with a check glyph), so
 * the two coexist until the popover screens migrate; the accessibility
 * contract is carried over unchanged, including the `before:` hit-area
 * expansion that takes the target past 24px without moving the visible track.
 */
export function BaseSwitch({
  on,
  label,
  busy,
  onClick,
}: {
  on: boolean;
  /** Accessible name: what this switch controls. */
  label: string;
  /** An operation is in flight: clicks are ignored but focus is kept. */
  busy?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      aria-busy={busy || undefined}
      aria-disabled={busy || undefined}
      onClick={busy ? undefined : onClick}
      // 36 x 20 with a 16px knob at a 2px inset, straight off the component set
      // and off every instance in `sidebar-menu-item` (434:128).
      //
      // This carried 32 x 17.78 / 14.22 / 1.78 from 2026-08-26 until the
      // 2026-08-30 audit: those numbers were measured off an instance scaled
      // 1.125x, and each one divides back exactly (36/1.125 = 32,
      // 20/1.125 = 17.78, 16/1.125 = 14.22, 2/1.125 = 1.78). The docstring
      // above had said 36x20 all along; only the code disagreed.
      //
      // The off track is `neutral-400` at 50%, which is `custom/outline`
      // (#a3a3a380) to the byte - the design's own value.
      className={`relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors before:absolute before:inset-x-0 before:-inset-y-1 before:content-[''] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
        busy ? "opacity-70" : ""
      } ${on ? "bg-blue-ribbon-700" : "bg-neutral-400/50"}`}
    >
      <span
        className={`absolute size-4 rounded-full bg-base-background shadow-base-lg transition-transform ${
          on ? "translate-x-[18px]" : "translate-x-[2px]"
        }`}
      />
    </button>
  );
}

/**
 * A placeholder for a value that is on its way.
 *
 * AG-576's rule is that a figure on screen has to be a figure something
 * measured. The first paint of the Overview used to break it twice over: an
 * unloaded counter rendered as an em dash, which reads as "we measured nothing",
 * and before that it rendered as `0`, which reads as "you sent nothing". A shape
 * where the number will be makes the only true statement available - this is
 * still coming - and it says it without moving the layout when the number lands.
 *
 * Sized by the caller, in the same box the real content will occupy. Hidden from
 * assistive tech: the container that owns a group of these carries `aria-busy`
 * and says once that it is loading, rather than announcing a row of blanks.
 */
export function Skeleton({
  className = "",
  style,
}: {
  className?: string;
  /** For a dimension no utility class expresses, such as a column height given
   *  as a fraction of the plot area. */
  style?: CSSProperties;
}) {
  return (
    <span
      aria-hidden
      style={style}
      className={`block animate-pulse rounded-sm bg-gray-200 ${className}`}
    />
  );
}

/**
 * What a card says when it has nothing to draw and that is the true answer
 * (Figma 228:89721, the empty Messages chart on the app pane).
 *
 * Distinct from a gap notice, which is what it says when it was not told. The
 * two are one keystroke apart in the markup and worlds apart to the user: one
 * reports on their traffic, the other admits we cannot.
 *
 * The glyph sits in the same 36px bordered tile `StatusTile` draws at its larger
 * size, but flat and neutral rather than a coloured gradient: an empty window is
 * not a warning, and giving it amber would report calm as trouble.
 *
 * Each card keeps its *own* glyph whichever sentence it is showing. An earlier
 * pass put a warning triangle on every "couldn't be read" note, which stacked
 * three of them down a pane whose single cause was already stated once in the
 * notice above - one refused credential reading as three problems. The sentence
 * carries the difference; the notice carries the alarm.
 */
export function EmptyNote({
  children,
  icon = "messageCircleX",
  className = "",
}: {
  children: ReactNode;
  /** The glyph above the sentence. Defaults to the design's own. */
  icon?: IconName;
  /** Spacing from whatever sits above it. The note owns its own padding; this is
   *  for the caller's layout, not its internals. */
  className?: string;
}) {
  return (
    <div className={`flex flex-col items-center gap-3 py-6 ${className}`}>
      <span
        aria-hidden
        className="flex size-9 items-center justify-center rounded-control border border-base-border text-base-muted-foreground"
      >
        <Icon name={icon} size={20} />
      </span>
      <p className="text-center text-base font-medium leading-6 tracking-heading-16 text-base-muted-foreground">
        {children}
      </p>
    </div>
  );
}

/**
 * The small uppercase mono badge the feed tables draw.
 *
 * Lives here rather than in `AppPane` because two surfaces now draw it - the
 * per-app recent-activity table and the live security feed - and a second copy is
 * how the two drift into disagreeing about what BLOCKED looks like. The colour
 * pair stays the caller's: this owns the shape, not the vocabulary.
 */
export function Pill({
  className,
  title,
  children,
}: {
  className: string;
  /** Hover detail, for a badge that stands in for more than it says - the merged
   *  security column uses it to keep the guardrail verdict on a failed row. */
  title?: string;
  children: ReactNode;
}) {
  return (
    <span
      title={title}
      className={`inline-block rounded-control px-2 py-1 font-mono text-base-xs font-medium uppercase leading-4 tracking-label ${className}`}
    >
      {children}
    </span>
  );
}

/**
 * One badge per row, under a single Security column (`table/recent-activity`,
 * 272:3150, sampled from the properties panel 2026-08-21).
 *
 * Status and security used to be two columns; the design merged them, so a row
 * that failed reads ERROR and every other row reads what the guardrails did.
 * `error` is in this map rather than a second one because they now compete for one
 * cell and the precedence has to live somewhere the reader can see it.
 *
 * `allow` is grey, not green, which is the change worth noticing: green reads as
 * "good", and the useful signal in this column is when something was *acted on*.
 * A wall of green ticks is what makes the one amber row easy to miss. It is the
 * one entry that is not a verdict, so it takes a neutral over `gray/100` rather
 * than a coloured pair.
 *
 * That neutral is `gray/600`, not the design's `base/muted-foreground`. The
 * sampled pair is 4.39:1 on `gray/100` and this text is 12px medium, so it misses
 * AA by a hair - on the badge that will sit in almost every row. `gray/600` is
 * 6.87:1 and reads as the same grey. The same call the `gc` switch track made
 * when 2.98:1 turned up on a hovered row.
 *
 * The 100 stop with 700 text, deliberately quieter than `Overview`'s 200/900
 * action pills: those name a policy, these report what happened to one request,
 * and a table of them should not shout. REDACTED's text sits at the 800 - the
 * design's own exception, not a rounding of ours - and ERROR and BLOCKED sample
 * identically. Redacted is violet, matching `chart.redacted`; purple here was a
 * slip, and this was the app's last use of it.
 */
export const BADGE_STYLES: Record<ActivitySecurity | ActivityStatus, string> = {
  allow: "bg-gray-100 text-gray-600",
  flagged: "bg-amber-100 text-amber-700",
  redacted: "bg-violet-100 text-violet-800",
  blocked: "bg-red-100 text-red-700",
  error: "bg-red-100 text-red-700",
  // Never rendered: a successful request shows its security action instead. Here
  // so the map stays exhaustive over both unions and a new status cannot be added
  // without deciding what it looks like. Matches `allow`, the other non-verdict.
  success: "bg-gray-100 text-gray-600",
};
