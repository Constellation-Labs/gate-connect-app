import { useRef } from "react";
import type { ReactNode, RefObject } from "react";
import { BaseSwitch, Skeleton, StatusTile } from "./base";
import { GateAiLogoMark } from "./GateAiLogoMark";
import { Icon } from "./Icon";
import { OutlineIconButton } from "./Topbar";
import { OverflowMenu } from "./OverflowMenu";
import type { MenuAction } from "./OverflowMenu";
import { countsAsRouted, STATUS_TEXT, statusDetail } from "./Sidebar";
import type { RowCount, SidebarGroup } from "./Sidebar";
import { routingState, showsFraction } from "../../lib/routingState";

/**
 * The tray popover (Figma `Flows / Tray` 694:34005, read 2026-08-28): a
 * 400x700 quick-status surface the tray icon toggles, beside the full 1024x720
 * window. Header lockup with an "Expand app" hand-off, one routing status card,
 * the same grouped rows the window's rail draws - at tray width, with a
 * status line per row - the command-line tools switch, and a footer naming the organization in front of an overflow
 * menu.
 *
 * Presentational, like `Sidebar`: every piece of state arrives as a prop so
 * the tray shell (`TrayApp`) owns data fetching and dispatch. Row and group
 * types are the rail's own (`SidebarGroup` / `SidebarApp`) so the two
 * surfaces cannot describe one app two ways.
 *
 * Deviations from the drawn frames, each deliberate:
 *
 * - **No "Not installed" section.** `Connect/full frame` (738:37377) draws one,
 *   collapsed to a count. Removed at the user's request on 2026-09-23: a quick
 *   status view is about what is routing, and a tool that is not on the machine
 *   has nothing to report.
 * - **The routing card renders no switch.** Every tray frame draws that switch
 *   at opacity 0, so what the frame *renders* is a status card; the switches
 *   that act live on the rows, and the engine's own control stays in the full
 *   app. If the invisible switch was reserved space rather than a decision,
 *   that is the designer's to say.
 * - **Rows draw the whole activity line, but its halves come from different
 *   places.** The frames draw "345 messages · 23 alerts" under each status. The
 *   alerts are live: the feed (AG-578) attributes each blocked or flagged
 *   request to a tool slug, and the tray listens to it for the per-row alert
 *   counts. The messages are *held*: `GET /v1/me/activity` answers for one tool at
 *   a time inside a throttle bucket keyed on the source address, so a read per
 *   row per open is the one fan-out that budget cannot take - `lib/toolMessages`
 *   opens on the readings already on disk and refreshes what has gone stale
 *   instead. A held figure discloses its age in the line's tooltip, the row
 *   having no width to print it. Rows the gateway cannot attribute - the chat
 *   domains, permanently - keep the two-line shape the design also draws (the
 *   compact `Other tools` rows in `Connect/routing`).
 * - **The routing card's unhappy states are inferred** - only the equivalent of
 *   "partly routed" and "Gate is protecting you" are drawn. Since AG-913 the
 *   words come from `lib/routingState`, shared with the topbar banner so the
 *   two surfaces cannot describe one reading differently. The drawn sub-line
 *   is "On/Off · N of M tools routing"; the On/Off half is gone, because the
 *   master switch it reported is gone - see `RoutingCard`.
 * - **Contact support is in the menu**, as `744:38201` draws it. It was omitted
 *   for as long as the address behind it 404'd; support resolved to the
 *   dashboard's own Overview page on 2026-09-07 (that is where the support
 *   floating action button lives), so the omission went with the reason for it.
 *   `Topbar` had already shipped its copy of this entry, which left one drawn
 *   menu item present on one surface and absent on the other.
 */

