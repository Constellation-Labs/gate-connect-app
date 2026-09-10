import { describe, expect, it } from "vitest";
import { compatibility, explain, isPinned, knownGoodFor, needsOf } from "./modelCompatibility";
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

  it("accepts a tagged model", () => {
    expect(compatibility(model("openai/gpt-4o"), needs)).toEqual({ ok: true });
  });

  it("treats no tags at all as unknown rather than as a denial", () => {
    // A terse catalogue row has not said the model lacks tools. Reading silence
    // as "no" would hide models that work, so the model is offered.
    expect(compatibility(model("some/model", []), needs)).toEqual({ ok: true });
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

  it("offers a model the gateway had no opinion on", () => {
    // The state most of the catalogue is in, and the reason silence is not a
    // denial: it has not been shown to fail.
    expect(compatibility(served("vendor/anything", { function: { verdict: "works" } }), needs)).toEqual({
      ok: true,
    });
  });

  it("coerces a verdict it does not recognise into silence, never a refusal", () => {
    // `adaptModels` already maps an unknown string to "unknown"; this asserts
    // the consequence, which is that a future fourth verdict degrades to "no
    // opinion" rather than to a denial.
    const m = served("openai/gpt-4o", { freeform: { verdict: "unknown" } });
    expect(compatibility(m, needs)).toEqual({ ok: true });
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
      expect(compatibility(model(id), needs), id).toEqual({ ok: true });
    }
  });

  it("does not let a lookalike id inherit the GPT-5 verdict", () => {
    // A different vendor path is a different model, and it was refused when it
    // was tried. It is offered rather than refused, because this build has no
    // evidence of its own about that id.
    expect(compatibility(model("openrouter/openai-gpt-5"), needs).ok).toBe(true);
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

  it("offers a model on a served works, even with no tags at all", () => {
    const m = served("some/model", { function: { verdict: "works" } }, []);
    expect(compatibility(m, needs)).toEqual({ ok: true });
  });
});

describe("what the picker will say", () => {
  it("names the app, because the limit is about that app and not the model", () => {
    expect(explain("no-freeform-tools", "Codex")).toMatch(/Codex/);
    expect(explain("no-tool-use", "Codex")).toMatch(/Codex/);
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
    ]) {
      expect(s).not.toContain("—");
    }
  });
});



describe("the dev pin list", () => {
  it("floats the models a developer has actually run", () => {
    const pinned = knownGoodFor("codex");
    expect(pinned.length).toBeGreaterThan(0);
    expect(isPinned(model("openai/gpt-5.6-terra"), pinned)).toBe(true);
  });

  it("covers both published spellings of that generation", () => {
    // The catalogue carries `gpt-5.6-terra` and `gpt-5-6-terra` as separate
    // ids, and the config on a real machine used the dashed one. A rule written
    // for one spelling silently matches none of the other.
    const pinned = knownGoodFor("codex");
    for (const id of ["openai/gpt-5.6-terra", "openai/gpt-5-6-terra"]) {
      expect(isPinned(model(id), pinned), id).toBe(true);
    }
  });

  it("does not pin the older GPT-5 generations, which real traffic refuted", () => {
    // They accept Codex's freeform tool and still 400 on
    // `reasoning.context: "all_turns"`. Pinning them would put a model that
    // fails every request at the top of a developer's list.
    const pinned = knownGoodFor("codex");
    for (const id of ["openai/gpt-5", "openai/gpt-5-1", "openai/gpt-5-3-codex"]) {
      expect(isPinned(model(id), pinned), id).toBe(false);
    }
  });

  it("uses exact ids, so no entry widens by accident", () => {
    for (const rule of knownGoodFor("codex")) {
      expect(rule.match.endsWith("*"), rule.match).toBe(false);
    }
  });

  it("carries a date and a traceable note on every entry", () => {
    for (const slug of ["codex", "claude-code"]) {
      for (const rule of knownGoodFor(slug)) {
        expect(rule.checked, rule.match).toMatch(/^\d{4}-\d{2}-\d{2}$/);
        expect(rule.note.length, rule.match).toBeGreaterThan(20);
      }
    }
  });

  it("is empty for a tool nobody has run, which is a fine answer", () => {
    // An empty list means the picker is simply not reordered. Nothing is
    // hidden and nothing is claimed.
    expect(knownGoodFor("claude-code")).toEqual([]);
    expect(knownGoodFor("opencode")).toEqual([]);
    expect(knownGoodFor(null)).toEqual([]);
  });

  it("decides nothing about compatibility", () => {
    // The separation that keeps this safe to ship: a pinned model and an
    // unpinned one get identical verdicts. The list only reorders.
    const codex = needsOf("codex");
    expect(compatibility(model("openai/gpt-5.6-terra"), codex)).toEqual(
      compatibility(model("qwen/qwen3-max"), codex),
    );
  });
});
