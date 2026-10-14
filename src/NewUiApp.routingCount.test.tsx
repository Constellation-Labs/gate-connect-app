import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { defaultState } from "../e2e/backend";
import { installFakeTauri } from "../e2e/install";
import { NewUiApp } from "./NewUiApp";
import { TrayApp } from "./TrayApp";

/**
 * The topbar, the rail and the tray agree on the routed count (AG-912).
 *
 * Each surface counts for itself. The topbar banner takes `railApps` in
 * `NewUiApp`, the tray card takes its own groups in `Tray.RoutingCard`, and
 * every eyebrow counts its own rows. So one backend state feeds both shells
 * here, and each case reads what every surface prints and holds it against
 * the others.
 *
 * The eyebrows count every row in their group, switched on or not, where the
 * banner's numerator is routed among the rows switched on. Those are the same
 * number: a row only reads Protected when every member its switch governs is
 * routing, and a routing member is one the person asked for. So the eyebrows
 * must add up to the banner, on both halves.
 */
afterEach(cleanup);

type State = ReturnType<typeof defaultState>;

interface Fraction {
  routed: number;
  available: number;
}

interface Reading extends Fraction {
  headline: string;
  noun: string;
  /** Each eyebrow's "N of M", by its label. */
  eyebrows: Record<string, Fraction>;
}

const FRACTION = /^(\d+) of (\d+)$/;
/** The banner's right half: "Protected · 1 of 3 Apps". */
const BANNER = /^(.+) · (\d+) of (\d+) (\S+)$/;
/** The tray card's sub-line, with or without an engine note in front. */
const CARD = /^(?:.+ · )?(\d+) of (\d+) (\S+)$/;

/** The eyebrows under `root`, as the rail and the tray both draw them. */
function eyebrows(root: Element): Record<string, Fraction> {
  const out: Record<string, Fraction> = {};
  for (const h of root.querySelectorAll("h2")) {
    const counter = h.nextElementSibling?.textContent ?? "";
    const m = FRACTION.exec(counter);
    // "Not installed" carries no counter on purpose: nobody has those apps.
    if (m) out[h.textContent!] = { routed: Number(m[1]), available: Number(m[2]) };
  }
  return out;
}

function sum(groups: Record<string, Fraction>): Fraction {
  return Object.values(groups).reduce(
    (a, g) => ({ routed: a.routed + g.routed, available: a.available + g.available }),
    { routed: 0, available: 0 },
  );
}

/** The window: the topbar banner and the rail's eyebrows. */
async function readWindow(state: State, want: Fraction): Promise<Reading> {
  installFakeTauri(state);
  const { container } = render(<NewUiApp />);
  await screen.findAllByRole("button", { name: /^Claude/ });
  let reading: Reading | undefined;
  // Settled once the sweep has answered and the banner prints the case's count.
  await waitFor(
    () => {
      const lines = [...container.querySelectorAll("p")].filter((p) =>
        BANNER.test(p.textContent ?? ""),
      );
      expect(lines).toHaveLength(1);
      const [, , routed, available, noun] = BANNER.exec(lines[0].textContent!)!;
      reading = {
        // The banner's left half: the tile, then the sentence.
        headline: lines[0].previousElementSibling?.querySelector("p")?.textContent ?? "",
        routed: Number(routed),
        available: Number(available),
        noun,
        eyebrows: eyebrows(container.querySelector('nav[aria-label="Main"]')!),
      };
      expect({ routed: reading.routed, available: reading.available }).toEqual(want);
    },
    { timeout: 3000 },
  );
  cleanup();
  return reading!;
}

