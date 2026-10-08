import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { defaultState } from "../e2e/backend";
import { installFakeTauri } from "../e2e/install";
import { NewUiApp } from "./NewUiApp";

/**
 * The "Code app" mark on the Gate models card: which model the Claude desktop
 * app's Code tab runs on. It follows the engine's own conditions for moving a
 * Code tab request (`code_tab_gate_models` in `proxy/engine.rs`), so it never
 * names a model the Code tab is not on.
 */
afterEach(cleanup);

const OPUS = "anthropic/claude-opus-5";
const KIMI = "moonshot/kimi-k3";

/** Claude Code connected on Gate models, on the second of its set (a `/model`
 *  pick), with the desktop app routed. */
function onGateModels() {
  const state = defaultState();
  state.proxy.running = true;
  state.proxy.port = 45980;
  state.proxy.ca_trusted = true;
  state.proxy.domains.find((d) => d.slug === "anthropic")!.enabled = true;
  state.tools.find((t) => t.slug === "claude-code")!.status = { kind: "connected" };
  state.toolModels.choices["claude-code"] = { source: "gate", model_ids: [OPUS, KIMI] };
  state.toolModels.paidAckUnix = 1_700_000_000;
  state.toolModels.configuredModel = { "claude-code": KIMI };
  return state;
}

async function modelCells() {
  render(<NewUiApp />);
  const [railRow] = await screen.findAllByRole("button", { name: /^Claude/ });
  fireEvent.click(railRow);
  const list = await screen.findByRole("list", { name: "Gate models for Claude" });
  return () => within(list).getAllByRole("listitem");
}

describe("the Code tab's model mark", () => {
  it("marks the model Claude Code starts on, which leads the card", async () => {
    installFakeTauri(onGateModels());
    const cells = await modelCells();

    await waitFor(() => expect(within(cells()[0]).getByText("Code app")).toBeTruthy());
    expect(cells()[0].textContent).toContain(KIMI);
    expect(within(cells()[1]).queryByText("Code app")).toBeNull();
  });

  /**
   * A config naming a model the set no longer has. The engine then starts the
   * Code tab on the set's first model, so that is the one the mark names
   * rather than none at all.
   */
  it("marks the set's first model when the config names one outside it", async () => {
    const state = onGateModels();
    state.toolModels.configuredModel = { "claude-code": "retired/model" };
    installFakeTauri(state);
    const cells = await modelCells();

    await waitFor(() => expect(within(cells()[0]).getByText("Code app")).toBeTruthy());
    expect(cells()[0].textContent).toContain(OPUS);
    expect(within(cells()[1]).queryByText("Code app")).toBeNull();
  });

  it("is not drawn while the desktop app is not routed", async () => {
    const state = onGateModels();
    state.proxy.domains.find((d) => d.slug === "anthropic")!.enabled = false;
    installFakeTauri(state);
    const cells = await modelCells();

    await waitFor(() => expect(cells()[0].textContent).toContain(KIMI));
    expect(screen.queryByText("Code app")).toBeNull();
  });

  it("is not drawn while Claude Code is switched off", async () => {
    const state = onGateModels();
    state.tools.find((t) => t.slug === "claude-code")!.status = { kind: "detected" };
    installFakeTauri(state);
    const cells = await modelCells();

    // Off, the config holds nothing, so the card falls back to the set's order.
    await waitFor(() => expect(cells()[0].textContent).toContain(OPUS));
    expect(screen.queryByText("Code app")).toBeNull();
  });
});
