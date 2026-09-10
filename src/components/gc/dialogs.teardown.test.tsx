import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { TeardownReportDialog } from "./dialogs";
import type { DialogTeardownReport } from "./dialogs";

const noop = () => {};

afterEach(cleanup);

const EMPTY: DialogTeardownReport = {
  defaults: [],
  still_gate: [],
  awaiting_reopen: [],
  failed: [],
};

function renderReport(overrides: Partial<DialogTeardownReport> = {}) {
  return render(
    <TeardownReportDialog report={{ ...EMPTY, ...overrides }} onClose={noop} />,
  );
}

/**
 * The row mark.
 *
 * `ModalSubject.icon` is a `ReactNode`, not an icon NAME - every other call site
 * hands it an element. This dialog passed the bare string `"cube"`, which React
 * rendered as the literal word inside the 40px tile: four rows reading "cube"
 * where the product marks belong. TypeScript could not object, because `string`
 * is a perfectly good `ReactNode`, so only a rendering test can hold this.
 */
describe("TeardownReportDialog row marks", () => {
  it("never renders an icon name as text", () => {
    renderReport({
      awaiting_reopen: [
        { slug: "claude-code", name: "Claude Code", next_action: "reopen_tool" },
      ],
    });

    expect(screen.getByText("Claude Code")).toBeTruthy();
    expect(screen.queryByText("cube")).toBeNull();
  });

  it("draws the mark the shell supplies", () => {
    renderReport({
      defaults: [
        {
          slug: "opencode",
          name: "OpenCode",
          next_action: "none",
          icon: <svg data-testid="opencode-mark" />,
        },
      ],
    });

    expect(screen.getByTestId("opencode-mark")).toBeTruthy();
    expect(screen.queryByText("cube")).toBeNull();
  });

  it("falls back to a glyph, not a word, when no mark is supplied", () => {
    renderReport({
      defaults: [{ slug: "mystery", name: "Mystery Tool", next_action: "none" }],
    });

    expect(screen.getByText("Mystery Tool")).toBeTruthy();
    expect(screen.queryByText("cube")).toBeNull();

    // Scoped to the ROW, not the dialog. A `container.querySelectorAll("svg")`
    // here was satisfied by the Modal's own tone tile, which always draws an
    // icon: dropping the fallback entirely and leaving the 40px row tile empty
    // kept all three tests green.
    const row = screen.getByText("Mystery Tool").closest("div")?.parentElement;
    expect(row?.querySelectorAll("svg").length ?? 0).toBeGreaterThan(0);
  });
});
