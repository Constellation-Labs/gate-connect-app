import { describe, it, expect } from "vitest";
import { adaptEvents, categoryTone } from "./toolEvents";

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

  it("calls an examined request with no category Regular, not a dash", () => {
    // AG-887. A guardrail category exists only where a guardrail fired, so
    // ordinary traffic carried none and the Type column was a dash on every
    // row. A dash reads as missing; what happened is that the request was
    // examined and nothing matched.
    const view = adaptEvents(
      envelope([raw({ securityAction: "allow", securityCategory: null })]),
    );

    expect(view.entries[0].category).toBe("Regular");
    expect(view.entries[0].categoryIcon).toBe("shieldCheck");
    // "Regular" is Connect's word, not the gateway's, so the cell explains
    // itself on hover the way the dash cell beside it always has.
    expect(view.entries[0].categoryTitle).toMatch(/no guardrail matched/);
  });

  it("adds no hover text to a category the gateway named", () => {
    // The gateway's own spelling stands alone; there is nothing to add to it.
    const view = adaptEvents(
      envelope([raw({ securityAction: "block", securityCategory: "pii" })]),
    );

    expect(view.entries[0].category).toBe("pii");
    expect(view.entries[0].categoryTitle).toBeNull();
  });

  it("keeps the dash where the gateway recorded nothing at all", () => {
    // Keyed on the ACTION: no action means the row was not examined, or is not
    // this caller's to see into. Promoting that to "Regular" would claim a
    // verdict we never got - principle 6, the same line `security` draws.
    const view = adaptEvents(
      envelope([raw({ securityAction: null, securityCategory: null })]),
    );

    expect(view.entries[0].category).toBeNull();
    expect(view.entries[0].categoryIcon).toBeNull();
  });

  it.each(["block", "flag", "redact"] as const)(
    "does not call an uncategorised %s Regular",
    (securityAction) => {
      // "Regular" is a verdict, and only `allow` is one. Staging returned
      // nothing but `allow`, so this was the untested half: a row the gateway
      // acted on but did not name would have drawn "Regular" and a shieldCheck
      // in the Type cell, two columns from a Security pill reading "blocked".
      // The dash is the honest reading - something fired, nobody said what.
      const view = adaptEvents(
        envelope([raw({ securityAction, securityCategory: null })]),
      );

      expect(view.entries[0].category).toBeNull();
      expect(view.entries[0].categoryIcon).toBeNull();
    },
  );

  it("still names the category on a row the gateway did categorise", () => {
    // The narrowing above is about the FALLBACK only: an action that fired and
    // named its category renders that category, whatever the action was.
    const view = adaptEvents(
      envelope([raw({ securityAction: "block", securityCategory: "injection" })]),
    );

    expect(view.entries[0].category).toBe("injection");
    expect(view.entries[0].categoryIcon).toBe("shieldAlert");
  });

  it("falls back to a glyph rather than none for a category it does not know", () => {
    // The frame puts a glyph in every Type cell, and the gateway's vocabulary is
    // not pinned down - so an unknown category still draws one, the way
    // `POLICY_ICONS` falls back on the Overview rows.
    const view = adaptEvents(envelope([raw({ securityCategory: "something-new" })]));

    expect(view.entries[0].category).toBe("something-new");
    expect(view.entries[0].categoryIcon).toBe("shieldCheck");
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

/**
 * The Type column's ink and its glyph, both keyed on the gateway's own
 * spellings.
 *
 * Found on the functional-review build: the column drew one ink for every
 * category where `661:16450` colours each, and two of the five spellings the
 * gateway can send matched nothing in the glyph table, so Credential and PHI
 * rows fell through to the generic shield.
 */
describe("the Type column's categories", () => {
  it("colours each category the way the frame draws it", () => {
    // Measured off `661:16450` and matched to the variables it resolves:
    // red/600, green/600, purple/600.
    expect(categoryTone("injection")).toContain("red-600");
    expect(categoryTone("pii")).toContain("green-600");
    expect(categoryTone("credential")).toContain("purple-600");
  });

  it("leaves the ink alone where no guardrail fired", () => {
    // A colour is what a guardrail firing looks like. "Regular" is the case
    // where one ran and matched nothing, and `other` is a category the frame
    // never drew.
    expect(categoryTone("Regular")).toBe("text-base-foreground");
    expect(categoryTone("other")).toBe("text-base-foreground");
    expect(categoryTone(null)).toBe("text-base-foreground");
  });

  it("knows every spelling the gateway can actually send", () => {
    // `toCategory` in `activity.controller.ts` narrows to exactly these five.
    // `credential` and `phi` used to match nothing here - the table was keyed
    // `credentials` and `pii-phi`, which are the *policy row* ids - so two of
    // the five drew the fallback shield instead of their own glyph.
    const iconFor = (c: string) =>
      adaptEvents(envelope([raw({ securityCategory: c })])).entries[0].categoryIcon;

    expect(iconFor("injection")).toBe("shieldAlert");
    expect(iconFor("pii")).toBe("userRound");
    expect(iconFor("phi")).toBe("userRound");
    expect(iconFor("credential")).toBe("key");
    // `other` has no glyph of its own and keeps the fallback, which is what
    // the frame's "a glyph in every Type cell" asks for.
    expect(iconFor("other")).toBe("shieldCheck");
  });

  it("still answers for the policy-row spellings, which are a second vocabulary", () => {
    expect(categoryTone("pii-phi")).toContain("green-600");
    expect(categoryTone("prompt-injection")).toContain("red-600");
    expect(categoryTone("credentials")).toContain("purple-600");
  });
});
