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
    provider: "anthropic",
    sessionRef: "824bd2c0-4123",
    conversationTitle: "Update our data-model.md",
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
    // Guards the specific leak: the gateway's own sentinel arriving as a lowercase
    // string and being printed where a model name goes. `not.toContain("unknown")`
    // used to sit here and passed only because `toContain` is case-sensitive,
    // which read as asserting the opposite of what the value says.
    expect(view.entries[0].model).not.toBe("unknown");
  });

  it("carries the conversation title the gateway sent", () => {
    // Superseded a test that asserted the opposite. Figma 272:3286 restored this
    // column and product accepted that the label is the user's own prompt; the
    // gateway gates it per row so a colleague's never arrives here.
    const view = adaptEvents(envelope([raw({ conversationTitle: "Update our data-model.md" })]));

    expect(view.entries[0].title).toBe("Update our data-model.md");
  });

  it("leaves the title null when the gateway sent none", () => {
    // No session, a placeholder name, or a row this caller may not see into. The
    // row does not distinguish them: all three mean nothing to show.
    const view = adaptEvents(envelope([raw({ conversationTitle: null })]));

    expect(view.entries[0].title).toBeNull();
  });

  it("carries the security category and picks its glyph", () => {
    // The frame's Type column. `pii` is the one spelling a fixture evidences, and
    // it takes the `Icon / UserRound` the frames draw for PII on both surfaces.
    const view = adaptEvents(envelope([raw({ securityCategory: "pii" })]));

    expect(view.entries[0].category).toBe("pii");
    expect(view.entries[0].categoryIcon).toBe("userRound");
  });

  it("falls back to a glyph rather than none for a category it does not know", () => {
    // The frame puts a glyph in every Type cell, and the gateway's vocabulary is
    // not pinned down - so an unknown category still draws one, the way
    // `POLICY_ICONS` falls back on the Overview rows.
    const view = adaptEvents(envelope([raw({ securityCategory: "something-new" })]));

    expect(view.entries[0].category).toBe("something-new");
    expect(view.entries[0].categoryIcon).toBe("shieldCheck");
  });

  it("leaves the category null when the gateway named none", () => {
    const view = adaptEvents(envelope([raw({ securityCategory: null })]));

    expect(view.entries[0].category).toBeNull();
    expect(view.entries[0].categoryIcon).toBeNull();
  });

  it("carries the provider for the vendor mark", () => {
    expect(adaptEvents(envelope([raw({ provider: "anthropic" })])).entries[0].provider).toBe(
      "anthropic",
    );
  });

  it("timestamps a row to the second, with its date", () => {
    const at = "2026-06-06T00:50:51.000Z";
    const view = adaptEvents(envelope([raw({ at })]));
    const time = view.entries[0].time;

    // Asserts the two properties the design asks for - a date, and seconds - and
    // not the format. `eventTime` goes through `toLocale*`, so pinning "Jun 6,
    // 00:50:51" would pass on CI and fail for anyone whose machine is not English
    // and UTC, which is a test failing on a fact about the developer.
    const d = new Date(at);
    expect(time).toContain(d.toLocaleDateString([], { month: "short", day: "numeric" }));
    expect(time).toContain(
      d.toLocaleTimeString([], {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
        hour12: false,
      }),
    );
    // Seconds specifically: an agent sends several requests a minute, and without
    // them four rows read as the same instant (Figma 116:30951).
    expect(time).toMatch(/\d{2}:\d{2}:\d{2}/);
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
