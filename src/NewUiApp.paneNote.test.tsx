import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { defaultState } from "../e2e/backend";
import { installFakeTauri } from "../e2e/install";
import { NewUiApp } from "./NewUiApp";
import { partlyProtectedCopy, upstreamFix } from "./lib/verdict";

/**
 * The app pane's card for a section that is only partly routing: it names the
 * surface that is off, and its button turns on that surface and nothing else -
 * in particular not claude.ai, which the app switch's cascade would sweep in.
 */
afterEach(cleanup);

/** Routing up, so an enabled domain is a routed one. */
function routingState() {
  const state = defaultState();
  state.proxy.running = true;
  state.proxy.port = 45980;
  state.proxy.ca_trusted = true;
  return state;
}

const callsTo = (cmd: string) =>
  window.__GATE_E2E__.calls.filter((c) => c.cmd === cmd).map((c) => c.args);

async function openApp(name: RegExp) {
  render(<NewUiApp />);
  const [railRow] = await screen.findAllByRole("button", { name });
  fireEvent.click(railRow);
}

/**
 * Click the card's button and wait for the whole click to land, not its first
 * write: the card goes once the re-read says every governing surface routes,
 * and that re-read runs after the last member. Asserting on the first write
 * alone would pass a card that also routed claude.ai a moment later.
 */
async function routeAndSettle(title: string) {
  fireEvent.click(screen.getByRole("button", { name: "Route it" }));
  await waitFor(() => expect(screen.queryByText(title)).toBeNull());
}

describe("the partly protected card", () => {
  it("names Claude Code when it is the surface that is off, and turns on only it", async () => {
    const state = routingState();
    state.proxy.domains.find((d) => d.slug === "anthropic")!.enabled = true;
    installFakeTauri(state);
    await openApp(/^Claude/);

    const card = await screen.findByText("Claude isn’t fully protected");
    const note = card.closest("[role=status]")!;
    expect(note.textContent).toContain("Claude Code isn’t routed through Gate");
    // Amber, not the neutral info card it replaced.
    expect(note.className).toContain("bg-amber-50");
    // Filled primary, as `ReopenAlert`'s Close tool is: the one fix on an
    // amber card.
    expect(screen.getByRole("button", { name: "Route it" }).className).toContain(
      "bg-base-primary",
    );

    await routeAndSettle("Claude isn’t fully protected");

    expect(callsTo("connect_tool")).toEqual([{ slug: "claude-code" }]);
    expect(callsTo("proxy_set_domain")).toEqual([]);
  });

  it("names Claude Desktop when it is the surface that is off, and leaves claude.ai alone", async () => {
    const state = routingState();
    state.tools.find((t) => t.slug === "claude-code")!.status = { kind: "connected" };
    installFakeTauri(state);
    await openApp(/^Claude/);

    const card = await screen.findByText("Claude isn’t fully protected");
    expect(card.closest("[role=status]")!.textContent).toContain(
      "Claude Desktop isn’t routed through Gate",
    );

    await routeAndSettle("Claude isn’t fully protected");

    expect(callsTo("proxy_set_domain")).toEqual([{ slug: "anthropic", enabled: true }]);
    expect(callsTo("connect_tool")).toEqual([]);
  });

  /**
   * The one shape where the button reaches a signed-in surface. Without the
   * Codex CLI the ChatGPT section has no brokered member, so its two chatgpt.com
   * surfaces govern it, and the card names and routes the one that is off.
   * Pinned so the comment in `statusNote` that says so stays true.
   */
  it("routes a signed-in ChatGPT surface when the section has nothing else", async () => {
    const state = routingState();
    state.tools = state.tools.filter((t) => t.slug !== "codex");
    state.proxy.domains.find((d) => d.slug === "chatgpt-apps")!.enabled = true;
    installFakeTauri(state);
    await openApp(/^ChatGPT/);

    const card = await screen.findByText("ChatGPT / Codex isn’t fully protected");
    expect(card.closest("[role=status]")!.textContent).toContain(
      "ChatGPT Work isn’t routed through Gate",
    );

    await routeAndSettle("ChatGPT / Codex isn’t fully protected");

    expect(callsTo("proxy_set_domain")).toEqual([{ slug: "chatgpt", enabled: true }]);
    expect(callsTo("connect_tool")).toEqual([]);
  });

  /**
   * The card is a click like the switch is, so it clears the last failure the
   * way `toggle` does. Without that a retry that worked left the failure on
   * screen over a section that had just come back. A domain's failure, because
   * that is the one the window banner carries; a tool's goes to its own pane
   * card, which clears itself on success.
   */
  it("clears the last failure when its retry goes through", async () => {
    const state = routingState();
    state.tools.find((t) => t.slug === "claude-code")!.status = { kind: "connected" };
    state.failures.proxy_set_domain = "permission denied";
    installFakeTauri(state);
    await openApp(/^Claude/);

    await screen.findByText("Claude isn’t fully protected");
    fireEvent.click(screen.getByRole("button", { name: "Route it" }));
    await screen.findByText(/permission denied/);

    delete state.failures.proxy_set_domain;
    await routeAndSettle("Claude isn’t fully protected");

    expect(screen.queryByText(/permission denied/)).toBeNull();
  });
});

