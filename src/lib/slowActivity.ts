/**
 * Slow the activity reads down so their loading states can actually be looked at.
 *
 * The Overview's skeletons, the chart's placeholder columns and the tables'
 * pending rows all live in the window between asking and being answered. Locally
 * that window is a few milliseconds, and it cannot be widened from devtools the
 * usual way: these are Tauri IPC calls, not HTTP, so the network throttle does
 * nothing to them.
 *
 * `gcSlowActivity(3000)` in devtools, then reload. The delay is stored rather
 * than held in memory because the load being inspected is the one that fires
 * during startup, before there is a console to type into.
 *
 * The two reads are delayed separately on purpose - they are the two halves of
 * AG-576 and they show different things:
 *
 * - `gcSlowActivity(3000)` delays only the network read. On a scope that has
 *   never loaded, that is three seconds of skeletons. On one that has, the held
 *   reading paints off disk immediately and is replaced when the fetch lands,
 *   which is the behaviour the pane exists for.
 * - `gcSlowActivity(3000, 3000)` delays both, so the skeletons show even when a
 *   cache file is sitting there.
 * - `gcSlowActivity(0)` clears it.
 *
 * **Dev builds only.** Both the global and the delay itself are behind
 * `import.meta.env.DEV`, so a stray localStorage key cannot slow a real build
 * down - unlike `gcNewUi`, which is deliberately live in production because it
 * is a fallback rather than an instrument.
 */

const KEY = "gc.slowActivity";

interface Delays {
  /** Milliseconds before the network read resolves. */
  net: number;
  /** Milliseconds before the disk read resolves. */
  cache: number;
}

const NONE: Delays = { net: 0, cache: 0 };

function stored(): Delays {
  if (!import.meta.env.DEV) return NONE;
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return NONE;
    const parsed = JSON.parse(raw) as Partial<Delays>;
    return {
      net: Number(parsed.net) || 0,
      cache: Number(parsed.cache) || 0,
    };
  } catch {
    // Unreadable or unparseable. A dev instrument is not worth an exception on
    // the path every activity read takes.
    return NONE;
  }
}

/** Hold for the configured delay, or return immediately when there is none. */
function hold(ms: number): Promise<void> {
  return ms > 0 ? new Promise((resolve) => setTimeout(resolve, ms)) : Promise.resolve();
}

export const slowNetworkRead = () => hold(stored().net);
export const slowCacheRead = () => hold(stored().cache);

if (import.meta.env.DEV) {
  (window as unknown as Record<string, unknown>).gcSlowActivity = (net = 3000, cache = 0) => {
    try {
      localStorage.setItem(KEY, JSON.stringify({ net, cache }));
    } catch {
      // Same reasoning as `newUi`: a flag that cannot persist is not worth
      // taking the window down over.
    }
    return { net, cache, note: "reload the window to catch the first load" };
  };
}
