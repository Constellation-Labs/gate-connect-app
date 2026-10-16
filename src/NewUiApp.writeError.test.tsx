import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { defaultState } from "../e2e/backend";
import { installFakeTauri } from "../e2e/install";
import { NewUiApp } from "./NewUiApp";

/**
 * The card a failed switch-on leaves on the app's pane. Its switch tries the
 * same write again, which is the fix for a write that failed, and no fix at all
 * for a connect the tool refused over something only the user can change: the
 * retry is refused the same way and the card redraws unchanged, which read as a
 * switch that does nothing.
 */
afterEach(cleanup);

function routingState() {
  const state = defaultState();
  state.proxy.running = true;
  state.proxy.port = 45980;
  state.proxy.ca_trusted = true;
  return state;
}

async function switchOnOpenCode() {
  render(<NewUiApp />);
  const [railRow] = await screen.findAllByRole("button", { name: /^OpenCode/ });
  fireEvent.click(railRow);
  fireEvent.click(await screen.findByRole("switch", { name: "Route OpenCode" }));
  // OpenCode and the terminal route together, so the switch asks first.
  fireEvent.click(await screen.findByRole("button", { name: "Turn on" }));
  return screen.findByText("Couldn’t connect this tool");
}

describe("a failed switch-on", () => {
  it("offers no retry when OpenCode refused for want of a provider", async () => {
    const state = routingState();
    state.failures.connect_tool =
      "Sign in to a provider in OpenCode first, or put it on Gate models, then turn this on again.";
    installFakeTauri(state);
    await switchOnOpenCode();

    expect(
      screen.getByText(
        "Sign in to a provider in OpenCode first, or choose Gate models for it, then try again.",
      ),
    ).toBeTruthy();
    expect(screen.queryByRole("switch", { name: "Try OpenCode again" })).toBeNull();
    expect(screen.getByRole("button", { name: "Dismiss alert" })).toBeTruthy();
  });

  it("still offers the retry for a write that failed", async () => {
    const state = routingState();
    state.failures.connect_tool = "failed to write opencode.json: permission denied";
    installFakeTauri(state);
    await switchOnOpenCode();

    expect(screen.getByRole("switch", { name: "Try OpenCode again" })).toBeTruthy();
  });
});
