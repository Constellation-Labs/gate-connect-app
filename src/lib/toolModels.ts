import { useCallback, useEffect, useRef, useState } from "react";
import {
  gateCredits,
  gateModelCatalogue,
  setToolModel,
  toolModelPreferences,
  type ToolModels,
} from "./api";
import { toFailure, type ActivityFailure } from "./activity";

/**
 * Which Gate model each app runs on (AG-588).
 *
 * **The choice is local to this install**, in `preferences.json` beside the other
 * user choices. An earlier revision kept it on the gateway, scoped to the
 * organization; that meant one developer's click changed what their colleagues'
 * requests were answered with, and the app pane is already scoped to this
 * machine. The trade is stated where it is stored - see `preferences.rs` - and
 * the part that matters here is that a read is a file read, so "we could not read
 * the setting" is rare rather than routine.
 *
 * Two rules the UI still has to get right:
 *
 * **1. A tool with no entry is not an error.** It is on its own default, which is
 * the true default. The map simply has no key for it.
 *
 * **2. A remembered model is not an active one.** `source` alone decides what
 * would be served. A tool on `"tool"` may still carry `modelIds`, because the
 * pane shows what the user would be switching to; drawing that as though it were
 * live is the conflation CLAUDE.md's principle 2 forbids, so {@link ToolModelChoice}
 * keeps the two apart rather than collapsing them into one "current model".
 *
 * The catalogue behind the picker is still a network read - see
 * {@link useGateModels} - because only the gateway knows what it can serve.
 */

/** What Gate serves for one platform. Mirrors the gateway's `source`. */
export type ModelSource = "tool" | "gate";

/** One tool's stored choice. */
export interface ToolModelChoice {
  source: ModelSource;
  /**
   * The chosen models, which may be non-empty while `source` is `"tool"`.
   *
   * That combination is a remembered choice, not an active one - see the module
   * doc.
   */
  modelIds: string[];
}

/** The whole reading. */
export interface ToolModelsView {
  /** Keyed by tool slug, so a pane looks its own up directly. */
  byTool: Map<string, ToolModelChoice>;
  /**
   * When this install first accepted paid Gate model use, or null if it never
   * has.
   *
   * Hoisted out of the entries because it decides whether the next switch to a
   * Gate model needs the confirmation, and that has to be answerable before any
   * choice exists.
   */
  paidAckUnix: number | null;
}

/**
 * Read the IPC payload.
 *
 * Defensive despite being typed: this crosses a serialization boundary, and a
 * build mismatch between the webview and the binary is exactly the case where a
 * confident cast turns a stale field into a wrong claim about billing.
 */
export function adaptPreferences(raw: ToolModels): ToolModelsView {
  const byTool = new Map<string, ToolModelChoice>();
  for (const [slug, choice] of Object.entries(raw?.tools ?? {})) {
    // An unrecognised source is dropped rather than defaulted. Defaulting to
    // `"tool"` would report a paid Gate model as inactive, and defaulting to
    // `"gate"` would claim Gate is serving something it may not be; neither is a
    // reading, so the tool reads as unconfigured - which is also its true
    // default.
    if (choice?.source !== "tool" && choice?.source !== "gate") continue;
    byTool.set(slug, {
      source: choice.source,
      modelIds: Array.isArray(choice.model_ids)
        ? choice.model_ids.filter((m): m is string => typeof m === "string")
        : [],
    });
  }
  return {
    byTool,
    paidAckUnix: typeof raw?.paid_ack_unix === "number" ? raw.paid_ack_unix : null,
  };
}

