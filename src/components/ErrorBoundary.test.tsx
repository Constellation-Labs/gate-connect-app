import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import type { Mock } from "vitest";
import { proxyStatus, quitApp } from "../lib/api";
import { ErrorBoundary } from "./ErrorBoundary";

vi.mock("../lib/analytics", () => ({ captureException: vi.fn() }));
vi.mock("../lib/api", () => ({
  proxyStatus: vi.fn(),
  quitApp: vi.fn(() => Promise.resolve()),
}));

function proxy(running: boolean) {
  return { running, port: 8080, pac_port: null, ca_trusted: true, domains: [] };
}

function Boom(): JSX.Element {
  throw new Error("render exploded");
}

/** React logs the caught error to console.error on its way through the
 * boundary, which is noise, not a failure. */
function renderCrash() {
  const spy = vi.spyOn(console, "error").mockImplementation(() => {});
  render(
    <ErrorBoundary>
      <Boom />
    </ErrorBoundary>,
  );
  spy.mockRestore();
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ErrorBoundary", () => {
  it("answers the only question that matters: is my traffic still routing", async () => {
    (proxyStatus as Mock).mockResolvedValue(proxy(true));
    renderCrash();
    // The engine lives in the Rust process and does not care that the webview
    // threw, so this is knowable rather than guessable. The screen used to
    // open with a stack trace and never mention routing at all.
    expect(await screen.findByText(/still routing through Gate/)).toBeTruthy();
  });

  it("says routing is off when it is, rather than reassuring by default", async () => {
    (proxyStatus as Mock).mockResolvedValue(proxy(false));
    renderCrash();
    expect(await screen.findByText(/Routing is off/)).toBeTruthy();
  });

  it("admits it could not check instead of guessing off", async () => {
    // The likeliest cause of a crash this deep is the bridge itself, and
    // reporting "off" there would tell the user their traffic stopped when it
    // is probably still flowing.
    (proxyStatus as Mock).mockRejectedValue(new Error("bridge gone"));
    renderCrash();
    expect(await screen.findByText(/couldn’t check whether routing is still on/)).toBeTruthy();
  });

  it("offers a way out of a window with no menu bar", async () => {
    (proxyStatus as Mock).mockResolvedValue(proxy(true));
    renderCrash();
    expect(screen.getByRole("button", { name: "Reload window" })).toBeTruthy();
    const quit = screen.getByRole("button", { name: "Quit Gate Connect" });
    quit.click();
    await waitFor(() => expect(quitApp).toHaveBeenCalledTimes(1));
  });

  it("keeps the stack trace collapsed", async () => {
    (proxyStatus as Mock).mockResolvedValue(proxy(true));
    renderCrash();
    // Evidence for a bug report, not the first thing a user reads about their
    // own machine.
    const details = document.querySelector("details");
    expect(details).toBeTruthy();
    expect(details?.hasAttribute("open")).toBe(false);
    expect(screen.getByText(/render exploded/)).toBeTruthy();
  });
});
