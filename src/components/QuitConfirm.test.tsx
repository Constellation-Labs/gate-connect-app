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
    expect(screen.getByText(/it can't connect until Gate Connect runs again/)).toBeTruthy();
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

  it("says disconnected tools come back at the next start", () => {
    renderConfirm(["Claude Code"]);
    expect(screen.getByText(/reconnects it at the next start/)).toBeTruthy();
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
    (disconnectToolsForQuit as Mock).mockResolvedValue(undefined);
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
    fireEvent.click(screen.getByRole("button", { name: "Quit anyway" }));
    await vi.waitFor(() => expect(quitApp).toHaveBeenCalledTimes(1));
    expect(disconnectToolsForQuit).not.toHaveBeenCalled();
    expect(track).toHaveBeenCalledWith("quit_confirmed", { integrations_disabled: false });
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
