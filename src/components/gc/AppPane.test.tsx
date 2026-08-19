import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, within } from "@testing-library/react";
import { AppPane } from "./AppPane";
import type { ActivityEntry } from "./AppPane";
import type { UsageStats } from "./metrics";

afterEach(cleanup);

const stats: UsageStats = {
  messages: 0,
  blockedFlagged: 0,
  tokensSavedPercent: 0,
  tokensSavedAmount: "+$0.00",
};

const entry: ActivityEntry = {
  id: "req-1",
  time: "04:14",
  status: "success",
  security: "flagged",
  model: "claude-opus-4",
  reference: "cnv_824bd2c0",
  onView: () => {},
};

/**
 * The card carrying a given heading.
 *
 * Needed because the chart and the feed use the *same* empty sentence, so an
 * unscoped query for it matches twice on a pane with no traffic at all. Worth
 * noting as a design question rather than a test inconvenience: two identical
 * lines stacked one above the other say the same thing twice.
 */
function card(heading: string): HTMLElement {
  const title = screen.getByRole("heading", { name: heading });
  const section = title.closest("section");
  if (!section) throw new Error(`no card around ${heading}`);
  return section as HTMLElement;
}

function pane(props: Partial<Parameters<typeof AppPane>[0]> = {}) {
  return (
    <AppPane
      name="Claude Code"
      isProtected
      onToggleProtected={() => {}}
      stats={stats}
      buckets={[]}
      modelChoice="app"
      onChooseModel={() => {}}
      gateModel={{ vendor: "-", id: "-" }}
      onChangeModel={() => {}}
      credits="-"
      onAddCredits={() => {}}
      activity={[]}
      {...props}
    />
  );
}

/**
 * The three states AG-576 established, now on the per-tool feed. The middle one
 * is the whole point: an unattributed tool is not an idle tool, and this pane is
 * the surface where that mistake would be most convincing.
 */
describe("AppPane recent activity", () => {
  it("draws placeholder rows while the first page is in flight", () => {
    render(pane({ eventsPending: true }));
    const feed = card("Recent activity");

    expect(within(feed).queryByText("No messages sent in the last 24hrs")).toBeNull();
    expect(within(feed).queryByText("Recent activity couldn't be read")).toBeNull();
    expect(feed.querySelectorAll(".animate-pulse").length).toBeGreaterThan(0);
  });

  it("admits it was not told when the feed was never read", () => {
    render(pane({ unavailable: { events: true } }));
    const feed = card("Recent activity");

    expect(within(feed).getByText("Recent activity couldn't be read")).toBeTruthy();
    expect(within(feed).queryByText("No messages sent in the last 24hrs")).toBeNull();
  });

  it("reads a genuinely empty feed as nothing recent", () => {
    render(pane());

    expect(within(card("Recent activity")).getByText("No messages sent in the last 24hrs")).toBeTruthy();
  });

  it("renders a row's model and reference, and no conversation title", () => {
    render(pane({ activity: [entry] }));

    expect(screen.getByText("claude-opus-4")).toBeTruthy();
    expect(screen.getByText("cnv_824bd2c0")).toBeTruthy();
    // The column is Model now. A title could only have come from prompt text.
    expect(screen.getByRole("columnheader", { name: "Model" })).toBeTruthy();
    expect(screen.queryByRole("columnheader", { name: "Conversation" })).toBeNull();
  });

  it("does not render a verdict pill for a row whose security detail was withheld", () => {
    render(pane({ activity: [{ ...entry, security: null }] }));

    // A withheld action must not read as `allow`.
    expect(screen.queryByText("allow")).toBeNull();
    expect(screen.queryByText("flagged")).toBeNull();
  });

  it("offers Load more only when there is another page", () => {
    const onLoadMore = vi.fn();
    render(pane({ activity: [entry], onLoadMore }));
    expect(screen.getByRole("button", { name: "Load more" })).toBeTruthy();

    cleanup();
    render(pane({ activity: [entry] }));
    expect(screen.queryByRole("button", { name: "Load more" })).toBeNull();
  });
});

describe("AppPane counters and chart", () => {
  it("draws skeletons rather than figures before the first reading lands", () => {
    render(pane({ pending: true }));

    expect(screen.getByText("Loading your activity")).toBeTruthy();
    expect(screen.queryByText("0")).toBeNull();
  });

  it("says nothing about traffic when the chart was never read", () => {
    render(pane({ unavailable: { chart: true } }));

    expect(within(card("Messages")).getByText("Messages couldn't be read")).toBeTruthy();
    // The feed is a separate read and still reports its own empty state.
    expect(within(card("Recent activity")).getByText("No messages sent in the last 24hrs")).toBeTruthy();
  });
});
