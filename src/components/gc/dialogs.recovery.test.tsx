import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import type { RecoverySummary, RecoveryTool } from "../../lib/api";
import { RestoreDetailsDialog } from "./dialogs";

const noop = () => {};
const NOW = new Date("2026-09-16T12:00:00Z");
const NOW_UNIX = Math.floor(NOW.getTime() / 1000);

afterEach(cleanup);

/** The row AG-886 was filed against: a provider the restore never reached. */
function openai(overrides: Partial<RecoveryTool> = {}): RecoveryTool {
  return {
    slug: "openai",
    name: "OpenAI",
    kind: "provider",
    stage: "pending",
    stage_complete: false,
    error_category: "none",
    error: null,
    stage_at_unix: NOW_UNIX - 240,
    last_verified_state: null,
    last_verified_unix: 0,
    check_state: null,
    check_reason: null,
    check_at_unix: 0,
    running: null,
    reopen_pending: false,
    next_step: "retry",
    ...overrides,
  };
}

function summary(tools: RecoveryTool[] = [openai()]): RecoverySummary {
  return {
    operation: "restore",
    updated_unix: NOW_UNIX - 240,
    requested_routing_on: true,
    tools,
  };
}

function open(tools?: RecoveryTool[]) {
  return render(
    <RestoreDetailsDialog summary={summary(tools)} now={NOW} onClose={noop} />,
  );
}

/**
 * AG-886. The reader arrives from "Routing didn't finish coming back" wanting
 * to know which app is affected and what to do. Every assertion here is about
 * what they meet before they expand anything.
 */
describe("RestoreDetailsDialog, read cold", () => {
  it("names the app that is not routed, instead of counting stages", () => {
    open();

    expect(screen.getByText("OpenAI is not routing through Gate yet.")).toBeTruthy();
    expect(screen.queryByText(/stages completed/)).toBeNull();
    expect(screen.queryByText(/still pending/)).toBeNull();
  });

  it("gives the one thing to do, pointing at a surface that can do it", () => {
    open();

    expect(
      screen.getByText("Choose Resume now on the routing notice to finish this."),
    ).toBeTruthy();
  });

  it("subtitles in the user's terms, not the journal's", () => {
    open();

    expect(screen.getByText(/Gate was turning routing on 4m ago and did not finish\./)).toBeTruthy();
    expect(screen.queryByText(/for every tool it had recorded/)).toBeNull();
  });

  /**
   * The AG-570 diagnostics are still in the DOM - handing this summary to
   * someone else is one of the things the review is for - but a collapsed
   * `<details>` is what keeps them out of the answer. Asserting on the closed
   * element rather than on absence, because `queryByText` finds text inside a
   * collapsed disclosure just as readily as outside one.
   */
  it("keeps the internal readings behind a closed disclosure", () => {
    const { container } = open();

    const details = container.querySelector("details");
    expect(details).not.toBeNull();
    expect(details?.hasAttribute("open")).toBe(false);

    for (const internal of [
      "Checked per tool, not per provider",
      "Gate has no process to look for",
      "No reading yet",
      "Not started",
    ]) {
      expect(details?.textContent).toContain(internal);
    }
  });

  it("does not put the journal's stage name on the pill beside the app", () => {
    const { container } = open();

    const pill = container.querySelector("span.font-mono");
    expect(pill?.textContent).toBe("Unfinished");
  });

  it("marks a settled row done rather than unfinished", () => {
    const { container } = open([
      openai({ stage: "restored", stage_complete: true, next_step: "none" }),
    ]);

    expect(container.querySelector("span.font-mono")?.textContent).toBe("Done");
    expect(screen.getByText("This one is set up and routing again.")).toBeTruthy();
    expect(screen.getByText(/Everything Gate recorded is done/)).toBeTruthy();
  });

  it("names several stuck apps in one line", () => {
    open([openai(), openai({ slug: "codex", name: "Codex" })]);

    expect(
      screen.getByText("2 apps are not routing through Gate yet: OpenAI and Codex."),
    ).toBeTruthy();
  });
});
