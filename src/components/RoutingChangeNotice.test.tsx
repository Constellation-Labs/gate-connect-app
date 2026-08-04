import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent, waitFor } from "@testing-library/react";
import type { Mock } from "vitest";
import { RoutingChangeNotice } from "./RoutingChangeNotice";

vi.mock("../lib/api", () => ({ closeRunningAgents: vi.fn() }));
vi.mock("../lib/analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));
import { closeRunningAgents } from "../lib/api";
import { track, trackError } from "../lib/analytics";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function renderNotice(routingOn: boolean, onDismiss = vi.fn()) {
  render(<RoutingChangeNotice routingOn={routingOn} onDismiss={onDismiss} />);
  return onDismiss;
}

describe("RoutingChangeNotice copy", () => {
  it("words the takeover for routing on", () => {
    renderNotice(true);
    expect(screen.getByText("Routing is on")).toBeTruthy();
    expect(screen.getByText(/aren’t routing through Gate yet/i)).toBeTruthy();
  });

  it("words the takeover for routing off", () => {
    renderNotice(false);
    expect(screen.getByText("Routing is off")).toBeTruthy();
    expect(screen.getByText(/still point at Gate/i)).toBeTruthy();
  });

  it("dismisses via Got it", () => {
    const onDismiss = renderNotice(true);
    fireEvent.click(screen.getByRole("button", { name: "Got it" }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});

describe("RoutingChangeNotice close-agents flow", () => {
  it("opens directly on the confirm step when startConfirming is set", () => {
    render(
      <RoutingChangeNotice routingOn startConfirming onDismiss={vi.fn()} />,
    );
    // No informational detour: the confirm copy and action are already up.
    expect(screen.getByText(/Desktop apps like Claude close too/i)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Close them" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Got it" })).toBeNull();
  });

  it("arms an inline confirm step first, and Cancel backs out without closing", () => {
    renderNotice(true);
    fireEvent.click(screen.getByRole("button", { name: "Close them…" }));
    // The confirm swaps the panel copy in place (the popover never stacks
    // dialogs) and warns that desktop apps are included.
    expect(screen.getByText(/Desktop apps like Claude close too/i)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(closeRunningAgents).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Close them…" })).toBeTruthy();
  });

  it("closes on confirm and reports the plural count", async () => {
    (closeRunningAgents as Mock).mockResolvedValue(3);
    renderNotice(true);
    fireEvent.click(screen.getByRole("button", { name: "Close them…" }));
    fireEvent.click(screen.getByRole("button", { name: "Close them" }));
    expect(await screen.findByText(/Closed 3 apps\. Open them again/)).toBeTruthy();
    expect(track).toHaveBeenCalledWith("agents_closed", { count: 3 });
    // The takeover ends with Done once the close has run.
    expect(screen.getByRole("button", { name: "Done" })).toBeTruthy();
  });

  it("reports a single close without a stray plural", async () => {
    (closeRunningAgents as Mock).mockResolvedValue(1);
    renderNotice(true);
    fireEvent.click(screen.getByRole("button", { name: "Close them…" }));
    fireEvent.click(screen.getByRole("button", { name: "Close them" }));
    expect(await screen.findByText(/Closed 1 app\. Open them again/)).toBeTruthy();
  });

  it("says when no agents were running", async () => {
    (closeRunningAgents as Mock).mockResolvedValue(0);
    renderNotice(true);
    fireEvent.click(screen.getByRole("button", { name: "Close them…" }));
    fireEvent.click(screen.getByRole("button", { name: "Close them" }));
    expect(await screen.findByText(/Nothing was running\./)).toBeTruthy();
  });

  it("surfaces a failed close and stays on the confirm step for a retry", async () => {
    (closeRunningAgents as Mock).mockRejectedValue("SIGTERM not permitted");
    renderNotice(true);
    fireEvent.click(screen.getByRole("button", { name: "Close them…" }));
    fireEvent.click(screen.getByRole("button", { name: "Close them" }));
    expect(await screen.findByText(/SIGTERM not permitted/)).toBeTruthy();
    expect(trackError).toHaveBeenCalledWith("SIGTERM not permitted", "close_agents");
    // No count means no Done; the confirm button is still there to retry.
    expect(screen.queryByRole("button", { name: "Done" })).toBeNull();
    expect(screen.getByRole("button", { name: "Close them" })).toBeTruthy();
  });

  it("dismisses via Done after a close", async () => {
    (closeRunningAgents as Mock).mockResolvedValue(2);
    const onDismiss = renderNotice(true);
    fireEvent.click(screen.getByRole("button", { name: "Close them…" }));
    fireEvent.click(screen.getByRole("button", { name: "Close them" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Done" })).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("notifies onAgentsClosed only after a successful close", async () => {
    (closeRunningAgents as Mock).mockResolvedValue(2);
    const onAgentsClosed = vi.fn();
    render(
      <RoutingChangeNotice routingOn onDismiss={vi.fn()} onAgentsClosed={onAgentsClosed} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Close them…" }));
    expect(onAgentsClosed).not.toHaveBeenCalled(); // arming the confirm isn't acting
    fireEvent.click(screen.getByRole("button", { name: "Close them" }));
    await waitFor(() => expect(onAgentsClosed).toHaveBeenCalledTimes(1));
  });

  it("does not notify onAgentsClosed when the close fails", async () => {
    (closeRunningAgents as Mock).mockRejectedValue("SIGTERM not permitted");
    const onAgentsClosed = vi.fn();
    render(
      <RoutingChangeNotice routingOn onDismiss={vi.fn()} onAgentsClosed={onAgentsClosed} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Close them…" }));
    fireEvent.click(screen.getByRole("button", { name: "Close them" }));
    await screen.findByText(/Couldn’t close the running tools and apps/);
    expect(onAgentsClosed).not.toHaveBeenCalled();
  });
});

describe("RoutingChangeNotice destructive grammar", () => {
  it("moves the heading to the question on the confirm step", async () => {
    render(<RoutingChangeNotice routingOn={false} onDismiss={vi.fn()} />);
    const dialog = screen.getByRole("dialog");
    const titleId = dialog.getAttribute("aria-labelledby")!;
    expect(document.getElementById(titleId)?.textContent).toBe("Routing is off");

    fireEvent.click(screen.getByRole("button", { name: "Close them…" }));
    // The heading is the aria-labelledby target; if it never moves, a screen
    // reader entering the confirm hears no change at all.
    expect(document.getElementById(titleId)?.textContent).toBe(
      "Close the tools and apps that are running?",
    );
  });

  it("does not dress the destructive action as the encouraged one", () => {
    render(<RoutingChangeNotice routingOn={false} onDismiss={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Close them…" }));
    const destroy = screen.getByRole("button", { name: "Close them" });
    const cancel = screen.getByRole("button", { name: "Cancel" });
    expect(destroy.className).not.toContain("bg-gc-accent");
    expect(destroy.className).toContain("bg-gc-error-deep");
    // Cancel is a full button, not a text link, so the safe path is its equal.
    expect(cancel.className).toContain("w-full");
  });
});
