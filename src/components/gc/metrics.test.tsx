import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { MessagesChart, StatTiles } from "./metrics";
import type { MessagesBucket } from "./metrics";

afterEach(cleanup);

const quiet = (n: number): MessagesBucket[] =>
  Array.from({ length: n }, (_, i) => ({
    label: String(i + 1),
    total: 0,
    blocked: 0,
    flagged: 0,
    redacted: 0,
  }));

describe("the metrics cards with nothing to show", () => {
  it("says so rather than drawing an empty axis", () => {
    // 24 buckets of zero is what a quiet day returns, and it is not the same as
    // no response at all - the chart used to draw a labelled axis under 24
    // invisible bars, which reads as broken.
    render(<MessagesChart buckets={quiet(24)} />);

    expect(screen.getByText("No messages sent in the last 24hrs")).toBeTruthy();
    expect(screen.queryByRole("img")).toBeNull();
  });

  it("says so for no buckets at all, which is what ships today", () => {
    render(<MessagesChart buckets={[]} />);
    expect(screen.getByText("No messages sent in the last 24hrs")).toBeTruthy();
  });

  it("still draws the chart as soon as one bucket carries traffic", () => {
    render(<MessagesChart buckets={[...quiet(23), { label: "24", total: 3, blocked: 1, flagged: 0, redacted: 0 }]} />);

    expect(screen.queryByText("No messages sent in the last 24hrs")).toBeNull();
    expect(screen.getByRole("img")).toBeTruthy();
  });

  it("reports an unmeasured saving as N/A, not as nothing saved", () => {
    render(
      <StatTiles
        stats={{
          messages: 0,
          blockedFlagged: 0,
          tokensSavedPercent: null,
          tokensSavedAmount: "+$0.00",
        }}
      />,
    );

    expect(screen.getByText("N/A")).toBeTruthy();
    // The amount goes with the percent: there is no figure to put beside it.
    expect(screen.queryByText("+$0.00")).toBeNull();
  });

  it("renders a real saving with its amount", () => {
    render(
      <StatTiles
        stats={{
          messages: 1284,
          blockedFlagged: 20,
          tokensSavedPercent: 38,
          tokensSavedAmount: "+$3.10",
        }}
      />,
    );

    expect(screen.getByText("38%")).toBeTruthy();
    expect(screen.getByText("+$3.10")).toBeTruthy();
    expect(screen.getByText("1,284")).toBeTruthy();
  });
});
