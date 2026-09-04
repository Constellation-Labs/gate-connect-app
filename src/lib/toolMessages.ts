import { useCallback, useEffect, useRef, useState } from "react";
import { activityCachedToolOverviews, activityOverview } from "./api";
import { parseOverview } from "./activity";

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

/** One tool's figure, when the gateway computed it, and that same instant as a
 *  number so the age can be compared rather than only printed. */
export interface ToolMessages {
  /** The `Messages` counter. A number, always: a reading whose counter the
   *  gateway declined is not in the map at all, because "the section was
   *  unavailable" and "we never read it" want the same thing from a row with one
   *  line to say it in. The app pane is the surface with room to tell them apart,
   *  and it does its own read. */
  messages: number;
  /** The reading's own `generatedAt`, as a local clock time. */
  measuredAt: string;
  /** The same instant, comparable. */
  measuredAtMs: number;
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

/**
 * What one body contributes, or null when it holds no usable figure.
 *
 * Three ways to hold none, and a row draws nothing for all three: the body will
 * not parse, the gateway declined the counter (`null`, which is not a zero), or
 * it carries a `generatedAt` that is not a date - a figure whose age cannot be
 * established has no business claiming one, and printing "measured Invalid Date"
 * in a tooltip is worse than printing nothing.
 */
function figure(text: string): ToolMessages | null {
  const view = parseOverview(text);
  if (!view || view.stats.messages === null || Number.isNaN(view.takenAtMs)) {
    return null;
  }
  return {
    messages: view.stats.messages,
    measuredAt: view.takenAt,
    measuredAtMs: view.takenAtMs,
  };
}

export function useToolMessages(
  enabled: boolean,
  /** The tools with rows to fill. Compared by contents rather than identity, so a
   *  caller need not memoise it. */
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
  /**
   * When each slug was last *asked for*, for the staleness decision.
   *
   * Our own ask time, not the body's `generatedAt`: that is the gateway's compute
   * time and sits behind our read by however long the request took, so deciding
   * "asked recently enough" from it would re-ask early, every time. The one place
   * `generatedAt` is used instead is a reading that came off disk, where there is
   * no local ask time to have - see the seed below.
   */
  const readAt = useRef<Map<string, number>>(new Map());
  /**
   * Which pass is current.
   *
   * The same epoch guard `useActivity` and `useSecurityFeed` use, and for the
   * same reason: every write below happens after an `await`, while the clear on a
   * scope change is synchronous. Without this, a response issued for org A lands
   * after the clear and repaints org A's count on a row whose header now says
   * org B - and stamps `readAt`, so the correct re-read is suppressed for the
   * length of the TTL. A boolean "already running" flag cannot do this job: it
   * de-duplicates passes but never invalidates one.
   */
  const attempt = useRef(0);
  /** The slug list by value, for invalidating `refresh` on its contents. */
  const wanted = slugs.join(",");

  // A reading belongs to the account and the machine it was taken for. Same rule
  // as everywhere else here: figures must never survive a switch and be
  // relabelled. Bumping the epoch is what makes the promise keepable - the clear
  // alone is undone by whatever is still in flight.
  useEffect(() => {
    attempt.current += 1;
    setByTool(new Map());
    setPending(new Set());
    readAt.current = new Map();
  }, [credential, installId]);

  const refresh = useCallback(() => {
    if (!enabled || installId === null || slugs.length === 0) return;
    // Supersede rather than refuse. An earlier pass dropping the replacement was
    // its own bug: a scope change bounces `installId` through null, and the new
    // scope's read arrived while the old pass was still draining, got refused,
    // and nothing rescheduled it - so the rows stayed as they were until the
    // popover was hidden and reopened.
    const mine = ++attempt.current;
    const current = () => mine === attempt.current;
    void (async () => {
      // Off disk first, in one read: a popover must paint before it asks
      // anything, and what landed here is what the window's own app-pane reads
      // already fetched.
      const held = await activityCachedToolOverviews(installId).catch(
        () => ({}) as Record<string, string>,
      );
      if (!current()) return;
      const fromDisk = new Map<string, ToolMessages>();
      for (const [slug, text] of Object.entries(held)) {
        const one = figure(text);
        if (one) fromDisk.set(slug, one);
      }
      if (fromDisk.size > 0) {
        setByTool((prev) => new Map([...prev, ...fromDisk]));
        // Seeded from the reading's own age, which is the only clock a body off
        // disk has. It runs behind our real ask time by the length of that
        // request, so this errs towards re-asking - the safe direction - while
        // still sparing a launch-and-peek the full N calls for readings that are
        // seconds old.
        for (const [slug, one] of fromDisk) {
          if (!readAt.current.has(slug)) readAt.current.set(slug, one.measuredAtMs);
        }
      }

      const now = Date.now();
      const stale = slugs.filter((s) => now - (readAt.current.get(s) ?? 0) > STALE_MS);
      if (stale.length === 0) return;
      setPending(new Set(stale.filter((s) => !fromDisk.has(s))));
      // Sequential, not `Promise.all`. A popover open must not be a burst of N
      // concurrent requests at a shared budget, and nobody is waiting on the
      // last row's figure to read the first one's.
      for (const slug of stale) {
        const text = await activityOverview(installId, slug).catch(() => null);
        if (!current()) return;
        // Recorded even for a failure: a gateway that just refused is not worth
        // asking again on the next look a second later.
        readAt.current.set(slug, Date.now());
        const one = text === null ? null : figure(text);
        setByTool((prev) => {
          if (one) return new Map(prev).set(slug, one);
          // A read that failed drops whatever was held. The held figure was true
          // of an account this credential may no longer be able to read at all -
          // a sign-out leaves `account.json` in place, so the hook stays enabled
          // and every fetch 401s - and a stale number under a signed-out org is
          // the one thing this must never draw. `useActivity` keeps its held
          // reading through a failure because its own header prints the age
          // beside it; a rail row has no such room.
          if (!prev.has(slug)) return prev;
          const next = new Map(prev);
          next.delete(slug);
          return next;
        });
        setPending((prev) => {
          if (!prev.has(slug)) return prev;
          const next = new Set(prev);
          next.delete(slug);
          return next;
        });
      }
    })();
    // `wanted` rather than `slugs`: the array's *contents* are what should
    // invalidate this callback, and a caller that built a fresh array each render
    // would otherwise re-create it every render, re-run the effect below, and
    // loop. The body reads `slugs` directly - the earlier version round-tripped
    // it through the string, which bought a shadowed variable and a silent break
    // on any slug containing a comma.
  }, [enabled, installId, wanted, credential]);

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
