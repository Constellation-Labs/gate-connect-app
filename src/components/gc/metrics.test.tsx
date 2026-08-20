import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { MessagesChart, StatTiles, type MessagesBucket, type UsageStats } from "./metrics";

afterEach(cleanup);

const buckets: MessagesBucket[] = [
  { id: "2026-08-19T11:00:00.000Z", label: "11", total: 4, blocked: 1, flagged: 0, redacted: 0 },
  { id: "2026-08-19T12:00:00.000Z", label: "12", total: 8, blocked: 2, flagged: 2, redacted: 0 },
];

/** The bars, which are `aria-hidden`, so they are not reachable by role. */
function columns(container: HTMLElement) {
  return Array.from(container.querySelectorAll("[aria-hidden] > div.w-5"));
}

/** The legend and the accessible table both use the tooltip's row labels, so
 *  presence proves nothing - only the count moves when the tooltip opens. */
const BASELINE = 2;

describe("MessagesChart tooltip", () => {
  it("stays hidden until a column is hovered", () => {
    const { container } = render(<MessagesChart buckets={buckets} />);
    expect(screen.getAllByText("Total messages")).toHaveLength(BASELINE);

    fireEvent.mouseEnter(columns(container)[1]);
    expect(screen.getAllByText("Total messages")).toHaveLength(BASELINE + 1);
  });

  it("reports the hovered bucket's own figures, not the stack total", () => {
    const { container } = render(<MessagesChart buckets={buckets} />);
    fireEvent.mouseEnter(columns(container)[1]);

    // The heading names the column; it is not a fifth figure. 8/2/2/0 sum to
    // 12 here only by coincidence of the design's own sample numbers, which is
    // exactly the confusion this asserts against.
    const tip = screen.getByText("Total messages", {
      selector: "div > span > span",
    }).closest("div[class*='absolute']") as HTMLElement;
    expect(within(tip).getByText("12")).toBeTruthy();
    expect(within(tip).getByText("8")).toBeTruthy();
    expect(within(tip).getAllByText("2")).toHaveLength(2);
    expect(within(tip).getByText("0")).toBeTruthy();
  });

  it("clears when the pointer leaves the plot area", () => {
    const { container } = render(<MessagesChart buckets={buckets} />);
    const plot = container.querySelector("[aria-hidden]") as HTMLElement;
    fireEvent.mouseEnter(columns(container)[0]);
    fireEvent.mouseLeave(plot);
    expect(screen.getAllByText("Total messages")).toHaveLength(BASELINE);
  });
});

describe("MessagesChart accessible table", () => {
  it("separates the remainder series from the stack total", () => {
    render(<MessagesChart buckets={buckets} />);
    const row = screen.getByRole("row", { name: /^12:00/ });
    const cells = within(row).getAllByRole("cell").map((c) => c.textContent);
    // total, blocked, flagged, redacted, then the sum.
    expect(cells).toEqual(["8", "2", "2", "0", "12"]);
  });
});

/**
 * The rule the tiles enforce: a measured zero prints `0`, and a counter with no
 * reading prints `N/A`. Figma 228:89333 draws `0` / `0` / `N/A` because it is an
 * org with no traffic, not because the first two can never say `N/A` - and a
 * screen where nothing was read says it three times.
 */
describe("StatTiles", () => {
  // The design's word for it, not an em dash.
  const NA = "N/A";
  const stats: UsageStats = {
    messages: 0,
    blockedFlagged: null,
    tokensSavedPercent: null,
    tokensSavedAmount: null,
  };

  it("keeps a measured zero a zero and marks an unread counter N/A", () => {
    render(<StatTiles stats={stats} />);

    // Messages answered zero, so it reads zero. The other two never answered.
    expect(screen.getByText("Messages").parentElement?.textContent).toContain("0");
    expect(screen.getByText("Blocked/Flagged").parentElement?.textContent).toContain(NA);
    expect(screen.getByText("Tokens saved").parentElement?.textContent).toContain(NA);
    // Never a fabricated percentage.
    expect(screen.queryByText("0%")).toBeNull();
  });

  /** The screen behind the product call of 2026-08-19: a refused credential, so
   *  nothing was read, so no tile claims anything about the user's traffic. */
  it("says N/A three times when nothing was read at all", () => {
    render(
      <StatTiles
        stats={{
          messages: null,
          blockedFlagged: null,
          tokensSavedPercent: null,
          tokensSavedAmount: null,
        }}
      />,
    );

    expect(screen.getAllByText(NA)).toHaveLength(3);
    expect(screen.queryByText("0")).toBeNull();
  });

  it("prints a real count rather than the fallback", () => {
    render(<StatTiles stats={{ ...stats, messages: 1204, blockedFlagged: 7 }} />);

    expect(screen.getByText("1,204")).toBeTruthy();
    expect(screen.getByText("7")).toBeTruthy();
  });

  it("prints no counter at all while the first load is in flight", () => {
    render(<StatTiles stats={{ ...stats, messages: 12 }} pending />);

    expect(screen.queryByText("12")).toBeNull();
  });
});

/**
 * The chart has three answers and they are three different sentences: the
 * series is coming, the series says nothing was sent, and nobody would tell us.
 * Only the middle one is a statement about the user's traffic.
 */
describe("MessagesChart empty and pending states", () => {
  const quiet: MessagesBucket[] = [
    { id: "2026-08-19T11:00:00.000Z", label: "11", total: 0, blocked: 0, flagged: 0, redacted: 0 },
    { id: "2026-08-19T12:00:00.000Z", label: "12", total: 0, blocked: 0, flagged: 0, redacted: 0 },
  ];
  const EMPTY = "No messages sent in the last 24hrs";

  it("says nothing was sent when every bucket really is zero", () => {
    render(<MessagesChart buckets={quiet} />);

    expect(screen.getByText(EMPTY)).toBeTruthy();
  });

  // What ships today: the endpoint has answered, and it answered with no
  // buckets at all. That is still a reading of zero traffic, not a refusal.
  it("says nothing was sent when the series came back with no buckets", () => {
    render(<MessagesChart buckets={[]} />);

    expect(screen.getByText(EMPTY)).toBeTruthy();
  });

  it("stays silent about traffic when the series was never read", () => {
    render(<MessagesChart buckets={[]} unavailable />);

    expect(screen.queryByText(EMPTY)).toBeNull();
  });

  it("draws placeholder columns rather than an empty plot while loading", () => {
    const { container } = render(<MessagesChart buckets={quiet} pending />);

    expect(screen.queryByText(EMPTY)).toBeNull();
    expect(columns(container)).toHaveLength(0);
    expect(container.querySelectorAll(".animate-pulse").length).toBeGreaterThan(0);
  });

  it("plots the series once it lands", () => {
    const { container } = render(<MessagesChart buckets={buckets} />);

    expect(screen.queryByText(EMPTY)).toBeNull();
    expect(columns(container).length).toBeGreaterThan(0);
  });
});
