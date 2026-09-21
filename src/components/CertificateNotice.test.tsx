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

describe("CertificateNotice depicts the prompt the platform actually raises", () => {
  // The depictions are `aria-hidden`, so they are unreachable through the
  // queries the rest of this file uses. Reaching for them by container is the
  // point: nothing else in the suite notices if a platform silently swaps the
  // capture for the drawing, which is exactly what happened when the capture
  // landed and all eight existing cases stayed green.
  function depiction() {
    return document.querySelector("figure");
  }

  it("shows Windows the captured dialog, not a drawing", () => {
    renderNotice({ platform: "windows" });
    const img = depiction()?.querySelector("img");
    expect(img).toBeTruthy();
    expect(img?.getAttribute("src")).toMatch(/windows-certificate-warning/);
    // Intrinsic size reserves the box before the PNG decodes; a capture that
    // lost these would reflow the tallest panel in the app on load.
    expect(img?.getAttribute("width")).toBe("516");
    expect(img?.getAttribute("height")).toBe("475");
  });

  it.each([
    ["macos", "Certificate Trust Settings"],
    ["linux", "Authentication Required"],
  ] as const)("draws %s a password field under its own title", (platform, title) => {
    renderNotice({ platform });
    const fig = depiction();
    expect(fig?.querySelector("img")).toBeNull();
    expect(fig?.textContent).toContain(title);
    // The field is the claim this drawing makes: these two ask for a password
    // rather than for a judgement.
    expect(fig?.textContent).toContain("Password");
  });

  it("draws no password field for an unknown platform", () => {
    renderNotice({ platform: "unknown" });
    // `trustPromptHint` promises only that something will ask the user to
    // confirm, so the drawing must not promise a password the copy will not.
    expect(depiction()?.textContent).not.toContain("Password");
    expect(screen.getByText(/Your system will ask you to confirm/i)).toBeTruthy();
  });

  it("hides every depiction from the accessibility tree, caption included", () => {
    renderNotice({ platform: "linux" });
    // The caption used to sit outside the hidden node, so a screen reader was
    // told what a picture it was never given roughly shows. `queryByText` does
    // not honour `aria-hidden`, so assert the containment directly rather than
    // the absence.
    const fig = depiction();
    expect(fig?.getAttribute("aria-hidden")).toBe("true");
    const caption = screen.getByText(/Roughly what your system will show/i);
    expect(fig?.contains(caption)).toBe(true);
  });
});

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
    // The store name is interpolated, so the sentence is split across nodes:
    // match on the paragraph's own text rather than on a single node.
    expect(
      screen.getByText((_t, el) =>
        el?.tagName === "P" && /keychain needs to trust its certificate/i.test(el.textContent ?? ""),
      ),
    ).toBeTruthy();
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
