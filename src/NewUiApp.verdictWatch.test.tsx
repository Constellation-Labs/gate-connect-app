import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { defaultState } from "../e2e/backend";
import { installFakeTauri } from "../e2e/install";
import { NewUiApp } from "./NewUiApp";
import { FAILURE_RECHECK_MS } from "./lib/verdictWatch";

/**
 * The failure re-check (AG-1097) against a routing write in flight. The write
 * paints its tools and their verdicts together once it settles; a timed sweep
 * landing in between paints the new verdict beside the old switch, which is
 * the in-between state `paintSnapshot` and `runCascade` exist to keep off
 * screen.
 */
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const sweeps = () =>
  window.__GATE_E2E__.calls.filter((c) => c.cmd === "routing_verdicts").length;

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

describe("the failure re-check during a routing write", () => {
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
    installFakeTauri(state);
    const release = hold("connect_tool");
    render(<NewUiApp />);

    const [railRow] = await screen.findAllByRole("button", { name: /^OpenCode/ });
    fireEvent.click(railRow);
    fireEvent.click(await screen.findByRole("switch", { name: "Route OpenCode" }));
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

    // The write's own settle re-reads, and the watch resumes after it.
    await act(async () => release());
    await waitFor(() => expect(sweeps()).toBeGreaterThan(before));
  });
});
