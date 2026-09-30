import { useCallback, useEffect, useRef, useState } from "react";
import {
  gateCredits,
  gateModelCatalogue,
  setToolModel,
  toolModelPreferences,
  type ConfiguredModel as RawConfiguredModel,
  type ToolModels,
} from "./api";
import { toFailure, type ActivityFailure } from "./activity";
import { getCurrentWindow } from "@tauri-apps/api/window";

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
 * **3. The tool's own config is the source of truth.** A Gate model choice is
 * written into the tool's config (Codex's `config.toml`, Hermes's
 * `config.yaml`), and the user can change the model from inside the tool. The
 * backend reads every config on `tool_model_preferences`, puts a tool that has
 * moved off its Gate models back on App default, and says so once through
 * `configured[slug].left_gate_models`. That report is held here until it is
 * dismissed or the next save for that tool, because the read that carries it
 * may be followed by another before anyone looks.
 *
 * The catalogue behind the picker is still a network read - see
 * {@link useGateModels} - because only the gateway knows what it can serve.
 */

/**
 * The tools whose pane draws the Gate model card.
 *
 * An explicit list rather than the rail's `coversAllProviders`, which answers a
 * different question - whether a row has one upstream host to name. Hermes
 * talks to several providers and so is multi-provider on the rail, yet its
 * `config.yaml` holds a provider entry Gate can write the chosen set into, so it
 * gets the card. OpenCode, OpenClaw, the environment channel and the chat
 * domains have no config Gate writes a model into, so they do not.
 *
 * All three are written by the backend (`Integration::supports_gate_models`):
 * Codex's `config.toml` and model catalog, Hermes' `config.yaml`, Claude
 * Code's `settings.json`.
 */
export const GATE_MODEL_TOOLS: ReadonlySet<string> = new Set(["codex", "hermes", "claude-code"]);

/**
 * The most Gate models one app can be put on at once (design, 2026-09-30).
 *
 * The picker stops at this many and the backend refuses more
 * (`tool_models::MAX_GATE_MODELS`), so the two move together.
 */
export const MAX_GATE_MODELS = 4;

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
  /**
   * What each supporting tool's own config says, keyed by slug. Only tools
   * whose config can hold Gate models appear; an absent key is "not reported",
   * which a binary that predates the field also yields.
   */
  configured: Map<string, ConfiguredModel>;
}

/** One tool's config, as {@link ToolModelsView.configured} holds it. */
export interface ConfiguredModel {
  state: "applied" | "not_applied" | "drifted";
  /** The model the config starts the tool on, when it is on Gate models. */
  model: string | null;
  /** The tool was moved off Gate models from inside itself, found on this
   *  read. */
  leftGateModels: boolean;
  /** With `leftGateModels`: the model its config names now, if any. */
  leftToModel: string | null;
  /**
   * Why this reading cannot be trusted, in words the card shows: the config
   * could not be read, or the tool could not be put back on its own model after
   * it drifted - which leaves it pointed at the Gate models route with every
   * request refused. Null when all is well.
   */
  problem: string | null;
}

/**
 * Read one `configured` entry, or null when it cannot be read.
 *
 * An unrecognised `state` drops the entry, for the reason `source` is dropped
 * below: guessing "applied" would claim the config holds a Gate model it may
 * not, and guessing "not_applied" would hide a paid one.
 */
function adaptConfigured(raw: RawConfiguredModel | undefined): ConfiguredModel | null {
  if (typeof raw !== "object" || raw === null) return null;
  const { state } = raw;
  if (state !== "applied" && state !== "not_applied" && state !== "drifted") return null;
  const str = (v: unknown) => (typeof v === "string" && v.length > 0 ? v : null);
  const leftGateModels = raw.left_gate_models === true;
  return {
    state,
    model: str(raw.model),
    leftGateModels,
    leftToModel: leftGateModels ? str(raw.left_to_model) : null,
    problem: str(raw.problem),
  };
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
  const configured = new Map<string, ConfiguredModel>();
  const rawConfigured = raw?.configured;
  if (typeof rawConfigured === "object" && rawConfigured !== null && !Array.isArray(rawConfigured)) {
    for (const [slug, entry] of Object.entries(rawConfigured)) {
      const view = adaptConfigured(entry);
      if (view) configured.set(slug, view);
    }
  }
  return {
    byTool,
    paidAckUnix: typeof raw?.paid_ack_unix === "number" ? raw.paid_ack_unix : null,
    configured,
  };
}

