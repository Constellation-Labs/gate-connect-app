import { describe, expect, it } from "vitest";
import type { Verdict, VerdictReason } from "./api";
import { NEXT_ACTION_LABEL, verdictStatus, verdictsBySlug } from "./verdict";

function verdict(overrides: Partial<Verdict> = {}): Verdict {
  return {
    slug: "claude-code",
    state: "on",
    reason: null,
    next_action: null,
    ...overrides,
  };
}

describe("verdictStatus", () => {
  it("renders a verified route as the design's Protected phrase", () => {
    expect(verdictStatus(verdict({ state: "on" }))).toEqual({ kind: "protected" });
  });

  it("renders Off with the suffix the design already draws", () => {
    expect(verdictStatus(verdict({ state: "off" }))).toEqual({
      kind: "not-routed",
      detail: "Off",
    });
  });

  it("maps a changed configuration onto Config drifted with no repeated suffix", () => {
    const status = verdictStatus(
      verdict({
        state: "needs_attention",
        reason: "configuration_changed",
        next_action: "apply_gate_configuration",
      }),
    );
    expect(status).toEqual({ kind: "drifted" });
  });

  it.each<[Exclude<VerdictReason, "configuration_changed">, string]>([
    ["reopen_required", "Reopen required"],
    ["connection_problem", "Connection problem"],
    ["access_problem", "Access problem"],
    ["verification_failed", "Verification failed"],
  ])("carries %s into the grey suffix", (reason, detail) => {
    expect(verdictStatus(verdict({ state: "needs_attention", reason }))).toEqual({
      kind: "not-protected",
      detail,
    });
  });

  /**
   * The whole point of AG-562: an unanswered sweep must not fall back to the
   * config-derived line, because a saved configuration is not evidence that
   * anything is routing.
   */
  it("says it is still checking rather than guessing from the config", () => {
    expect(verdictStatus(undefined)).toEqual({ kind: "not-protected", detail: "Checking" });
  });

  it("never reports Protected before a verdict arrives", () => {
    expect(verdictStatus(undefined).kind).not.toBe("protected");
  });
});

describe("verdictsBySlug", () => {
  it("indexes a sweep so a row can look itself up", () => {
    const map = verdictsBySlug([
      verdict({ slug: "claude-code", state: "on" }),
      verdict({ slug: "codex", state: "off" }),
    ]);
    expect(map.get("claude-code")?.state).toBe("on");
    expect(map.get("codex")?.state).toBe("off");
    expect(map.get("opencode")).toBeUndefined();
  });
});

describe("NEXT_ACTION_LABEL", () => {
  /** The labels are AG-562's own words; a missing one would ship a blank button. */
  it("labels every action the backend can send", () => {
    expect(Object.values(NEXT_ACTION_LABEL).every((l) => l.length > 0)).toBe(true);
    expect(NEXT_ACTION_LABEL.apply_gate_configuration).toBe("Apply Gate configuration");
    expect(NEXT_ACTION_LABEL.sign_in).toBe("Sign in");
  });
});
