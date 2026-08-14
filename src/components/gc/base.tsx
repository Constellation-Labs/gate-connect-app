import type { IconName } from "./Icon";
import { Icon } from "./Icon";

/**
 * Shared primitives for the new app UI (Figma "Gate Connect"). Named for the
 * `base.*` token group in `tailwind.config.ts`, which mirrors the design's own
 * variable names. Distinct from `gc/ui.tsx`, which is the menu-bar popover's
 * primitive set and stays until those screens migrate.
 */

/**
 * The 28x28 status chip that fronts the routing banners, and at 36px the alert
 * banner. One shape, two palettes: a vertical 50 -> 200 gradient, a 300 border
 * and a 700 icon, all on Tailwind's default ramps (Figma 113:16788 / 113:16891).
 *
 * Tone classes are spelled out rather than interpolated - Tailwind only sees
 * literal class names at build time.
 */
const TILE_TONES = {
  green: "from-green-50 to-green-200 border-green-300 text-green-700",
  amber: "from-amber-50 to-amber-200 border-amber-300 text-amber-700",
} as const;

export function StatusTile({
  tone,
  icon,
  size = 28,
}: {
  tone: keyof typeof TILE_TONES;
  icon: IconName;
  size?: 28 | 36;
}) {
  return (
    <span
      aria-hidden
      className={`flex shrink-0 items-center justify-center rounded-base border bg-gradient-to-b ${TILE_TONES[tone]} ${
        size === 36 ? "size-9" : "size-7"
      }`}
    >
      <Icon name={icon} size={size === 36 ? 20 : 16} />
    </span>
  );
}

/**
 * 36x20 track, 16px thumb (Figma 113:16827). Geometry differs from the
 * popover's `gc/ui.tsx` Switch (38x22 with a check glyph), so the two coexist
 * until the popover screens migrate; the accessibility contract is carried
 * over unchanged, including the `before:` hit-area expansion that takes the
 * target past 24px without moving the visible track.
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
      className={`relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors before:absolute before:inset-x-0 before:-inset-y-1 before:content-[''] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
        busy ? "opacity-70" : ""
      } ${on ? "bg-blue-ribbon-700" : "bg-base-input"}`}
    >
      <span
        className={`absolute size-4 rounded-full bg-base-background shadow-base-lg transition-transform ${
          on ? "translate-x-4" : "translate-x-1"
        }`}
      />
    </button>
  );
}
