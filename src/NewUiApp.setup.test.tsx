import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { defaultState } from "../e2e/backend";
import { installFakeTauri } from "../e2e/install";

vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn() }));

import { relaunch } from "@tauri-apps/plugin-process";
import { NewUiApp } from "./NewUiApp";

/**
 * An OAuth account whose gateway refuses the session strands the user on the
 * org picker, where Settings (and its Change server) is out of reach. These pin
 * the way back: the setup panes offer the same confirmed switch Settings does.
 * The staging and dev servers are listed because vitest runs as a dev build.
 */
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const REFUSED =
  'gateway /v1/me/orgs returned 401 Unauthorized: {"error":{"code":"invalid_gate_token"}}';
const STAGING = "https://gateway-staging.constellationgate.ai";

function bootStuckOnOrgPicker(edit: (s: ReturnType<typeof defaultState>) => void = () => {}) {
  const state = defaultState();
  state.account!.org_id = null;
  state.account!.org_name = null;
  state.failures.oauth_list_orgs = REFUSED;
  edit(state);
  installFakeTauri(state);
  render(<NewUiApp />);
  return state;
}

describe("NewUiApp setup: a gateway that refuses the session", () => {
  it("titles the refusal as a session, and points at the panes on screen", async () => {
    bootStuckOnOrgPicker();

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Gateway rejected your session");
    expect(alert.textContent).not.toContain("Settings");
  });

  it("offers the switch on the org picker, behind Settings' confirmation", async () => {
    const state = bootStuckOnOrgPicker();
    await screen.findByText("Choose an organization");

    fireEvent.click(screen.getByRole("button", { name: "change" }));
    // The dialog, not the inline list: nothing has moved yet.
    await screen.findByText("Change gateway server");
    expect(window.__GATE_E2E__.calls.some((c) => c.cmd === "switch_gateway")).toBe(false);

    fireEvent.click(screen.getByRole("radio", { name: /Staging/ }));
    fireEvent.click(screen.getByRole("button", { name: "Switch and relaunch" }));

    await waitFor(() => expect(relaunch).toHaveBeenCalled());
    expect(state.account!.gateway_base_url).toBe(STAGING);
    expect(
      window.__GATE_E2E__.calls.filter((c) => c.cmd === "switch_gateway"),
    ).toEqual([{ cmd: "switch_gateway", args: { baseUrl: STAGING } }]);
  });

  it("reports a failed switch on the setup pane", async () => {
    bootStuckOnOrgPicker((s) => {
      s.failures.switch_gateway = "could not stop the engine";
    });
    await screen.findByText("Choose an organization");

    fireEvent.click(screen.getByRole("button", { name: "change" }));
    fireEvent.click(await screen.findByRole("radio", { name: /Staging/ }));
    fireEvent.click(screen.getByRole("button", { name: "Switch and relaunch" }));

    await waitFor(() =>
      expect(
        screen.getAllByRole("alert").some((a) => a.textContent?.includes("Something went wrong")),
      ).toBe(true),
    );
    expect(relaunch).not.toHaveBeenCalled();
    // It replaced the older refusal rather than sitting behind it.
    expect(screen.queryByText("Gateway rejected your session")).toBeNull();
  });
});
