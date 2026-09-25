import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { CollectedDataDialog, SendDiagnosticsDialog } from "./dialogs";
import type { SendDiagnosticsState } from "./dialogs";

/**
 * The send-one-report flow (AG-603) and the disclosure it shares with Settings.
 *
 * Undrawn - the Figma has no frame for either dialog's send half, because
 * AG-603 was blocked on AG-602's handoff. So what is pinned here is not the
 * file's geometry but the criteria, and above all the two the product's
 * reassurance rests on: what the disclosure claims, and that there is nowhere
 * to type a support message.
 */

const noop = () => {};

afterEach(cleanup);

function renderSend(state: SendDiagnosticsState, overrides = {}) {
  return render(
    <SendDiagnosticsDialog
      state={state}
      onSend={noop}
      onCopyReference={noop}
      onClose={noop}
      {...overrides}
    />,
  );
}

describe("SendDiagnosticsDialog", () => {
  it("shows the field list with Send and Cancel", () => {
    renderSend({ kind: "confirm" });
    expect(screen.getByRole("button", { name: "Send" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeTruthy();
    // The field list, not a summary of it.
    expect(screen.getByText("Never sent")).toBeTruthy();
    expect(screen.getByText("Only in a report you send yourself")).toBeTruthy();
  });

  it("says the send does not change the collection setting", () => {
    // The criterion allows Send whether automatic collection is On or Off, so
    // the dialog has to answer the question a user opted OUT will ask: does
    // pressing this turn it back on.
    renderSend({ kind: "confirm" });
    expect(
      screen.getByText(/does not change your Share diagnostic data setting/i),
    ).toBeTruthy();
  });

  it("has nowhere to type a support message", () => {
    // "The report does not include the support message" is guaranteed by there
    // being no box to fill in - a field we promised not to send would be one
    // the user fills in and expects to be read.
    const { container } = renderSend({ kind: "confirm" });
    expect(container.querySelector("textarea")).toBeNull();
    expect(container.querySelector("input")).toBeNull();
  });

  it("cannot be dismissed while a report is in flight", () => {
    const onClose = vi.fn();
    renderSend({ kind: "sending" }, { onClose });
    // Both buttons refused: closing mid-send would drop the reference for a
    // report that did arrive.
    // `Modal` expresses a refused button as `aria-disabled` plus a dead
    // onClick, not the `disabled` attribute, so that it stays focusable and a
    // screen reader can still say what it is.
    expect(
      screen.getByRole("button", { name: "Sending…" }).getAttribute("aria-disabled"),
    ).toBe("true");
    const cancel = screen.getByRole("button", { name: "Cancel" });
    expect(cancel.getAttribute("aria-disabled")).toBe("true");
    // And refused in fact, not only in the attribute.
    fireEvent.click(cancel);
    expect(onClose).not.toHaveBeenCalled();
  });

  it("shows a reference to copy once the report is sent", () => {
    renderSend({ kind: "sent", reference: "GC-7Q2M-4KX9" });
    expect(screen.getByText("GC-7Q2M-4KX9")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Copy reference" })).toBeTruthy();
    expect(screen.getByText(/paste this reference into your support request/i)).toBeTruthy();
  });

  it("says routing was not interrupted", () => {
    renderSend({ kind: "sent", reference: "GC-7Q2M-4KX9" });
    expect(screen.getByText(/routing and event delivery were not interrupted/i)).toBeTruthy();
  });

  it("shows the error and offers Retry on a failure", () => {
    renderSend({
      kind: "failed",
      title: "Could not reach Constellation Gate",
      hint: "Check your connection and try again.",
    });
    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("Could not reach Constellation Gate");
    expect(alert.textContent).toContain("Check your connection and try again.");
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
    // The field list stays up: a retry is still a send, and the user has not
    // stopped being owed the disclosure because the first attempt failed.
    expect(screen.getByText("Never sent")).toBeTruthy();
  });
});

describe("the collected-data disclosure", () => {
  it("no longer claims paths and hostnames are never sent", () => {
    // It used to, and a manually sent report carries both. The claim that
    // survives is the one that is still absolute.
    render(<CollectedDataDialog onClose={noop} />);
    expect(screen.queryByText(/File paths, hostnames/i)).toBeNull();
    expect(screen.getByText("The contents of any config file.")).toBeTruthy();
  });

  it("keeps the claims that are still absolute", () => {
    render(<CollectedDataDialog onClose={noop} />);
    expect(screen.getByText("Prompts or model responses.")).toBeTruthy();
    expect(
      screen.getByText("API keys, credentials, or anything from your keychain."),
    ).toBeTruthy();
  });

  it("says what a report the user sends adds", () => {
    // The disclosure has to be complete on the screen that claims to be the
    // disclosure, not only on the one where the extra data is about to leave.
    render(<CollectedDataDialog onClose={noop} />);
    expect(screen.getByText("Only in a report you send yourself")).toBeTruthy();
    expect(screen.getByText(/your email, organization name and organization id/i)).toBeTruthy();
  });
});
