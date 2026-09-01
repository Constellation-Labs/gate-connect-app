import { describe, expect, it } from "vitest";
import { compatibility, explain, needsOf, refutedLabel, unverifiedLabel } from "./modelCompatibility";
import type { GateModel } from "./toolModels";

/**
 * Every expectation here was taken from a live staging request, not from
 * reading a provider's documentation. The failures this guards against are the
 * ones a user pays a prompt to discover.
 */
const model = (id: string, tags: string[] = ["tool-use"]): GateModel => ({
  id,
  vendor: id.split("/")[0],
  name: id,
  tags,
});

/** A row as a gateway serving AG-729's `tool_shapes` would send it. */
const served = (
  id: string,
  toolShapes: GateModel["toolShapes"],
  tags: string[] = ["tool-use"],
): GateModel => ({ ...model(id, tags), toolShapes });

describe("what each app needs", () => {
  it("knows Codex sends freeform tools and Claude Code does not", () => {
    expect(needsOf("codex")).toEqual({ tools: true, freeformTools: true });
    expect(needsOf("claude-code")).toEqual({ tools: true, freeformTools: false });
  });

  it("asks nothing of an app it has not studied", () => {
    // Claiming a tool needs something we never checked would hide every model
    // from it. Being wrong in this direction costs a failed request; being wrong
    // in the other costs the feature entirely.
    expect(needsOf("opencode")).toEqual({ tools: false, freeformTools: false });
    expect(needsOf(null)).toEqual({ tools: false, freeformTools: false });
  });
});

describe("tool support, which the catalogue does report", () => {
  const needs = needsOf("claude-code");

  it("refuses a model the gateway does not tag for tools", () => {
    // Staging answers this one `404 No endpoints found that support tool use`.
    expect(compatibility(model("openai/gpt-3-5-turbo-instruct", ["vision"]), needs)).toEqual({
      ok: false,
      reason: "no-tool-use",
    });
  });

  it("accepts a tagged model cleanly, with nothing left unverified", () => {
    expect(compatibility(model("openai/gpt-4o"), needs)).toEqual({ ok: true });
  });

  it("treats no tags at all as unknown rather than as a denial", () => {
    // A terse catalogue row has not said the model lacks tools. Reading silence
    // as "no" would hide models that work. It is offered, but marked unverified
    // rather than presented as confirmed.
    expect(compatibility(model("some/model", []), needs)).toEqual({ ok: true, unverified: true });
  });
});

describe("freeform tools, answered by the served verdict when there is one", () => {
  const needs = needsOf("codex");

  it("accepts a model the gateway verified", () => {
    const m = served("vendor/anything", { freeform: { verdict: "works", checked: "2026-08-28" } });
    expect(compatibility(m, needs)).toEqual({ ok: true });
  });

  it("refuses a model the gateway refuted", () => {
    const m = served("vendor/anything", { freeform: { verdict: "fails", checked: "2026-08-28" } });
    expect(compatibility(m, needs)).toEqual({ ok: false, reason: "no-freeform-tools" });
  });

  it("offers a model the gateway had no opinion on, marked unverified", () => {
    // The state most of the catalogue is in. Offering it is the point: it has
    // not been shown to fail.
    expect(compatibility(served("vendor/anything", { function: { verdict: "works" } }), needs)).toEqual({
      ok: true,
      unverified: true,
    });
  });

  it("coerces a verdict it does not recognise into unverified, never a refusal", () => {
    // `adaptModels` already maps an unknown string to "unknown"; this asserts
    // the consequence, which is that a future fourth verdict degrades to "no
    // opinion" rather than to a denial.
    const m = served("openai/gpt-4o", { freeform: { verdict: "unknown" } });
    expect(compatibility(m, needs)).toEqual({ ok: true, unverified: true });
  });
});

describe("verdict precedence: the gateway outranks the built-in table", () => {
  const needs = needsOf("codex");

  it("lets a served verdict overturn the local fallback", () => {
    // The case the fallback exists to survive: gpt-4o is hard-coded as failing
    // here, and if the platform ever verifies it, this build must believe the
    // platform rather than itself. Otherwise the table can never be outgrown.
    const m = served("openai/gpt-4o", { freeform: { verdict: "works", checked: "2027-01-01" } });
    expect(compatibility(m, needs)).toEqual({ ok: true });
  });

  it("lets a served refusal overturn a local pass", () => {
    const m = served("openai/gpt-5", { freeform: { verdict: "fails", checked: "2027-01-01" } });
    expect(compatibility(m, needs)).toEqual({ ok: false, reason: "no-freeform-tools" });
  });
});

