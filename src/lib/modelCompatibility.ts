/**
 * Which Gate models an app can actually be served with (AG-590, AG-729).
 *
 * The picker offers the whole catalogue - 344 models on staging - and most of
 * them cannot serve most apps. Choosing one costs the user a prompt to find out,
 * and the failure arrives as a raw provider error about a field they have never
 * heard of. This module is what the picker asks first.
 *
 * **Two kinds of knowledge, deliberately kept apart.**
 *
 * The first comes from the gateway: each catalogue row carries `tags`, and
 * `tool-use` is load-bearing. A model without it answers a tools request with
 * `404 No endpoints found that support tool use`. That is the gateway's own
 * account of its own catalogue, and it stays fresh.
 *
 * The second is which *shape* of tool definition a model accepts. Codex sends
 * freeform tools - OpenAI's `type: "custom"` - and `openai/gpt-4o` carries
 * `tool-use` and still fails with `Missing required parameter: 'tools[0].custom'`.
 * Tool support and tool shape are different facts, and for a long time only the
 * first was reported.
 *
 * AG-729 moved the second onto the platform. The gateway now serves a per-shape
 * verdict on each catalogue row ({@link GateModel.toolShapes}), curated with a
 * date attached. This module prefers that verdict and falls back to
 * {@link FALLBACK_FREEFORM_RULES} when talking to a gateway that predates it.
 *
 * **Three states, not two.** The reason this file stopped answering yes/no is
 * that "nobody has checked" is the most common answer and it is not a "no".
 * Reporting it as one would hide every model that works but was never swept, and
 * the table would quietly shrink the catalogue as the world moved on. So a model
 * is compatible, refuted, or unverified - and the picker renders the three
 * differently rather than hiding two of them.
 */

import type { GateModel, ToolShape } from "./toolModels";

/**
 * The only things this module reads about a model.
 *
 * Narrower than `GateModel` on purpose: the picker has its own view type, and
 * asking it for a full catalogue row to answer a yes/no question would couple
 * the two for nothing.
 */
export interface ModelFacts {
  id: string;
  tags: string[];
  toolShapes?: GateModel["toolShapes"];
}

/** What an app needs from a model before Gate can serve it. */
export interface ToolNeeds {
  /** Sends tool definitions, so the model must support tools at all. */
  tools: boolean;
  /**
   * Sends OpenAI freeform (`type: "custom"`) tools.
   *
   * Codex does, for its shell tool.
   */
  freeformTools: boolean;
}

/**
 * What each app asks of a model.
 *
 * Keyed on the tool slug the preferences use. Which shape a tool sends is
 * client knowledge and deliberately stays here rather than moving to the
 * gateway: the gateway knows what models accept, the client knows what it
 * sends, and neither can answer the other's half.
 *
 * An app that is not listed is assumed to need nothing in particular: saying "no
 * models suit you" to a tool we have not studied would be a claim we have not
 * earned, and the cost of being wrong is hiding every model from a tool that
 * would have worked.
 */
const NEEDS: Record<string, ToolNeeds> = {
  // Codex sends its shell tool as a freeform tool on every request.
  codex: { tools: true, freeformTools: true },
  // Claude Code sends ordinary tool definitions.
  "claude-code": { tools: true, freeformTools: false },
};

export function needsOf(slug: string | null | undefined): ToolNeeds {
  return (slug && NEEDS[slug]) || { tools: false, freeformTools: false };
}

/** A verdict for one (model, shape) pair, from wherever it was learned. */
type Verdict = "works" | "fails" | "unknown";

/**
 * What this client knew about freeform tools before the gateway could say.
 *
 * Verified against staging on 2026-08-28: every `openai/gpt-5*` id served a
 * `type: "custom"` tool, and the families listed below refused it. It is a
 * fallback for older gateways only, and {@link toolShapeVerdict} prefers a
 * served verdict whenever there is one.
 *
 * **The `else` branch is `unknown`, not `fails`.** The regex this replaced
 * answered a bare boolean, so every model outside the GPT-5 family read as a
 * refusal, including hundreds nobody had ever tried. That was tolerable only
 * because it was the sole source of truth. Now that it is a fallback, claiming a
 * refusal it never observed would outrank the gateway's silence and hide working
 * models. Assert only what was actually seen.
 *
 * DELETE THIS once every deployment serves `tool_shapes`. It is dated so that
 * decision can be made on evidence rather than on nerve.
 */
