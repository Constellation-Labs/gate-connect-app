import { describe, expect, it } from "vitest";
import { compatibility, explain, needsOf } from "./modelCompatibility";
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
    expect(compatibility(model("openai/gpt-4o"), needs).ok).toBe(true);
  });

  it("treats no tags at all as unknown rather than as a denial", () => {
    // A terse catalogue row has not said the model lacks tools. Reading silence
    // as "no" would hide models that work.
    expect(compatibility(model("some/model", []), needs).ok).toBe(true);
  });
});

describe("freeform tools, which it does not", () => {
  const needs = needsOf("codex");

  it("accepts the GPT-5 family, which is what served Codex", () => {
    for (const id of [
      "openai/gpt-5",
      "openai/gpt-5-1",
      "openai/gpt-5-1-codex",
      "openai/gpt-5-3-codex",
      "openai/gpt-5-6-terra",
    ]) {
      expect(compatibility(model(id), needs).ok, id).toBe(true);
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

  it("does not let a lookalike id through", () => {
    // `openai/gpt-50` is not the GPT-5 family, and a loose prefix would say it
    // was. Nothing in the catalogue is named this today; the point is that the
    // rule is a rule rather than a substring.
    expect(compatibility(model("openai/gpt-4-5"), needs).ok).toBe(false);
    expect(compatibility(model("openrouter/openai-gpt-5"), needs).ok).toBe(false);
  });

  it("says nothing about freeform tools to an app that sends none", () => {
    // Claude Code is served fine by models that reject Codex's shape.
    expect(compatibility(model("anthropic/claude-haiku-4-5"), needsOf("claude-code")).ok).toBe(
      true,
    );
  });
});

describe("what the picker will say", () => {
  it("names the app, because the limit is about that app and not the model", () => {
    expect(explain("no-freeform-tools", "Codex")).toMatch(/Codex/);
    expect(explain("no-tool-use", "Codex")).toMatch(/Codex/);
  });
});
