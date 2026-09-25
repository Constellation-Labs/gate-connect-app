import type { ReactNode } from "react";
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

/**
 * Overview and Settings, and that is the whole nav block - which is what the
 * Sidenav frame (408:15625) has always drawn.
 *
 * A third entry, `security`, sat between them from AG-578 until AG-853 moved the
 * live feed onto the Overview pane. It was recorded as an undrawn addition at
 * the time; removing it puts the rail back on the frame rather than away from
 * it. Anything that used to navigate here goes to the Overview's
 * `SECURITY_SECTION_ID` instead.
 */
export type SidebarView =
  | { kind: "overview" }
  | { kind: "settings" }
  /** An app row is selected and its detail pane is open. */
  | { kind: "app"; slug: string };

/**
 * What is actually happening to this app's traffic. The design draws three
 * (`status-label`, 434:136): "Protected", "Not protected", "Not routed". Each
 * renders as a coloured phrase plus an optional grey suffix.
 *
 * Three phrases, one per pairing of switch and outcome: on and working, on and
 * not working, off. Every way of being on and not working - drift, a process
 * that has to be reopened, a provider Gate does not inspect, a section only
 * partly routed, a certificate that blocks it - is `not-protected`, and its
 * `detail` says which. The rail prints the phrase alone and the app pane draws
 * the reason as a card. Until 2026-09-25 the rail had seven phrases besides
 * "Not installed", each argued as the only honest reading of its state; it was
 * cut back to the drawn three then, with the reason moved to the pane.
 */
export type AppStatus =
  | { kind: "protected"; since?: string }
  /** `detail` carries the reason ("Connection problem", "Config drifted",
   * "Reopen to finish"), which is what turns an amber phrase into something the
   * user can act on. See `lib/verdict.ts`. */
  | { kind: "not-protected"; detail?: string }
  | { kind: "not-routed"; detail?: string }
  /**
   * Detection did not find the app on this machine. Drawn only under the
   * rail's "Not installed" group, on a row that opens nothing - there is no
   * traffic to report and no route to change.
   *
   * No rail frame draws it. The tray's `Connect/full frame` (738:37377) draws a
   * "Not installed" section collapsed to a count, which is the nearest thing
   * the file has; the tray itself dropped that section at the user's request
   * (see `Tray`), and the rail lists the rows instead so it says what Gate can
   * route once they are there.
   */
  | { kind: "not-installed" };

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

/**
 * One figure on an app row: a reading, or the fact that one has not landed yet.
 *
 * The third case is the absence of the field altogether, and it is the important
 * one. A figure on screen is something Gate measured, so a row with nothing
 * behind a counter draws nothing rather than a `0` standing in for the answer
 * nobody gave - which on a list of app rows would read as a quiet day over
 * traffic Gate cannot see. Principle 6. The field below says what its own
 * absence covers.
 */
export type RowCount =
  /** A real reading. `0` is an answer and prints as one. */
  | {
      kind: "count";
      count: number;
      /** When the gateway computed it, as a local clock time, for a figure that
       * can be older than the moment it is drawn.
       *
       * Absent means "now": a count derived from the live feed is current by
       * construction and has nothing to disclose. A held reading does - it can be
       * a minute old, or the last thing that landed before the network went - and
       * a row with no room to print an age says it in a tooltip instead. An age
       * that has to be recomputed to stay true is why this is a clock time and
       * not a duration; `ActivityView.takenAt` gives the same reasoning. */
      measuredAt?: string;
    }
  /** A read that is actually running has not answered yet. */
  | { kind: "pending" };

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
  /**
   * The programs behind this row, shown on hover over its label.
   *
   * A row's visible text is a surface kind - "API", "Chat", "CLI" - which is
   * legible under its heading and still does not name anything the user could
   * go and open. That matters most for the rows with no config file behind
   * them: nothing else in the app says the word "Cowork". `lib/groups.ts`
   * carries the copy.
   *
   * A native `title`, like every other hover in this app (`AppPane`,
   * `SettingsPane`, `banners`). It is a supplement, never the only place a
   * fact lives - the pane draws the row's full description - so a viewer that
   * never shows it loses nothing load-bearing.
   */
  hint?: string;
  /** A toggle is in flight: the switch ignores clicks but keeps focus. */
  busy?: boolean;
  /**
   * Requests this app sent, from the last activity reading held for it.
   *
   * Drawn by the **tray** row, like `alerts`. Absent where no reading has landed:
   * `GET /v1/me/activity` answers for one tool at a time, so the tray opens on
   * what is on disk and refreshes what has gone stale rather than asking per row
   * per open - see `lib/toolMessages.ts`. A chat domain has none at all, its
   * traffic being unattributed at the gateway.
   */
  messages?: RowCount;
  /**
   * Blocked and flagged requests attributed to this app, from the live feed.
   *
   * Drawn by the **tray** row, not by the rail: the tray frames are the ones
   * that carry an activity line under the status (`Tray`'s docstring records
   * what they draw), and the 1024px window reports the same traffic on the app
   * pane, where there is room for the events themselves. It lives on the shared
   * row type because the two surfaces build their rows from one shape.
   *
   * Absent where the feed has no attribution to give: it keys events on the tool
   * slug, and a chat domain's traffic arrives unattributed on purpose -
   * `NewUiApp`'s `openDomain` note has the reason - so those rows have no
   * reading, permanently. An unreadable feed is the same case, and the Overview's
   * Security events section is the surface that says so.
   */
  alerts?: RowCount;
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
  /** The apps detection did not find. Its rows are display-only and its
   *  eyebrow carries no counter: "0 of 3" would score apps nobody has. */
  notInstalled?: true;
}

