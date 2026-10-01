import { describe, expect, it } from "vitest";
import {
  adaptCredits,
  adaptModels,
  adaptPreferences,
  formatCredits,
  formatPlan,
  leftGateModelsNotice,
  modelLabelsFor,
  stepForChoice,
} from "./toolModels";
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

describe("adaptPreferences: what each tool's config says", () => {
  it("reads an applied config and the model it starts the tool on", () => {
    const view = adaptPreferences(
      payload({
        configured: {
          codex: {
            state: "applied",
            model: "openai/gpt-5",
            left_gate_models: false,
            left_to_model: null,
          },
        },
      }),
    );

    expect(view.configured.get("codex")).toEqual({
      state: "applied",
      model: "openai/gpt-5",
      leftGateModels: false,
      leftToModel: null,
      problem: null,
    });
  });

  it("carries a tool moved off Gate models from inside itself, with where it went", () => {
    const view = adaptPreferences(
      payload({
        configured: {
          codex: {
            state: "not_applied",
            model: null,
            left_gate_models: true,
            left_to_model: "gpt-6-sol",
          },
          hermes: { state: "not_applied", model: null, left_gate_models: true, left_to_model: null },
        },
      }),
    );

    expect(view.configured.get("codex")?.leftGateModels).toBe(true);
    expect(view.configured.get("codex")?.leftToModel).toBe("gpt-6-sol");
    // Moved, but to nothing the config names: reported without a model.
    expect(view.configured.get("hermes")?.leftGateModels).toBe(true);
    expect(view.configured.get("hermes")?.leftToModel).toBeNull();
  });

  it("does not report a destination unless the tool actually left", () => {
    // A stray `left_to_model` beside `left_gate_models: false` is not a report.
    const view = adaptPreferences(
      payload({
        configured: {
          codex: { state: "applied", model: "a/b", left_gate_models: false, left_to_model: "x" },
        },
      }),
    );
    expect(view.configured.get("codex")?.leftToModel).toBeNull();
  });

  it("drops an entry whose state it cannot read, rather than guessing one", () => {
    const view = adaptPreferences(
      payload({
        configured: {
          codex: { state: "sideways", model: "a/b", left_gate_models: true, left_to_model: null },
          hermes: null,
        },
      } as unknown as Partial<ToolModels>),
    );
    expect(view.configured.size).toBe(0);
  });

  it("reads a payload from a binary that predates the field as nothing reported", () => {
    // Older builds send no `configured` at all, and a malformed one is the same.
    expect(adaptPreferences(payload()).configured.size).toBe(0);
    expect(
      adaptPreferences({ ...payload(), configured: "nope" } as unknown as ToolModels).configured
        .size,
    ).toBe(0);
  });

  it("reads a left_gate_models that is not literally true as not left", () => {
    const view = adaptPreferences(
      payload({
        configured: {
          codex: { state: "not_applied", model: null, left_gate_models: "yes", left_to_model: "m" },
        },
      } as unknown as Partial<ToolModels>),
    );
    expect(view.configured.get("codex")?.leftGateModels).toBe(false);
  });
});

describe("leftGateModelsNotice", () => {
  it("names the app and the model it moved to", () => {
    expect(leftGateModelsNotice("Codex", "gpt-6-sol")).toBe(
      "You switched Codex to gpt-6-sol in Codex, so it is back on App default.",
    );
  });

  it("does not invent a model when the config names none", () => {
    expect(leftGateModelsNotice("Hermes", null)).toBe(
      "You switched Hermes to another model in Hermes, so it is back on App default.",
    );
  });
});

