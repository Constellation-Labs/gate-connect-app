import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { SecurityPane } from "./SecurityPane";
import type { SecurityEvent } from "../../lib/api";

afterEach(cleanup);

const blocked: SecurityEvent = {
  id: "01A",
  requestId: "req-1",
  at: "2026-08-31T14:03:00Z",
  action: "block",
  category: "credential",
  tool: "claude-code",
  model: "claude-opus-4",
  provider: "anthropic",
};

function pane(props: Partial<Parameters<typeof SecurityPane>[0]> = {}) {
  return (
    <SecurityPane
      events={[]}
      state="live"
      loading={false}
      unavailable={false}
      onRetry={() => {}}
      onOpenEvent={() => {}}
      {...props}
    />
  );
}

describe("the three states a period can be in", () => {
  // AC6 turns on these three being distinguishable. A zero is a reading, an
  // unavailable feed is not, and a feed still being read is neither.
  it("says No security events when the feed loaded and there were none", () => {
    render(pane());
    expect(screen.getByText("No security events")).toBeTruthy();
    expect(screen.queryByText("Unavailable")).toBeNull();
  });

  it("says Unavailable and offers a way out when the feed could not load", () => {
    render(pane({ unavailable: true }));
    expect(screen.getByText("Unavailable")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
    // The empty sentence is a claim about the user's traffic and must not be
    // made by a screen that failed to ask.
    expect(screen.queryByText("No security events")).toBeNull();
  });

  it("claims neither while the first read is still in flight", () => {
    render(pane({ loading: true }));
    expect(screen.queryByText("No security events")).toBeNull();
    expect(screen.queryByText("Unavailable")).toBeNull();
  });
});

describe("the feed's own connection state", () => {
  // AC4: the feed reports its connection independently of routing. Nothing in
  // this pane reads a routing value, which is the point.
  it.each([
    ["live", "Live"],
    ["reconnecting", "Reconnecting"],
    ["offline", "Offline"],
  ] as const)("shows %s as %s", (state, label) => {
    render(pane({ state }));
    expect(screen.getByRole("status", { name: `Event feed ${label}` })).toBeTruthy();
  });

  it("keeps showing the events it has while reconnecting", () => {
    // A feed having a bad minute is not an empty feed, and blanking the table
    // would lose what the user was reading.
    render(pane({ state: "reconnecting", events: [blocked] }));
    expect(screen.getByText("Blocked")).toBeTruthy();
    expect(screen.queryByText("No security events")).toBeNull();
  });
});

describe("what a row shows, and what it must not", () => {
  it("names the verdict, category, tool and model", () => {
    render(pane({ events: [blocked] }));
    expect(screen.getByText("Blocked")).toBeTruthy();
    expect(screen.getByText("credential")).toBeTruthy();
    expect(screen.getByText("claude-code")).toBeTruthy();
    expect(screen.getByText("claude-opus-4")).toBeTruthy();
  });

  it("draws a flagged event as Flagged, not Blocked", () => {
    render(pane({ events: [{ ...blocked, id: "01B", action: "flag" }] }));
    expect(screen.getByText("Flagged")).toBeTruthy();
    expect(screen.queryByText("Blocked")).toBeNull();
  });

  it("renders an unattributed tool or model as an ordinary dash", () => {
    // Null is the normal outcome for an agent the gateway's allowlist does not
    // name. It is not an error state and must not read as one.
    render(pane({ events: [{ ...blocked, tool: null, model: null, category: null }] }));
    expect(screen.queryByText("Unknown")).toBeNull();
    expect(screen.queryByText("Unavailable")).toBeNull();
    expect(screen.getAllByText("-").length).toBe(3);
  });

  it("shows newest first", () => {
    const older = { ...blocked, id: "01A", at: "2026-08-31T10:00:00Z", category: "pii" };
    const newer = { ...blocked, id: "01B", at: "2026-08-31T14:00:00Z", category: "injection" };
    render(pane({ events: [older, newer] }));
    const cells = screen.getAllByText(/pii|injection/);
    expect(cells[0].textContent).toBe("injection");
  });
});

describe("opening an event", () => {
  it("hands the whole event to the caller rather than a bare id", () => {
    // AC7 needs the summary to stay on screen until the dashboard opens, so the
    // shell needs the event itself, not just something to build a URL from.
    const onOpenEvent = vi.fn();
    render(pane({ events: [blocked], onOpenEvent }));
    fireEvent.click(screen.getByRole("button", { name: /View/ }));
    expect(onOpenEvent).toHaveBeenCalledWith(blocked);
  });

  it("retries on demand when the feed is unavailable", () => {
    const onRetry = vi.fn();
    render(pane({ unavailable: true, onRetry }));
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(onRetry).toHaveBeenCalled();
  });
});
