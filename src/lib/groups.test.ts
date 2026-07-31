import { describe, expect, it } from "vitest";
import type { ProxyDomain, Tool } from "./api";
import { buildGroups, groupSummary } from "./groups";

function tool(
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
    requires_upstream_credential: false,
    status,
  };
}

function domain(overrides: Partial<ProxyDomain> = {}): ProxyDomain {
  return {
    slug: "anthropic",
    display_name: "Claude Desktop / Cowork",
    hosts: ["api.anthropic.com"],
    upstream_url: "https://api.anthropic.com",
    rewrite_prefixes: [],
    passthrough_prefixes: [],
    enabled: true,
    supported: true,
    ...overrides,
  };
}

const ON = { proxyOn: true, caTrusted: true };

describe("buildGroups", () => {
  it("groups tools and domains by model family", () => {
    const groups = buildGroups(
      [
        tool("claude-code", "Claude Code", { kind: "connected" }),
        tool("codex", "Codex", { kind: "detected" }, "OpenAI"),
      ],
      [domain(), domain({ slug: "openrouter", display_name: "OpenRouter apps", enabled: false })],
      ON,
    );
    expect(groups.map((g) => g.name)).toEqual(["Claude", "OpenAI", "OpenRouter"]);
    expect(groups[0].members.map((m) => m.name)).toEqual([
      "Claude Code",
      "Claude Desktop / Cowork",
    ]);
  });

  it("leaves out what cannot be routed today", () => {
    const groups = buildGroups(
      [tool("hermes", "Hermes", { kind: "not_installed" })],
      [domain({ slug: "openai", display_name: "OpenAI apps", supported: false })],
      ON,
    );
    expect(groups).toEqual([]);
  });

  it("does not count an enabled domain as routed while the CA is untrusted", () => {
    const trusted = buildGroups([], [domain()], ON);
    expect(trusted[0].routed).toBe(1);

    const untrusted = buildGroups([], [domain()], { proxyOn: true, caTrusted: false });
    expect(untrusted[0].routed).toBe(0);
    expect(untrusted[0].members[0].attention).toBe("needs-trust");

    const off = buildGroups([], [domain()], { proxyOn: false, caTrusted: true });
    expect(off[0].routed).toBe(0);
  });

  it("keeps an unknown upstream rather than dropping the tool", () => {
    const groups = buildGroups(
      [tool("mystery", "Mystery", { kind: "detected" }, "Acme Labs")],
      [],
      ON,
    );
    expect(groups).toHaveLength(1);
    expect(groups[0].id).toBe("acme-labs");
    expect(groups[0].name).toBe("Acme-labs");
  });

  it("marks drifted and errored members for the summary", () => {
    const groups = buildGroups(
      [
        tool("codex", "Codex", { kind: "drifted", reason: "r" }, "OpenAI"),
        tool("openclaw", "OpenClaw", { kind: "error", message: "m" }),
      ],
      [],
      ON,
    );
    const byId = Object.fromEntries(groups.map((g) => [g.id, g]));
    expect(byId.openai.members[0].attention).toBe("drifted");
    expect(byId.anthropic.members[0].attention).toBe("error");
  });
});

describe("groupSummary", () => {
  it("counts, and names a single failure", () => {
    const [group] = buildGroups(
      [
        tool("claude-code", "Claude Code", { kind: "connected" }),
        tool("openclaw", "OpenClaw", { kind: "error", message: "m" }),
      ],
      [],
      ON,
    );
    expect(groupSummary(group)).toEqual({
      count: "1 of 2 routing",
      exception: "OpenClaw failed",
    });
  });

  it("aggregates several failures rather than naming one", () => {
    const [group] = buildGroups(
      [
        tool("openclaw", "OpenClaw", { kind: "error", message: "m" }),
        tool("opencode", "OpenCode", { kind: "error", message: "m" }),
      ],
      [],
      ON,
    );
    expect(groupSummary(group).exception).toBe("2 failed");
  });

  it("prefers the certificate over a drifted setup, and reports nothing when all is well", () => {
    const [blocked] = buildGroups(
      [tool("codex", "Codex", { kind: "drifted", reason: "r" }, "OpenAI")],
      [domain({ slug: "openai", display_name: "OpenAI apps" })],
      { proxyOn: true, caTrusted: false },
    );
    expect(groupSummary(blocked).exception).toBe("certificate not trusted");

    const [clean] = buildGroups([tool("claude-code", "Claude Code", { kind: "connected" })], [], ON);
    expect(groupSummary(clean)).toEqual({ count: "1 of 1 routing", exception: null });
  });
});
