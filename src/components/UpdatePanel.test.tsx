import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { Mock } from "vitest";

vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn() }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn() }));
vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn().mockResolvedValue("1.0.0"),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    onFocusChanged: vi.fn().mockResolvedValue(() => undefined),
  }),
}));
vi.mock("../lib/api", () => ({ setUpdaterRelaunching: vi.fn() }));

import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { setUpdaterRelaunching } from "../lib/api";
import { UpdatePanel } from "./UpdatePanel";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

// The ordering is load-bearing: the relaunch mark must land before
// downloadAndInstall() starts, because on Windows the installer exits the
// app from inside that call - marking afterwards would never run.
describe("UpdatePanel install", () => {
  function trackOrder() {
    const order: string[] = [];
    (setUpdaterRelaunching as Mock).mockImplementation(async (flag: boolean) => {
      order.push(`mark:${flag}`);
    });
    (relaunch as Mock).mockImplementation(async () => {
      order.push("relaunch");
    });
    return order;
  }

  it("marks the updater relaunch before installing, then relaunches", async () => {
    const order = trackOrder();
    const downloadAndInstall = vi.fn(async () => {
      order.push("install");
    });
    (check as Mock).mockResolvedValue({ version: "9.9.9", downloadAndInstall });

    render(<UpdatePanel />);
    fireEvent.click(await screen.findByText("Install & relaunch"));

    await waitFor(() => expect(order).toEqual(["mark:true", "install", "relaunch"]));
  });

  it("resets the relaunch mark and skips relaunch when the install fails", async () => {
    const order = trackOrder();
    const downloadAndInstall = vi.fn(async () => {
      order.push("install");
      throw new Error("download failed");
    });
    (check as Mock).mockResolvedValue({ version: "9.9.9", downloadAndInstall });

    render(<UpdatePanel />);
    fireEvent.click(await screen.findByText("Install & relaunch"));

    await waitFor(() => expect(order).toEqual(["mark:true", "install", "mark:false"]));
    expect(relaunch).not.toHaveBeenCalled();
    // The panel stays up offering a retry.
    expect(await screen.findByText("Retry update")).toBeTruthy();
  });
});
