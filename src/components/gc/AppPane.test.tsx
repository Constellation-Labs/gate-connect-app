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
  provider: "anthropic",
  title: "Update our data-model.md",
  reference: "824bd2c0-4123",
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
      gateModel={null}
      onChangeModel={() => {}}
      credits={null}
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

  it("shows one badge, and lets a failure outrank the guardrail verdict", () => {
    render(pane({ activity: [{ ...entry, status: "error", security: "flagged" }] }));
    const feed = card("Recent activity");

    // Status and security share a column now, so the row cannot show both. The
    // failure wins, and the verdict it displaced stays reachable on hover rather
    // than being dropped.
    expect(within(feed).getByText("error")).toBeTruthy();
    expect(within(feed).queryByText("flagged")).toBeNull();
    expect(within(feed).getByTitle("Request failed. Guardrails: flagged.")).toBeTruthy();
  });

  it("shows the guardrail verdict when the request succeeded", () => {
    render(pane({ activity: [{ ...entry, status: "success", security: "redacted" }] }));

    expect(within(card("Recent activity")).getByText("redacted")).toBeTruthy();
  });

  it("marks a row whose security detail is absent, without inventing a verdict", () => {
    render(pane({ activity: [{ ...entry, security: null }] }));
    const feed = card("Recent activity");

    // Absence of a verdict is not enough: a cell that rendered nothing at all
    // would satisfy that, and the point is that the row says so.
    const cell = within(feed).getByTitle("No security action recorded, or not your request");
    expect(cell.textContent).toBe("\u2014");
    expect(within(feed).queryByText("allow")).toBeNull();
    expect(within(feed).queryByText("flagged")).toBeNull();
  });

  it("offers a per-row action when the surface supplies a destination", () => {
    const onView = vi.fn();
    render(pane({ activity: [{ ...entry, onView }] }));

    within(card("Recent activity")).getByRole("button", { name: "View" }).click();
    expect(onView).toHaveBeenCalledOnce();
  });

  it("draws no action when there is nowhere to send the user", () => {
    // The row type makes `onView` optional for exactly this: an inert control is
    // worse than an absent one.
    render(pane({ activity: [entry] }));

    expect(within(card("Recent activity")).queryByRole("button", { name: "View" })).toBeNull();
  });

  it("renders the message, its reference, and the vendor beside the model", () => {
    render(pane({ activity: [entry] }));
    const feed = card("Recent activity");

    expect(within(feed).getByText("Update our data-model.md")).toBeTruthy();
    expect(within(feed).getByText("824bd2c0-4123")).toBeTruthy();
    expect(within(feed).getByText("claude-opus-4")).toBeTruthy();
    expect(within(feed).getByTitle("anthropic")).toBeTruthy();
    expect(within(feed).getByRole("columnheader", { name: "Message" })).toBeTruthy();
  });

  it("shows the reference alone when there is no message to show", () => {
    // Null covers three cases the row does not distinguish - no session, a
    // placeholder name, and a row this caller may not see into.
    render(pane({ activity: [{ ...entry, title: null }] }));
    const feed = card("Recent activity");

    expect(within(feed).queryByText("Update our data-model.md")).toBeNull();
    expect(within(feed).getByText("824bd2c0-4123")).toBeTruthy();
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

/**
 * The model card, whose whole job is to not overstate what it knows.
 *
 * Three separate states get confused if the card is careless, and each is a
 * different sentence to the user: "we have not read this yet", "this app cannot
 * have one", and "a model is chosen but not in use". Collapsing any pair of them
 * produces a control that lies about what it does.
 */
describe("AppPane model selection", () => {
  const model = { vendor: "anthropic", id: "anthropic/claude-opus-5" };

  it("selects neither option when no reading landed", () => {
    // Principle 2, in its purest form: an org that HAD switched to a Gate model
    // would see App default selected the instant a read failed, and clicking
    // Gate model would look like a change when it was a no-op.
    render(pane({ modelChoice: null }));
    const card_ = card("Model selection");

    for (const radio of within(card_).getAllByRole("radio")) {
      expect(radio.getAttribute("aria-checked")).toBe("false");
      expect((radio as HTMLButtonElement).disabled).toBe(true);
    }
    expect(within(card_).getByText(/could not read this app's model setting/i)).toBeTruthy();
  });

  it("draws skeletons rather than a default while the reading is in flight", () => {
    // `modelPending`, not the pane's `pending`: the latter tracks the activity
    // reading, and an unattributed machine has nothing to say about a setting.
    // Sharing one flag made this card draw skeletons forever on such a machine.
    render(pane({ modelChoice: null, modelPending: true }));
    const card_ = card("Model selection");

    expect(within(card_).queryByRole("radio")).toBeNull();
    expect(card_.querySelectorAll(".animate-pulse").length).toBeGreaterThan(0);
  });

  it("marks a remembered model as not in use under App default", () => {
    render(pane({ modelChoice: "app", gateModel: model }));
    const card_ = card("Model selection");

    expect(within(card_).getByText(model.id)).toBeTruthy();
    expect(within(card_).getByText(/not in use/i)).toBeTruthy();
  });

  it("drops the qualifier once Gate is actually serving it", () => {
    render(pane({ modelChoice: "gate", gateModel: model }));
    const card_ = card("Model selection");

    expect(within(card_).getByText(model.id)).toBeTruthy();
    expect(within(card_).queryByText(/not in use/i)).toBeNull();
  });

  it("says no model is chosen rather than drawing an empty row", () => {
    render(pane({ modelChoice: "app", gateModel: null }));
    expect(within(card("Model selection")).getByText(/No Gate model chosen yet/i)).toBeTruthy();
  });

  it("refuses a second click while a write is in flight", () => {
    render(pane({ modelChoice: "app", gateModel: model, modelBusy: true }));
    const card_ = card("Model selection");

    for (const radio of within(card_).getAllByRole("radio")) expect((radio as HTMLButtonElement).disabled).toBe(true);
    expect((within(card_).getByRole("button", { name: "Change model" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("reads N/A for a credit balance nothing reports", () => {
    // Not a dash: a dash reads as a value. No endpoint returns a Gate balance.
    render(pane({ credits: null }));
    expect(within(card("Model selection")).getByText("N/A")).toBeTruthy();
  });

  it("chooses through the callback rather than deciding locally", async () => {
    const onChooseModel = vi.fn();
    render(pane({ modelChoice: "app", gateModel: model, onChooseModel }));
    const card_ = card("Model selection");

    within(card_).getByRole("radio", { name: /Gate model/ }).click();
    expect(onChooseModel).toHaveBeenCalledWith("gate");
  });
});
