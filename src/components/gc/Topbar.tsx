import { ConstellationHexMark } from "./ConstellationHexMark";
import { Icon } from "./Icon";
import type { IconName } from "./Icon";

/**
 * Window chrome for the new app UI (Figma `nav/topbar` 113:16763 and
 * `topnav/menu` 116:17428). A 48px strip: window controls left, brand lockup
 * centred, overflow menu and collapse right.
 *
 * The lockup lands slightly left of true centre (504px of 1024) because the
 * design uses space-between across three groups of unequal width rather than
 * absolute centring - reproduced here by doing the same thing.
 *
 * Presentational: the shell supplies every handler.
 */

export type TopnavAction = "dashboard" | "support" | "docs";

const MENU_ITEMS: { action: TopnavAction; icon: IconName; label: string }[] = [
  { action: "dashboard", icon: "layoutDashboard", label: "Visit dashboard" },
  { action: "support", icon: "headset", label: "Contact support" },
  { action: "docs", icon: "bookOpenText", label: "Read Gate docs" },
];

export function Topbar({
  onCollapse,
  menuOpen,
  onMenuToggle,
  onMenuSelect,
}: {
  /** The Minimize2 button - shrinks the window back down. */
  onCollapse: () => void;
  menuOpen: boolean;
  onMenuToggle: () => void;
  onMenuSelect: (action: TopnavAction) => void;
}) {
  return (
    <header className="flex h-12 w-full items-center justify-between border-b border-base-border bg-base-card px-4">
      {/* Reserves the 60px the design gives its own traffic lights, so the
       * lockup keeps its drawn position (504px of 1024) and never slides under
       * the window controls the OS draws there. The controls themselves are the
       * system's - see the window config note in plans/new-app-ui-figma.md. */}
      <span aria-hidden className="w-[60px] shrink-0" />

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
        <OutlineIconButton icon="minimize2" label="Collapse window" onClick={onCollapse} />
        {menuOpen && <TopnavMenu onSelect={onMenuSelect} />}
      </div>
    </header>
  );
}

/**
 * shadcn's outline icon button. The paired inset shadows are what give it its
 * slightly domed face - a top white bloom and a bottom shade - over the 1px
 * `base/input` border.
 */
function OutlineIconButton({
  icon,
  label,
  onClick,
  expanded,
}: {
  icon: IconName;
  label: string;
  onClick: () => void;
  expanded?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      aria-haspopup={expanded === undefined ? undefined : "menu"}
      aria-expanded={expanded}
      className="flex size-8 items-center justify-center rounded-base border border-base-input bg-base-card text-neutral-900 shadow-[0_1px_2px_0_rgba(0,0,0,0.05),inset_0_4px_6px_0_rgba(255,255,255,0.4),inset_0_-4px_4px_0_rgba(0,0,0,0.06)] transition-colors hover:bg-neutral-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
    >
      <Icon name={icon} size={16} />
    </button>
  );
}

/** The 224px overflow menu. Every destination opens outside the app, so each
 *  row carries the external-link glyph. */
export function TopnavMenu({ onSelect }: { onSelect: (action: TopnavAction) => void }) {
  return (
    <div
      role="menu"
      className="absolute right-0 top-10 z-10 w-56 rounded-lg border border-base-border bg-base-card p-[9px] shadow-base-lg"
    >
      {MENU_ITEMS.map(({ action, icon, label }) => (
        <button
          key={action}
          type="button"
          role="menuitem"
          onClick={() => onSelect(action)}
          className="flex h-8 w-full items-center justify-between rounded-base px-1.5 text-neutral-900 transition-colors hover:bg-gray-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
        >
          <span className="flex items-center gap-2">
            <Icon name={icon} size={16} />
            <span className="text-base-xs font-medium leading-4">{label}</span>
          </span>
          <Icon name="squareArrowOutUpRight" size={12} className="text-neutral-500" />
        </button>
      ))}
    </div>
  );
}