/*
 * The engine's own switch used to live here, above the families that ride on
 * it, as the one control the Figma does not draw. It is gone, and so is the
 * argument for it: routing is not a thing the user sets any more, it runs for
 * exactly as long as Gate Connect is open. There is no state the window can
 * reach that it has no control for, because there is nothing to control - the
 * rail is now the org header, Overview/Settings and the app groups, which is
 * what the drawn sidebar (`440:953`) has always been.
 *
 * The shell-environment sub-setting sat at the foot of the rail as a second
 * card and came out on 2026-09-16 for the same reason.
 * `proxy.env_export_opted_in` is untouched by either removal: the backend still
 * honours whatever it holds, there is just no control for it in the window.
 */

export const STATUS_TEXT: Record<AppStatus["kind"], { label: string; className: string }> = {
  protected: { label: "Protected", className: "text-green-600" },
  "not-protected": { label: "Not protected", className: "text-amber-600" },
  // Grey, not amber: `status-label` status=not-routed (434:134) resolves
  // `base/muted-foreground`, where not-protected beside it is amber-600.
  "not-routed": { label: "Not routed", className: "text-base-muted-foreground" },
  "not-installed": { label: "Not installed", className: "text-base-muted-foreground" },
};

/**
 * Does this row count as protected, for the counters and the topbar banner?
 *
 * Protected only. A row routed to a provider Gate does not inspect reads "Not
 * protected", so the counters count it out too, and the topbar says it is not
 * protected rather than contradicting the row.
 *
 * Four counters read this: the topbar's `protectedCount`, `RoutingCard`, the
 * tray's per-group fraction and the rail's. They were four separate
 * `kind === "protected"` filters and are one predicate, because four copies of
 * a pairing is four chances to disagree.
 */
export function countsAsProtected(status: AppStatus): boolean {
  return status.kind === "protected";
}

/**
 * The grey suffix a rail row draws: "2m ago", "Off" - the short ones the design
 * draws inside 250px.
 *
 * A `not-protected` detail is deliberately not among them. It is the reason
 * ("Configuration update failed", "Reopen to finish"), and the app pane draws it
 * as a card rather than as a suffix. The tray prints the same line as the rail.
 */
export function statusSuffix(status: AppStatus): string | undefined {
  if (status.kind === "protected") return status.since;
  if (status.kind === "not-routed") return status.detail;
  return undefined;
}

/**
 * The rail lists apps and says what is happening to each. **It does not route
 * them.**
 *
 * Each row carried a switch until 2026-09-22. Design removed it: the row drew
 * the app's state and a control for its intent on one line, and the two are
 * different questions (principle 2). Routing now happens on the app's own
 * pane, where there is room to say what the switch will do before it is
 * flipped - the consent question, the provider question, the drift review -
 * rather than firing them from a 250px row.
 *
 * `SidebarApp.on` survives and is still intent, not observation: the topbar's
 * counters read it to say how many apps the person has asked to route. It is
 * simply no longer drawn here.
 *
 * Not in the Figma yet; design is updating the frames.
 */
export function Sidebar({
  orgName,
  onSwitchOrg,
  view,
  onNavigate,
  groups,
  onSelectApp,
  onRefresh,
  refreshing,
  inventory,
}: {
  orgName: string;
  /** Omitted when the account cannot switch organizations, which is every
   *  API-key account. See `OrgSwitcher`. */
  onSwitchOrg?: () => void;
  view: SidebarView;
  onNavigate: (view: SidebarView) => void;
  groups: SidebarGroup[];
  /** Opens the per-app pane. */
  onSelectApp: (slug: string) => void;
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
                 * (`Components / Sidenav`, read 2026-08-23) except "Not
                 * installed", where "0 of 3" would score apps nobody has.
                 * Derived from the rows so it can never disagree with them.
                 * Not uppercase: the drawn counter is Geist Mono Regular and
                 * reads "1 of 2". */}
                {!group.notInstalled && (
                  <span className="shrink-0 font-mono text-base-xs font-normal leading-4 text-base-muted-foreground">
                    {group.apps.filter((a) => countsAsProtected(a.status)).length} of{" "}
                    {group.apps.length}
                  </span>
                )}
              </div>
            )}
            <ul className="flex flex-col gap-1">
              {group.apps.map((app) => (
                <AppRow
                  key={app.slug}
                  app={app}
                  // A pane left open on an app uninstalled since does not
                  // mark the display-only row it now sits on.
                  selected={!group.notInstalled && view.kind === "app" && view.slug === app.slug}
                  onSelect={group.notInstalled ? undefined : onSelectApp}
                />
              ))}
            </ul>
          </div>
        ))}

      </div>
    </nav>
  );
}

