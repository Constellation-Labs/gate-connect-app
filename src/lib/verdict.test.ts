import { describe, expect, it } from "vitest";
import type { UpstreamCoverage, Verdict, VerdictReason } from "./api";
import { countsAsRouted } from "../components/gc/Sidebar";
import { NEXT_ACTION_LABEL, verdictStatus, verdictsBySlug } from "./verdict";

// `sectionStatus` is tested in `groups.test.ts`, against a real `buildGroups`
// ledger: what it answers is a question about a section's members, and this
// file has no fixtures for those.


function verdict(overrides: Partial<Verdict> = {}): Verdict {
  return {
    slug: "claude-code",
    state: "on",
    reason: null,
    next_action: null,
    route_in_use: null,
    requested_route: null,
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

  it("maps a changed configuration onto Not protected, with drift as the reason", () => {
    const status = verdictStatus(
      verdict({
        state: "needs_attention",
        reason: "configuration_changed",
        next_action: "apply_gate_configuration",
      }),
    );
    expect(status).toEqual({ kind: "not-protected", detail: "Config drifted" });
  });

  it.each<[Exclude<VerdictReason, "configuration_changed" | "reopen_required">, string]>([
    ["configuration_overridden", "Configuration overridden"],
    ["connection_problem", "Connection problem"],
    ["access_problem", "Access problem"],
    ["verification_failed", "Verification failed"],
  ])("carries %s into the grey suffix", (reason, detail) => {
    expect(verdictStatus(verdict({ state: "needs_attention", reason }))).toEqual({
      kind: "not-protected",
      detail,
    });
  });

  /** The write landed and a process has to restart, so the traffic is still on
   *  the old route: Not protected, with the reopen as the reason. */
  it("reads a pending reopen as Not protected, with the reopen as the reason", () => {
    expect(
      verdictStatus(
        verdict({
          state: "needs_attention",
          reason: "reopen_required",
          next_action: "reopen_tool",
        }),
      ),
    ).toEqual({ kind: "not-protected", detail: "Reopen to finish" });
  });

  /**
   * AG-674 on the status line. An override is not drift: the file Gate wrote is
   * intact, so the row must not read "Config drifted" and send the reader to
   * re-apply a configuration that is already correct.
   */
  it("keeps an override off the Config drifted phrase", () => {
    const status = verdictStatus(
      verdict({
        state: "needs_attention",
        reason: "configuration_overridden",
        next_action: "show_conflicting_config",
      }),
    );
    expect(status).toEqual({ kind: "not-protected", detail: "Configuration overridden" });
    expect(NEXT_ACTION_LABEL.show_conflicting_config).toBe("Show conflicting file");
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

  /**
   * A failed write leaves nothing on disk, so the sweep keeps describing the
   * state from before the click - true, and not what the user needs to know.
   */
  it("a failed write outranks the sweep, which still describes the old state", () => {
    expect(verdictStatus(verdict({ state: "off" }), { writeFailed: true })).toEqual({
      kind: "not-protected",
      detail: "Configuration update failed",
    });
  });

  it("a failed write outranks even a verified On", () => {
    expect(verdictStatus(verdict({ state: "on" }), { writeFailed: true })).toEqual({
      kind: "not-protected",
      detail: "Configuration update failed",
    });
  });

  it("says nothing about a write that did not fail", () => {
    expect(verdictStatus(verdict({ state: "on" }), { writeFailed: false })).toEqual({
      kind: "protected",
    });
  });
});

describe("verdictStatus and upstream coverage (AG-932)", () => {
  /** A switched-off row: one catalog slug, the hosts it claims, and the tools
   *  turning it on would also connect. Keyed by slug since #327, because one
   *  row can claim several hosts. */
  const off = (slug: string, ...hosts: string[]) => ({ slug, hosts, tools: [] });
  const coverage = (over: Partial<UpstreamCoverage> = {}): UpstreamCoverage => ({
    defaulted: false,
    switched_off: [],
    unknown: [],
    ...over,
  });
  const covered = coverage();
  const on = () => verdict({ state: "on" });

  it("says routed-not-inspected when Gate has no entry for the provider", () => {
    // The whole ticket. The sweep says "on" because the tool really is
    // pointed at Gate; what it cannot know is that the provider it points at
    // is one Gate never intercepts, so every request tunnels past unread.
    expect(
      verdictStatus(on(), {
        coverage: coverage({ unknown: ["bedrock-runtime.us-east-1.amazonaws.com"] }),
      }),
    ).toEqual({
      kind: "not-protected",
      detail: "Routed, not inspected: bedrock-runtime.us-east-1.amazonaws.com",
    });
  });

  it("says it for a known provider whose switch is off, too", () => {
    // AG-930's dialog offers to fix this at connect time, and the row still
    // has to say it: the dialog fires once, and the switch can be turned off
    // afterwards from somewhere else. Removing and re-trusting a certificate
    // reset one to off hours later, which is how this was found.
    expect(
      verdictStatus(on(), {
        coverage: coverage({ switched_off: [off("openrouter", "openrouter.ai")] }),
      }),
    ).toEqual({ kind: "not-protected", detail: "Routed, not inspected: openrouter.ai" });
  });

  it("names one host and counts the rest, because the rail is 250px", () => {
    expect(
      verdictStatus(on(), {
        coverage: coverage({
          switched_off: [off("openrouter", "openrouter.ai")],
          unknown: ["api.groq.com", "api.together.xyz"],
        }),
      }),
    ).toEqual({ kind: "not-protected", detail: "Routed, not inspected: openrouter.ai +2" });
  });

  it("stays Protected when coverage is complete, absent, or null", () => {
    // Three shapes reach this: a tool that reports full coverage, and the
    // every-other-tool case where the backend sends nothing at all.
    expect(verdictStatus(on(), { coverage: covered })).toEqual({ kind: "protected" });
    expect(verdictStatus(on(), {})).toEqual({ kind: "protected" });
    expect(verdictStatus(on(), { coverage: null })).toEqual({ kind: "protected" });
  });

  it("does not outrank a state that is already amber", () => {
    // A drifted or unrouted tool has a larger problem than an uninspected
    // provider, and stacking the two would bury it. Only the "on" arm is
    // reinterpreted.
    const uninspected = coverage({ unknown: ["api.groq.com"] });
    expect(
      verdictStatus(verdict({ state: "off" }), { coverage: uninspected }),
    ).toEqual({ kind: "not-routed", detail: "Off" });
    expect(
      verdictStatus(
        verdict({ state: "needs_attention", reason: "configuration_changed" }),
        { coverage: uninspected },
      ),
    ).toEqual({ kind: "not-protected", detail: "Config drifted" });
  });

  it("counts only Protected as routed, so the banner and the row cannot contradict", () => {
    // An uninspected provider reads Not protected on the row, so the topbar
    // must not count it as protected either.
    const status = verdictStatus(on(), {
      coverage: coverage({ unknown: ["bedrock-runtime.us-east-1.amazonaws.com"] }),
    });
    expect(countsAsRouted(status)).toBe(false);
    // And the rest, because four counters read this one predicate.
    expect(countsAsRouted({ kind: "protected" })).toBe(true);
    expect(countsAsRouted({ kind: "not-routed", detail: "Off" })).toBe(false);
    expect(countsAsRouted({ kind: "not-protected", detail: "Config drifted" })).toBe(false);
  });

  it("does not outrank a failed write either", () => {
    // The write-failed guard runs before the sweep is even read, and should:
    // what the user needs to know is that their click did not land.
    expect(
      verdictStatus(on(), {
        writeFailed: true,
        coverage: coverage({ unknown: ["api.groq.com"] }),
      }).kind,
    ).toBe("not-protected");
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
