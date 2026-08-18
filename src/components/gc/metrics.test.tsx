import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { MessagesChart, StatTiles, type MessagesBucket, type UsageStats } from "./metrics";

afterEach(cleanup);

const buckets: MessagesBucket[] = [
  { label: "11", total: 4, blocked: 1, flagged: 0, redacted: 0 },
  { label: "12", total: 8, blocked: 2, flagged: 2, redacted: 0 },
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
 * A declined counter must not print a figure. AG-576's rule, and the reason
 * `UsageStats` is nullable: "0 blocked" is a claim about the user's traffic, and
 * the tile is the one place they would look to check it.
 */
describe("StatTiles", () => {
  const dash = "\u2014";
  const stats: UsageStats = {
    messages: 0,
    blockedFlagged: null,
    tokensSavedPercent: null,
    tokensSavedAmount: null,
  };

  it("dashes a declined counter while keeping a real zero", () => {
    render(<StatTiles stats={stats} />);

    // Messages answered zero, so it reads zero. The other two never answered.
    expect(screen.getByText("Messages").parentElement?.textContent).toContain("0");
    expect(screen.getByText("Blocked/Flagged").parentElement?.textContent).toContain(dash);
    expect(screen.getByText("Tokens saved").parentElement?.textContent).toContain(dash);
    expect(screen.queryByText("0%")).toBeNull();
  });

  it("dashes every counter while the first load is in flight", () => {
    render(<StatTiles stats={{ ...stats, messages: 12 }} pending />);

    expect(screen.queryByText("12")).toBeNull();
  });
});
