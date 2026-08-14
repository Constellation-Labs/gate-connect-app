import type { ReactNode } from "react";
import { BaseSwitch } from "./base";
import { Icon } from "./Icon";

/**
 * Left navigation rail for the new app UI (Figma `nav/sidebar/overview`,
 * node 113:16794). 250px wide, fixed, sits beside a scrolling content pane -
 * it is not one of the popover's sliding panels and carries no back affordance.
 *
 * Presentational: every piece of state arrives as a prop so the shell can own
 * data fetching. Nothing here talks to `lib/api`.
 */

export type SidebarView = "overview" | "settings";

export interface SidebarApp {
  slug: string;
  name: string;
  /** Routed through Gate right now. Drives both the switch and the status line. */
  isProtected: boolean;
  /** Relative age of the current status ("2m ago"). The design only shows this
   * for protected apps; omit it and the status line renders on its own. */
  since?: string;
  /** 16px brand mark, rendered inside the tile. Falls back to the app's initial
   * while the marks are still being exported from Figma. */
  logo?: ReactNode;
  /** A toggle is in flight: the switch ignores clicks but keeps focus. */
  busy?: boolean;
}

export function Sidebar({
  orgName,
  onSwitchOrg,
  view,
  onNavigate,
  apps,
  onToggleApp,
}: {
  orgName: string;
  onSwitchOrg: () => void;
  view: SidebarView;
  onNavigate: (view: SidebarView) => void;
  apps: SidebarApp[];
  onToggleApp: (slug: string, next: boolean) => void;
}) {
  const protectedCount = apps.filter((a) => a.isProtected).length;

  return (
    <nav
      aria-label="Main"
      className="flex w-[250px] shrink-0 flex-col border-r border-black/[0.08] bg-base-card p-4"
    >
      <div className="flex flex-1 flex-col gap-6">
        <div className="flex flex-col gap-4">
          <OrgSwitcher name={orgName} onClick={onSwitchOrg} />

          <div className="flex flex-col gap-1">
            <NavItem
              icon="layoutDashboard"
              label="Overview"
              active={view === "overview"}
              onClick={() => onNavigate("overview")}
            />
            <NavItem
              icon="settings2"
              label="Settings"
              active={view === "settings"}
              onClick={() => onNavigate("settings")}
            />
          </div>

          <hr className="border-t border-base-border" />

          <div className="flex flex-col gap-4">
            {/* The Figma mock reads "4/4" beside two switched-off apps, which
             * can only be stale copy - the count is protected-over-total. */}
            <h2 className="flex items-center gap-2 font-mono text-base-xs font-medium uppercase leading-4 tracking-eyebrow text-base-muted-foreground">
              <span className="flex-1">Protected apps</span>
              <span>
                {protectedCount}/{apps.length}
              </span>
            </h2>

            <ul className="flex flex-col gap-1">
              {apps.map((app) => (
                <AppRow key={app.slug} app={app} onToggle={onToggleApp} />
              ))}
            </ul>
          </div>
        </div>
      </div>
    </nav>
  );
}

function OrgSwitcher({ name, onClick }: { name: string; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex w-full items-center justify-between rounded-base border border-base-input bg-base-card px-1.5 py-2 shadow-base-2xs"
    >
      <span className="flex items-center gap-2">
        <Icon name="usersRound" size={16} />
        <span className="text-base-xs font-medium leading-4 text-neutral-900">{name}</span>
      </span>
      <Icon name="chevronsUpDown" size={16} />
    </button>
  );
}

function NavItem({
  icon,
  label,
  active,
  onClick,
}: {
  icon: "layoutDashboard" | "settings2";
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={active ? "page" : undefined}
      className={`flex w-full items-center gap-2 rounded-base px-1.5 py-2 text-base-xs font-medium leading-4 ${
        active
          ? "border border-base-border bg-gray-100 text-base-primary shadow-base-2xs"
          : "text-neutral-900"
      }`}
    >
      <Icon name={icon} size={16} />
      {label}
    </button>
  );
}

function AppRow({
  app,
  onToggle,
}: {
  app: SidebarApp;
  onToggle: (slug: string, next: boolean) => void;
}) {
  return (
    <li className="flex w-full items-center gap-4 rounded-lg px-1 py-1.5">
      <span className="flex min-w-0 flex-1 items-center gap-2">
        <span
          aria-hidden
          className="flex size-8 shrink-0 items-center justify-center rounded-base border border-white/[0.24] bg-black text-base-2xs font-medium text-white"
          style={{
            backgroundImage:
              "linear-gradient(180deg, rgba(255,255,255,0.32) 0%, rgba(0,0,0,0.32) 100%)",
          }}
        >
          {app.logo ?? app.name.charAt(0)}
        </span>
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-base-xs font-medium leading-4 text-neutral-900">
            {app.name}
          </span>
          <span className="text-base-2xs font-medium leading-4">
            <span className={app.isProtected ? "text-green-600" : "text-amber-600"}>
              {app.isProtected ? "Protected" : "Not protected"}
            </span>
            {app.since && <span className="text-neutral-500"> - {app.since}</span>}
          </span>
        </span>
      </span>
      <BaseSwitch
        on={app.isProtected}
        label={app.name}
        busy={app.busy}
        onClick={() => onToggle(app.slug, !app.isProtected)}
      />
    </li>
  );
}
