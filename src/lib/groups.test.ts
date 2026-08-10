import { describe, expect, it } from "vitest";
import type { ProviderState, ProxyDomain, Tool } from "./api";
import { buildGroups, groupSummary, MULTI_PROVIDER_ID } from "./groups";

function tool(slug: string, name: string, status: Tool["status"], upstream = "Anthropic"): Tool {
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

function provider(
  slug: string,
  display_name: string,
  tool_slugs: string[],
  domain_slugs: string[],
): ProviderState {
  return {
    slug,
    display_name,
    subtitle: "",
    enabled: false,
    available: true,
    tool_slugs,
    domain_slugs,
  };
}

/** Mirrors the real catalog: Claude Code and Codex are each claimed by one
 * provider; OpenCode / OpenClaw / Hermes deliberately are not. */
const CATALOG = [
  provider("anthropic", "Claude", ["claude-code"], ["anthropic"]),
  provider("openai", "OpenAI", ["codex"], ["openai"]),
  provider("openrouter", "OpenRouter", [], ["openrouter"]),
];

const ON = { proxyOn: true, caTrusted: true };

describe("buildGroups", () => {
  it("groups by the catalog, not by the tool's display prose", () => {
    const groups = buildGroups(
      CATALOG,
      [
        tool("claude-code", "Claude Code", { kind: "connected" }),
        tool("codex", "Codex", { kind: "detected" }, "OpenAI"),
      ],
      [domain()],
      ON,
    );
    expect(groups.map((g) => g.name)).toEqual(["Claude", "OpenAI"]);
    expect(groups[0].members.map((m) => m.name)).toEqual([
      "Claude Code",
      "Claude Desktop / Cowork",
    ]);
  });

  it("gives the multi-provider tools their own group instead of a wrong family", () => {
    // Their upstream_provider_name is literally "your existing providers":
    // display prose that must never become a family name.
    const groups = buildGroups(
      CATALOG,
      [
        tool("opencode", "OpenCode", { kind: "detected" }, "your existing providers"),
        tool("openclaw", "OpenClaw", { kind: "detected" }, "your existing providers"),
      ],
      [],
      ON,
    );
    expect(groups).toHaveLength(1);
    expect(groups[0].id).toBe(MULTI_PROVIDER_ID);
    expect(groups[0].name).toBe("Other tools");
    expect(groups[0].members.map((m) => m.name)).toEqual(["OpenCode", "OpenClaw"]);
    expect(groups[0].name).not.toContain("existing providers");
    // No single upstream host is true of these, so the detail must not print one.
    expect(groups[0].members.every((m) => m.coversAllProviders)).toBe(true);
    // Common noun, so it must not stay capitalised inside a sentence the way a
    // family name does.
    expect(groups[0].switchLabel).toBe("Route other tools through Gate");
  });

  it("builds the fourth row the ledger could not previously reach", () => {
    // All three harnesses are listed now, so a family row and the
    // multi-provider row coexist - the four-row ledger the sizing notes
    // describe but no test could produce while every harness was hidden.
    const groups = buildGroups(
      CATALOG,
      [
        tool("claude-code", "Claude Code", { kind: "connected" }),
        tool("opencode", "OpenCode", { kind: "detected" }, "your existing providers"),
        tool("openclaw", "OpenClaw", { kind: "detected" }, "your existing providers"),
        tool("hermes", "Hermes", { kind: "detected" }, "your existing providers"),
      ],
      [],
      ON,
    );
    const harnesses = groups.find((g) => g.id === MULTI_PROVIDER_ID);
    expect(harnesses).toBeTruthy();
    expect(harnesses!.members.map((m) => m.name)).toEqual(["OpenCode", "OpenClaw", "Hermes"]);
    // The family row is still its own row: a harness must not be absorbed into
    // Claude just because it can talk to Anthropic.
    const claude = groups.find((g) => g.id === "anthropic");
    expect(claude!.members.map((m) => m.name)).toEqual(["Claude Code"]);
  });

  it("drops families with nothing routable and leaves out what cannot route", () => {
    const groups = buildGroups(
      CATALOG,
      [tool("hermes", "Hermes", { kind: "not_installed" }, "openrouter")],
      [domain({ slug: "openai", display_name: "OpenAI apps", supported: false })],
      ON,
    );
    expect(groups).toEqual([]);
  });

  it("does not count an enabled domain as routed while the CA is untrusted", () => {
    const trusted = buildGroups(CATALOG, [], [domain()], ON);
    expect(trusted[0].routed).toBe(1);

    const untrusted = buildGroups(CATALOG, [], [domain()], { proxyOn: true, caTrusted: false });
    expect(untrusted[0].routed).toBe(0);
    expect(untrusted[0].members[0].attention).toBe("needs-trust");

    const off = buildGroups(CATALOG, [], [domain()], { proxyOn: false, caTrusted: true });
    expect(off[0].routed).toBe(0);
  });

  it("marks drifted and errored members for the summary", () => {
    const groups = buildGroups(
      CATALOG,
      [
        tool("codex", "Codex", { kind: "drifted", reason: "r" }, "OpenAI"),
        tool("claude-code", "Claude Code", { kind: "error", message: "m" }),
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
      CATALOG,
      [tool("claude-code", "Claude Code", { kind: "error", message: "m" })],
      [domain()],
      ON,
    );
    expect(groupSummary(group)).toEqual({
      count: "1 of 2 routing",
      exception: "Claude Code failed",
      kind: "error",
    });
  });

  it("aggregates several failures rather than naming one", () => {
    const [group] = buildGroups(
      CATALOG,
      [
        tool("opencode", "OpenCode", { kind: "error", message: "m" }, "your existing providers"),
        tool("openclaw", "OpenClaw", { kind: "error", message: "m" }, "your existing providers"),
      ],
      [],
      ON,
    );
    expect(groupSummary(group).exception).toBe("2 failed");
  });

  it("prefers the certificate over a drifted setup, and reports nothing when all is well", () => {
    const [blocked] = buildGroups(
      CATALOG,
      [tool("codex", "Codex", { kind: "drifted", reason: "r" }, "OpenAI")],
      [domain({ slug: "openai", display_name: "OpenAI apps" })],
      { proxyOn: true, caTrusted: false },
    );
    expect(groupSummary(blocked).exception).toBe("certificate not trusted");

    const [clean] = buildGroups(
      CATALOG,
      [tool("claude-code", "Claude Code", { kind: "connected" })],
      [],
      ON,
    );
    expect(groupSummary(clean)).toEqual({
      count: "1 of 1 routing",
      exception: null,
      kind: null,
    });
  });
});

describe("intent versus flow", () => {
  it("keeps an enabled domain switched on while the certificate blocks it", () => {
    // The round-6 P0: these were one field. `routed` false made the switch
    // render off, and clicking it sent `!enabled` = false, disabling the very
    // thing the user was trying to enable.
    const [group] = buildGroups(CATALOG, [], [domain()], { proxyOn: true, caTrusted: false });
    const m = group.members[0];
    expect(m.desired).toBe(true);
    expect(m.routed).toBe(false);
    expect(m.attention).toBe("needs-trust");
    expect(group.desired).toBe(1);
    expect(group.routed).toBe(0);
  });

  it("does not call a config tool routed while the master is off", () => {
    const [group] = buildGroups(
      CATALOG,
      [tool("claude-code", "Claude Code", { kind: "connected" })],
      [],
      { proxyOn: false, caTrusted: true },
    );
    const m = group.members[0];
    // Its config points at a relay that isn't running: switched on, not routing.
    expect(m.desired).toBe(true);
    expect(m.routed).toBe(false);
    expect(m.attention).toBe("master-off");
    expect(groupSummary(group)).toEqual({
      count: "0 of 1 routing",
      exception: "waiting on routing",
      kind: "master-off",
    });
  });

  it("still reports a tool as routing when the master is on", () => {
    const [group] = buildGroups(
      CATALOG,
      [tool("claude-code", "Claude Code", { kind: "connected" })],
      [],
      ON,
    );
    expect(group.members[0].routed).toBe(true);
    expect(group.members[0].attention).toBeNull();
  });
});
