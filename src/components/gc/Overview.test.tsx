import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { Overview } from "./Overview";
import type { UsageStats } from "./metrics";

afterEach(cleanup);

const stats: UsageStats = {
  messages: 0,
  blockedFlagged: 0,
  tokensSavedPercent: 0,
  tokensSavedAmount: "+$0.00",
};

function pane(props: Partial<Parameters<typeof Overview>[0]> = {}) {
  return (
    <Overview
      stats={stats}
      buckets={[]}
      policies={[]}
      savings={[]}
      onManagePolicies={() => {}}
      onManageSavings={() => {}}
      {...props}
    />
  );
}

/**
 * An empty table has two possible causes and they are not interchangeable: an
 * org that configured no guardrails, and a list the gateway would not hand over.
 * Reporting the second as the first tells the user their protection is off.
 */
describe("Overview tables", () => {
  it("reads an empty list as configured-nothing when the section answered", () => {
    render(pane());

    expect(screen.getByText("No policies configured")).toBeTruthy();
    expect(screen.getByText("No savings configured")).toBeTruthy();
  });

  it("admits it was not told when the section declined", () => {
    render(pane({ unavailable: { policies: true, savings: true } }));

    expect(screen.getByText("Policies couldn't be read")).toBeTruthy();
    expect(screen.getByText("Token savings couldn't be read")).toBeTruthy();
    expect(screen.queryByText("No policies configured")).toBeNull();
  });

  it("claims neither while the first load is in flight", () => {
    render(pane({ pending: true }));

    expect(screen.queryByText("No policies configured")).toBeNull();
    expect(screen.queryByText("Policies couldn't be read")).toBeNull();
    expect(screen.getByText("Loading your activity")).toBeTruthy();
  });
});
