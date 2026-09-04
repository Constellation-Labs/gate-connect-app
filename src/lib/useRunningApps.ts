import { useCallback, useEffect, useRef, useState } from "react";
import { closeRunningAgents, runningAgents, routingVerdicts } from "./api";
import type { Verdict } from "./api";
import { track, trackError } from "./analytics";
import {
  allSettled,
  isResting,
  nextStage,
  reopenTools,
  type ReopenPresence,
  type ReopenStage,
  type ReopenTool,
} from "./reopen";

/**
 * What happens after a tool's config is rewritten while that tool is running.
 *
 * A running app read its configuration at launch, so the new route does not
 * apply to it until it restarts. Gate can close it but **cannot reopen it** -
 * every tool in the registry is a terminal program whose shell session Gate does
 * not own, which `RunningAgent.can_reopen` reports rather than this file
 * assuming. That is the whole reason this is a conversation rather than
 * something done quietly: closing an editor mid-session is the user's call, not
 * ours, and the user is the only one who can start it again.
 *
 * Four stages: offer, confirm, work, and the account of what happened. The
 * offer's primary is the *passive* option ("I will reopen later"), the
 * destructive one is deliberately the secondary, and only after a second
 * confirmation does anything actually get killed.
 *
 * **The work stage watches rather than assumes.** Closing a process proves
 * nothing about the route that replaces it, so each tool is followed from
 * closing to reopened to verified, off the same two probes the rail uses. A row
 * only reads verified once a *new* process is up and the sweep has answered for
 * it - AG-566 AC 8, and the reason a closed tool does not quietly count as done
 * even though `verdict_for` would happily call its config `on`.
 *
 * Deliberately not part of `useRouting`. That hook's prompt is a **gate**: it
 * blocks a write until answered. This is the opposite - it runs after a
 * successful write and can be walked away from at any point without changing
 * what was saved.
 */

/** How often the work stage re-reads the process table and the sweep.
 *
 *  Two cadences, because the two waits are different lengths: while Gate is
 *  acting the row should move under the reader, and while it is waiting for
 *  someone to open a terminal it should not walk the process table twenty times
 *  a minute for an answer that arrives when it arrives. */
const WATCH_MS = 3000;
const IDLE_WATCH_MS = 10_000;

export type RunningAppsStage =
  /** Affected apps are running; offer to close them. */
  | { kind: "offer"; tools: ReopenTool[]; slugs?: string[] }
  /** "Close affected apps" pressed; confirm before signalling anything. */
  | { kind: "confirm"; tools: ReopenTool[]; slugs?: string[] }
  /** Signalled. Each tool is now followed to its own conclusion. */
  | { kind: "work"; tools: ReopenTool[]; slugs?: string[] };

export interface RunningApps {
  stage: RunningAppsStage | null;
  busy: boolean;
  /**
   * Probe, and open the sequence only if something is actually running.
   *
   * `slugs` are the tools whose configs the write actually touched. Omitting
   * them asks about every tool, which only the master toggle means.
   */
  offerAfterChange: (slugs?: string[]) => Promise<void>;
  goToConfirm: () => void;
  goBack: () => void;
  closeApps: () => Promise<void>;
  /** Move one row, for an action the shell owns: a retried write, a tool put
   *  back on its own defaults. Scoped to one slug on purpose - AG-566 AC 10
   *  requires that retrying one tool repeats nothing for another. */
  markStage: (slug: string, stage: ReopenStage, error?: string) => void;
  /** Re-read the probes now rather than on the next tick, for a row's Retry
   *  verification. */
  checkNow: () => Promise<void>;
  dismiss: () => void;
}

