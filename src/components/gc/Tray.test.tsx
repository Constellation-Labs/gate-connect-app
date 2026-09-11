import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { Tray } from "./Tray";
import type { SidebarGroup } from "./Sidebar";

const noop = () => {};

const GROUPS: SidebarGroup[] = [
  {
    id: "anthropic",
    label: "Anthropic",
    apps: [
      {
        slug: "claude-code",
        name: "Claude Code",
        status: { kind: "protected", since: "2m ago" },
        on: true,
      },
      {
        slug: "claude-web",
        name: "Claude Desktop",
        status: { kind: "not-routed", detail: "Off" },
        on: false,
      },
    ],
  },
];

/** Every required prop, so a test can re-render with one of them changed -
 *  which is how the menu's focus-return is observed. */
function trayProps(
  overrides: Partial<Parameters<typeof Tray>[0]> = {},
): Parameters<typeof Tray>[0] {
  return {
    master: { on: true },
    groups: GROUPS,
    notInstalled: [],
    notInstalledOpen: false,
    onToggleNotInstalled: noop,
    orgName: "Acme Engineering",
    onToggleApp: noop,
    onExpand: noop,
    menuOpen: false,
    onMenuToggle: noop,
    onMenuSelect: noop,
    ...overrides,
  };
}

function renderTray(overrides: Partial<Parameters<typeof Tray>[0]> = {}) {
  return render(<Tray {...trayProps(overrides)} />);
}

afterEach(cleanup);

type Row = SidebarGroup["apps"][number];

/** The first group with one row carrying the given figures, the other bare. */
const withFigures = (figures: Pick<Row, "messages" | "alerts">) => [
  {
    ...GROUPS[0],
    apps: [{ ...GROUPS[0].apps[0], ...figures }, GROUPS[0].apps[1]],
  },
];
const withAlerts = (alerts: Row["alerts"]) => withFigures({ alerts });

/** The named app's own row, so a figure cannot be matched off a neighbouring row
 *  or off the security card. */
const rowOf = (name: string) => screen.getByText(name).closest("li");
const row = (name: string) => rowOf(name)?.textContent ?? "";

describe("the master status card", () => {
  it("reads protecting when every row is routed", () => {
    renderTray({
      groups: [
        {
          id: "anthropic",
          label: "Anthropic",
          apps: [
            { slug: "a", name: "A", status: { kind: "protected" }, on: true },
            { slug: "b", name: "B", status: { kind: "protected" }, on: true },
          ],
        },
      ],
    });
    expect(
      screen.getByRole("heading", { name: "Gate is protecting you" }),
    ).toBeTruthy();
    expect(screen.getByText("On · 2 of 2 tools routing")).toBeTruthy();
  });

  it("reads partially routed when only some rows the user asked for are", () => {
    renderTray({
      groups: [
        {
          id: "anthropic",
          label: "Anthropic",
          apps: [
            { slug: "a", name: "A", status: { kind: "protected" }, on: true },
            { slug: "b", name: "B", status: { kind: "not-routed" }, on: true },
          ],
        },
      ],
    });
    expect(
      screen.getByRole("heading", { name: "Partially routed" }),
    ).toBeTruthy();
    expect(screen.getByText("On · 1 of 2 tools routing")).toBeTruthy();
  });

  /**
   * The denominator is intent, so a row nobody switched on is not a gap.
   *
   * This is the fixture the bug lived on: `claude-web` is a chat surface, it
   * ships off, and no family switch flips it (`cascadeTargets` returns false for
   * a `chat` member). Counting it meant green required routing a session-cookie
   * surface nobody asked for, so the card was pinned to amber "1 of 2" forever
   * and disagreed with the topbar - which filters by intent - by exactly that
   * row. Load-bearing: drop the `.filter((a) => a.on)` in `MasterCard` and this
   * goes back to "Partially routed".
   */
  it("does not count a row the user never switched on", () => {
    renderTray();
    expect(
      screen.getByRole("heading", { name: "Gate is protecting you" }),
    ).toBeTruthy();
    expect(screen.getByText("On · 1 of 1 tools routing")).toBeTruthy();
  });

  it("reads not protected with nothing routing, carrying the Off intent", () => {
    // The off state is not drawn; this pins the inferred vocabulary so a
    // redesign replaces it deliberately rather than by accident.
    renderTray({
      master: { on: false },
      groups: [
        {
          id: "anthropic",
          label: "Anthropic",
          apps: [
            { slug: "a", name: "A", status: { kind: "not-routed" }, on: true },
          ],
        },
      ],
    });
    expect(screen.getByRole("heading", { name: "Not protected" })).toBeTruthy();
    expect(screen.getByText("Off · 0 of 1 tools routing")).toBeTruthy();
  });

  /**
   * Nothing asked for is not a gap either, so no fraction is printed.
   *
   * With the denominator on intent, switching everything off reaches an empty
   * ratio by the user's own action rather than only on a machine with no tools.
   * "0 of 0 tools routing" reports a fault they caused deliberately, with both
   * halves of the ratio meaningless.
   */
  it("prints no ratio when the user has asked for nothing", () => {
    renderTray({
      master: { on: false },
      groups: [
        {
          id: "anthropic",
          label: "Anthropic",
          apps: [
            { slug: "a", name: "A", status: { kind: "not-routed" }, on: false },
          ],
        },
      ],
    });
    expect(screen.getByText("Off · No apps set to route")).toBeTruthy();
    expect(screen.queryByText(/0 of 0/)).toBeNull();
  });

  it("renders no switch: the drawn card is a status, not a control", () => {
    renderTray();
    // Row switches remain; nothing is named for the master.
    expect(
      screen.queryByRole("switch", { name: /route traffic/i }),
    ).toBeNull();
  });
});

