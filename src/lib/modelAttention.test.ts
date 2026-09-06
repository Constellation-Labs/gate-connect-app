import { describe, expect, it } from "vitest";
import { modelAttention } from "./modelAttention";
import type { Credits, GateModel, ToolModelChoice } from "./toolModels";

/**
 * AG-592's Needs attention.
 *
 * The interesting cases are the ones where nothing should be said. Silence has
 * to mean "nothing to report", never "checked and fine" - and every input here
 * can be unknown, because a catalogue or a balance that has not been read yet is
 * the normal first paint.
 */
const CATALOGUE: GateModel[] = [
  { id: "anthropic/claude-opus-5", vendor: "anthropic", name: "Claude Opus 5", tags: ["tool-use"] },
  { id: "openai/gpt-5", vendor: "openai", name: "GPT-5", tags: ["tool-use"] },
];

const FUNDED: Credits = {
  plan: "pro",
  paygEnabled: true,
  balanceCents: 1025,
  lowBalanceThresholdCents: 500,
  billingUrl: null,
  autoTopupArmed: false,
};

const gate = (...modelIds: string[]): ToolModelChoice => ({ source: "gate", modelIds });

describe("modelAttention", () => {
  it("says nothing about a tool running its own model", () => {
    // A model remembered under App default is not in use, so a warning about it
    // would be noise about a request nobody is making.
    const a = modelAttention({
      choice: { source: "tool", modelIds: ["anthropic/gone-model"] },
      catalogue: CATALOGUE,
      credits: FUNDED,
    });
    expect(a).toBeNull();
  });

  it("says nothing about a tool with no preference at all", () => {
    expect(modelAttention({ choice: undefined, catalogue: CATALOGUE, credits: FUNDED })).toBeNull();
  });

  it("reports a chosen model the catalogue no longer offers", () => {
    const a = modelAttention({
      choice: gate("anthropic/retired-model"),
      catalogue: CATALOGUE,
      credits: FUNDED,
    });
    expect(a?.cause).toBe("model-unavailable");
    expect(a?.models).toEqual(["anthropic/retired-model"]);
    // The ticket's hard rule, in the sentence the user reads.
    expect(a?.message).toMatch(/will not pick a replacement/i);
  });

  it("stays quiet while one chosen model is still servable", () => {
    // Gate uses a model the user chose, which is not a substitution. Warning
    // here would cry wolf on a tool that is working.
    const a = modelAttention({
      choice: gate("anthropic/retired-model", "openai/gpt-5"),
      catalogue: CATALOGUE,
      credits: FUNDED,
    });
    expect(a).toBeNull();
  });

  it("reports an empty balance", () => {
    const a = modelAttention({
      choice: gate("openai/gpt-5"),
      catalogue: CATALOGUE,
      credits: { ...FUNDED, balanceCents: 0 },
    });
    expect(a?.cause).toBe("no-credits");
  });

  it("reports PAYG being off separately from having no money", () => {
    // Different cause, different fix: one is a billing setting, the other is a
    // top-up. The gateway's balance gate keeps them apart for the same reason.
    const a = modelAttention({
      choice: gate("openai/gpt-5"),
      catalogue: CATALOGUE,
      credits: { ...FUNDED, paygEnabled: false, balanceCents: 4200 },
    });
    expect(a?.cause).toBe("payg-disabled");
  });

  it("prefers the missing model over the empty balance", () => {
    // The most specific fact wins: a model that has gone stops requests however
    // well funded the account is, and "add credits" would be the wrong advice.
    const a = modelAttention({
      choice: gate("anthropic/retired-model"),
      catalogue: CATALOGUE,
      credits: { ...FUNDED, balanceCents: 0 },
    });
    expect(a?.cause).toBe("model-unavailable");
  });

  it("says nothing while the catalogue has not been read", () => {
    // An unchecked model is not a healthy model. Reporting "available" here
    // would be a claim nothing verified.
    const a = modelAttention({
      choice: gate("anthropic/retired-model"),
      catalogue: null,
      credits: FUNDED,
    });
    expect(a).toBeNull();
  });

  it("says nothing while the balance has not been read", () => {
    const a = modelAttention({
      choice: gate("openai/gpt-5"),
      catalogue: CATALOGUE,
      credits: null,
    });
    expect(a).toBeNull();
  });

  it("does not treat an unknown balance as an empty one", () => {
    // `null` cents is "nobody answered". Warning about credits here would tell a
    // funded org their tools are about to stop.
    const a = modelAttention({
      choice: gate("openai/gpt-5"),
      catalogue: CATALOGUE,
      credits: { ...FUNDED, balanceCents: null },
    });
    expect(a).toBeNull();
  });

  it("reports failing traffic, which is the only thing that sees a bad pairing", () => {
    // The catalogue says the model exists and the balance says it is affordable,
    // and the requests fail anyway - at a provider, for reasons only the traffic
    // shows. This is the case that broke Codex: a Gate model it could not be
    // served with, and an app that said nothing.
    const a = modelAttention({
      choice: gate("openai/gpt-5"),
      catalogue: CATALOGUE,
      credits: FUNDED,
      recent: [{ status: "error" }, { status: "error" }, { status: "error" }],
    });
    expect(a?.cause).toBe("requests-failing");
    expect(a?.message).toMatch(/App default/);
  });

  it("outranks the account-level causes", () => {
    // A request that HAS failed is more certain than a reason it would, and it
    // is what the user is actually experiencing.
    const a = modelAttention({
      choice: gate("openai/gpt-5"),
      catalogue: CATALOGUE,
      credits: { ...FUNDED, balanceCents: 0 },
      recent: [{ status: "error" }, { status: "error" }, { status: "error" }],
    });
    expect(a?.cause).toBe("requests-failing");
  });

  it("fires on the transition, where the older successes predate the Gate model", () => {
    // Found against a real gateway, not by reading the code. The user runs on the
    // app's own model, it works, they switch to a Gate model and everything after
    // that fails. The window still holds the successes from before the switch, so
    // a rule of "every request in the window failed" is silent at exactly the
    // moment the tool breaks - the one moment it had to speak.
    const a = modelAttention({
      choice: gate("openai/gpt-5"),
      catalogue: CATALOGUE,
      credits: FUNDED,
      recent: [
        { status: "error" },
        { status: "error" },
        { status: "error" },
        { status: "success" },
        { status: "success" },
      ],
    });
    expect(a?.cause).toBe("requests-failing");
  });

  it("clears itself as soon as the tool is answering again", () => {
    // A success at the head means Gate is serving this tool now, whatever the run
    // behind it. Leaving the warning up would outlive the problem, and a warning
    // that does that stops being read.
    const a = modelAttention({
      choice: gate("openai/gpt-5"),
      catalogue: CATALOGUE,
      credits: FUNDED,
      recent: [
        { status: "success" },
        { status: "error" },
        { status: "error" },
        { status: "error" },
      ],
    });
    expect(a).toBeNull();
  });

  it("stays quiet when only some requests failed", () => {
    // A tool that is mostly working does not need an alarm, and one that cried
    // wolf here would be ignored when it mattered.
    const a = modelAttention({
      choice: gate("openai/gpt-5"),
      catalogue: CATALOGUE,
      credits: FUNDED,
      recent: [{ status: "error" }, { status: "success" }, { status: "error" }],
    });
    expect(a).toBeNull();
  });

  it("stays quiet on a single failure", () => {
    expect(
      modelAttention({
        choice: gate("openai/gpt-5"),
        catalogue: CATALOGUE,
        credits: FUNDED,
        recent: [{ status: "error" }],
      }),
    ).toBeNull();
  });

  it("says nothing about failures while the app runs its own model", () => {
    // Whatever is failing, it is not Gate serving a model - so this module has
    // nothing to say about it and must not claim otherwise.
    const a = modelAttention({
      choice: { source: "tool", modelIds: ["openai/gpt-5"] },
      catalogue: CATALOGUE,
      credits: FUNDED,
      recent: [{ status: "error" }, { status: "error" }, { status: "error" }],
    });
    expect(a).toBeNull();
  });

  it("says nothing while the feed has not been read", () => {
    expect(
      modelAttention({ choice: gate("openai/gpt-5"), catalogue: CATALOGUE, credits: FUNDED, recent: null }),
    ).toBeNull();
  });
});
