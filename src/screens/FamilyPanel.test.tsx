import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent } from "@testing-library/react";
import type { ProviderState, Tool, ProxyDomain } from "../lib/api";
import { buildGroups, type Group } from "../lib/groups";
import { FamilyPanel } from "./FamilyPanel";

function makeTool(
  slug: string,
  name: string,
  status: Tool["status"],
  upstream = "Anthropic",
): Tool {
  return {
    slug,
    name,
    upstream_provider_name: upstream,
    default_upstream_url: "https://api.anthropic.com",
    status,
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
    ...overrides,
  };
}

/** The chat-protocol domain as the backend ships it: supported, off, and in the
 * family's `chat_domain_slugs` rather than its `domain_slugs`. */
function makeChatDomain(overrides: Partial<ProxyDomain> = {}): ProxyDomain {
  return makeDomain({
    slug: "claude-web",
    display_name: "Claude Desktop chat",
    hosts: ["claude.ai"],
    upstream_url: "https://claude.ai/api",
    enabled: false,
    ...overrides,
  });
}

/** Mirrors the real catalog: Claude Code and Codex are claimed; OpenCode and
 * OpenClaw deliberately are not, so they land in "Other tools". */
const CATALOG: ProviderState[] = [
  {
    slug: "anthropic",
    display_name: "Claude",
    subtitle: "",
    enabled: false,
    available: true,
    tool_slugs: ["claude-code"],
    domain_slugs: ["anthropic"],
    chat_domain_slugs: ["claude-web"],
  },
  {
    slug: "openai",
    display_name: "OpenAI",
    subtitle: "",
    enabled: false,
    available: true,
    tool_slugs: ["codex"],
    domain_slugs: ["openai"],
    // Mirrors the real catalog: the Codex subscription endpoint and the
    // ChatGPT app's chat turn both ride this field, so the family switch skips
    // both. Only a test that passes those domains in gets the rows.
    chat_domain_slugs: ["chatgpt", "chatgpt-apps"],
  },
];

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
  const groups = buildGroups(CATALOG, tools, domains, { proxyOn, caTrusted });
  const group: Group = (family ? groups.find((g) => g.id === family) : groups[0])!;
  render(
    <FamilyPanel
      group={group}
      busy={false}
      onBack={vi.fn()}
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
    renderPanel({ tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })] });
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
        makeTool("claude-code", "Claude Code", { kind: "connected" }),
        makeTool("codex", "Codex", { kind: "connected" }, "OpenAI"),
      ],
    });
    // Three of the old panel's four visible rows were a copy of the screen the
    // user had just left.
    expect(screen.queryByText("OpenAI")).toBeNull();
    expect(screen.queryByText("Codex")).toBeNull();
  });

  it("opens with the members already there, with nothing to expand first", () => {
    renderPanel({
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      domains: [makeDomain()],
    });
    // `initialOpen` existed so a tapped row did not charge a second click for
    // the family just tapped. A panel about one family arrives open by being.
    expect(screen.getByText("Claude Code")).toBeTruthy();
    expect(screen.getByText("Claude Desktop / Cowork")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Claude details" })).toBeNull();
  });

  it("labels its switch without saying the family name twice", () => {
    renderPanel({ tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })] });
    // The h1 carries the name; the control row carries the control's own label.
    expect(screen.getByText("Route through Gate")).toBeTruthy();
    expect(screen.getAllByRole("heading", { name: "Claude" })).toHaveLength(1);
  });

  it("counts the members it is showing", () => {
    renderPanel({
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
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
      tools: [makeTool("claude-code", "Claude Code", { kind: "error", message: "bad json" })],
      domains: [makeDomain()],
    });
    // `groupSummary` would say "Claude Code failed" here. Every exception it can
    // name has a banner below with the remedy attached, so printing the summary
    // too would state one fact twice and put the unactionable copy first.
    expect(screen.queryByText("Claude Code failed")).toBeNull();
    expect(screen.getByText(/isn.t reporting its routing state/)).toBeTruthy();
  });

  it("routes the whole family with one flip", () => {
    const onToggleGroup = vi.fn();
    renderPanel(
      { tools: [makeTool("claude-code", "Claude Code", { kind: "detected" })] },
      { onToggleGroup },
    );
    fireEvent.click(screen.getByRole("switch", { name: "Route Claude through Gate" }));
    expect(onToggleGroup).toHaveBeenCalledWith("anthropic", true);
  });

  it("gives the chat surface its own row, with a switch the app does not offer", () => {
    // The row is here so a session-cookie surface is never intercepted
    // invisibly - it reports its own state, and reports it under the family
    // whose switch leaves it alone. What it does not do is let the app turn it
    // on: the switch is inert, and `proxy domain claude-web on` is the only way
    // in. Clicking it must reach no backend.
    const onSetDomain = vi.fn().mockResolvedValue(undefined);
    renderPanel({ domains: [makeDomain(), makeChatDomain()] }, { onSetDomain });
    expect(
      screen.getByRole("heading", { level: 2, name: "Claude Desktop chat" }),
    ).toBeTruthy();
    const chat = screen.getByRole("switch", { name: "Route Claude Desktop chat through Gate" });
    expect((chat as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(chat);
    expect(onSetDomain).not.toHaveBeenCalled();
  });

  it("keeps the Codex subscription row flippable, though it rides the same field", () => {
    // `chatgpt` sits in the provider's `chat_domain_slugs` so the family switch
    // skips it, but it is a model endpoint reached with a subscription bearer,
    // not a chat surface Gate reads conversations off. Locking the two chat
    // surfaces must not take this switch with them.
    const onSetDomain = vi.fn().mockResolvedValue(undefined);
    renderPanel(
      {
        domains: [
          makeDomain({ slug: "openai", display_name: "OpenAI apps", hosts: ["api.openai.com"] }),
          makeChatDomain({
            slug: "chatgpt",
            display_name: "ChatGPT (Codex subscription)",
            hosts: ["chatgpt.com"],
          }),
        ],
        family: "openai",
      },
      { onSetDomain },
    );
    fireEvent.click(
      screen.getByRole("switch", { name: "Route ChatGPT (Codex subscription) through Gate" }),
    );
    expect(onSetDomain).toHaveBeenCalledWith("chatgpt", true);
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
    expect(onToggleGroup).toHaveBeenCalledWith("anthropic", true);
  });

  it("says what a credential-sensitive row routes, since it is not a brokered key", () => {
    // Wording that has to hold for both rows of this kind: claude.ai's session
    // cookie and the ChatGPT subscription behind Codex's Responses endpoint.
    // "conversations" would be wrong about the second.
    renderPanel({ domains: [makeDomain(), makeChatDomain()] });
    fireEvent.click(screen.getByRole("button", { name: "Claude Desktop chat details" }));
    expect(screen.getByText(/already signed in with, not an API key Gate brokers/)).toBeTruthy();
    expect(screen.getByText(/family switch above leaves this row alone/)).toBeTruthy();
  });

  it("returns to Home from its own header", () => {
    const onBack = vi.fn();
    renderPanel(
      { tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })] },
      { onBack },
    );
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it("does not carry the shell-environment switch, which belongs to no family", () => {
    renderPanel({ tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })] });
    // It routes every command-line tool at once, whatever provider they talk to,
    // so a panel about one family is the one place it cannot live. Home has it.
    expect(
      screen.queryByRole("switch", { name: "Route command-line tools through Gate" }),
    ).toBeNull();
  });
});