/**
 * The count needs a scope on screen, and "recent" is the only one that is true.
 *
 * Stated as a bare absolute, "No security events" sat beside an Overview
 * reporting blocked traffic in the last 24 hours and read as a broken feed - and
 * the LIVE pill next to it made that reading worse, not better. The scope lived
 * in the component's docstring and nowhere the user could see it.
 *
 * The first attempt at a scope was "since Gate Connect started", and that is
 * the claim these tests now pin *against*: the figure is
 * `securityFeed.events.length`, an array capped at `FEED_CAPACITY` with the
 * oldest evicted and emptied outright whenever the credential changes. So a
 * busy machine would read "200 events since Gate Connect started" with the true
 * number in the thousands, and two seconds after an org switch it would read
 * none. "Recent" survives the cap, the clear and a genuinely quiet run, which
 * is why the word went into the count itself rather than onto a second line.
 *
 * An earlier version of this docstring asserted the launch scope and said the
 * card "has to say so", which is the exact sentence the component stopped
 * saying - it was left behind when the copy was fixed, contradicted by the
 * inline comment three lines below it.
 */
describe("the security card", () => {
  it("scopes an empty count rather than claiming none ever", () => {
    renderTray({ security: { state: "live", count: 0, onOpen: noop } });

    expect(screen.getByText("No recent security events")).toBeTruthy();
    // Not an absolute, and not a run-length claim either: the buffer is capped
    // and is emptied on a credential change, so "since Gate Connect started"
    // would be its own overclaim.
    expect(screen.queryByText(/Since Gate Connect started/)).toBeNull();
  });

  it("scopes a non-empty count the same way", () => {
    renderTray({ security: { state: "live", count: 3, onOpen: noop } });

    expect(screen.getByText("3 recent security events")).toBeTruthy();
  });

  /**
   * Principle 6's last step: an offline feed is not reading, so its zero is not
   * a reading. "No recent security events" beside an OFFLINE pill asserts a
   * quiet machine when what actually happened is that nobody looked - the same
   * class of overclaim as the two absolutes this card already dropped, just
   * quieter for being technically the count of an empty buffer.
   */
  it("says the feed is unavailable rather than reporting none while offline", () => {
    renderTray({ security: { state: "offline", count: 0, onOpen: noop } });

    expect(screen.getByText("Security events unavailable")).toBeTruthy();
    expect(screen.queryByText(/No recent security events/)).toBeNull();
  });

  /** Reconnecting still counts: the buffer it counts is real, and the pill
   *  beside it already says the stream is catching up. */
  it("keeps a real count while reconnecting", () => {
    renderTray({ security: { state: "reconnecting", count: 2, onOpen: noop } });

    expect(screen.getByText("2 recent security events")).toBeTruthy();
  });

  it("counts one event in the singular", () => {
    renderTray({ security: { state: "live", count: 1, onOpen: noop } });

    expect(screen.getByText("1 recent security event")).toBeTruthy();
  });
});

