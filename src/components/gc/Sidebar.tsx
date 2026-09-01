import type { ReactNode } from "react";
import { BaseSwitch } from "./base";
import { Icon } from "./Icon";
import type { IconName } from "./Icon";

/**
 * Left navigation rail for the new app UI (Figma `Components / Sidenav`, node
 * 408:15625; the `sidebar` set is `437:161`). 256px wide, fixed, sits beside a
 * scrolling pane - it is not one of the popover's sliding panels and carries no
 * back affordance.
 *
 * Presentational: every piece of state arrives as a prop so the shell can own
 * data fetching. Nothing here talks to `lib/api`.
 */

export type SidebarView =
  | { kind: "overview" }
  /** The live security-event feed (AG-578). Undrawn: the Sidenav frame
   *  (408:15625) draws Overview and Settings only, so this entry is built from
   *  the `NavItem` component set rather than copied from a frame. */
  | { kind: "security" }
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

/**
 * What the last detection scan established, which is not the same as how many
 * rows it produced.
 *
 * An empty list used to mean both "we looked and there is nothing" and "the look
 * failed" - `listTools().catch(() => [])` collapsed the second into the first, so
 * a machine that could not be scanned rendered as a machine with no AI tools on
 * it. Those need different words and different actions, which is the whole of
 * AG-560's first two criteria.
 */
export type InventoryState =
  /** Tools were found; the list speaks for itself. */
  | { kind: "ok" }
  /** The scan completed and found nothing. A real answer, so it carries when it
   * was taken. */
  | { kind: "none"; scannedAt: string }
  /** The scan could not complete. Says so rather than showing an empty shelf. */
  | { kind: "failed" };

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

/**
 * One eyebrow-labelled cluster of app rows. The design draws the rail grouped
 * by family (Figma `Flows / App`, read 2026-08-21: "ANTHROPIC" over the two
 * Claude apps, "OPEN AI" over Codex, and so on), replacing the earlier flat
 * list under a "Protected apps N/N" counter, which no frame draws any more.
 */
export interface SidebarGroup {
  /** Stable key - the family's id. */
  id: string;
  /** The eyebrow. Empty renders no header, which is the state before the
   * catalog has loaded and grouping is not yet known. */
  label: string;
  apps: SidebarApp[];
}

/**
 * The engine itself, above the families that ride on it.
 *
 * Not in the Figma, and the omission is load-bearing: with routing off, a family
 * switch can still start the engine (a config member's connect does it
 * implicitly) but a chat domain cannot, so the window could reach a state it had
 * no control for. `envExport` is the master's sub-setting - whether the proxy
 * also goes into the shell environment, which reaches `git` and `curl` and not
 * just the AI tools - and is absent on Linux, where those variables *are* the
 * system proxy and cannot be declined separately.
 */
export interface MasterRouting {
  on: boolean;
  busy?: boolean;
  onToggle: (next: boolean) => void;
  /** Whether the certificate is in the system trust store. Routing without it
   * inspects nothing, so the card says so rather than leaving the switch to
   * imply otherwise. */
  caTrusted?: boolean;
  envExport?: { on: boolean; onToggle: (next: boolean) => void };
}

export const STATUS_TEXT: Record<AppStatus["kind"], { label: string; className: string }> = {
  protected: { label: "Protected", className: "text-green-600" },
  "not-protected": { label: "Not protected", className: "text-amber-600" },
  drifted: { label: "Config drifted", className: "text-amber-600" },
  "not-routed": { label: "Not routed", className: "text-amber-600" },
};

/**
 * The grey suffix a rail row draws: "2m ago", "Off", "Blocked" - the short ones
 * the design draws inside 250px.
 *
 * A "Not protected" detail is deliberately not among them. Those are the
 * verdict's reasons ("Configuration update failed", "Verification failed"), and
 * at rail width they truncate mid-word, which turns an actionable sentence into
 * an ellipsis. The row keeps the coloured phrase and the app pane's header
 * carries the reason in full - see `statusDetail`.
 */
function statusSuffix(status: AppStatus): string | undefined {
  if (status.kind === "protected") return status.since;
  if (status.kind === "not-routed") return status.detail;
  return undefined;
}

