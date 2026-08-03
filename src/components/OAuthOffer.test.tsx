import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent, waitFor } from "@testing-library/react";
import { OAuthOffer } from "./OAuthOffer";

vi.mock("../lib/analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("OAuthOffer", () => {
  it("leads with sign-in and keeps declining a real, named choice", () => {
    render(<OAuthOffer onUpgrade={vi.fn(() => Promise.resolve())} onDismiss={vi.fn()} />);
    const accept = screen.getByRole("button", { name: /Sign in with Constellation/ });
    const decline = screen.getByRole("button", { name: "Keep using my API key" });
    expect(accept.className).toContain("bg-gc-accent");
    // A pasted key is supported, so declining must not read as postponing.
    expect(decline.className).not.toContain("bg-gc-accent");
    expect(decline.className).toContain("w-full");
  });

  it("is a labelled modal", () => {
    render(<OAuthOffer onUpgrade={vi.fn(() => Promise.resolve())} onDismiss={vi.fn()} />);
    const dialog = screen.getByRole("dialog");
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    const titleId = dialog.getAttribute("aria-labelledby")!;
    expect(document.getElementById(titleId)?.textContent).toBe(
      "Sign in instead of pasting a key",
    );
  });

  it("dismisses after a successful hand-off, so it never returns", async () => {
    const onUpgrade = vi.fn(() => Promise.resolve());
    const onDismiss = vi.fn();
    render(<OAuthOffer onUpgrade={onUpgrade} onDismiss={onDismiss} />);
    fireEvent.click(screen.getByRole("button", { name: /Sign in with Constellation/ }));
    await waitFor(() => expect(onDismiss).toHaveBeenCalledTimes(1));
  });

  it("stays open and explains itself when the hand-off fails", async () => {
    const onDismiss = vi.fn();
    render(
      <OAuthOffer onUpgrade={() => Promise.reject("browser did not open")} onDismiss={onDismiss} />,
    );
    fireEvent.click(screen.getByRole("button", { name: /Sign in with Constellation/ }));
    // Losing the offer on a failure would strand the user with no retry.
    await waitFor(() => expect(screen.getByRole("alert")).toBeTruthy());
    expect(onDismiss).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: /Sign in with Constellation/ })).toBeTruthy();
  });

  it("declining calls dismiss without touching the account", () => {
    const onUpgrade = vi.fn(() => Promise.resolve());
    const onDismiss = vi.fn();
    render(<OAuthOffer onUpgrade={onUpgrade} onDismiss={onDismiss} />);
    fireEvent.click(screen.getByRole("button", { name: "Keep using my API key" }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
    expect(onUpgrade).not.toHaveBeenCalled();
  });
});
