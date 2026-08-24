import { describe, expect, it } from "vitest";
import { adaptModels, adaptPreferences } from "./toolModels";
import type { ToolModels } from "./api";

/**
 * The adapters, which are where a careless reading turns into a claim about
 * someone's billing.
 *
 * The load-bearing case is `source`: it alone decides what Gate serves, so an
 * entry whose source cannot be read is not an entry. Defaulting it either way is
 * a lie in one direction or the other.
 */
function payload(over: Partial<ToolModels> = {}): ToolModels {
  return { tools: {}, paid_ack_unix: null, ...over };
}

describe("adaptPreferences", () => {
  it("keys by tool slug, which is what the pane already knows", () => {
    const view = adaptPreferences(
      payload({
        tools: { "claude-code": { source: "gate", model_ids: ["anthropic/claude-opus-5"] } },
        paid_ack_unix: 1767330245,
      }),
    );

    expect(view.byTool.get("claude-code")?.source).toBe("gate");
    expect(view.paidAckUnix).toBe(1767330245);
  });

  it("keeps a model that is remembered but not in use", () => {
    // `source: "tool"` with a model is a real state, not a contradiction: the
    // pane shows what the user would switch to.
    const view = adaptPreferences(
      payload({ tools: { codex: { source: "tool", model_ids: ["openai/gpt-5"] } } }),
    );

    const pref = view.byTool.get("codex");
    expect(pref?.source).toBe("tool");
    expect(pref?.modelIds).toEqual(["openai/gpt-5"]);
  });

  it("drops an entry whose source it cannot read, rather than guessing one", () => {
    // Defaulting to "tool" would report a paid Gate model as inactive;
    // defaulting to "gate" would claim Gate is serving something it may not be.
    const view = adaptPreferences(
      payload({
        tools: {
          codex: { source: "sideways", model_ids: ["openai/gpt-5"] },
          cursor: { model_ids: [] },
        } as unknown as ToolModels["tools"],
      }),
    );

    expect(view.byTool.size).toBe(0);
  });

  it("reads a tool with no entry as absent, not as an error", () => {
    // Absent IS the answer: that tool picks its own model, which is the default.
    const view = adaptPreferences(payload({ tools: { codex: { source: "gate", model_ids: ["a/b"] } } }));
    expect(view.byTool.has("claude-code")).toBe(false);
  });

  it("reads a missing acknowledgement as never, not as unknown", () => {
    expect(adaptPreferences(payload()).paidAckUnix).toBeNull();
  });

  it("survives a payload that is not the shape at all", () => {
    const view = adaptPreferences({ tools: "nope" } as unknown as ToolModels);
    expect(view.byTool.size).toBe(0);
    expect(view.paidAckUnix).toBeNull();
  });

  it("keeps only the string entries of a mixed model list", () => {
    const view = adaptPreferences(
      payload({
        tools: { codex: { source: "gate", model_ids: ["openai/gpt-5", 42, null] } },
      } as unknown as Partial<ToolModels>),
    );
    expect(view.byTool.get("codex")?.modelIds).toEqual(["openai/gpt-5"]);
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
