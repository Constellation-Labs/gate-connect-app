import { describe, expect, it } from "vitest";
import type { RecoverySummary, RecoveryTool } from "./api";
import {
  ago,
  operationLine,
  recoveryRow,
  recoveryRows,
  stageCounts,
  unresolved,
} from "./recovery";

const NOW = new Date("2026-09-04T12:00:00Z");
const NOW_UNIX = Math.floor(NOW.getTime() / 1000);

function toolRow(overrides: Partial<RecoveryTool> = {}): RecoveryTool {
  return {
    slug: "claude-code",
    name: "Claude Code",
    kind: "tool",
    stage: "restored",
    stage_complete: true,
    error_category: "none",
    error: null,
    stage_at_unix: NOW_UNIX - 240,
    last_verified_state: "on",
    last_verified_unix: NOW_UNIX - 600,
    check_state: "on",
    check_reason: null,
    check_at_unix: NOW_UNIX - 60,
    running: false,
    reopen_pending: false,
    next_step: "none",
    ...overrides,
  };
}

function summary(overrides: Partial<RecoverySummary> = {}): RecoverySummary {
  return {
    operation: "restore",
    updated_unix: NOW_UNIX - 300,
    requested_routing_on: true,
    tools: [toolRow()],
    ...overrides,
  };
}

describe("ago", () => {
  /** 0 is the backend's "the clock would not answer". Rendering it as an age
   *  would date every such reading to 1970. */
  it("renders an unreadable clock as unknown, not as 1970", () => {
    expect(ago(0, NOW)).toBe("unknown");
    expect(ago(-5, NOW)).toBe("unknown");
  });

  it("counts in whole units, largest two first", () => {
    expect(ago(NOW_UNIX - 240, NOW)).toBe("4m ago");
    expect(ago(NOW_UNIX - (2 * 3600 + 46 * 60), NOW)).toBe("2h 46m ago");
    expect(ago(NOW_UNIX - (3 * 86400 + 4 * 3600), NOW)).toBe("3d 4h ago");
  });

  /** A clock that moved backwards between the write and the read. Reads as
   *  recent rather than as a negative age. */
  it("does not render a future reading as a negative age", () => {
    expect(ago(NOW_UNIX + 90, NOW)).toBe("just now");
  });
});

describe("recoveryRow", () => {
  /** The four readings stay four readings. A row that folded them would have to
   *  pick one, and the case below is exactly where picking is wrong. */
  it("keeps a finished write and a stale process as separate statements", () => {
    const row = recoveryRow(
      toolRow({
        stage: "restored",
        stage_complete: true,
        check_state: "needs_attention",
        check_reason: "reopen_required",
        running: true,
        reopen_pending: true,
        next_step: "reopen_tool",
      }),
      NOW,
    );
    expect(row.stageLine).toBe("Configuration written 4m ago");
    expect(row.checkResult).toBe("Needs attention - reopen required (1m ago)");
    expect(row.runningState).toBe("Running, using the settings it started with");
    expect(row.action).toBe("Reopen tool");
  });

  /** The last verified route survives a failed verification, and the row shows
   *  both - the older reading that still stands and the newer one that could
   *  not conclude. */
  it("shows a standing verified route beside a check that could not conclude", () => {
    const row = recoveryRow(
      toolRow({
        last_verified_state: "on",
        last_verified_unix: NOW_UNIX - 3600,
        check_state: "needs_attention",
        check_reason: "verification_failed",
        check_at_unix: NOW_UNIX - 30,
      }),
      NOW,
    );
    expect(row.lastVerified).toBe("Routing through Gate, 1h 0m ago");
    expect(row.checkResult).toContain("could not be verified");
  });

  /** No reading is a real answer, and it is not "off". */
  it("says so when nothing has ever verified the tool", () => {
    const row = recoveryRow(
      toolRow({ last_verified_state: null, last_verified_unix: 0, check_state: null }),
      NOW,
    );
    expect(row.lastVerified).toBeNull();
    expect(row.checkResult).toBe("Never checked");
  });

  /** A seeded entry has no useful timestamp: it was written when the operation
   *  began, and dating it suggests something happened to that tool then. */
  it("does not date a stage that was never attempted", () => {
    const row = recoveryRow(
      toolRow({ stage: "pending", stage_complete: false, next_step: "retry" }),
      NOW,
    );
    expect(row.stageLine).toBe("Not started");
    expect(row.action).toBe("Retry");
  });

  it("names the failure category only for a stage that failed", () => {
    expect(
      recoveryRow(toolRow({ stage: "write_failed", error_category: "write" }), NOW)
        .errorCategory,
    ).toBe("Configuration write");
    expect(recoveryRow(toolRow(), NOW).errorCategory).toBe("");
  });

  it("offers no action for a settled row", () => {
    expect(recoveryRow(toolRow(), NOW).action).toBeNull();
  });
});

