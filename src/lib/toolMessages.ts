import { useCallback, useEffect, useRef, useState } from "react";
import { activityCachedToolOverviews, activityOverview } from "./api";
import { adapt } from "./activity";

/**
 * A messages figure per tool, for a surface that draws one on every row (the
 * tray's quick status).
 *
 * **Why this is not just `useActivity` per row.** `GET /v1/me/activity` answers
 * for one tool at a time and sits in a 100-per-minute throttle bucket keyed on
 * the source address rather than the credential, so it is shared with every other
 * Gate Connect user behind the same egress. A read per row on a popover the tray
 * icon opens and closes all day is the one fan-out that budget cannot take. So
 * this hook does two things instead:
 *
 * 1. **Opens on what is already on disk.** `activity_cached_tool_overviews` is one
 *    file read that returns every held per-tool reading for this scope, so the
 *    first paint carries real figures rather than skeletons - the same bargain
 *    `useActivity` strikes for the Overview, extended across rows.
 * 2. **Refreshes only what has gone stale**, on {@link STALE_MS}, one tool at a
 *    time. Opening the popover refreshes; opening it again ten seconds later does
 *    not, because nothing has changed and the second look would spend the budget
 *    for the first one's answer.
 *
 * That pairing is what makes "refresh whenever the user looks" affordable: the
 * TTL never limits how often somebody may look, it only collapses repeated looks
 * inside a window where the answer would be the same. Which matters because one
 * of the three moments people open this app is "debugging why a tool isn't
 * connecting", and that is precisely the click-close-click case.
 *
 * **Every figure is a measurement, and it says when it was taken.** `measuredAt`
 * is the reading's own `generatedAt`, not the time it was displayed, so a surface
 * that shows a held figure can say how old it is (principle 6). A tool with no
 * reading is absent from the map rather than present with a zero.
 */

/**
 * How old a reading may be before a look refreshes it.
 *
 * 45 seconds, and the reasoning is the reverse of a poll's. Opening the quick
 * status is rare, so nearly every real open is past this and re-reads; the only
 * thing the window suppresses is the burst of a user clicking the tray icon
 * repeatedly, where the gateway would answer with the figure it just gave. A
 * longer TTL would start showing stale numbers to someone who came back
 * deliberately; a shorter one stops protecting the burst.
 */
export const STALE_MS = 45_000;

/** One tool's figure, and when the gateway computed it. */
export interface ToolMessages {
  /** The `Messages` counter. A number, always: a reading whose counter the
   *  gateway declined is not in the map at all, because "the section was
   *  unavailable" and "we never read it" want the same thing from a row with one
   *  line to say it in. The app pane is the surface with room to tell them apart,
   *  and it does its own read. */
  messages: number;
  /** The reading's own `generatedAt`, as a local clock time. */
  measuredAt: string;
}

export interface ToolMessagesView {
  /** Tool slug -> its figure. A tool with no reading is absent. */
  byTool: Map<string, ToolMessages>;
  /** Slugs with a read in flight and nothing held yet, so a row can hold a place
   *  rather than draw a zero it has not measured. */
  pending: Set<string>;
  /** Re-read whatever has gone stale. Idempotent and safe to call on every
   *  visibility edge; it does nothing when everything is fresh. */
  refresh: () => void;
}

/** What one body contributes, or null when it holds no usable figure - either it
 *  will not parse, or the gateway declined the counter. */
function figure(text: string): ToolMessages | null {
  try {
    // Typed off `adapt` itself rather than from an exported payload interface:
    // `RawOverview` is private to `activity.ts` on purpose - that module is the
    // single place that knows the shape - and this keeps the cast unable to drift
    // from the function it feeds.
    const view = adapt(JSON.parse(text) as Parameters<typeof adapt>[0]);
    // `null` is the gateway saying it could not answer that counter. Not a zero,
    // and not a figure - so no entry, which is what the row draws nothing for.
    if (view.stats.messages === null) return null;
    return { messages: view.stats.messages, measuredAt: view.takenAt };
  } catch {
    // A body that will not parse is not a reading. It cannot be repaired here and
    // saying so is not this hook's job: the row simply has no figure, which is
    // the same state as never having read one.
    return null;
  }
}