/** The tray: its routing card and its eyebrows. */
async function readTray(state: State, want: Fraction): Promise<Reading> {
  installFakeTauri({ ...state, windowLabel: "tray" });
  const { container } = render(<TrayApp />);
  await screen.findByRole("switch", { name: "Claude" });
  let reading: Reading | undefined;
  await waitFor(
    () => {
      const h1 = container.querySelector("h1")!;
      const m = CARD.exec(h1.nextElementSibling?.textContent ?? "");
      expect(m).not.toBeNull();
      reading = {
        headline: h1.textContent!,
        routed: Number(m![1]),
        available: Number(m![2]),
        noun: m![3],
        eyebrows: eyebrows(container),
      };
      expect({ routed: reading.routed, available: reading.available }).toEqual(want);
    },
    { timeout: 3000 },
  );
  cleanup();
  return reading!;
}

/** Switch on the named tools and domains, with the engine up and trusted so
 *  a switched-on row routes. */
function routing(slugs: string[], edit: (s: State) => void = () => {}): State {
  const s = defaultState();
  s.proxy.running = true;
  s.proxy.ca_trusted = true;
  for (const d of s.proxy.domains) if (slugs.includes(d.slug)) d.enabled = true;
  for (const t of s.tools) if (slugs.includes(t.slug)) t.status = { kind: "connected" };
  edit(s);
  return s;
}

const CLAUDE = ["claude-code", "anthropic"];
const EVERY_APP = [...CLAUDE, "codex", "opencode"];
const CHATS = ["claude-web", "chatgpt-apps"];

const CASES: {
  name: string;
  state: () => State;
  routed: number;
  available: number;
  headline: string;
}[] = [
  {
    name: "nothing enabled",
    state: () => routing([]),
    routed: 0,
    available: 3,
    headline: "No apps are routed",
  },
  {
    name: "some enabled, all of it routed",
    state: () => routing(CLAUDE),
    routed: 1,
    available: 3,
    headline: "Gate Connect is protecting you",
  },
  {
    name: "some enabled, one of them blocked",
    state: () =>
      routing(CLAUDE, (s) => {
        const codex = s.tools.find((t) => t.slug === "codex")!;
        codex.status = { kind: "drifted", reason: "rewritten" };
      }),
    routed: 1,
    available: 3,
    headline: "Gate Connect is partly routing your apps",
  },
  {
    name: "everything enabled, chat rows switched off",
    state: () => routing(EVERY_APP),
    routed: 3,
    available: 3,
    headline: "Gate Connect is protecting you",
  },
  {
    name: "everything enabled, chat rows opted in",
    state: () => routing([...EVERY_APP, ...CHATS]),
    routed: 3,
    available: 3,
    headline: "Gate Connect is protecting you",
  },
  {
    // A session surface alone does not make its app routed (`sectionStatus`),
    // so all three surfaces must say so together.
    name: "a chat row opted in on its own",
    state: () => routing(["claude-web"]),
    routed: 0,
    available: 3,
    headline: "No apps are routed",
  },
  {
    // A CLI-only row is drawn once it is on, so the denominator grows.
    name: "an opted-in row that adds to the denominator",
    state: () => routing(["openai"]),
    routed: 1,
    available: 4,
    headline: "Gate Connect is protecting you",
  },
];

describe("the topbar, the rail and the tray agree on the routed count", () => {
  it.each(CASES)("$name", async ({ state, routed, available, headline }) => {
    const want = { routed, available };
    // One backend state, read by both shells.
    const fixture = state();
    const window = await readWindow(structuredClone(fixture), want);
    const tray = await readTray(structuredClone(fixture), want);

    // Numerator and denominator, every surface against every other.
    expect({ routed: tray.routed, available: tray.available }).toEqual(want);
    expect(sum(window.eyebrows)).toEqual(want);
    expect(sum(tray.eyebrows)).toEqual(want);
    expect(tray.eyebrows).toEqual(window.eyebrows);

    // The state behind the numbers, from the one module both read.
    expect(window.headline).toBe(headline);
    expect(tray.headline).toBe(headline);

    // The noun. Both shells name the population "Apps" (decided
    // 2026-10-13), so the topbar and the tray card read the same.
    expect(window.noun).toBe("Apps");
    expect(tray.noun).toBe("Apps");
  });
});
