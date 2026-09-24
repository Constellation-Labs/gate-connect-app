import { useEffect, useRef } from "react";
import type { RefObject } from "react";
import { ConstellationHexMark } from "./ConstellationHexMark";
import { Icon } from "./Icon";
import { OverflowMenu } from "./OverflowMenu";
import type { MenuAction } from "./OverflowMenu";
import type { IconName } from "./Icon";

/**
 * Window chrome for the new app UI: a 48px strip, brand lockup centred,
 * overflow menu on the right.
 *
 * Drawn as `nav/topbar` (`744:37739`) and `topnav/menu` (`744:37692`). Both
 * live on the **Banners** and **Menus** component canvases - the library was
 * not deleted, it moved off the old `113:16762` page into three sections
 * (Banners `744:37738`, Menus `744:37691`, Sidenav `408:15625`). An earlier
 * note here said both nodes were gone; they are not.
 *
 * The component and the flow instance disagree about the menu's CONTENTS:
 * `744:37692` draws three rows - dashboard, Contact support, docs - and no
 * Quit, while the Overview instance `116:27225` draws four including Quit
 * (its 146px is the height: 4x32 plus 9 top and bottom). We follow the
 * instance. The menu itself is `OverflowMenu`, shared with the tray.
 *
 * Two things the design draws are deliberately absent. The traffic lights are
 * the operating system's, so we only reserve the space. The Minimize2 button
 * is gone: with system controls that job is already the OS's, and a second
 * minimise would have been a duplicate affordance.
 *
 * Presentational: the shell supplies every handler.
 */

export function Topbar({
  menuOpen,
  onMenuToggle,
  onMenuSelect,
}: {
  menuOpen: boolean;
  onMenuToggle: () => void;
  onMenuSelect: (action: MenuAction) => void;
}) {
  /** The button the menu opens from. Focus returns to it when the panel
   *  unmounts, and the panel is what holds focus while it is open. */
  const menuButton = useRef<HTMLButtonElement>(null);

  /**
   * Escape closes the menu, from outside the panel.
   *
   * The panel handles the key itself while focus is inside it. This is the case
   * where focus has since moved elsewhere: the menu is one tab stop, so Tab
   * leaves it as a unit and the menu stays open with focus behind the scrim -
   * exactly the gap `TrayApp`'s own document handler covers for the popover.
   *
   * **Bubble phase, deliberately**, the same argument `TrayApp` makes for its
   * window-hiding handler: `useFocusTrap` takes Escape in the capture phase and
   * calls `stopPropagation`, so a dialog over the window answers the key first -
   * which is what should happen. Registering in capture would race that trap.
   */
  useEffect(() => {
    if (!menuOpen) return;
    const onKeyDown = (e: KeyboardEvent) => {
      // Only ever a close: the effect is not registered while the menu is shut,
      // so the toggle cannot be the thing that opens it.
      if (e.key === "Escape") onMenuToggle();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [menuOpen, onMenuToggle]);

  return (
    <header className="flex h-12 w-full items-center justify-between border-b border-base-border bg-base-card px-4">
      {/* Mirrors the width of the button cluster opposite so the lockup sits
       * dead centre. The design places it at 504px of 1024 rather than 512,
       * but only because it had a second button on the right; with that gone,
       * centred is what the design was approximating. macOS paints its traffic
       * lights over this empty span, which is why nothing else goes here. */}
      <span aria-hidden className="w-8 shrink-0" />

      <span className="flex items-center gap-2">
        <ConstellationHexMark size={24} />
        <span className="text-base font-semibold leading-6 tracking-[-0.16px]">
          <span className="text-blue-ribbon-800">Gate</span>{" "}
          <span className="text-neutral-600">Connect</span>
        </span>
      </span>

      <div className="relative flex items-center gap-3">
        <OutlineIconButton
          icon="ellipsis"
          label="More"
          onClick={onMenuToggle}
          expanded={menuOpen}
          buttonRef={menuButton}
        />
        {menuOpen && (
          <OverflowMenu
            surface="topbar"
            onSelect={onMenuSelect}
            onDismiss={onMenuToggle}
            triggerRef={menuButton}
          />
        )}
      </div>
    </header>
  );
}

/**
 * shadcn's outline icon button. The paired inset shadows are what give it its
 * slightly domed face - a top white bloom and a bottom shade - over the 1px
 * `base/input` border. Exported for the tray popover's footer, which draws the
 * same 32px ellipsis button in front of its own menu.
 */
/**
 * Radius follows the surface, not the component: the topbar draws 4px
 * (`127:46660`) and the tray's footer 8px (`694:34124` default,
 * `744:38191` pressed). One button, two surfaces, two numbers - so the call
 * site says which, and `rounded-sm` (6px in this config) was wrong for both.
 *
 * A map rather than `rounded-${radius}`: an interpolated class name is
 * invisible to Tailwind's scanner and emits no CSS at all, which is the bug
 * six `rounded-base` uses in `dialogs.tsx` shipped as.
 */
const ICON_BUTTON_RADIUS = {
  control: "rounded-control",
  md: "rounded-md",
} as const;

export function OutlineIconButton({
  icon,
  label,
  onClick,
  expanded,
  radius = "control",
  buttonRef,
}: {
  icon: IconName;
  label: string;
  onClick: () => void;
  expanded?: boolean;
  radius?: keyof typeof ICON_BUTTON_RADIUS;
  /** For a caller that has to give focus back to this button - a menu trigger
   *  whose panel unmounts, where `activeElement` would otherwise fall to
   *  `<body>` and the next Tab restart from the top of the surface. */
  buttonRef?: RefObject<HTMLButtonElement>;
}) {
  return (
    <button
      ref={buttonRef}
      type="button"
      onClick={onClick}
      aria-label={label}
      aria-haspopup={expanded === undefined ? undefined : "menu"}
      aria-expanded={expanded}
      className={`flex size-8 items-center justify-center ${ICON_BUTTON_RADIUS[radius]} border border-base-input bg-base-card text-base-primary shadow-[0_1px_2px_0_rgba(0,0,0,0.05),inset_0_4px_6px_0_rgba(255,255,255,0.4),inset_0_-4px_4px_0_rgba(0,0,0,0.04)] transition-colors hover:bg-neutral-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary`}
    >
      <Icon name={icon} size={16} />
    </button>
  );
}
