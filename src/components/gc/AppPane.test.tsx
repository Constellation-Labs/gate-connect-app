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
  time: "Jun 6, 00:50:51",
  status: "success",
  security: "flagged",
  model: "claude-opus-4",
  reference: "cnv_824bd2c0",
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

    expect(within(feed).queryByText("No recent messages")).toBeNull();
    expect(within(feed).queryByText("Recent activity couldn't be read")).toBeNull();
    expect(feed.querySelectorAll(".animate-pulse").length).toBeGreaterThan(0);
  });

  it("admits it was not told when the feed was never read", () => {
    render(pane({ unavailable: { events: true } }));
    const feed = card("Recent activity");

    expect(within(feed).getByText("Recent activity couldn't be read")).toBeTruthy();
    expect(within(feed).queryByText("No recent messages")).toBeNull();
  });

  it("reads a genuinely empty feed as nothing recent", () => {
    render(pane());

    // Distinct from the chart's sentence on purpose: the feed keeps older rows,
    // so it must not claim a 24-hour window.
    expect(within(card("Recent activity")).getByText("No recent messages")).toBeTruthy();
  });

  it("renders a row's model and reference, and no conversation title", () => {
    render(pane({ activity: [entry] }));

    expect(screen.getByText("claude-opus-4")).toBeTruthy();
    expect(screen.getByText("cnv_824bd2c0")).toBeTruthy();
    // Model and Message are separate columns now, per `table/recent-activity`.
    // The Message cell carries only the reference: a title could only have come
    // from prompt text.
    expect(screen.getByRole("columnheader", { name: "Model" })).toBeTruthy();
    expect(screen.getByRole("columnheader", { name: "Message" })).toBeTruthy();
    expect(screen.queryByRole("columnheader", { name: "Conversation" })).toBeNull();
  });

  it("wears one pill per row, and the recorded verdict outranks the error", () => {
    render(
      pane({
        activity: [
          { ...entry, id: "req-flagged", status: "error", security: "flagged" },
          { ...entry, id: "req-error", status: "error", security: null },
        ],
      }),
    );
    const feed = card("Recent activity");

    // The design merged the old Status column into Security: no SUCCESS pill
    // exists, and ERROR appears only when the gateway recorded no action.
    expect(within(feed).queryByRole("columnheader", { name: "Status" })).toBeNull();
    expect(within(feed).getByText("flagged")).toBeTruthy();
    expect(within(feed).getByText("error")).toBeTruthy();
    expect(within(feed).queryByText("success")).toBeNull();
  });

  it("marks a row whose security detail is absent, without inventing a verdict", () => {
    render(pane({ activity: [{ ...entry, security: null }] }));
    const feed = card("Recent activity");

    // Absence of a verdict is not enough: a cell that rendered nothing at all
    // would satisfy that, and the point is that the row says so. `status` is
    // "success" here, so no ERROR pill stands in either.
    const cell = within(feed).getByTitle("No security action recorded, or not your request");
    expect(cell.textContent).toBe("\u2014");
    expect(within(feed).queryByText("allow")).toBeNull();
    expect(within(feed).queryByText("flagged")).toBeNull();
    expect(within(feed).queryByText("error")).toBeNull();
  });

  it("draws a View per row, and hands the entry to whoever owns a destination", () => {
    const onViewEntry = vi.fn();
    render(pane({ activity: [entry], onViewEntry }));

    // Drawn ahead of its wiring by decision (2026-08-21): the dashboard cannot
    // filter by request yet, so the shell passes no handler in production.
    const view = within(card("Recent activity")).getByRole("button", { name: "View" });
    view.click();
    expect(onViewEntry).toHaveBeenCalledWith(entry);
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
    expect(within(card("Recent activity")).getByText("No recent messages")).toBeTruthy();
  });
});
