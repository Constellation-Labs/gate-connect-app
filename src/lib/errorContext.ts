/**
 * The state Gate was in when something failed, for the analytics error events
 * and nothing else (AG-603).
 *
 * **Errors only, deliberately.** These could ride every event as PostHog
 * super-properties, and should not: a routing verdict on a `popover_opened` is
 * noise on the volume events, and the reason to send any of it is that a
 * *failure* is unreadable without the state around it. "Codex stopped routing"
 * is answerable from `verdict_states` and `feed_state`; nothing about a tour
 * being skipped is.
 *
 * **Nothing identifying.** The install id here is the same anonymous per-machine
 * id the gateway already sees on `x-gate-install-id`; it names a machine, not a
 * person. The two fields the ticket lists that would break that - the
 * installation *name* (the device name, routinely "someone's MacBook") and the
 * selected organization id - are deliberately absent, so `analytics.ts`'s
 * anonymous-only posture and the disclosure's "no name, email, or account
 * identifier" both stay true. A report the user sends by hand carries those,
 * once, on purpose; the automatic stream does not.
 *
 * **A cached snapshot, read synchronously.** This is the `currentPlatform()`
 * pattern, and the reason is harder here: `routingVerdicts()` does network I/O
 * and walks the process table, so resolving any of this at the moment of a
 * failure would put a slow call on the error path of an app that is already
 * misbehaving. The shell pushes state in as it changes; `errorContext()` only
 * reads what is already in memory and never awaits.
 *
 * Every field is optional and absent until something sets it. A missing field
 * is a smaller lie than a stale or invented one, and an error thrown in the
 * first seconds of launch legitimately has no verdicts to report.
 */
import type { Props } from "./analytics";
import type { FeedState, Tool, Verdict } from "./api";
import { installId, osName } from "./api";

/** What the last push left here. Module-level because the readers are not React
 *  and must not await. */
let context: Props = {};

/**
 * Resolve the two fields that never change for the life of the process.
 *
 * Best-effort and unawaited by its caller: a failed lookup drops one field, and
 * an error event with a hole in it is worth more than a launch that blocked on
 * one. Called alongside `initAnalytics`, and deliberately NOT gated on consent -
 * this only fills a local object, and `trackError` is what decides whether any
 * of it is allowed to leave.
 */
export async function initErrorContext(): Promise<void> {
  const [id, os] = await Promise.all([
    installId().catch(() => null),
    osName().catch(() => null),
  ]);
  if (id) context.install_id = id;
  if (os) context.os_version = os;
}

/**
 * Push what the shell knows. Called from an effect on the state it summarises,
 * so the snapshot tracks the window rather than being rebuilt per error.
 *
 * The verdict summary is `slug:state` per tool, plus `:reason` where there is
 * one, sorted so two reports from the same machine compare as strings. Bounded
 * by the registry, which is a handful of tools - this is a summary, not a dump.
 */
export function setRoutingContext(input: {
  tools: Tool[];
  verdicts: Map<string, Verdict>;
  routingOn: boolean | null;
  feedState: FeedState;
}): void {
  const detected = input.tools
    .filter((t) => t.status.kind !== "not_installed")
    .map((t) => t.slug)
    .sort();
  context = {
    ...context,
    tools_detected: detected.join(","),
    verdict_states: [...input.verdicts.values()]
      .map((v) => (v.reason ? `${v.slug}:${v.state}:${v.reason}` : `${v.slug}:${v.state}`))
      .sort()
      .join(","),
    feed_state: input.feedState,
    ...(input.routingOn === null ? {} : { routing_on: input.routingOn }),
  };
}

/** The snapshot, for `trackError` to merge under its own allowlist. */
export function errorContext(): Props {
  return context;
}

/** Test seam. Nothing in the app clears this - the process outlives every
 *  error - but a suite that did not would leak one test's state into the next. */
export function resetErrorContext(): void {
  context = {};
}
