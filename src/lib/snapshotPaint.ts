import type { ProxyState, Tool, Verdict } from "./api";

/**
 * Serialize a sweep: one at a time, in the order asked.
 *
 * Three callers ask for the routing sweep, and two replies crossing meant an
 * older reading could land last and stand until the next ask. A failed sweep
 * resolves to `null` rather than rejecting, and does not block the next.
 */
export function serialSweeps<T>(sweep: () => Promise<T | null>): () => Promise<T | null> {
  let queue: Promise<unknown> = Promise.resolve();
  return () => {
    const run = queue.then(() => sweep().catch(() => null));
    queue = run.catch(() => null);
    return run;
  };
}

/** What the window holds now, so the plan can tell what moved. */
export interface Held {
  tools: Tool[];
  proxy: ProxyState | null;
  verdicts: Map<string, Verdict>;
}

/** A fresh reading. `null` leaves that part as it is. */
export interface Reading {
  tools: Tool[] | null;
  proxy: ProxyState | null;
}

/** What to set, as one update. `null` means leave it. */
export interface Paint {
  tools: Tool[] | null;
  proxy: ProxyState | null;
  verdicts: Map<string, Verdict> | null;
}

/**
 * Decide what a reading paints, given whether its sweep answered.
 *
 * With verdicts, all three land together: the switch, the row and the pane
 * move at once, and nothing in between is drawn.
 *
 * Without them, the reading still lands. A write that landed must move the
 * switch, or the next click undoes it. But no tool whose status moved keeps
 * the verdict from before the move: that pairing (new intent, old verdict)
 * is "Not protected" with a banner, and the sweep never said so. Those tools
 * read "Checking" instead, which is the truth and raises no banner. The
 * engine coming up or going down moves every verdict, so all of them go.
 */
export function paintPlan(held: Held, reading: Reading, swept: Map<string, Verdict> | null): Paint {
  if (swept) return { tools: reading.tools, proxy: reading.proxy, verdicts: swept };

  const engineMoved =
    !!reading.proxy && !!held.proxy && reading.proxy.running !== held.proxy.running;
  if (engineMoved) {
    return { tools: reading.tools, proxy: reading.proxy, verdicts: new Map() };
  }

  if (!reading.tools) return { tools: null, proxy: reading.proxy, verdicts: null };
  const before = new Map(held.tools.map((t) => [t.slug, t.status.kind]));
  const moved = reading.tools.filter((t) => before.get(t.slug) !== t.status.kind);
  if (moved.length === 0) return { tools: reading.tools, proxy: reading.proxy, verdicts: null };
  const verdicts = new Map(held.verdicts);
  for (const t of moved) verdicts.delete(t.slug);
  return { tools: reading.tools, proxy: reading.proxy, verdicts };
}