describe("what a row is, and what Gate could actually see", () => {
  /** The report that started this: a master-on with the engine still coming up
   *  drew five rows, four of them reading "Write failed / Configuration write /
   *  Gate could not write this tool's config". Nothing had been written. */
  it("says an engine-deferred entry is waiting, and blames nobody", () => {
    const row = recoveryRow(
      toolRow({
        slug: "openclaw",
        name: "OpenClaw",
        stage: "deferred_engine_down",
        stage_complete: false,
        // No category: nothing failed, so nothing belongs under a failure
        // heading.
        error_category: "none",
        next_step: "retry",
      }),
      NOW,
    );
    expect(row.stage).toBe("Waiting for routing");
    expect(row.errorCategory).toBe("");
    expect(row.stageDetail).toContain("Nothing was attempted");
    // A resume is still the move, so the control stays.
    expect(row.action).toBe("Retry");
  });

  /** A provider is not a tool, and OpenRouter has no config file at all -
   *  `tool_ids` is empty, it routes entirely through a proxy domain. Telling its
   *  row that Gate could not write "this tool's config" described a file that
   *  does not exist. `kind` was on the DTO the whole time and nothing read it. */
  it("describes a provider's routing rather than a config file it may not have", () => {
    const provider = recoveryRow(
      toolRow({
        slug: "openrouter",
        name: "OpenRouter",
        kind: "provider",
        stage: "write_failed",
        stage_complete: false,
        error_category: "write",
      }),
      NOW,
    );
    expect(provider.stageDetail).toContain("this provider's routing");
    expect(provider.stageDetail).not.toContain("config");

    const tool = recoveryRow(
      toolRow({ stage: "write_failed", stage_complete: false, error_category: "write" }),
      NOW,
    );
    expect(tool.stageDetail).toContain("this tool's config");
  });

  /** The sweep walks the registry, so a provider slug can never have a verdict
   *  logged against it. "Never checked" reads as a gap somebody could close,
   *  and invites the reader to go and check a thing that is checked through its
   *  members. Confirmed by a real `verdict-log.json`, which holds the six tool
   *  slugs and none of the three provider ones. */
  it("does not tell a provider row it has never been checked", () => {
    const provider = recoveryRow(
      toolRow({ slug: "anthropic", name: "Anthropic", kind: "provider", check_state: null }),
      NOW,
    );
    expect(provider.checkResult).toBe("Checked per tool, not per provider");
    // A tool with no check really has never been checked.
    expect(recoveryRow(toolRow({ check_state: null }), NOW).checkResult).toBe(
      "Never checked",
    );
  });

  /** Three answers, not two. `AGENT_PROCESSES` has no name for a provider slug,
   *  for OpenClaw or Hermes, or for `env-proxy` - which is not a process - so
   *  "Not running" was asserting a walk of the process table that never ran.
   *  The app's own rule: a figure is a measurement, or the card says it is not. */
  it("does not claim a process is absent when it never looked for one", () => {
    expect(recoveryRow(toolRow({ running: null }), NOW).runningState).toBe(
      "Gate has no process to look for",
    );
    expect(recoveryRow(toolRow({ running: false }), NOW).runningState).toBe("Not running");
    expect(recoveryRow(toolRow({ running: true }), NOW).runningState).toBe(
      "Running, with current settings",
    );
    // A stale process still outranks both: it is the more specific reading.
    expect(
      recoveryRow(toolRow({ running: true, reopen_pending: true }), NOW).runningState,
    ).toBe("Running, using the settings it started with");
  });

  /** The category says which step; only this says what happened. Before the
   *  journal carried it, the message went to stderr - where nobody reading this
   *  dialog will find it. */
  it("carries the backend's own words for a failure, and nothing otherwise", () => {
    expect(
      recoveryRow(
        toolRow({
          stage: "write_failed",
          error_category: "write",
          error: "the Gate proxy is not running",
        }),
        NOW,
      ).error,
    ).toBe("the Gate proxy is not running");
    expect(recoveryRow(toolRow(), NOW).error).toBeNull();
  });

  /** An engine deferral is unfinished, so the notice stays up and the resume
   *  still has something to do. */
  it("counts an engine deferral as outstanding", () => {
    const s = summary({
      tools: [
        toolRow({ stage: "restored", stage_complete: true, next_step: "none" }),
        toolRow({
          slug: "env-proxy",
          stage: "deferred_engine_down",
          stage_complete: false,
          error_category: "none",
          next_step: "retry",
        }),
      ],
    });
    expect(stageCounts(s)).toEqual({ complete: 1, pending: 1, total: 2 });
    expect(unresolved(s).map((t) => t.slug)).toEqual(["env-proxy"]);
  });
});