describe("the fallback, for a gateway that predates tool_shapes", () => {
  const needs = needsOf("codex");

  it("accepts the GPT-5 family, which is what served Codex", () => {
    for (const id of [
      "openai/gpt-5",
      "openai/gpt-5-1",
      "openai/gpt-5-1-codex",
      "openai/gpt-5-3-codex",
      "openai/gpt-5-6-terra",
    ]) {
      expect(compatibility(model(id), needs), id).toEqual({ ok: true });
    }
  });

  it("refuses gpt-4o, which carries tool-use and still fails", () => {
    // The case that cost a real prompt: `tool-use` is present, so the first
    // check passes, and the request still dies on
    // `Missing required parameter: 'tools[0].custom'`.
    expect(compatibility(model("openai/gpt-4o"), needs)).toEqual({
      ok: false,
      reason: "no-freeform-tools",
    });
  });

  it("refuses the other families that were tried and refused it", () => {
    // gpt-4-1: "Invalid value: 'custom'". Anthropic wants `custom.input_schema`.
    // Gemini does not understand the Responses body. DeepSeek 422s.
    for (const id of [
      "openai/gpt-4-1",
      "anthropic/claude-haiku-4-5",
      "google/gemini-2-5-flash",
      "deepseek/deepseek-chat",
    ]) {
      expect(compatibility(model(id), needs).ok, id).toBe(false);
    }
  });

  it("calls everything else UNVERIFIED, not refused", () => {
    // The behaviour change AG-729 turns on, and the reason the fallback is safe
    // to keep. The regex this replaced answered a bare boolean, so every one of
    // these read as a refusal despite nobody ever having tried them. A model
    // nobody swept is offered, below the divider.
    for (const id of ["mistralai/mistral-large", "qwen/qwen3-max", "x-ai/grok-4", "openai/gpt-4-5"]) {
      expect(compatibility(model(id), needs), id).toEqual({ ok: true, unverified: true });
    }
  });

  it("does not let a lookalike id inherit the GPT-5 verdict", () => {
    // A different vendor path is a different model, and it was refused when it
    // was tried. It is now unverified rather than refused, because this build
    // has no evidence of its own about that id.
    expect(compatibility(model("openrouter/openai-gpt-5"), needs).unverified).toBe(true);
  });

  it("says nothing about freeform tools to an app that sends none", () => {
    // Claude Code is served fine by models that reject Codex's shape.
    expect(compatibility(model("anthropic/claude-haiku-4-5"), needsOf("claude-code"))).toEqual({
      ok: true,
    });
  });
});

describe("the function shape, when the gateway reports it", () => {
  const needs = needsOf("claude-code");

  it("believes an explicit refusal over a tag", () => {
    // A declared `tools: false` is the gateway saying no. It outranks a tag
    // list that happens to carry tool-use.
    const m = served("some/model", { function: { verdict: "fails" } }, ["tool-use"]);
    expect(compatibility(m, needs)).toEqual({ ok: false, reason: "no-tool-use" });
  });

  it("believes a verdict over an empty tag list, and stops calling it unverified", () => {
    // Empty tags alone mean unknown. A served `works` is a real answer, so the
    // model is offered as confirmed rather than below the divider.
    const m = served("some/model", { function: { verdict: "works" } }, []);
    expect(compatibility(m, needs)).toEqual({ ok: true });
  });
});

describe("what the picker will say", () => {
  it("names the app, because the limit is about that app and not the model", () => {
    expect(explain("no-freeform-tools", "Codex")).toMatch(/Codex/);
    expect(explain("no-tool-use", "Codex")).toMatch(/Codex/);
    expect(unverifiedLabel("Codex")).toMatch(/Codex/);
    expect(refutedLabel("Codex")).toMatch(/Codex/);
  });

  it("does not call an untested model unavailable, because it is selectable", () => {
    // The section heading labels rows the user can pick and that mostly work.
    // Saying "unavailable" would state the opposite of what the list does, and
    // would collapse the distinction between "nobody checked" and "measured
    // failing" that the rest of this module exists to keep apart.
    expect(unverifiedLabel("Codex")).toBe("Not tested with Codex");
    expect(unverifiedLabel("Codex")).not.toMatch(/unavailable|incompatible|cannot/i);
  });

  it("keeps the headings short, because they label a collapsed row", () => {
    // They sit in an accordion header beside a count, not in a paragraph.
    for (const s of [unverifiedLabel("Claude Code"), refutedLabel("Claude Code")]) {
      expect(s.length).toBeLessThan(40);
    }
  });

  it("says the refused models were actually tested, rather than blaming a family", () => {
    // The old copy named the GPT-5 family, which stops being true the moment
    // the table grows. The claim that survives is what was measured.
    expect(explain("no-freeform-tools", "Codex")).toMatch(/verified to reject/);
  });

  it("uses no em dash anywhere, per the house copy rule", () => {
    for (const s of [
      explain("no-tool-use", "Codex"),
      explain("no-freeform-tools", "Codex"),
      unverifiedLabel("Codex"),
      refutedLabel("Codex"),
    ]) {
      expect(s).not.toContain("—");
    }
  });
});
