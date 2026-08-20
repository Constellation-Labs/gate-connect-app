import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import { useActivity } from "./activity";

vi.mock("./api", () => ({
  activityOverview: vi.fn(),
  activityCachedOverview: vi.fn(),
  activityInstallations: vi.fn(),
}));
import { activityCachedOverview, activityOverview } from "./api";

/**
 * The two-read race, which is the riskiest logic in the Overview and was the only
 * part with no test.
 *
 * Four behaviours, each of which fails silently if a refactor gets it wrong: the
 * cache paints first so the pane opens on real numbers; the network answer
 * outranks it whichever order they land in; a *failed* fetch keeps the held
 * reading rather than blanking the pane; and a reply from a scope the user has
 * left never repaints.
 */
const net = activityOverview as unknown as ReturnType<typeof vi.fn>;
const disk = activityCachedOverview as unknown as ReturnType<typeof vi.fn>;

function body(orgName: string, messages: number) {
  return JSON.stringify({
    generatedAt: "2026-08-19T04:20:00.000Z",
    window: { from: "2026-08-18T04:20:00.000Z", to: "2026-08-19T04:20:00.000Z" },
    org: { orgId: "org-1", name: orgName },
    counters: {
      blockedOrFlagged: { state: "ok", value: 0 },
      needsReview: { state: "ok", value: 0 },
      requestsRouted: { state: "ok", value: messages },
      tokensSaved: { state: "ok", fraction: 0, amount: 0, currency: "USD" },
    },
    requestsByHour: { state: "ok", buckets: [] },
    policies: { state: "ok", rows: [] },
    tokenSavings: { state: "ok", rows: [] },
  });
}

/** Renders the hook and records every value it returns. */
function harness(props: { credential?: string } = {}) {
  const seen: ReturnType<typeof useActivity>[] = [];
  function Probe({ credential }: { credential?: string }) {
    seen.push(useActivity(true, null, credential ?? "cred-a"));
    return null;
  }
  const utils = render(<Probe {...props} />);
  return { seen, rerender: (next: typeof props) => utils.rerender(<Probe {...next} />) };
}

const flush = () =>
  act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });

beforeEach(() => {
  net.mockReset();
  disk.mockReset();
});
afterEach(cleanup);

describe("useActivity", () => {
  it("paints the held reading before the network answers", async () => {
    disk.mockResolvedValue(body("Held Org", 5));
    let releaseNet: ((v: string) => void) | undefined;
    net.mockImplementation(() => new Promise<string>((r) => (releaseNet = r)));

    const { seen } = harness();
    await flush();

    // This is the whole point of the second read: real numbers on the frame the
    // pane opens on, rather than a skeleton that resolves a round trip later.
    expect(seen.at(-1)?.view?.orgName).toBe("Held Org");
    expect(seen.at(-1)?.loading).toBe(true);
    releaseNet?.(body("Fresh Org", 9));
  });

  it("lets the network answer replace the held one", async () => {
    disk.mockResolvedValue(body("Held Org", 5));
    net.mockResolvedValue(body("Fresh Org", 9));

    const { seen } = harness();
    await flush();

    expect(seen.at(-1)?.view?.orgName).toBe("Fresh Org");
    expect(seen.at(-1)?.view?.stats.messages).toBe(9);
    expect(seen.at(-1)?.loading).toBe(false);
  });

  it("ignores a slow disk read that lands after the network", async () => {
    let releaseDisk: ((v: string | null) => void) | undefined;
    disk.mockImplementation(() => new Promise<string | null>((r) => (releaseDisk = r)));
    net.mockResolvedValue(body("Fresh Org", 9));

    const { seen } = harness();
    await flush();
    expect(seen.at(-1)?.view?.orgName).toBe("Fresh Org");

    await act(async () => {
      releaseDisk?.(body("Held Org", 5));
      await Promise.resolve();
    });

    // A stale body arriving late must not overwrite a current reading.
    expect(seen.at(-1)?.view?.orgName).toBe("Fresh Org");
  });

  it("keeps the held reading when the fetch fails, and reports the cause", async () => {
    disk.mockResolvedValue(body("Held Org", 5));
    net.mockRejectedValue(JSON.stringify({ code: "offline", message: "no route" }));

    const { seen } = harness();
    await flush();

    // Blanking the pane would lose the last thing the user knows actually
    // happened; the reading carries its own timestamp to say when.
    expect(seen.at(-1)?.view?.orgName).toBe("Held Org");
    expect(seen.at(-1)?.failure?.code).toBe("offline");
  });

  it("does not paper over a failed fetch with a late cache read", async () => {
    let releaseDisk: ((v: string | null) => void) | undefined;
    disk.mockImplementation(() => new Promise<string | null>((r) => (releaseDisk = r)));
    net.mockRejectedValue(JSON.stringify({ code: "offline", message: "no route" }));

    const { seen } = harness();
    await flush();
    expect(seen.at(-1)?.failure?.code).toBe("offline");

    await act(async () => {
      releaseDisk?.(body("Held Org", 5));
      await Promise.resolve();
    });

    // The failure has already been reported; a body arriving afterwards would
    // make the pane look current when it is not.
    expect(seen.at(-1)?.view).toBeNull();
    expect(seen.at(-1)?.failure?.code).toBe("offline");
  });

  it("drops a reply for a credential the user has already left", async () => {
    disk.mockResolvedValue(null);
    let releaseFirst: ((v: string) => void) | undefined;
    net.mockImplementationOnce(() => new Promise<string>((r) => (releaseFirst = r)));

    const { seen, rerender } = harness({ credential: "cred-a" });
    await flush();

    net.mockResolvedValueOnce(body("Org B", 2));
    rerender({ credential: "cred-b" });
    await flush();

    await act(async () => {
      releaseFirst?.(body("Org A", 1));
      await Promise.resolve();
    });

    // One org's figures must never sit on screen under another org's name.
    expect(seen.at(-1)?.view?.orgName).toBe("Org B");
  });

  it("blanks the view when the credential changes", async () => {
    disk.mockResolvedValue(null);
    net.mockResolvedValueOnce(body("Org A", 1));

    const { seen, rerender } = harness({ credential: "cred-a" });
    await flush();
    expect(seen.at(-1)?.view).not.toBeNull();

    net.mockImplementationOnce(() => new Promise<string>(() => {}));
    rerender({ credential: "cred-b" });

    expect(seen.at(-1)?.view).toBeNull();
  });

  it("survives an unreadable cache entry", async () => {
    disk.mockResolvedValue("not json");
    net.mockResolvedValue(body("Fresh Org", 9));

    const { seen } = harness();
    await flush();

    // A mangled entry means the pane waits for the network, which is what it did
    // before the cache existed.
    expect(seen.at(-1)?.view?.orgName).toBe("Fresh Org");
  });
});
