import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, waitFor } from "@testing-library/react";
import { defaultState } from "../e2e/backend";
import { installFakeTauri } from "../e2e/install";

/**
 * The main window is the only sender of the session note and `app_launched`
 * (AG-960): no other window reads the account. The seam's own rules are tested
 * in `lib/analytics.test.ts`; this pins that `NewUiApp` actually calls it, on
 * the fake backend the e2e suite uses. Only the analytics calls are spied on.
 */
vi.mock("./lib/analytics", async (importOriginal) => {
  const real = await importOriginal<typeof import("./lib/analytics")>();
  return {
    ...real,
    noteSession: vi.fn(),
    track: vi.fn(),
    noteTrafficObserved: vi.fn(),
  };
});

import { noteSession, noteTrafficObserved, track } from "./lib/analytics";
import { NewUiApp } from "./NewUiApp";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function boot(edit: (s: ReturnType<typeof defaultState>) => void = () => {}) {
  const state = defaultState();
  edit(state);
  installFakeTauri(state);
  render(<NewUiApp />);
  return state;
}

describe("NewUiApp sends the main window's analytics", () => {
  it("notes the OAuth session once the account and OAuth reads are in", async () => {
    const state = boot();
    await waitFor(() =>
      expect(noteSession).toHaveBeenCalledWith({
        signedIn: true,
        sessionUnknown: false,
        authMode: "oauth",
        sub: state.oauth!.sub,
        orgId: "org-1",
      }),
    );
  });

  it("sends app_launched with the launch's props", async () => {
    boot();
    await waitFor(() =>
      expect(track).toHaveBeenCalledWith("app_launched", expect.objectContaining({ has_account: true })),
    );
  });

  it("forwards the relay's traffic report to the seam", async () => {
    boot();
    await waitFor(() => expect(noteSession).toHaveBeenCalled());
    const report = ["claude-code"];
    window.__GATE_E2E__.emit("traffic-observed", report);
    await waitFor(() => expect(noteTrafficObserved).toHaveBeenCalledWith(report));
  });
});
