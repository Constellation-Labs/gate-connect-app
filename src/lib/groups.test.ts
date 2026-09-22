import { describe, expect, it } from "vitest";
import type { ClientId, Credential, ProxyDomain, Scope, Tool, Verdict } from "./api";
import type { Band, Group, GroupMember } from "./groups";
import { sectionStatus } from "./verdict";
import {
  PROXY_REOPEN_ADVICE,
  BAND_LABELS,
  isProviderEndpoint,
  SECTIONS,
  browserTrustRestartAdvice,
  buildGroups,
  cascadeTargets,
  groupSummary,
  hasBrowserSurface,
  hostReloadAdvice,
  isSettingsManaged,
  needsSessionConsent,
  proxyReopenAdvice,
  sessionMembers,
} from "./groups";

/** A tool row as the backend ships one.
 *
 * `client` is the grouping key now, so it is a parameter rather than a
 * constant: the thing under test is which heading a row lands under, and a
 * fixture that always said "claude-code" could not fail that. `scope` and
 * `credential` default to what every config tool answers. */
function tool(
  slug: string,
  name: string,
  status: Tool["status"],
  client: ClientId = "claude-code",
  overrides: Partial<Tool> = {},
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
    scope: "client" as Scope,
    credential: "brokered" as Credential,
    ...overrides,
  };
}

/** A domain row as the catalog ships one: brokered, host-scoped, aimed at the
 * Claude desktop app. The session surfaces override `credential`. */
function domain(overrides: Partial<ProxyDomain> = {}): ProxyDomain {
  return {
    slug: "anthropic",
    display_name: "API",
    hosts: ["api.anthropic.com"],
    upstream_url: "https://api.anthropic.com",
    rewrite_prefixes: [],
    passthrough_prefixes: [],
    enabled: true,
    supported: true,
    client: "claude-desktop",
    credential: "brokered",
    scope: "host",
    ...overrides,
  };
}

/** The session-credential domain as the backend ships it: supported, off, and
 * `additive`, which is the single fact that keeps it off a group switch. */
