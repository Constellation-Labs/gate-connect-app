import { describe, it, expect } from "vitest";
import { adaptEvents, labelEntries } from "./toolEvents";

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

function envelope(events: ReturnType<typeof raw>[]) {
  return {
    generatedAt: "2026-08-19T04:20:00.000Z",
    window: { from: "2026-08-18T04:20:00.000Z", to: "2026-08-19T04:20:00.000Z" },
    toolScope: { tool: "claude-code" },
    events,
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

  /**
   * AG-951. Design asked the table for "the same model labels we used in Gate
   * currently"; those live in the gateway's catalogue, not in the id this row
   * carries, so the gateway sends the label and the client prints it.
   */
  it("prefers the gateway's display name over the id", () => {
    const view = adaptEvents(
      envelope([raw({ model: "anthropic/claude-opus-5", modelName: "Claude Opus 5" })]),
    );

    expect(view.entries[0].model).toBe("Claude Opus 5");
    // The id is kept alongside, so the cell can still say which model it was.
    expect(view.entries[0].modelId).toBe("anthropic/claude-opus-5");
  });

  it("prints the label exactly as the gateway spells it", () => {
    // Including the uneven ones. The catalogue currently holds "Claude Opus 4
    // 5" beside "Claude Opus 4.1", and names carrying a vendor prefix - and
    // the client must not tidy either, or it becomes a second opinion about a
    // field the gateway owns. Fixing the values is the gateway-side half of
    // AG-951; this pins that the client does not paper over them.
    const view = adaptEvents(
      envelope([
        raw({ model: "anthropic/claude-opus-4-5", modelName: "Claude Opus 4 5" }),
        raw({ requestId: "r2", model: "aion-labs/aion-2-0", modelName: "AionLabs: Aion-2.0" }),
      ]),
    );

    expect(view.entries[0].model).toBe("Claude Opus 4 5");
    expect(view.entries[1].model).toBe("AionLabs: Aion-2.0");
  });

  it("falls back to the id while the gateway sends no label", () => {
    // Which is every row today: the field does not exist upstream yet. An id
    // is honest and searchable, which is why it is the fallback rather than
    // something this side composes.
    const withNothing = adaptEvents(envelope([raw({ model: "openai/gpt-6-luna" })]));
    const withNull = adaptEvents(
      envelope([raw({ model: "openai/gpt-6-luna", modelName: null })]),
    );

    expect(withNothing.entries[0].model).toBe("openai/gpt-6-luna");
    expect(withNull.entries[0].model).toBe("openai/gpt-6-luna");
  });

  /**
   * The catalogue is where the dashboard's labels come from, and this app
   * already holds it for the picker. Naming rows from it is what makes the two
   * agree; the rules below are about not overriding anything more authoritative.
   */
  describe("labelEntries", () => {
    const table: Record<string, { name: string; vendor: string }> = {
      "anthropic/claude-opus-4-5": { name: "Claude Opus 4.5", vendor: "anthropic" },
      "openai/gpt-6-luna": { name: "GPT-6 Luna", vendor: "openai" },
      "gpt-6-luna": { name: "GPT-6 Luna", vendor: "openai" },
    };
    const catalogue = (id: string) => table[id];
    const entries = (o: Record<string, unknown>) => adaptEvents(envelope([raw(o)])).entries;

    it("names a row from the catalogue when the gateway sent only the id", () => {
      const [row] = labelEntries(entries({ model: "anthropic/claude-opus-4-5" }), catalogue);
      expect(row.model).toBe("Claude Opus 4.5");
      // The id is still the hover, and still what the reader can search for.
      expect(row.modelId).toBe("anthropic/claude-opus-4-5");
    });

    it("keeps the gateway's own label when it sent one", () => {
      const [row] = labelEntries(
        entries({ model: "anthropic/claude-opus-4-5", modelName: "Claude Opus 4 5" }),
        catalogue,
      );
      expect(row.model).toBe("Claude Opus 4 5");
    });

    it("keeps the id for a model the catalogue does not list", () => {
      const [row] = labelEntries(entries({ model: "aion-labs/aion-2-0" }), catalogue);
      expect(row.model).toBe("aion-labs/aion-2-0");
    });

    it("leaves an unattributed row alone", () => {
      const [row] = labelEntries(entries({ model: null }), catalogue);
      expect(row.model).toBe("Unknown model");
      expect(row.modelId).toBeNull();
    });

    it("changes nothing while the catalogue is unread", () => {
      const before = entries({ model: "openai/gpt-6-luna" });
      expect(labelEntries(before, () => undefined)).toEqual(before);
    });

    it("takes the catalogue's vendor for a bare id, so the mark can be drawn", () => {
      const [row] = labelEntries(entries({ provider: null, model: "gpt-6-luna" }), catalogue);
      expect(row.model).toBe("GPT-6 Luna");
      expect(row.vendor).toBe("openai");
      // Still not a claim about who served it.
      expect(row.provider).toBeNull();
    });

    it("does not override a vendor the id already named", () => {
      const [row] = labelEntries(entries({ model: "anthropic/claude-opus-4-5" }), catalogue);
      expect(row.vendor).toBe("anthropic");
    });

    it("fills the vendor on a row the gateway labelled, without touching the label", () => {
      const [row] = labelEntries(
        entries({ provider: null, model: "gpt-6-luna", modelName: "GPT-6 (Luna)" }),
        catalogue,
      );
      expect(row.model).toBe("GPT-6 (Luna)");
      expect(row.vendor).toBe("openai");
    });

    it("returns the same entry when the catalogue has nothing to add", () => {
      // Listed, so the lookup answers; but the gateway labelled it and the id
      // already names its vendor, so neither field changes. A memoised
      // consumer must see no change where there was none.
      const before = entries({ model: "anthropic/claude-opus-4-5", modelName: "Claude Opus 4 5" });
      expect(labelEntries(before, catalogue)[0]).toBe(before[0]);
    });
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

  it("reads an absent events array as an empty feed", () => {
    const view = adaptEvents({ ...envelope([]), events: undefined });

    expect(view.entries).toEqual([]);
  });
});

/**
 * The Type column's glyph, keyed on the gateway's own spellings.
 *
 * Found on the functional-review build: two of the five spellings the gateway
 * can send matched nothing in the glyph table, so Credential and PHI rows fell
 * through to the generic shield.
 */
describe("the Type column's categories", () => {
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
});

/**
 * The vendor mark's provider, when the gateway names none.
 *
 * Reported as "the Model column doesn't include the provider icon". The
 * component was right all along - `VendorMark` resolves `providerMarkFor` - and
 * the data was the problem: `provider` holds the pipeline's `unknown` sentinel
 * on a large share of rows, the endpoint maps that to null rather than have a
 * client draw a mark for a provider nobody has, and the cell then rendered an
 * empty spacer.
 */
describe("the model row's provider", () => {
  const rowFor = (o: Record<string, unknown>) => adaptEvents(envelope([raw(o)])).entries[0];

  it("keeps the namespace beside the provider, for the mark's fallback", () => {
    // Carried separately so `VendorMark` can try the provider first and fall
    // through to the model's own vendor - see `ActivityEntry.vendor`.
    const row = rowFor({
      provider: "openai_compatible:Marcus OpenRouter",
      model: "anthropic/claude-opus-5",
    });

    expect(row.vendor).toBe("anthropic");
    expect(row.provider).toBe("openai_compatible:Marcus OpenRouter");
  });

  it("reads the pipeline's unknown sentinel as nothing, on either column", () => {
    // The tool-events endpoint maps both to null itself; this is the net under
    // it, so a gateway that stopped could not make the row announce "unknown"
    // as the upstream or print it where a model name goes.
    const row = rowFor({ provider: "unknown", model: "unknown" });

    expect(row.provider).toBeNull();
    expect(row.model).toBe("Unknown model");
    expect(row.modelId).toBeNull();
    expect(row.vendor).toBeNull();
  });

  it("falls back to the model id's own namespace for the mark", () => {
    // The id is canonical `provider/model`, so the vendor is already on the
    // row - the same split `NewUiApp` makes in two other places.
    expect(rowFor({ provider: null, model: "anthropic/claude-opus-5" }).vendor).toBe(
      "anthropic",
    );
  });

  it("does not turn a derived vendor into a claim about who served it", () => {
    // The half the first version of this got wrong. `VendorMark` puts
    // `provider` in a `title` and an `sr-only` string, so deriving into that
    // field made the row *say* "anthropic" for a request that never reached
    // anyone - and rows with no provider are disproportionately exactly those,
    // since the gateway writes its sentinel on the paths that failed before
    // routing. The mark is decorative and sits beside the id it came from; the
    // words are a reading and stay null.
    const row = rowFor({ provider: null, model: "anthropic/claude-opus-5" });

    expect(row.provider).toBeNull();
    expect(row.vendor).toBe("anthropic");
  });

  it("names no vendor for a model id that carries none", () => {
    // A bare id names no vendor, and inferring one from the model family would
    // be a guess about whose mark to draw.
    expect(rowFor({ provider: null, model: "gpt-5" }).vendor).toBeNull();
    expect(rowFor({ provider: null, model: null }).vendor).toBeNull();
    expect(rowFor({ provider: null, model: "/leading-slash" }).vendor).toBeNull();
  });
});

describe("the category table's lookup", () => {
  it("does not hand back a prototype member", () => {
    // `constructor` and `__proto__` both survive `toLowerCase()` and come back
    // truthy off `Object.prototype`, so a bare index made `??` unreachable.
    for (const key of ["constructor", "__proto__", "valueOf"]) {
      const row = adaptEvents(envelope([raw({ securityCategory: key })])).entries[0];
      expect(row.categoryIcon).toBe("shieldCheck");
    }
  });

  it("matches a spelling whatever its case", () => {
    const row = adaptEvents(envelope([raw({ securityCategory: "PII" })])).entries[0];

    expect(row.categoryIcon).toBe("userRound");
  });
});
