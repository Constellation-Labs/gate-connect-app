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
  category: "pii",
  categoryIcon: "userRound",
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
      plan={null}
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

describe("AppPane header", () => {
  it("says what the app is, because its name no longer does", () => {
    // The h1 is a surface kind now ("CLI"), which the rail makes legible with a
    // vendor eyebrow the pane does not have. Without the sentence this header
    // is one word and no way to tell which terminal tool it means.
    render(pane({ name: "CLI", description: "Claude Code in your terminal." }));
    expect(screen.getByRole("heading", { level: 1, name: "CLI" })).toBeTruthy();
    expect(screen.getByText("Claude Code in your terminal.")).toBeTruthy();
  });

  it("draws no line at all for a row nobody wrote copy for", () => {
    // Absent means absent: a placeholder under a one-word heading is worse than
    // the heading alone.
    render(pane({ name: "OpenCode Zen / Go" }));
    expect(screen.getByRole("heading", { level: 1, name: "OpenCode Zen / Go" })).toBeTruthy();
    expect(screen.getByText("Protected")).toBeTruthy();
  });
});

describe("AppPane model card", () => {
  it("draws the model card when the app has one model family", () => {
    render(pane());
    expect(screen.getByRole("heading", { name: "Model selection" })).toBeTruthy();
  });

  /**
   * The multi-provider tools - OpenCode, OpenClaw, Hermes - route whichever of
   * their configured providers Gate covers, so "what does this app use on Gate
   * model" has no single answer for them. `main` never asks it: it has no model
   * UI at all and these appear only as routing targets.
   *
   * The Figma's answer is a multi-select picker, which needs a model list no
   * endpoint reports yet and a selection shape `ModelChoice` cannot hold. Until
   * then the card is withheld, which is a decision and not an oversight -
   * regressing it looks like "the model card is missing on OpenCode".
   */
  it("omits it entirely when there is no single model family", () => {
    render(pane({ onChooseModel: undefined }));
    expect(screen.queryByRole("heading", { name: "Model selection" })).toBeNull();
    expect(screen.queryByText("Change model")).toBeNull();
    expect(screen.queryByText("App default")).toBeNull();
    // The rest of the pane is untouched: it still routes, and still reports.
    expect(screen.getByRole("heading", { name: "Recent activity" })).toBeTruthy();
  });
});

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
    // failure wins, and the verdict it displaced stays reachable rather than being
    // dropped - on hover for pointer users, and as `sr-only` text for everyone
    // else, since a tooltip alone would leave keyboard and screen reader users
    // with no route to it at all.
    expect(within(feed).getByText("error")).toBeTruthy();
    expect(within(feed).queryByText("flagged")).toBeNull();
    expect(within(feed).getByTitle("Request failed. Guardrails: flagged.")).toBeTruthy();
    expect(within(feed).getByText("Guardrails: flagged.")).toBeTruthy();
  });

  it("adds no displaced verdict when a failed row had none", () => {
    render(pane({ activity: [{ ...entry, status: "error", security: null }] }));
    const feed = card("Recent activity");

    expect(within(feed).getByTitle("Request failed.")).toBeTruthy();
    expect(within(feed).queryByText(/Guardrails:/)).toBeNull();
  });

  it("shows the guardrail verdict when the request succeeded", () => {
    render(pane({ activity: [{ ...entry, status: "success", security: "redacted" }] }));

    expect(within(card("Recent activity")).getByText("redacted")).toBeTruthy();
  });

  it("wears one pill per row, with no column left for a status", () => {
    render(
      pane({
        activity: [
          { ...entry, id: "req-flagged", status: "error", security: "flagged" },
          { ...entry, id: "req-error", status: "error", security: null },
        ],
      }),
    );
    const feed = card("Recent activity");

    // The design merged the old Status column into Security, so there is no
    // second cell to put a transport outcome in and no SUCCESS pill at all.
    // Which of the two facts a row shows when it has both is the precedence
    // question, pinned above; what this pins is that it only ever shows one.
    expect(within(feed).queryByRole("columnheader", { name: "Status" })).toBeNull();
    expect(within(feed).queryByText("success")).toBeNull();
    // Both rows failed, so under the precedence above both wear ERROR: the
    // first displaced its `flagged` into the tooltip, the second never had a
    // verdict to displace.
    expect(within(feed).getAllByText("error")).toHaveLength(2);
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

  it("draws the columns the frame draws, and not the prompt", () => {
    render(pane({ activity: [entry] }));
    const feed = card("Recent activity");

    // `table/recent-activity` on `Flows / App` draws these five, in this order,
    // across all three frames that carry the card.
    for (const name of ["Time", "Type", "Security", "Model", "Action"]) {
      expect(within(feed).getByRole("columnheader", { name })).toBeTruthy();
    }
    // No Message column: the frame has none, so the prompt and its reference are
    // not on this surface even though the feed still carries both.
    expect(within(feed).queryByRole("columnheader", { name: "Message" })).toBeNull();
    expect(within(feed).queryByText("Update our data-model.md")).toBeNull();
    expect(within(feed).queryByText("824bd2c0-4123")).toBeNull();
  });

  it("names the guardrail category as the gateway spelled it", () => {
    render(pane({ activity: [entry] }));
    const feed = card("Recent activity");

    expect(within(feed).getByText("pii")).toBeTruthy();
    expect(within(feed).getByText("claude-opus-4")).toBeTruthy();
    expect(within(feed).getByTitle("anthropic")).toBeTruthy();
    // The monogram is decorative, so the provider has to be named in text too -
    // otherwise the one-letter glyph is all a screen reader gets.
    expect(within(feed).getByText("anthropic")).toBeTruthy();
  });

  it("withholds the category rather than inventing one", () => {
    // Same split the Security cell makes: the gateway named no category, or the
    // row is not this caller's to see into. Both draw the dash.
    render(pane({ activity: [{ ...entry, category: null, categoryIcon: null }] }));
    const feed = card("Recent activity");

    expect(within(feed).queryByText("pii")).toBeNull();
    expect(
      within(feed).getByTitle("No guardrail category recorded, or not your request"),
    ).toBeTruthy();
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
  const model = { vendor: "anthropic", ids: ["anthropic/claude-opus-5"] };

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

  it("does not report a current Gate model under App default", () => {
    // A section headed "Current Gate model" while the app runs its own is a
    // sentence about nothing current. The remembered model is still named - by
    // the radio, which is the control that would put it to use.
    render(pane({ modelChoice: "app", gateModel: model }));
    const card_ = card("Model selection");

    expect(within(card_).queryByText(/Current Gate model/i)).toBeNull();
    expect(within(card_).queryByRole("button", { name: "Change model" })).toBeNull();
    expect(within(card_).getByText(`Use ${model.ids[0]}`)).toBeTruthy();
  });

  it("reports it once Gate is the one serving", () => {
    render(pane({ modelChoice: "gate", gateModel: model }));
    const card_ = card("Model selection");

    expect(within(card_).getByText(/Current Gate model/i)).toBeTruthy();
    expect(within(card_).getByText(model.ids[0])).toBeTruthy();
  });

  it("lists every enabled model, not the first of them", () => {
    // Reported from the running app: six models chosen, one drawn, and a heading
    // reading "Current Gate models" above it. A plural heading over a single row
    // is indistinguishable from the card having lost the other five.
    const ids = [
      "openai/gpt-5-6-terra",
      "openai/gpt-5-6-sol",
      "openai/gpt-5-6-luna",
      "openai/gpt-5-3-codex",
      "openai/gpt-5-2",
      "openai/gpt-5-1",
    ];
    render(pane({ modelChoice: "gate", gateModel: { vendor: "openai", ids } }));
    const card_ = card("Model selection");

    for (const id of ids) expect(within(card_).getByText(id)).toBeTruthy();
    expect(within(card_).getByText("Current Gate models")).toBeTruthy();
    // One action for the set, not one per row.
    expect(within(card_).getAllByRole("button", { name: "Change model" })).toHaveLength(1);
  });

  it("names the size of the set on the radio rather than one of its members", () => {
    // "Use openai/gpt-5-6-terra" beside six enabled models says Gate will use
    // that one, which is the opposite of what a set means.
    render(
      pane({
        modelChoice: "app",
        gateModel: { vendor: "openai", ids: ["openai/gpt-5-2", "openai/gpt-5-1"] },
      }),
    );
    expect(within(card("Model selection")).getByText("Use any of 2 Gate models")).toBeTruthy();
  });

  it("shows the plan the gateway named, which AG-592 asks the tool detail for", () => {
    render(pane({ modelChoice: "gate", gateModel: { vendor: "openai", ids: ["openai/gpt-5"] }, plan: "paid" }));
    expect(within(card("Model selection")).getByText("Paid plan")).toBeTruthy();
  });

  it("says nothing about a plan nobody named", () => {
    // It used to default to "free". A plan is what a reader acts on, by going to
    // upgrade - and "Free" would send them to change something they may already
    // have changed. Principle 6: no figure without a reading behind it.
    render(pane({ modelChoice: "gate", gateModel: { vendor: "openai", ids: ["openai/gpt-5"] }, plan: null }));
    expect(within(card("Model selection")).queryByText(/plan/i)).toBeNull();
  });

  it("says no model is chosen rather than drawing an empty row", () => {
    // Reachable while Gate is the source and the set came back empty - the state
    // the pane must not draw as a blank row pretending to name something.
    render(pane({ modelChoice: "gate", gateModel: null }));
    expect(within(card("Model selection")).getByText(/No Gate model chosen yet/i)).toBeTruthy();
  });

  it("refuses a second click while a write is in flight", () => {
    render(pane({ modelChoice: "gate", gateModel: model, modelBusy: true }));
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
