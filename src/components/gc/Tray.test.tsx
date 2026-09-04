import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
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

function renderTray(overrides: Partial<Parameters<typeof Tray>[0]> = {}) {
  return render(
    <Tray
      master={{ on: true }}
      groups={GROUPS}
      notInstalled={[]}
      notInstalledOpen={false}
      onToggleNotInstalled={noop}
      orgName="Acme Engineering"
      onToggleApp={noop}
      onExpand={noop}
      menuOpen={false}
      onMenuToggle={noop}
      onMenuSelect={noop}
      {...overrides}
    />,
  );
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

  it("reads partially routed when only some rows are", () => {
    renderTray();
    expect(
      screen.getByRole("heading", { name: "Partially routed" }),
    ).toBeTruthy();
    expect(screen.getByText("On · 1 of 2 tools routing")).toBeTruthy();
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
            { slug: "a", name: "A", status: { kind: "not-routed" }, on: false },
          ],
        },
      ],
    });
    expect(screen.getByRole("heading", { name: "Not protected" })).toBeTruthy();
    expect(screen.getByText("Off · 0 of 1 tools routing")).toBeTruthy();
  });

  it("renders no switch: the drawn card is a status, not a control", () => {
    renderTray();
    // Row switches remain; nothing is named for the master.
    expect(
      screen.queryByRole("switch", { name: /route traffic/i }),
    ).toBeNull();
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
    screen.getByRole("switch", { name: "Codex" }).click();
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

describe("the footer", () => {
  it("names the organization", () => {
    renderTray();
    expect(screen.getByText("Acme Engineering")).toBeTruthy();
  });

  it("menu carries dashboard, docs and quit - and no support entry", () => {
    const onMenuSelect = vi.fn();
    renderTray({ menuOpen: true, onMenuSelect });
    expect(screen.getByRole("menuitem", { name: /visit dashboard/i })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: /read gate docs/i })).toBeTruthy();
    // Same omission as the topbar: there is no support address to open.
    expect(screen.queryByRole("menuitem", { name: /support/i })).toBeNull();
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