function sessionDomain(overrides: Partial<ProxyDomain> = {}): ProxyDomain {
  return domain({
    slug: "claude-web",
    display_name: "Chat",
    hosts: ["claude.ai"],
    upstream_url: "https://claude.ai/api",
    enabled: false,
    credential: "additive",
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
  it("draws one row per app, not one per routable surface", () => {
    // The shape this file exists to pin. Claude Code's CLI, the desktop app's
    // API surface and its chat surface are three mechanisms and one app, and
    // the user gets one switch.
    const groups = buildGroups(
      [tool("claude-code", "CLI", { kind: "connected" })],
      [domain(), sessionDomain()],
      ON,
    );
    expect(groups.map((g) => g.name)).toEqual(["Claude"]);
    expect(groups[0].members.map((m) => m.key)).toEqual([
      "claude-code",
      "anthropic",
      "claude-web",
    ]);
    expect(groups[0].band).toBe("anthropic");
  });

  it("puts Codex and the ChatGPT app's surfaces on one switch", () => {
    const groups = buildGroups(
      [tool("codex", "CLI", { kind: "detected" }, "codex")],
      [
        sessionDomain({ slug: "chatgpt-apps", hosts: ["chatgpt.com"], client: "chatgpt" }),
        sessionDomain({ slug: "chatgpt", hosts: ["chatgpt.com"], client: "chatgpt" }),
      ],
      ON,
    );
    expect(groups.map((g) => g.name)).toEqual(["ChatGPT / Codex"]);
    expect(groups[0].members.map((m) => m.key)).toEqual([
      "codex",
      "chatgpt-apps",
      "chatgpt",
    ]);
  });

  it("keeps the environment channel and the OpenAI host on separate switches", () => {
    // Both are `any-app` and they are still two rows: different mechanisms,
    // different blast radii, and nothing depends on both. Folding either into
    // an app switch would route it as a side effect of routing that app.
    const groups = buildGroups(
      [tool("env-proxy", "Terminal tools", { kind: "detected" }, "any-app", { scope: "machine" })],
      [domain({ slug: "openai", display_name: "OpenAI API", client: "any-app" })],
      ON,
    );
    // Order follows `SECTIONS`, where `openai-api` now sits beside `chatgpt`
    // so the OpenAI group's rows are contiguous. It used to be last in the
    // array, which is what drew the group's header twice.
    expect(groups.map((g) => g.name)).toEqual(["OpenAI API", "Terminal"]);
    // Two rows, and since 2026-09-22 two different groups: grouping is by
    // vendor, so the OpenAI host joins OpenAI while the environment channel
    // has no vendor and falls to the catch-all. They used to share a band,
    // which is what made "separate switches" the only thing this could assert;
    // the separateness is the point either way.
    expect(groups.map((g) => g.band)).toEqual(["openai", "other"]);
  });

  it("keeps a tool and a domain of the same slug in one section, both drawn", () => {
    // `opencode` is both a tool slug and a domain slug, so the two share a
    // key. A map keyed on the slug alone would silently draw one of them.
    const groups = buildGroups(
      [tool("opencode", "OpenCode", { kind: "detected" }, "opencode")],
      [domain({ slug: "opencode", display_name: "Zen / Go", client: "opencode" })],
      ON,
    );
    expect(groups).toHaveLength(1);
    expect(groups[0].members.map((m) => m.kind)).toEqual(["config", "proxy"]);
  });

  it("gives a member no section names a section of its own", () => {
    // A catalog entry added before anyone gives it a home must be visible, not
    // absent. Every hand-kept grouping this file has had failed the other way.
    const groups = buildGroups(
      [tool("some-new-harness", "CLI", { kind: "detected" }, "opencode")],
      [],
      ON,
    );
    expect(groups.map((g) => g.id)).toEqual(["some-new-harness"]);
    expect(groups[0].members.map((m) => m.key)).toEqual(["some-new-harness"]);
  });

  it("reads the switch off the brokered half, so a session row alone is not routing", () => {
    // The switch FLIPS the session row and RENDERS off the brokered ones. With
    // only the chat surface on, a `desired`-driven switch would read on over an
    // app whose model calls are not routed at all.
    const [claude] = buildGroups(
      [],
      [domain({ enabled: false }), sessionDomain({ enabled: true })],
      ON,
    );
    expect(claude.desired).toBe(1);
    expect(claude.cascadeDesired).toBe(0);
    expect(claude.routed).toBe(1);
  });

  it("carries the taxonomy onto every member", () => {
    const [claude] = buildGroups(
      [tool("claude-code", "CLI", { kind: "connected" })],
      [domain(), sessionDomain()],
      ON,
    );
    const chat = claude.members.find((m) => m.key === "claude-web")!;
    expect(chat.credential).toBe("additive");
    expect(chat.scope).toBe("host");
    expect(chat.cascade).toBe(false);
    expect(claude.members.find((m) => m.key === "anthropic")!.cascade).toBe(true);
  });

  it("drops sections with nothing routable and leaves out what cannot route", () => {
    const groups = buildGroups(
      [tool("hermes", "Hermes", { kind: "not_installed" }, "hermes")],
      [domain({ slug: "openai", display_name: "OpenAI API", supported: false })],
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

  it("marks drifted and errored members for the summary", () => {
    const groups = buildGroups(
      [
        tool("codex", "Codex", { kind: "drifted", reason: "r" }, "codex"),
        tool("claude-code", "Claude Code", { kind: "error", message: "m" }),
      ],
      [],
      ON,
    );
    const byKey = new Map(groups.flatMap((g) => g.members).map((m) => [m.key, m.attention]));
    expect(byKey.get("codex")).toBe("drifted");
    expect(byKey.get("claude-code")).toBe("error");
  });
});

describe("session consent", () => {
  it("is needed by the app sections that hold a signed-in surface", () => {
    const [claude] = buildGroups([], [domain(), sessionDomain()], ON);
    expect(needsSessionConsent(claude)).toBe(true);
    expect(sessionMembers(claude).map((m) => m.key)).toEqual(["claude-web"]);
  });

  it("is not needed by a section whose rows are all brokered", () => {
    const [openrouter] = buildGroups(
      [],
      [domain({ slug: "openrouter", display_name: "OpenRouter", client: "any-app" })],
      ON,
    );
    expect(needsSessionConsent(openrouter)).toBe(false);
    expect(sessionMembers(openrouter)).toEqual([]);
  });

  it("lets the app switch reach the session row, which is the whole shape", () => {
    // The one place the frontend deliberately breaks the invariant the rest of
    // the tree enforces. `provider::cascade_domains` still refuses these rows
    // in Rust, so the CLI and the restore path cannot route them; here consent
    // is what stands in for the refusal.
    const [claude] = buildGroups(
      [],
      [domain({ enabled: false }), sessionDomain({ enabled: false })],
      ON,
    );
    // Only with `sessions`, which is the caller saying it has asked. Without it
    // the session row is left alone - which is what the popover does, having no
    // dialog to ask with.
    expect(cascadeTargets(claude, true, { sessions: true }).map((m) => m.key)).toEqual([
      "anthropic",
      "claude-web",
    ]);
    expect(cascadeTargets(claude, true).map((m) => m.key)).toEqual(["anthropic"]);
  });
});

/**
 * `sectionStatus` lives in `./verdict`, and its tests live here on purpose.
 *
 * What they exercise is the status line over a REAL ledger - `buildGroups`'
 * output, with this file's tool, domain and sweep fixtures behind it - because
 * the questions they ask are about member composition: a drifted sibling, a
 * section that is partly routed, a session surface that must not define the
 * line. Moved next to the function, they would need those fixtures copied, and
 * a copy of a fixture is how two files come to disagree about what a section is.
 */
describe("sectionStatus", () => {
  it("lets an exception outrank routing, so a section cannot claim more than it does", () => {
    // A section spans mechanisms, so one surface can be drifted while another
    // routes. Reporting "Protected" over that is the claim principle 6
    // forbids.
    const [claude] = buildGroups(
      [tool("claude-code", "CLI", { kind: "drifted", reason: "r" })],
      [domain()],
      ON,
    );
    const apps = new Map([["claude-code", { status: { kind: "drifted" } } as never]]);
    expect(sectionStatus(claude, apps)?.kind).toBe("drifted");
  });

  it("says partly protected, a state a per-surface ledger never had to describe", () => {
    const [opencode] = buildGroups(
      [tool("opencode", "OpenCode", { kind: "connected" }, "opencode")],
      [domain({ slug: "opencode", display_name: "Zen / Go", client: "opencode", enabled: false })],
      { ...ON, ...sweep("opencode") },
    );
    const apps = new Map([["opencode", { status: { kind: "protected" } } as never]]);
    // The count, not the adverb: "partly" does not say how much of the app is
    // covered, and this line used to render as a bare "Not protected" because
    // the rail drops a `not-protected` detail.
    expect(sectionStatus(opencode, apps)).toEqual({
      kind: "partly-protected",
      detail: "1 of 2",
    });
  });

  /**
   * The common shape of a group switch: it writes one config and enables the
   * hosts beside it, the hosts route immediately, and the tool waits on a
   * restart. The honest line names the restart.
   *
   * **And nothing else.** This asserted `detail: "CLI"` until 2026-09-22, on
   * the reasoning that the heading is the app and the suffix should say which
   * program inside it to reopen. `Tool.name` is a surface label, so the rail
   * drew the row's state and the row's TYPE in one line - "Reopen to finish -
   * CLI" - which design rejected: "we're mixing CLI and On/Off ... the type
   * cannot live in the same label."
   *
   * The product name is not the fix either. On a one-member section it
   * repeats the row's own heading, which is why the old code special-cased
   * those, and on a multi-member one the remedy is the same whichever member
   * it is: reopen the app this row names. The pane says which program, where
   * there is room for it.
   */
  it("names the restart and nothing else", () => {
    const [claude] = buildGroups(
      [tool("claude-code", "CLI", { kind: "connected" })],
      [domain()],
      ON,
    );
    const apps = new Map([["claude-code", { status: { kind: "reopen" } } as never]]);
    const status = sectionStatus(claude, apps);

    expect(status).toEqual({ kind: "reopen" });
    // Named explicitly: a suffix of any kind is what design ruled out, so an
    // assertion that only checks the kind would let the type back in.
    expect(status).not.toHaveProperty("detail");
  });

  it("does not read off as off because a session surface is off", () => {
    // The session row answers the same switch but does not define it: with the
    // brokered surface routing, the section is routing.
    const [claude] = buildGroups([], [domain(), sessionDomain()], ON);
    expect(sectionStatus(claude, new Map())).toEqual({ kind: "protected" });
  });
});

describe("member hints", () => {
  it("names the desktop apps, which nothing else in the ledger does", () => {
    // The row that routes Cowork is labelled "API" under a heading reading
    // "Claude". These apps have no config file, so they never reach
    // `list_tools`, and without this the word "Cowork" appears nowhere a user
    // could find it.
    const [claude] = buildGroups(
      [tool("claude-code", "CLI", { kind: "connected" })],
      [domain(), sessionDomain()],
      ON,
    );
    const byKey = new Map(claude.members.map((m) => [m.key, m.hint]));
    // Named inside its app, not beside it: Cowork is a mode in the Claude
    // desktop app rather than a product of its own.
    expect(byKey.get("anthropic")).toContain("Cowork included");
    expect(byKey.get("claude-web")).toContain("Claude desktop app");
  });

  it("names the CLI's other surface, which is not a terminal", () => {
    const [claude] = buildGroups([tool("claude-code", "CLI", { kind: "connected" })], [], ON);
    expect(claude.members[0].hint).toBe("Claude Code CLI and IDE plugins");
  });

  it("scopes the session row to the surface, not to the whole app", () => {
    // Both Claude rows name the same product, and they route different halves
    // of it: model calls on the API row, chat turns on this one.
    const [claude] = buildGroups([], [domain(), sessionDomain()], ON);
    const chat = claude.members.find((m) => m.key === "claude-web")!;
    expect(chat.hint).toMatch(/^Chats in/);
  });

  it("gives the OpenAI rows their equivalents", () => {
    const groups = buildGroups(
      [tool("codex", "CLI", { kind: "detected" }, "codex")],
      [
        sessionDomain({ slug: "chatgpt-apps", hosts: ["chatgpt.com"], client: "chatgpt" }),
        sessionDomain({ slug: "chatgpt", hosts: ["chatgpt.com"], client: "chatgpt" }),
      ],
      ON,
    );
    const byKey = new Map(groups.flatMap((g) => g.members).map((m) => [m.key, m.hint]));
    expect(byKey.get("codex")).toContain("IDE extension");
    expect(byKey.get("chatgpt-apps")).toMatch(/^Chats in/);
    // Work, not Cowork: one letter apart, different vendors, and naming them as
    // modes of their host app rather than as apps is the correction that cost
    // a process row - see `AGENT_PROCESSES`.
    expect(byKey.get("chatgpt")).toContain("Work in the ChatGPT app");
    expect(byKey.get("chatgpt")).not.toContain("Cowork");
  });

  it("says nothing on a row whose subject is a host rather than a product", () => {
    const [group] = buildGroups(
      [],
      [domain({ slug: "openai", display_name: "OpenAI API", client: "any-app" })],
      ON,
    );
    expect(group.members[0].hint).toBeUndefined();
  });
});

describe("groupSummary", () => {
  it("counts, and names a single failure", () => {
    // OpenCode's editor and its Zen / Go host, which is a group with both
    // kinds of member in it. Claude Code and the `anthropic` domain used to be
    // that pair and are two groups now: one is the CLI, the other the desktop
    // app, and the summary is per group.
    const [group] = buildGroups(
      [tool("opencode", "OpenCode", { kind: "error", message: "m" }, "opencode")],
      [domain({ slug: "opencode", display_name: "Zen / Go", client: "opencode" })],
      ON,
    );
    expect(groupSummary(group)).toEqual({
      count: "1 of 2 routing",
      exception: "OpenCode failed",
      kind: "error",
    });
  });

  it("aggregates several failures rather than naming one", () => {
    // One section with two failing members: OpenCode's editor and its Zen / Go
    // host. The environment channel is its own section now, so pairing those
    // two would be two summaries rather than one.
    const [group] = buildGroups(
      [tool("opencode", "OpenCode", { kind: "error", message: "m" }, "opencode")],
      [domain({ slug: "opencode", display_name: "Zen / Go", client: "opencode", enabled: false })],
      { proxyOn: false, caTrusted: true },
    );
    expect(group.members).toHaveLength(2);
    expect(groupSummary(group).exception).toBe("OpenCode failed");
  });

  it("prefers the certificate over a drifted setup, and reports nothing when all is well", () => {
    const [blocked] = buildGroups(
      [tool("opencode", "OpenCode", { kind: "drifted", reason: "r" }, "opencode")],
      [domain({ slug: "opencode", display_name: "Zen / Go", client: "opencode" })],
      { proxyOn: true, caTrusted: false },
    );
    expect(groupSummary(blocked).exception).toBe("certificate not trusted");

    const [clean] = buildGroups(
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

/**
 * AG-674 in the ledger. The row has to say the traffic is somewhere else while
 * the switch keeps saying what the user asked for, which is the same split this
 * module opens with - one story later.
 */
describe("an overridden tool", () => {
  it("is switched on, not routing, and named as routed elsewhere", () => {
    const [group] = buildGroups(
      [
        tool("claude-code", "Claude Code", {
          kind: "overridden",
          source: "/etc/claude-code/managed-settings.json sets HTTPS_PROXY",
        }),
      ],
      [],
      ON,
    );
    const m = group.members[0];
    // Intent: Gate's configuration is in Gate's file, which is exactly what the
    // switch means. Rendering it off would make the click turn off the thing the
    // user was trying to keep on.
    expect(m.desired).toBe(true);
    expect(m.routed).toBe(false);
    expect(m.attention).toBe("overridden");
    expect(groupSummary(group)).toEqual({
      count: "0 of 1 routing",
      exception: "Claude Code routed elsewhere",
      kind: "overridden",
    });
  });

});

describe("intent versus flow", () => {
  it("keeps an enabled domain switched on while the certificate blocks it", () => {
    // The round-6 P0: these were one field. `routed` false made the switch
    // render off, and clicking it sent `!enabled` = false, disabling the very
    // thing the user was trying to enable.
    const [group] = buildGroups([], [domain()], { proxyOn: true, caTrusted: false });
    const m = group.members[0];
    expect(m.desired).toBe(true);
    expect(m.routed).toBe(false);
    expect(m.attention).toBe("needs-trust");
    expect(group.desired).toBe(1);
    expect(group.routed).toBe(0);
  });

  it("does not call a config tool routed while the master is off", () => {
    const [group] = buildGroups(
      [tool("claude-code", "Claude Code", { kind: "connected" })],
      [],
      { proxyOn: false, caTrusted: true },
    );
    const m = group.members[0];
    // Its config points at a relay that isn't running: switched on, not routing.
    expect(m.desired).toBe(true);
    expect(m.routed).toBe(false);
    expect(m.attention).toBe("not-routing");
    expect(groupSummary(group)).toEqual({
      count: "0 of 1 routing",
      exception: "not routing",
      kind: "not-routing",
    });
  });

  it("still reports a tool as routing when the master is on and the sweep agrees", () => {
    const [group] = buildGroups(
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
    client: "codex",
    scope: "client",
    credential: "brokered",
    cascade: true,
    ...over,
  });
  const group = (members: GroupMember[]): Group => ({
    id: "codex",
    name: "Codex",
    band: "anthropic",
    switchLabel: "Route Codex through Gate",
    members,
    routed: members.filter((m) => m.routed).length,
    desired: members.filter((m) => m.desired).length,
    cascadeDesired: members.filter((m) => m.desired && m.cascade).length,
    // The app switch's own predicate, spelled out rather than imported so a
    // change to it fails these tests loudly instead of quietly rewriting what
    // they assert. `governingMembers`' fallback included: a section with no
    // brokered member is described by the members it has.
    switchOn: governing(members).some(isIntended),
    switchDesired: governing(members).filter(isIntended).length,
  });

  it("rides an additive member only when the caller has asked", () => {
    // Reversed deliberately. A section covers everything an app does, and for
    // ChatGPT that includes a surface the user is signed in to. What stands in
    // for the old refusal is `needsSessionConsent`, checked by the caller
    // before it gets here; the backend's `cascade_domains` still refuses these
    // rows, so the CLI and the restore path are unaffected.
    const g = group([
      member(),
      member({
        key: "chatgpt",
        name: "Subscription",
        kind: "proxy",
        credential: "additive",
        scope: "host",
        cascade: false,
      }),
    ]);
    expect(cascadeTargets(g, true, { sessions: true }).map((m) => m.key)).toEqual([
      "codex",
      "chatgpt",
    ]);
    // And the default stays the safe one: a caller that has not asked gets the
    // brokered half, which is what it has always got.
    expect(cascadeTargets(g, true).map((m) => m.key)).toEqual(["codex"]);
    expect(cascadeTargets(g, false, { sessions: true }).map((m) => m.key)).toEqual([]);
  });

  it("never adopts a drifted config from a family switch", () => {
    // That decision belongs to the review dialog, not to a switch two levels up.
    const g = group([member({ attention: "drifted" })]);
    expect(cascadeTargets(g, true)).toEqual([]);
  });

  it("never re-applies a configuration something else outranks", () => {
    // The switch writes Gate's config, and an overridden member already has it.
    // Including it would spend a click on a no-op the user reads as a fix.
    const g = group([member({ attention: "overridden" })]);
    expect(cascadeTargets(g, true)).toEqual([]);
  });

  it("still turns an overridden member off", () => {
    // Disconnecting removes our values, which is real work whoever is winning.
    const g = group([member({ attention: "overridden", desired: true })]);
    expect(cascadeTargets(g, false).map((m) => m.key)).toEqual(["codex"]);
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

/**
 * Which rows a browser tab can be sitting on, and what to say when one is
 * routed. The taxonomy cannot answer the first question - see
 * `BROWSER_SURFACE_ROWS` - so these are the tests that hold the answer in place.
 */
describe("hasBrowserSurface", () => {
  const members = (...domains: ProxyDomain[]) =>
    buildGroups([], domains, ON).flatMap((g) => g.members);

  it("is true for the two surfaces a person opens in a browser", () => {
    const [claudeWeb] = members(sessionDomain());
    const [chatgptApps] = members(
      sessionDomain({ slug: "chatgpt-apps", hosts: ["chatgpt.com"], client: "chatgpt" }),
    );
    expect(hasBrowserSurface(claudeWeb)).toBe(true);
    expect(hasBrowserSurface(chatgptApps)).toBe(true);
  });

  it("is false for the subscription row, which is two programs and no tab", () => {
    // The row this table exists for. `chatgpt` is `Client::ChatGpt`,
    // `Credential::Additive` and `Scope::Host` - identical to `chatgpt-apps` on
    // every field the ledger carries - and what actually talks to it is Codex
    // through the relay and the ChatGPT app's Work mode. Told to reload a tab,
    // its user has nothing to reload.
    const [subscription] = members(
      sessionDomain({ slug: "chatgpt", display_name: "Subscription", hosts: ["chatgpt.com"] }),
    );
    expect(subscription.credential).toBe("additive");
    expect(hasBrowserSurface(subscription)).toBe(false);
  });

  it("is false for a brokered host row, which is an API nobody browses", () => {
    const [api] = members(domain());
    expect(hasBrowserSurface(api)).toBe(false);
  });
});

describe("hostReloadAdvice", () => {
  const cascade = (...domains: ProxyDomain[]) =>
    buildGroups([], domains, ON).flatMap((g) => g.members);

  it("names the host, because that is the only part the user can recognise", () => {
    const advice = hostReloadAdvice(cascade(sessionDomain()));
    expect(advice?.body).toContain("claude.ai");
  });

  it("says what is wrong with the open page, not just that it is open", () => {
    // The whole reason the notice exists: the page is not merely stale, it is
    // bypassing Gate, and a sentence that only said "reload" would read as
    // housekeeping.
    const advice = hostReloadAdvice(cascade(sessionDomain()));
    expect(advice?.body).toContain("go around Gate");
  });

  it("says nothing when what moved has no browser surface", () => {
    // A section of config rows, and the subscription row on its own. Both would
    // otherwise get a notice telling them to reload a page that does not exist.
    expect(hostReloadAdvice(cascade(domain()))).toBeUndefined();
    expect(
      hostReloadAdvice(
        cascade(sessionDomain({ slug: "chatgpt", hosts: ["chatgpt.com"] })),
      ),
    ).toBeUndefined();
  });

  it("names one host once when two surfaces share it", () => {
    // `chatgpt-apps` and `chatgpt` are both chatgpt.com, and a section switch
    // moves them together. "chatgpt.com, chatgpt.com" reads as a bug in the
    // sentence rather than as two surfaces.
    const advice = hostReloadAdvice(
      cascade(
        sessionDomain({ slug: "chatgpt-apps", hosts: ["chatgpt.com"], client: "chatgpt" }),
        sessionDomain({ slug: "chatgpt", hosts: ["chatgpt.com"], client: "chatgpt" }),
      ),
    );
    expect(advice?.body.match(/chatgpt\.com/g)).toHaveLength(1);
  });
});

/**
 * The certificate's own restart note, which is a different fact from the proxy
 * advice above and has to stay one: that one is about a pointer a browser on the
 * desktop's settings re-reads live, this one about a trust store nothing re-reads
 * at all.
 */
describe("browserTrustRestartAdvice", () => {
  it("is Linux-only, because that is where the trust store is read once", () => {
    // Chromium reads `~/.pki/nssdb` and Firefox the system bundle through
    // p11-kit, both at process start (`ca_linux.rs`). macOS and Windows
    // re-evaluate trust for a running process, so naming browsers there would
    // be caution rather than mechanism.
    expect(browserTrustRestartAdvice("linux", "trusted")).toBeDefined();
    expect(browserTrustRestartAdvice("macos", null)).toBeUndefined();
    expect(browserTrustRestartAdvice("windows", null)).toBeUndefined();
    expect(browserTrustRestartAdvice("unknown", null)).toBeUndefined();
  });

  it("names the vault the platform's own way", () => {
    // Through `trustStoreName`, like every other string that says where
    // something of the user's lives. "keyring" would be the wrong vault and
    // "keychain" the wrong platform.
    expect(browserTrustRestartAdvice("linux", "trusted")?.body).toContain(
      "certificate store",
    );
  });

  it("asks for a full quit, not a new window", () => {
    // Reopening a window in a process that is still running changes nothing:
    // the NSS read happened at startup. A browser told to "reload" would fail
    // exactly as before and read as Gate being broken.
    expect(browserTrustRestartAdvice("linux", "trusted")?.body).toContain(
      "quit",
    );
  });

  it("names the package only where a package is the fix", () => {
    // `tools_missing` is the one state an install ends. Prescribing it for
    // `write_failed` - a locked or unwritable database - sends someone to
    // install a package they already have, at the one moment they are reading
    // closely. `CertutilFailure` draws this line in the log messages already.
    const missing = browserTrustRestartAdvice("linux", "tools_missing");
    expect(missing?.body).toContain("libnss3-tools");
    expect(missing?.body).toContain("nss-tools");
    expect(
      browserTrustRestartAdvice("linux", "write_failed")?.body,
    ).not.toContain("libnss3-tools");
  });

  it("keeps the reopen where a store did take the certificate", () => {
    // `write_failed` is per-store: the reading is "at least one refused", so
    // the browsers whose store took it are working and still need the quit.
    // Withholding that sentence left them with no true instruction at all.
    expect(browserTrustRestartAdvice("linux", "write_failed")?.body).toContain(
      "quit and reopen",
    );
  });

  it("asks for something that actually re-runs the write", () => {
    // Not "switch routing on again": the note is raised from inside the enable
    // that just trusted the CA, so routing is already on, and
    // `manager_linux::enable` returns early before `ca::ensure_trusted` when
    // the client is connected. Off and on again is what reaches the write.
    //
    // Unconditional, for all three failure readings. Guarding the second
    // assertion on the body mentioning "routing" made this vacuous for
    // `write_failed`, whose copy did not - and what the guard was hiding was
    // that `write_failed` prescribed no retry at all: it named the store, and
    // then left the reader with nothing to do once they had unlocked it.
    for (const nss of ["tools_missing", "write_failed", "not_written"] as const) {
      const body = browserTrustRestartAdvice("linux", nss)?.body ?? "";
      expect(body).not.toContain("switch routing on again");
      expect(body).toContain("turn routing off and on again");
    }
  });

  it("sends nobody to the report when no store refused", () => {
    // `not_written` is the reading `--system-trust` leaves: the stores
    // answered and nobody wrote them. `write_failed`'s sentence promises the
    // report names which store and why, and here there is no refusal in it -
    // so a shared sentence would send the reader to a report with nothing in
    // it, at the moment they are already stuck.
    const advice = browserTrustRestartAdvice("linux", "not_written");
    expect(advice?.body).not.toContain("diagnostics report");
    expect(advice?.body).not.toContain("libnss3-tools");
    expect(advice?.body).toContain("turn routing off and on again");
  });

  it("counts the stores the way the report does", () => {
    // The refusals are a list and the report prints one line per entry, so a
    // title saying "one store" undercounts what the body and the report both
    // say on a machine with two locked databases.
    const advice = browserTrustRestartAdvice("linux", "write_failed");
    expect(advice?.title).not.toContain("One browser");
    expect(advice?.body).toContain("at least one");
  });

  it("says who is unaffected, because most of the machine is", () => {
    // The system anchor still serves Firefox and `NODE_EXTRA_CA_CERTS` still
    // serves the Node CLIs, so this is one family of browsers rather than
    // routing being broken - and a note that did not say so would read as the
    // latter.
    for (const nss of ["tools_missing", "write_failed"] as const) {
      expect(browserTrustRestartAdvice("linux", nss)?.body).toContain(
        "Firefox and command-line tools are unaffected",
      );
    }
  });

  it("falls back to the reopen note when there is no reading", () => {
    // `null` is not a negative reading: it means no browser here keeps such a
    // store, or no write has happened in this process. Either way the restart
    // is still the one thing Gate cannot check. Principle 6 in the small.
    expect(browserTrustRestartAdvice("linux", null)?.title).toBe(
      browserTrustRestartAdvice("linux", "trusted")?.title,
    );
  });
});

/**
 * The chat rows' scope sentence, which the window shell had no room for and
 * therefore did not say at all.
 */
/**
 * The one card a pane draws over its switch, composed from both notes above.
 *
 * Section-shaped fixtures rather than lone members, because what is under test
 * is how a section's surfaces read TOGETHER: the Claude pane is the one with
 * both a signed-in surface and a brokered host, and it is the pane that drew
 * the card twice.
 */
/** `governingMembers`' rule, restated for the fixtures above. */
const governing = (members: GroupMember[]): GroupMember[] => {
  const brokered = members.filter((m) => m.cascade);
  return brokered.length > 0 ? brokered : members;
};
/** `intended`'s rule: asked for, or drifted while asked for. */
const isIntended = (m: GroupMember): boolean => m.desired || m.attention === "drifted";

describe("settings-managed members", () => {
  it("keeps the shell-environment channel out of the app list", () => {
    // AG-893. A machine-wide setting is not an app, and the rail is a list of
    // apps. Its control is in Settings now; the tray reports it.
    expect(isSettingsManaged("env-proxy")).toBe(true);
    expect(isSettingsManaged("opencode")).toBe(false);
  });

  it("must filter the TOOL LIST, not the built ledger", () => {
    // The trap: `buildGroups` gives a member no section claims a section of its
    // own, so filtering afterwards puts the row back under its raw name with
    // none of the section copy. Filtering the input is what removes it.
    const tools = [
      tool("env-proxy", "Terminal tools", { kind: "detected" }, "any-app", {
        scope: "machine",
      }),
      tool("opencode", "OpenCode", { kind: "connected" }, "opencode"),
    ];

    const filtered = buildGroups(
      tools.filter((t) => !isSettingsManaged(t.slug)),
      [],
      { proxyOn: true, caTrusted: true, verdicts: new Map() },
    );
    expect(filtered.map((g) => g.id)).not.toContain("terminal");

    // And the trap itself, so a future refactor that filters afterwards fails.
    const unfiltered = buildGroups(tools, [], {
      proxyOn: true,
      caTrusted: true,
      verdicts: new Map(),
    });
    expect(unfiltered.map((g) => g.id)).toContain("terminal");
  });
});

/**
 * Which group a section draws under.
 *
 * **This describe used to pin the opposite** and the history is worth keeping.
 * AG-897 split the rail into two bands asking different questions - an app the
 * user launches, or a mechanism they opt into - and filed both provider
 * endpoints together under Tools on that reasoning.
 *
 * The Figma sidebar (`440:1593`) groups by vendor instead, and design confirmed
 * it on 2026-09-22, so the file wins per CLAUDE.md. The casualty is AG-897's
 * precedent: `openai` and `openrouter` are both provider endpoints and no
 * longer share a group, because OpenAI has a vendor group to belong to and
 * OpenRouter does not. That is a real loss of a distinction, made knowingly.
 */
describe("isProviderEndpoint", () => {
  // Nothing covered this, so the unknown-id branch its doc argues for was
  // unpinned. Raised in review on #323.
  it("is true only for a destination other programs are pointed at", () => {
    expect(isProviderEndpoint("openrouter")).toBe(true);
    expect(isProviderEndpoint("openai-api")).toBe(true);
  });

  it("is false for an app the user launches", () => {
    expect(isProviderEndpoint("claude")).toBe(false);
    expect(isProviderEndpoint("chatgpt")).toBe(false);
  });

  it("is false for an id no section names", () => {
    // A catalog entry with no home yet. Claiming it is an endpoint would put
    // "any app on this machine can be pointed here" on it, which is the
    // sentence this flag exists to withhold.
    expect(isProviderEndpoint("nope")).toBe(false);
  });
});

describe("which group a section draws under", () => {
  // `any-app`, which is what both provider-endpoint rows are: neither is one
  // program's surface.
  const bandFor = (slug: string) =>
    buildGroups(
      [],
      [domain({ slug, client: "any-app", display_name: slug })],
      ON,
    ).find((g) => g.members.some((m) => m.key === slug))?.band;

  it("files a row with no vendor group under the catch-all", () => {
    // Design's answer on 2026-09-22 was "no OpenRouter group", so the frame's
    // third group is dropped rather than interpreted - it is labelled
    // `OPENCode` and contains OpenRouter, which is a slip no rule settles.
    expect(bandFor("openrouter")).toBe("other");
  });

  it("files the OpenAI endpoint with the rest of OpenAI, not with the endpoints", () => {
    // The AG-897 precedent inverted. Grouping by vendor puts `openai` beside
    // Codex and ChatGPT; grouping by kind put it beside OpenRouter.
    expect(bandFor("openai")).toBe("openai");
    expect(bandFor("openai")).not.toBe(bandFor("openrouter"));
  });

  it("files Anthropic's apps under Anthropic", () => {
    const groups = buildGroups(
      [tool("claude-code", "CLI", { kind: "connected" })],
      [domain(), sessionDomain()],
      ON,
    );
    expect(groups[0].band).toBe("anthropic");
  });

  it("keeps each group's sections contiguous, or the rail draws it twice", () => {
    // Not a style rule. `NewUiApp` and `TrayApp` both emit a group header on
    // every CHANGE of band while walking sections in order, so a band that
    // appears, stops, and appears again gets two headers with the same name
    // and two counters that each count half the rows.
    //
    // Found by doing it: `openai-api` sat last in the array while `chatgpt`
    // sat second, so regrouping by vendor drew "OpenAI" twice and a strict
    // locator matched both. Ordering the array is the whole fix, which is
    // exactly why it needs a test - nothing else makes the constraint visible
    // at the point someone adds a section.
    const bands = SECTIONS.map((section) => section.band);
    const firstSeen = new Map<Band, number>();
    bands.forEach((band, i) => {
      if (!firstSeen.has(band)) firstSeen.set(band, i);
    });
    for (const [band, first] of firstSeen) {
      const last = bands.lastIndexOf(band);
      const run = bands.slice(first, last + 1);
      expect(
        run.every((b) => b === band),
        `sections for "${band}" are interrupted by another group; order SECTIONS so each band is contiguous`,
      ).toBe(true);
    }
  });

  it("labels the three groups the way design named them", () => {
    // "Other apps", which is design's wording, not the frame's "Other tools".
    expect(BAND_LABELS.anthropic).toBe("Anthropic");
    expect(BAND_LABELS.openai).toBe("OpenAI");
    expect(BAND_LABELS.other).toBe("Other apps");
  });
});
