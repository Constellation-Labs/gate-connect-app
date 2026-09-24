import type { RefObject } from "react";
import { useRovingMenu } from "../../lib/useRovingMenu";
import { Icon } from "./Icon";
import type { IconName } from "./Icon";

/**
 * The overflow menu, drawn on two surfaces: the window's topbar (`116:27225`)
 * and the tray popover's footer (`744:38192`). Both are instances of
 * `topnav/menu`, and they agree on everything but three properties - the panel
 * shadow (`shadow/lg` in the window, `shadow/md` in the tray), the glyph size
 * (16 against 14) and which way the panel opens. So the entries, the row
 * markup and the scrim live here once, and the surface names the rest.
 *
 * The two used to be separate components, argued as "different shadows,
 * paddings, glyph sizes and corners". Read against the frames on 2026-09-24
 * only the shadow and the glyph held: both draw 8px padding and the same
 * corners. The copies had meanwhile drifted where the frames do not - the
 * tray padded 9px, and dropped `label/12`'s tracking on Quit - and one grew a
 * row line the other did not have.
 *
 * **No row line.** Every drawn row carries `shadow/2xs`, which Figma renders as
 * nothing on a row with no fill. As a CSS `box-shadow` it paints a 1px rule
 * under each row, so the rows take none and match what the frames render.
 *
 * Keyboard and focus are `useRovingMenu`'s.
 */

/**
 * All four drawn entries, in the drawn order: dashboard, support, docs, quit.
 *
 * **Contact support has a working destination as of 2026-09-07.** It opens the
 * dashboard's own Overview page, because that is where the support floating
 * action button lives - there is no dedicated support route, so the link lands
 * the user on the page carrying the control rather than on the control itself.
 * Built from `dashboardLinks(...).support`, so it follows the environment this
 * install talks to (AG-598).
 *
 * `SettingsPane` keeps its Support row omitted, and that is not the same
 * decision: no Settings frame draws one.
 */
export type MenuAction = "dashboard" | "support" | "docs" | "quit";

/** Every entry but Quit opens outside the app, so those rows carry the
 *  external-link glyph. */
const EXTERNAL_ITEMS: { action: MenuAction; icon: IconName; label: string }[] = [
  { action: "dashboard", icon: "layoutDashboard", label: "Visit dashboard" },
  { action: "support", icon: "headset", label: "Contact support" },
  { action: "docs", icon: "bookOpenText", label: "Read Gate docs" },
];

/**
 * What differs by surface. A map of whole class names rather than
 * interpolation, which Tailwind's scanner cannot see.
 *
 * The z values are each surface's own. The topbar's panel sits above
 * `AppShell`'s notice wrapper (z-30): notices are raised over the modal scrim
 * so a failed rename stays clickable, and the reopen banner had drawn across
 * this menu and hidden its middle entries. That costs nothing, because a menu
 * and a dialog never coexist - selecting closes the menu before opening one.
 */
const SURFACE = {
  topbar: {
    scrim: "z-40",
    panel: "right-0 top-10 z-50 shadow-base-lg",
    glyph: 16,
  },
  tray: {
    scrim: "z-10",
    panel: "bottom-12 right-4 z-20 shadow-base-md",
    glyph: 14,
  },
} as const;

const ROW =
  "flex h-8 w-full items-center rounded-control px-1.5 transition-colors hover:bg-gray-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary";
const LABEL = "text-base-xs font-medium leading-4 tracking-label-12";

export function OverflowMenu({
  surface,
  onSelect,
  onDismiss,
  triggerRef,
}: {
  surface: keyof typeof SURFACE;
  onSelect: (action: MenuAction) => void;
  /** Close without choosing: Escape from inside the panel, or a click anywhere
   *  else. */
  onDismiss: () => void;
  /** The button that opened this; focus returns to it on close. See
   *  `useRovingMenu`. */
  triggerRef?: RefObject<HTMLButtonElement>;
}) {
  const { panel, current, onKeyDown } = useRovingMenu(triggerRef, onDismiss);
  const { scrim, panel: placement, glyph } = SURFACE[surface];

  return (
    <>
      {/* An invisible scrim, so a click anywhere else closes the menu *and
          nothing else happens*. Without it a click meant to dismiss the menu
          landed on whatever was under the pointer - in the tray, a row's
          switch, which is a routing change the user did not ask for. It paints
          nothing, so it is not a visual change; the design draws no scrim. It
          also covers the trigger, which is why clicking that while open still
          closes once rather than toggling twice. */}
      <div aria-hidden className={`fixed inset-0 ${scrim}`} onClick={onDismiss} />
      <div
        ref={panel}
        role="menu"
        // Without a name this announces as a bare "menu". Named for the control
        // that opens it, so it reads "More, menu".
        aria-label="More"
        onKeyDown={onKeyDown}
        className={`absolute ${placement} w-56 rounded-md border border-base-border bg-base-card p-2`}
      >
        {EXTERNAL_ITEMS.map(({ action, icon, label }, i) => (
          <button
            key={action}
            type="button"
            role="menuitem"
            // Roving: only the current item is a tab stop, so Tab leaves the menu
            // as a unit and the arrows move within it.
            tabIndex={current === i ? 0 : -1}
            onClick={() => onSelect(action)}
            className={`${ROW} justify-between text-base-foreground`}
          >
            <span className="flex items-center gap-2">
              <Icon name={icon} size={glyph} />
              <span className={LABEL}>{label}</span>
            </span>
            <Icon name="squareArrowOutUpRight" size={12} className="text-neutral-500" />
          </button>
        ))}
        {/* Quit, the one entry that does not leave the app - so it carries
            destructive ink and no external-link glyph. Both frames carry the
            glyph in their metadata and neither renders it. */}
        <button
          type="button"
          role="menuitem"
          tabIndex={current === EXTERNAL_ITEMS.length ? 0 : -1}
          onClick={() => onSelect("quit")}
          className={`${ROW} gap-2 text-red-600`}
        >
          <Icon name="logOut" size={glyph} />
          <span className={LABEL}>Quit Gate Connect</span>
        </button>
      </div>
    </>
  );
}
