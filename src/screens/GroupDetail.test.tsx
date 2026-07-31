import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent } from "@testing-library/react";
import type { ProxyDomain, Tool } from "../lib/api";
import { buildGroups } from "../lib/groups";
import { GroupDetail } from "./GroupDetail";

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

function renderDetail(
  tools: Tool[],
  domains: ProxyDomain[] = [domain],
  props: Partial<React.ComponentProps<typeof GroupDetail>> = {},
) {
  const [group] = buildGroups(tools, domains, { proxyOn: true, caTrusted: true });
  render(
    <GroupDetail
      group={group}
      busy={false}
      onBack={vi.fn()}
      onToggleGroup={vi.fn()}
      onToggleTool={vi.fn()}
      onSetDomain={vi.fn()}
      onOpenTool={vi.fn()}
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

  it("toggles one member without touching the rest", () => {
    const onToggleTool = vi.fn();
    const onSetDomain = vi.fn();
    renderDetail([tool("opencode", "OpenCode", { kind: "detected" })], [domain], {
      onToggleTool,
      onSetDomain,
    });
    fireEvent.click(screen.getByRole("switch", { name: "Route OpenCode through Gate" }));
    expect(onToggleTool).toHaveBeenCalledWith("opencode", true);
    expect(onSetDomain).not.toHaveBeenCalled();

    fireEvent.click(
      screen.getByRole("switch", { name: "Route Claude Desktop / Cowork through Gate" }),
    );
    expect(onSetDomain).toHaveBeenCalledWith("anthropic", false);
  });

  it("routes the whole family from the group switch", () => {
    const onToggleGroup = vi.fn();
    renderDetail([tool("opencode", "OpenCode", { kind: "detected" })], [], { onToggleGroup });
    fireEvent.click(screen.getByRole("switch", { name: "Route Claude through Gate" }));
    expect(onToggleGroup).toHaveBeenCalledWith("anthropic", true);
  });

  it("sends a drifted member to its own screen instead of replacing the setup", () => {
    const onToggleTool = vi.fn();
    const onOpenTool = vi.fn();
    renderDetail([tool("codex", "Codex", { kind: "drifted", reason: "r" })], [], {
      onToggleTool,
      onOpenTool,
    });
    fireEvent.click(screen.getByRole("switch", { name: "Route Codex through Gate" }));
    expect(onToggleTool).not.toHaveBeenCalled();
    expect(onOpenTool).toHaveBeenCalledWith("codex");
  });

  it("says the group switch will leave a hand-written setup alone", () => {
    renderDetail([tool("codex", "Codex", { kind: "drifted", reason: "r" })], []);
    expect(screen.getByText(/group switch leaves/)).toBeTruthy();
  });

  it("only drills into config members; a proxy row has nothing more to say", () => {
    renderDetail([tool("claude-code", "Claude Code", { kind: "connected" })]);
    expect(screen.getByRole("button", { name: "Claude Code details" })).toBeTruthy();
    expect(
      screen.queryByRole("button", { name: "Claude Desktop / Cowork details" }),
    ).toBeNull();
  });
});