/**
 * The sentence for a tool the backend found moved off Gate models from inside
 * itself (R3). Says who did it, where, and what the card shows as a result, so
 * the radio having moved to App default is not a mystery.
 *
 * `toModel` is the model the tool's config names now; null when it names none,
 * which the sentence covers without inventing one.
 */
export function leftGateModelsNotice(appName: string, toModel: string | null): string {
  return `You switched ${appName} to ${toModel ?? "another model"} in ${appName}, so it is back on App default.`;
}

/** What a click on one of the card's two radios should do. */
export type ModelChoiceStep =
  /** Hand the app to Gate for this set, confirming billing first if needed. */
  | { kind: "activate"; modelIds: string[] }
  /** Gate model with nothing enabled yet: the picker comes first. */
  | { kind: "pick" }
  /** App default, remembering this set for the Gate radio to name. */
  | { kind: "remember"; modelIds: string[] };

/**
 * The step behind a radio click, given the app's remembered set.
 *
 * App default keeps the WHOLE set. It kept only the first until AG-590's sets
 * made that a loss: a round trip through App default dropped every model after
 * the first, and the Gate radio then named one where the user had enabled six.
 */
export function stepForChoice(choice: "app" | "gate", modelIds: string[]): ModelChoiceStep {
  if (choice === "app") return { kind: "remember", modelIds: [...modelIds] };
  // Gate cannot serve a model nobody enabled.
  return modelIds.length > 0 ? { kind: "activate", modelIds: [...modelIds] } : { kind: "pick" };
}

/** The outcome of one {@link useToolModels} save. */
export interface ToolModelSave {
  /** Null when the choice was stored. */
  failure: ActivityFailure | null;
  /** The tool's own config was rewritten, so a running copy needs a restart to
   *  pick it up. False on a failure, and when Gate does not manage the tool's
   *  config right now (the choice is stored and applied on its next connect). */
  applied: boolean;
}