describe("FamilyPanel explains the family named by exclusion", () => {
  /** Only OpenCode is installed and no provider claims it, so the ledger is the
   * multi-provider group alone. */
  const otherTools = {
    tools: [makeTool("opencode", "OpenCode", { kind: "detected" }, "your existing providers")],
  };

  it("renders the blurb that had never been rendered anywhere", () => {
    renderPanel(otherTools);
    // The field existed from the round that retired "Agent harnesses" and moved
    // the definition here; nothing displayed it, so the category's only
    // description in the UI was 18 characters in a truncating slot.
    expect(screen.getByRole("heading", { level: 1, name: "Other tools" })).toBeTruthy();
    expect(screen.getByText(/talk to several providers, not one model family/)).toBeTruthy();
  });

  it("names the boundary rather than promising Gate takes everything", () => {
    renderPanel(otherTools);
    // The half a user running a local model needs, and the half the old copy
    // got backwards.
    expect(screen.getByText(/Gate routes the ones it covers/)).toBeTruthy();
    expect(screen.getByText(/including a local model, keeps going where it always did/)).toBeTruthy();
    expect(screen.queryByText(/every provider you.+ve set up in them/)).toBeNull();
  });

  it("stays silent on a family whose name already says what it covers", () => {
    renderPanel({ tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })] });
    // "Everything that talks to Anthropic." under an h1 reading "Anthropic" is
    // the same fact twice, so those per-family lines were deleted, not shown.
    expect(screen.queryByText(/Everything that talks to/)).toBeNull();
    expect(screen.queryByText(/talk to several providers/)).toBeNull();
  });
});

describe("FamilyPanel accessibility", () => {
  it("exposes the members as the list, since there are no families to count", () => {
    renderPanel({
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      domains: [makeDomain()],
    });
    expect(screen.getByRole("list")).toBeTruthy();
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
  });

  it("keeps the members navigable by heading under the panel's h1", () => {
    renderPanel({
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      domains: [makeDomain()],
    });
    // They were plain divs while the families above them were the h2s. With the
    // family promoted to the title, the outline would otherwise be one h1 and
    // nothing else.
    expect(screen.getByRole("heading", { level: 2, name: "Claude Code" })).toBeTruthy();
    expect(
      screen.getByRole("heading", { level: 2, name: "Claude Desktop / Cowork" }),
    ).toBeTruthy();
  });

  it("announces the family's new state after a flip", () => {
    renderPanel({ tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })] });
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
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
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
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      domains: [makeDomain()],
    });
    const row = screen.getByRole("button", { name: "Claude Code details" });
    expect(row.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(row);
    expect(row.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText(/points at your Gate gateway/)).toBeTruthy();
  });

  it("keeps the mechanism chip on every member without opening anything", () => {
    renderPanel({
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      domains: [makeDomain()],
    });
    expect(screen.getByText("config file")).toBeTruthy();
    expect(screen.getByText("proxy")).toBeTruthy();
  });
});
