import { useCallback, useEffect, useRef, useState } from "react";
import { activityToolEvents } from "./api";
import { toFailure, type ActivityFailure } from "./activity";
import type { ActivityEntry } from "./toolEventRow";

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
  /** Null either because no decision was recorded for this request, or because
   *  the caller may not see this row's security detail. The gateway deliberately
   *  does not distinguish them - saying *that* something was withheld would leak
   *  that a colleague's request was acted on - and both want the same thing on
   *  screen. What null is not, in either case, is `allow`. */
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
 * What the conversation cell says when the row carries no session reference.
 *
 * Not a withholding and not an error: a request that belonged to no session has no
 * conversation to name. The security cell draws its own dash for its own reason,
 * with its own tooltip - see `AppPane`.
 */
const NO_REFERENCE = "\u2014";

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
 * When a request happened, to the second (Figma 116:30951, "Jun 6, 00:50:51").
 *
 * Seconds because this is a feed of individual requests and an agent sends several
 * a minute: without them, four rows read as the same moment and the order looks
 * arbitrary. The date because the window is 24 hours and so straddles midnight -
 * a bare clock time makes yesterday evening look like this evening.
 *
 * A timestamp rather than an age, for the reason `ActivityView.takenAt` gives: an
 * age has to be recomputed to stay true, and a "2 minutes ago" written twenty
 * minutes ago is a worse lie than the age it was added to disclose.
 */
function eventTime(at: string): string {
  const d = new Date(at);
  const date = d.toLocaleDateString([], { month: "short", day: "numeric" });
  const time = d.toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
  return `${date}, ${time}`;
}

/** One row, formatted. */
function toEntry(raw: RawEvent): ActivityEntry {
  return {
    id: raw.requestId,
    time: eventTime(raw.at),
    status: raw.status,
    // `allow` is the honest default *only* when the gateway answered. A null
    // action is unknown to us and renders as a dash, not as a verdict.
    security: raw.securityAction ? SECURITY[raw.securityAction] : null,
    model: raw.model ?? NO_MODEL,
    reference: raw.sessionRef ?? NO_REFERENCE,
  };
}

export interface ToolEventsView {
  entries: ActivityEntry[];
  /** Pass to `loadMore`. Null when this is the last page. */
  nextCursor: string | null;
}

export function adaptEvents(raw: RawToolEvents): ToolEventsView {
  return {
    entries: (raw.events ?? []).map(toEntry),
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
    // `credential` is not read in the body and belongs here anyway: it is the
    // refetch trigger for an account switch, matching `useActivity`. An
    // `exhaustive-deps` autofix would drop it as unused and silently stop the feed
    // re-reading when the user changes account, leaving one account's requests on
    // screen under another's.
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
    // Guarded here rather than at the call site. With no cursor `fetchPage`
    // re-reads page one and *replaces* the list, so a `loadMore` on the last page
    // would silently discard every page already loaded. The pane happens to hide
    // the control when `nextCursor` is null, but that is the pane being careful
    // about a hazard the hook should not have.
    loadMore: () => {
      if (view?.nextCursor) fetchPage(view.nextCursor);
    },
    reload: () => fetchPage(null),
  };
}