describe("the group rows", () => {
  it("draws the eyebrow with its protected-over-total counter", () => {
    renderTray();
    expect(screen.getByRole("heading", { name: "Anthropic" })).toBeTruthy();
    expect(screen.getByText("1 of 2")).toBeTruthy();
  });

  it("phrases a row as the coloured status plus grey qualifier", () => {
    renderTray();
    expect(screen.getByText("Protected")).toBeTruthy();
    expect(screen.getByText("- 2m ago")).toBeTruthy();
  });

  it("dispatches the switch from intent, not observed state", () => {
    // The lib/groups.ts bug this guards: a drifted tool observes
    // not-protected but its switch says what the user asked for, so
    // clicking must send the opposite of `on`, not of the status.
    const onToggleApp = vi.fn();
    renderTray({
      groups: [
        {
          id: "anthropic",
          label: "Anthropic",
          apps: [
            { slug: "codex", name: "Codex", status: { kind: "drifted" }, on: true },
          ],
        },
      ],
      onToggleApp,
    });
    // The card's eyebrow in front of the row label: "CLI" and its siblings name
    // a surface, and the heading is what says whose.
    screen.getByRole("switch", { name: "Anthropic Codex" }).click();
    expect(onToggleApp).toHaveBeenCalledWith("codex", false);
  });
});

describe("the not-installed section", () => {
  const NOT_INSTALLED = [
    { slug: "opencode", name: "OpenCode" },
    { slug: "openclaw", name: "OpenClaw" },
  ];

  it("collapses to a count", () => {
    renderTray({ notInstalled: NOT_INSTALLED });
    const toggle = screen.getByRole("button", { name: /not installed/i });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(toggle.textContent).toContain("2");
    expect(screen.queryByText("OpenCode")).toBeNull();
  });

  it("expands to rows without switches: there is nothing to route", () => {
    renderTray({ notInstalled: NOT_INSTALLED, notInstalledOpen: true });
    expect(screen.getByText("OpenCode")).toBeTruthy();
    // One switch per installed row only - the two absent tools add none.
    expect(screen.getAllByRole("switch")).toHaveLength(GROUPS[0].apps.length);
  });

  it("is absent entirely when detection found everything installed", () => {
    renderTray();
    expect(screen.queryByRole("button", { name: /not installed/i })).toBeNull();
  });
});

describe("the command-line tools card", () => {
  it("dispatches the shell-environment toggle", () => {
    const onToggle = vi.fn();
    renderTray({ cli: { on: false, onToggle } });
    screen.getByRole("switch", { name: "Command-line tools" }).click();
    expect(onToggle).toHaveBeenCalledWith(true);
  });

  it("is absent where the channel is not separable", () => {
    renderTray();
    expect(
      screen.queryByRole("switch", { name: "Command-line tools" }),
    ).toBeNull();
  });
});

