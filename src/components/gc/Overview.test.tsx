import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { Overview } from "./Overview";
import type { UsageStats } from "./metrics";
import type { SecurityEventsProps } from "./SecurityEvents";

afterEach(cleanup);

const stats: UsageStats = {
  messages: 0,
  blockedFlagged: 0,
  tokensSavedPercent: 0,
};

/** A feed that has answered and had nothing to report, which is the state that
 *  says least about the rest of the pane. */
const quietFeed: SecurityEventsProps = {
  events: [],
  state: "live",
  loading: false,
  unavailable: false,
  onRetry: () => {},
  onOpenEvent: () => {},
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
      security={quietFeed}
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

/**
 * AG-853: the feed is a section of this pane now, and it is the last one.
 *
 * Order is the requirement, not just presence - the summaries above are the
 * period's totals and the feed is the period's detail, so a feed that drifted
 * above Token savings would read as the headline rather than the evidence.
 */
describe("the Security events section", () => {
  it("draws the feed after Token savings", () => {
    render(pane());

    const headings = screen
      .getAllByRole("heading", { level: 2 })
      .map((h) => h.textContent);
    expect(headings).toEqual([
      "Messages",
      "Policies",
      "Token savings",
      "Security events",
    ]);
  });

  it("keeps the feed's own states clear of the activity read's", () => {
    // `pending` is the 24-hour read, which the feed knows nothing about: a pane
    // still loading its counters must not claim the feed is loading too, nor
    // the other way round.
    render(pane({ pending: true }));

    expect(screen.getByText("No security events")).toBeTruthy();
    expect(screen.queryByText("Loading security events")).toBeNull();
  });
});

describe("the Tokens saved tile", () => {
  const saving = { id: "s1", name: "Prompt compression", icon: "layers" as const, enabled: true };

  it("is a button that jumps when the section it jumps to has rows", () => {
    // AG-572's decision, and the half of it that works: there is something to
    // land on.
    render(pane({ savings: [saving] }));

    expect(screen.getByRole("button", { name: /Tokens saved/i })).toBeTruthy();
  });

  it("is an ordinary tile when the section below is empty", () => {
    // AG-883. Token savings is the second-to-last card, so the pane pins at its
    // maximum scroll rather than putting the heading at the top - and with the
    // table and the feed both empty, the click reads as the page jumping to a
    // screen of nothing. An offer to navigate somewhere blank is worse than no
    // offer, so the tile stops being a button.
    render(pane({ savings: [] }));

    expect(screen.queryByRole("button", { name: /Tokens saved/i })).toBeNull();
  });

  it("does not offer the jump while the read is in flight or failed", () => {
    // The section draws placeholders in one case and a sentence in the other.
    // Neither is a destination.
    const { unmount } = render(pane({ savings: [saving], pending: true }));
    expect(screen.queryByRole("button", { name: /Tokens saved/i })).toBeNull();
    unmount();

    render(pane({ savings: [saving], unavailable: { savings: true } }));
    expect(screen.queryByRole("button", { name: /Tokens saved/i })).toBeNull();
  });
});