export function Tray({
  engine,
  groups,
  cli,
  orgName,
  onSwitchOrg,
  signedOut,
  accountUnread,
  onToggleApp,
  onExpand,
  menuOpen,
  onMenuToggle,
  onMenuSelect,
  dialog,
  rootRef,
}: {
  /** The engine's observed state. Omit while the first proxy read is in
   * flight, and the card is omitted with it - a status card with no reading
   * behind it would be a claim.
   *
   * Named for the engine rather than a master, because there is no master:
   * `running` is whether the launch enable succeeded, not a setting anybody
   * chose. `starting` is whether that enable is still in flight, which is
   * all that separates "not yet" from "didn't". */
  engine?: { running: boolean; starting: boolean };
  groups: SidebarGroup[];
  /** The shell-environment channel, drawn as its own card ("Command-line
   * tools"). Absent on Linux, where those variables are the system proxy and
   * cannot be declined separately. */
  /** Reported, not offered: the window's Settings pane owns this control.
   *  See {@link CliCard}. */
  cli?: { on: boolean };
  orgName: string;
  /** Open the organization selector (AG-582). The tray does not own one - it
   *  hands over to the window, which does. Omit and the footer draws the org as
   *  a plain label, which is what it did before the selector was reachable. */
  onSwitchOrg?: () => void;
  /** No usable credential: the tray cannot route anything, so it says so and
   * hands over to the full app, where setup lives. Not drawn; inferred. */
  signedOut?: boolean;
  /** The account could not be READ, which is a different fact from having
   *  none and must not borrow its copy: "Sign in to get started" told a
   *  signed-in user they had no account whenever the keychain read failed.
   *  Not drawn - inferred from principle 6, like every other unread state. */
  accountUnread?: boolean;
  onToggleApp: (slug: string, next: boolean) => void;
  /** The header's "Expand app": reveal the full window and dismiss the tray. */
  onExpand: () => void;
  menuOpen: boolean;
  onMenuToggle: () => void;
  onMenuSelect: (action: MenuAction) => void;
  /** The dialog covering the popover, if any - drift review, close-apps
   * offer. Same slot contract as `AppShell`. */
  dialog?: ReactNode;
  /** The popover's own root, so the shell can move focus into it when the
   *  window is revealed.
   *
   *  A tray window is shown and hidden rather than created and destroyed, so
   *  nothing moves focus on its own: a keyboard user opened the popover and
   *  focus was still in whatever they were doing before. Held by the shell
   *  because the reveal is the shell's event, not this component's. */
  rootRef?: RefObject<HTMLDivElement>;
}) {
  /** The menu's trigger, so the panel can hand focus back to it on close. */
  const menuTrigger = useRef<HTMLButtonElement>(null);

  return (
    // `tabular-nums` on the root, not per figure. "Always use tabular nums on
    // numbers" is design's standing rule (2026-09-04): the point is that a
    // column of counts, percentages and currency lines up, and Geist's
    // proportional digits do not. Set once here so no figure added later can
    // miss it - the same argument the `label/copy` tracking tokens make.
    <div
      ref={rootRef}
      // Focusable only programmatically: the shell focuses this on reveal so the
      // first Tab lands inside the popover, and -1 keeps it out of the tab order
      // itself so it is never a stop of its own.
      tabIndex={-1}
      // `relative` for `Modal`: its scrim is `absolute inset-0` and the panel's
      // height cap is `max-h-full` against it, so the scrim needs a positioned
      // ancestor to measure. Without one it resolved against the initial
      // containing block, which in a popover happens to be the same box - right
      // by coincidence rather than construction, and only until something wraps
      // this. `AppShell` carries it for the same reason and says so.
      className="relative flex h-screen w-full flex-col bg-base-background tabular-nums outline-none"
    >
      <header className="flex h-16 shrink-0 items-center justify-between border-b border-base-border bg-base-card px-4">
        <span className="flex items-center gap-2.5">
          <GateAiLogoMark height={27} />
          {/* The Gate AI lockup, not the topbar's: the frame inks "Gate" in
           * the mark's own navy and "Connect" in its accent blue
           * (694:34020/21), neither of which is a `base.*` or ramp token. */}
          <span className="flex items-center gap-[2px] text-base font-semibold leading-6 tracking-[-0.16px]">
            <span className="text-[#002554]">Gate</span>
            <span className="text-[#3646e7]">Connect</span>
          </span>
        </span>
        <button
          type="button"
          onClick={onExpand}
          className="flex h-8 items-center gap-2 rounded-md border border-base-input bg-base-card px-3 text-base-xs font-medium leading-4 tracking-button-xs text-base-primary shadow-base-btn-sm transition-colors hover:bg-neutral-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
        >
          <Icon name="expand" size={16} />
          Expand app
        </button>
      </header>

      {accountUnread ? (
        <AccountUnreadNote onExpand={onExpand} />
      ) : signedOut ? (
        <SignedOutNote onExpand={onExpand} />
      ) : (
        <div className="flex min-h-0 flex-1 flex-col gap-5 px-4 pt-4">
          {engine && (
            <RoutingCard
              running={engine.running}
              starting={engine.starting}
              groups={groups}
            />
          )}

          <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto pb-4">
            {groups.map((group) => (
              <TrayGroup key={group.id} group={group} onToggleApp={onToggleApp} />
            ))}


            {cli && <CliCard cli={cli} />}
          </div>
        </div>
      )}

      <footer className="relative flex h-14 shrink-0 items-center justify-between border-t border-base-border bg-base-card px-4">
        {/* The frame draws this as a label beside a `users` glyph (744:38188),
            and AG-582 asks the same line to open the organization selector. So
            it stays visually the label it draws and becomes a control: no
            chevron, no button chrome, just a hover ground and a focus ring, at
            the drawn type. Without a handler it renders as the plain label the
            frame draws, so nothing here invents an affordance that leads
            nowhere. */}
        {onSwitchOrg ? (
          <button
            type="button"
            onClick={onSwitchOrg}
            aria-label={`Organization: ${orgName}. Switch organization`}
            className="-mx-1.5 flex min-w-0 items-center gap-2 rounded-control px-1.5 py-1 text-sm font-medium leading-5 tracking-label-14 text-base-foreground transition-colors hover:bg-gray-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
          >
            <Icon name="users" size={20} />
            <span className="truncate">{orgName}</span>
          </button>
        ) : (
          <span className="flex min-w-0 items-center gap-2 text-sm font-medium leading-5 tracking-label-14 text-base-foreground">
            <Icon name="users" size={20} />
            <span className="truncate">{orgName}</span>
          </span>
        )}
        <OutlineIconButton
          buttonRef={menuTrigger}
          radius="md"
          icon="ellipsis"
          label="More"
          onClick={onMenuToggle}
          expanded={menuOpen}
        />
        {menuOpen && (
          <OverflowMenu
            surface="tray"
            onSelect={onMenuSelect}
            onDismiss={onMenuToggle}
            triggerRef={menuTrigger}
          />
        )}
      </footer>

      {dialog}
    </div>
  );
}

