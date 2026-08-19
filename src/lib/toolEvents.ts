import { useCallback, useEffect, useRef, useState } from "react";
import { activityToolEvents } from "./api";
import { toFailure, type ActivityFailure } from "./activity";
import type { ActivityEntry } from "../components/gc/AppPane";

/**
 * Adapter between `GET /v1/me/tool-events` and the app pane's activity feed
 * (AG-574).
 *
 * The gateway sends one row per request, carrying only a timestamp, an enum, and
 * identifiers. There is no title and there will not be one: the ticket forbids
 * showing prompt text, and the only human-readable string the gateway holds for a
 * conversation is the user's own prompt, stored unredacted. So this module's job
 * is narrower than the Overview adapter's - format a time, name a state - and it
 * has no copy to invent.
 */

/** Per-row shape as the gateway sends it. */
interface RawEvent {
  requestId: string;
  at: string;
  status: "success" | "error";
  /** Null means the caller may not see this row's security detail, which is not
   *  the same as `allow` - see the rendering below. */
  securityAction: "allow" | "flag" | "redact" | "block" | null;
  securityCategory: string | null;
  model: string | null;
  sessionRef: string | null;
}

interface RawToolEvents {
  generatedAt: string;
  window: { from: string; to: string };
  toolScope: { tool: string };
  installation?: { installId: string | null };
  events?: RawEvent[];
  nextCursor?: string | null;
}

/**
 * What a row says when the gateway withheld its security action.
 *
 * The withholding is a visibility rule - security detail is self-only for every
 * role - so this is "not yours to see", not "nothing happened". Rendering it as
 * `allow` would report a colleague's blocked request as permitted, which is the
 * one mistake this whole field exists to prevent.
 */
const WITHHELD = "\u2014";

/** What a row says when no model was attributed to the request. */
const NO_MODEL = "Unknown model";

/**
 * The gateway's action verbs, in the pane's own vocabulary.
 *
 * The gateway records what a criterion *did* (`block`, `redact`) because that is
 * the policy's own wording; the design's pills read as what happened to the
 * request (`blocked`, `redacted`). Mapped here rather than renamed on either side:
 * the gateway's column is shared with the dashboard and the policy config, and the
 * pills are the Figma's copy.
 */
const SECURITY: Record<NonNullable<RawEvent["securityAction"]>, ActivityEntry["security"]> = {
  allow: "allow",
  flag: "flagged",
  redact: "redacted",
  block: "blocked",
};

/**
 * One row, formatted.
 *
 * The time is a clock time rather than an age, for the reason `ActivityView.takenAt`
 * gives: an age has to be recomputed to stay true, and a stale one is a worse lie
 * than the age it was added to disclose.
 */
function toEntry(raw: RawEvent, onView: (event: RawEvent) => void): ActivityEntry {
  return {
    id: raw.requestId,
    time: new Date(raw.at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }),
    status: raw.status,
    // `allow` is the honest default *only* when the gateway answered. A null
    // action is unknown to us and renders as a dash, not as a verdict.
    security: raw.securityAction ? SECURITY[raw.securityAction] : null,
    model: raw.model ?? NO_MODEL,
    reference: raw.sessionRef ?? WITHHELD,
    onView: () => onView(raw),
  };
}

export interface ToolEventsView {
  entries: ActivityEntry[];
  /** Pass to `loadMore`. Null when this is the last page. */
  nextCursor: string | null;
}

export function adaptEvents(
  raw: RawToolEvents,
  onView: (event: RawEvent) => void = () => {},
): ToolEventsView {
  return {
    entries: (raw.events ?? []).map((e) => toEntry(e, onView)),
    nextCursor: raw.nextCursor ?? null,
  };
}

/**
 * Load one tool's feed, and pages of it on demand.
 *
 * Separate from `useActivity` rather than folded into it, which is the opposite
 * call to the one made for the tool *scope*: that shared a request shape and a
 * race, this does not. A feed pages and accumulates; an overview replaces itself
 * wholesale. Sharing the generation guard between them would mean a "load more"
 * and a scope change fighting over the same ref.
 *
 * No disk cache, deliberately: the held reading is one slot and belongs to the
 * Overview. See `activity_cache.rs`.
 */
export function useToolEvents(
  enabled: boolean,
  tool: string | null,
  installId: string | null = null,
  credential = "",
): {
  view: ToolEventsView | null;
  failure: ActivityFailure | null;
  loading: boolean;
  loadMore: () => void;
  reload: () => void;
} {
  const [view, setView] = useState<ToolEventsView | null>(null);
  const [failure, setFailure] = useState<ActivityFailure | null>(null);
  const [loading, setLoading] = useState(false);
  /** Which scope is current, so a page that arrives after the user has moved on
   *  is dropped rather than appended to a different tool's feed. */
  const attempt = useRef(0);

  const fetchPage = useCallback(
    (cursor: string | null) => {
      if (!enabled || !tool) return;
      const mine = ++attempt.current;
      setLoading(true);
      setFailure(null);
      activityToolEvents(tool, installId ?? undefined, cursor ?? undefined)
        .then((text) => {
          if (mine !== attempt.current) return;
          const page = adaptEvents(JSON.parse(text) as RawToolEvents);
          // Append when paging, replace when starting over. `cursor` is what
          // distinguishes the two, so "load more" cannot silently reset the list.
          setView((prev) =>
            cursor && prev
              ? { entries: [...prev.entries, ...page.entries], nextCursor: page.nextCursor }
              : page,
          );
        })
        .catch((e) => {
          if (mine !== attempt.current) return;
          // The pages already loaded stay: they are a real reading of this same
          // scope, and dropping them because the *next* page failed loses what
          // the user was reading.
          setFailure(toFailure(e));
        })
        .finally(() => {
          if (mine === attempt.current) setLoading(false);
        });
    },
    [enabled, tool, installId, credential],
  );

  // A feed belongs to the scope it was read for, so a scope change drops it
  // rather than leaving one tool's requests under another tool's name.
  useEffect(() => {
    setView(null);
    setFailure(null);
  }, [credential, installId, tool]);

  useEffect(() => {
    fetchPage(null);
  }, [fetchPage]);

  return {
    view,
    failure,
    loading,
    loadMore: () => fetchPage(view?.nextCursor ?? null),
    reload: () => fetchPage(null),
  };
}
