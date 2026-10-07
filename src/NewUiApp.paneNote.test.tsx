import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { defaultState } from "../e2e/backend";
import { installFakeTauri } from "../e2e/install";
import { NewUiApp } from "./NewUiApp";

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

async function openClaude() {
  render(<NewUiApp />);
  const [railRow] = await screen.findAllByRole("button", { name: /^Claude/ });
  fireEvent.click(railRow);
}

describe("the partly protected card", () => {
  it("names Claude Code when it is the surface that is off, and turns on only it", async () => {
    const state = routingState();
    state.proxy.domains.find((d) => d.slug === "anthropic")!.enabled = true;
    installFakeTauri(state);
    await openClaude();

    const card = await screen.findByText("Claude isn’t fully protected");
    const note = card.closest("[role=status]")!;
    expect(note.textContent).toContain("Claude Code isn’t routed through Gate");
    // Amber, not the neutral info card it replaced.
    expect(note.className).toContain("bg-amber-50");

    fireEvent.click(screen.getByRole("button", { name: "Turn on Claude Code" }));

    await waitFor(() => expect(callsTo("connect_tool")).toEqual([{ slug: "claude-code" }]));
    expect(callsTo("proxy_set_domain")).toEqual([]);
  });

  it("names Claude Desktop when it is the surface that is off, and leaves claude.ai alone", async () => {
    const state = routingState();
    state.tools.find((t) => t.slug === "claude-code")!.status = { kind: "connected" };
    installFakeTauri(state);
    await openClaude();

    const card = await screen.findByText("Claude isn’t fully protected");
    expect(card.closest("[role=status]")!.textContent).toContain(
      "Claude Desktop isn’t routed through Gate",
    );

    fireEvent.click(screen.getByRole("button", { name: "Turn on Claude Desktop" }));

    await waitFor(() =>
      expect(callsTo("proxy_set_domain")).toEqual([{ slug: "anthropic", enabled: true }]),
    );
    expect(callsTo("proxy_set_domain").some((a) => a.slug === "claude-web")).toBe(false);
    expect(callsTo("connect_tool")).toEqual([]);
  });
});
