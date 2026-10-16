import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, renderHook } from "@testing-library/react";

import type { Verdict } from "./api";
import { REOPEN_IDLE_WATCH_MS } from "./reopen";
import { FAILURE_RECHECK_MS, recheckCadence, useVerdictWatch } from "./verdictWatch";

function verdict(slug: string, state: Verdict["state"], reason: Verdict["reason"] = null): Verdict {
  return { slug, state, reason, next_action: null, route_in_use: null, requested_route: null };
}

/**
 * AG-1097: a row that read Not protected kept reading it for hours after the
 * cause was gone, because nothing re-checked a failure.
 */
describe("recheckCadence", () => {
  it("adds no probes while everything is on or off", () => {
    expect(recheckCadence([verdict("codex", "on"), verdict("hermes", "off")])).toBeNull();
    expect(recheckCadence([])).toBeNull();
  });

  it("re-checks a failure on the slower cadence", () => {
    expect(
      recheckCadence([verdict("codex", "on"), verdict("claude-code", "needs_attention", "connection_problem")]),
    ).toBe(FAILURE_RECHECK_MS);
  });

  it("keeps the reopen watch's faster cadence when a reopen is pending", () => {
    expect(
      recheckCadence([
        verdict("claude-code", "needs_attention", "connection_problem"),
        verdict("hermes", "needs_attention", "reopen_required"),
      ]),
    ).toBe(REOPEN_IDLE_WATCH_MS);
  });

  it("clears a failure inside the minute the ticket asks for", () => {
    expect(FAILURE_RECHECK_MS).toBeLessThanOrEqual(60_000);
    expect(FAILURE_RECHECK_MS).toBeGreaterThan(REOPEN_IDLE_WATCH_MS);
  });
});

describe("useVerdictWatch", () => {
  let hidden = false;
  beforeEach(() => {
    vi.useFakeTimers();
    hidden = false;
    vi.spyOn(document, "hidden", "get").mockImplementation(() => hidden);
  });
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it("does nothing without a cadence", () => {
    const refresh = vi.fn();
    renderHook(() => useVerdictWatch(null, refresh));
    vi.advanceTimersByTime(FAILURE_RECHECK_MS * 3);
    expect(refresh).not.toHaveBeenCalled();
  });

  it("re-checks a failure on its cadence and stops once it is gone", () => {
    const refresh = vi.fn();
    const { rerender } = renderHook(({ cadence }) => useVerdictWatch(cadence, refresh), {
      initialProps: { cadence: FAILURE_RECHECK_MS as number | null },
    });
    vi.advanceTimersByTime(FAILURE_RECHECK_MS * 2);
    expect(refresh).toHaveBeenCalledTimes(2);
    rerender({ cadence: null });
    vi.advanceTimersByTime(FAILURE_RECHECK_MS * 2);
    expect(refresh).toHaveBeenCalledTimes(2);
  });

  it("pauses the failure re-check while hidden and resumes it once visible", () => {
    const refresh = vi.fn();
    renderHook(() => useVerdictWatch(FAILURE_RECHECK_MS, refresh));
    hidden = true;
    vi.advanceTimersByTime(FAILURE_RECHECK_MS * 3);
    expect(refresh).not.toHaveBeenCalled();
    hidden = false;
    // The reveal itself is the shell's sweep, not this one's: both shells
    // already re-check on their own reveal edge.
    document.dispatchEvent(new Event("visibilitychange"));
    expect(refresh).not.toHaveBeenCalled();
    vi.advanceTimersByTime(FAILURE_RECHECK_MS);
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("keeps the reopen watch running while hidden, as before", () => {
    const refresh = vi.fn();
    renderHook(() => useVerdictWatch(REOPEN_IDLE_WATCH_MS, refresh));
    hidden = true;
    vi.advanceTimersByTime(REOPEN_IDLE_WATCH_MS * 2);
    expect(refresh).toHaveBeenCalledTimes(2);
  });
});
