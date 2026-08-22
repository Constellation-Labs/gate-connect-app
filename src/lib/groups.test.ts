import { describe, expect, it } from "vitest";
import type { ProviderState, ProxyDomain, Tool } from "./api";
import type { Group, GroupMember } from "./groups";
import { buildGroups, groupSummary, MULTI_PROVIDER_ID, cascadeTargets } from "./groups";

function tool(slug: string, name: string, status: Tool["status"], upstream = "Anthropic"): Tool {
  return {
    slug,
    name,
    upstream_provider_name: upstream,
    default_upstream_url: "https://api.anthropic.com",
    requires_upstream_credential: false,
  config_location: null,
  // No model preference in these fixtures: they exercise routing and grouping,
  // and a platform id only matters where a model is chosen.
  platform_id: null,
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
  chat_domain_slugs: string[] = [],
): ProviderState {
  return {
    slug,
    display_name,
    subtitle: "",
    enabled: false,
    available: true,
    tool_slugs,
    domain_slugs,
    chat_domain_slugs,
  };
}

/** Mirrors the real catalog: Claude Code and Codex are each claimed by one
 * provider; OpenCode / OpenClaw / Hermes deliberately are not. Both families
 * also carry credential-sensitive domains, which the catalog keeps out of
 * `domain_slugs` so the family switch cannot reach them - and OpenAI carries
 * two of them, both on chatgpt.com. */
const CATALOG = [
  provider("anthropic", "Claude", ["claude-code"], ["anthropic"], ["claude-web"]),
  provider("openai", "OpenAI", ["codex"], ["openai"], ["chatgpt-apps", "chatgpt"]),
  provider("openrouter", "OpenRouter", [], ["openrouter"]),
];

/** The chat-protocol domain as the backend ships it: supported, off. */
function chatDomain(overrides: Partial<ProxyDomain> = {}): ProxyDomain {
  return domain({
    slug: "claude-web",
    display_name: "Claude Desktop chat",
    hosts: ["claude.ai"],
    upstream_url: "https://claude.ai/api",
    enabled: false,
    ...overrides,
  });
}

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

  it("gives a chat surface a row under its family, after the cascaded members", () => {
    const [claude] = buildGroups(
      CATALOG,
      [tool("claude-code", "Claude Code", { kind: "connected" })],
      [domain(), chatDomain()],
      ON,
    );
    expect(claude.members.map((m) => m.name)).toEqual([
      "Claude Code",
      "Claude Desktop / Cowork",
      "Claude Desktop chat",
    ]);
    expect(claude.members.map((m) => m.chat)).toEqual([undefined, undefined, true]);
  });

  it("keeps a chat surface out of the family switch's count", () => {
    // The switch is driven by `cascadeDesired` precisely so this case cannot
    // happen: the chat row is the only thing switched on, so a `desired`-driven
    // switch would read "on" over a family routing nothing it can flip, and
    // clicking it would ask to turn off an already-off set.
    const [claude] = buildGroups(
      CATALOG,
      [],
      [domain({ enabled: false }), chatDomain({ enabled: true })],
      ON,
    );
    expect(claude.desired).toBe(1);
    expect(claude.cascadeDesired).toBe(0);
    // Reality still speaks for every member: the chat surface IS routing.
    expect(claude.routed).toBe(1);
    expect(groupSummary(claude).count).toBe("1 of 2 routing");
  });

  it("does not let a family switch reach the chat surface", () => {
    // `App.tsx`'s `setGroupRouted` filters on `chat`, and the backend keeps
    // these slugs out of `proxy_domain_slugs` for the same reason. This pins
    // the field the frontend half filters on.
    const [claude] = buildGroups(CATALOG, [], [domain(), chatDomain()], ON);
    const cascade = claude.members.filter((m) => !m.chat);
    expect(cascade.map((m) => m.key)).toEqual(["anthropic"]);
    expect(claude.cascadeDesired).toBe(1);
  });

  it("gives OpenAI both of its credential-sensitive rows, each on its own switch", () => {
    // Two entries claim chatgpt.com and each serves paths the other ignores:
    // the app's chat turn, and the Responses endpoint a ChatGPT subscription
    // reaches (the one OpenClaw's model calls need). Rows rather than a
    // side effect of connecting a tool, which is what OpenClaw used to do.
    const groups = buildGroups(
      CATALOG,
      [tool("codex", "Codex", { kind: "detected" }, "OpenAI")],
      [
        domain({
          slug: "openai",
          display_name: "OpenAI apps",
          hosts: ["api.openai.com"],
          enabled: false,
        }),
        chatDomain({
          slug: "chatgpt-apps",
          display_name: "ChatGPT app chat + Codex tools",
          hosts: ["chatgpt.com"],
          enabled: true,
        }),
        chatDomain({
          slug: "chatgpt",
          display_name: "ChatGPT (Codex subscription)",
          hosts: ["chatgpt.com"],
          enabled: true,
        }),
      ],
      ON,
    );
    const openai = groups.find((g) => g.id === "openai");
    expect(openai?.members.map((m) => m.key)).toEqual([
      "codex",
      "openai",
      "chatgpt-apps",
      "chatgpt",
    ]);
    expect(openai?.members.filter((m) => m.chat).map((m) => m.key)).toEqual([
      "chatgpt-apps",
      "chatgpt",
    ]);
    // Both switched on, and the family switch still reads off, because neither
    // is its to flip - however many of them there are.
    expect(openai?.desired).toBe(2);
    expect(openai?.cascadeDesired).toBe(0);
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

describe("cascadeTargets", () => {
  const member = (over: Partial<GroupMember> = {}): GroupMember => ({
    key: "codex",
    kind: "config",
    name: "Codex",
    routed: false,
    desired: false,
    attention: null,
    ...over,
  });
  const group = (members: GroupMember[]): Group => ({
    id: "openai",
    name: "OpenAI",
    switchLabel: "Route OpenAI through Gate",
    members,
    routed: members.filter((m) => m.routed).length,
    desired: members.filter((m) => m.desired).length,
    cascadeDesired: members.filter((m) => m.desired && !m.chat).length,
  });

  it("never rides a chat member on a family switch", () => {
    // They intercept a session-cookie surface, so routing one is a deliberate
    // per-row act. The backend keeps those slugs out of proxy_domain_slugs for
    // the same reason.
    const g = group([member(), member({ key: "chatgpt", name: "ChatGPT", kind: "proxy", chat: true })]);
    expect(cascadeTargets(g, true).map((m) => m.key)).toEqual(["codex"]);
    expect(cascadeTargets(g, false).map((m) => m.key)).toEqual([]);
  });

  it("never adopts a drifted config from a family switch", () => {
    // That decision belongs to the review dialog, not to a switch two levels up.
    const g = group([member({ attention: "drifted" })]);
    expect(cascadeTargets(g, true)).toEqual([]);
  });

  it("still turns a drifted member off", () => {
    // Disconnecting restores what was there, so there is nothing to review.
    const g = group([member({ attention: "drifted", desired: true })]);
    expect(cascadeTargets(g, false).map((m) => m.key)).toEqual(["codex"]);
  });

  it("leaves members that already say the right thing alone", () => {
    // So a family switch does not rewrite a config that is already correct.
    const on = group([member({ desired: true })]);
    expect(cascadeTargets(on, true)).toEqual([]);
    const off = group([member({ desired: false })]);
    expect(cascadeTargets(off, false)).toEqual([]);
  });

  it("takes proxy members by their enabled flag, not by what is flowing", () => {
    // An enabled domain behind an untrusted certificate is not routed, and a
    // family switch turning it "on" again would be a no-op the user cannot see.
    const g = group([
      member({ key: "anthropic-api", kind: "proxy", name: "Anthropic API", desired: true, routed: false }),
    ]);
    expect(cascadeTargets(g, true)).toEqual([]);
    expect(cascadeTargets(g, false).map((m) => m.key)).toEqual(["anthropic-api"]);
  });
});
