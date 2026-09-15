import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent } from "@testing-library/react";
import type { ClientId, Tool, ProxyDomain, Verdict } from "../lib/api";
import { buildGroups, type Group } from "../lib/groups";
import { FamilyPanel } from "./FamilyPanel";

function makeTool(
  slug: string,
  name: string,
  status: Tool["status"],
  client: ClientId = "claude-code",
): Tool {
  return {
    slug,
    name,
    // The flat-list name; the rail's one-word label is `name`.
    product_name: name,
    upstream_provider_name: "Anthropic",
    default_upstream_url: "https://api.anthropic.com",
    config_location: null,
    status,
    client,
    scope: "client",
    credential: "brokered",
  };
}

function makeDomain(overrides: Partial<ProxyDomain> = {}): ProxyDomain {
  return {
    slug: "anthropic",
    display_name: "Claude Desktop / Cowork",
    hosts: ["api.anthropic.com"],
    upstream_url: "https://api.anthropic.com",
    rewrite_prefixes: ["/v1/messages"],
    passthrough_prefixes: [],
    enabled: true,
    supported: true,
    // Same client as the fixture tools, so these suites keep rendering ONE
    // group holding a config row and a host row - which is what the panel is
    // about. Membership is the row's own client now.
    client: "claude-code",
    credential: "brokered",
    scope: "host",
    ...overrides,
  };
}

/** The session-credential domain as the backend ships it: supported, off, and
 * `additive`, which is what keeps it off the group switch. */
function makeChatDomain(overrides: Partial<ProxyDomain> = {}): ProxyDomain {
  return makeDomain({
    slug: "claude-web",
    display_name: "Chat",
    hosts: ["claude.ai"],
    upstream_url: "https://claude.ai/api",
    enabled: false,
    credential: "additive",
    ...overrides,
  });
}

/** A sweep that confirms every connected tool.
 *
 * These suites are about the ledger's layout and copy, not about verification:
 * AG-570 stops a config file alone from producing `routed`, so a test that wants
 * a routing row now has to say the check agreed. `lib/groups.test.ts` covers the
 * rule itself.
 */
function sweep(tools: Tool[]): Map<string, Verdict> {
  return new Map(
    tools
      .filter((t) => t.status.kind === "connected")
      .map((t) => [
        t.slug,
        {
          slug: t.slug,
          state: "on" as const,
          reason: null,
          next_action: null,
          route_in_use: null,
          requested_route: null,
        },
      ]),
  );
}

/** Renders the panel for one family, the way App does: it resolves the group out
 * of the same ledger Home renders and hands over that one. `family` picks which,
 * defaulting to the first, so a test can prove the others are absent. */