/**
 * The engine's observed state over the rows it carries, drawn as the success
 * tile recipe on green or amber (`Connect/routing` 694:34185 vs
 * `Connect/partial` 694:34024). Counts derive from the rows so the card can
 * never disagree with the switches under it; "tools routing" is the drawn
 * phrase.
 *
 * **The denominator is intent, not every row.** It counted every row, chat
 * domains included, and that docstring read like a drawn decision - it predates
 * the chat rows existing. They ship `enabled: false` and no family switch flips
 * them (`groups.ts`, `cascadeTargets` returns false for a `chat` member), so
 * counting them made green require routing a session-cookie surface nobody
 * asked for: the card sat on amber "0 of 5 tools routing" forever, and after
 * the topbar moved to an intent filter the two shells disagreed by exactly the
 * switched-off chat rows - green "3 of 3 Apps" above, amber "0 of 5" under the
 * same tray icon.
 *
 * Principle 2 applied to a fraction, the same way the topbar does it and the
 * same way `Group.cascadeDesired` already did it for the family switch: `on` is
 * what the user asked for, `protected` is what happened. A row nobody turned on
 * is not a gap. An opted-in chat row still counts, and can still go green -
 * `proxyMemberStatus` reports `protected` once it routes - so this hides
 * nothing the user chose.
 */