describe("the reopen notice", () => {
  it("names the route the tool is still on", () => {
    // AG-584: a pending change shows Needs attention with Reopen required, **the
    // route in use**, and Reopen tool. The first and last were here; the
    // sentence used to gesture at the route - "the route it started with" -
    // without saying which address that is.
    renderTray({
      reopen: { names: ["Claude Code"], route: "http://127.0.0.1:8123/anthropic", onReopen: vi.fn() },
    });
    expect(screen.getByText(/Claude Code is still on/)).toBeTruthy();
    expect(screen.getByText("http://127.0.0.1:8123/anthropic")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Close tool" })).toBeTruthy();
  });

  it("falls back to the phrase when no route was measured", () => {
    // What one waiting tool now looks like in practice: `route_in_use` is null
    // on every verdict the backend sends, because nothing can read where a
    // running process is pointed. The card keeps the `route` prop for a future
    // reading, and must read correctly without one - naming an endpoint here
    // from anything less than a measurement is a claim about the user's
    // traffic.
    renderTray({ reopen: { names: ["Claude Code"], route: null, onReopen: vi.fn() } });
    expect(
      screen.getByText(/Claude Code is on the route it started with/),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Close tool" })).toBeTruthy();
  });

  it("does not truncate the address it exists to name", () => {
    // At this width "Claude Code is still on " leaves roughly 26 characters, and
    // the relay routes we produce are longer - so `truncate` cut the one thing
    // the AC asks to be named, which is less use than the vague phrase it
    // replaced. It wraps instead.
    const route = "http://127.0.0.1:8123/anthropic";
    renderTray({ reopen: { names: ["Claude Code"], route, onReopen: vi.fn() } });

    const address = screen.getByText(route);
    expect(address.className).toContain("break-all");
    expect(address.closest("p")?.className).not.toContain("truncate");
  });

  it("keeps the plural phrase even if a caller passes a route for several tools", () => {
    // The singular is the caller's invariant (`reopenPending.length === 1`), and
    // this card must not depend on it being kept one file away: testing `many`
    // first means a later change there reads as the plural phrase rather than
    // "Claude Code, Codex is still on <one address>".
    renderTray({
      reopen: {
        names: ["Claude Code", "Codex"],
        route: "http://127.0.0.1:8123/anthropic",
        onReopen: vi.fn(),
      },
    });
    expect(
      screen.getByText(/Claude Code, Codex are on the route they started with/),
    ).toBeTruthy();
    expect(screen.queryByText("http://127.0.0.1:8123/anthropic")).toBeNull();
  });

  it("keeps the phrase when several tools wait, since their routes can differ", () => {
    // One address under two names would be wrong about at least one of them.
    renderTray({
      reopen: { names: ["Claude Code", "Codex"], route: null, onReopen: vi.fn() },
    });
    expect(
      screen.getByText(/Claude Code, Codex are on the route they started with/),
    ).toBeTruthy();
  });

  it("reopens on the card's own action", () => {
    const onReopen = vi.fn();
    renderTray({ reopen: { names: ["Codex"], route: null, onReopen } });
    screen.getByRole("button", { name: "Close tool" }).click();
    expect(onReopen).toHaveBeenCalledTimes(1);
  });
});

describe("the footer", () => {
  it("names the organization", () => {
    renderTray();
    expect(screen.getByText("Acme Engineering")).toBeTruthy();
  });

  it("the organization line opens the selector when there is one to open", () => {
    const onSwitchOrg = vi.fn();
    renderTray({ onSwitchOrg });
    // Named for a screen reader, which gets the label and the action in one
    // string: the visible text is only the org name, so "Acme Engineering" on
    // its own says nothing about being able to change it.
    screen
      .getByRole("button", { name: /Organization: Acme Engineering\. Switch organization/ })
      .click();
    expect(onSwitchOrg).toHaveBeenCalledTimes(1);
  });

  it("draws the organization as the plain label the frame draws when there is nothing to switch to", () => {
    // An API-key account holds no org locally, so the selector would open on
    // nothing. The footer goes back to the label `744:38188` draws.
    renderTray();
    expect(screen.getByText("Acme Engineering")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Switch organization/ })).toBeNull();
  });

  it("focuses the first item when the menu opens, so the keyboard can reach it", () => {
    // `role="menu"` promises arrow-key navigation, and the menu had none. It
    // opens from a button, so focus stayed on the button and Tab walked the list
    // *behind* the menu rather than into it.
    renderTray({ menuOpen: true, onMenuSelect: vi.fn() });
    expect(document.activeElement?.textContent).toBe("Visit dashboard");
  });

  it("moves between items with the arrow keys, wrapping at both ends", () => {
    renderTray({ menuOpen: true, onMenuSelect: vi.fn() });
    const menu = screen.getByRole("menu");

    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(document.activeElement?.textContent).toBe("Contact support");
    fireEvent.keyDown(menu, { key: "ArrowUp" });
    expect(document.activeElement?.textContent).toBe("Visit dashboard");
    // Wrapping is what a menu does, and the last entry is the destructive one,
    // so arriving there by pressing Up once is deliberate rather than a slip.
    fireEvent.keyDown(menu, { key: "ArrowUp" });
    expect(document.activeElement?.textContent).toBe("Quit Gate Connect");
    fireEvent.keyDown(menu, { key: "Home" });
    expect(document.activeElement?.textContent).toBe("Visit dashboard");
    fireEvent.keyDown(menu, { key: "End" });
    expect(document.activeElement?.textContent).toBe("Quit Gate Connect");
  });

  it("is a single tab stop, so Tab leaves the menu rather than walking behind it", () => {
    // `role="menu"` promises one stop with the arrows moving inside it. As four
    // plain buttons every item was tabbable, so Tab past the last one moved
    // focus to the app-list switches - visible behind the open menu and
    // click-blocked by the scrim, so the focus ring went where the pointer
    // could not follow.
    renderTray({ menuOpen: true, onMenuSelect: vi.fn() });
    const items = screen.getAllByRole("menuitem");

    expect(items.map((el) => el.getAttribute("tabindex"))).toEqual(["0", "-1", "-1", "-1"]);

    // The stop moves with the arrows: still exactly one, on the focused item.
    fireEvent.keyDown(screen.getByRole("menu"), { key: "End" });
    expect(items.map((el) => el.getAttribute("tabindex"))).toEqual(["-1", "-1", "-1", "0"]);
  });

  it("gives focus back to the trigger when it closes", () => {
    // Every exit unmounts the panel, so the focused element goes with it and
    // `activeElement` falls to `<body>` - the next Tab would restart from the
    // top of the popover instead of the control the user was just on.
    const { rerender } = renderTray({ menuOpen: true, onMenuSelect: vi.fn() });
    expect(document.activeElement?.textContent).toBe("Visit dashboard");

    rerender(<Tray {...trayProps({ menuOpen: false, onMenuSelect: vi.fn() })} />);

    expect(document.activeElement?.getAttribute("aria-label")).toBe("More");
  });

  it("names the menu, so it does not announce as a bare menu", () => {
    renderTray({ menuOpen: true, onMenuSelect: vi.fn() });
    expect(screen.getByRole("menu", { name: "More" })).toBeTruthy();
  });

  it("closes on Escape without choosing anything", () => {
    const onMenuSelect = vi.fn();
    const onMenuToggle = vi.fn();
    renderTray({ menuOpen: true, onMenuSelect, onMenuToggle });

    fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });

    expect(onMenuToggle).toHaveBeenCalledTimes(1);
    expect(onMenuSelect).not.toHaveBeenCalled();
  });

  it("closes on a click outside, and that click does nothing else", () => {
    // The bug the scrim fixes: the menu sat over the app list with no dismissal,
    // so clicking a row still visible beside it toggled that app's routing while
    // trying to dismiss the menu - a routing change nobody asked for.
    const onMenuToggle = vi.fn();
    const onToggleApp = vi.fn();
    const { container } = renderTray({
      menuOpen: true,
      onMenuSelect: vi.fn(),
      onMenuToggle,
      onToggleApp,
    });

    const scrim = container.querySelector("div.fixed.inset-0") as HTMLElement;
    fireEvent.click(scrim);

    expect(onMenuToggle).toHaveBeenCalledTimes(1);
    expect(onToggleApp).not.toHaveBeenCalled();
  });

  it("menu carries all four drawn entries, in the drawn order", () => {
    const onMenuSelect = vi.fn();
    renderTray({ menuOpen: true, onMenuSelect });
    // `744:38196` / `38201` / `38206` / `38211`. Support was omitted while its
    // address 404'd, which left one drawn item on the topbar and not here; it
    // resolved to the dashboard's Overview page on 2026-09-07.
    expect(
      screen.getAllByRole("menuitem").map((el) => el.textContent),
    ).toEqual([
      "Visit dashboard",
      "Contact support",
      "Read Gate docs",
      "Quit Gate Connect",
    ]);
    screen.getByRole("menuitem", { name: "Contact support" }).click();
    expect(onMenuSelect).toHaveBeenCalledWith("support");
    screen.getByRole("menuitem", { name: "Quit Gate Connect" }).click();
    expect(onMenuSelect).toHaveBeenCalledWith("quit");
  });
});

