import { describe, it, expect } from "vitest";
import { adaptEvents } from "./toolEvents";

/**
 * What a row in the tool feed is allowed to say.
 *
 * Two rules meet here and neither is obvious from the row on screen. A withheld
 * security action is not `allow` - it means the caller may not see that row's
 * detail, because security detail is self-only for every role - and a missing
 * model is not a model called "unknown". Both would read as ordinary values if
 * this adapter passed them through.
 */
function raw(overrides: Record<string, unknown> = {}) {
  return {
    requestId: "req-1",
    at: "2026-08-19T04:14:00.000Z",
    status: "success" as const,
    securityAction: "flag" as const,
    securityCategory: "pii",
    model: "claude-opus-4",
    sessionRef: "cnv_824bd2c0",
    ...overrides,
  };
}

function envelope(events: ReturnType<typeof raw>[], nextCursor: string | null = null) {
  return {
    generatedAt: "2026-08-19T04:20:00.000Z",
    window: { from: "2026-08-18T04:20:00.000Z", to: "2026-08-19T04:20:00.000Z" },
    toolScope: { tool: "claude-code" },
    events,
    nextCursor,
  };
}

describe("adaptEvents", () => {
  it("translates the gateway's verbs into the pane's pill labels", () => {
    const view = adaptEvents(
      envelope([
        raw({ requestId: "a", securityAction: "block" }),
        raw({ requestId: "b", securityAction: "redact" }),
        raw({ requestId: "c", securityAction: "flag" }),
        raw({ requestId: "d", securityAction: "allow" }),
      ]),
    );

    // The gateway records what the criterion did; the design's pills say what
    // happened to the request.
    expect(view.entries.map((e) => e.security)).toEqual(["blocked", "redacted", "flagged", "allow"]);
  });

  it("leaves a withheld security action null rather than reading it as allow", () => {
    const view = adaptEvents(envelope([raw({ securityAction: null })]));

    // Null is "not visible to you". Rendering it as `allow` would report a
    // colleague's blocked request as permitted.
    expect(view.entries[0].security).toBeNull();
  });

  it("says a model was not attributed rather than naming one", () => {
    const view = adaptEvents(envelope([raw({ model: null })]));

    expect(view.entries[0].model).toBe("Unknown model");
    expect(view.entries[0].model).not.toContain("unknown");
  });

  it("reads no title from the payload at all", () => {
    // Guards against reviving the conversation title, which could only come from
    // the user's own prompt text.
    const view = adaptEvents(envelope([raw({ title: "Update our data-model.md" })]));

    expect(JSON.stringify(view.entries[0])).not.toContain("data-model");
  });

  it("timestamps a row to the second, with its date", () => {
    const view = adaptEvents(envelope([raw({ at: "2026-06-06T00:50:51.000Z" })]));

    // Seconds because an agent sends several requests a minute, and the date
    // because a 24-hour window straddles midnight (Figma 116:30951).
    expect(view.entries[0].time).toMatch(/^[A-Z][a-z]{2} \d{1,2}, \d{2}:\d{2}:\d{2}$/);
  });

  it("carries the cursor through, and reports its absence", () => {
    expect(adaptEvents(envelope([raw()], "b3Vy")).nextCursor).toBe("b3Vy");
    expect(adaptEvents(envelope([raw()])).nextCursor).toBeNull();
  });

  it("reads an absent events array as an empty feed", () => {
    const view = adaptEvents({ ...envelope([]), events: undefined });

    expect(view.entries).toEqual([]);
  });
});
