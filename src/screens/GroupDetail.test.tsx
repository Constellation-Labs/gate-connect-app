import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent, waitFor } from "@testing-library/react";
import type { ProviderState, ProxyDomain, Tool } from "../lib/api";
import { buildGroups } from "../lib/groups";
import { GroupDetail } from "./GroupDetail";

vi.mock("../lib/analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));

function tool(slug: string, name: string, status: Tool["status"]): Tool {
  return {
    slug,
    name,
    upstream_provider_name: "Anthropic",
    default_upstream_url: "https://api.anthropic.com",
    requires_upstream_credential: false,
    status,
  };
}

const domain: ProxyDomain = {
  slug: "anthropic",
  display_name: "Claude Desktop / Cowork",
  hosts: ["api.anthropic.com"],
  upstream_url: "https://api.anthropic.com",
  rewrite_prefixes: [],
  passthrough_prefixes: [],
  enabled: true,
  supported: true,
};

const CATALOG: ProviderState[] = [
  {
    slug: "anthropic",
    display_name: "Claude",
    subtitle: "",
    enabled: false,
    available: true,
    tool_slugs: ["claude-code", "openclaw", "codex"],
    domain_slugs: ["anthropic"],
  },
];

function renderDetail(
  tools: Tool[],
  domains: ProxyDomain[] = [domain],
  props: Partial<React.ComponentProps<typeof GroupDetail>> = {},
) {
  const [group] = buildGroups(CATALOG, tools, domains, { proxyOn: true, caTrusted: true });
  render(
    <GroupDetail
      group={group}
      busy={false}
      onBack={vi.fn()}
      onToggleGroup={vi.fn()}
      onToggleTool={vi.fn(() => Promise.resolve())}
      onSetDomain={vi.fn(() => Promise.resolve())}
      {...props}
    />,
  );
  return group;
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("GroupDetail", () => {
  it("shows the mechanism per member, which Home deliberately omits", () => {
    renderDetail([tool("claude-code", "Claude Code", { kind: "connected" })]);
    expect(screen.getByText("config file")).toBeTruthy();
    expect(screen.getByText("proxy")).toBeTruthy();
    expect(screen.getByText("api.anthropic.com")).toBeTruthy();
  });

  it("toggles one member without touching the rest", async () => {
    const onToggleTool = vi.fn(() => Promise.resolve());
    const onSetDomain = vi.fn(() => Promise.resolve());
    renderDetail([tool("claude-code", "Claude Code", { kind: "detected" })], [domain], {
      onToggleTool,
      onSetDomain,
    });
    fireEvent.click(screen.getByRole("switch", { name: "Route Claude Code through Gate" }));
    await waitFor(() => expect(onToggleTool).toHaveBeenCalledWith("claude-code", true));
    expect(onSetDomain).not.toHaveBeenCalled();

    fireEvent.click(
      screen.getByRole("switch", { name: "Route Claude Desktop / Cowork through Gate" }),
    );
    await waitFor(() => expect(onSetDomain).toHaveBeenCalledWith("anthropic", false));
  });

  it("routes the whole family from the group switch", () => {
    const onToggleGroup = vi.fn();
    renderDetail([tool("claude-code", "Claude Code", { kind: "detected" })], [], { onToggleGroup });
    fireEvent.click(screen.getByRole("switch", { name: "Route Claude through Gate" }));
    expect(onToggleGroup).toHaveBeenCalledWith("anthropic", true);
  });
});

describe("GroupDetail inline expansion", () => {
  it("explains a member in place rather than on a third screen", () => {
    renderDetail([tool("claude-code", "Claude Code", { kind: "connected" })], []);
    const row = screen.getByRole("button", { name: "Claude Code details" });
    expect(row.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(row);
    expect(row.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText(/never lands in the config file/)).toBeTruthy();
  });

  it("shows a failure's whole message inline", () => {
    renderDetail(
      [
        tool("openclaw", "OpenClaw", {
          kind: "error",
          message: "failed to parse ~/.openclaw/openclaw.json: unexpected character",
        }),
      ],
      [],
    );
    fireEvent.click(screen.getByRole("button", { name: "OpenClaw details" }));
    expect(screen.getByText(/unexpected character/)).toBeTruthy();
  });

  it("surfaces a failed toggle next to the member that failed", async () => {
    const onToggleTool = vi.fn(() => Promise.reject("No upstream credential saved."));
    renderDetail([tool("claude-code", "Claude Code", { kind: "detected" })], [], { onToggleTool });
    fireEvent.click(screen.getByRole("switch", { name: "Route Claude Code through Gate" }));
    expect(await screen.findByText("Couldn't connect this tool")).toBeTruthy();
  });

  it("asks before replacing a hand-written setup, and only writes on confirm", async () => {
    const onToggleTool = vi.fn(() => Promise.resolve());
    renderDetail([tool("codex", "Codex", { kind: "drifted", reason: "r" })], [], { onToggleTool });
    fireEvent.click(screen.getByRole("switch", { name: "Route Codex through Gate" }));
    // First flip only arms the confirm; nothing is written yet.
    expect(onToggleTool).not.toHaveBeenCalled();
    expect(screen.getByText(/Replace Codex/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Replace setup" }));
    await waitFor(() => expect(onToggleTool).toHaveBeenCalledWith("codex", true));
  });

  it("says the group switch will leave a hand-written setup alone", () => {
    renderDetail([tool("codex", "Codex", { kind: "drifted", reason: "r" })], []);
    expect(screen.getByText(/group switch leaves/)).toBeTruthy();
  });
});
