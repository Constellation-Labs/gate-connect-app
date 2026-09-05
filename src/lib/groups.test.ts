import { describe, expect, it } from "vitest";
import type { ProviderState, ProxyDomain, Tool, Verdict } from "./api";
import type { Group, GroupMember } from "./groups";
import {
  buildGroups,
  groupSummary,
  MULTI_PROVIDER_ID,
  cascadeTargets,
  proxyReopenAdvice,
  PROXY_REOPEN_ADVICE,
} from "./groups";

function tool(slug: string, name: string, status: Tool["status"], upstream = "Anthropic"): Tool {
  return {
    slug,
    name,
    // The flat-list name; the rail's one-word label is `name`.
    product_name: name,
    upstream_provider_name: upstream,
    default_upstream_url: "https://api.anthropic.com",
    config_location: null,
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
  provider("anthropic", "Anthropic", ["claude-code"], ["anthropic"], ["claude-web"]),
  // No `domain_slugs`, mirroring the backend: the `openai` domain is generic
  // interception of api.openai.com and belongs to no OpenAI tool, so it moved
  // to the Experimental heading with the harnesses that depend on it.
  provider("openai", "OpenAI", ["codex"], [], ["chatgpt-apps", "chatgpt"]),
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

/** A sweep that says these slugs are routing.
 *
 * Every test that expects a config tool to read `routed` has to supply one now:
 * AG-570 forbids a completed file write from producing On by itself, so the
 * ledger takes the verdict and a member with none does not count. The helper is
 * the shape a caller actually passes - `verdictsBySlug`'s output. */
function sweep(...on: string[]): { verdicts: Map<string, Verdict> } {
  return {
    verdicts: new Map(
      on.map((slug) => [
        slug,
        {
          slug,
          state: "on" as const,
          reason: null,
          next_action: null,
          route_in_use: null,
          requested_route: null,
        },
      ]),
    ),
  };
}

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
    expect(groups.map((g) => g.name)).toEqual(["Anthropic", "OpenAI"]);
    expect(groups[0].members.map((m) => m.name)).toEqual([
      "Claude Code",
      "Claude Desktop / Cowork",
    ]);
  });

  it("gives the multi-provider tools headings of their own instead of a wrong family", () => {
    // Their upstream_provider_name is literally "your existing providers":
    // display prose that must never become a family name. It used to be one
    // "Other tools" row; each is its own heading now, which is what lets the
    // row underneath be named for a surface the way a family's rows are.
    const groups = buildGroups(
      CATALOG,
      [
        tool("opencode", "OpenCode", { kind: "detected" }, "your existing providers"),
        tool("openclaw", "CLI", { kind: "detected" }, "your existing providers"),
      ],
      [],
      ON,
    );
    expect(groups.map((g) => g.name)).toEqual(["OpenClaw", "Experimental"]);
    expect(groups.every((g) => g.multiProvider)).toBe(true);
    expect(groups.flatMap((g) => g.members.map((m) => m.key))).toEqual([
      "openclaw",
      "opencode",
    ]);
    expect(groups.map((g) => g.name).join()).not.toContain("existing providers");
    // No single upstream host is true of these, so the detail must not print one.
    expect(groups.every((g) => g.members.every((m) => m.coversAllProviders))).toBe(true);
    // A proper noun stays capitalised inside the sentence; a common noun does
    // not - the rule "other tools" was written for, applied to its successors.
    expect(groups[0].switchLabel).toBe("Route OpenClaw through Gate");
    expect(groups[1].switchLabel).toBe("Route experimental tools through Gate");
  });

  it("files the generic OpenAI host under Experimental, not under OpenAI", () => {
    // api.openai.com belongs to no OpenAI tool: Codex routes through the relay,
    // which resolves against the whole catalog rather than the enabled set, and
    // the ChatGPT desktop app is on chatgpt.com. Its switch intercepts that host
    // for any system-proxy client - which in practice is OpenClaw and Hermes,
    // sitting right beside it.
    const groups = buildGroups(
      CATALOG,
      [
        tool("codex", "CLI", { kind: "detected" }, "OpenAI"),
        tool("opencode", "OpenCode", { kind: "detected" }, "your existing providers"),
      ],
      [domain({ slug: "openai", display_name: "OpenAI API", enabled: false })],
      ON,
    );
    const experimental = groups.find((g) => g.id === "experimental");
    expect(experimental!.members.map((m) => m.key)).toEqual(["opencode", "openai"]);
    // A domain, so it keeps its host detail: `coversAllProviders` is a claim
    // about a config tool that repoints several providers, and would suppress
    // the one line this row has to show.
    const host = experimental!.members.find((m) => m.key === "openai");
    expect(host!.kind).toBe("proxy");
    expect(host!.coversAllProviders).toBeUndefined();
    // And it is genuinely gone from the family, not drawn twice.
    expect(
      groups.find((g) => g.id === "openai")!.members.map((m) => m.key),
    ).toEqual(["codex"]);
  });

  it("lets the Experimental switch cascade over that domain", () => {
    // It is not a chat surface - no session cookie, no subscription bearer - so
    // nothing exempts it from the heading's switch the way `claude-web` is
    // exempt from Anthropic's.
    const [group] = buildGroups(
      CATALOG,
      [],
      [domain({ slug: "openai", display_name: "OpenAI API", enabled: false })],
      ON,
    );
    expect(group.id).toBe("experimental");
    expect(cascadeTargets(group, true).map((m) => m.key)).toEqual(["openai"]);
  });

  it("draws the domain once when a provider still claims it", () => {
    // A slug in both places would otherwise get a row under its family and a
    // second one under the heading that named it.
    const claimedByOpenAi = CATALOG.map((p) =>
      p.slug === "openai" ? { ...p, domain_slugs: ["openai"] } : p,
    );
    const groups = buildGroups(
      claimedByOpenAi,
      [tool("codex", "CLI", { kind: "detected" }, "OpenAI")],
      [domain({ slug: "openai", display_name: "OpenAI API", enabled: false })],
      ON,
    );
    expect(groups.flatMap((g) => g.members.map((m) => m.key)).filter((k) => k === "openai"))
      .toHaveLength(1);
    expect(groups.find((g) => g.id === "experimental")).toBeUndefined();
  });

  it("keeps a catch-all for a tool no heading claims", () => {
    // The only thing that reaches "Other tools" now. An integration shipped
    // without a heading is a bug, and the row it should have had must not
    // vanish while someone fixes it.
    const groups = buildGroups(
      CATALOG,
      [tool("some-new-harness", "CLI", { kind: "detected" }, "your existing providers")],
      [],
      ON,
    );
    expect(groups).toHaveLength(1);
    expect(groups[0].id).toBe(MULTI_PROVIDER_ID);
    expect(groups[0].name).toBe("Other tools");
    expect(groups[0].multiProvider).toBe(true);
    expect(groups[0].switchLabel).toBe("Route other tools through Gate");
  });

  it("describes every row it can, because the labels no longer describe themselves", () => {
    // "App", "Web", "CLI" are legible under a heading and meaningless without a
    // sentence. `buildGroups` is where the two are joined, so a member that
    // reaches a surface without its description is the failure to catch here.
    const groups = buildGroups(
      CATALOG,
      [
        tool("claude-code", "CLI", { kind: "connected" }),
        tool("opencode", "OpenCode", { kind: "detected" }, "your existing providers"),
      ],
      [domain({ slug: "anthropic", display_name: "App" })],
      ON,
    );
    const byKey = new Map(
      groups.flatMap((g) => g.members).map((m) => [m.key, m.description]),
    );
    expect(byKey.get("claude-code")).toBe("Claude Code in your terminal.");
    expect(byKey.get("anthropic")).toBe("The Claude desktop app and Cowork.");
    expect(byKey.get("opencode")).toBe("The OpenCode editor.");
  });

  it("names the host in the sentence, not in the label", () => {
    // The one row whose subject is a host. The label stays a short sans phrase
    // and `api.openai.com` lives in the description: the popover already prints
    // the host in a mono identifier slot on this row, and the window UI prints
    // it nowhere else, so the sentence is the only place it belongs.
    const [group] = buildGroups(
      CATALOG,
      [],
      [domain({ slug: "openai", display_name: "OpenAI API", enabled: false })],
      ON,
    );
    const member = group.members.find((m) => m.key === "openai")!;
    expect(member.name).toBe("OpenAI API");
    expect(member.name).not.toContain("api.openai.com");
    expect(member.description).toContain("api.openai.com");
  });

  it("builds the rows the ledger could not previously reach", () => {
    // All three harnesses are listed, and each now heads its own row - the
    // six-row ledger the sizing note describes, which no test could produce
    // while every harness was hidden and then only ever made a fourth row.
    const groups = buildGroups(
      CATALOG,
      [
        tool("claude-code", "CLI", { kind: "connected" }),
        tool("opencode", "OpenCode", { kind: "detected" }, "your existing providers"),
        tool("openclaw", "CLI", { kind: "detected" }, "your existing providers"),
        tool("hermes", "CLI", { kind: "detected" }, "your existing providers"),
        tool("env-proxy", "Terminal tools", { kind: "detected" }, "your existing providers"),
      ],
      [],
      ON,
    );
    expect(groups.map((g) => g.name)).toEqual([
      "Anthropic",
      "OpenClaw",
      "Hermes",
      "Experimental",
    ]);
    // OpenCode and the environment channel share a heading because they share a
    // mechanism: OpenCode routes on the proxy variables, which is what the
    // Terminal tools row is.
    const experimental = groups.find((g) => g.id === "experimental");
    expect(experimental!.members.map((m) => m.key)).toEqual(["opencode", "env-proxy"]);
    // The family row is still its own row: a harness must not be absorbed into
    // Anthropic just because it can talk to Anthropic.
    const claude = groups.find((g) => g.id === "anthropic");
    expect(claude!.members.map((m) => m.key)).toEqual(["claude-code"]);
    // Nothing fell through to the catch-all: every leftover has a heading.
    expect(groups.find((g) => g.id === MULTI_PROVIDER_ID)).toBeUndefined();
  });

  it("drops families with nothing routable and leaves out what cannot route", () => {
    const groups = buildGroups(
      CATALOG,
      [tool("hermes", "Hermes", { kind: "not_installed" }, "openrouter")],
      [domain({ slug: "openai", display_name: "OpenAI API", supported: false })],
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
          display_name: "OpenAI API",
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
    // No `openai` row: that domain is generic interception of api.openai.com,
    // which no OpenAI tool rides, and it lives under Experimental now.
    expect(openai?.members.map((m) => m.key)).toEqual([
      "codex",
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
        // One group, so one summary: OpenCode and the environment channel are
        // the pair that shares a heading.
        tool("opencode", "OpenCode", { kind: "error", message: "m" }, "your existing providers"),
        tool("env-proxy", "Terminal tools", { kind: "error", message: "m" }, "your existing providers"),
      ],
      [],
      ON,
    );
    expect(groupSummary(group).exception).toBe("2 failed");
  });

  it("prefers the certificate over a drifted setup, and reports nothing when all is well", () => {
    const [blocked] = buildGroups(
      CATALOG,
      [tool("claude-code", "CLI", { kind: "drifted", reason: "r" })],
      [domain()],
      { proxyOn: true, caTrusted: false },
    );
    expect(groupSummary(blocked).exception).toBe("certificate not trusted");

    const [clean] = buildGroups(
      CATALOG,
      [tool("claude-code", "Claude Code", { kind: "connected" })],
      [],
      { ...ON, ...sweep("claude-code") },
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

  it("still reports a tool as routing when the master is on and the sweep agrees", () => {
    const [group] = buildGroups(
      CATALOG,
      [tool("claude-code", "Claude Code", { kind: "connected" })],
      [],
      { ...ON, ...sweep("claude-code") },
    );
    expect(group.members[0].routed).toBe(true);
    expect(group.members[0].attention).toBeNull();
  });

  /** The AG-570 rule, on the surface it is actually about: the popover's ledger
   *  used to read this state as routing, which is a claim about the user's
   *  traffic made from a file Gate itself wrote. */
  it("does not call a connected tool routed until a check says so", () => {
    const [group] = buildGroups(
      CATALOG,
      [tool("claude-code", "Claude Code", { kind: "connected" })],
      [],
      ON,
    );
    const m = group.members[0];
    // Intent is unchanged - the switch still reads on, because the user did ask
    // for this - and reality withholds.
    expect(m.desired).toBe(true);
    expect(m.routed).toBe(false);
    expect(m.attention).toBe("unverified");
    expect(groupSummary(group)).toEqual({
      count: "0 of 1 routing",
      exception: "Claude Code not verified",
      kind: "unverified",
    });
  });

  /** A sweep that concluded the tool is *not* routing is the same answer as a
   *  sweep that could not conclude, as far as this ledger goes: one value, and
   *  the reason lives in the window shell that has room for it. */
  it("treats a needs-attention verdict as unverified rather than as routing", () => {
    const [group] = buildGroups(
      CATALOG,
      [tool("claude-code", "Claude Code", { kind: "connected" })],
      [],
      {
        ...ON,
        verdicts: new Map([
          [
            "claude-code",
            {
              slug: "claude-code",
              state: "needs_attention" as const,
              reason: "reopen_required" as const,
              next_action: "reopen_tool" as const,
              route_in_use: "https://api.anthropic.com",
              requested_route: "https://gateway.example.com",
            },
          ],
        ]),
      },
    );
    expect(group.members[0].routed).toBe(false);
    expect(group.members[0].attention).toBe("unverified");
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

/**
 * The one piece of routing copy that is true on one platform and false on the
 * other two, which is why it is a function of the platform rather than a
 * sentence somebody could paste into a shared component.
 */
describe("proxyReopenAdvice", () => {
  it("is Linux-only, because that is where the environment channel is", () => {
    // Windows refreshes WinINET after the registry write and macOS's auto-proxy
    // URL is applied to new connections as it changes, so on both the advice
    // would be wrong rather than merely cautious.
    expect(proxyReopenAdvice("proxy", "linux")).toBe(PROXY_REOPEN_ADVICE);
    expect(proxyReopenAdvice("proxy", "macos")).toBeUndefined();
    expect(proxyReopenAdvice("proxy", "windows")).toBeUndefined();
    expect(proxyReopenAdvice("proxy", "unknown")).toBeUndefined();
  });

  it("says nothing on a config-routed row, on any platform", () => {
    // Those have a file to re-read and a verdict that measures it. Advice beside
    // a reading would invite the reader to weigh a guess against a measurement.
    expect(proxyReopenAdvice("config", "linux")).toBeUndefined();
  });

  it("says out loud that it is not a reading", () => {
    // Principle 6 in the other direction: this is the one routing line with
    // nothing behind it, so it has to admit that in its own words.
    expect(PROXY_REOPEN_ADVICE.body).toContain("advice rather than a reading");
  });
});