describe("the header", () => {
  it("hands over to the full app through Expand app", () => {
    const onExpand = vi.fn();
    renderTray({ onExpand });
    screen.getByRole("button", { name: "Expand app" }).click();
    expect(onExpand).toHaveBeenCalled();
  });
});

describe("signed out", () => {
  it("says so and hands over, instead of painting empty groups", () => {
    const onExpand = vi.fn();
    renderTray({ signedOut: true, onExpand });
    expect(
      screen.getByRole("heading", { name: "Sign in to get started" }),
    ).toBeTruthy();
    expect(screen.queryByRole("switch")).toBeNull();
    screen.getByRole("button", { name: "Open Gate Connect" }).click();
    expect(onExpand).toHaveBeenCalled();
  });
});

/**
 * The alert half of the drawn activity line (`Tray`'s docstring records why the
 * message half is not here). What is worth asserting is the distinction the
 * figure exists to hold: a measured zero says so in words, and a row the feed
 * cannot attribute draws nothing at all rather than a `0` nobody measured.
 */
describe("the row activity line", () => {
  it("draws the count under the status", () => {
    renderTray({ groups: withAlerts({ kind: "count", count: 23 }) });

    expect(row("Claude Code")).toContain("Protected");
    expect(row("Claude Code")).toContain("23 alerts");
  });

  it("says a measured zero in words, and counts one in the singular", () => {
    renderTray({ groups: withAlerts({ kind: "count", count: 0 }) });
    expect(row("Claude Code")).toContain("No alerts");

    cleanup();
    renderTray({ groups: withAlerts({ kind: "count", count: 1 }) });
    expect(row("Claude Code")).toContain("1 alert");
  });

  it("draws nothing for a row the feed cannot attribute", () => {
    // The chat-domain case, and it is permanent: the feed keys events on the
    // tool slug and a domain's traffic arrives unattributed on purpose.
    renderTray({ groups: withAlerts({ kind: "count", count: 23 }) });

    expect(row("Claude Desktop")).toContain("Not routed");
    expect(row("Claude Desktop")).not.toContain("alert");
  });

  it("holds a place while a feed that is running has not answered", () => {
    // A skeleton, not a zero: neither a figure nor "none" is true while we are
    // still asking.
    renderTray({ groups: withAlerts({ kind: "pending" }) });

    expect(row("Claude Code")).not.toContain("alert");
    expect(document.querySelectorAll(".animate-pulse")).toHaveLength(1);
  });
});

