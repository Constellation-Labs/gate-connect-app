import { useCallback, useState } from "react";
import { closeRunningAgents, runningAgents } from "./api";
import { track, trackError } from "./analytics";

/**
 * What happens after a tool's config is rewritten while that tool is running.
 *
 * A running app read its configuration at launch, so the new route does not
 * apply to it until it restarts. Gate can close it but cannot reopen it, which
 * is the whole reason this is a conversation rather than something done quietly:
 * closing an editor mid-session is the user's call, not ours.
 *
 * Three stages, matching the design: offer, confirm, confirm-done. The offer's
 * primary is the *passive* option ("I will reopen later") - the destructive one
 * is deliberately the secondary, and only after a second confirmation does
 * anything actually get killed.
 *
 * Deliberately not part of `useRouting`. That hook's prompt is a **gate**: it
 * blocks a write until answered. This is the opposite - it runs after a
 * successful write and can be walked away from at any point without changing
 * what was saved.
 */

export type RunningAppsStage =
  /** Affected apps are running; offer to close them. */
  | { kind: "offer"; apps: string[] }
  /** "Close affected apps" pressed; confirm before signalling anything. */
  | { kind: "confirm"; apps: string[] }
  /** Closed. Names what was closed so the copy is not a guess. */
  | { kind: "done"; apps: string[] };

export interface RunningApps {
  stage: RunningAppsStage | null;
  busy: boolean;
  /** Probe, and open the sequence only if something is actually running. */
  offerAfterChange: () => Promise<void>;
  goToConfirm: () => void;
  goBack: () => void;
  closeApps: () => Promise<void>;
  dismiss: () => void;
}

export function useRunningApps({ onError }: { onError?: (err: unknown) => void } = {}): RunningApps {
  const [stage, setStage] = useState<RunningAppsStage | null>(null);
  const [busy, setBusy] = useState(false);

  /**
   * Called after a config write that succeeded. Nothing running means nothing to
   * close, and a dialog saying so would be a dialog about nothing.
   *
   * A failed probe stays silent rather than defaulting to showing: the popover
   * defaults the other way, but it is choosing whether to show *advice*, and
   * this sequence offers to kill processes. Guessing wrong here means offering
   * to close apps that may not be open.
   */
  const offerAfterChange = useCallback(async () => {
    try {
      const { agents } = await runningAgents();
      if (agents.length === 0) return;
      // Process names, deduplicated: two `claude` processes are one app to the
      // person reading this, and the pid is not something they can act on.
      const apps = [...new Set(agents.map((a) => a.name))];
      setStage({ kind: "offer", apps });
      track("routing_notice_shown");
    } catch (err) {
      trackError(err, "close_agents");
    }
  }, []);

  const goToConfirm = useCallback(() => {
    setStage((s) => (s?.kind === "offer" ? { kind: "confirm", apps: s.apps } : s));
  }, []);

  const goBack = useCallback(() => {
    setStage((s) => (s?.kind === "confirm" ? { kind: "offer", apps: s.apps } : s));
  }, []);

  const closeApps = useCallback(async () => {
    if (stage?.kind !== "confirm" || busy) return;
    setBusy(true);
    try {
      const closed = await closeRunningAgents();
      track("agents_closed", { count: closed });
      // Report what was asked for rather than the count: the backend signals
      // processes, and a name the user recognises beats a number they cannot
      // check. Zero closed still lands here - the apps are gone either way.
      setStage({ kind: "done", apps: stage.apps });
    } catch (err) {
      onError?.(err);
      trackError(err, "close_agents");
      // Stay on the confirmation: the apps are still open, and the user should
      // be able to try again or back out.
    } finally {
      setBusy(false);
    }
  }, [stage, busy, onError]);

  const dismiss = useCallback(() => setStage(null), []);

  return { stage, busy, offerAfterChange, goToConfirm, goBack, closeApps, dismiss };
}
