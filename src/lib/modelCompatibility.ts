/**
 * Which Gate models an app can actually be served with (AG-590).
 *
 * The picker offers the whole catalogue - 405 models on staging - and most of
 * them cannot serve most apps. Choosing one costs the user a prompt to find out,
 * and the failure arrives as a raw provider error about a field they have never
 * heard of. This module is what the picker asks first.
 *
 * **Two kinds of knowledge, deliberately kept apart.**
 *
 * The first comes from the gateway: each catalogue row carries `tags`, and
 * `tool-use` is load-bearing. A model without it answers a tools request with
 * `404 No endpoints found that support tool use`. That is the gateway's own
 * account of its own catalogue, it stays fresh, and it rules out 115 of 405.
 *
 * The second comes only from trying it. Codex sends *freeform* tools - OpenAI's
 * `type: "custom"` - and nothing in the catalogue says which models accept them.
 * `openai/gpt-4o` carries `tool-use` and still fails:
 * `Missing required parameter: 'tools[0].custom'`. Tested across vendors against
 * staging, the models that accept them today are the OpenAI GPT-5 family and
 * nothing else - `gpt-4-1` rejects the value outright, Anthropic wants a
 * different shape, Gemini does not understand the request at all.
 *
 * So the second kind lives in a table with its evidence attached, not in a tag
 * lookup dressed up as one. It **will** go stale - a new model will support
 * freeform tools and this file will not know - which is why nothing here hides a
 * model outright. It reports, the picker explains, and the user can still choose.
 */

/**
 * The only two things this module reads about a model.
 *
 * Narrower than `GateModel` on purpose: the picker has its own view type, and
 * asking it for a full catalogue row to answer a yes/no question would couple
 * the two for nothing.
 */
export interface ModelFacts {
  id: string;
  tags: string[];
}

/** What an app needs from a model before Gate can serve it. */
export interface ToolNeeds {
  /** Sends tool definitions, so the model must support tools at all. */
  tools: boolean;
  /**
   * Sends OpenAI freeform (`type: "custom"`) tools.
   *
   * Codex does, for its shell tool. It is not a tag the catalogue carries, so it
   * is answered by [`acceptsFreeformTools`].
   */
  freeformTools: boolean;
}

/**
 * What each app asks of a model.
 *
 * Keyed on the tool slug the preferences use. An app that is not listed is
 * assumed to need nothing in particular: saying "no models suit you" to a tool
 * we have not studied would be a claim we have not earned, and the cost of being
 * wrong is hiding every model from a tool that would have worked.
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

/**
 * Does this model accept OpenAI freeform tools?
 *
 * The empirical half, and the half that dates. Verified against staging on
 * 2026-08-28: every `openai/gpt-5*` id served a `type: "custom"` tool; the four
 * other families tried refused it. Widen this when a model is shown to work, not
 * when it looks like it should.
 */
function acceptsFreeformTools(model: ModelFacts): boolean {
  return /^openai\/gpt-5/.test(model.id);
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
}

/**
 * Can this app be served with this model?
 *
 * An empty tag list is treated as unknown, not as a denial: a catalogue row that
 * carries no tags has not said it lacks tools, and refusing every such model
 * would hide models that work because the gateway was terse about them.
 */
export function compatibility(model: ModelFacts, needs: ToolNeeds): Compatibility {
  if (needs.tools && model.tags.length > 0 && !model.tags.includes("tool-use")) {
    return { ok: false, reason: "no-tool-use" };
  }
  if (needs.freeformTools && !acceptsFreeformTools(model)) {
    return { ok: false, reason: "no-freeform-tools" };
  }
  return { ok: true };
}

/** One sentence naming what would go wrong, for the app this is about. */
export function explain(reason: Incompatibility, appName: string): string {
  switch (reason) {
    case "no-tool-use":
      return `Gate does not list tool support for this model, and ${appName} sends tools with every request.`;
    case "no-freeform-tools":
      return `${appName} sends its tools in a form only OpenAI's GPT-5 models accept. Others reject the request.`;
  }
}