function RoutingCard({
  running,
  starting,
  groups,
}: {
  running: boolean;
  starting: boolean;
  groups: SidebarGroup[];
}) {
  const all = groups.flatMap((g) => g.apps);
  const apps = all.filter((a) => a.on);
  const routed = apps.filter((a) => countsAsRouted(a.status)).length;
  // The same reading the topbar banner takes, from the same module (AG-913).
  // This card used to word it differently - "Partially routed" against the
  // banner's "partly routing your apps" - and to fold "nothing was asked for"
  // into "Not protected", which reports a fault the user caused on purpose.
  const state = routingState(routed, apps.length);
  const { tone, icon } = state;
  // Switched on, out of every app on the rail - NOT routed out of requested,
  // which is what the headline above already answers. Same call as the topbar
  // banner's, from the same module, so the two cannot drift apart again.
  //
  // This does NOT line up with the group eyebrows below, which count routed
  // over group. It lines up on the denominator and not on the numerator, and
  // "on" is the only thing in the words that says so. `routingState` carries
  // the argument and the design question.
  const fraction = showsFraction(all.length)
    ? `${apps.length} of ${all.length} tools on`
    : "";
  // This line used to lead with "On" / "Off", from the engine's running flag
  // standing in for a master switch. There is no such switch - routing is on
  // for exactly as long as Gate Connect is open - so "On" was unfalsifiable
  // furniture, and with nothing switched on it printed "On" directly under
  // "No apps are set to route". (That state draws a fraction of its own now,
  // "0 of 8 tools on", where it used to draw nothing at all.)
  //
  // The engine failing to come up is still worth printing, so it keeps its
  // half in the words already decided for it elsewhere: a launch enable that
  // did not complete reads "Didn’t start", never "Off", because "Off" sends
  // the reader looking for a control that is not there.
  //
  // Only once the enable has settled, though. The engine is also not running
  // while the startup thread is still getting to it - an OAuth refresh and a
  // reconcile run first - and "Didn’t start" over that reports a failure at the
  // moment the user opens the tray to check. "Starting…" is inferred, not
  // drawn, under the same licence as the other unhappy states in this file's
  // header; it is owed to design.
  const engineNote = running ? "" : starting ? "Starting…" : "Didn’t start";
  const detail = [engineNote, fraction]
    .filter(Boolean)
    .join(" · ");
  return (
    <div
      className={`flex shrink-0 items-center gap-3 rounded-md border bg-base-card p-3 ${
        tone === "green" ? "border-green-300" : "border-amber-300"
      }`}
    >
      <StatusTile tone={tone} icon={icon} size={36} />
      <div className="flex min-w-0 flex-col gap-0.5">
        <h1 className="text-sm font-medium leading-5 text-base-foreground">
          {state.headline}
        </h1>
        {detail && (
          <p className="text-base-xs leading-4 tracking-label-12 text-base-muted-foreground">
            {detail}
          </p>
        )}
      </div>
    </div>
  );
}

/** One eyebrow-labelled card of rows - the rail's grouping at tray width, with
 * the rows inside one bordered card under a rule apiece rather than the rail's
 * free-standing hover rows (738:37552). */
function TrayGroup({
  group,
  onToggleApp,
}: {
  group: SidebarGroup;
  onToggleApp: (slug: string, next: boolean) => void;
}) {
  return (
    <section className="flex shrink-0 flex-col gap-2">
      {group.label && (
        <div className="flex items-baseline justify-between gap-2">
          {/* `mono/eyebrow` at the tray's drawn 14px (738:37554), against the
           * rail's 12. Tracking is the same 8%. */}
          <h2 className="truncate font-mono text-sm font-medium uppercase leading-5 tracking-eyebrow-14 text-base-muted-foreground">
            {group.label}
          </h2>
          <span className="shrink-0 font-mono text-base-xs font-normal leading-4 text-base-muted-foreground">
            {group.apps.filter((a) => countsAsRouted(a.status)).length} of{" "}
            {group.apps.length}
          </span>
        </div>
      )}
      <ul className="divide-y divide-base-border overflow-hidden rounded-md border border-base-border bg-base-card shadow-base-xs">
        {group.apps.map((app) => (
          <li key={app.slug} className="flex items-center gap-4 p-2">
            <span className="flex min-w-0 flex-1 items-center gap-3">
              <AppTile name={app.name} logo={app.logo} />
              <span className="flex min-w-0 flex-1 flex-col">
                {/* Same hover as the rail's rows, and for the same reason: the
                    label is a surface kind, and the desktop apps behind these
                    rows are named nowhere else in the app. `SidebarApp.hint`
                    carries the copy. */}
                <span
                  title={app.hint}
                  className="truncate text-base-xs font-medium leading-4 tracking-label-12 text-base-foreground"
                >
                  {app.name}
                </span>
                <StatusLine app={app} />
                {(app.messages || app.alerts) && (
                  <ActivityLine messages={app.messages} alerts={app.alerts} />
                )}
              </span>
            </span>
            <BaseSwitch
              on={app.on}
              // The row's own name, same as the rail's rows (`Sidebar.tsx`).
              // The eyebrow used to be a vendor and a row used to be a surface,
              // so "CLI" alone named three of these and the heading was what
              // told them apart. A row is an app now and its name is already
              // unique, while the heading above it is a band - so prefixing it
              // named the switch "Apps Claude", which is not a thing on anyone's
              // machine.
              label={app.name}
              busy={app.busy}
              onClick={() => onToggleApp(app.slug, !app.on)}
            />
          </li>
        ))}
      </ul>
    </section>
  );
}

