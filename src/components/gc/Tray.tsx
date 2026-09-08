import { useEffect, useRef, useState } from "react";
import type { ReactNode, RefObject } from "react";
import type { FeedState } from "../../lib/api";
import { BaseSwitch, Skeleton, StatusTile } from "./base";
import { GateAiLogoMark } from "./GateAiLogoMark";
import { Icon } from "./Icon";
import type { IconName } from "./Icon";
import { OutlineIconButton } from "./Topbar";
import { STATUS_TEXT, statusDetail } from "./Sidebar";
import type { RowCount, SidebarGroup } from "./Sidebar";

/**
 * The tray popover (Figma `Flows / Tray` 694:34005, read 2026-08-28): a
 * 400x700 quick-status surface the tray icon toggles, beside the full 1024x720
 * window. Header lockup with an "Expand app" hand-off, one master status card,
 * the same grouped rows the window's rail draws - at tray width, with a
 * status line per row - a collapsed "Not installed" section, the command-line
 * tools switch, and a footer naming the organization in front of an overflow
 * menu.
 *
 * Presentational, like `Sidebar`: every piece of state arrives as a prop so
 * the tray shell (`TrayApp`) owns data fetching and dispatch. Row and group
 * types are the rail's own (`SidebarGroup` / `SidebarApp`) so the two
 * surfaces cannot describe one app two ways.
 *
 * Deviations from the drawn frames, each deliberate:
 *
 * - **The master card renders no switch.** Every tray frame draws that switch
 *   at opacity 0, so what the frame *renders* is a status card; the switches
 *   that act live on the rows, and the engine's own control stays in the full
 *   app. If the invisible switch was reserved space rather than a decision,
 *   that is the designer's to say.
 * - **Rows draw the whole activity line, but its halves come from different
 *   places.** The frames draw "345 messages · 23 alerts" under each status. The
 *   alerts are live: the feed (AG-578) attributes each blocked or flagged
 *   request to a tool slug, and the tray already listens to it for the security
 *   card. The messages are *held*: `GET /v1/me/activity` answers for one tool at
 *   a time inside a throttle bucket keyed on the source address, so a read per
 *   row per open is the one fan-out that budget cannot take - `lib/toolMessages`
 *   opens on the readings already on disk and refreshes what has gone stale
 *   instead. A held figure discloses its age in the line's tooltip, the row
 *   having no width to print it. Rows the gateway cannot attribute - the chat
 *   domains, permanently - keep the two-line shape the design also draws (the
 *   compact `Other tools` rows in `Connect/routing`).
 * - **The master card's off state is inferred** - only "Partially routed" and
 *   "Gate is protecting you" are drawn - following the status vocabulary:
 *   "Not protected" in amber, with the drawn "On/Off · N of M tools routing"
 *   sub-line carrying the intent.
 * - **Contact support is in the menu**, as `744:38201` draws it. It was omitted
 *   for as long as the address behind it 404'd; support resolved to the
 *   dashboard's own Overview page on 2026-09-07 (that is where the support
 *   floating action button lives), so the omission went with the reason for it.
 *   `Topbar` had already shipped its copy of this entry, which left one drawn
 *   menu item present on one surface and absent on the other.
 */

export type TrayMenuAction = "dashboard" | "support" | "docs" | "quit";

/** A tool the detection scan saw but found not installed - the collapsed
 * "Not installed" section's rows. No switch: there is nothing to route, and a
 * connect would materialise a config for a tool the user does not have. */
export interface TrayNotInstalledApp {
  slug: string;
  name: string;
  logo?: ReactNode;
}

