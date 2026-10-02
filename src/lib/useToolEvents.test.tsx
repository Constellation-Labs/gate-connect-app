import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import { useToolEvents } from "./toolEvents";

vi.mock("./api", () => ({ activityToolEvents: vi.fn() }));
import { activityToolEvents } from "./api";

/**
 * The hook, not the adapter.
 *
 * `adaptEvents` is pure and covered next door. What is risky here is the state
 * machine around it: a generation ref that has to drop a reply from a scope the
 * user has already left, and a scope change having to blank the feed rather than
 * show one tool's requests under another's name. Both are the kind of thing
 * that looks right and misbehaves only under a race.
 */
const mockCall = activityToolEvents as unknown as ReturnType<typeof vi.fn>;

function page(ids: string[], nextCursor: string | null) {
  return JSON.stringify({
    generatedAt: "2026-08-19T04:20:00.000Z",
    window: { from: "2026-08-18T04:20:00.000Z", to: "2026-08-19T04:20:00.000Z" },
    toolScope: { tool: "claude-code" },
    events: ids.map((id) => ({
      requestId: id,
      at: "2026-08-19T04:14:00.000Z",
      status: "success",
      securityAction: "flag",
      securityCategory: "pii",
      model: "claude-opus-4",
      sessionRef: "cnv_824bd2c0",
    })),
    nextCursor,
  });
}

/** Drives the hook from a component, exposing its latest value to the test. */
function harness(props: { tool: string | null; installId?: string | null }) {
  const seen: ReturnType<typeof useToolEvents>[] = [];
  function Probe({ tool, installId }: { tool: string | null; installId?: string | null }) {
    seen.push(useToolEvents(true, tool, installId ?? null, "cred"));
    return null;
  }
  const utils = render(<Probe {...props} />);
  return { seen, rerender: (next: typeof props) => utils.rerender(<Probe {...next} />) };
}

const flush = () => act(async () => { await Promise.resolve(); });

beforeEach(() => mockCall.mockReset());
afterEach(cleanup);

describe("useToolEvents", () => {
  it("reads the first page for the tool it was given", async () => {
    mockCall.mockResolvedValue(page(["a", "b"], null));
    const { seen } = harness({ tool: "claude-code", installId: "m-1" });
    await flush();

    expect(mockCall).toHaveBeenCalledWith("claude-code", "m-1");
    expect(seen.at(-1)?.view?.entries.map((e) => e.id)).toEqual(["a", "b"]);
  });

  it("drops a reply for a tool the user has already left", async () => {
    let releaseFirst: ((v: string) => void) | undefined;
    mockCall.mockImplementationOnce(
      () => new Promise<string>((resolve) => (releaseFirst = resolve)),
    );
    const { seen, rerender } = harness({ tool: "claude-code" });
    await flush();

    // Switch tools while the first read is still open, then let it land.
    mockCall.mockResolvedValueOnce(page(["codex-1"], null));
    rerender({ tool: "codex" });
    await flush();
    await act(async () => {
      releaseFirst?.(page(["claude-1"], null));
      await Promise.resolve();
    });

    // The stale reply must not repaint the pane under the new tool's name.
    expect(seen.at(-1)?.view?.entries.map((e) => e.id)).toEqual(["codex-1"]);
  });

  it("blanks the feed when the tool changes, rather than showing the previous one", async () => {
    mockCall.mockResolvedValueOnce(page(["a"], null));
    const { seen, rerender } = harness({ tool: "claude-code" });
    await flush();
    expect(seen.at(-1)?.view).not.toBeNull();

    mockCall.mockImplementationOnce(() => new Promise<string>(() => {}));
    rerender({ tool: "codex" });

    expect(seen.at(-1)?.view).toBeNull();
  });

  it("does not read at all without a tool", async () => {
    harness({ tool: null });
    await flush();

    expect(mockCall).not.toHaveBeenCalled();
  });
});