/** One model the gateway offers. */
export interface GateModel {
  /** Canonical id, e.g. `anthropic/claude-opus-5`. An identifier: rendered mono. */
  id: string;
  /** Provider namespace, e.g. `anthropic`. Drives the vendor line and the mark. */
  vendor: string;
  /** Human-readable name, falling back to the id when discovery gave none. */
  name: string;
  /**
   * Capabilities the gateway advertises, e.g. `tool-use`, `reasoning`, `vision`.
   *
   * Empty when the row carried none, which is not the same as "can do nothing":
   * an older catalogue row simply may not say. `modelCompatibility` treats an
   * absent tag list as unknown rather than as a denial, for that reason.
   */
  tags: string[];
  /**
   * Which wire form of tool definition this model accepts (AG-729).
   *
   * Answers the question `tags` cannot: `openai/gpt-4o` carries `tool-use` and
   * still rejects Codex's freeform tools. A shape the gateway said nothing
   * about is absent here, and absent means unknown, never "no". The whole field
   * is absent when talking to a gateway that predates it, which
   * `modelCompatibility` handles with a dated local fallback.
   */
  toolShapes?: Partial<Record<ToolShape, ToolShapeReport>>;
}

/** The wire forms a client can send tool definitions in. */
export type ToolShape = "function" | "freeform";

/** What the gateway knows about one (model, shape) pair. */
export interface ToolShapeReport {
  verdict: "works" | "fails" | "unknown";
  /** ISO date of the evidence, present only for a curated verdict. */
  checked?: string;
}

interface RawModel {
  id?: unknown;
  owned_by?: unknown;
  name?: unknown;
  tags?: unknown;
  tool_shapes?: unknown;
}

/**
 * Read the tool-shape block off one catalogue row.
 *
 * Defensive in the same spirit as the tag filter above: a malformed field costs
 * the field, never the model, because the id is what a preference stores and
 * dropping the row would make a saved choice look unavailable. An unrecognised
 * verdict string becomes `unknown` rather than being discarded, so a gateway
 * that grows a fourth verdict degrades to "no opinion" instead of to a denial.
 */
function adaptToolShapes(raw: unknown): Partial<Record<ToolShape, ToolShapeReport>> | undefined {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return undefined;

  const out: Partial<Record<ToolShape, ToolShapeReport>> = {};
  for (const shape of ["function", "freeform"] as const) {
    const entry = (raw as Record<string, unknown>)[shape];
    if (typeof entry !== "object" || entry === null) continue;
    const { verdict, checked } = entry as { verdict?: unknown; checked?: unknown };
    out[shape] = {
      verdict: verdict === "works" || verdict === "fails" ? verdict : "unknown",
      ...(typeof checked === "string" ? { checked } : {}),
    };
  }
  return Object.keys(out).length > 0 ? out : undefined;
}

/**
 * Read the catalogue.
 *
 * Shape is Vercel AI Gateway's, which the gateway mirrors deliberately. Rows
 * without an id are dropped: the id is what a preference stores, so a row that
 * cannot be selected is not worth listing.
 *
 * An empty list is a real answer. The catalogue is built from platform provider
 * accounts, and a deployment with none has nothing to offer - which the picker
 * says in words rather than drawing as an empty list.
 */
export function adaptModels(raw: { data?: unknown }): GateModel[] {
  const list = Array.isArray(raw.data) ? (raw.data as RawModel[]) : [];
  const models: GateModel[] = [];
  for (const m of list) {
    if (typeof m.id !== "string" || m.id.length === 0) continue;
    const toolShapes = adaptToolShapes(m.tool_shapes);
    models.push({
      id: m.id,
      vendor: typeof m.owned_by === "string" ? m.owned_by : m.id.split("/")[0],
      name: typeof m.name === "string" && m.name.length > 0 ? m.name : m.id,
      // Only the string entries: a malformed row should lose a tag, not the
      // whole model, because the id is what a preference stores.
      tags: Array.isArray(m.tags) ? m.tags.filter((t): t is string => typeof t === "string") : [],
      ...(toolShapes ? { toolShapes } : {}),
    });
  }
  return models;
}

