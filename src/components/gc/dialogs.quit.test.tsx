import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { QuitDialog, QuitSafeToCloseDialog } from "./dialogs";

/**
 * The quit flow's two drawn dialogs (`Flows / Overview`, `overview-quit`, read
 * 2026-08-28). What is worth pinning here is the copy - it is the file's, and
 * the confirmation's two branches say different things about the user's
 * machine - plus the primary label, which is named for the action and so has
 * to follow the selection.
 */

const noop = () => {};

afterEach(cleanup);

function renderChooser(overrides: Partial<Parameters<typeof QuitDialog>[0]> = {}) {
  return render(
    <QuitDialog
      tools={["Claude Code", "Codex"]}
      reverting={["Claude Code", "Codex"]}
      platform="macos"
      choice="disconnect"
      onChoose={noop}
      onContinue={noop}
      onCancel={noop}
      {...overrides}
    />,
  );
}

describe("the quit chooser", () => {
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

  it("offers both outcomes, recommending the safe one", () => {
    renderChooser();
    const safe = screen.getByRole("radio", { name: /Disconnect tools and quit/ });
    expect(safe.getAttribute("aria-checked")).toBe("true");
    expect(safe.textContent).toContain("Safest");
    expect(
      screen
        .getByRole("radio", { name: /Quit without disconnecting/ })
        .getAttribute("aria-checked"),
    ).toBe("false");
  });

  it("names the primary for the action, so the choice renames it", () => {
    const onContinue = vi.fn();
    renderChooser({ onContinue });
    screen.getByRole("button", { name: "Disconnect" }).click();
    expect(onContinue).toHaveBeenCalled();
    cleanup();
    // "Disconnect" over "Quit without disconnecting" would label a button with
    // the opposite of what it does. The second label is inferred: the frame
    // draws only the first row selected.
    renderChooser({ choice: "leave" });
    expect(screen.getByRole("button", { name: "Continue" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Disconnect" })).toBeNull();
  });

  /**
   * What a plain quit will actually do, per tool.
   *
   * The drawn description ("Leave configurations pointed at Gate. Requests that
   * depend on the local proxy may pause.") stopped being true of this branch
   * when the exit began putting the stranded configs back: it warns of a pause
   * for tools that will be handed their own settings, and glosses the tools
   * that really do keep pointing at Gate - which the forwarder keeps working.
   * `reverting` is the backend's own split, and the row names both sides of it.
   */
  it("names which tools a plain quit puts back, and which keep working", () => {
    const row = () =>
      screen.getByRole("radio", { name: /Quit without disconnecting/ })
        .textContent ?? "";

    // Everything reverts: the macOS/Windows relay case, where the address each
    // config names lives in the process that is about to exit.
    renderChooser({ reverting: ["Claude Code", "Codex"] });
    expect(row()).toContain(
      "Gate puts Claude Code and Codex back on their own settings",
    );
    expect(row()).not.toContain("may pause");
    cleanup();

    // Mixed: one config names the relay, the other the forwarder, which is a
    // per-install difference rather than a per-tool one.
    renderChooser({ reverting: ["Codex"] });
    expect(row()).toContain("Gate puts Codex back on its own settings");
    expect(row()).toContain("Claude Code keeps working without Gate");
    cleanup();

    // Nothing reverts - Linux, where the relay is a daemon that outlives the
    // window, or an install whose configs all name the forwarder.
    renderChooser({ reverting: [] });
    expect(row()).toContain("Leave configurations pointed at Gate");
    expect(row()).toContain(
      "Claude Code and Codex keep working without Gate until Gate Connect runs again",
    );
  });

  it("says that closing the window is a different thing", () => {
    renderChooser();
    expect(
      screen.getByText(/Closing the main window is a different action/),
    ).toBeTruthy();
  });

  it("reports the selection rather than acting on it", () => {
    const onChoose = vi.fn();
    const onContinue = vi.fn();
    renderChooser({ onChoose, onContinue });
    screen.getByRole("radio", { name: /Quit without disconnecting/ }).click();
    expect(onChoose).toHaveBeenCalledWith("leave");
    // Selecting is not committing: the primary is still the only way on.
    expect(onContinue).not.toHaveBeenCalled();
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
