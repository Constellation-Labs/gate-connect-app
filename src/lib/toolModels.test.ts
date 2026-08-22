import { describe, expect, it } from "vitest";
import { adaptModels, adaptPreferences } from "./toolModels";

/**
 * The adapters, which are where a careless reading turns into a claim about
 * someone's billing.
 *
 * The load-bearing case is `source`: it alone decides what Gate serves, so a row
 * whose source cannot be read is not a row. Defaulting it either way is a lie in
 * one direction or the other.
 */
describe("adaptPreferences", () => {
  it("keys by platform id, not by anything that looks like a tool slug", () => {
    const view = adaptPreferences({
      preferences: [
        { platformId: "claude-desktop", source: "gate", modelIds: ["anthropic/claude-opus-5"], updatedAt: "t" },
      ],
      firstPaidAckAt: "2026-01-02T03:04:05.000Z",
    });

    expect(view.byPlatform.get("claude-desktop")?.source).toBe("gate");
    expect(view.firstPaidAckAt).toBe("2026-01-02T03:04:05.000Z");
  });

  it("keeps a model that is remembered but not in use", () => {
    // `source: "tool"` with a model is a real state, not a contradiction: the
    // pane shows what the user would switch to. The gateway allows the pair for
    // exactly this reason.
    const view = adaptPreferences({
      preferences: [{ platformId: "codex", source: "tool", modelIds: ["openai/gpt-5"], updatedAt: "t" }],
    });

    const pref = view.byPlatform.get("codex");
    expect(pref?.source).toBe("tool");
    expect(pref?.modelIds).toEqual(["openai/gpt-5"]);
  });

  it("drops a row whose source it cannot read, rather than guessing one", () => {
    // Defaulting to "tool" would report a paid Gate model as inactive;
    // defaulting to "gate" would claim Gate is serving something it may not be.
    // Neither is a reading, so the platform reads as unconfigured.
    const view = adaptPreferences({
      preferences: [
        { platformId: "codex", source: "sideways", modelIds: ["openai/gpt-5"], updatedAt: "t" },
        { platformId: "cursor", modelIds: [], updatedAt: "t" },
      ],
    });

    expect(view.byPlatform.size).toBe(0);
  });

  it("drops a row with no platform id, which nothing could look up", () => {
    expect(adaptPreferences({ preferences: [{ source: "gate", modelIds: ["a/b"] }] }).byPlatform.size).toBe(0);
  });

  it("reads a missing acknowledgement as never, not as unknown", () => {
    // There is no third state here: the gateway returns the stamp or null, and
    // null means this org has never accepted paid use - which is what makes the
    // next switch ask.
    expect(adaptPreferences({ preferences: [] }).firstPaidAckAt).toBeNull();
    expect(adaptPreferences({}).firstPaidAckAt).toBeNull();
  });

  it("survives a payload that is not the shape at all", () => {
    const view = adaptPreferences({ preferences: "nope" as unknown });
    expect(view.byPlatform.size).toBe(0);
    expect(view.firstPaidAckAt).toBeNull();
  });

  it("keeps only the string entries of a mixed model list", () => {
    const view = adaptPreferences({
      preferences: [{ platformId: "codex", source: "gate", modelIds: ["openai/gpt-5", 42, null], updatedAt: "t" }],
    });
    expect(view.byPlatform.get("codex")?.modelIds).toEqual(["openai/gpt-5"]);
  });
});

describe("adaptModels", () => {
  it("reads the catalogue's Vercel-shaped rows", () => {
    const models = adaptModels({
      data: [{ id: "anthropic/claude-opus-5", owned_by: "anthropic", name: "Claude Opus 5" }],
    });
    expect(models).toEqual([
      { id: "anthropic/claude-opus-5", vendor: "anthropic", name: "Claude Opus 5" },
    ]);
  });

  it("falls back to the id's own namespace for a vendor, and to the id for a name", () => {
    // Both fields are optional upstream. Neither absence is worth dropping a
    // selectable model over.
    expect(adaptModels({ data: [{ id: "mistralai/mistral-large" }] })).toEqual([
      { id: "mistralai/mistral-large", vendor: "mistralai", name: "mistralai/mistral-large" },
    ]);
  });

  it("drops a row with no id, which could not be selected", () => {
    expect(adaptModels({ data: [{ owned_by: "anthropic", name: "Nameless" }, { id: "" }] })).toEqual([]);
  });

  it("reads an empty catalogue as an empty catalogue", () => {
    // A real answer, not a failure: a gateway with no platform provider accounts
    // offers nothing of its own. The picker says so in words.
    expect(adaptModels({ data: [] })).toEqual([]);
    expect(adaptModels({})).toEqual([]);
  });
});
