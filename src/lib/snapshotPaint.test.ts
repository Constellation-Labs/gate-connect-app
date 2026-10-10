import { describe, expect, it } from "vitest";
import type { ProxyState, Tool, Verdict } from "./api";
import { paintPlan, serialSweeps } from "./snapshotPaint";

const tool = (slug: string, kind: "detected" | "connected"): Tool =>
  ({ slug, name: slug, status: { kind } }) as unknown as Tool;
const proxy = (running: boolean): ProxyState => ({ running }) as unknown as ProxyState;
const verdict = (slug: string, state: "on" | "off"): Verdict =>
  ({ slug, state }) as unknown as Verdict;
const map = (...vs: Verdict[]) => new Map(vs.map((v) => [v.slug, v]));

describe("serialSweeps", () => {
  it("answers in the order asked, even when a later ask resolves first", async () => {
    const pending: Array<(v: number | null) => void> = [];
    const sweep = serialSweeps<number>(() => new Promise((resolve) => pending.push(resolve)));
    const order: number[] = [];
    const a = sweep().then((v) => order.push(v!));
    const b = sweep().then((v) => order.push(v!));
    // The queue starts a sweep on a microtask. The second ask has not even
    // started: the first holds it.
    await Promise.resolve();
    expect(pending).toHaveLength(1);
    pending[0]!(1);
    await a;
    await Promise.resolve();
    expect(pending).toHaveLength(2);
    pending[1]!(2);
    await b;
    expect(order).toEqual([1, 2]);
  });

  it("turns a rejected sweep into null without blocking the next", async () => {
    let calls = 0;
    const sweep = serialSweeps<number>(() => {
      calls += 1;
      return calls === 1 ? Promise.reject(new Error("down")) : Promise.resolve(7);
    });
    expect(await sweep()).toBeNull();
    expect(await sweep()).toBe(7);
  });
});

describe("paintPlan", () => {
  const held = {
    tools: [tool("codex", "detected"), tool("hermes", "connected")],
    proxy: proxy(true),
    verdicts: map(verdict("codex", "off"), verdict("hermes", "on")),
  };

  it("paints all three together when the sweep answered", () => {
    const swept = map(verdict("codex", "on"), verdict("hermes", "on"));
    const reading = { tools: [tool("codex", "connected"), tool("hermes", "connected")], proxy: proxy(true) };
    expect(paintPlan(held, reading, swept)).toEqual({ ...reading, verdicts: swept });
  });

  it("drops the stale verdict of a tool whose status moved when the sweep failed", () => {
    // Codex was turned on and its write landed. The old "off" verdict would
    // draw "Not protected" beside an On switch; "Checking" is what is known.
    const reading = { tools: [tool("codex", "connected"), tool("hermes", "connected")], proxy: null };
    const plan = paintPlan(held, reading, null);
    expect(plan.tools).toBe(reading.tools);
    expect(plan.verdicts?.has("codex")).toBe(false);
    expect(plan.verdicts?.get("hermes")).toEqual(verdict("hermes", "on"));
  });

  it("keeps every verdict when nothing moved and the sweep failed", () => {
    const reading = { tools: held.tools, proxy: proxy(true) };
    expect(paintPlan(held, reading, null)).toEqual({ ...reading, verdicts: null });
  });

  it("drops every verdict when the engine moved and the sweep failed", () => {
    const reading = { tools: null, proxy: proxy(false) };
    expect(paintPlan(held, reading, null)).toEqual({ tools: null, proxy: reading.proxy, verdicts: new Map() });
  });

  it("lands the first reading even without a sweep, so a failed sweep is not an empty machine", () => {
    const first = { tools: [], proxy: null, verdicts: new Map<string, Verdict>() };
    const reading = { tools: [tool("codex", "detected")], proxy: proxy(true) };
    const plan = paintPlan(first, reading, null);
    expect(plan.tools).toBe(reading.tools);
    expect(plan.proxy).toBe(reading.proxy);
  });
});
