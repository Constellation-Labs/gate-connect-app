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

function page(ids: string[]) {
  return JSON.stringify({
    generatedAt: "2026-08-19T04:20:00.000Z",
    window: { from: "2026-08-18T04:20:00.000Z", to: "2026-08-19T04:20:00.000Z" },
    toolScope: { tool: "claude-code", tools: ["claude-code"] },
    events: ids.map((id) => ({
      requestId: id,
      at: "2026-08-19T04:14:00.000Z",
      status: "success",
      securityAction: "flag",
      securityCategory: "pii",
      model: "claude-opus-4",
      sessionRef: "cnv_824bd2c0",
    })),
  });
}

/** Drives the hook from a component, exposing its latest value to the test. */
type HarnessProps = {
  tools: readonly string[] | null;
  installId?: string | null;
  enabled?: boolean;
};
function harness(props: HarnessProps) {
  const seen: ReturnType<typeof useToolEvents>[] = [];
  function Probe({ tools, installId, enabled }: HarnessProps) {
    seen.push(useToolEvents(enabled ?? true, tools, installId ?? null, "cred"));
    return null;
  }
  const utils = render(<Probe {...props} />);
  return { seen, rerender: (next: typeof props) => utils.rerender(<Probe {...next} />) };
}

const flush = () => act(async () => { await Promise.resolve(); });

beforeEach(() => mockCall.mockReset());
afterEach(cleanup);

describe("useToolEvents", () => {
  it("reads the first page for the tools it was given", async () => {
    mockCall.mockResolvedValue(page(["a", "b"]));
    const { seen } = harness({ tools: ["claude-code"], installId: "m-1" });
    await flush();

    expect(mockCall).toHaveBeenCalledWith(["claude-code"], "m-1");
    expect(seen.at(-1)?.view?.entries.map((e) => e.id)).toEqual(["a", "b"]);
  });

  it("drops a reply for a tool the user has already left", async () => {
    let releaseFirst: ((v: string) => void) | undefined;
    mockCall.mockImplementationOnce(
      () => new Promise<string>((resolve) => (releaseFirst = resolve)),
    );
    const { seen, rerender } = harness({ tools: ["claude-code"] });
    await flush();

    // Switch tools while the first read is still open, then let it land.
    mockCall.mockResolvedValueOnce(page(["codex-1"]));
    rerender({ tools: ["codex"] });
    await flush();
    await act(async () => {
      releaseFirst?.(page(["claude-1"]));
      await Promise.resolve();
    });

    // The stale reply must not repaint the pane under the new tool's name.
    expect(seen.at(-1)?.view?.entries.map((e) => e.id)).toEqual(["codex-1"]);
  });

  it("blanks the feed when the tool changes, rather than showing the previous one", async () => {
    mockCall.mockResolvedValueOnce(page(["a"]));
    const { seen, rerender } = harness({ tools: ["claude-code"] });
    await flush();
    expect(seen.at(-1)?.view).not.toBeNull();

    mockCall.mockImplementationOnce(() => new Promise<string>(() => {}));
    rerender({ tools: ["codex"] });

    expect(seen.at(-1)?.view).toBeNull();
  });

  it("does not read at all without a tool", async () => {
    harness({ tools: null });
    await flush();

    expect(mockCall).not.toHaveBeenCalled();
  });

  it("reads a section's names as one feed, in a stable order", async () => {
    mockCall.mockResolvedValue(page(["a"]));
    harness({ tools: ["claude-web", "claude-code", "claude-desktop"], installId: "m-1" });
    await flush();

    expect(mockCall).toHaveBeenCalledTimes(1);
    expect(mockCall).toHaveBeenCalledWith(
      ["claude-code", "claude-desktop", "claude-web"],
      "m-1",
    );
  });

  it("keys on the names, not the array, so a fresh array keeps the page", async () => {
    mockCall.mockResolvedValueOnce(page(["a"]));
    const { seen, rerender } = harness({ tools: ["claude-code", "claude-web"] });
    await flush();

    // A new array with the same names, in another order: what a caller that
    // builds the list each render hands in.
    rerender({ tools: ["claude-web", "claude-code"] });
    await flush();

    expect(mockCall).toHaveBeenCalledTimes(1);
    expect(seen.at(-1)?.view?.entries.map((e) => e.id)).toEqual(["a"]);
  });

  it("treats a change in the set as a new scope", async () => {
    mockCall.mockResolvedValueOnce(page(["a"]));
    const { seen, rerender } = harness({ tools: ["claude-code"] });
    await flush();

    mockCall.mockImplementationOnce(() => new Promise<string>(() => {}));
    rerender({ tools: ["claude-code", "claude-web"] });

    expect(seen.at(-1)?.view).toBeNull();
    expect(mockCall).toHaveBeenLastCalledWith(["claude-code", "claude-web"], undefined);
  });

  it("drops its page when disabled, so re-enabling never answers from it", async () => {
    mockCall.mockResolvedValueOnce(page(["old"]));
    const { seen, rerender } = harness({ tools: ["claude-code"] });
    await flush();
    expect(seen.at(-1)?.view?.entries.map((e) => e.id)).toEqual(["old"]);

    rerender({ tools: ["claude-code"], enabled: false });
    await flush();
    expect(seen.at(-1)?.view).toBeNull();

    mockCall.mockImplementationOnce(() => new Promise<string>(() => {}));
    rerender({ tools: ["claude-code"], enabled: true });
    await flush();
    expect(seen.at(-1)?.view).toBeNull();
  });

  it("drops a read still in flight when it is disabled", async () => {
    let land: (text: string) => void = () => {};
    mockCall.mockImplementationOnce(() => new Promise<string>((r) => (land = r)));
    const { seen, rerender } = harness({ tools: ["claude-code"] });
    await flush();

    rerender({ tools: ["claude-code"], enabled: false });
    await act(async () => {
      land(page(["late"]));
      await Promise.resolve();
    });

    expect(seen.at(-1)?.view).toBeNull();
    expect(seen.at(-1)?.loading).toBe(false);
  });

  it("does not read for an empty set", async () => {
    harness({ tools: [] });
    await flush();

    expect(mockCall).not.toHaveBeenCalled();
  });
});