/** One model the gateway offers. */
export interface GateModel {
  /** Canonical id, e.g. `anthropic/claude-opus-5`. */
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
 * `save` resolves to the failure rather than throwing, so the caller can branch,
 * and says whether the tool's config was rewritten, so the caller can offer the
 * restart notice.
 *
 * `leftGateModels` holds each tool the backend reported as moved off Gate
 * models from inside itself, with the model it moved to (null when its config
 * names none). The backend says it once, so it is kept here until
 * `dismissLeft` or the next save for that tool - a focus re-read a second later
 * would otherwise wipe the only account of why the card changed.
 *
 * Re-read when the window regains focus, as `useCredits` does and for the same
 * kind of reason: the user changes the model inside the tool, in another
 * window, and the pane has to show what the tool's config says on the way back.
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
  ) => Promise<ToolModelSave>;
  leftGateModels: ReadonlyMap<string, string | null>;
  dismissLeft: (tool: string) => void;
} {
  const [view, setView] = useState<ToolModelsView | null>(null);
  const [failure, setFailure] = useState<ActivityFailure | null>(null);
  const [loading, setLoading] = useState(false);
  const [leftGateModels, setLeftGateModels] = useState<ReadonlyMap<string, string | null>>(
    () => new Map(),
  );
  const attempt = useRef(0);

  const reload = useCallback(() => {
    if (!enabled) return;
    const mine = ++attempt.current;
    setLoading(true);
    toolModelPreferences()
      .then((payload) => {
        const next = adaptPreferences(payload);
        // Merged from EVERY response, before the staleness guard below. The
        // backend reports a tool leaving Gate models once, on the read that
        // folded it, and a newer read in flight (a quick double focus, the
        // reload inside `save`) would otherwise drop that one report with the
        // stale view: the card flips to App default and never says why. Added
        // to, never replaced.
        const left = [...next.configured].filter(([, c]) => c.leftGateModels);
        if (left.length > 0) {
          setLeftGateModels((held) => {
            const merged = new Map(held);
            for (const [slug, c] of left) merged.set(slug, c.leftToModel);
            return merged;
          });
        }
        if (mine !== attempt.current) return;
        setView(next);
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
    setLeftGateModels(new Map());
  }, [credential]);

  useEffect(reload, [reload]);

  useEffect(() => {
    if (!enabled) return;
    // Window focus, with the blur latch, exactly as `useCredits` does it: the
    // case is alt-tabbing to the tool, changing its model there, and coming
    // back, which never hides the document.
    let blurred = false;
    let cancelled = false;
    const pending = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (!focused) {
        blurred = true;
        return;
      }
      if (blurred) {
        blurred = false;
        reload();
      }
    });
    return () => {
      cancelled = true;
      void pending.then((unlisten) => {
        if (cancelled) unlisten();
      });
    };
  }, [enabled, reload]);

  const dismissLeft = useCallback((tool: string) => {
    setLeftGateModels((held) => {
      if (!held.has(tool)) return held;
      const next = new Map(held);
      next.delete(tool);
      return next;
    });
  }, []);

  const save = useCallback(
    async (
      tool: string,
      source: ModelSource,
      modelIds: string[],
      acknowledgePaidUse = false,
    ): Promise<ToolModelSave> => {
      try {
        const applied = await setToolModel(tool, source, modelIds, acknowledgePaidUse);
        // A new choice supersedes the report of the old one leaving.
        dismissLeft(tool);
        // Re-read rather than patching the local map. The write can change
        // something it was not asked to - the acknowledgement stamp - and the
        // file is shared with the CLI, so what landed is worth reading back
        // rather than assumed.
        reload();
        // Strictly `true`: an older binary resolved nothing, and offering to
        // close a running app on a guess is the thing `offerAfterChange` is
        // careful never to do.
        return { failure: null, applied: applied === true };
      } catch (e) {
        return { failure: toFailure(e), applied: false };
      }
    },
    [reload, dismissLeft],
  );

  return { view, failure, loading, reload, save, leftGateModels, dismissLeft };
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

  // Once per session, not once per opening: the catalogue is large and
  // unchanging within a session (see the doc above), so re-fetching 300+ models
  // every time the picker is raised buys nothing.
  useEffect(() => {
    if (enabled && models === null) reload();
  }, [enabled, models, reload]);
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
  /**
   * Where this org manages billing, or null when the gateway named no
   * destination (AG-592, AG-729).
   *
   * Null is a real answer and the one every gateway gave until now. A control
   * pointing nowhere is worse than no control, because the user has to click it
   * to discover that, so a null here means the action is not drawn at all
   * rather than drawn disabled.
   */
  billingUrl: string | null;
}

export function adaptCredits(raw: Partial<Credits> & { billing?: unknown }): Credits {
  return {
    plan: typeof raw?.plan === "string" && raw.plan.length > 0 ? raw.plan : null,
    paygEnabled: raw?.paygEnabled === true,
    balanceCents: typeof raw?.balanceCents === "number" ? raw.balanceCents : null,
    lowBalanceThresholdCents:
      typeof raw?.lowBalanceThresholdCents === "number" ? raw.lowBalanceThresholdCents : null,
    autoTopupArmed: raw?.autoTopupArmed === true,
    billingUrl: adaptBillingUrl(raw?.billing),
  };
}

/**
 * Read the billing destination off the credits payload.
 *
 * Anything that is not a usable http(s) URL reads as null, which is the state
 * the app already handles. The scheme check is not paranoia: this value is
 * handed to the system opener, and that is not somewhere to forward an
 * arbitrary string a response happened to contain.
 */
function adaptBillingUrl(billing: unknown): string | null {
  if (typeof billing !== "object" || billing === null) return null;
  const url = (billing as { manageUrl?: unknown }).manageUrl;
  if (typeof url !== "string" || url.length === 0) return null;
  try {
    const parsed = new URL(url);
    return parsed.protocol === "https:" || parsed.protocol === "http:" ? url : null;
  } catch {
    return null;
  }
}

