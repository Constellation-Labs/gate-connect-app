import { describe, expect, it } from "vitest";
import type { RunningAgent, Verdict } from "./api";
import {
  bucketOf,
  isTerminal,
  nextStage,
  REOPEN_STAGE_DETAIL,
  reopenTools,
  type ReopenStage,
  type ReopenTool,
} from "./reopen";

const agent = (over: Partial<RunningAgent> = {}): RunningAgent => ({
  slug: "codex",
  name: "codex",
  product_name: "Codex",
  verifiable: true,
  can_reopen: false,
  pid: 1,
  started_at_unix: 100,
  needs_reopen: true,
  ...over,
});

const verdict = (over: Partial<Verdict> = {}): Verdict => ({
  slug: "codex",
  state: "on",
  reason: null,
  next_action: null,
  route_in_use: null,
  requested_route: null,
  ...over,
});

const tool = (over: Partial<ReopenTool> = {}): ReopenTool => ({
  key: "codex:Codex",
  slug: "codex",
  name: "Codex",
  canReopen: false,
  running: true,
  verifiable: true,
  routeInUse: null,
  requestedRoute: null,
  stage: "awaiting_reopen",
  ...over,
});

describe("reopenTools", () => {
  it("names tools rather than processes, and counts two of one as one", () => {
    const tools = reopenTools(
      [
        agent({ pid: 1 }),
        agent({ pid: 2 }),
        agent({ slug: "claude-code", name: "claude", product_name: "Claude Code" }),
      ],
      new Map([
        ["codex", "Codex"],
        ["claude-code", "Claude Code"],
      ]),
      new Map(),
    );

    // The product name, not the process name: this is a list of tools, and
    // "claude" is what the OS calls a program.
    expect(tools.map((t) => [t.slug, t.name])).toEqual([
      ["codex", "Codex"],
      ["claude-code", "Claude Code"],
    ]);
  });

  it("names a process by what the backend resolved it to, not by its slug", () => {
    // A Code-tab session is `claude-code`, which the registry calls Claude
    // Code; the backend knows it runs inside Claude Desktop.
    const [t] = reopenTools(
      [
        agent({
          slug: "claude-code",
          name: "claude.exe",
          product_name: "Claude Code in Claude Desktop",
        }),
      ],
      new Map([["claude-code", "Claude Code"]]),
      new Map(),
    );
    expect(t.name).toBe("Claude Code in Claude Desktop");
  });

  it("keeps the Code tab and a terminal CLI as two rows of one slug", () => {
    const codeTab = { slug: "claude-code", product_name: "Claude Code in Claude Desktop" };
    const tools = reopenTools(
      [
        agent({ slug: "claude-code", pid: 1, product_name: "Claude Code" }),
        agent({ ...codeTab, pid: 2 }),
        agent({ ...codeTab, pid: 3 }),
      ],
      new Map([["claude-code", "Claude Code"]]),
      new Map(),
    );
    expect(tools.map((t) => [t.slug, t.name])).toEqual([
      ["claude-code", "Claude Code"],
      ["claude-code", "Claude Code in Claude Desktop"],
    ]);
    expect(new Set(tools.map((t) => t.key)).size).toBe(2);
  });

  it("carries both routes, or neither", () => {
    const [both] = reopenTools(
      [agent()],
      new Map(),
      new Map([
        [
          "codex",
          verdict({
            state: "needs_attention",
            reason: "reopen_required",
            route_in_use: "https://api.openai.com",
            requested_route: "https://gate.example/v1",
          }),
        ],
      ]),
    );
    const [neither] = reopenTools([agent()], new Map(), new Map());

    expect(both.routeInUse).toBe("https://api.openai.com");
    expect(neither.routeInUse).toBeNull();
    expect(neither.requestedRoute).toBeNull();
  });

  it("reports what the backend said about reopening, rather than assuming", () => {
    // Nothing in the registry can be relaunched today, and the copy that says
    // who reopens what is built from this rather than written into a sentence.
    const [t] = reopenTools([agent()], new Map(), new Map());
    expect(t.canReopen).toBe(false);
  });
});

describe("nextStage", () => {
  it("keeps a closed tool waiting however healthy its config reads", () => {
    // The sweep answers `on` for a tool that is not running: the config carries
    // Gate's values and the relay answers. Nothing has read that file, so the
    // reopen is what gets verified, never the config on its own.
    expect(nextStage(tool({ stage: "closing" }), verdict(), "gone", 1)).toBe(
      "awaiting_reopen",
    );
  });

  it("verifies a tool that came back", () => {
    expect(nextStage(tool({ stage: "awaiting_reopen" }), verdict(), "fresh", 1)).toBe(
      "routing",
    );
    expect(
      nextStage(tool({ stage: "awaiting_reopen" }), verdict({ state: "off" }), "fresh", 1),
    ).toBe("not_routed");
  });

  it("gives a close a moment before calling it failed", () => {
    // SIGTERM is asynchronous, and a tool flushing state is not a tool refusing
    // to die.
    expect(nextStage(tool({ stage: "closing" }), undefined, "stale", 1)).toBe("closing");
    expect(nextStage(tool({ stage: "closing" }), undefined, "stale", 2)).toBe(
      "close_failed",
    );
  });

  it("stops calling an unanswered check progress", () => {
    const waiting = tool({ stage: "verifying" });
    expect(nextStage(waiting, undefined, "fresh", 1)).toBe("verifying");
    expect(nextStage(waiting, undefined, "fresh", 10)).toBe("verify_failed");
  });

  it("reports drift as a configuration failure and a dead relay as an unproven route", () => {
    const drifted = verdict({ state: "needs_attention", reason: "configuration_changed" });
    const unreachable = verdict({ state: "needs_attention", reason: "connection_problem" });

    expect(nextStage(tool({ stage: "verifying" }), drifted, "fresh", 1)).toBe(
      "config_failed",
    );
    expect(nextStage(tool({ stage: "verifying" }), unreachable, "fresh", 1)).toBe(
      "verify_failed",
    );
  });

  it("leaves a settled row alone", () => {
    // The watch keeps running for the other rows, and a resolved one must not
    // be re-decided under the reader.
    expect(nextStage(tool({ stage: "close_failed" }), verdict(), "fresh", 5)).toBe(
      "close_failed",
    );
  });
});