describe("modelLabelsFor", () => {
  const labels = modelLabelsFor(
    adaptModels({
      data: [
        { id: "anthropic/claude-opus-5", owned_by: "anthropic", name: "Claude Opus 5" },
        // No name: `adaptModels` falls back to the id, so the lookup does too.
        { id: "openai/gpt-6-luna", owned_by: "openai" },
        // Two vendors, one native spelling.
        { id: "meta-llama/llama-4-scout", owned_by: "meta-llama", name: "Llama 4 Scout" },
        { id: "together/llama-4-scout", owned_by: "together", name: "Llama 4 Scout (Together)" },
        // The free catalogue's alias for a paid row.
        { id: "constellation/claude-opus-5", owned_by: "constellation", name: "constellation/claude-opus-5" },
        // Ambiguous dated, unique undated.
        { id: "a/dated-20260101", owned_by: "a", name: "A dated" },
        { id: "b/dated-20260101", owned_by: "b", name: "B dated" },
        { id: "c/dated", owned_by: "c", name: "C" },
      ],
    }),
  );

  it("answers a canonical id with the catalogue's name and vendor", () => {
    expect(labels("anthropic/claude-opus-5")).toEqual({ name: "Claude Opus 5", vendor: "anthropic" });
    expect(labels("openai/gpt-6-luna")).toEqual({ name: "openai/gpt-6-luna", vendor: "openai" });
    expect(labels("missing/model")).toBeUndefined();
  });

  it("answers the provider's own spelling, which is the canonical id without its vendor", () => {
    // What the security feed carries: `resolved_model` on Anthropic Direct.
    expect(labels("claude-opus-5")).toEqual({ name: "Claude Opus 5", vendor: "anthropic" });
    // Dated, as Anthropic also spells them.
    expect(labels("claude-opus-5-20260514")).toEqual({ name: "Claude Opus 5", vendor: "anthropic" });
  });

  it("does not name a native spelling two vendors share", () => {
    // Naming it would be choosing a vendor, which is the guess this refuses.
    expect(labels("llama-4-scout")).toBeUndefined();
  });

  it("does not count the free catalogue's alias as a second vendor", () => {
    // `constellation/claude-opus-5` shares the native spelling of the Anthropic
    // row; without this the headline case stayed an id on any free model.
    expect(labels("claude-opus-5")).toEqual({ name: "Claude Opus 5", vendor: "anthropic" });
    // The alias is still answered by its own id.
    expect(labels("constellation/claude-opus-5")?.vendor).toBe("constellation");
  });

  it("keeps an ambiguous dated spelling an id rather than trying it undated", () => {
    expect(labels("dated-20260101")).toBeUndefined();
    expect(labels("dated")).toEqual({ name: "C", vendor: "c" });
  });

  it("undoes only Anthropic's date, not other providers' suffixes", () => {
    expect(labels("claude-opus-5-2026-05-14")).toBeUndefined();
    expect(labels("claude-opus-5-001")).toBeUndefined();
    expect(labels("claude-opus-5@001")).toBeUndefined();
  });

  it("does not treat a namespaced id it does not list as a native spelling", () => {
    expect(labels("bedrock/claude-opus-5")).toBeUndefined();
  });

  it("does not answer a wrapped id", () => {
    // Bedrock's region and version wrapping is the gateway's to undo, by
    // sending the canonical id on the event.
    expect(labels("us.anthropic.claude-opus-5-20260514-v1:0")).toBeUndefined();
  });

  it("finds nothing for an unread catalogue", () => {
    expect(modelLabelsFor(null)("anthropic/claude-opus-5")).toBeUndefined();
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

  describe("tool_shapes (AG-729)", () => {
    const one = (tool_shapes: unknown) =>
      adaptModels({ data: [{ id: "openai/gpt-5", tags: ["tool-use"], tool_shapes }] })[0]!;

    it("reads a served verdict, keeping the evidence date", () => {
      // The date is what lets a surface judge how stale a verdict is, so it
      // survives the boundary rather than being flattened away.
      expect(one({ freeform: { verdict: "works", checked: "2026-08-28", source: "probe" } }).toolShapes).toEqual({
        freeform: { verdict: "works", checked: "2026-08-28" },
      });
    });

    it("reads both shapes independently", () => {
      expect(
        one({ function: { verdict: "works" }, freeform: { verdict: "fails", checked: "2026-08-28" } }).toolShapes,
      ).toEqual({
        function: { verdict: "works" },
        freeform: { verdict: "fails", checked: "2026-08-28" },
      });
    });

    it("leaves the field off entirely for an older gateway that sends none", () => {
      // Absence is the wire spelling of unknown, and `modelCompatibility` has a
      // dated local fallback for exactly this case.
      expect(one(undefined).toolShapes).toBeUndefined();
      expect(one({}).toolShapes).toBeUndefined();
    });

    it("coerces an unrecognised verdict to unknown rather than dropping it", () => {
      // A gateway that grows a fourth verdict must degrade to "no opinion",
      // never to a denial.
      expect(one({ freeform: { verdict: "probably" } }).toolShapes).toEqual({
        freeform: { verdict: "unknown" },
      });
    });

    it("ignores a non-string checked date instead of carrying it through", () => {
      expect(one({ freeform: { verdict: "works", checked: 20260828 } }).toolShapes).toEqual({
        freeform: { verdict: "works" },
      });
    });

    it("drops a malformed field, never the model", () => {
      // The id is what a preference stores. Losing the row would make a saved
      // choice look unavailable and offer to remove it.
      for (const bad of ["nonsense", 42, [], null, { freeform: "works" }]) {
        const m = one(bad);
        expect(m.id, JSON.stringify(bad)).toBe("openai/gpt-5");
        expect(m.toolShapes, JSON.stringify(bad)).toBeUndefined();
      }
    });

    it("keeps a well-formed shape while discarding a malformed sibling", () => {
      expect(one({ function: null, freeform: { verdict: "works" } }).toolShapes).toEqual({
        freeform: { verdict: "works" },
      });
    });
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
    billingUrl: null,
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

  describe("the billing destination (AG-729)", () => {
    it("reads the URL the gateway named", () => {
      expect(adaptCredits({ billing: { manageUrl: "https://dashboard.example.com/billing" } }).billingUrl).toBe(
        "https://dashboard.example.com/billing",
      );
    });

    it("reads a gateway that named none as null, which draws no control", () => {
      // Every gateway answered this way until AG-729, so it is the common path
      // rather than an edge case, and it must stay indistinguishable from today.
      expect(adaptCredits({}).billingUrl).toBeNull();
      expect(adaptCredits({ billing: { manageUrl: null } } as never).billingUrl).toBeNull();
      expect(adaptCredits({ billing: {} } as never).billingUrl).toBeNull();
    });

    it("refuses anything that is not an http(s) URL", () => {
      // This value is handed to the system opener. A response is not somewhere
      // to take an arbitrary string and open it.
      for (const bad of ["javascript:alert(1)", "file:///etc/passwd", "not a url", "", 42, {}, []]) {
        expect(adaptCredits({ billing: { manageUrl: bad } } as never).billingUrl, String(bad)).toBeNull();
      }
    });

    it("survives a malformed billing block without losing the rest of the reading", () => {
      const c = adaptCredits({ balanceCents: 1025, paygEnabled: true, billing: "nonsense" } as never);
      expect(c.billingUrl).toBeNull();
      expect(c.balanceCents).toBe(1025);
    });
  });
});

describe("formatPlan", () => {
  it("says Pro for the paid plan, as the dashboard does", () => {
    // The gateway reports `paid`; the dashboard renders "Pro" in its sidebar,
    // its billing page and its emails. Connect said "Paid", so one account was
    // described by two products in two words - which is what AG-879 asks us to
    // stop doing.
    expect(formatPlan("paid")).toBe("Pro");
  });

  it("says Free for the free plan", () => {
    expect(formatPlan("free")).toBe("Free");
  });

  it("passes through a plan the gateway grows later", () => {
    // Better an unfamiliar plan name than none: it is still the user's plan,
    // and hiding it would read as though they had no plan at all.
    expect(formatPlan("enterprise")).toBe("Enterprise");
  });

  it("keeps an unreported plan null rather than defaulting to Free", () => {
    // "Free" is the one value a reader acts on, by upgrading something they may
    // already have upgraded.
    expect(formatPlan(null)).toBeNull();
  });
});

describe("stepForChoice", () => {
  const set = ["openai/gpt-5-6-sol", "openai/gpt-5-6-luna", "openai/gpt-5-2"];

  it("keeps the whole set when switching back to App default", () => {
    // It kept only the first, so a round trip through App default dropped the
    // rest and the Gate radio then named one model where three were enabled.
    expect(stepForChoice("app", set)).toEqual({ kind: "remember", modelIds: set });
  });

  it("switches to Gate with the whole set", () => {
    expect(stepForChoice("gate", set)).toEqual({ kind: "activate", modelIds: set });
  });

  it("asks for a model first when none is enabled", () => {
    expect(stepForChoice("gate", [])).toEqual({ kind: "pick" });
  });

  it("opens the picker on a remembered set over the limit, which the backend refuses", () => {
    const five = ["a/1", "a/2", "a/3", "a/4", "a/5"];
    expect(stepForChoice("gate", five)).toEqual({ kind: "pick" });
    expect(stepForChoice("gate", five.slice(0, 4))).toEqual({
      kind: "activate",
      modelIds: five.slice(0, 4),
    });
    // App default still keeps all of it.
    expect(stepForChoice("app", five)).toEqual({ kind: "remember", modelIds: five });
  });

  it("remembers nothing when nothing was chosen", () => {
    expect(stepForChoice("app", [])).toEqual({ kind: "remember", modelIds: [] });
  });
});

describe("configured problems", () => {
  it("carries a problem the card must show, and nothing when there is none", () => {
    const view = adaptPreferences({
      tools: {},
      paid_ack_unix: null,
      configured: {
        codex: {
          state: "drifted",
          model: null,
          left_gate_models: false,
          left_to_model: null,
          problem: "Codex could not be put back on its own model: disk full",
        },
        hermes: { state: "applied", model: "a/b", left_gate_models: false, left_to_model: null },
      },
    });
    expect(view.configured.get("codex")?.problem).toMatch(/could not be put back/);
    expect(view.configured.get("hermes")?.problem).toBeNull();
  });
});

