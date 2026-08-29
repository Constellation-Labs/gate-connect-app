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
