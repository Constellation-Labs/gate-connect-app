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
 * The design draws a fourth entry, Contact support, and it is not here.
 *
 * `GATE_SUPPORT_URL` does exist in `lib/config.ts`, and it 404s - so there is
 * still nothing to open. An entry that cannot do anything is worse than an
 * absent one, because the user cannot tell "not built" from "broken", and one
 * that opens a broken page is worse again. `SettingsPane` omits its Support row
 * for the same reason. Add both back together once there is a real address; see
 * that constant for what has to change (AG-598).
 */
export type TopnavAction = "dashboard" | "docs" | "quit";

const MENU_ITEMS: { action: TopnavAction; icon: IconName; label: string }[] = [
  { action: "dashboard", icon: "layoutDashboard", label: "Visit dashboard" },
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
        />
        {menuOpen && <TopnavMenu onSelect={onMenuSelect} />}
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
}: {
  icon: IconName;
  label: string;
  onClick: () => void;
  expanded?: boolean;
  radius?: keyof typeof ICON_BUTTON_RADIUS;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      aria-haspopup={expanded === undefined ? undefined : "menu"}
      aria-expanded={expanded}
      className={`flex size-8 items-center justify-center ${ICON_BUTTON_RADIUS[radius]} border border-base-input bg-base-card text-base-foreground shadow-[0_1px_2px_0_rgba(0,0,0,0.05),inset_0_4px_6px_0_rgba(255,255,255,0.4),inset_0_-4px_4px_0_rgba(0,0,0,0.04)] transition-colors hover:bg-neutral-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary`}
    >
      <Icon name={icon} size={16} />
    </button>
  );
}

/** The 224px overflow menu. Every destination but Quit opens outside the app,
 *  so those rows carry the external-link glyph and Quit does not. */
export function TopnavMenu({ onSelect }: { onSelect: (action: TopnavAction) => void }) {
  return (
    <div
      role="menu"
      className="absolute right-0 top-10 z-10 w-56 rounded-md border border-base-border bg-base-card p-2 shadow-base-lg"
    >
      {MENU_ITEMS.map(({ action, icon, label }) => (
        <button
          key={action}
          type="button"
          role="menuitem"
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
        onClick={() => onSelect(QUIT_ITEM.action)}
        className="flex h-8 w-full items-center gap-2 rounded-control px-1.5 text-red-600 shadow-base-2xs transition-colors hover:bg-gray-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
      >
        <Icon name={QUIT_ITEM.icon} size={16} />
        <span className="text-base-xs font-medium leading-4 tracking-label-12">{QUIT_ITEM.label}</span>
      </button>
    </div>
  );
}