describe("the account of what happened", () => {
  it("files a tool that is only waiting as waiting, not as a failure", () => {
    expect(bucketOf("awaiting_reopen")).toBe("manual_reopen");
    expect(bucketOf("reopen_required")).toBe("manual_reopen");
  });

  it("has no bucket for a tool still in flight", () => {
    expect(bucketOf("closing")).toBeNull();
    expect(bucketOf("verifying")).toBeNull();
    expect(isTerminal("verifying")).toBe(false);
  });
});

describe("what a stage says", () => {
  it("explains every stage", () => {
    const stages: ReopenStage[] = [
      "applying",
      "reopen_required",
      "closing",
      "awaiting_reopen",
      "reopening",
      "verifying",
      "reopened",
      "routing",
      "not_routed",
      "close_failed",
      "config_failed",
      "verify_failed",
    ];
    for (const stage of stages) {
      expect(REOPEN_STAGE_DETAIL[stage].length).toBeGreaterThan(0);
    }
  });
});

describe("a gone process, and who is putting it back", () => {
  const tool = (canReopen: boolean) => ({
    key: "anthropic:Claude Desktop",
    slug: "anthropic",
    name: "Claude Desktop",
    canReopen,
    running: true,
    // The desktop-app row: `routing_verdicts` walks the registry and this slug
    // is not in it.
    verifiable: false,
    routeInUse: null,
    requestedRoute: null,
    stage: "closing" as const,
  });

  it("tells the user to reopen a tool Gate cannot", () => {
    expect(nextStage(tool(false), undefined, "gone", 0)).toBe("awaiting_reopen");
  });

  it("says Gate is reopening one it can", () => {
    // The row must not read "Closed. Open it again" while Gate is mid-relaunch,
    // or the user starts a second copy of an app that is already starting.
    expect(nextStage(tool(true), undefined, "gone", 0)).toBe("reopening");
  });

  it("hands the move back if the relaunch never takes", () => {
    expect(nextStage(tool(true), undefined, "gone", 99)).toBe("awaiting_reopen");
  });
});

describe("a tool the sweep is never going to answer for", () => {
  /** The desktop-app row: its slug is a proxy-domain key, so `routing_verdicts`
   *  - which walks the registry - has no entry for it whatever it is doing. */
  const app = (over: Partial<ReopenTool> = {}): ReopenTool =>
    tool({
      slug: "anthropic",
      name: "Claude Desktop",
      canReopen: true,
      verifiable: false,
      stage: "reopening",
      ...over,
    });

  it("stops at reopened instead of waiting out a check that does not exist", () => {
    // The bug this replaces: with no verdict coming, the row spun in Verifying
    // for the whole budget and then reported that verification had FAILED - on
    // macOS, for the one row Gate closes and reopens itself, so it was the row
    // the user watched. Nothing failed; there was nothing to check.
    expect(nextStage(app(), undefined, "fresh", 0)).toBe("reopened");
    expect(nextStage(app(), undefined, "fresh", 99)).toBe("reopened");
  });

  it("does not claim to have verified it either", () => {
    // Terminal, so the flow stops - but filed on its own, because a card that
    // said "Applied and verified" over a tool nobody took a reading on would be
    // the one routing claim in this app with nothing behind it.
    expect(isTerminal("reopened")).toBe(true);
    expect(bucketOf("reopened")).toBe("reopened");
  });

  it("still lets Gate close and relaunch it first", () => {
    // Unverifiable is not untouchable: the stages before the check are about
    // the process, which Gate can see perfectly well.
    expect(nextStage(app({ stage: "closing" }), undefined, "stale", 0)).toBe("closing");
    expect(nextStage(app({ stage: "closing" }), undefined, "gone", 0)).toBe("reopening");
  });

  it("names it from the scan when list_tools cannot", () => {
    const [row] = reopenTools(
      [
        agent({
          slug: "anthropic",
          name: "Claude",
          product_name: "Claude Desktop",
          verifiable: false,
        }),
      ],
      // Empty on purpose: `list_tools` has no `anthropic` row to read a name
      // off, which is exactly the case that used to leave surfaces drawing the
      // process name or the raw slug.
      new Map(),
      new Map(),
    );
    expect(row.name).toBe("Claude Desktop");
    expect(row.verifiable).toBe(false);
  });
});
