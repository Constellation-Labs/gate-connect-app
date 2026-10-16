import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { defaultState } from "../e2e/backend";
import { installFakeTauri } from "../e2e/install";
import { TrayApp } from "./TrayApp";
import { FAILURE_RECHECK_MS } from "./lib/verdictWatch";

/**
 * The failure re-check (AG-1097) on the tray's reveal edge. The reveal handler
 * already sweeps whenever a re-check is armed, so the watch must not sweep on
 * the same edge as well: every sweep probes the relay and the gateway session,
 * and this popover is revealed all day.
 */
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.useRealTimers();
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

/** Hold `cmd`'s answer until the test releases it. The call still reaches the
 *  fake backend, so its state has already moved. */
function hold(cmd: string) {
  let release: (v: unknown) => void = () => undefined;
  const internals = (window as any).__TAURI_INTERNALS__;
  const original = internals.invoke;
  internals.invoke = (c: string, args: Record<string, unknown> = {}) => {
    const recorded = original(c, args);
    if (c !== cmd) return recorded;
    return new Promise((resolve) => {
      release = () => resolve(recorded);
    });
  };
  return () => release(undefined);
}

describe("the tray's failure re-check during a routing write", () => {
  it("does not sweep until the write has settled", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true, toFake: ["setInterval", "clearInterval"] });
    const state = defaultState();
    state.proxy.running = true;
    state.proxy.port = 45980;
    state.proxy.ca_trusted = true;
    // A drifted Claude Code reads Not protected, which arms the re-check.
    state.tools.find((t) => t.slug === "claude-code")!.status = {
      kind: "drifted",
      reason: "base URL changed",
    };
    installFakeTauri({ ...state, windowLabel: "tray" });
    const release = hold("connect_tool");
    render(<TrayApp />);

    fireEvent.click(await screen.findByRole("switch", { name: "OpenCode" }));
    // OpenCode and the terminal route together, so the switch asks first.
    fireEvent.click(await screen.findByRole("button", { name: "Turn on" }));
    await waitFor(() =>
      expect(window.__GATE_E2E__.calls.some((c) => c.cmd === "connect_tool")).toBe(true),
    );
    await act(() => new Promise((r) => setTimeout(r, 50)));

    const before = sweeps();
    act(() => {
      vi.advanceTimersByTime(FAILURE_RECHECK_MS * 2);
    });
    await act(() => new Promise((r) => setTimeout(r, 50)));
    expect(sweeps()).toBe(before);

    await act(async () => release());
    await waitFor(() => expect(sweeps()).toBeGreaterThan(before));
  });
});
