import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent } from "@testing-library/react";
import type { Mock } from "vitest";
import { QuitConfirm } from "./QuitConfirm";

vi.mock("../lib/api", () => ({ disconnectToolsForQuit: vi.fn(), quitApp: vi.fn() }));
vi.mock("../lib/analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));
import { disconnectToolsForQuit, quitApp } from "../lib/api";
import { track, trackError } from "../lib/analytics";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function renderConfirm(tools: string[], onCancel = vi.fn()) {
  render(<QuitConfirm tools={tools} onCancel={onCancel} />);
  return onCancel;
}

describe("QuitConfirm copy", () => {
  it("names a single connected tool in the singular", () => {
    renderConfirm(["Claude Code"]);
    expect(screen.getByText(/Claude Code still routes through Gate/)).toBeTruthy();
    expect(screen.getByText(/it can’t connect until Gate Connect runs again/)).toBeTruthy();
  });

  it("lists two tools joined with and, in the plural", () => {
    renderConfirm(["Claude Code", "Codex"]);
    expect(
      screen.getByText(/Claude Code and Codex still route through Gate/),
    ).toBeTruthy();
  });

  it("uses the serial comma for three or more tools", () => {
    renderConfirm(["Claude Code", "Codex", "OpenCode"]);
    expect(screen.getByText(/Claude Code, Codex, and OpenCode still route/)).toBeTruthy();
  });

  it("names which app's next start brings the tools back", () => {
    renderConfirm(["Claude Code"]);
    // "at the next start" left the subject open, and the tool's own next launch
    // is the wrong answer. Matches the notification this choice fires.
    expect(screen.getByText(/reconnects it when Gate Connect starts again/)).toBeTruthy();
  });

  it("backs out via Cancel without touching integrations or quitting", () => {
    const onCancel = renderConfirm(["Codex"]);
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(disconnectToolsForQuit).not.toHaveBeenCalled();
    expect(quitApp).not.toHaveBeenCalled();
  });
});

describe("QuitConfirm actions", () => {
  it("disconnects the tools before quitting", async () => {
    // Empty list = every tool was put back. A non-empty one is the partial
    // teardown covered below.
    (disconnectToolsForQuit as Mock).mockResolvedValue([]);
    (quitApp as Mock).mockResolvedValue(undefined);
    renderConfirm(["Claude Code"]);
    fireEvent.click(screen.getByRole("button", { name: "Disconnect tools and quit" }));
    await vi.waitFor(() => expect(quitApp).toHaveBeenCalledTimes(1));
    expect(disconnectToolsForQuit).toHaveBeenCalledTimes(1);
    expect(track).toHaveBeenCalledWith("quit_confirmed", { integrations_disabled: true });
  });

  it("quits anyway without disconnecting anything", async () => {
    (quitApp as Mock).mockResolvedValue(undefined);
    renderConfirm(["Claude Code"]);
    fireEvent.click(screen.getByRole("button", { name: "Quit without disconnecting" }));
    await vi.waitFor(() => expect(quitApp).toHaveBeenCalledTimes(1));
    expect(disconnectToolsForQuit).not.toHaveBeenCalled();
    expect(track).toHaveBeenCalledWith("quit_confirmed", { integrations_disabled: false });
  });

  /**
   * The teardown is best-effort per tool, so it can succeed overall while leaving
   * one tool pointing at a relay that dies with this process. Quitting there
   * would strand it silently, which is the thing AG-596 rules out: Gate Connect
   * "does not claim cleanup completed".
   */
  it("names the tools it could not put back, and does not quit", async () => {
    (disconnectToolsForQuit as Mock).mockResolvedValue(["Codex"]);
    (quitApp as Mock).mockResolvedValue(undefined);
    renderConfirm(["Claude Code", "Codex"]);
    fireEvent.click(screen.getByRole("button", { name: "Disconnect tools and quit" }));

    expect(await screen.findByText(/Couldn’t put Codex back/)).toBeTruthy();
    expect(quitApp).not.toHaveBeenCalled();
    // The primary becomes a retry rather than repeating a label that already ran.
    expect(screen.getByRole("button", { name: "Try disconnecting again" })).toBeTruthy();
    // Quitting stays available: refusing to let someone quit their own app is
    // worse than letting them quit informed.
    expect(screen.getByRole("button", { name: "Quit without disconnecting" })).toBeTruthy();
  });

  it("surfaces a failed disconnect and does not quit", async () => {
    (disconnectToolsForQuit as Mock).mockRejectedValue("config file locked");
    renderConfirm(["Claude Code"]);
    fireEvent.click(screen.getByRole("button", { name: "Disconnect tools and quit" }));
    expect(await screen.findByText(/config file locked/)).toBeTruthy();
    expect(trackError).toHaveBeenCalledWith("config file locked", "quit_disable");
    expect(quitApp).not.toHaveBeenCalled();
    // The buttons come back so the user can retry or quit anyway.
    expect(
      screen.getByRole("button", { name: "Disconnect tools and quit" }),
    ).toBeTruthy();
  });
});
