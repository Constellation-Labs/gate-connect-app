import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { OAuthOfferDialog } from "./dialogs";

const noop = () => {};

afterEach(cleanup);

/**
 * The one-time offer to move a pasted key onto Constellation sign-in.
 *
 * These exist because of what a reviewer hit on the release build: the browser
 * sign-in was abandoned (the page opened in the wrong Chrome profile), and the
 * dialog then had no working exit for the five minutes `LOGIN_TIMEOUT_SECS`
 * allows. The decline was guarded as `() => !busy && onKeepKey()`, so it went on
 * rendering as a live button and silently did nothing, and `onDismiss` was
 * undefined, so Escape and the scrim were dead too.
 */
function offer(overrides: Partial<Parameters<typeof OAuthOfferDialog>[0]> = {}) {
  return render(
    <OAuthOfferDialog
      secretStore="the keychain"
      onSignIn={noop}
      onKeepKey={noop}
      {...overrides}
    />,
  );
}

describe("OAuthOfferDialog", () => {
  it("lets the user decline while the browser sign-in is still waiting", () => {
    const onKeepKey = vi.fn();
    offer({ busy: true, onKeepKey });

    screen.getByRole("button", { name: "Keep using my API key" }).click();

    expect(onKeepKey).toHaveBeenCalledOnce();
  });

  it("still declines when nothing is in flight", () => {
    const onKeepKey = vi.fn();
    offer({ onKeepKey });

    screen.getByRole("button", { name: "Keep using my API key" }).click();

    expect(onKeepKey).toHaveBeenCalledOnce();
  });

  it("keeps the primary out of reach while it waits, and says what it waits on", () => {
    // The primary is the one control that genuinely must not fire twice: a
    // second `login` would bind a second loopback listener.
    //
    // `aria-disabled`, not the attribute: `Modal` keeps the button focusable so
    // it stays announced and reachable, and drops the handler instead.
    const onSignIn = vi.fn();
    offer({ busy: true, onSignIn });

    const primary = screen.getByRole("button", { name: "Waiting for browser..." });
    expect(primary.getAttribute("aria-disabled")).toBe("true");
    primary.click();
    expect(onSignIn).not.toHaveBeenCalled();
  });

  it("lets Escape and the scrim close it mid-flow, without spending the offer", () => {
    // Both halves matter. Dismissal was dead while busy, which is half of what
    // was reported - and making it live is sharp on its own, because the
    // decline permanently marks a one-time offer seen. A stray Escape while
    // the person is typing their password in the browser must not burn an
    // offer they never answered.
    const onKeepKey = vi.fn();
    const onDismissOffer = vi.fn();
    offer({ busy: true, onKeepKey, onDismissOffer });

    fireEvent.keyDown(document, { key: "Escape" });

    expect(onDismissOffer).toHaveBeenCalled();
    expect(onKeepKey).not.toHaveBeenCalled();
  });

  it("falls back to the decline for a caller that draws no distinction", () => {
    const onKeepKey = vi.fn();
    offer({ busy: true, onKeepKey });

    fireEvent.keyDown(document, { key: "Escape" });

    expect(onKeepKey).toHaveBeenCalled();
  });

  it("names the platform's secret store in the subtitle", () => {
    offer({ secretStore: "Credential Manager" });

    expect(screen.getByText(/Credential Manager/)).toBeTruthy();
  });
});
