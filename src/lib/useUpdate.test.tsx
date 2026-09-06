import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Mock } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import { useUpdate } from "./useUpdate";

vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn() }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn() }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: vi.fn() }));
vi.mock("./api", () => ({ setUpdaterRelaunching: vi.fn() }));
vi.mock("./analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));

import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";
import { setUpdaterRelaunching } from "./api";

function harness() {
  const api: { current: ReturnType<typeof useUpdate> | null } = { current: null };
  function Probe() {
    api.current = useUpdate();
    return null;
  }
  render(<Probe />);
  return api;
}

/** An `Update` handle that records the order its phases run in. */
function fakeUpdate(order: string[], over: { download?: () => Promise<void>; install?: () => Promise<void> } = {}) {
  return {
    version: "2.0.0",
    download: over.download ?? (async () => void order.push("download")),
    install: over.install ?? (async () => void order.push("install")),
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  (getVersion as Mock).mockResolvedValue("1.0.0");
  (setUpdaterRelaunching as Mock).mockResolvedValue(undefined);
  (relaunch as Mock).mockResolvedValue(undefined);
  (check as Mock).mockResolvedValue(null);
});
afterEach(cleanup);

describe("useUpdate: checking", () => {
  it("offers an update the endpoint reports", async () => {
    (check as Mock).mockResolvedValue(fakeUpdate([]));
    const api = harness();

    await act(async () => {
      await api.current!.checkNow();
    });

    expect(api.current!.available).toEqual({ version: "2.0.0" });
  });

  it("stays silent when a background check fails", async () => {
    // Offline, or an unreachable endpoint, is not worth interrupting anyone
    // about.
    (check as Mock).mockRejectedValue(new Error("offline"));
    const api = harness();

    await act(async () => {
      await api.current!.checkNow();
    });

    expect(api.current!.available).toBeNull();
    expect(api.current!.outcome).toBe("idle");
  });

  it("reports back when the user asked", async () => {
    // Silence on a button the user just pressed reads as broken.
    const api = harness();

    await act(async () => {
      await api.current!.checkNow(true);
    });
    expect(api.current!.outcome).toBe("up-to-date");

    (check as Mock).mockRejectedValue(new Error("offline"));
    await act(async () => {
      await api.current!.checkNow(true);
    });
    expect(api.current!.outcome).toBe("failed");
  });

  it("clears a stale offer when the update is gone", async () => {
    (check as Mock).mockResolvedValue(fakeUpdate([]));
    const api = harness();
    await act(async () => {
      await api.current!.checkNow();
    });
    expect(api.current!.available).not.toBeNull();

    (check as Mock).mockResolvedValue(null);
    await act(async () => {
      await api.current!.checkNow();
    });

    expect(api.current!.available).toBeNull();
  });
});

describe("useUpdate: installing", () => {
  /**
   * The ordering this pins is load-bearing, and neither half is guessable:
   *
   * - the relaunch mark lands *after* the download, because quitting mid-download
   *   is a genuine user exit and a set mark would make the exit handler skip
   *   clearing the routing intent and completing a deferred launch-at-login
   *   opt-out, with no relaunch coming to redo them;
   * - and *before* install(), because on Windows the installer exits the app
   *   from inside that call, so marking afterwards would never run.
   */
  it("marks the relaunch between the download and the install", async () => {
    const order: string[] = [];
    (setUpdaterRelaunching as Mock).mockImplementation(async (flag: boolean) => {
      order.push(`mark:${flag}`);
    });
    (relaunch as Mock).mockImplementation(async () => void order.push("relaunch"));
    (check as Mock).mockResolvedValue(fakeUpdate(order));
    const api = harness();

    await act(async () => {
      await api.current!.checkNow();
    });
    await act(async () => {
      await api.current!.install();
    });

    expect(order).toEqual(["download", "mark:true", "install", "relaunch"]);
  });

  it("does not mark a relaunch that will not happen", async () => {
    // A failed download means no install and no restart, so the exit handler
    // must keep doing its cleanup.
    const order: string[] = [];
    (check as Mock).mockResolvedValue(
      fakeUpdate(order, { download: async () => { throw new Error("network"); } }),
    );
    const api = harness();

    await act(async () => {
      await api.current!.checkNow();
    });
    await act(async () => {
      await api.current!.install();
    });

    expect(setUpdaterRelaunching).not.toHaveBeenCalled();
    expect(api.current!.failed).toBe(true);
    expect(api.current!.installing).toBe(false);
  });

  it("takes the mark back off when the install fails", async () => {
    // Otherwise the next genuine quit is mistaken for an updater relaunch.
    const order: string[] = [];
    (check as Mock).mockResolvedValue(
      fakeUpdate(order, { install: async () => { throw new Error("bad package"); } }),
    );
    const api = harness();

    await act(async () => {
      await api.current!.checkNow();
    });
    await act(async () => {
      await api.current!.install();
    });

    expect((setUpdaterRelaunching as Mock).mock.calls.map(([f]) => f)).toEqual([true, false]);
    expect(api.current!.failed).toBe(true);
    expect(relaunch).not.toHaveBeenCalled();
  });

  it("does nothing without an update to install", async () => {
    const api = harness();

    await act(async () => {
      await api.current!.install();
    });

    expect(setUpdaterRelaunching).not.toHaveBeenCalled();
  });

  it("ignores a second press while one install is running", async () => {
    let release: (() => void) | undefined;
    const order: string[] = [];
    (check as Mock).mockResolvedValue(
      fakeUpdate(order, {
        download: () => new Promise<void>((r) => { release = () => r(); }),
      }),
    );
    const api = harness();
    await act(async () => {
      await api.current!.checkNow();
    });

    await act(async () => {
      void api.current!.install();
    });
    await act(async () => {
      await api.current!.install();
    });

    expect(api.current!.installing).toBe(true);
    await act(async () => {
      release?.();
    });
  });
});