/**
 * The plan, as a person reads it.
 *
 * The gateway reports `free` or `paid` - the column is constrained to exactly
 * those two (`CHK_org_entitlements_plan`), and no endpoint anywhere reports
 * "pro". The dashboard maps `paid` to **"Pro"** for display, in its sidebar,
 * its billing page and its billing emails, so that is the word the user has
 * already seen for this account. Connect title-cased the raw value instead and
 * said "Paid", which is the same account described by two different products in
 * two different words - and AG-879 asks for the opposite: what Connect shows
 * must equal what the dashboard shows.
 *
 * Anything else the gateway grows is passed through title-cased rather than
 * hidden, since an unknown plan name is still the user's plan.
 *
 * Null stays null. A plan nobody reported is not "Free": that is the one value
 * a reader acts on, by going to upgrade something they may already have.
 */
export function formatPlan(plan: string | null): string | null {
  if (plan === null) return null;
  if (plan === "paid") return "Pro";
  if (plan === "free") return "Free";
  return plan.charAt(0).toUpperCase() + plan.slice(1);
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
 * Re-read when the window regains focus, which is that same argument followed
 * through. The balance moves while the user is elsewhere - running the very tool
 * this pane is about - so a figure read once when the pane opened is stale by
 * the time they come back to check what it cost. It showed `$9.99 available`
 * after eight cents had been spent, which is not a stale number so much as a
 * wrong one: principle 6 asks that a figure on screen be something Gate actually
 * measured, and this is the screen someone opens to see spending. Costs nothing
 * while the window is hidden, and one read on return.
 *
 * **The two gates are separate on purpose.** `enabled` is "may this be read at
 * all", and it widened to the whole signed-in session when Settings needed the
 * plan (AG-891) - that pane's row could not have been wired to anything while
 * this was gated on an app pane being open. `refreshOnFocus` is the narrower
 * question of whether to spend a read on every alt-tab, and it stays on the
 * surface that argued for it: the balance is what moves, the app pane is what
 * draws it, and a plan does not change while somebody switches windows. Folding
 * the two together would put a `/v1/me/credits` on every return to the window
 * for the life of the session, against a throttle bucket the gateway keys on
 * the source address and therefore shares between everyone behind it.
 */
export function useCredits(
  enabled: boolean,
  credential = "",
  refreshOnFocus = false,
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
    // `refreshOnFocus` is a dependency although the body never reads it, so
    // that the surface which cares about the balance re-reads on the way in.
    // `useEffect(reload, [reload])` below is the only thing that fires the
    // first read, and it fires when this identity changes: while `enabled` was
    // the app pane, opening the pane re-read for free. `enabled` is the whole
    // session now (AG-891), so without this the pane would draw whatever was
    // fetched at sign-in - a balance beside a button that spends it, which is
    // the figure this hook exists to keep honest.
  }, [enabled, credential, refreshOnFocus]);

  useEffect(() => {
    setCredits(null);
    setFailure(null);
  }, [credential]);

  useEffect(reload, [reload]);

  useEffect(() => {
    if (!enabled || !refreshOnFocus) return;
    // Window FOCUS, not `visibilitychange`. The motivating case is alt-tabbing
    // to the tool that spends the credits and back, and that never hides the
    // document: `visibilitychange` fires on minimise and full occlusion only, so
    // it missed the exact scenario this exists for and the pane kept showing the
    // balance from before the spending. `onFocusChanged` is the primitive that
    // matches, and `useWindowReopen` and `App` already use it for the same
    // reason.
    let blurred = false;
    let cancelled = false;
    const pending = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (!focused) {
        blurred = true;
        return;
      }
      // Only on a real return. Without the blur latch the initial focus event
      // would re-read a balance the mount has just read.
      if (blurred) {
        blurred = false;
        reload();
      }
    });
    return () => {
      cancelled = true;
      void pending.then((unlisten) => {
        if (cancelled) unlisten();
      });
    };
  }, [enabled, refreshOnFocus, reload]);

  return { credits, failure, reload };
}
