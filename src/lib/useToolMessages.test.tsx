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
function body(messages: number | null, generatedAt = "2026-09-04T09:00:00.000Z") {
  return JSON.stringify({
    generatedAt,
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

  it("drops figures when the credential changes, and re-reads for the new one", async () => {
    disk.mockResolvedValue({ "claude-code": body(1032) });
    net.mockImplementation(() => new Promise<string>(() => {}));
    const h = harness();
    await flush();
    expect(h.last().byTool.get("claude-code")?.messages).toBe(1032);

    // One org's traffic must never sit on screen under another org's name - the
    // rule every reading in this app follows.
    disk.mockResolvedValue({});
    const readsBefore = disk.mock.calls.length;
    h.rerender({ credential: "cred-b" });
    await flush();

    expect(h.last().byTool.has("claude-code")).toBe(false);
    // And the new scope is actually read. An earlier version of this test passed
    // without this line while `credential` was missing from `refresh`'s deps: the
    // figures were cleared and then nothing re-read them, so the rows sat blank
    // until the popover was reopened.
    expect(disk.mock.calls.length).toBeGreaterThan(readsBefore);
  });

  it("never applies an answer issued for a scope the user has left", async () => {
    // The race the epoch guard exists for. One request is in flight when the
    // credential changes; it resolves into a hook that has already cleared.
    //
    // Every pass's resolver is captured separately on purpose: the replacement
    // pass calls the same mock, so a single `release` variable would be
    // overwritten and the test would end up releasing the *new* request and
    // proving nothing. That is exactly how the first version of this test passed
    // against the bug.
    const releases: ((v: string) => void)[] = [];
    net.mockImplementation(() => new Promise<string>((r) => releases.push(r)));
    const h = harness({ slugs: ["claude-code"] });
    await flush();
    expect(releases).toHaveLength(1);

    h.rerender({ slugs: ["claude-code"], credential: "cred-b" });
    await flush();
    // The old scope's answer, arriving late.
    releases[0](body(999));
    await flush();

    expect(h.last().byTool.has("claude-code")).toBe(false);

    // And it did not suppress the new scope's own read either: that request is
    // live, and its answer is the one that lands. Before the guard, the late
    // answer stamped `readAt` and the correct read was skipped for the whole TTL.
    expect(releases.length).toBeGreaterThan(1);
    releases[releases.length - 1](body(4));
    await flush();
    expect(h.last().byTool.get("claude-code")?.messages).toBe(4);
  });

  it("drops a held figure when the read stops working", async () => {
    // A sign-out leaves `account.json` in place, so this hook stays enabled and
    // every fetch 401s. A number held from before that must not stay on screen
    // under an account that can no longer read it.
    //
    // The held reading is fresh so the first look leaves it alone - otherwise the
    // mount's own fetch replaces it and the test proves nothing about holding.
    const justNow = new Date(Date.now() - 1_000).toISOString();
    disk.mockResolvedValue({ "claude-code": body(1032, justNow) });
    const h = harness({ slugs: ["claude-code"] });
    await flush();
    expect(h.last().byTool.get("claude-code")?.messages).toBe(1032);

    net.mockRejectedValue("401 invalid_key");
    vi.spyOn(Date, "now").mockReturnValue(Date.now() + STALE_MS + 1);
    act(() => {
      h.last().refresh();
    });
    await flush();

    expect(h.last().byTool.has("claude-code")).toBe(false);
  });

  it("holds a place for a row it has no figure for yet", async () => {
    net.mockImplementation(() => new Promise<string>(() => {}));
    const h = harness({ slugs: ["claude-code"] });
    await flush();

    // Not a zero and not `N/A`: neither is true while we are still asking.
    expect(h.last().pending.has("claude-code")).toBe(true);
  });

  it("spares a launch-and-peek when the held reading is fresh enough", async () => {
    // `readAt` is per-session, so without seeding it from the body's own age the
    // first look after every restart re-asks for all N rows - even for readings
    // that landed seconds ago.
    const justNow = new Date(Date.now() - 1_000).toISOString();
    disk.mockResolvedValue({ "claude-code": body(50, justNow) });
    const h = harness({ slugs: ["claude-code"] });
    await flush();

    expect(h.last().byTool.get("claude-code")?.messages).toBe(50);
    expect(net).not.toHaveBeenCalled();
  });

  it("carries the reading's own age, not the time it was drawn", async () => {
    const anHourAgo = new Date(Date.UTC(2026, 8, 4, 9, 0, 0)).toISOString();
    disk.mockResolvedValue({ "claude-code": body(12, anHourAgo) });
    net.mockImplementation(() => new Promise<string>(() => {}));
    const h = harness({ slugs: ["claude-code"] });
    await flush();

    // The body's `generatedAt`, to the millisecond - not `Date.now()`. A
    // `toBeTruthy()` here would pass the exact regression this forbids, which is
    // what the first version of this test did.
    expect(h.last().byTool.get("claude-code")?.measuredAtMs).toBe(Date.parse(anHourAgo));
  });

  it("has no figure for a body whose age is not a date", async () => {
    // A figure that cannot say when it was measured has no business claiming an
    // age, and "measured Invalid Date" in a tooltip is worse than nothing.
    disk.mockResolvedValue({ "claude-code": body(12, "not-a-date") });
    net.mockImplementation(() => new Promise<string>(() => {}));
    const h = harness({ slugs: ["claude-code"] });
    await flush();

    expect(h.last().byTool.has("claude-code")).toBe(false);
  });
});
