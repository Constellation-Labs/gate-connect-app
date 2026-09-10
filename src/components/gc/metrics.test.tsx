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
  // Keyed on the stacking direction, not the width. It used to select `.w-5`,
  // which broke the moment the bars took the frame's 32px - and `.w-8` would be
  // worse than brittle: the loading placeholder's skeletons are that width too,
  // so the "no columns while loading" assertion would start matching them.
  return Array.from(container.querySelectorAll("[aria-hidden] > div.flex-col-reverse"));
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
    // exactly the confusion this asserts against - and which the tooltip
    // settles by heading the column "12:00" rather than "12".
    //
    // The axis went the other way on 2026-09-08 (bare "12", no minutes - see
    // `hourTick`), and this is why the tooltip did not follow it: read on its
    // own, over a stack of four figures, "12" is the ambiguity the heading
    // exists to remove.
    const tip = screen.getByText("Total messages", {
      selector: "div > span > span",
    }).closest("div[class*='absolute']") as HTMLElement;
    expect(within(tip).getByText("12:00")).toBeTruthy();
    expect(within(tip).queryByText("12")).toBeNull();
    expect(within(tip).getByText("8")).toBeTruthy();
    expect(within(tip).getAllByText("2")).toHaveLength(2);
    expect(within(tip).getByText("0")).toBeTruthy();
  });

  it("labels the axis with bare hours, no minutes", () => {
    // `116:30705`, the Overview Messages card, draws digits with no `:00`. The
    // component sample `706:9997` draws "00:00" over 12 buckets and is what
    // this printed everywhere until 2026-09-08; at the card's real 24 buckets
    // that ran the labels into each other and off the card edge.
    const { container } = render(<MessagesChart buckets={buckets} />);
    const ticks = Array.from(
      container.querySelectorAll("div.mt-1 > span"),
    ).map((el) => el.textContent);
    expect(ticks).toEqual(["11", "12"]);
  });

  it("gives the bars and the ticks the same geometry, so a bar sits under its label", () => {
    // Reported from a running build: with 20px bars under 32px labels,
    // `justify-between` distributed the leftover space differently in the two
    // rows, so a bar and its tick had different centres - worst on the first
    // and last bucket, where one edge is pinned and the whole difference shows.
    // Asserting the classes rather than layout because jsdom computes no
    // geometry; equal width and gap is the property that makes the centres
    // coincide, for any number of buckets.
    const { container } = render(<MessagesChart buckets={buckets} />);
    const bar = columns(container)[0] as HTMLElement;
    const tick = container.querySelector("div.mt-1 > span") as HTMLElement;
    expect(bar.className).toContain("w-8");
    expect(tick.className).toContain("w-8");
    expect((bar.parentElement as HTMLElement).className).toContain("gap-2");
    expect((tick.parentElement as HTMLElement).className).toContain("gap-2");
  });

  it("names the hour in full in the accessible table's row headers", () => {
    // The other half of the axis change. A row header is announced on its own,
    // with no neighbouring ticks to make "11" read as a time, so the table
    // keeps the full form the axis dropped.
    render(<MessagesChart buckets={buckets} />);
    expect(screen.getByRole("rowheader", { name: "11:00" })).toBeTruthy();
    expect(screen.getByRole("rowheader", { name: "12:00" })).toBeTruthy();
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
 * reading prints `n/a`. Figma 228:89333 draws `0` / `0` / `n/a` because it is an
 * org with no traffic, not because the first two can never say `n/a` - and a
 * screen where nothing was read says it three times.
 */
describe("StatTiles", () => {
  // The design's word for it, in the design's own case, and not an em dash.
  const NA = "n/a";
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
