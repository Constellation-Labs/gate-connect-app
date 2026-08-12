import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent, waitFor } from "@testing-library/react";
import type { Mock } from "vitest";

// The tour's close path is the one place where a thrown side effect can
// strand the user: Tauri prevents the native close whenever JS listens for
// close-requested, so the window only goes away if our handler resolves and
// finish() actually reaches close().
const close = vi.fn();
const onCloseRequested = vi.fn((_handler: () => unknown) => Promise.resolve(() => {}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ close, onCloseRequested }),
}));
vi.mock("@tauri-apps/api/event", () => ({ emit: vi.fn() }));
// Every command the tour can issue goes through here: `proxy_status` on the
// certificate step, `proxy_trust_ca` from its button, `reveal_popover` from the
// last step. Returning a promise matters - the real `invoke` always does, and
// the status read is awaited during mount.
const defaultInvoke = async (cmd: string) => {
  if (cmd === "proxy_status") return { ca_trusted: caTrusted };
  if (cmd === "proxy_trust_ca") return { ca_trusted: true };
  return undefined;
};
const invoke = vi.fn(defaultInvoke);
let caTrusted = false;
vi.mock("@tauri-apps/api/core", () => ({ invoke: (cmd: string) => invoke(cmd) }));
vi.mock("../lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/platform")>()),
  usePlatform: () => "macos",
}));
vi.mock("../lib/analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));
vi.mock("../lib/tour", () => ({ setTourSeen: vi.fn(), TOUR_SEEN_EVENT: "gc:tour-seen" }));

import { track } from "../lib/analytics";
import { emit } from "@tauri-apps/api/event";
import { setTourSeen } from "../lib/tour";
import { Onboarding } from "./Onboarding";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  // `clearAllMocks` clears calls but keeps implementations, and the tests below
  // make `track` and `setTourSeen` throw on purpose. Left in place they fake a
  // failure in every later test - which is how a real `track` (it swallows its
  // own errors) ended up looking like it could break the certificate step.
  (track as Mock).mockImplementation(() => {});
  (setTourSeen as Mock).mockImplementation(() => {});
  invoke.mockImplementation(defaultInvoke);
  caTrusted = false;
});

/** Walk to the last step and press Get started. */
function completeTour() {
  render(<Onboarding />);
  for (let i = 0; i < 10; i++) {
    const next = screen.queryByRole("button", { name: "Next" });
    if (!next) break;
    fireEvent.click(next);
  }
  fireEvent.click(screen.getByRole("button", { name: "Get started" }));
}

describe("Onboarding close path", () => {
  it("closes the window when the tour completes", async () => {
    completeTour();
    await waitFor(() => expect(close).toHaveBeenCalledTimes(1));
    expect(setTourSeen).toHaveBeenCalled();
  });

  it("still closes when analytics throws", async () => {
    (track as Mock).mockImplementation(() => {
      throw new Error("posthog unreachable");
    });
    completeTour();
    // Telemetry is bookkeeping; it must never hold the window open.
    await waitFor(() => expect(close).toHaveBeenCalledTimes(1));
  });

  it("still closes when the seen-flag write throws", async () => {
    (setTourSeen as Mock).mockImplementation(() => {
      throw new Error("localStorage blocked");
    });
    completeTour();
    await waitFor(() => expect(close).toHaveBeenCalledTimes(1));
  });

  it("registers a close-requested handler that never rejects", async () => {
    (track as Mock).mockImplementation(() => {
      throw new Error("posthog unreachable");
    });
    render(<Onboarding />);
    await waitFor(() => expect(onCloseRequested).toHaveBeenCalled());
    // Tauri awaits this handler and only destroys the window if it resolves.
    const handler = onCloseRequested.mock.calls[0][0];
    await expect(Promise.resolve(handler())).resolves.not.toThrow();
  });
});

// The step exists so the OS trust dialog is never the first the user hears of
// it: the popover is too small to explain it, and until this step the dialog
// could arrive before any screen that describes it had rendered.
describe("Onboarding certificate step", () => {
  /** Walk forward to the step carrying the certificate copy. Matched on the
   *  body, because the title differs by whether the CA is already trusted. */
  async function goToTrustStep() {
    render(<Onboarding />);
    while (!screen.queryByText(/no gateway setting to point anywhere/)) {
      fireEvent.click(screen.getByRole("button", { name: "Next" }));
    }
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("proxy_status"));
  }

  it("installs the certificate and tells the popover to re-read state", async () => {
    await goToTrustStep();
    const install = await screen.findByRole("button", { name: "Install certificate" });
    fireEvent.click(install);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("proxy_trust_ca"));
    // The popover holds its own copy of proxy state in another webview.
    await waitFor(() => expect(emit).toHaveBeenCalledWith("gc:ca-trusted"));
    expect(await screen.findByText(/Installed\. Nothing to do here/)).toBeTruthy();
  });

  it("offers nothing to do when the certificate is already trusted", async () => {
    caTrusted = true;
    await goToTrustStep();
    expect(await screen.findByText(/Installed\. Nothing to do here/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Install certificate" })).toBeNull();
    // And the step stops warning about a dialog that will not arrive.
    expect(screen.queryByText(/One prompt to expect/)).toBeNull();
    expect(screen.getByText(/certificate is in place/)).toBeTruthy();
  });

  it("keeps the tour moving when the trust fails", async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === "proxy_status") return { ca_trusted: false };
      // What `ca_windows.rs` bails with when the user chooses No.
      throw new Error("the certificate trust dialog was cancelled or denied");
    });
    await goToTrustStep();
    fireEvent.click(await screen.findByRole("button", { name: "Install certificate" }));
    // Surfaced, not swallowed - and Next still works, because a declined
    // dialog is a "later", not a dead end.
    expect(await screen.findByText(/certificate wasn’t trusted/)).toBeTruthy();
    const next = screen.getByRole("button", { name: "Next" }) as HTMLButtonElement;
    expect(next.disabled).toBe(false);
  });
});
