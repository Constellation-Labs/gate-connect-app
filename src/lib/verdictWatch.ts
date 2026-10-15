import { useEffect, useRef } from "react";
import type { Verdict } from "./api";
import { REOPEN_IDLE_WATCH_MS } from "./reopen";

/**
 * How often a shell re-checks while a tool reads Not protected for any reason
 * other than a pending reopen.
 *
 * Nothing else re-checks a failure. Sweeps ran after an action, on the reopen
 * watch, and on the running-apps stage, so a row that failed while the
 * forwarder was down kept reading "Not protected" for four hours after it was
 * back, until somebody toggled the app (AG-1097). Slower than the reopen
 * watch: a pending reopen resolves the moment someone opens a terminal, while
 * a failure usually waits on something outside the app, and every sweep
 * probes the relay and, for a signed-in account, the gateway session.
 */
export const FAILURE_RECHECK_MS = 30_000;

/**
 * How often the shell should re-check its verdicts, or `null` for not at all.
 *
 * A pending reopen keeps its own cadence and wins: it is the faster of the
 * two and was already the rule. Otherwise any tool that needs attention arms
 * the slower failure re-check, so the row clears on its own once the cause
 * is gone. A healthy app adds no probes.
 */
export function recheckCadence(verdicts: Iterable<Verdict>): number | null {
  let failing = false;
  for (const v of verdicts) {
    if (v.reason === "reopen_required") return REOPEN_IDLE_WATCH_MS;
    if (v.state === "needs_attention") failing = true;
  }
  return failing ? FAILURE_RECHECK_MS : null;
}

/**
 * Re-run `refresh` on the cadence {@link recheckCadence} chose.
 *
 * The failure re-check pauses while the window is hidden, and runs once as it
 * becomes visible again: a row nobody can see does not need probing every
 * 30s, and the moment someone looks is the moment it has to be right. The
 * reopen watch is left as it was, running hidden or not.
 */
export function useVerdictWatch(cadence: number | null, refresh: () => void): void {
  const latest = useRef(refresh);
  latest.current = refresh;

  useEffect(() => {
    if (cadence === null) return;
    const pausesWhenHidden = cadence === FAILURE_RECHECK_MS;
    const id = setInterval(() => {
      if (pausesWhenHidden && document.hidden) return;
      latest.current();
    }, cadence);
    const onVisible = () => {
      if (pausesWhenHidden && !document.hidden) latest.current();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      clearInterval(id);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [cadence]);
}