function renderPanel(
  {
    tools = [] as Tool[],
    domains = [] as ProxyDomain[],
    proxyOn = true,
    caTrusted = true,
    family = undefined as string | undefined,
  } = {},
  props: Partial<React.ComponentProps<typeof FamilyPanel>> = {},
) {
  const groups = buildGroups(tools, domains, {
    proxyOn,
    caTrusted,
    // With the engine down the sweep never returns `on` - it reports a
    // connection problem - so a fixture that claimed both would be a machine
    // that cannot exist.
    verdicts: proxyOn ? sweep(tools) : new Map(),
  });
  const group: Group = (family ? groups.find((g) => g.id === family) : groups[0])!;
  render(
    <FamilyPanel
      group={group}
      busy={false}
      onBack={vi.fn()}
      browserChannel={true}
      onToggleGroup={vi.fn()}
      onToggleTool={vi.fn()}
      onSetDomain={vi.fn()}
      onTrustCa={vi.fn()}
      trustPending={false}
      proxyOn={proxyOn}
      onEnableRouting={vi.fn()}
      {...props}
    />,
  );
  return group;
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("FamilyPanel is about one family", () => {
  it("titles itself with the family, not with the question Home asks", () => {
    renderPanel({ tools: [makeTool("claude-code", "CLI", { kind: "connected" })] });
    // The panel used to be titled "What routes through Gate" and list all four
    // families, which is the sentence Home still heads its rows with. Four
    // chevrons reaching one identically-titled screen is what made the
    // navigation read as a no-op.
    expect(screen.getByRole("heading", { level: 1, name: "Claude" })).toBeTruthy();
    expect(screen.queryByText("What routes through Gate")).toBeNull();
  });

  it("shows no other family, and no way to reach one", () => {
    renderPanel({
      tools: [
        makeTool("claude-code", "CLI", { kind: "connected" }),
        makeTool("codex", "Codex", { kind: "connected" }, "codex"),
      ],
    });
    // Three of the old panel's four visible rows were a copy of the screen the
    // user had just left.
    expect(screen.queryByText("codex")).toBeNull();
    expect(screen.queryByText("Codex")).toBeNull();
  });

  it("opens with the members already there, with nothing to expand first", () => {
    renderPanel({
      tools: [makeTool("claude-code", "CLI", { kind: "connected" })],
      domains: [makeDomain()],
    });
    // `initialOpen` existed so a tapped row did not charge a second click for
    // the family just tapped. A panel about one family arrives open by being.
    // The h1 names the app, the rows name its surfaces - which is the pairing
    // the section table ships ("CLI" and "API" under "Claude").
    expect(screen.getByRole("heading", { level: 1, name: "Claude" })).toBeTruthy();
    expect(screen.getByText("CLI")).toBeTruthy();
    expect(screen.getByText("Claude Desktop / Cowork")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Claude Code details" })).toBeNull();
  });

  it("labels its switch without saying the family name twice", () => {
    renderPanel({ tools: [makeTool("claude-code", "CLI", { kind: "connected" })] });
    // The h1 carries the name; the control row carries the control's own label.
    expect(screen.getByText("Route through Gate")).toBeTruthy();
    expect(screen.getAllByRole("heading", { name: "Claude" })).toHaveLength(1);
  });

  it("counts the members it is showing", () => {
    renderPanel({
      tools: [makeTool("claude-code", "CLI", { kind: "connected" })],
      domains: [makeDomain({ enabled: false })],
    });
    // The denominator was unreferenced on the old screens: "8 of 8 routing" over
    // a list of 4 families. Here the members are the itemization, so the
    // arithmetic is checkable where it is asserted.
    expect(screen.getByText("1 of 2 routing")).toBeTruthy();
    // Twice: the visible pill, and the sr-only sentence the switch beside it
    // points at, which is the same pairing the member rows use.
    expect(screen.getAllByText("Partly routed")).toHaveLength(2);
  });

  it("leaves the exception sentence to the banner that can fix it", () => {
    renderPanel({
      tools: [makeTool("claude-code", "CLI", { kind: "error", message: "bad json" })],
      domains: [makeDomain()],
    });
    // `groupSummary` would say "Claude Code failed" here. Every exception it can
    // name has a banner below with the remedy attached, so printing the summary
    // too would state one fact twice and put the unactionable copy first.
    expect(screen.queryByText("CLI failed")).toBeNull();
    expect(screen.getByText(/isn.t reporting its routing state/)).toBeTruthy();
  });

  it("routes the whole family with one flip", () => {
    const onToggleGroup = vi.fn();
    renderPanel(
      { tools: [makeTool("claude-code", "CLI", { kind: "detected" })] },
      { onToggleGroup },
    );
    fireEvent.click(screen.getByRole("switch", { name: "Route Claude through Gate" }));
    expect(onToggleGroup).toHaveBeenCalledWith("claude", true);
  });

  it("gives the chat surface its own row and switch under the family", () => {
    const onSetDomain = vi.fn().mockResolvedValue(undefined);
    renderPanel({ domains: [makeDomain(), makeChatDomain()] }, { onSetDomain });
    expect(
      screen.getByRole("heading", { level: 2, name: "Chat" }),
    ).toBeTruthy();
    fireEvent.click(
      screen.getByRole("switch", { name: "Route Chat through Gate" }),
    );
    expect(onSetDomain).toHaveBeenCalledWith("claude-web", true);
  });

  it("leaves the family switch off when only the chat surface is on", () => {
    // The chat surface carries the user's session cookie, so the family switch
    // must not reach it - and must not report itself on because of it. Reading
    // this switch off `desired` put it in a state where it rendered on over a
    // family routing nothing it could flip, and clicking it asked to turn off a
    // set that was already off, so the switch never moved.
    const onToggleGroup = vi.fn();
    renderPanel(
      { domains: [makeDomain({ enabled: false }), makeChatDomain({ enabled: true })] },
      { onToggleGroup },
    );
    const family = screen.getByRole("switch", { name: "Route Claude through Gate" });
    expect(family.getAttribute("aria-checked")).toBe("false");
    fireEvent.click(family);
    expect(onToggleGroup).toHaveBeenCalledWith("claude", true);
  });

  it("says what a credential-sensitive row routes, since it is not a brokered key", () => {
    // Wording that has to hold for both rows of this kind: claude.ai's session
    // cookie and the ChatGPT subscription behind Codex's Responses endpoint.
    // "conversations" would be wrong about the second.
    renderPanel({ domains: [makeDomain(), makeChatDomain()] });
    fireEvent.click(screen.getByRole("button", { name: "Chat details" }));
    expect(screen.getByText(/already signed in with, not an API key Gate brokers/)).toBeTruthy();
    expect(screen.getByText(/family switch above leaves this row alone/)).toBeTruthy();
  });

  it("returns to Home from its own header", () => {
    const onBack = vi.fn();
    renderPanel(
      { tools: [makeTool("claude-code", "CLI", { kind: "connected" })] },
      { onBack },
    );
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it("does not carry the shell-environment switch, which belongs to no family", () => {
    renderPanel({ tools: [makeTool("claude-code", "CLI", { kind: "connected" })] });
    // It routes every command-line tool at once, whatever provider they talk to,
    // so a panel about one family is the one place it cannot live. Home has it.
    expect(
      screen.queryByRole("switch", { name: "Route command-line tools through Gate" }),
    ).toBeNull();
  });
});

describe("FamilyPanel explains the sections that need a sentence", () => {
  /** The machine-wide group: the environment channel plus the hosts that cover
   * whatever reaches them. The only heading that does not name a program the
   * user installed, and so the only one that owes them a sentence. */
  const terminal = {
    tools: [makeTool("env-proxy", "Terminal tools", { kind: "detected" }, "any-app")],
  };

  it("renders the blurb that had never been rendered anywhere", () => {
    renderPanel(terminal);
    // The field existed from the round that retired "Agent harnesses" and moved
    // the definition here; nothing displayed it, so the category's only
    // description in the UI was 18 characters in a truncating slot.
    expect(screen.getByRole("heading", { level: 1, name: "Terminal" })).toBeTruthy();
    expect(screen.getByText(/every program started after your next login/)).toBeTruthy();
  });

  it("names the boundary rather than promising Gate takes everything", () => {
    renderPanel(terminal);
    // The half a user running a local model needs, and the half the old copy
    // got backwards.
    expect(
      screen.getByText(/including a local model, keeps going where it always did/),
    ).toBeTruthy();
    expect(screen.queryByText(/every provider you.+ve set up in them/)).toBeNull();
  });

  it("gives a single-tool group its own heading instead of a shared one", () => {
    // The reason the row below it can read "CLI" at all: "OpenClaw" is said
    // once, by the heading, so the row is free to say which surface it is.
    renderPanel({
      tools: [makeTool("openclaw", "CLI", { kind: "detected" }, "openclaw")],
    });
    expect(screen.getByRole("heading", { level: 1, name: "OpenClaw" })).toBeTruthy();
    expect(screen.queryByText(/every program started after your next login/)).toBeNull();
  });

  it("stays silent on a family whose name already says what it covers", () => {
    renderPanel({ tools: [makeTool("claude-code", "CLI", { kind: "connected" })] });
    // "Everything that talks to Anthropic." under an h1 reading "Anthropic" is
    // the same fact twice, so those per-family lines were deleted, not shown.
    expect(screen.queryByText(/Everything that talks to/)).toBeNull();
    expect(screen.queryByText(/talk to several providers/)).toBeNull();
  });
});

describe("FamilyPanel accessibility", () => {
  it("exposes the members as the list, since there are no families to count", () => {
    renderPanel({
      tools: [makeTool("claude-code", "CLI", { kind: "connected" })],
      domains: [makeDomain()],
    });
    expect(screen.getByRole("list")).toBeTruthy();
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
  });

  it("keeps the members navigable by heading under the panel's h1", () => {
    renderPanel({
      tools: [makeTool("claude-code", "CLI", { kind: "connected" })],
      domains: [makeDomain()],
    });
    // They were plain divs while the families above them were the h2s. With the
    // family promoted to the title, the outline would otherwise be one h1 and
    // nothing else.
    expect(screen.getByRole("heading", { level: 2, name: "CLI" })).toBeTruthy();
    expect(
      screen.getByRole("heading", { level: 2, name: "Claude Desktop / Cowork" }),
    ).toBeTruthy();
  });

  it("announces the family's new state after a flip", () => {
    renderPanel({ tools: [makeTool("claude-code", "CLI", { kind: "connected" })] });
    const live = document.querySelector('[aria-live="polite"]')!;
    expect(live.textContent).toBe("");
    fireEvent.click(screen.getByRole("switch", { name: "Route Claude through Gate" }));
    expect(live.textContent).toContain("Claude");
    expect(live.textContent).toContain("Routed");
  });

  it("points the family switch at the reality sentence when it reads on over nothing", () => {
    // DESIGN.md, "Intent versus Reality": a switch that can read "on" over
    // something broken points `aria-describedby` at what is actually happening.
    const group = renderPanel({
      tools: [makeTool("claude-code", "CLI", { kind: "connected" })],
      proxyOn: false,
    });
    const toggle = screen.getByRole("switch", { name: "Route Claude through Gate" });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(toggle.getAttribute("aria-describedby")).toBe(`group-desc-${group.id}`);
    const sentence = document.getElementById(`group-desc-${group.id}`);
    expect(sentence?.textContent).toBe("Not routed");
  });
});

describe("FamilyPanel member detail", () => {
  it("opens one member's mechanism and prose in place", () => {
    renderPanel({
      tools: [makeTool("claude-code", "CLI", { kind: "connected" })],
      domains: [makeDomain()],
    });
    const row = screen.getByRole("button", { name: "CLI details" });
    expect(row.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(row);
    expect(row.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText(/points at your Gate gateway/)).toBeTruthy();
  });

  it("keeps the mechanism chip on every member without opening anything", () => {
    renderPanel({
      tools: [makeTool("claude-code", "CLI", { kind: "connected" })],
      domains: [makeDomain()],
    });
    expect(screen.getByText("config file")).toBeTruthy();
    expect(screen.getByText("proxy")).toBeTruthy();
  });
});
