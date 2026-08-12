import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent } from "@testing-library/react";
import { CertificateNotice } from "./CertificateNotice";

afterEach(() => {
  cleanup();
});

function renderNotice(
  props: Partial<React.ComponentProps<typeof CertificateNotice>> = {},
) {
  const onInstall = vi.fn();
  const onDecline = vi.fn();
  render(
    <CertificateNotice
      platform="windows"
      pending={false}
      onInstall={onInstall}
      onDecline={onDecline}
      {...props}
    />,
  );
  return { onInstall, onDecline };
}

describe("CertificateNotice warns before the system dialog", () => {
  it("names the dialog the platform is about to raise, in future tense", () => {
    renderNotice();
    expect(screen.getByText("One prompt to expect")).toBeTruthy();
    // The reassurance Windows needs before its red warning: expected, and which
    // button ends it.
    expect(screen.getByText(/security warning: that’s expected, choose Yes/i)).toBeTruthy();
  });

  it("switches to present tense while the dialog is up", () => {
    renderNotice({ pending: true });
    // Windows quotes the CA's common name back at the user, so naming it is what
    // lets them match the dialog in front of them to the app that raised it.
    expect(screen.getByText(/Windows is asking you to confirm “Gate Connect Local CA”/i)).toBeTruthy();
    expect(screen.queryByText(/that’s expected, choose Yes/i)).toBeNull();
  });

  it("says where the certificate is trusted in the platform's own vocabulary", () => {
    renderNotice({ platform: "macos" });
    expect(screen.getByText(/your keychain has to trust its certificate/i)).toBeTruthy();
    expect(screen.getByText(/macOS will ask for your login password/i)).toBeTruthy();
  });
});

describe("CertificateNotice hands the decision back", () => {
  it("installs on the primary", () => {
    const { onInstall, onDecline } = renderNotice();
    fireEvent.click(screen.getByRole("button", { name: "Install certificate" }));
    expect(onInstall).toHaveBeenCalledTimes(1);
    expect(onDecline).not.toHaveBeenCalled();
  });

  it("declines on Not now", () => {
    const { onInstall, onDecline } = renderNotice();
    fireEvent.click(screen.getByRole("button", { name: "Not now" }));
    expect(onDecline).toHaveBeenCalledTimes(1);
    expect(onInstall).not.toHaveBeenCalled();
  });

  it("declines on Escape", () => {
    const { onDecline } = renderNotice();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onDecline).toHaveBeenCalledTimes(1);
  });

  it("lands focus on the way out, not on the install", () => {
    renderNotice();
    // DESIGN.md's rule for every takeover: Enter on an unread panel must not
    // decide the outcome, and this panel exists to be read before a root
    // certificate goes into the user's trust store.
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Not now" }));
  });

  it("takes no second answer once the dialog is up", () => {
    const { onInstall, onDecline } = renderNotice({ pending: true });
    // The OS owns the decision from here, and Escape cannot cancel a system
    // dialog - dismissing the panel would leave a security warning on screen
    // with no app behind it, which is what the pin exists to prevent.
    fireEvent.keyDown(document, { key: "Escape" });
    fireEvent.click(screen.getByRole("button", { name: "Waiting…" }));
    expect(onInstall).not.toHaveBeenCalled();
    expect(onDecline).not.toHaveBeenCalled();
  });
});
