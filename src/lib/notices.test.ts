import { describe, expect, it } from "vitest";
import type { Group, GroupMember, MemberAttention } from "./groups";
import { machineNotices, memberNotices } from "./notices";

/**
 * Which surface gets which card. `noticeFor`'s copy is asserted through the
 * two builders rather than on its own: what matters is that Overview collapses
 * the whole-machine causes and never draws a tool's own, and that a pane gets
 * one card per member, named for that member and dismissable by its key.
 */

function member(key: string, attention: MemberAttention, name = key): GroupMember {
  return {
    key,
    kind: "config",
    name,
    routed: false,
    desired: true,
    attention,
    tool: {
      slug: key,
      name,
      product_name: name,
      upstream_provider_name: "OpenAI",
      default_upstream_url: "https://gw.example",
      config_location: null,
      client: "codex",
      scope: "client",
      credential: "brokered",
      status:
        attention === "error"
          ? { kind: "error", message: `${name} read failed` }
          : { kind: "connected" },
    },
    client: "codex",
    scope: "client",
    credential: "brokered",
    cascade: true,
  };
}

function group(id: string, members: GroupMember[]): Group {
  return {
    id,
    name: id,
    switchLabel: id,
    band: "apps",
    members,
    routed: 0,
    desired: members.length,
    cascadeDesired: members.length,
    switchOn: true,
    switchDesired: members.length,
  };
}

const groups = [
  group("chatgpt", [member("codex", "not-routing", "CLI"), member("chatgpt", "not-routing")]),
  group("openrouter", [member("openrouter", "not-routing")]),
  group("claude", [member("claude-code", "drifted", "Claude Code")]),
  group("opencode", [member("opencode", "error", "OpenCode")]),
];

describe("machineNotices", () => {
  it("collapses a whole-machine cause to one counted card keyed on the cause", () => {
    const notices = machineNotices(groups);
    expect(notices).toHaveLength(1);
    expect(notices[0]).toMatchObject({
      id: "not-routing",
      title: "3 apps aren’t protected",
      action: { kind: "enable-routing" },
    });
  });

  it("names the one tool when only one is affected", () => {
    const notices = machineNotices([group("openrouter", [member("openrouter", "needs-trust")])]);
    expect(notices).toHaveLength(1);
    expect(notices[0]).toMatchObject({
      id: "needs-trust:openrouter",
      title: "openrouter needs the Gate certificate",
    });
  });

  it("never draws a tool's own cause", () => {
    // Drift and a check error are about one tool and belong on its pane.
    expect(machineNotices(groups.slice(2))).toEqual([]);
  });
});

describe("memberNotices", () => {
  it("gives every affected member its own card without a count", () => {
    const notices = memberNotices(groups, ["codex", "chatgpt"]);
    expect(notices.map((n) => n.id)).toEqual(["not-routing:codex", "not-routing:chatgpt"]);
    expect(notices[0].title).toBe("CLI isn’t protected");
  });

  it("only sees the members it was asked about", () => {
    // OpenRouter's cause is the same as ChatGPT's; its card is still its own.
    expect(memberNotices(groups, ["openrouter"]).map((n) => n.id)).toEqual([
      "not-routing:openrouter",
    ]);
    expect(memberNotices(groups, ["nothing-here"])).toEqual([]);
  });

  it("draws the per-tool causes Overview leaves out, and names the tool", () => {
    expect(memberNotices(groups, ["claude-code"])[0]).toMatchObject({
      id: "drifted:claude-code",
      body: "Claude Code's config changed outside Gate, so its traffic isn't routed.",
      action: { kind: "reconnect", slug: "claude-code" },
    });
    expect(memberNotices(groups, ["opencode"])[0]).toMatchObject({
      id: "error:opencode",
      body: "OpenCode read failed",
    });
  });

  it("ranks by how directly the user can act, not by section order", () => {
    const mixed = [group("x", [member("a", "error"), member("b", "drifted"), member("c", "not-routing")])];
    expect(memberNotices(mixed, ["a", "b", "c"]).map((n) => n.id)).toEqual([
      "not-routing:c",
      "drifted:b",
      "error:a",
    ]);
  });
});