export function useRunningApps({
  onError,
  nameFor,
}: {
  onError?: (err: unknown) => void;
  /** The tool's product name for a slug, which the shell reads off
   *  `list_tools`. The scan reports process names ("claude"), and every surface
   *  of this flow is a list of tools read by someone who knows it as Claude
   *  Code. */
  nameFor?: (slug: string) => string | undefined;
} = {}): RunningApps {
  const [stage, setStage] = useState<RunningAppsStage | null>(null);
  const [busy, setBusy] = useState(false);
  /**
   * The stage, mirrored where the watch can read it.
   *
   * Every move goes through `commit` rather than a `setStage` updater, and the
   * ref is why: the close signals and then immediately looks, and React has not
   * re-rendered in between. Reading state through the render would have that
   * first look decide the operation had not started yet, which is exactly the
   * moment it needs to be right.
   */
  const stageRef = useRef<RunningAppsStage | null>(null);
  const commit = useCallback((next: RunningAppsStage | null) => {
    stageRef.current = next;
    setStage(next);
  }, []);
  /** One tool moved, the rest left exactly as they were. */
  const commitTools = useCallback(
    (map: (tool: ReopenTool) => ReopenTool) => {
      const current = stageRef.current;
      if (current?.kind !== "work") return;
      commit({ ...current, tools: current.tools.map(map) });
    },
    [commit],
  );
  /** How many ticks each tool has spent in its current stage. Kept out of the
   *  stage object because it is bookkeeping for the watch, not something any
   *  surface draws. */
  const waited = useRef(new Map<string, number>());

  const names = useRef(nameFor);
  useEffect(() => {
    names.current = nameFor;
  }, [nameFor]);

  const nameMap = useCallback((slugs: string[]): Map<string, string> => {
    const map = new Map<string, string>();
    for (const slug of slugs) {
      const name = names.current?.(slug);
      if (name) map.set(slug, name);
    }
    return map;
  }, []);

  const verdictMap = useCallback(async (): Promise<Map<string, Verdict>> => {
    try {
      return new Map((await routingVerdicts()).map((v) => [v.slug, v]));
    } catch (err) {
      // A sweep that will not answer leaves the rows where they are, which the
      // watch already reads as "not verified yet". Louder handling here would
      // turn a slow probe into a failure.
      trackError(err, "close_agents");
      return new Map();
    }
  }, []);

  /**
   * Called after a config write that succeeded, with the tools it changed.
   * Nothing running means nothing to close, and a dialog saying so would be a
   * dialog about nothing.
   *
   * A failed probe stays silent rather than defaulting to showing: the popover
   * defaults the other way, but it is choosing whether to show *advice*, and
   * this sequence offers to kill processes. Guessing wrong here means offering
   * to close apps that may not be open.
   */
  const offerAfterChange = useCallback(
    async (slugs?: string[]) => {
      try {
        // Narrowed to what actually changed. A master toggle passes nothing and
        // still offers everything, because it moved every tool's route; a single
        // app's toggle moved only its own, and naming the others would ask to
        // kill work for no reason.
        const { agents } = await runningAgents(slugs);
        if (agents.length === 0) return;
        // The verdict is read here rather than in the dialog because the two
        // routes it carries are the point of the step: "reopen required"
        // without them does not say what reopening would change.
        const verdicts = await verdictMap();
        const tools = reopenTools(
          agents,
          nameMap(agents.map((a) => a.slug)),
          verdicts,
        );
        if (tools.length === 0) return;
        waited.current = new Map();
        commit({ kind: "offer", tools, slugs });
        track("routing_notice_shown");
      } catch (err) {
        trackError(err, "close_agents");
      }
    },
    [commit, nameMap, verdictMap],
  );

  const goToConfirm = useCallback(() => {
    const s = stageRef.current;
    if (s?.kind === "offer") commit({ kind: "confirm", tools: s.tools, slugs: s.slugs });
  }, [commit]);

  const goBack = useCallback(() => {
    const s = stageRef.current;
    if (s?.kind === "confirm") commit({ kind: "offer", tools: s.tools, slugs: s.slugs });
  }, [commit]);

  const markStage = useCallback(
    (slug: string, next: ReopenStage, error?: string) => {
      waited.current.set(slug, 0);
      commitTools((t) => (t.slug === slug ? { ...t, stage: next, error } : t));
    },
    [commitTools],
  );

  /**
   * One pass of the watch: where is each tool's process, and what does the
   * sweep say about the ones that are back.
   *
   * Both probes are read for the whole set rather than per row, for the reason
   * `routing_health` gives about its own: two rows in one pass must not be able
   * to disagree about shared infrastructure.
   */
  const tick = useCallback(async () => {
    const current = stageRef.current;
    if (current?.kind !== "work") return;
    const slugs = current.tools.map((t) => t.slug);
    let presence = new Map<string, ReopenPresence>();
    try {
      const { agents } = await runningAgents(slugs);
      presence = new Map(
        slugs.map((slug) => {
          const mine = agents.filter((a) => a.slug === slug);
          if (mine.length === 0) return [slug, "gone" as ReopenPresence];
          return [
            slug,
            mine.some((a) => a.predates_routing) ? "stale" : "fresh",
          ] as [string, ReopenPresence];
        }),
      );
    } catch (err) {
      // The scan is the half that says whether the tool is even open. Without
      // it nothing can be concluded, so the pass is skipped rather than
      // resolved on the sweep alone.
      trackError(err, "close_agents");
      return;
    }
    const verdicts = await verdictMap();
    commitTools((tool) => {
      const at = (waited.current.get(tool.slug) ?? 0) + 1;
      waited.current.set(tool.slug, at);
      const verdict = verdicts.get(tool.slug);
      const stage = nextStage(
        tool,
        verdict,
        presence.get(tool.slug) ?? "gone",
        at,
      );
      if (stage !== tool.stage) waited.current.set(tool.slug, 0);
      return {
        ...tool,
        stage,
        running: presence.get(tool.slug) !== "gone",
        // Kept current: the routes move as the tool comes back, and a card still
        // naming the pre-close pair would describe a moment that has passed.
        routeInUse: verdict?.route_in_use ?? tool.routeInUse,
        requestedRoute: verdict?.requested_route ?? tool.requestedRoute,
      };
    });
  }, [verdictMap, commitTools]);

  const tickRef = useRef(tick);
  tickRef.current = tick;

  const watching = stage?.kind === "work" && !allSettled(stage.tools);
  /** Is Gate itself doing something, or is it waiting for the user to open a
   *  tool? The first deserves the fast cadence; the second does not. */
  const acting =
    stage?.kind === "work" && stage.tools.some((t) => !isResting(t.stage));
  useEffect(() => {
    if (!watching) return;
    const id = setInterval(
      () => void tickRef.current(),
      acting ? WATCH_MS : IDLE_WATCH_MS,
    );
    return () => clearInterval(id);
  }, [watching, acting]);

  const checkNow = useCallback(async () => {
    await tickRef.current();
  }, []);

  const closeApps = useCallback(async () => {
    if (stage?.kind !== "confirm" || busy) return;
    setBusy(true);
    const tools = stage.tools;
    waited.current = new Map();
    commit({
      kind: "work",
      slugs: stage.slugs,
      tools: tools.map((t) => ({ ...t, stage: "closing" as ReopenStage })),
    });
    try {
      // The same filter the offer was built from, narrowed to the rows on
      // screen. Killing a wider set than the one the user agreed to would
      // signal processes they were never shown.
      const closed = await closeRunningAgents(tools.map((t) => t.slug));
      track("agents_closed", { count: closed });
      // Not "done": the signal was sent, and whether the process went, came
      // back and routes is what the watch is for. A tool Gate could relaunch
      // would go to `reopening` here; none can, so they all wait for the user.
      commitTools((t) => ({
        ...t,
        stage: t.canReopen ? "reopening" : "awaiting_reopen",
      }));
      await tickRef.current();
    } catch (err) {
      onError?.(err);
      trackError(err, "close_agents");
      // Every row failed together: the command signals the whole set, so
      // nothing here can say which one it stopped at.
      const detail = err instanceof Error ? err.message : String(err);
      commitTools((t) => ({
        ...t,
        stage: "close_failed" as ReopenStage,
        error: detail,
      }));
    } finally {
      setBusy(false);
    }
  }, [stage, busy, onError, commit, commitTools]);

  const dismiss = useCallback(() => commit(null), [commit]);

  return {
    stage,
    busy,
    offerAfterChange,
    goToConfirm,
    goBack,
    closeApps,
    markStage,
    checkNow,
    dismiss,
  };
}