describe("the summary's own header", () => {
  it("names the operation, when it was touched, and what it wanted", () => {
    expect(operationLine(summary(), NOW)).toBe(
      "Turning routing back on, last updated 5m ago. It was trying to leave routing on for every tool it had recorded.",
    );
  });

  /** An unreadable clock drops the clause rather than printing "unknown ago". */
  it("drops the timing clause when the clock said nothing", () => {
    expect(operationLine(summary({ updated_unix: 0 }), NOW)).toBe(
      "Turning routing back on. It was trying to leave routing on for every tool it had recorded.",
    );
  });

  /** Dropped entries are finished, so they count as complete. Reporting them as
   *  pending would ask for action nobody can take. */
  it("counts dropped stages as complete", () => {
    const counts = stageCounts(
      summary({
        tools: [
          toolRow({ stage: "restored", stage_complete: true }),
          toolRow({ slug: "codex", stage: "not_installed", stage_complete: true }),
          toolRow({ slug: "opencode", stage: "pending", stage_complete: false }),
        ],
      }),
    );
    expect(counts).toEqual({ complete: 2, pending: 1, total: 3 });
  });
});

describe("unresolved", () => {
  /** Derived from the offered step, not from the stage: a tool whose write
   *  finished but whose process is stale is not routing, and the notice is the
   *  only thing saying so. */
  it("includes a finished write whose process still has to be reopened", () => {
    const rows = unresolved(
      summary({
        tools: [
          toolRow({ stage: "restored", stage_complete: true, next_step: "reopen_tool" }),
          toolRow({ slug: "codex", next_step: "none" }),
        ],
      }),
    );
    expect(rows.map((t) => t.slug)).toEqual(["claude-code"]);
  });

  it("is empty once every tool is settled", () => {
    expect(unresolved(summary())).toEqual([]);
  });
});

describe("recoveryRows", () => {
  /** One clock for the whole summary, so two rows cannot disagree about now. */
  it("renders every tool the operation touched, in the given order", () => {
    const rows = recoveryRows(
      summary({
        tools: [toolRow({ slug: "a", name: "A" }), toolRow({ slug: "b", name: "B" })],
      }),
      NOW,
    );
    expect(rows.map((r) => r.name)).toEqual(["A", "B"]);
  });
});
