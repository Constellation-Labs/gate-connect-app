import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { TeardownLeftBehindDialog } from "./dialogs";

/**
 * The report a Disconnect, Reset or sign-out raises when a tool is still
 * pointing at Gate afterwards. Its copy agrees in number with the list, and it
 * has exactly two ways out: try again, or close.
 */

const noop = () => {};

afterEach(cleanup);

describe("the teardown left-behind report", () => {
  it("names one tool in the singular", () => {
    render(<TeardownLeftBehindDialog tools={["Codex"]} onRetry={noop} onCancel={noop} />);

    expect(screen.getByRole("heading", { name: "One tool stayed on Gate" })).toBeTruthy();
    expect(screen.getByText(/Couldn’t put Codex back on its own settings\. It still points/)).toBeTruthy();
  });

  it("names several tools in the plural", () => {
    render(
      <TeardownLeftBehindDialog
        tools={["Claude Code", "Codex"]}
        onRetry={noop}
        onCancel={noop}
      />,
    );

    expect(screen.getByRole("heading", { name: "Some tools stayed on Gate" })).toBeTruthy();
    expect(
      screen.getByText(/Couldn’t put Claude Code and Codex back on their own settings\. They still point/),
    ).toBeTruthy();
  });

  it("offers Try again and Close, and nothing to do with quitting", () => {
    const onRetry = vi.fn();
    const onCancel = vi.fn();
    render(<TeardownLeftBehindDialog tools={["Codex"]} onRetry={onRetry} onCancel={onCancel} />);

    screen.getByRole("button", { name: "Try again" }).click();
    screen.getByRole("button", { name: "Close" }).click();
    expect(onRetry).toHaveBeenCalledTimes(1);
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("button", { name: /quit/i })).toBeNull();
  });
});
