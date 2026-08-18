import type { ReactNode } from "react";
import { BaseSwitch } from "./base";
import { Icon } from "./Icon";
import type { IconName } from "./Icon";

/**
 * Left navigation rail for the new app UI (Figma `nav/sidebar/overview`,
 * node 113:16794). 250px wide, fixed, sits beside a scrolling content pane -
 * it is not one of the popover's sliding panels and carries no back affordance.
 *
 * Presentational: every piece of state arrives as a prop so the shell can own
 * data fetching. Nothing here talks to `lib/api`.
 */

export type SidebarView =
  | { kind: "overview" }
  | { kind: "families" }
  | { kind: "settings" }
  /** An app row is selected and its detail pane is open. */
  | { kind: "app"; slug: string };

/**
 * What is actually happening to this app's traffic. The design draws four:
 * "Protected - 2m ago", "Not protected", "Config drifted", "Not routed - Off".
 * Each renders as a coloured phrase plus an optional grey suffix.
 */
export type AppStatus =
  | { kind: "protected"; since?: string }
  /** `detail` carries the routing verdict's reason ("Reopen required",
   * "Connection problem"), which is what turns an amber phrase into something
   * the user can act on. See `lib/verdict.ts`. */
  | { kind: "not-protected"; detail?: string }
  | { kind: "drifted" }
  | { kind: "not-routed"; detail?: string };

export interface SidebarApp {
  slug: string;
  name: string;
  /**
   * Observed: what is happening right now, which drives the status line.
   * Deliberately separate from `on` below.
   */
  status: AppStatus;
  /**
   * Intent: what the user asked for, which drives the switch.
   *
   * These were one field early on and `lib/groups.ts` documents why they
   * cannot be: an enabled domain whose certificate is not trusted is not
   * routing, so a switch driven by the observed state renders off, and
   * clicking it sends `!enabled === false` - turning off the very setting the
   * user was trying to turn on, without the switch ever moving.
   */
  on: boolean;
  /** 16px brand mark, rendered inside the tile. Falls back to the app's initial
   * while the marks are still being exported from Figma. */
  logo?: ReactNode;
  /** A toggle is in flight: the switch ignores clicks but keeps focus. */
  busy?: boolean;
}

const STATUS_TEXT: Record<AppStatus["kind"], { label: string; className: string }> = {
  protected: { label: "Protected", className: "text-green-600" },
  "not-protected": { label: "Not protected", className: "text-amber-600" },
  drifted: { label: "Config drifted", className: "text-amber-600" },
  "not-routed": { label: "Not routed", className: "text-amber-600" },
};

function statusSuffix(status: AppStatus): string | undefined {
  if (status.kind === "protected") return status.since;
  if (status.kind === "not-routed") return status.detail;
  if (status.kind === "not-protected") return status.detail;
  return undefined;
}

export function Sidebar({
  orgName,
  onSwitchOrg,
  view,
  onNavigate,
  apps,
  onSelectApp,
  onToggleApp,
  onRefresh,
  refreshing,
}: {
  orgName: string;
  onSwitchOrg: () => void;
  view: SidebarView;
  onNavigate: (view: SidebarView) => void;
  apps: SidebarApp[];
  /** Opens the per-app pane. */
  onSelectApp: (slug: string) => void;
  onToggleApp: (slug: string, next: boolean) => void;
  /** Re-run detection now. Omitted leaves the control out entirely, on the same
   * rule the Settings rows follow: a button that does nothing is worse than no
   * button. */
  onRefresh?: () => void;
  /** A scan is in flight; the control refuses clicks and says so to assistive
   * technology. */
  refreshing?: boolean;
}) {
  const protectedCount = apps.filter((a) => a.status.kind === "protected").length;

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
              active={view.kind === "overview"}
              onClick={() => onNavigate({ kind: "overview" })}
            />
            {/* Not in the Figma. The model families that actually carry routing
             * have no destination in the drawn IA, so they get one here rather
             * than being dropped. See plans/new-app-ui-figma.md. */}
            <NavItem
              icon="layers"
              label="Families"
              active={view.kind === "families"}
              onClick={() => onNavigate({ kind: "families" })}
            />
            <NavItem
              icon="settings2"
              label="Settings"
              active={view.kind === "settings"}
              onClick={() => onNavigate({ kind: "settings" })}
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
              {/* Provisional: the Figma draws no refresh control. Detection only
                  ran on backend events, so installing a tool while this window
                  was open showed nothing until something unrelated changed it.
                  Small and unlabelled because it sits in a 12px eyebrow - the
                  glyph carries an aria-label rather than visible text. */}
              {onRefresh && (
                <button
                  type="button"
                  onClick={onRefresh}
                  aria-label="Refresh apps"
                  aria-busy={refreshing || undefined}
                  disabled={refreshing}
                  className="flex size-5 items-center justify-center rounded-base text-base-muted-foreground transition-colors hover:bg-gray-100 hover:text-neutral-900 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-base-primary disabled:cursor-not-allowed disabled:opacity-50"
                >
                  {/* No spinner: the scan is fast enough that one would flash.
                      `aria-busy` plus the disabled state is the whole signal. */}
                  <Icon name="refresh" size={12} />
                </button>
              )}
            </h2>

            <ul className="flex flex-col gap-1">
              {apps.map((app) => (
                <AppRow
                  key={app.slug}
                  app={app}
                  selected={view.kind === "app" && view.slug === app.slug}
                  onSelect={onSelectApp}
                  onToggle={onToggleApp}
                />
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
  icon: IconName;
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

/**
 * Two targets in one row: the row opens the app's pane, the switch routes it.
 * The switch is a sibling rather than a child so a click on it never also
 * navigates.
 */
function AppRow({
  app,
  selected,
  onSelect,
  onToggle,
}: {
  app: SidebarApp;
  selected: boolean;
  onSelect: (slug: string) => void;
  onToggle: (slug: string, next: boolean) => void;
}) {
  const status = STATUS_TEXT[app.status.kind];
  const suffix = statusSuffix(app.status);

  return (
    <li
      className={`flex w-full items-center gap-4 rounded-lg px-1 py-1.5 ${
        selected ? "bg-gray-100" : ""
      }`}
    >
      <button
        type="button"
        onClick={() => onSelect(app.slug)}
        aria-current={selected ? "page" : undefined}
        className="flex min-w-0 flex-1 items-center gap-2 rounded-base text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
      >
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
          <span
            className={`truncate text-base-xs font-medium leading-4 ${
              selected ? "text-base-primary" : "text-neutral-900"
            }`}
          >
            {app.name}
          </span>
          <span className="truncate text-base-2xs font-medium leading-4">
            <span className={status.className}>{status.label}</span>
            {suffix && <span className="text-neutral-500"> - {suffix}</span>}
          </span>
        </span>
      </button>
      <BaseSwitch
        on={app.on}
        label={app.name}
        busy={app.busy}
        onClick={() => onToggle(app.slug, !app.on)}
      />
    </li>
  );
}