/**
 * This install's model choices, plus a writer.
 *
 * Keeps `useActivity`'s generation guard for the same reason: a reply from a
 * superseded attempt must not repaint the pane after the user has moved on. The
 * read is a local file, so it is fast and rarely fails - but "rarely" is not
 * "never", and a failed read still has to leave the card with no selection
 * rather than a default.
 *
 * `credential` no longer scopes the data - the choice belongs to the machine,
 * not the account. It stays in the dependency list on purpose: signing into a
 * different org is a moment when the pane rebuilds anyway, and re-reading a
 * cheap local file then costs nothing and keeps one fewer special case.
 *
 * `save` resolves to the failure rather than throwing, so the caller can branch.
 */
export function useToolModels(
  enabled: boolean,
  credential = "",
): {
  view: ToolModelsView | null;
  failure: ActivityFailure | null;
  loading: boolean;
  reload: () => void;
  save: (
    tool: string,
    source: ModelSource,
    modelIds: string[],
    acknowledgePaidUse?: boolean,
  ) => Promise<ActivityFailure | null>;
} {
  const [view, setView] = useState<ToolModelsView | null>(null);
  const [failure, setFailure] = useState<ActivityFailure | null>(null);
  const [loading, setLoading] = useState(false);
  const attempt = useRef(0);

  const reload = useCallback(() => {
    if (!enabled) return;
    const mine = ++attempt.current;
    setLoading(true);
    toolModelPreferences()
      .then((payload) => {
        if (mine !== attempt.current) return;
        setView(adaptPreferences(payload));
        setFailure(null);
      })
      .catch((e) => {
        if (mine !== attempt.current) return;
        // The previous reading is dropped, unlike the activity pane's. There it
        // is a measurement that stays true; here it is a *setting*, and showing a
        // stale one invites the user to act on it - toggling a switch whose
        // current position we no longer know.
        setView(null);
        setFailure(toFailure(e));
      })
      .finally(() => {
        if (mine === attempt.current) setLoading(false);
      });
  }, [enabled, credential]);

  useEffect(() => {
    setView(null);
    setFailure(null);
  }, [credential]);

  useEffect(reload, [reload]);

  const save = useCallback(
    async (tool: string, source: ModelSource, modelIds: string[], acknowledgePaidUse = false) => {
      try {
        await setToolModel(tool, source, modelIds, acknowledgePaidUse);
        // Re-read rather than patching the local map. The write can change
        // something it was not asked to - the acknowledgement stamp - and the
        // file is shared with the CLI, so what landed is worth reading back
        // rather than assumed.
        reload();
        return null;
      } catch (e) {
        return toFailure(e);
      }
    },
    [reload],
  );

  return { view, failure, loading, reload, save };
}

/**
 * The catalogue, read once the picker needs it.
 *
 * Its own hook rather than part of {@link useToolModels}: the list is large,
 * unchanging within a session, and only the picker wants it, so loading it with
 * the preferences would make every pane open pay for a dialog that is usually
 * never raised.
 *
 * `enabled` is what defers it. Pass the picker's own visibility.
 */
export function useGateModels(enabled: boolean): {
  models: GateModel[] | null;
  failure: ActivityFailure | null;
  loading: boolean;
  reload: () => void;
} {
  const [models, setModels] = useState<GateModel[] | null>(null);
  const [failure, setFailure] = useState<ActivityFailure | null>(null);
  const [loading, setLoading] = useState(false);
  const attempt = useRef(0);

  const reload = useCallback(() => {
    if (!enabled) return;
    const mine = ++attempt.current;
    setLoading(true);
    gateModelCatalogue()
      .then((text) => {
        if (mine !== attempt.current) return;
        setModels(adaptModels(JSON.parse(text) as { data?: unknown }));
        setFailure(null);
      })
      .catch((e) => {
        if (mine !== attempt.current) return;
        setFailure(toFailure(e));
      })
      .finally(() => {
        if (mine === attempt.current) setLoading(false);
      });
  }, [enabled]);

  useEffect(reload, [reload]);

  return { models, failure, loading, reload };
}