/** The coloured phrase plus grey qualifier, in the tray's own type. Uses
 * `statusDetail` rather than the rail's suffix: the frames draw the
 * "Not protected" qualifier too ("- 3d ago", 738:37562), and 368px rows have
 * the room the 250px rail does not. */
function StatusLine({ app }: { app: SidebarGroup["apps"][number] }) {
  const status = STATUS_TEXT[app.status.kind];
  const suffix = statusDetail(app.status);
  return (
    <span className="truncate text-base-2xs font-medium leading-4">
      <span className={status.className}>{status.label}</span>
      {suffix && <span className="text-base-muted-foreground"> - {suffix}</span>}
    </span>
  );
}

/**
 * The drawn activity line: "345 messages · 23 alerts", traffic first and then the
 * subset of it that fired something.
 *
 * Grey, on the status line's own ramp. A blocked request is Gate doing its job,
 * not this app failing, so a fault colour here would put a second amber phrase
 * under the one line on the row entitled to report a fault. A measured zero of
 * messages says so in words: the frames draw digits but their own empty-state
 * copy is a phrase ("No recent messages"), and a bare `0` under a status line
 * reads as a figure that failed to arrive rather than as an answer. A zero of
 * alerts draws no half at all, because no frame words one: Claude Desktop's
 * quiet row is "No recent messages" alone.
 *
 * Either half can be absent, and an absent half takes its separator with it - a
 * row never draws a dangling dot for a figure it does not have. The message half
 * is missing until a reading lands; a chat domain's row has neither, permanently.
 *
 * **The age is on the line, not in it.** A held figure can be a minute old and the
 * row has no width for "measured 14:03", so `measuredAt` becomes the line's
 * tooltip. Saying nothing at all would let a stale number read as a live one,
 * which is the failure mode principle 6 exists to prevent; printing it would cost
 * the figures their room.
 */
function ActivityLine({
  messages,
  alerts,
}: {
  messages?: RowCount;
  alerts?: RowCount;
}) {
  const half = (count: RowCount, label: (n: number) => string) =>
    count.kind === "pending" ? (
      <Skeleton className="h-3 w-16" />
    ) : (
      <span className="truncate">{label(count.count)}</span>
    );
  const measuredAt = messages?.kind === "count" ? messages.measuredAt : undefined;
  const shownAlerts = alerts?.kind === "count" && alerts.count === 0 ? undefined : alerts;
  return (
    <span
      title={measuredAt && `Messages measured ${measuredAt}`}
      className="flex min-w-0 items-center gap-1 text-base-2xs leading-4 text-base-muted-foreground"
    >
      {messages && half(messages, messagesLabel)}
      {messages && shownAlerts && <span aria-hidden>·</span>}
      {shownAlerts && half(shownAlerts, alertsLabel)}
    </span>
  );
}

/** Thousands separated: a four-figure message count is ordinary, and `1032` at
 *  `base-2xs` is not scannable. */
function messagesLabel(count: number): string {
  if (count === 0) return "No messages";
  return count === 1 ? "1 message" : `${count.toLocaleString()} messages`;
}

function alertsLabel(count: number): string {
  return count === 1 ? "1 alert" : `${count.toLocaleString()} alerts`;
}