export function Tray({
  master,
  groups,
  notInstalled,
  notInstalledOpen,
  onToggleNotInstalled,
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
  security,
  recovery,
  reopen,
  dialog,
  rootRef,
}: {
  /** The engine's observed state. Omit while the first proxy read is in
   * flight, and the card is omitted with it - a status card with no reading
   * behind it would be a claim. */
  master?: { on: boolean };
  groups: SidebarGroup[];
  notInstalled: TrayNotInstalledApp[];
  notInstalledOpen: boolean;
  onToggleNotInstalled: () => void;
  /** The shell-environment channel, drawn as its own card ("Command-line
   * tools"). Absent on Linux, where those variables are the system proxy and
   * cannot be declined separately. */
  cli?: { on: boolean; busy?: boolean; onToggle: (next: boolean) => void };
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
  onMenuSelect: (action: TrayMenuAction) => void;
  /** The live security-event feed, compacted to what fits a 400px popover
   * (AG-578): how many blocked or flagged events this session has seen, and
   * whether the feed is actually connected. Omitted while the first read is in
   * flight, and the card is omitted with it - a count with no reading behind it
   * would be a claim, which is the same rule `master` follows one prop up. */
  security?: { state: FeedState; count: number; onOpen: () => void };
  /** An interrupted routing operation that has not finished (AG-570).
   *
   * The tray gets the action, not just the fact: AC 4 requires the recovery to
   * stay reachable from Overview, tool detail *and* the tray, and a tray that
   * only reported it would be the one surface that says something is wrong and
   * offers nothing. Resume is the whole-batch call here rather than the window's
   * per-tool walk - a 400px popover has no room for a progress list, and the
   * details it would name live in the window, which `onReview` opens.
   *
   * Omitted when nothing is outstanding, and the card goes with it. */
  recovery?: {
    names: string[];
    busy?: boolean;
    onResume: () => void;
    onReview: () => void;
  };
  /** Tools whose configuration is applied and whose running process has not
   *  picked it up (AG-566 AC 3, which asks for this on tool detail, Overview
   *  *and* the tray).
   *
   *  Same division as `recovery`: the tray carries the fact and the one action,
   *  and the per-tool routes and stages stay in the window, which has the width
   *  for them. Omitted when nothing needs reopening. */
  /** The tools waiting to be reopened, and the route they are still on.
   *
   *  `route` is the address for a SINGLE waiting tool and must be null for two
   *  or more: they can be on different routes, so one address under several
   *  names would be wrong about at least one of them. The card tests the plural
   *  first rather than trusting this, but the invariant is the caller's. */
  reopen?: { names: string[]; route?: string | null; onReopen: () => void };
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
      className="flex h-screen w-full flex-col bg-base-background tabular-nums outline-none"
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
          {master && <MasterCard on={master.on} groups={groups} />}
          {/* Above the master card's siblings and below the card itself: it is
            * the most urgent thing on the popover, and it is also a statement
            * about the routing the card above describes. */}
          {recovery && <RecoveryCard recovery={recovery} />}
          {/* Below the recovery card: an operation that did not finish outranks
            * one that finished and is waiting on the user. */}
          {reopen && <ReopenCard reopen={reopen} />}
          {security && <SecurityCard security={security} />}

          <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto pb-4">
            {groups.map((group) => (
              <TrayGroup key={group.id} group={group} onToggleApp={onToggleApp} />
            ))}

            {notInstalled.length > 0 && (
              <NotInstalledSection
                apps={notInstalled}
                open={notInstalledOpen}
                onToggle={onToggleNotInstalled}
              />
            )}

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
          <TrayMenu
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
 * phrase, and it counts every row - chat domains included.
 */
function MasterCard({ on, groups }: { on: boolean; groups: SidebarGroup[] }) {
  const apps = groups.flatMap((g) => g.apps);
  const routed = apps.filter((a) => a.status.kind === "protected").length;
  const all = apps.length > 0 && routed === apps.length;
  const { tone, icon, title } = all
    ? { tone: "green" as const, icon: "shieldCheck" as IconName, title: "Gate is protecting you" }
    : routed > 0
      ? { tone: "amber" as const, icon: "shieldBan" as IconName, title: "Partially routed" }
      : // Not drawn: the page stops at "partially". The vocabulary's amber
        // phrase covers it, and the sub-line below carries whether that is
        // intent (Off) or circumstance (On with nothing routing).
        { tone: "amber" as const, icon: "shieldBan" as IconName, title: "Not protected" };
  return (
    <div
      className={`flex shrink-0 items-center gap-3 rounded-md border bg-base-card p-3 ${
        all ? "border-green-300" : "border-amber-300"
      }`}
    >
      <StatusTile tone={tone} icon={icon} size={36} />
      <div className="flex min-w-0 flex-col gap-0.5">
        <h1 className="text-sm font-medium leading-5 text-base-foreground">{title}</h1>
        <p className="text-base-xs leading-4 tracking-label-12 text-base-muted-foreground">
          {on ? "On" : "Off"} · {routed} of {apps.length} tools routing
        </p>
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
            {group.apps.filter((a) => a.status.kind === "protected").length} of{" "}
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
                <span className="truncate text-base-xs font-medium leading-4 tracking-label-12 text-base-foreground">
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
              // The card's eyebrow in front of the row label, same as the
              // rail's rows: "CLI" alone names three of these, and the heading
              // that separates them is a sibling rather than a parent.
              label={group.label ? `${group.label} ${app.name}` : app.name}
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
 * under the one line on the row entitled to report a fault. A measured zero says
 * so in words: the frames draw digits but their own empty-state copy is a phrase
 * ("No recent messages"), and a bare `0` under a status line reads as a figure
 * that failed to arrive rather than as an answer.
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
  return (
    <span
      title={measuredAt && `Messages measured ${measuredAt}`}
      className="flex min-w-0 items-center gap-1 text-base-2xs leading-4 text-base-muted-foreground"
    >
      {messages && half(messages, messagesLabel)}
      {messages && alerts && <span aria-hidden>·</span>}
      {alerts && half(alerts, alertsLabel)}
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
  if (count === 0) return "No alerts";
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

/**
 * The tools detection saw and found absent, collapsed to a count
 * (`Connect/full frame` 738:37377: "Not installed · 8 ˅"). Only the collapsed
 * state is drawn; expanding lists the same row anatomy without a switch,
 * because there is nothing to route and a connect would write a config for a
 * tool the user does not have.
 */
function NotInstalledSection({
  apps,
  open,
  onToggle,
}: {
  apps: TrayNotInstalledApp[];
  open: boolean;
  onToggle: () => void;
}) {
  return (
    <section className="flex shrink-0 flex-col gap-2">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        className="flex w-full items-baseline justify-between gap-2 rounded-sm text-base-muted-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
      >
        <span className="font-mono text-sm font-medium uppercase leading-5 tracking-eyebrow-14">
          Not installed
        </span>
        <span className="flex items-center gap-3">
          <span className="font-mono text-sm font-normal leading-5">{apps.length}</span>
          <Icon
            name="chevronDown"
            size={20}
            className={`transition-transform ${open ? "rotate-180" : ""}`}
          />
        </span>
      </button>
      {open && (
        <ul className="divide-y divide-base-border overflow-hidden rounded-md border border-base-border bg-base-card shadow-base-xs">
          {apps.map((app) => (
            <li key={app.slug} className="flex items-center gap-3 p-2">
              <AppTile name={app.name} logo={app.logo} />
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate text-base-xs font-medium leading-4 tracking-label-12 text-base-foreground">
                  {app.name}
                </span>
                <span className="truncate text-base-2xs font-medium leading-4 text-base-muted-foreground">
                  Not installed
                </span>
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/** The shell-environment switch as the tray draws it (735:37341), with the
 * frame's own copy - shorter than the rail card's, and naming the mechanism
 * (`HTTPS_PROXY`) outright. */
function CliCard({
  cli,
}: {
  cli: { on: boolean; busy?: boolean; onToggle: (next: boolean) => void };
}) {
  return (
    <div className="flex shrink-0 items-center justify-between gap-4 rounded-md border border-base-border bg-base-card py-3 pl-3 pr-2">
      <div className="flex min-w-0 flex-col gap-0.5">
        <p className="text-sm font-medium leading-5 tracking-label-14 text-base-foreground">Command-line tools</p>
        <p className="text-base-xs leading-4 text-base-muted-foreground">
          Sets HTTPS_PROXY for your whole shell, so OpenCode and other terminal tools route too.
        </p>
      </div>
      <BaseSwitch
        on={cli.on}
        label="Command-line tools"
        busy={cli.busy}
        onClick={() => cli.onToggle(!cli.on)}
      />
    </div>
  );
}

/** The footer's overflow menu (744:38192), opening upward over the list. Same
 * anatomy as the topbar's, plus the Quit entry the tray owes its users - it is
 * the popover surface, and the drawn menu carries it in destructive ink with
 * no external-link glyph (the rendered frame drops the one its metadata
 * carries: quitting does not leave the app). */
function TrayMenu({
  onSelect,
  onDismiss,
  triggerRef,
}: {
  onSelect: (action: TrayMenuAction) => void;
  /** Close without choosing: Escape, or a click anywhere else. */
  onDismiss: () => void;
  /** The button that opened this. Focus returns to it on close - all three
   *  exits unmount the panel, and with the focused element gone `activeElement`
   *  falls to `<body>`, so the next Tab restarted from the top of the popover
   *  rather than from the control the user was just on. */
  triggerRef?: RefObject<HTMLButtonElement>;
}) {
  const external: { action: TrayMenuAction; icon: IconName; label: string }[] = [
    { action: "dashboard", icon: "layoutDashboard", label: "Visit dashboard" },
    { action: "support", icon: "headset", label: "Contact support" },
    { action: "docs", icon: "bookOpenText", label: "Read Gate docs" },
  ];
  const panel = useRef<HTMLDivElement>(null);
  /**
   * Which item is the menu's single tab stop (roving tabindex).
   *
   * `role="menu"` is meant to be ONE stop, with the arrows moving inside it. As
   * four plain buttons every item was in the tab order, so Tab past the last one
   * walked into the app-list switches - visible behind the open menu, and
   * click-blocked by the scrim, so the focus ring went somewhere the pointer
   * could not follow.
   */
  const [current, setCurrent] = useState(0);

  /**
   * Focus the first item when the menu opens.
   *
   * Without it the menu was unreachable by keyboard: it opens from a button, so
   * focus stayed on that button and Tab walked into the list *behind* the menu
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
   * not. `preventDefault` on the arrows because this popover's content scrolls:
   * without it Down would move the selection *and* scroll the list behind.
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
    <>
      {/* An invisible scrim, so a click outside closes the menu *and nothing
          else happens*. Without it the menu sat over the app list with no
          dismissal at all: clicking a row still visible beside it toggled that
          app's routing with the menu open, which is a routing change the user
          did not ask for while trying to dismiss something. The design draws no
          scrim, and this one is not a visual change - it paints nothing. */}
      <div aria-hidden className="fixed inset-0 z-10" onClick={onDismiss} />
    <div
      ref={panel}
      role="menu"
      // Without a name this announces as a bare "menu". Named for the control
      // that opens it, so it reads "More, menu".
      aria-label="More"
      onKeyDown={onKeyDown}
      className="absolute bottom-12 right-4 z-20 w-56 rounded-md border border-base-border bg-base-card p-[9px] shadow-base-md"
    >
      {external.map(({ action, icon, label }, i) => (
        <button
          key={action}
          type="button"
          role="menuitem"
          // Roving: only the current item is a tab stop, so Tab leaves the menu
          // as a unit and the arrows move within it.
          tabIndex={current === i ? 0 : -1}
          onClick={() => onSelect(action)}
          className="flex h-8 w-full items-center justify-between rounded-control px-1.5 text-base-foreground transition-colors hover:bg-gray-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
        >
          <span className="flex items-center gap-2">
            <Icon name={icon} size={14} />
            <span className="text-base-xs font-medium leading-4 tracking-label-12">{label}</span>
          </span>
          <Icon name="squareArrowOutUpRight" size={12} className="text-neutral-500" />
        </button>
      ))}
      <button
        type="button"
        role="menuitem"
        tabIndex={current === external.length ? 0 : -1}
        onClick={() => onSelect("quit")}
        className="flex h-8 w-full items-center gap-2 rounded-control px-1.5 text-red-600 transition-colors hover:bg-gray-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
      >
        <Icon name="logOut" size={14} />
        <span className="text-base-xs font-medium leading-4">Quit Gate Connect</span>
      </button>
    </div>
    </>
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

/** The security feed, at popover size.
 *
 * A count and a connection state, and a way into the full window. Deliberately
 * not a list: at 400px a row would have to drop either the category or the
 * tool, and an event that cannot say what fired or where is not worth the
 * space.
 *
 * **It does not open the security pane.** `onOpen` is the header's `expand`:
 * `reveal_popover_window` shows the main window wherever it was last left and
 * takes no pane argument, so the user lands on whatever pane they were on. The
 * sentence here used to say the full feed was one click away, which is one
 * click plus finding it. Wiring it properly needs the reveal to carry a
 * destination; until then this card promises less.
 *
 * The count is "this session", not "today". The feed buffers what it has received
 * since the app started, and calling that a daily total would be a claim about
 * traffic the app was not running for.
 */
/**
 * The interrupted-routing notice at tray width.
 *
 * The window's banner carries per-tool rows, a retry per row and a read-only
 * review. None of that fits 400px, and cutting it down would produce a second,
 * shorter account of the same operation - the thing `lib/recovery.ts` exists to
 * prevent. So this says the one thing the tray is for (something did not
 * finish, here is what is waiting), offers the batch resume, and sends the rest
 * to the window.
 */
function RecoveryCard({
  recovery,
}: {
  recovery: {
    names: string[];
    busy?: boolean;
    onResume: () => void;
    onReview: () => void;
  };
}) {
  const many = recovery.names.length > 1;
  return (
    <div className="flex shrink-0 flex-col gap-2 rounded-md border border-amber-300 bg-amber-50 p-3">
      <div className="flex items-center gap-3">
        <StatusTile tone="amber" icon="refresh" size={36} />
        <div className="flex min-w-0 flex-col gap-0.5">
          <h2 className="text-sm font-medium leading-5 text-base-foreground">
            Routing didn’t finish
          </h2>
          <p className="truncate text-base-xs leading-4 text-amber-900/80">
            {recovery.names.join(", ")} {many ? "are" : "is"} still waiting
          </p>
        </div>
      </div>
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={recovery.onResume}
          disabled={recovery.busy}
          className="flex h-8 flex-1 items-center justify-center rounded-md border border-amber-300 bg-base-card px-3 text-base-xs font-medium leading-4 tracking-button-xs text-amber-900 shadow-base-btn-sm transition-colors hover:bg-amber-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-amber-600 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {recovery.busy ? "Resuming…" : "Resume now"}
        </button>
        <button
          type="button"
          onClick={recovery.onReview}
          className="flex h-8 items-center justify-center rounded-md px-3 text-base-xs font-medium leading-4 tracking-button-xs text-amber-900 underline decoration-amber-300 transition-colors hover:bg-amber-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-amber-600"
        >
          Review details
        </button>
      </div>
    </div>
  );
}

/**
 * The reopen notice at tray width.
 *
 * The window draws two routes per tool and a stage each; this says which tools
 * are waiting and offers the one action a 400px popover can carry. "Reopen
 * tool" here opens the same close-and-verify conversation the window uses -
 * Gate cannot start a terminal tool itself, so what the button does is deal
 * with the process that is in the way.
 */
function ReopenCard({
  reopen,
}: {
  reopen: { names: string[]; route?: string | null; onReopen: () => void };
}) {
  const many = reopen.names.length > 1;
  return (
    <div className="flex shrink-0 flex-col gap-2 rounded-md border border-amber-300 bg-amber-50 p-3">
      <div className="flex items-center gap-3">
        <StatusTile tone="amber" icon="refresh" size={36} />
        <div className="flex min-w-0 flex-col gap-0.5">
          <h2 className="text-sm font-medium leading-5 text-base-foreground">
            Reopen to finish
          </h2>
          {/* AG-584 asks the pending change to name **the route in use**, and
              this sentence used to gesture at it - "the route it started with"
              describes an address without saying which. `Verdict.route_in_use`
              is that address, set exactly when the reason is `reopen_required`.
              Absent for two or more tools, which can be on two different routes:
              one address under both names would be wrong about at least one of
              them, and the phrase is true for any number. */}
          {/* AG-584 asks the pending change to name **the route in use**, and
              this sentence used to gesture at it - "the route it started with"
              describes an address without saying which. `Verdict.route_in_use`
              is that address, set exactly when the reason is `reopen_required`.

              `many` is tested FIRST, so the singular verb and the type agree
              without depending on a rule kept in another file: the caller only
              passes a route when one tool is waiting, and if that ever changes
              this reads "Claude Code, Codex are on the route they started with"
              rather than "... is still on <one address>".

              Not truncated. The address is the payload of the sentence, and at
              this width "Claude Code is still on " leaves roughly 26 characters
              - less than the relay routes we actually produce, so `truncate`
              cut the one thing the AC asks to be named. It wraps instead, with
              `break-all` because a URL has no spaces to break at. */}
          <p className="text-base-xs leading-4 text-amber-900/80">
            {many || !reopen.route ? (
              <>
                {reopen.names.join(", ")} {many ? "are" : "is"} on the route{" "}
                {many ? "they" : "it"} started with
              </>
            ) : (
              <>
                {reopen.names[0]} is still on{" "}
                <span className="break-all font-medium">{reopen.route}</span>
              </>
            )}
          </p>
        </div>
      </div>
      <button
        type="button"
        onClick={reopen.onReopen}
        className="flex h-8 items-center justify-center rounded-md border border-amber-300 bg-base-card px-3 text-base-xs font-medium leading-4 tracking-button-xs text-amber-900 shadow-base-btn-sm transition-colors hover:bg-amber-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-amber-600"
      >
        Reopen tool
      </button>
    </div>
  );
}

function SecurityCard({
  security,
}: {
  security: { state: FeedState; count: number; onOpen: () => void };
}) {
  const feed = FEED_LABEL[security.state];
  return (
    <button
      type="button"
      onClick={security.onOpen}
      className="flex w-full items-center justify-between gap-3 rounded-md border border-base-border bg-base-card p-3 text-left shadow-base-sm transition-colors hover:bg-neutral-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
    >
      <span className="flex items-center gap-2.5">
        <Icon name="shieldCheck" size={20} />
        <span className="text-sm font-medium leading-5 text-base-foreground">
          {security.count === 0
            ? "No security events"
            : `${security.count} security event${security.count === 1 ? "" : "s"}`}
        </span>
      </span>
      <span
        role="status"
        aria-label={`Event feed ${feed.label}`}
        className={`inline-block rounded-xs px-1.5 py-0.5 font-mono text-base-xs font-medium uppercase leading-4 tracking-label ${feed.className}`}
      >
        {feed.label}
      </span>
    </button>
  );
}

/** Same three states and the same colours the pane draws. Duplicated as a
 *  constant rather than imported from `SecurityPane`, which would pull the whole
 *  window pane into the tray bundle for three strings. */
const FEED_LABEL: Record<FeedState, { label: string; className: string }> = {
  live: { label: "Live", className: "bg-green-100 text-green-900" },
  reconnecting: { label: "Reconnecting", className: "bg-amber-100 text-amber-900" },
  offline: { label: "Offline", className: "bg-gray-100 text-neutral-700" },
};
