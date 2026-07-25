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

// The ordering is load-bearing: the relaunch mark must land after the
// download (quitting mid-download is a genuine user exit that must keep the
// exit-time cleanup) but before install() starts, because on Windows the
// installer exits the app from inside that call - marking afterwards would
// never run.
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

  it("downloads, then marks the updater relaunch, installs, and relaunches", async () => {
    const order = trackOrder();
    const download = vi.fn(async () => {
      order.push("download");
    });
    const install = vi.fn(async () => {
      order.push("install");
    });
    (check as Mock).mockResolvedValue({ version: "9.9.9", download, install });

    render(<UpdatePanel />);
    fireEvent.click(await screen.findByText("Install & relaunch"));

    await waitFor(() => expect(order).toEqual(["download", "mark:true", "install", "relaunch"]));
  });

  it("never marks the relaunch when the download fails", async () => {
    const order = trackOrder();
    const download = vi.fn(async () => {
      order.push("download");
      throw new Error("download failed");
    });
    const install = vi.fn();
    (check as Mock).mockResolvedValue({ version: "9.9.9", download, install });

    render(<UpdatePanel />);
    fireEvent.click(await screen.findByText("Install & relaunch"));

    await waitFor(() => expect(order).toEqual(["download"]));
    expect(setUpdaterRelaunching).not.toHaveBeenCalled();
    expect(install).not.toHaveBeenCalled();
    expect(relaunch).not.toHaveBeenCalled();
    // The panel stays up offering a retry.
    expect(await screen.findByText("Retry update")).toBeTruthy();
  });

  it("resets the relaunch mark and skips relaunch when the install fails", async () => {
    const order = trackOrder();
    const download = vi.fn(async () => {
      order.push("download");
    });
    const install = vi.fn(async () => {
      order.push("install");
      throw new Error("install failed");
    });
    (check as Mock).mockResolvedValue({ version: "9.9.9", download, install });

    render(<UpdatePanel />);
    fireEvent.click(await screen.findByText("Install & relaunch"));

    await waitFor(() =>
      expect(order).toEqual(["download", "mark:true", "install", "mark:false"]),
    );
    expect(relaunch).not.toHaveBeenCalled();
    // The panel stays up offering a retry.
    expect(await screen.findByText("Retry update")).toBeTruthy();
  });
});
