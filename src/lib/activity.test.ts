import { describe, it, expect } from "vitest";
import { adapt } from "./activity";

/**
 * What the Overview does with the gateway's policy rows.
 *
 * The row's action is the gateway's reading of a per-org setting, and the
 * gateway omits it when the policy states no single mode - which is the common
 * case, since the seeded default sets none and the pipeline then acts per entity
 * or per confidence tier. Filling in a default here would print a verb for
 * enforcement nobody configured, on the pane whose job is honest system state.
 */
function overview(
  rows: Array<{ id: string; label: string; action?: string; enabled: boolean }>,
  counters?: Record<string, unknown>,
) {
  return {
    generatedAt: "2026-08-17T12:00:00.000Z",
    window: { from: "2026-08-16T13:00:00.000Z", to: "2026-08-17T12:00:00.000Z" },
    org: { orgId: "org-1", name: "Acme" },
    counters: {
      blockedOrFlagged: { state: "ok" as const, value: 0 },
      needsReview: { state: "ok" as const, value: 0 },
      requestsRouted: { state: "ok" as const, value: 0 },
      tokensSaved: { state: "ok" as const, fraction: 0, amount: 0, currency: "USD" },
      ...counters,
    },
    requestsByHour: { state: "ok" as const, buckets: [] },
    policies: { state: "ok" as const, rows },
    tokenSavings: { state: "ok" as const, rows: [] },
  } as unknown as Parameters<typeof adapt>[0];
}

describe("adapt", () => {
  it("leaves a policy's action null when the gateway sends none", () => {
    const view = adapt(overview([{ id: "prompt-injection", label: "Prompt injection", enabled: true }]));

    expect(view.policies[0]).toMatchObject({ name: "Prompt injection", action: null, enabled: true });
  });

  it("passes through the action the policy does state, including allow", () => {
    const view = adapt(
      overview([
        { id: "pii-phi", label: "PII / PHI", action: "redact", enabled: true },
        // Enabled and watching, but acting on nothing. Not the same as off, and
        // the row's own Status column is what says whether it runs.
        { id: "credentials", label: "Credentials", action: "allow", enabled: true },
      ]),
    );

    expect(view.policies.map((p) => p.action)).toEqual(["redact", "allow"]);
  });

  /**
   * The rule AG-576 states and the tiles enforce: a counter the gateway declined
   * has no value, and null is what the tiles turn into a dash. A zero here would
   * be a claim about the user's traffic made out of a section that failed - and a
   * counter that genuinely answered `0` has to stay a zero, or the honest reading
   * "nothing happened in this window" becomes indistinguishable from a gap.
   */
  it("nulls a declined counter and keeps a real zero", () => {
    const view = adapt(
      overview([], {
        blockedOrFlagged: { state: "unavailable", reason: "attribution" },
        tokensSaved: { state: "unavailable", reason: "attribution" },
      }),
    );

    expect(view.stats.blockedFlagged).toBeNull();
    expect(view.stats.tokensSavedPercent).toBeNull();
    expect(view.stats.tokensSavedAmount).toBeNull();
    // Answered, and the answer was nothing.
    expect(view.stats.messages).toBe(0);
  });

  /**
   * The taxonomy ships with the gateway, not with this build, so a newer gateway
   * can name a cause this app has never heard of. `activityGaps` switches
   * exhaustively over the union with no default branch, so an unrecognised value
   * reaching it would render a banner with no text instead of naming the gap.
   */
  it("narrows an unrecognised reason to connectivity", () => {
    const view = adapt(
      overview([], { requestsRouted: { state: "unavailable", reason: "quota_exhausted" } }),
    );

    expect(view.gaps).toEqual([{ section: "Messages", reason: "connectivity" }]);
  });
});