const FALLBACK_FREEFORM_RULES: ReadonlyArray<{ prefix: string; verdict: "works" | "fails" }> = [
  { prefix: "openai/gpt-5", verdict: "works" },
  { prefix: "openai/gpt-4o", verdict: "fails" },
  { prefix: "openai/gpt-4-1", verdict: "fails" },
  { prefix: "anthropic/", verdict: "fails" },
  { prefix: "google/", verdict: "fails" },
  { prefix: "deepseek/", verdict: "fails" },
];

/**
 * What is known about this model for one tool shape.
 *
 * Precedence is the whole point: a verdict the gateway served is evidence
 * gathered against the gateway's own serving path, and it outranks anything
 * compiled into this build - including a disagreement, which is exactly the
 * case where the local table has gone stale.
 */
function toolShapeVerdict(model: ModelFacts, shape: ToolShape): Verdict {
  const served = model.toolShapes?.[shape];
  if (served) return served.verdict;

  if (shape === "freeform") {
    const rule = FALLBACK_FREEFORM_RULES.find((r) => model.id.startsWith(r.prefix));
    if (rule) return rule.verdict;
  }
  return "unknown";
}

/** Why a model is not offered, in the terms the picker needs to say it. */
export type Incompatibility =
  /** The catalogue does not list tool support for it. */
  | "no-tool-use"
  /** Tools yes, but not the freeform kind this app sends. */
  | "no-freeform-tools";

export interface Compatibility {
  ok: boolean;
  /** Set only when `ok` is false. */
  reason?: Incompatibility;
  /**
   * True when this model can be offered but nothing has actually confirmed it.
   *
   * Only ever set alongside `ok: true`. The picker shows these below a divider
   * rather than hiding them: an unverified model is a model that probably works
   * and definitely has not been shown not to.
   */
  unverified?: boolean;
}

/**
 * Can this app be served with this model?
 *
 * An empty tag list is treated as unknown, not as a denial: a catalogue row that
 * carries no tags has not said it lacks tools, and refusing every such model
 * would hide models that work because the gateway was terse about them. That
 * stance predates AG-729 and survives it unchanged.
 */
export function compatibility(model: ModelFacts, needs: ToolNeeds): Compatibility {
  let unverified = false;

  if (needs.tools) {
    // A served verdict for the ordinary function shape is a better answer than
    // the tag, because it distinguishes "declared unsupported" from "never
    // said". Fall back to the tag when the gateway offered no verdict.
    const fn = toolShapeVerdict(model, "function");
    if (fn === "fails") return { ok: false, reason: "no-tool-use" };
    if (fn === "unknown") {
      if (model.tags.length > 0 && !model.tags.includes("tool-use")) {
        return { ok: false, reason: "no-tool-use" };
      }
      if (model.tags.length === 0) unverified = true;
    }
  }

  if (needs.freeformTools) {
    const freeform = toolShapeVerdict(model, "freeform");
    if (freeform === "fails") return { ok: false, reason: "no-freeform-tools" };
    if (freeform === "unknown") unverified = true;
  }

  return unverified ? { ok: true, unverified: true } : { ok: true };
}

/** One sentence naming what would go wrong, for the app this is about. */
export function explain(reason: Incompatibility, appName: string): string {
  switch (reason) {
    case "no-tool-use":
      return `Gate does not list tool support for this model, and ${appName} sends tools with every request.`;
    case "no-freeform-tools":
      return `These models were tested and verified to reject the form ${appName} sends its tools in.`;
  }
}

/**
 * The note above the unverified section.
 *
 * Says what is true - nobody checked - rather than implying a risk nobody
 * measured. The user can pick one, and most of them work.
 */
export function unverifiedNote(appName: string): string {
  return `Not yet tested with ${appName}. These are likely to work, and Gate will tell you if one does not.`;
}