/** The same suffix, plus the reason the rail drops. For a surface with the room
 *  to print it. */
export function statusDetail(status: AppStatus): string | undefined {
  return status.kind === "not-protected" ? status.detail : statusSuffix(status);
}

export function Sidebar({
  orgName,
  onSwitchOrg,
  view,
  onNavigate,
  groups,
  master,
  onSelectApp,
  onToggleApp,
  onRefresh,
  refreshing,
  inventory,
}: {
  orgName: string;
  onSwitchOrg: () => void;
  view: SidebarView;
  onNavigate: (view: SidebarView) => void;
  groups: SidebarGroup[];
  /** The engine's switch, above the families it carries. Omit on a platform with
   * no proxy subsystem, where there is nothing to turn on and the card would
   * describe nothing. */
  master?: MasterRouting;
  /** Opens the per-app pane. */
  onSelectApp: (slug: string) => void;
  onToggleApp: (slug: string, next: boolean) => void;
  /** Re-run detection now, for the inventory card's Refresh / Try again. There is
   * no control for this while the list has rows: detection polls itself, so a
   * tool installed while the window is open appears on its own. The card keeps
   * one because a *failed* scan is a state the user may want to retry against
   * rather than wait out. Omitted leaves it out entirely, on the same rule the
   * Settings rows follow: a button that does nothing is worse than no button. */
  onRefresh?: () => void;
  /** A scan is in flight; the card's control refuses clicks and says so to
   * assistive technology. */
  refreshing?: boolean;
  /** What the last scan actually established. Omitted keeps the old behaviour of
   * rendering the list and nothing else. */
  inventory?: InventoryState;
}) {
  return (
    <nav
      aria-label="Main"
      // 256px fixed, sectioned the way the `sidebar` set (437:161, read
      // 2026-08-28) draws it: a 12px-padded header and nav each closed by a
      // 1px `base/border` bottom edge, then the app groups on 12/16 padding.
      // The old single 16px pad and its `hr` are gone from the file.
      //
      // 256 held the rail at 250 until 2026-08-28, on the strength of the
      // `Settings / Dimensions` annotated spec (191:79795) drawing 250 + 774
      // = 1024 while the then-current frames overflowed the window at 256.
      // The flow frames have been redrawn since - `overview-loading`
      // (228:85602) and `App/Claude-desktop` (228:89241) both fit 256 + 768
      // = 1024 cleanly - and the Dimensions frame still carries its old
      // pre-redraw internals, so it is the stale one now and the set's 256
      // stands.
      className="flex w-[256px] shrink-0 flex-col border-r border-base-border bg-base-card"
    >
      <div className="border-b border-base-border p-3">
        <OrgSwitcher name={orgName} onClick={onSwitchOrg} />
      </div>

      <div className="flex flex-col gap-1 border-b border-base-border p-3">
        <NavItem
          icon="layoutDashboard"
          label="Overview"
          active={view.kind === "overview"}
          onClick={() => onNavigate({ kind: "overview" })}
        />
        <NavItem
          icon="shieldCheck"
          label="Security events"
          active={view.kind === "security"}
          onClick={() => onNavigate({ kind: "security" })}
        />
        <NavItem
          icon="settings2"
          label="Settings"
          active={view.kind === "settings"}
          onClick={() => onNavigate({ kind: "settings" })}
        />
      </div>

      {/* 16px between groups, 8px inside one - `sidebar-group-list`. The
       * section scrolls on its own: the set draws a scroll indicator over this
       * region and the header and nav stay put above it. */}
      <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-3 py-4">
        {master && <MasterCard master={master} />}

        {inventory && inventory.kind !== "ok" ? (
          <InventoryState
            state={inventory}
            onRefresh={onRefresh}
            refreshing={refreshing}
          />
        ) : null}

        {groups.map((group) => (
          <div key={group.id} className="flex flex-col gap-2">
            {group.label && (
              // Label and counter, which is all the drawn eyebrow holds. A
              // family switch lived here briefly when the Families pane was
              // retired; it was removed on 2026-08-27 as a third control
              // over the same traffic the row switches and the master switch
              // already cover, on a rail the design draws without one.
              <div className="flex items-baseline justify-between gap-2">
                <h2 className="truncate font-mono text-base-xs font-medium uppercase leading-4 tracking-eyebrow text-base-muted-foreground">
                  {group.label}
                </h2>
                {/* Protected over total, drawn on every eyebrow
                 * (`Components / Sidenav`, read 2026-08-23). Derived from
                 * the rows so it can never disagree with them. Not
                 * uppercase: the drawn counter is Geist Mono Regular and
                 * reads "1 of 2". */}
                <span className="shrink-0 font-mono text-base-xs font-normal leading-4 text-base-muted-foreground">
                  {group.apps.filter((a) => a.status.kind === "protected").length} of{" "}
                  {group.apps.length}
                </span>
              </div>
            )}
            <ul className="flex flex-col gap-1">
              {group.apps.map((app) => (
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
        ))}
      </div>
    </nav>
  );
}

function OrgSwitcher({ name, onClick }: { name: string; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      // Radius 4 (`rounded-control`) on a `base/input` line, drawn by all three
      // set variants. Padding is 6/8 per the settings and app variants; the
      // overview one draws p-8 and loses 2 to 1.
      className="flex w-full items-center justify-between rounded-control border border-base-input bg-base-card px-1.5 py-2 shadow-base-2xs"
    >
      <span className="flex items-center gap-2">
        <Icon name="usersRound" size={16} />
        <span className="text-base-xs font-medium leading-4 text-base-foreground">{name}</span>
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
      // Radius 4, 6px uniform padding, and an active fill under `shadow/2xs` -
      // the settings and app variants' reading (the overview one pads 8/6 and
      // loses 2 to 1). Active text is `base/primary` on `base/background`,
      // which is the same #f9fafb the set's `sidebar-primary-foreground`
      // variable resolves to.
      className={`flex w-full items-center gap-2 rounded-control p-1.5 text-base-xs font-medium leading-4 ${
        active
          ? "border border-base-border bg-base-background text-base-primary shadow-base-2xs"
          : "text-base-foreground"
      }`}
    >
      <Icon name={icon} size={16} />
      {label}
    </button>
  );
}

/**
 * The engine's switch and its shell-environment sub-setting.
 *
 * Laid out label-over-description rather than the pane's label-beside-switch:
 * the rail is 256px, and the certificate warning is a sentence, not a phrase.
 * Above the app groups because that is what it governs - "everything below
 * stays off until this is on" is literally true of what follows it.
 */
function MasterCard({ master }: { master: MasterRouting }) {
  return (
    <div className="flex flex-col gap-2 rounded-md border border-base-border bg-base-card p-3">
      <div className="flex items-start justify-between gap-2">
        <p className="min-w-0 text-base-xs font-medium leading-4 text-base-foreground">
          Route traffic through Gate
        </p>
        <BaseSwitch
          on={master.on}
          label="Route traffic through Gate"
          busy={master.busy}
          onClick={() => master.onToggle(!master.on)}
        />
      </div>
      <p className="text-base-2xs leading-4 text-base-muted-foreground">
        {master.on
          ? master.caTrusted === false
            ? "Running, but the certificate is not trusted - nothing is being inspected"
            : "The local engine is running"
          : "Everything below stays off until this is on"}
      </p>

      {master.envExport && (
        <div className="flex flex-col gap-2 border-t border-base-border pt-2">
          <div className="flex items-start justify-between gap-2">
            <p className="min-w-0 text-base-xs leading-4 text-base-foreground">
              Also set shell environment variables
            </p>
            <BaseSwitch
              on={master.envExport.on}
              label="Also set shell environment variables"
              busy={master.busy}
              onClick={() => master.envExport?.onToggle(!master.envExport.on)}
            />
          </div>
          <p className="text-base-2xs leading-4 text-base-muted-foreground">
            Routes command-line tools too. Machine-wide: it reaches git and curl, not
            only your AI tools.
          </p>
        </div>
      )}
    </div>
  );
}

/**
 * Two targets in one row: the row opens the app's pane, the switch routes it.
 * The switch is a sibling rather than a child so a click on it never also
 * navigates.
 */
/**
 * The two states an empty app list can be in, told apart.
 *
 * Provisional layout: the Figma draws no empty inventory. It lives in the 256px
 * rail rather than the content pane because it is the *inventory's* state, and
 * moving it would leave the rail silently blank - which is the ambiguity this
 * exists to remove. Vertical space is ample even if width is not.
 */
function InventoryState({
  state,
  onRefresh,
  refreshing,
}: {
  state: Exclude<InventoryState, { kind: "ok" }>;
  onRefresh?: () => void;
  refreshing?: boolean;
}) {
  const failed = state.kind === "failed";
  return (
    <div
      role="status"
      className={`flex flex-col gap-2 rounded-md border p-3 ${
        failed ? "border-amber-200 bg-amber-50" : "border-base-border bg-base-card"
      }`}
    >
      <p className="text-sm font-medium leading-5 text-base-foreground">
        {failed ? "Couldn’t check for apps" : "No apps detected"}
      </p>
      <p className="text-base-xs leading-4 text-neutral-600">
        {failed
          ? // Not "no apps": the difference between "we looked and found none"
            // and "we could not look" is the whole point of this component.
            "Gate couldn’t read this device’s app list, so it doesn’t know what is installed. Nothing has been changed."
          : "Gate looked for supported AI apps and found none installed. Install one and refresh, and it will appear here."}
      </p>
      {state.kind === "none" && (
        // The scan time is what makes "none" an answer rather than a shrug.
        <p className="font-mono text-base-2xs leading-4 text-base-muted-foreground">
          Checked {state.scannedAt}
        </p>
      )}
      {onRefresh && (
        <button
          type="button"
          onClick={onRefresh}
          disabled={refreshing}
          className="flex h-8 items-center justify-center rounded-control border border-base-border bg-base-card px-3 text-base-xs font-medium tracking-button-xs text-base-primary shadow-base-btn-sm transition-colors hover:bg-gray-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary disabled:cursor-not-allowed disabled:opacity-50"
        >
          {refreshing ? "Checking…" : failed ? "Try again" : "Refresh"}
        </button>
      )}
    </div>
  );
}

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
      // Hover and selection share one treatment, read off `sidebar-menu-item`
      // state=selected (434:128, re-read 2026-08-26): a `base/background` fill
      // inside a 1px `base/border` under `shadow/xs`. That replaces the
      // neutral-100/200 pairing the retired row-hover variant drew. The border
      // is reserved while at rest so rows do not shift on hover. Drawn radius
      // is 4px (`rounded-control`); padding is the drawn `spacing/1-5`, 6px
      // uniform.
      className={`group flex w-full items-center gap-4 rounded-control border p-1.5 ${
        selected
          ? "border-base-border bg-base-background shadow-base-xs"
          : "border-transparent hover:border-base-border hover:bg-base-background"
      }`}
    >
      <button
        type="button"
        onClick={() => onSelect(app.slug)}
        aria-current={selected ? "page" : undefined}
        className="flex min-w-0 flex-1 items-center gap-2 rounded-sm text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
      >
        <span
          aria-hidden
          className="flex size-8 shrink-0 items-center justify-center rounded-control border border-white/[0.24] bg-black text-base-2xs font-medium text-white"
          // `logo-wrapper` (408:14180): the overlay pair is 24%, not the 32%
          // this had.
          style={{
            backgroundImage:
              "linear-gradient(180deg, rgba(255,255,255,0.24) 0%, rgba(0,0,0,0.24) 100%)",
          }}
        >
          {app.logo ?? app.name.charAt(0)}
        </span>
        <span className="flex min-w-0 flex-1 flex-col">
          <span
            className={`truncate text-base-xs font-medium leading-4 ${
              selected ? "text-base-primary" : "text-base-foreground group-hover:text-base-primary"
            }`}
          >
            {app.name}
          </span>
          <span className="truncate text-base-2xs font-medium leading-4">
            <span className={status.className}>{status.label}</span>
            {suffix && <span className="text-base-muted-foreground"> - {suffix}</span>}
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
