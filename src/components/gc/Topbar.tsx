import { useEffect, useRef, useState } from "react";
import type { RefObject } from "react";
import { ConstellationHexMark } from "./ConstellationHexMark";
import { Icon } from "./Icon";
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
 * instance on Quit and omit support for the reason below. Raised with
 * design; see `docs/figma-questions-for-design.md`.
 *
 * Two things the design draws are deliberately absent. The traffic lights are
 * the operating system's, so we only reserve the space. The Minimize2 button
 * is gone: with system controls that job is already the OS's, and a second
 * minimise would have been a duplicate affordance.
 *
 * Presentational: the shell supplies every handler.
 */

/**
 * All four drawn entries, in the drawn order: dashboard, support, docs, quit
 * (`116:27225`, and the `topnav/menu` component `744:37692` draws the first
 * three).
 *
 * **Contact support has a working destination as of 2026-09-07.** It opens the
 * dashboard's own Overview page, because that is where the support floating
 * action button lives - there is no dedicated support route, so the link lands
 * the user on the page carrying the control rather than on the control itself.
 * Built from `dashboardLinks(...).support`, so it follows the environment this
 * install talks to (AG-598).
 *
 * This entry shipped for three days ahead of that address, by a decision on
 * 2026-09-04 that overruled the argument that an entry opening a 404 is worse
 * than an absent one. The history matters because the same argument had kept
 * `TrayMenu` from drawing its copy, so one drawn item was present on one
 * surface and absent on the other until the address arrived. Both draw it now.
 *
 * `SettingsPane` keeps its Support row omitted, and that is still not the same
 * decision: no Settings frame draws one. That omission survives the address
 * being fixed.
 */
export type TopnavAction = "dashboard" | "support" | "docs" | "quit";

const MENU_ITEMS: { action: TopnavAction; icon: IconName; label: string }[] = [
  { action: "dashboard", icon: "layoutDashboard", label: "Visit dashboard" },
  { action: "support", icon: "headset", label: "Contact support" },
  { action: "docs", icon: "bookOpenText", label: "Read Gate docs" },
];

/** Quit, drawn into the menu on 2026-08-28 (`116:27225`) and the one entry that
 *  does not leave the app - so it carries destructive ink and no external-link
 *  glyph, which is how the frame renders it. */
const QUIT_ITEM = { action: "quit" as const, icon: "logOut" as IconName, label: "Quit Gate Connect" };

