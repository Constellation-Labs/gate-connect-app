import type { Credits, GateModel, ToolModelChoice } from "./toolModels";

/**
 * Why a tool's Gate model needs attention, if it does (AG-592).
 *
 * A tool on a Gate model can stop working without anything on screen changing:
 * the model leaves the catalogue, the credits run out, or pay-as-you-go is
 * switched off for the organization. All three end the same way - requests fail -
 * and they need different things from the reader, so they are different causes
 * rather than one "something is wrong".
 *
 * **Gate never substitutes.** AG-592 is explicit, and this module is why it can
 * be: it reports and the pane explains, and nothing anywhere quietly picks a
 * different model. A substitution would be the worst outcome available here -
 * the user would be billed for a model they did not choose, on a screen still
 * naming the one they did.
 *
 * **Silence is not reassurance.** Every input can be unknown - the catalogue may
 * not have loaded, the balance may not have been read - and an unknown input
 * yields `null`, meaning "nothing to say", never "all clear". Claiming a model
 * is fine because we could not check is the failure mode CLAUDE.md's principle 6
 * exists to prevent.
 */

/** What is wrong, in the terms the pane needs to explain it. */
export type ModelAttentionCause =
  /** Gate is serving this tool and its recent requests are failing. */
  | "requests-failing"
  /** A chosen model is no longer in the catalogue Gate serves from. */
  | "model-unavailable"
  /** PAYG is on and the balance cannot cover a request. */
  | "no-credits"
  /** PAYG is switched off for this organization entirely. */
  | "payg-disabled";

export interface ModelAttention {
  cause: ModelAttentionCause;
  /** The models this concerns. Empty for causes that are about the account
   *  rather than a model. */
  models: string[];
  /** One sentence, already written for the person reading it. */
  message: string;
}

/**
 * How many recent requests must have failed before this says so.
 *
 * Low, because the user has already noticed - their tool is broken - and the
 * question is only whether the app can name the cause. Not one, because a single
 * error is a blip and an alarm on it would train people to ignore this.
 */
const MIN_FAILURES = 3;

/**
 * The catalogue is the definition of "available".
 *
 * `GET /v1/models` returns only what the gateway will actually serve - it
 * applies health, pricing, serving-suppression and catalogue-exclusion filters
 * before answering. So a chosen id that is absent from it is precisely a model
 * Gate can no longer serve, and no separate availability check is needed.
 */
function unavailable(modelIds: string[], catalogue: GateModel[]): string[] {
  const servable = new Set(catalogue.map((m) => m.id));
  return modelIds.filter((id) => !servable.has(id));
}

export function modelAttention({
  choice,
  catalogue,
  credits,
  recent,
}: {
  /** The tool's stored preference, or undefined when it has none. */
  choice: ToolModelChoice | undefined;
  /** The models Gate offers, or null when the catalogue has not been read. */
  catalogue: GateModel[] | null;
  /** This org's credit standing, or null when it has not been read. */
  credits: Credits | null;
  /**
   * This tool's most recent requests, newest first, or null when the feed has
   * not been read.
   *
   * The point of the whole module: a Gate model that the tool cannot actually be
   * served with breaks the tool, and nothing else here can see that. The
   * catalogue says the model exists, the balance says it is affordable, and the
   * requests still fail - at a provider, for reasons only the traffic shows. So
   * the traffic is consulted.
   */
  recent?: { status: "success" | "error" }[] | null;
}): ModelAttention | null {
  // Nothing is at stake unless Gate is actually serving this tool. A model
  // remembered under the tool's own default is not in use, so a warning about it
  // would be noise about a request nobody is making.
  if (choice?.source !== "gate" || choice.modelIds.length === 0) return null;

  // Failing traffic outranks everything below. The others are reasons a request
  // *would* fail; this is the request having failed, which is both more certain
  // and what the user is actually experiencing - their tool stopped working.
  //
  // Every recent request, not merely some: one error beside successes is a blip,
  // and a tool that is mostly working does not need an alarm. All of them
  // failing since a Gate model was chosen is the shape of a model this tool
  // cannot be served with.
  if (recent && recent.length >= MIN_FAILURES && recent.every((r) => r.status === "error")) {
    return {
      cause: "requests-failing",
      models: choice.modelIds,
      message:
        "Every recent request from this app has failed while Gate has been serving it. The model may not work with this app. Switch back to App default to use the app's own model again, or choose a different Gate model.",
    };
  }

  // A model that has gone comes first among the rest: it is the most specific
  // fact, and it stops requests regardless of how well funded the account is.
  if (catalogue !== null) {
    const gone = unavailable(choice.modelIds, catalogue);
    // Only when EVERY chosen model has gone. With one still servable the tool
    // keeps working, and Gate uses one the user chose - which is not a
    // substitution and not worth an alarm.
    if (gone.length === choice.modelIds.length) {
      return {
        cause: "model-unavailable",
        models: gone,
        message:
          gone.length === 1
            ? `${gone[0]} is no longer available from Gate. Requests will fail until you choose another model or return to App default - Gate will not pick a replacement for you.`
            : `None of the ${gone.length} models chosen here are available from Gate any more. Requests will fail until you choose another or return to App default - Gate will not pick a replacement for you.`,
      };
    }
  }

  if (credits !== null) {
    if (!credits.paygEnabled) {
      return {
        cause: "payg-disabled",
        models: [],
        message:
          "Pay-as-you-go is off for this organization, so Gate cannot serve a model for it. Requests will fail until it is enabled or this app returns to App default.",
      };
    }
    if (credits.balanceCents !== null && credits.balanceCents <= 0) {
      return {
        cause: "no-credits",
        models: [],
        message:
          "There are no Gate credits left, so requests using a Gate model will fail. Add credits, or return this app to App default to use its own model again.",
      };
    }
  }

  return null;
}
