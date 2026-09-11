import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { AppShell } from "./AppShell";

afterEach(cleanup);

const noop = () => {};

function shell(props: Partial<Parameters<typeof AppShell>[0]> = {}) {
  return (
    <AppShell
      menuOpen={false}
      onMenuToggle={noop}
      onMenuSelect={noop}
      routing={{ protectedCount: 1, totalCount: 2 }}
      orgName="Acme Engineering"
      onSwitchOrg={noop}
      view={{ kind: "overview" }}
      onNavigate={noop}
      appGroups={[]}
      onSelectApp={noop}
      onToggleApp={noop}
      {...props}
    >
      <div>pane</div>
    </AppShell>
  );
}

/** The div the shell wraps the notice in. The notice itself is a `<p>`, so the
 *  nearest ancestor div is that wrapper. */
const noticeWrapper = () => screen.getByText("a notice").closest("div");

/**
 * The notice slot's elevation.
 *
 * `Modal`'s scrim is `z-20` and covers the whole window, chrome included, which
 * is what the design draws. One notice is exempt: `ErrorBanner` is the only
 * report a failed rename or key replacement gets, and under the scrim its
 * dismiss button is readable and unclickable.
 *
 * The exemption used to be unconditional on the slot rather than on that one
 * banner. That was correct while `ErrorBanner` was the only thing in the slot
 * and wrong as soon as the recovery and reopen banners joined it: both then
 * floated over every dialog, lit while everything round them dimmed. The
 * sharpest case was `ReopenBanner`, whose "Close tool" button sat on top of the
 * close-apps dialog that button opens.
 */
/**
 * The organization line is a control only when it can act.
 *
 * `/v1/me/orgs` answers "which organizations may this *user* act on", so it
 * needs a Cognito bearer; the gateway refuses an API key for it outright. And an
 * API key resolves to exactly one organization (AG-572's contract), so there is
 * nothing to choose between either way. The window offered the button to every
 * account regardless, and on an API key clicking it could only raise an error
 * banner. The tray had this right already.
 */
describe("AppShell organization line", () => {
  it("is a button when the account can switch", () => {
    const onSwitchOrg = vi.fn();
    render(shell({ orgName: "Acme Engineering", onSwitchOrg }));

    const control = screen.getByText("Acme Engineering").closest("button");
    expect(control).not.toBeNull();
    control?.click();
    expect(onSwitchOrg).toHaveBeenCalledTimes(1);
  });

  it("is a plain label when it cannot, and still names the org", () => {
    render(shell({ orgName: "Acme Engineering", onSwitchOrg: undefined }));

    // The name must survive: this line is the only place the window says which
    // organization the traffic belongs to.
    expect(screen.getByText("Acme Engineering")).toBeTruthy();
    expect(screen.getByText("Acme Engineering").closest("button")).toBeNull();
  });
});

describe("AppShell notice elevation", () => {
  it("leaves a notice under the dialog scrim by default", () => {
    render(shell({ notice: <p>a notice</p> }));

    expect(noticeWrapper()?.className ?? "").not.toContain("z-30");
  });

  it("lifts the notice above the scrim only when the caller asks", () => {
    render(shell({ notice: <p>a notice</p>, noticeAboveDialog: true }));

    expect(noticeWrapper()?.className ?? "").toContain("z-30");
  });
});
