import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { defaultState } from "../e2e/backend";
import { installFakeTauri } from "../e2e/install";
import { TrayApp } from "./TrayApp";

/**
 * The failure re-check (AG-1097) on the tray's reveal edge. The reveal handler
 * already sweeps whenever a re-check is armed, so the watch must not sweep on
 * the same edge as well: every sweep probes the relay and the gateway session,
 * and this popover is revealed all day.
 */
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const sweeps = () =>
  window.__GATE_E2E__.calls.filter((c) => c.cmd === "routing_verdicts").length;

describe("a tray reveal while a row reads Not protected", () => {
  it("re-checks once, not once per listener", async () => {
    const state = defaultState();
    // Switched on with the engine down: a connection problem, which arms the
    // failure re-check.
    state.tools.find((t) => t.slug === "claude-code")!.status = { kind: "connected" };
    state.proxy.running = false;
    installFakeTauri({ ...state, windowLabel: "tray" });
    let hidden = false;
    vi.spyOn(document, "hidden", "get").mockImplementation(() => hidden);
    render(<TrayApp />);
    await screen.findByRole("switch", { name: "Claude" });
    await waitFor(() => expect(sweeps()).toBeGreaterThan(0));
    await act(() => new Promise((r) => setTimeout(r, 50)));

    hidden = true;
    act(() => {
      document.dispatchEvent(new Event("visibilitychange"));
    });
    const before = sweeps();
    hidden = false;
    act(() => {
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await waitFor(() => expect(sweeps()).toBeGreaterThan(before));
    await act(() => new Promise((r) => setTimeout(r, 50)));
    expect(sweeps()).toBe(before + 1);
  });
});