/** The 32px `logo-wrapper` tile, treatment shared with the rail's `AppRow`. */
function AppTile({ name, logo }: { name: string; logo?: ReactNode }) {
  return (
    <span
      aria-hidden
      className="flex size-8 shrink-0 items-center justify-center rounded-control border border-white/[0.24] bg-black text-base-2xs font-medium text-white"
      style={{
        backgroundImage:
          "linear-gradient(180deg, rgba(255,255,255,0.28) 0%, rgba(0,0,0,0.28) 100%)",
      }}
    >
      {logo ?? name.charAt(0)}
    </span>
  );
}

/** The shell-environment channel as the tray draws it (735:37341), with the
 * frame's own copy - shorter than the rail card's, and naming the mechanism
 * (`HTTPS_PROXY`) outright.
 *
 * **A status card, not a switch**, which is the same call the routing card makes
 * one section up and for the same reason: the tray reports what the window
 * decides, and it introduces no concept of its own. Two switches for one
 * machine-wide setting is what AG-893 reported, and the window's Settings pane
 * is where a setting belongs.
 *
 * The frame draws a switch here. So does every tray frame for the routing card,
 * at opacity 0 - see this file's header. The drawn control is kept as the
 * drawn LAYOUT and rendered as state, rather than as a second control that can
 * disagree with the first. */
function CliCard({ cli }: { cli: { on: boolean } }) {
  return (
    <div className="flex shrink-0 items-center justify-between gap-4 rounded-md border border-base-border bg-base-card py-3 pl-3 pr-2">
      <div className="flex min-w-0 flex-col gap-0.5">
        <p className="text-sm font-medium leading-5 tracking-label-14 text-base-foreground">Command-line tools</p>
        <p className="text-base-xs leading-4 text-base-muted-foreground">
          Sets HTTPS_PROXY for your whole shell, so OpenCode and other terminal tools route too.
        </p>
      </div>
      {/* The same vocabulary the rows use for a state they report rather than
        * offer, so the card reads as a reading and not as a control someone
        * failed to wire. */}
      <span className="shrink-0 text-base-xs font-medium leading-4 tracking-label-12 text-base-muted-foreground">
        {cli.on ? "On" : "Off"}
      </span>
    </div>
  );
}

/**
 * What the tray says with no usable credential. Not drawn - the Tray page
 * assumes a signed-in install - but a popover that painted empty groups over
 * "No organization" would read as broken rather than signed out. Setup lives
 * in the full window, so the card hands over rather than reproducing it.
 */
/**
 * The account read failed. Deliberately not `SignedOutNote`: that one tells
 * the user to sign in, and a user whose credential is merely unreadable is
 * already signed in - on macOS this is the dismissed-keychain-prompt case
 * CLAUDE.md describes. Says what happened and offers the surface that can
 * retry, rather than a sentence about setup.
 */
function AccountUnreadNote({ onExpand }: { onExpand: () => void }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 px-8 text-center">
      <h1 className="text-sm font-medium leading-5 text-base-foreground">
        Your account couldn&apos;t be read
      </h1>
      <p className="text-base-xs leading-4 tracking-label-12 text-base-muted-foreground">
        Gate Connect could not reach your stored credential, so it cannot tell
        what is routed. Open the app window to try again.
      </p>
      <button
        type="button"
        onClick={onExpand}
        className="flex h-8 items-center gap-2 rounded-md border border-base-input bg-base-card px-3 text-base-xs font-medium leading-4 tracking-button-xs text-base-primary shadow-base-btn-sm transition-colors hover:bg-neutral-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
      >
        Open Gate Connect
      </button>
    </div>
  );
}

function SignedOutNote({ onExpand }: { onExpand: () => void }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 px-8 text-center">
      <h1 className="text-sm font-medium leading-5 text-base-foreground">
        Sign in to get started
      </h1>
      <p className="text-base-xs leading-4 text-base-muted-foreground">
        Gate Connect needs a Gate account or API key before it can route your
        tools. Sign in from the app window.
      </p>
      <button
        type="button"
        onClick={onExpand}
        className="flex h-8 items-center gap-2 rounded-md border border-base-input bg-base-card px-3 text-base-xs font-medium leading-4 tracking-button-xs text-base-primary shadow-base-btn-sm transition-colors hover:bg-neutral-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
      >
        Open Gate Connect
      </button>
    </div>
  );
}
