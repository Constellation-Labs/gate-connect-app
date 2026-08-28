import { describe, expect, it } from "vitest";
import { adaptCredits, adaptModels, adaptPreferences, formatCredits } from "./toolModels";
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
      data: [
        {
          id: "anthropic/claude-opus-5",
          owned_by: "anthropic",
          name: "Claude Opus 5",
          tags: ["tool-use", "vision"],
        },
      ],
    });
    expect(models).toEqual([
      {
        id: "anthropic/claude-opus-5",
        vendor: "anthropic",
        name: "Claude Opus 5",
        tags: ["tool-use", "vision"],
      },
    ]);
  });

  it("falls back to the id's own namespace for a vendor, and to the id for a name", () => {
    // Both fields are optional upstream. Neither absence is worth dropping a
    // selectable model over.
    expect(adaptModels({ data: [{ id: "mistralai/mistral-large" }] })).toEqual([
      {
        id: "mistralai/mistral-large",
        vendor: "mistralai",
        name: "mistralai/mistral-large",
        // No tags is not "no capabilities": `modelCompatibility` reads an empty
        // list as unknown rather than as a denial.
        tags: [],
      },
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

/**
 * The credits line (Figma 228:89517 draws it as "$10.25 available").
 *
 * Three answers that a careless formatter collapses into one, and the collapse
 * is expensive: this number sits beside a control that starts spending money.
 */
describe("formatCredits", () => {
  const funded = {
    plan: "pro",
    paygEnabled: true,
    balanceCents: 1025,
    lowBalanceThresholdCents: 500,
    autoTopupArmed: false,
  };

  it("formats a real balance the way the design words it", () => {
    expect(formatCredits(funded)).toBe("$10.25 available");
  });

  it("reads an unknown balance as no answer, never as zero", () => {
    // Printing $0.00 for a balance nobody reported would tell a funded org their
    // tools are about to stop. Null lets the card say N/A instead.
    expect(formatCredits({ ...funded, balanceCents: null })).toBeNull();
    expect(formatCredits(null)).toBeNull();
  });

  it("keeps a measured zero, which is the whole explanation for a failing tool", () => {
    expect(formatCredits({ ...funded, balanceCents: 0 })).toBe("$0.00 available");
  });

  it("says PAYG is off rather than showing money that cannot be spent here", () => {
    // An org can hold a balance and still have PAYG disabled, and the fix is
    // different from adding funds - the balance gate reports them as separate
    // causes for the same reason.
    expect(formatCredits({ ...funded, paygEnabled: false })).toBe("Not enabled");
  });
});

describe("adaptCredits", () => {
  it("defaults an unreadable payload to the safe reading", () => {
    // Safe here means "we do not know and PAYG is off", not "you have nothing":
    // one sends the user to look, the other tells them a falsehood about money.
    const c = adaptCredits({} as never);
    expect(c.balanceCents).toBeNull();
    expect(c.paygEnabled).toBe(false);
    // Not "free": a plan nobody named is unknown, and "Free" is the one value a
    // reader would act on - by upgrading something they may already have.
    expect(c.plan).toBeNull();
  });

  it("keeps a zero balance as a reading", () => {
    expect(adaptCredits({ balanceCents: 0, paygEnabled: true }).balanceCents).toBe(0);
  });
});