/**
 * The organization line, a control only when there is something to switch to.
 *
 * `onClick` is optional, and its absence is a real state rather than an
 * oversight: `/v1/me/orgs` answers "which organizations may this *user* act
 * on", which only a signed-in session can answer, and the gateway refuses an
 * API key for it outright ("X-Gate-Authorization must be `Bearer
 * <cognito-access-token>`"). An API key resolves to exactly one organization -
 * AG-572's contract says so and the activity reading proves it - so for those
 * accounts there is nothing to choose between.
 *
 * **The box is drawn either way**, which is the newer call (2026-09-16). It
 * used to drop the line, the ground and the elevation with the chevron, on the
 * argument that a bordered, elevated box that cannot be pressed is the one
 * thing in the rail claiming to be a control and lying. The frame draws the
 * box (`691:29773`: white, 1px `base/input`, 4px, 6/8, `shadow/2xs`) and an
 * account with one organization is a list of one, not an absence - so the row
 * reports the same fact in the same shape whichever account is signed in, and
 * a reader stops having to learn that the rail looks different on an API key.
 *
 * The CHEVRON still tracks pressability, because that is the part that says
 * "there is another one of these". Nothing here invents an affordance that
 * leads nowhere; it just stops the container from being the affordance.
 */
function OrgSwitcher({ name, onClick }: { name: string; onClick?: () => void }) {
  // Same box in both states; see above. Only the element and the chevron differ.
  const box =
    "flex w-full min-w-0 items-center justify-between gap-2 rounded-control border border-base-input bg-base-card px-1.5 py-2 shadow-base-2xs";
  // The rail is 256px wide and an organization name is not, so the name
  // truncates in BOTH states. It used to do so in neither, and then in only the
  // label, which made how a long name renders depend on which account was
  // signed in rather than on how long the name is.
  const line = (
    <span className="flex min-w-0 items-center gap-2" title={name}>
      <Icon name="usersRound" size={16} />
      <span className="truncate text-base-xs font-medium leading-4 tracking-label-12 text-base-foreground">
        {name}
      </span>
    </span>
  );

  if (!onClick) return <span className={box}>{line}</span>;

  return (
    <button
      type="button"
      onClick={onClick}
      // Named for a screen reader the way the tray's line is: the visible text
      // is the organization name alone, which says nothing about being able to
      // change it, and the chevron that does say so is decorative.
      aria-label={`Organization: ${name}. Switch organization`}
      className={box}
    >
      {line}
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
      className={`flex w-full items-center gap-2 rounded-control p-1.5 text-base-xs font-medium leading-4 tracking-label-12 ${
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
        <p className="text-base-2xs leading-4 text-base-muted-foreground">
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
}: {
  app: SidebarApp;
  selected: boolean;
  /** Omitted for a row that opens nothing - the "Not installed" group's. It
   *  renders as plain content with no hover, rather than a button that does
   *  nothing. */
  onSelect?: (slug: string) => void;
}) {
  const status = STATUS_TEXT[app.status.kind];
  const suffix = statusSuffix(app.status);
  const content = (
    <>
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
          title={app.hint}
          className={`truncate text-base-xs font-medium leading-4 tracking-label-12 ${
            selected
              ? "text-base-primary"
              : onSelect
                ? "text-base-foreground group-hover:text-base-primary"
                : "text-base-foreground"
          }`}
        >
          {app.name}
        </span>
        <span className="truncate text-base-2xs font-medium leading-4">
          <span className={status.className}>{status.label}</span>
          {suffix && <span className="text-base-muted-foreground"> - {suffix}</span>}
        </span>
      </span>
    </>
  );

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
          : onSelect
            ? "border-transparent hover:border-base-border hover:bg-base-background"
            : "border-transparent"
      }`}
    >
      {onSelect ? (
        <button
          type="button"
          onClick={() => onSelect(app.slug)}
          aria-current={selected ? "page" : undefined}
          className="flex min-w-0 flex-1 items-center gap-2 rounded-sm text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
        >
          {content}
        </button>
      ) : (
        <span className="flex min-w-0 flex-1 items-center gap-2">{content}</span>
      )}
    </li>
  );
}
