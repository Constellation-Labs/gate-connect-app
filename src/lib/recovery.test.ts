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