/** What the pane needs to know about this org's ability to pay for a Gate model. */
export interface Credits {
  /**
   * The org's plan, or null when the gateway did not name one.
   *
   * Null rather than a default. It used to fall back to "free", which puts a
   * plan on screen that nobody reported - and "Free" is the one value a reader
   * would act on, by going to upgrade something they may already have upgraded.
   */
  plan: string | null;
  paygEnabled: boolean;
  /** Whole cents, or null when it could not be read. Null is not zero - see
   *  {@link formatCredits}. */
  balanceCents: number | null;
  lowBalanceThresholdCents: number | null;
  autoTopupArmed: boolean;
}

export function adaptCredits(raw: Partial<Credits>): Credits {
  return {
    plan: typeof raw?.plan === "string" && raw.plan.length > 0 ? raw.plan : null,
    paygEnabled: raw?.paygEnabled === true,
    balanceCents: typeof raw?.balanceCents === "number" ? raw.balanceCents : null,
    lowBalanceThresholdCents:
      typeof raw?.lowBalanceThresholdCents === "number" ? raw.lowBalanceThresholdCents : null,
    autoTopupArmed: raw?.autoTopupArmed === true,
  };
}

/**
 * The credits line for the model card, as Figma 228:89517 words it
 * ("$10.25 available").
 *
 * Three different answers, deliberately not collapsed:
 *
 * - No reading -> "N/A". Printing "$0.00" for a balance nobody reported would
 *   tell a funded org their tools are about to stop (CLAUDE.md principle 6).
 * - PAYG off -> "Not enabled". The org may have money and still be unable to
 *   spend it here, and the fix is different from adding funds - the gateway's
 *   balance gate reports these as separate causes for the same reason.
 * - Otherwise the amount, which may legitimately be $0.00 and is then the whole
 *   explanation for why requests started failing.
 */
export function formatCredits(credits: Credits | null): string | null {
  if (!credits || credits.balanceCents === null) return null;
  if (!credits.paygEnabled) return "Not enabled";
  return `$${(credits.balanceCents / 100).toFixed(2)} available`;
}

/**
 * The org's credit balance.
 *
 * Read whenever an app pane is open, because the card shows it and a switch to a
 * Gate model turns on spending. Not polled: it changes as requests are served,
 * but a timer here would spend the gateway's address-keyed rate limit on a
 * number that only matters when someone is looking at it.
 *
 * Re-read when the window becomes visible, which is that same argument followed
 * through. The balance moves while the user is elsewhere - running the very tool
 * this pane is about - so a figure read once when the pane opened is stale by
 * the time they come back to check what it cost. It showed `$9.99 available`
 * after eight cents had been spent, which is not a stale number so much as a
 * wrong one: principle 6 asks that a figure on screen be something Gate actually
 * measured, and this is the screen someone opens to see spending. Costs nothing
 * while the window is hidden, and one read on return.
 */
export function useCredits(
  enabled: boolean,
  credential = "",
): { credits: Credits | null; failure: ActivityFailure | null; reload: () => void } {
  const [credits, setCredits] = useState<Credits | null>(null);
  const [failure, setFailure] = useState<ActivityFailure | null>(null);
  const attempt = useRef(0);

  const reload = useCallback(() => {
    if (!enabled) return;
    const mine = ++attempt.current;
    gateCredits()
      .then((text) => {
        if (mine !== attempt.current) return;
        setCredits(adaptCredits(JSON.parse(text) as Partial<Credits>));
        setFailure(null);
      })
      .catch((e) => {
        if (mine !== attempt.current) return;
        // Dropped rather than kept: a balance is a figure that moves, and a
        // stale one beside a button that spends money is worse than no figure.
        setCredits(null);
        setFailure(toFailure(e));
      });
  }, [enabled, credential]);

  useEffect(() => {
    setCredits(null);
    setFailure(null);
  }, [credential]);

  useEffect(reload, [reload]);

  useEffect(() => {
    if (!enabled) return;
    const onVisible = () => {
      if (!document.hidden) reload();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => document.removeEventListener("visibilitychange", onVisible);
  }, [enabled, reload]);

  return { credits, failure, reload };
}
