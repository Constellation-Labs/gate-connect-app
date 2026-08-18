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
function overview(rows: Array<{ id: string; label: string; action?: string; enabled: boolean }>) {
  return {
    generatedAt: "2026-08-17T12:00:00.000Z",
    window: { from: "2026-08-16T13:00:00.000Z", to: "2026-08-17T12:00:00.000Z" },
    org: { orgId: "org-1", name: "Acme" },
    counters: {
      blockedOrFlagged: { state: "ok" as const, value: 0 },
      needsReview: { state: "ok" as const, value: 0 },
      requestsRouted: { state: "ok" as const, value: 0 },
      tokensSaved: { state: "ok" as const, fraction: 0, amount: 0, currency: "USD" },
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
});
