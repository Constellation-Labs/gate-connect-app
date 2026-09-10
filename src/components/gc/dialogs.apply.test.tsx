import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { ApplyChangesDialog } from "./dialogs";
import type { DialogReopenTool } from "./dialogs";

const noop = () => {};

afterEach(() => {
  cleanup();
  vi.unstubAllEnvs();
});

const codex: DialogReopenTool = {
  slug: "codex",
  name: "Codex",
  stage: "reopen_required",
  running: true,
  canReopen: false,
  verifiable: true,
  routeInUse: "https://gateway-staging.constellationgate.ai",
  requestedRoute: "https://api.openai.com/v1",
};

function renderOffer(tools: DialogReopenTool[] = [codex]) {
  return render(
    <ApplyChangesDialog tools={tools} onCloseApps={noop} onReopenLater={noop} />,
  );
}

/**
 * The routes are a development affordance, not shipped copy.
 *
 * AG-566 AC 1 asks the offer step to name the route in use and the route
 * requested, and it was built to. The frame does not draw either: `130:58427`
 * gives Codex a name, one description line and an `OPEN` pill. The file wins on
 * what ships, so the pair is gated on `import.meta.env.DEV` rather than deleted
 * - it is the fastest way to see which endpoint a tool is actually on while
 * debugging, and it is false in every `vite build`.
 *
 * **Pass `vi.stubEnv` a boolean, never a string.** `stubEnv` does reach
 * `import.meta.env.DEV` - measured both ways on this tree, and a review that
 * claimed otherwise was checked and rejected - but it writes the value it is
 * given, and `import.meta.env` is not the `process.env` string coercion. So
 * `vi.stubEnv("DEV", "false")` stores the *string* `"false"`, which is truthy,
 * and the gate stays on: the "ships neither route" test below would then be
 * asserting the dev build twice and passing for the wrong reason. The two tests
 * are each other's check - they are mutually exclusive on one flag, so a stub
 * that failed to bite could not leave both green - and that only holds while
 * the values stay booleans.
 */
describe("ApplyChangesDialog route pair", () => {
  it("names both routes in a development build", () => {
    vi.stubEnv("DEV", true);
    renderOffer();

    expect(screen.getByText(/In use:/)).toBeTruthy();
    expect(
      screen.getByText("https://gateway-staging.constellationgate.ai"),
    ).toBeTruthy();
    expect(screen.getByText("https://api.openai.com/v1")).toBeTruthy();
  });

  it("ships neither route, because the frame draws neither", () => {
    vi.stubEnv("DEV", false);
    renderOffer();

    expect(screen.queryByText(/In use:/)).toBeNull();
    expect(
      screen.queryByText("https://gateway-staging.constellationgate.ai"),
    ).toBeNull();
    // The row itself survives: the tool, its state and the pill are drawn.
    expect(screen.getByText("Codex")).toBeTruthy();
    expect(screen.getByText("Open")).toBeTruthy();
  });

  it("says who reopens once, in the note, not once per row", () => {
    vi.stubEnv("DEV", false);
    renderOffer([codex, { ...codex, slug: "claude-code", name: "Claude Code" }]);

    // The frame draws no per-row sentence, and the note below already carries
    // the same `canReopen` split for the whole set. Two rows repeating it cost
    // four lines in a 400px popover to say what the next block says properly.
    expect(screen.queryByText(/Gate Connect can close it, and you reopen it/)).toBeNull();
    expect(
      screen.queryByText(/Gate Connect can close and reopen this one/),
    ).toBeNull();

    // The fact itself survives, once, for the whole set.
    expect(
      screen.getByText(/Gate Connect can close these apps, but cannot reopen them/),
    ).toBeTruthy();
  });

  it("still names which tools Gate can reopen when some of them differ", () => {
    vi.stubEnv("DEV", false);
    renderOffer([
      codex,
      { ...codex, slug: "claude-code", name: "Claude Code", canReopen: true },
    ]);

    expect(screen.getByText(/Gate Connect will reopen Claude Code/)).toBeTruthy();
  });

  it("draws the mark the shell supplies rather than a fallback word", () => {
    vi.stubEnv("DEV", false);
    renderOffer([{ ...codex, icon: <svg data-testid="codex-mark" /> }]);

    expect(screen.getByTestId("codex-mark")).toBeTruthy();
    expect(screen.queryByText("cube")).toBeNull();
  });
});
