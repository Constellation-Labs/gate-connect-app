import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { QuitDialog, QuitSafeToCloseDialog } from "./dialogs";

/**
 * The quit flow's two dialogs.
 *
 * The first was a chooser until 2026-09-24 - two selectable rows, a "Safest"
 * pill, and a note that closing the window is a different action. Product
 * removed the second option, the pill with it (nothing left to be safest
 * against) and the note. So what is left to pin is the count, the one sentence
 * and the primary; the tests for the other branch's per-tool copy went with
 * the branch.
 */

const noop = () => {};

afterEach(cleanup);

function renderChooser(overrides: Partial<Parameters<typeof QuitDialog>[0]> = {}) {
  return render(
    <QuitDialog
      tools={["Claude Code", "Codex"]}
      onContinue={noop}
      onCancel={noop}
      {...overrides}
    />,
  );
}

describe("the quit dialog", () => {
  it("counts the apps still routed, and agrees with itself on number", () => {
    // Asserted on the dialog's text rather than one node: the count is drawn in
    // Medium inside a regular sentence, so it is a span of its own.
    const dialog = () => screen.getByRole("dialog").textContent ?? "";
    renderChooser();
    expect(dialog()).toContain("2 protected apps are still routed through Gate");
    cleanup();
    renderChooser({ tools: ["Codex"] });
    expect(dialog()).toContain("1 protected app is still routed through Gate");
  });

  /**
   * The one sentence that replaced the chooser.
   *
   * A second paragraph stood beside it - "Save your work first. Tools and
   * agents you have running will need restarting." - and was true while
   * quit-and-disconnect STOPPED the forwarder: every tool already holding its
   * address lost its route, and a reporter's editor died on this button. #363
   * changed the teardown to DRAIN it, so the forwarder serves whatever already
   * holds its address until the login session ends and nothing needs
   * restarting. The claim is asserted absent rather than merely dropped: it
   * was correct for a fortnight, and it should not come back without the
   * behaviour coming back with it.
   */
  it("says what it does, and claims no cost that no longer exists", () => {
    renderChooser();
    const dialog = screen.getByRole("dialog").textContent ?? "";
    expect(dialog).toContain("Restores your configurations and turns routing off");
    expect(dialog).not.toContain("Save your work");
    expect(dialog).not.toContain("restarting");
  });

  it("offers one way to quit, not a choice between two", () => {
    renderChooser();
    expect(screen.queryAllByRole("radio")).toHaveLength(0);
    expect(screen.queryByText(/Quit without disconnecting/)).toBeNull();
    expect(screen.queryByText("Safest")).toBeNull();
    // The note about closing the window went on the same request.
    expect(screen.queryByText(/Closing the main window/)).toBeNull();
  });

  it("names the primary for the action", () => {
    const onContinue = vi.fn();
    renderChooser({ onContinue });
    screen.getByRole("button", { name: "Disconnect" }).click();
    expect(onContinue).toHaveBeenCalled();
  });

});

describe("the safe-to-close confirmation", () => {
  it("reports a teardown that ran", () => {
    render(
      <QuitSafeToCloseDialog
        disconnected
        reverting={[]}
        onClose={noop}
        onCancel={noop}
      />,
    );
    expect(
      screen.getByText(
        // Not the drawn "Setup will be waiting the next time you open the
        // app": quitting with a disconnect puts tool configs back and leaves
        // the session, the org and the certificate alone, so the drawn sentence
        // described a reset that does not happen. Third standing copy
        // exception - see CLAUDE.md before "fixing" this back to the frame.
        "Tools are disconnected and their previous settings are restored. You will still be signed in the next time you open the app.",
      ),
    ).toBeTruthy();
  });

  it("reports the branch that reverts nothing, without claiming a restore", () => {
    render(
      <QuitSafeToCloseDialog
        disconnected={false}
        reverting={[]}
        onClose={noop}
        onCancel={noop}
      />,
    );
    expect(
      screen.getByText(
        "Routing settings were left in place. Some tools may need Gate Connect running to complete requests.",
      ),
    ).toBeTruthy();
    expect(screen.queryByText(/previous settings are restored/)).toBeNull();
  });

  /** The exit itself reverts the stranded configs, so the last screen before
   * the app closes has to name them: "routing settings were left in place" is
   * false about exactly the tools it is most about, and there is no screen
   * after this one to correct it on. */
  it("names what closing will put back, on the branch that declined", () => {
    render(
      <QuitSafeToCloseDialog
        disconnected={false}
        reverting={["Codex"]}
        onClose={noop}
        onCancel={noop}
      />,
    );
    const note = screen.getByRole("dialog").textContent ?? "";
    expect(note).toContain("except for Codex");
    expect(note).toContain("puts its own settings back");
    expect(note).toContain("reconnects it when Gate Connect starts again");
  });

  /** A read that failed says so, on the last screen before the app closes,
   * rather than borrowing the sentence for "nothing reverts". */
  it("admits when it could not check what closing will put back", () => {
    render(
      <QuitSafeToCloseDialog
        disconnected={false}
        reverting={null}
        onClose={noop}
        onCancel={noop}
      />,
    );
    const note = screen.getByRole("dialog").textContent ?? "";
    expect(note).toContain("couldn't check which those are");
    expect(note).not.toContain("Some tools may need Gate Connect running");
  });

  it("closes on the primary and stays open on Cancel", () => {
    const onClose = vi.fn();
    const onCancel = vi.fn();
    render(
      <QuitSafeToCloseDialog
        disconnected
        reverting={[]}
        onClose={onClose}
        onCancel={onCancel}
      />,
    );
    screen.getByRole("button", { name: "Close Gate Connect" }).click();
    expect(onClose).toHaveBeenCalled();
    screen.getByRole("button", { name: "Cancel" }).click();
    expect(onCancel).toHaveBeenCalled();
  });
});