/**
 * The card for an app routed through Gate to a provider whose domain is off:
 * its button turns that domain on. Hermes is the app that reports this.
 */
describe("the uninspected provider card", () => {
  function hermesState(switchedOff: { slug: string; hosts: string[]; tools: string[] }[]) {
    const state = routingState();
    state.tools.push({
      slug: "hermes",
      name: "CLI",
      displayName: "Hermes",
      upstream_provider_name: "your existing providers",
      default_upstream_url: "https://openrouter.ai/api/v1",
      status: { kind: "connected" },
      coverage: { defaulted: false, switched_off: switchedOff, unknown: [] },
    });
    return state;
  }

  it("turns on OpenRouter and records it as Hermes'", async () => {
    const state = hermesState([{ slug: "openrouter", hosts: ["openrouter.ai"], tools: [] }]);
    installFakeTauri(state);
    await openApp(/^Hermes/);

    const card = await screen.findByText("Hermes isn’t protected");
    expect(card.closest("[role=status]")!.textContent).toContain(
      "Gate can’t see requests to openrouter.ai while its provider is turned off.",
    );

    fireEvent.click(screen.getByRole("button", { name: "Turn it on" }));
    await waitFor(() =>
      expect(callsTo("proxy_set_domain")).toEqual([{ slug: "openrouter", enabled: true }]),
    );
    // Recorded, so turning Hermes off gives it back: it has no row of its own.
    await waitFor(() =>
      expect(state.preferences.auto_enabled_domains.hermes).toEqual(["openrouter"]),
    );
  });

  it("offers no button for a provider with its own row", async () => {
    // Turning `anthropic` on intercepts it for every client on the machine;
    // that is the Claude row's decision, not a card's on Hermes' pane.
    const state = hermesState([
      { slug: "anthropic", hosts: ["api.anthropic.com"], tools: ["Claude Code"] },
    ]);
    installFakeTauri(state);
    await openApp(/^Hermes/);

    const card = await screen.findByText("Hermes isn’t protected");
    expect(card.closest("[role=status]")!.textContent).toContain(
      "Gate can’t see requests to api.anthropic.com while its provider is turned off.",
    );
    expect(screen.queryByRole("button", { name: /^Turn (it|them) on$/ })).toBeNull();
  });

  it("offers no button when coverage is not why Hermes is amber", async () => {
    // Coverage arrives whatever the verdict; an overridden Hermes has a
    // different problem, and a button turning on OpenRouter would not fix it.
    const state = hermesState([{ slug: "openrouter", hosts: ["openrouter.ai"], tools: [] }]);
    state.tools.find((t) => t.slug === "hermes")!.status = { kind: "overridden" } as never;
    installFakeTauri(state);
    await openApp(/^Hermes/);

    const card = await screen.findByText("Hermes isn’t protected");
    expect(card.closest("[role=status]")!.textContent).toContain("Configuration overridden");
    expect(screen.queryByRole("button", { name: "Turn it on" })).toBeNull();
  });
});

describe("upstreamFix", () => {
  it("has nothing to offer for an unknown provider alone", () => {
    expect(upstreamFix({ defaulted: false, switched_off: [], unknown: ["api.groq.com"] })).toBe(
      undefined,
    );
  });

  it("offers only the domains a tool owns", () => {
    expect(
      upstreamFix({
        defaulted: false,
        switched_off: [
          { slug: "openrouter", hosts: ["openrouter.ai"], tools: [] },
          { slug: "openai", hosts: ["api.openai.com"], tools: [] },
        ],
        unknown: [],
      }),
    ).toEqual({ slugs: ["openrouter"], label: "Turn it on" });
    expect(
      upstreamFix({
        defaulted: false,
        switched_off: [{ slug: "chatgpt", hosts: ["chatgpt.com"], tools: [] }],
        unknown: [],
      }),
    ).toBe(undefined);
  });
});

describe("partlyProtectedCopy", () => {
  it("speaks of one surface in the singular", () => {
    expect(partlyProtectedCopy("Claude", ["Claude Code"])).toEqual({
      title: "Claude isn’t fully protected",
      body: "Claude Code isn’t routed through Gate, so its traffic goes straight to the provider.",
      label: "Route it",
    });
  });

  it("joins two with 'and', and three with commas before it", () => {
    const two = partlyProtectedCopy("App", ["A", "B"]);
    expect(two.body).toBe(
      "A and B aren’t routed through Gate, so their traffic goes straight to the provider.",
    );
    expect(two.label).toBe("Route them");
    expect(partlyProtectedCopy("App", ["A", "B", "C"]).body).toMatch(/^A, B and C aren’t /);
  });
});
