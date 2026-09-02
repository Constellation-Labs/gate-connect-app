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
 * **Only refusals are asserted.** "Nobody has checked" is the most common
 * answer and it is not a "no". Reporting it as one would hide every model that
 * works but was never swept, and the local table would quietly shrink the
 * catalogue as the world moved on. So this answers what can be shown to be
 * false and offers everything else, which leaves filtering where it belongs: on
 * the catalogue, not in a hand-maintained list in the client.
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

/** A model somebody has actually run this tool against, with the date. */
export interface KnownGoodRule {
  /** Exact public id, or a family written with a trailing `*`. */
  match: string;
  /** ISO date the pairing was last confirmed by hand. */
  checked: string;
  /** What was actually done. An entry nobody can trace is not evidence. */
  note: string;
}

/**
 * Model and tool pairings a developer has actually run, per tool.
 *
 * **This is a development aid and it is inert in a shipped build.** It changes
 * the ORDER of the picker and nothing else: entries are floated to the top so
 * whoever is testing reaches a model they know works without scrolling a
 * 344-model catalogue. It grants nothing, hides nothing, and no verdict depends
 * on it. See {@link pinnedModels} for the gate.

 * Deliberately not a product claim. An earlier revision let this table decide
 * which models the picker recommended, which made a hand-maintained list into a
 * user-facing guarantee that only a person could keep true. Filtering is the
 * catalogue's job and this is a shortcut for the person testing it.
 *
 * Hardcoded on purpose. A file read at runtime would need a schema, a parse and
 * a decision about what a malformed one means, all to serve a convenience that
 * never reaches a user.
 */
const KNOWN_GOOD: Record<string, readonly KnownGoodRule[]> = {
  codex: [
    // Exact ids, not a `gpt-5.6*` prefix. The generation is published under two
    // spellings (`gpt-5.6-terra` and `gpt-5-6-terra`) and a prefix on either one
    // silently misses the other, which is how the first draft of this entry
    // matched none of the ids actually configured on this machine. A prefix
    // would also sweep in the `-pro` variants, which nobody has run.
    {
      match: "openai/gpt-5.6-terra",
      checked: "2026-09-01",
      note: "Logged a successful Codex request end to end through Gate. The generation older models fail against; see the note on the sibling entries.",
    },
    {
      match: "openai/gpt-5-6-terra",
      checked: "2026-09-01",
      note: "Same model as openai/gpt-5.6-terra, published under the dashed spelling. Both are live in the catalogue.",
    },
    {
      match: "openai/gpt-5.6-sol",
      checked: "2026-09-01",
      note: "Same generation as gpt-5.6-terra and configured alongside it. Confirm or drop if it was never actually driven.",
    },
    {
      match: "openai/gpt-5-6-sol",
      checked: "2026-09-01",
      note: "Dashed spelling of openai/gpt-5.6-sol.",
    },
    {
      match: "openai/gpt-5.6-luna",
      checked: "2026-09-01",
      note: "Same generation as gpt-5.6-terra and configured alongside it. Confirm or drop if it was never actually driven.",
    },
    {
      match: "openai/gpt-5-6-luna",
      checked: "2026-09-01",
      note: "Dashed spelling of openai/gpt-5.6-luna.",
    },
  ],
  // Deliberately empty until somebody runs it. Claude Code sends ordinary
  // function tools, which most of the catalogue accepts, so the temptation is
  // to list the Anthropic family here on the strength of that. Accepting the
  // shape is not the same as serving the tool well, this file's own doc says so,
  // and an unrun entry at the top of the picker is a recommendation nobody made.
  "claude-code": [],
};

/**
 * The raw table for this tool. Exported for tests; call {@link pinnedModels}
 * from the UI so the production gate is never accidentally skipped.
 */
export function knownGoodFor(slug: string | null | undefined): readonly KnownGoodRule[] {
  return (slug && KNOWN_GOOD[slug]) || [];
}

/**
 * Models to float to the top of the picker, or nothing outside development.
 *
 * The gate is the whole reason this is safe to keep in the tree. A released app
 * orders the catalogue exactly as the gateway returns it, so a stale entry here
 * can never put a model in front of a user, and nobody has to remember to empty
 * the table before a release.
 */
export function pinnedModels(slug: string | null | undefined): readonly KnownGoodRule[] {
  if (!import.meta.env.DEV) return [];
  return knownGoodFor(slug);
}

/** Is this model on the dev pin list? Used for ordering only. */
export function isPinned(model: ModelFacts, pinned: readonly KnownGoodRule[]): boolean {
  return pinned.some((r) =>
    r.match.endsWith("*") ? model.id.startsWith(r.match.slice(0, -1)) : model.id === r.match,
  );
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
}

/**
 * Can this app be served with this model?
 *
 * Answers only what can be shown to be false. A model is refused when something
 * MEASURED says the request would fail, and offered otherwise. There is no
 * third "probably fine" state on the wire or on screen: the picker lists what
 * the catalogue offers, and the ordering aid above decides nothing.
 *
 * **Silence is never a denial.** A row with no tags has not said it lacks
 * tools, and a shape the gateway holds no verdict on has not been shown to
 * fail. Reading either as "no" would hide models that work, which is what the
 * `acceptsFreeformTools` regex this replaced did to every model outside one
 * family: it answered a bare boolean, so hundreds nobody had ever tried read as
 * refusals.
 */
export function compatibility(model: ModelFacts, needs: ToolNeeds): Compatibility {
  if (needs.tools) {
    const fn = toolShapeVerdict(model, "function");
    // A served verdict beats the tag, because it separates "declared
    // unsupported" from "never said". The tag answers only when there is none.
    if (fn === "fails") return { ok: false, reason: "no-tool-use" };
    if (fn === "unknown" && model.tags.length > 0 && !model.tags.includes("tool-use")) {
      return { ok: false, reason: "no-tool-use" };
    }
  }

  if (needs.freeformTools && toolShapeVerdict(model, "freeform") === "fails") {
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
      return `These models were tested and verified to reject the form ${appName} sends its tools in.`;
  }
}
