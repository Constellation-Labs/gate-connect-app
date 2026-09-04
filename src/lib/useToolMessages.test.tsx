import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import { STALE_MS, useToolMessages } from "./toolMessages";

vi.mock("./api", () => ({
  activityOverview: vi.fn(),
  activityCachedToolOverviews: vi.fn(),
}));
import { activityCachedToolOverviews, activityOverview } from "./api";

/**
 * The bargain this hook exists to strike: a figure on every row, without a read
 * per row per look.
 *
 * What is worth pinning is what fails silently. The TTL is the whole design - a
 * refactor that dropped it would look correct on screen and quietly turn every
 * tray open into N gateway calls at a budget shared with everyone on the same
 * egress. The rest is the same honesty rule as everywhere else here: a figure is
 * a measurement or it is absent, and a declined section is not a zero.
 */
const net = activityOverview as unknown as ReturnType<typeof vi.fn>;
const disk = activityCachedToolOverviews as unknown as ReturnType<typeof vi.fn>;

/** An overview body with one messages figure in it. `messages: null` is the
 *  gateway declining that section, which must not read as zero. */
function body(messages: number | null) {
  return JSON.stringify({
    generatedAt: "2026-09-04T09:00:00.000Z",
    window: { from: "2026-09-03T09:00:00.000Z", to: "2026-09-04T09:00:00.000Z" },
    org: { orgId: "org-1", name: "Constellation Labs" },
    counters: {
      blockedOrFlagged: { state: "ok", value: 0 },
      needsReview: { state: "ok", value: 0 },
      requestsRouted:
        messages === null ? { state: "unavailable", reason: "upstream" } : { state: "ok", value: messages },
      tokensSaved: { state: "ok", fraction: 0, amount: 0, currency: "USD" },
    },
    requestsByHour: { state: "ok", buckets: [] },
    policies: { state: "ok", rows: [] },
    tokenSavings: { state: "ok", rows: [] },
  });
}

function harness(
  props: { slugs?: string[]; installId?: string | null; credential?: string } = {},
) {
  const seen: ReturnType<typeof useToolMessages>[] = [];
  function Probe({ slugs, installId, credential }: typeof props) {
    seen.push(
      useToolMessages(
        true,
        slugs ?? ["claude-code", "codex"],
        installId === undefined ? "install-7" : installId,
        credential ?? "cred-a",
      ),
    );
    return null;
  }
  const utils = render(<Probe {...props} />);
  return {
    seen,
    last: () => seen[seen.length - 1],
    rerender: (next: typeof props) => utils.rerender(<Probe {...next} />),
  };
}

const flush = () =>
  act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });

beforeEach(() => {
  net.mockReset();
  disk.mockReset();
  disk.mockResolvedValue({});
  net.mockResolvedValue(body(0));
  vi.useRealTimers();
});
afterEach(cleanup);

describe("useToolMessages", () => {
  it("opens on what is held on disk, in one read", async () => {
    disk.mockResolvedValue({ "claude-code": body(1032), codex: body(7) });
    // Held with the network still in flight, which is the state a popover opens
    // in: it must draw real figures before anything has been asked.
    net.mockImplementation(() => new Promise<string>(() => {}));
    const h = harness();
    await flush();

    expect(h.last().byTool.get("claude-code")?.messages).toBe(1032);
    expect(h.last().byTool.get("codex")?.messages).toBe(7);
    // One file read for both rows, not one per row: the file holds them all.
    expect(disk).toHaveBeenCalledTimes(1);
  });

  it("does not re-ask the gateway for a figure it just read", async () => {
    const h = harness();
    await flush();
    const first = net.mock.calls.length;
    expect(first).toBe(2);

    // A second look, well inside the window. This is the click-close-click case,
    // and the whole reason a look may refresh at all.
    act(() => {
      h.last().refresh();
    });
    await flush();

    expect(net.mock.calls.length).toBe(first);
  });

  it("re-asks once the reading has gone stale", async () => {
    const h = harness();
    await flush();
    const first = net.mock.calls.length;

    // Time moved rather than the code being asked to trust a timer: the hook
    // compares against `Date.now()`, so that is what the test moves.
    const later = Date.now() + STALE_MS + 1;
    vi.spyOn(Date, "now").mockReturnValue(later);
    act(() => {
      h.last().refresh();
    });
    await flush();

    expect(net.mock.calls.length).toBeGreaterThan(first);
  });

  it("asks one tool at a time", async () => {
    let inFlight = 0;
    let peak = 0;
    net.mockImplementation(async () => {
      inFlight += 1;
      peak = Math.max(peak, inFlight);
      await Promise.resolve();
      inFlight -= 1;
      return body(3);
    });
    harness({ slugs: ["claude-code", "codex", "opencode"] });
    await flush();

    // A look must not be a burst of N concurrent requests at a shared budget.
    expect(peak).toBe(1);
  });

  it("has no figure for a declined section, rather than a zero", async () => {
    net.mockResolvedValue(body(null));
    const h = harness({ slugs: ["claude-code"] });
    await flush();

    // `0` here would be a claim about this person's traffic that the gateway
    // explicitly refused to make.
    expect(h.last().byTool.has("claude-code")).toBe(false);
  });

  it("has no figure when the read fails", async () => {
    net.mockRejectedValue("gateway unreachable");
    const h = harness({ slugs: ["claude-code"] });
    await flush();

    expect(h.last().byTool.has("claude-code")).toBe(false);
    expect(h.last().pending.has("claude-code")).toBe(false);
  });

  it("asks nothing while the machine is unattributed", async () => {
    // A null installId means org-wide, and org-wide figures on one machine's rows
    // would be wrong in exactly the case the app exists to explain.
    harness({ installId: null });
    await flush();

    expect(net).not.toHaveBeenCalled();
    expect(disk).not.toHaveBeenCalled();
  });

  it("drops figures when the credential changes", async () => {
    disk.mockResolvedValue({ "claude-code": body(1032) });
    net.mockImplementation(() => new Promise<string>(() => {}));
    const h = harness();
    await flush();
    expect(h.last().byTool.get("claude-code")?.messages).toBe(1032);

    // One org's traffic must never sit on screen under another org's name - the
    // rule every reading in this app follows.
    disk.mockResolvedValue({});
    h.rerender({ credential: "cred-b" });
    await flush();

    expect(h.last().byTool.has("claude-code")).toBe(false);
  });

  it("carries the reading's own age, not the time it was drawn", async () => {
    disk.mockResolvedValue({ "claude-code": body(12) });
    const h = harness({ slugs: ["claude-code"] });
    await flush();

    // Whatever the local formatting, it is derived from the body's `generatedAt`
    // and not from `Date.now()` - which is what lets a row disclose that its
    // figure is held rather than live.
    expect(h.last().byTool.get("claude-code")?.measuredAt).toBeTruthy();
  });
});
