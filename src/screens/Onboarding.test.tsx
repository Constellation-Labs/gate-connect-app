import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent, waitFor } from "@testing-library/react";
import type { Mock } from "vitest";

// The tour's close path is the one place where a thrown side effect can
// strand the user: Tauri prevents the native close whenever JS listens for
// close-requested, so the window only goes away if our handler resolves and
// finish() actually reaches close().
const close = vi.fn();
const onCloseRequested = vi.fn((_handler: () => unknown) => Promise.resolve(() => {}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ close, onCloseRequested }),
}));
vi.mock("@tauri-apps/api/event", () => ({ emit: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/platform")>()),
  usePlatform: () => "macos",
}));
vi.mock("../lib/analytics", () => ({ track: vi.fn() }));
vi.mock("../lib/tour", () => ({ setTourSeen: vi.fn(), TOUR_SEEN_EVENT: "gc:tour-seen" }));

import { track } from "../lib/analytics";
import { setTourSeen } from "../lib/tour";
import { Onboarding } from "./Onboarding";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

/** Walk to the last step and press Get started. */
function completeTour() {
  render(<Onboarding />);
  for (let i = 0; i < 10; i++) {
    const next = screen.queryByRole("button", { name: "Next" });
    if (!next) break;
    fireEvent.click(next);
  }
  fireEvent.click(screen.getByRole("button", { name: "Get started" }));
}

describe("Onboarding close path", () => {
  it("closes the window when the tour completes", async () => {
    completeTour();
    await waitFor(() => expect(close).toHaveBeenCalledTimes(1));
    expect(setTourSeen).toHaveBeenCalled();
  });

  it("still closes when analytics throws", async () => {
    (track as Mock).mockImplementation(() => {
      throw new Error("posthog unreachable");
    });
    completeTour();
    // Telemetry is bookkeeping; it must never hold the window open.
    await waitFor(() => expect(close).toHaveBeenCalledTimes(1));
  });

  it("still closes when the seen-flag write throws", async () => {
    (setTourSeen as Mock).mockImplementation(() => {
      throw new Error("localStorage blocked");
    });
    completeTour();
    await waitFor(() => expect(close).toHaveBeenCalledTimes(1));
  });

  it("registers a close-requested handler that never rejects", async () => {
    (track as Mock).mockImplementation(() => {
      throw new Error("posthog unreachable");
    });
    render(<Onboarding />);
    await waitFor(() => expect(onCloseRequested).toHaveBeenCalled());
    // Tauri awaits this handler and only destroys the window if it resolves.
    const handler = onCloseRequested.mock.calls[0][0];
    await expect(Promise.resolve(handler())).resolves.not.toThrow();
  });
});