/**
 * The message half of the same line, which is a *held* figure rather than a live
 * one - it comes off the last activity reading for that tool, refreshed when the
 * quick status is looked at. So what matters here is that it reads as an answer
 * and discloses its age.
 */
describe("the row message count", () => {
  it("draws traffic first, then the subset that fired", () => {
    renderTray({
      groups: withFigures({
        messages: { kind: "count", count: 1032, measuredAt: "14:03" },
        alerts: { kind: "count", count: 23 },
      }),
    });

    expect(row("Claude Code")).toContain("1,032 messages");
    expect(row("Claude Code")).toContain("23 alerts");
    expect(row("Claude Code").indexOf("messages")).toBeLessThan(
      row("Claude Code").indexOf("alerts"),
    );
  });

  it("says when the figure was measured, since the row cannot print it", () => {
    renderTray({
      groups: withFigures({
        messages: { kind: "count", count: 8, measuredAt: "14:03" },
      }),
    });

    // A held number that says nothing about its age reads as a live one, which is
    // the reading principle 6 exists to prevent. The row has no width for it, so
    // it goes in the tooltip.
    const line = rowOf("Claude Code")?.querySelector("[title]");
    expect(line?.getAttribute("title")).toBe("Messages measured 14:03");
  });

  it("drops the separator when only one half has a reading", () => {
    // The common state on a fresh install: the feed has answered and no activity
    // reading has landed yet.
    renderTray({ groups: withAlerts({ kind: "count", count: 2 }) });

    expect(row("Claude Code")).toContain("2 alerts");
    expect(row("Claude Code")).not.toContain("messages");
    expect(row("Claude Code")).not.toContain("\u00b7");
  });

  it("says a measured zero in words", () => {
    renderTray({
      groups: withFigures({ messages: { kind: "count", count: 0, measuredAt: "14:03" } }),
    });

    expect(row("Claude Code")).toContain("No messages");
  });

  it("holds a place for a figure still being read", () => {
    renderTray({ groups: withFigures({ messages: { kind: "pending" } }) });

    expect(row("Claude Code")).not.toContain("messages");
    expect(document.querySelectorAll(".animate-pulse")).toHaveLength(1);
  });

  it("draws no tooltip for a figure with no age to report", () => {
    // The alert half is live by construction and has nothing to disclose, so a
    // row showing only alerts must not carry an empty or misleading title.
    renderTray({ groups: withAlerts({ kind: "count", count: 2 }) });

    expect(rowOf("Claude Code")?.querySelector("[title]")).toBeNull();
  });
});
