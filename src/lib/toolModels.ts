import { useCallback, useEffect, useRef, useState } from "react";
import { gateModelCatalogue, setToolModel, toolModelPreferences, type ToolModels } from "./api";
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

  useEffect(() => {
    if (enabled && models === null) reload();
  }, [enabled, models, reload]);
  return { models, failure, loading, reload };
}
