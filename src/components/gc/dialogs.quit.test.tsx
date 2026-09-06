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
      <QuitSafeToCloseDialog disconnected onClose={noop} onCancel={noop} />,
    );
    expect(
      screen.getByText(
        "Tools are disconnected and their previous settings are restored. Setup will be waiting the next time you open the app.",
      ),
    ).toBeTruthy();
  });

  it("reports the branch that touched nothing, without claiming a restore", () => {
    render(
      <QuitSafeToCloseDialog
        disconnected={false}
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

  it("closes on the primary and stays open on Cancel", () => {
    const onClose = vi.fn();
    const onCancel = vi.fn();
    render(
      <QuitSafeToCloseDialog
        disconnected
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