export function useToolMessages(
  enabled: boolean,
  /** The tools with rows to fill. Read by value, not by identity: this is a fresh
   *  array on every render of the caller. */
  slugs: string[],
  /** This machine, as the gateway named it. `null` means org-wide, which is a
   *  different question - see `machineKnown` in `NewUiApp` - so the caller must
   *  gate `enabled` on having one rather than passing null and getting the whole
   *  org's traffic under one machine's rows. */
  installId: string | null,
  /** Whose readings these are; changing it clears them. Same contract as
   *  `useActivity`'s. */
  credential = "",
): ToolMessagesView {
  const [byTool, setByTool] = useState<Map<string, ToolMessages>>(new Map());
  const [pending, setPending] = useState<Set<string>>(new Set());
  /** When each slug's held reading landed *here*, for the staleness decision.
   *  Deliberately not the body's `generatedAt`: that is the gateway's compute
   *  time and can sit behind our own read by however long the request took, so
   *  using it to decide "asked recently enough" would re-ask early, every time. */
  const readAt = useRef<Map<string, number>>(new Map());
  /** One in-flight refresh at a time. Two overlapping passes would double the
   *  spend for one answer. */
  const running = useRef(false);
  const wanted = slugs.join(",");

  // A reading belongs to the account and the machine it was taken for. Same rule
  // as everywhere else here: figures must never survive a switch and be relabelled.
  useEffect(() => {
    setByTool(new Map());
    setPending(new Set());
    readAt.current = new Map();
  }, [credential, installId]);

  const refresh = useCallback(() => {
    if (!enabled || installId === null || running.current) return;
    const slugs = wanted ? wanted.split(",") : [];
    if (slugs.length === 0) return;
    running.current = true;
    void (async () => {
      try {
        // Off disk first, in one read: a popover must paint before it asks
        // anything, and what landed here is what the window's own app-pane reads
        // already fetched.
        const held = await activityCachedToolOverviews(installId ?? undefined).catch(
          () => ({}) as Record<string, string>,
        );
        const fromDisk = new Map<string, ToolMessages>();
        for (const [slug, text] of Object.entries(held)) {
          const one = figure(text);
          if (one) fromDisk.set(slug, one);
        }
        if (fromDisk.size > 0) {
          setByTool((prev) => new Map([...prev, ...fromDisk]));
        }

        const now = Date.now();
        const stale = slugs.filter((s) => (now - (readAt.current.get(s) ?? 0)) > STALE_MS);
        if (stale.length === 0) return;
        setPending(new Set(stale.filter((s) => !fromDisk.has(s))));
        // Sequential, not `Promise.all`. A popover open must not be a burst of N
        // concurrent requests at a shared budget, and nobody is waiting on the
        // last row's figure to read the first one's.
        for (const slug of stale) {
          const text = await activityOverview(installId ?? undefined, slug).catch(() => null);
          // Recorded even for a failure: a gateway that just refused is not worth
          // asking again on the next open a second later.
          readAt.current.set(slug, Date.now());
          const one = text === null ? null : figure(text);
          if (one) {
            setByTool((prev) => new Map(prev).set(slug, one));
          }
          setPending((prev) => {
            if (!prev.has(slug)) return prev;
            const next = new Set(prev);
            next.delete(slug);
            return next;
          });
        }
      } finally {
        running.current = false;
        setPending(new Set());
      }
    })();
  }, [enabled, installId, wanted]);

  // On mount, on scope change, and on every fresh look. The `refresh` identity
  // covers the first two; the visibility edge is the third, and it is the one the
  // tray needs - the popover is shown and hidden, not created and destroyed, so
  // mounting happens once and every later open is a visibility change.
  useEffect(() => {
    refresh();
    const onVisible = () => {
      if (!document.hidden) refresh();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [refresh]);

  return { byTool, pending, refresh };
}
