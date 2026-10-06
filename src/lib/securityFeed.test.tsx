import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import { useSecurityFeed } from "./securityFeed";
import type { SecurityEvent } from "./api";

/** The `security-event` listener the hook registers, so a test can push to it. */
let pushEvent: ((e: { payload: SecurityEvent }) => void) | undefined;

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, handler: (e: { payload: unknown }) => void) => {
    if (name === "security-event") pushEvent = handler as typeof pushEvent;
    return Promise.resolve(() => {});
  }),
}));
vi.mock("./api", () => ({
  securityFeedRecent: vi.fn(() => Promise.resolve([])),
  securityFeedState: vi.fn(() => Promise.resolve("live")),
  securityFeedHistoryOk: vi.fn(() => Promise.resolve(true)),
}));

afterEach(cleanup);

const event = (i: number): SecurityEvent => ({
  id: `evt-${i}`,
  requestId: `req-${i}`,
  at: "2026-08-31T14:03:00Z",
  action: "block",
  category: "credential",
  tool: "claude-code",
  model: "claude-opus-4",
  provider: "anthropic",
});

/**
 * The window's buffer is capped at the backend's 200, oldest dropped first.
 *
 * Pinned here rather than in the e2e it used to live in: that test revealed the
 * whole buffer through "Load more", and the table now draws only the newest ten.
 */
describe("useSecurityFeed", () => {
  it("keeps the newest 200 events a long-running window receives", async () => {
    const seen: ReturnType<typeof useSecurityFeed>[] = [];
    function Probe() {
      seen.push(useSecurityFeed(true));
      return null;
    }
    render(<Probe />);
    await act(async () => {
      await Promise.resolve();
    });

    await act(async () => {
      for (let i = 0; i < 205; i++) pushEvent?.({ payload: event(i) });
    });

    const ids = seen.at(-1)!.events.map((e) => e.id);
    expect(ids).toHaveLength(200);
    expect(ids[0]).toBe("evt-5");
    expect(ids.at(-1)).toBe("evt-204");
  });
});
