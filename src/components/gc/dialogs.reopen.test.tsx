import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import type { DialogReopenTool } from "./dialogs";
import { ReopenProgressDialog } from "./dialogs";

const noop = () => {};
afterEach(cleanup);

function tool(over: Partial<DialogReopenTool> = {}): DialogReopenTool {
  return {
    slug: "codex",
    name: "Codex",
    canReopen: false,
    running: true,
    verifiable: true,
    routeInUse: null,
    requestedRoute: null,
    stage: "routing",
    ...over,
  };
}

/** The exact set the ticket screenshotted: five tools across three buckets. */
function settled() {
  return [
    tool({ slug: "claude-code", name: "Claude Code", stage: "routing" }),
    tool({ slug: "opencode", name: "OpenCode", stage: "routing" }),
    tool({ slug: "chatgpt", name: "ChatGPT", stage: "reopened" }),
    tool({ slug: "codex", name: "Codex", stage: "verify_failed" }),
  ];
}

function open(tools = settled()) {
  return render(
    <ReopenProgressDialog tools={tools} onAction={noop} onDone={noop} />,
  );
}

describe("ReopenProgressDialog, once everything has settled (AG-898)", () => {
  it("does not sort the tools into headed outcome groups", () => {
    open();

    // Titles only the grouping ever drew. "Verification failed" is deliberately
    // not in this list: it is also `REOPEN_STAGE_LABEL.verify_failed`, which
    // the Codex row still carries and should.
    for (const heading of [
      "Applied and verified",
      "Reopened, not checked",
      "Waiting for you to reopen",
      "Could not be closed or reopened",
    ]) {
      expect(screen.queryByText(heading)).toBeNull();
    }
  });

  it("drops the per-bucket explanations the reader had to cross-reference", () => {
    open();

    for (const blurb of [
      "Gate checked these after they came back.",
      "Their configuration is saved. Open each one and Gate finishes the check.",
      "These are open, and Gate could not confirm their route.",
    ]) {
      expect(screen.queryByText(blurb)).toBeNull();
    }
  });

  it("lists ChatGPT and Codex as the one app they are elsewhere in Connect", () => {
    open();

    expect(screen.getByText("ChatGPT / Codex")).toBeTruthy();
    // Neither surface gets a row of its own beside the app's.
    expect(screen.queryByText("ChatGPT")).toBeNull();
  });

  /** The app reports its worse half, and says so rather than hiding it. */
  it("accounts for the members when they disagree", () => {
    open();

    expect(screen.getByText(/Codex: Verification failed/)).toBeTruthy();
    expect(screen.getByText(/ChatGPT: Reopened/)).toBeTruthy();
  });

  it("says nothing per surface when an app's members agree", () => {
    open([
      tool({ slug: "chatgpt", name: "ChatGPT", stage: "routing" }),
      tool({ slug: "codex", name: "Codex", stage: "routing" }),
    ]);

    expect(screen.queryByText(/Codex: /)).toBeNull();
  });

  it("offers no route to support from a failed routing check", () => {
    open();

    expect(screen.queryByRole("button", { name: "Contact support" })).toBeNull();
    expect(screen.getByRole("button", { name: "Retry verification" })).toBeTruthy();
  });

  it("gives one row per app rather than one per running process", () => {
    const { container } = open();

    // Claude, OpenCode, ChatGPT / Codex.
    expect(container.querySelectorAll("div.rounded-md.border")).toHaveLength(3);
  });
});