export function Topbar({
  menuOpen,
  onMenuToggle,
  onMenuSelect,
}: {
  menuOpen: boolean;
  onMenuToggle: () => void;
  onMenuSelect: (action: TopnavAction) => void;
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
          <>
            {/* An invisible scrim, so a click anywhere else closes the menu
                *and nothing else happens* - the same one `TrayMenu` draws, and
                for the same reason: without it the menu had no dismissal but
                the ellipsis button itself, and a click meant to dismiss it
                landed on whatever pane control was under the pointer. It paints
                nothing, so it is not a visual change; the design draws no
                scrim. It also covers the trigger, which is why clicking that
                while open still closes once rather than toggling twice. */}
            <div aria-hidden className="fixed inset-0 z-10" onClick={onMenuToggle} />
            <TopnavMenu
              onSelect={onMenuSelect}
              onDismiss={onMenuToggle}
              triggerRef={menuButton}
            />
          </>
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

/** The 224px overflow menu. Every destination but Quit opens outside the app,
 *  so those rows carry the external-link glyph and Quit does not.
 *
 *  Keyboard behaviour is `TrayMenu`'s, ported: the panel takes focus on open,
 *  the arrows move within it, and it is a single tab stop. The two menus are
 *  deliberately not one component - they draw different shadows, paddings,
 *  glyph sizes and corners off different frames - but the interaction is the
 *  same and diverging on it was the bug. */
export function TopnavMenu({
  onSelect,
  onDismiss,
  triggerRef,
}: {
  onSelect: (action: TopnavAction) => void;
  /** Close without choosing: Escape from inside the panel. The scrim and the
   *  window-level Escape are the shell's, and go through the same handler. */
  onDismiss: () => void;
  /** The button that opened this. Focus returns to it on close - every exit
   *  unmounts the panel, and with the focused element gone `activeElement`
   *  falls to `<body>`, so the next Tab restarts from the top of the window
   *  rather than from the control the user was just on. */
  triggerRef?: RefObject<HTMLButtonElement>;
}) {
  const panel = useRef<HTMLDivElement>(null);
  /**
   * Which item is the menu's single tab stop (roving tabindex).
   *
   * `role="menu"` is meant to be ONE stop, with the arrows moving inside it. As
   * four plain buttons every item was in the tab order, so Tab past the last
   * one walked into the sidebar behind the open menu - visible beside it, and
   * click-blocked by the scrim, so the focus ring went somewhere the pointer
   * could not follow.
   */
  const [current, setCurrent] = useState(0);

  /**
   * Focus the first item when the menu opens.
   *
   * Without it the menu was unreachable by keyboard: it opens from a button, so
   * focus stayed on that button and Tab walked into the shell *behind* the menu
   * rather than into it. `role="menu"` promises arrow-key navigation, so the
   * roles were describing behaviour that did not exist.
   */
  useEffect(() => {
    panel.current?.querySelector<HTMLButtonElement>("[role='menuitem']")?.focus();
    // One cleanup covers every exit: Escape, the scrim and selecting an item all
    // unmount this panel.
    const trigger = triggerRef;
    return () => trigger?.current?.focus();
  }, [triggerRef]);

  /**
   * Arrow keys move between items, Escape closes, Home/End jump.
   *
   * Wrapping at both ends, which is what a menu does and what a listbox does
   * not. `stopPropagation` on Escape so the shell's document handler does not
   * also fire on the same key.
   */
  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      onDismiss();
      return;
    }
    const keys = ["ArrowDown", "ArrowUp", "Home", "End"];
    if (!keys.includes(e.key)) return;
    const items = Array.from(
      panel.current?.querySelectorAll<HTMLButtonElement>("[role='menuitem']") ?? [],
    );
    if (items.length === 0) return;
    e.preventDefault();
    const at = items.indexOf(document.activeElement as HTMLButtonElement);
    const next =
      e.key === "Home"
        ? 0
        : e.key === "End"
          ? items.length - 1
          : e.key === "ArrowDown"
            ? (at + 1 + items.length) % items.length
            : (at - 1 + items.length) % items.length;
    setCurrent(next);
    items[next]?.focus();
  };

  return (
    <div
      ref={panel}
      role="menu"
      onKeyDown={onKeyDown}
      className="absolute right-0 top-10 z-20 w-56 rounded-md border border-base-border bg-base-card p-2 shadow-base-lg"
    >
      {MENU_ITEMS.map(({ action, icon, label }, i) => (
        <button
          key={action}
          type="button"
          role="menuitem"
          // Roving: only the current item is a tab stop, so Tab leaves the menu
          // as a unit and the arrows move within it.
          tabIndex={current === i ? 0 : -1}
          onClick={() => onSelect(action)}
          className="flex h-8 w-full items-center justify-between rounded-control px-1.5 text-base-foreground shadow-base-2xs transition-colors hover:bg-gray-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
        >
          <span className="flex items-center gap-2">
            <Icon name={icon} size={16} />
            <span className="text-base-xs font-medium leading-4 tracking-label-12">{label}</span>
          </span>
          <Icon name="squareArrowOutUpRight" size={12} className="text-neutral-500" />
        </button>
      ))}
      <button
        type="button"
        role="menuitem"
        tabIndex={current === MENU_ITEMS.length ? 0 : -1}
        onClick={() => onSelect(QUIT_ITEM.action)}
        className="flex h-8 w-full items-center gap-2 rounded-control px-1.5 text-red-600 shadow-base-2xs transition-colors hover:bg-gray-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
      >
        <Icon name={QUIT_ITEM.icon} size={16} />
        <span className="text-base-xs font-medium leading-4 tracking-label-12">{QUIT_ITEM.label}</span>
      </button>
    </div>
  );
}
