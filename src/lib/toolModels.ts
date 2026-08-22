import { useCallback, useEffect, useRef, useState } from "react";
import { gateModelCatalogue, setToolModel, toolModelPreferences } from "./api";
import { toFailure, type ActivityFailure } from "./activity";

/**
 * Which Gate model each app runs on (AG-588).
 *
 * The gateway owns the store; this module owns the shape and the two rules the
 * UI has to get right.
 *
 * **1. The preference is org-wide, and keyed on the gateway's platform id.** One
 * read covers every app in the sidebar. The key is `agent_framework` - what the
 * gateway derives per request - not the tool slug this app guesses from a
 * User-Agent, and the two are different namespaces even where the strings match.
 * `Tool.platform_id` carries the mapping from Rust so nothing here has to keep a
 * second copy of it; a tool whose `platform_id` is null cannot hold a preference
 * at all.
 *
 * **2. A remembered model is not an active one.** `source` alone decides what
 * Gate serves. A tool on `"tool"` may still carry `modelIds`, because the pane
 * shows what the user would be switching to; drawing that as though it were live
 * is the exact conflation CLAUDE.md's principle 2 forbids, so
 * {@link ToolModelChoice} keeps the two fields apart rather than collapsing them
 * into one "current model".
 */

/** What Gate serves for one platform. Mirrors the gateway's `source`. */
export type ModelSource = "tool" | "gate";

/** One platform's stored preference, as the endpoint reports it. */
export interface ToolModelChoice {
  /** The gateway's platform id, e.g. `claude-desktop`. Not a tool slug. */
  platformId: string;
  /** What is actually served. `"tool"` means Gate does not intervene. */
  source: ModelSource;
  /**
   * The chosen models, which may be non-empty while `source` is `"tool"`.
   *
   * That combination is a remembered choice, not an active one - see the module
   * doc. Never empty when `source` is `"gate"`: the gateway's schema refuses it.
   */
  modelIds: string[];
  updatedAt: string;
}

/** The whole reading. */
export interface ToolModelsView {
  /** Keyed by platform id, so a pane looks its own up by `Tool.platform_id`. */
  byPlatform: Map<string, ToolModelChoice>;
  /**
   * When this org first accepted paid Gate model use, or null if it never has.
   *
   * Org-wide and hoisted out of the rows on purpose: it decides whether the next
   * switch to a Gate model needs the confirmation, and that has to be answerable
   * before any preference exists.
   */
  firstPaidAckAt: string | null;
}

interface RawPreference {
  platformId?: unknown;
  source?: unknown;
  modelIds?: unknown;
  updatedAt?: unknown;
}

interface RawPreferences {
  preferences?: unknown;
  firstPaidAckAt?: unknown;
}

function adaptPreference(raw: RawPreference): ToolModelChoice | null {
  if (typeof raw.platformId !== "string") return null;
  // An unrecognised source is dropped rather than defaulted. Defaulting to
  // `"tool"` would report a paid Gate model as inactive, and defaulting to
  // `"gate"` would claim Gate is serving something it may not be; neither is a
  // reading, so the row is treated as absent and the pane shows the default it
  // shows for a platform nobody has configured.
  if (raw.source !== "tool" && raw.source !== "gate") return null;
  return {
    platformId: raw.platformId,
    source: raw.source,
    modelIds: Array.isArray(raw.modelIds) ? raw.modelIds.filter((m): m is string => typeof m === "string") : [],
    updatedAt: typeof raw.updatedAt === "string" ? raw.updatedAt : "",
  };
}

export function adaptPreferences(raw: RawPreferences): ToolModelsView {
  const list = Array.isArray(raw.preferences) ? (raw.preferences as RawPreference[]) : [];
  const byPlatform = new Map<string, ToolModelChoice>();
  for (const entry of list) {
    const choice = adaptPreference(entry);
    if (choice) byPlatform.set(choice.platformId, choice);
  }
  return {
    byPlatform,
    firstPaidAckAt: typeof raw.firstPaidAckAt === "string" ? raw.firstPaidAckAt : null,
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
}

interface RawModel {
  id?: unknown;
  owned_by?: unknown;
  name?: unknown;
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
    models.push({
      id: m.id,
      vendor: typeof m.owned_by === "string" ? m.owned_by : m.id.split("/")[0],
      name: typeof m.name === "string" && m.name.length > 0 ? m.name : m.id,
    });
  }
  return models;
}

/**
 * The org's model preferences, plus a writer.
 *
 * Modelled on `useActivity`'s generation guard and for the same reason: a reply
 * from a superseded attempt must not repaint the pane after the user has moved
 * on. `credential` is in the dependency list so switching account or org
 * re-reads rather than showing the previous identity's choices.
 *
 * `save` resolves to the failure rather than throwing, so the caller can branch:
 * `needs_paid_ack` is not an error to report but the signal to raise the billing
 * confirmation and try again with it accepted.
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
      .then((text) => {
        if (mine !== attempt.current) return;
        setView(adaptPreferences(JSON.parse(text) as RawPreferences));
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
        // Re-read rather than patching the local map from the reply. The write
        // can change something it was not asked to - the org's acknowledgement
        // stamp - and a second machine may have changed another platform since
        // this view was taken.
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
