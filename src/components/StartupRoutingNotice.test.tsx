import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent, waitFor } from "@testing-library/react";
import type { Mock } from "vitest";
import { StartupRoutingNotice } from "./StartupRoutingNotice";

vi.mock("../lib/api", () => ({ closeRunningAgents: vi.fn() }));
vi.mock("../lib/analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));
import { closeRunningAgents } from "../lib/api";
import { track, trackError } from "../lib/analytics";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function renderNotice(routingOn: boolean, onDismiss = vi.fn()) {
  render(<StartupRoutingNotice routingOn={routingOn} onDismiss={onDismiss} />);
  return onDismiss;
}

describe("StartupRoutingNotice copy", () => {
  it("words the takeover for routing on", () => {
    renderNotice(true);
    expect(screen.getByText("Routing is on")).toBeTruthy();
    expect(screen.getByText(/won't route through Gate/i)).toBeTruthy();
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

describe("StartupRoutingNotice close-agents flow", () => {
  it("arms an inline confirm step first, and Cancel backs out without closing", () => {
    renderNotice(true);
    fireEvent.click(screen.getByRole("button", { name: "Close running agents" }));
    // The confirm swaps the panel copy in place (the popover never stacks
    // dialogs) and warns that desktop apps are included.
    expect(screen.getByText(/including desktop apps like Claude/i)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(closeRunningAgents).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Close running agents" })).toBeTruthy();
  });

  it("closes on confirm and reports the plural count", async () => {
    (closeRunningAgents as Mock).mockResolvedValue(3);
    renderNotice(true);
    fireEvent.click(screen.getByRole("button", { name: "Close running agents" }));
    fireEvent.click(screen.getByRole("button", { name: "Close agents" }));
    expect(await screen.findByText(/Closed 3 agents\./)).toBeTruthy();
    expect(track).toHaveBeenCalledWith("agents_closed", { count: 3 });
    // The takeover ends with Done once the close has run.
    expect(screen.getByRole("button", { name: "Done" })).toBeTruthy();
  });

  it("uses the singular form for one closed agent", async () => {
    (closeRunningAgents as Mock).mockResolvedValue(1);
    renderNotice(true);
    fireEvent.click(screen.getByRole("button", { name: "Close running agents" }));
    fireEvent.click(screen.getByRole("button", { name: "Close agents" }));
    expect(await screen.findByText(/Closed 1 agent\./)).toBeTruthy();
  });

  it("says when no agents were running", async () => {
    (closeRunningAgents as Mock).mockResolvedValue(0);
    renderNotice(true);
    fireEvent.click(screen.getByRole("button", { name: "Close running agents" }));
    fireEvent.click(screen.getByRole("button", { name: "Close agents" }));
    expect(await screen.findByText(/No running agents found\./)).toBeTruthy();
  });

  it("surfaces a failed close and stays on the confirm step for a retry", async () => {
    (closeRunningAgents as Mock).mockRejectedValue("SIGTERM not permitted");
    renderNotice(true);
    fireEvent.click(screen.getByRole("button", { name: "Close running agents" }));
    fireEvent.click(screen.getByRole("button", { name: "Close agents" }));
    expect(await screen.findByText(/SIGTERM not permitted/)).toBeTruthy();
    expect(trackError).toHaveBeenCalledWith("SIGTERM not permitted", "close_agents");
    // No count means no Done; the confirm button is still there to retry.
    expect(screen.queryByRole("button", { name: "Done" })).toBeNull();
    expect(screen.getByRole("button", { name: "Close agents" })).toBeTruthy();
  });

  it("dismisses via Done after a close", async () => {
    (closeRunningAgents as Mock).mockResolvedValue(2);
    const onDismiss = renderNotice(true);
    fireEvent.click(screen.getByRole("button", { name: "Close running agents" }));
    fireEvent.click(screen.getByRole("button", { name: "Close agents" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Done" })).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("notifies onAgentsClosed only after a successful close", async () => {
    (closeRunningAgents as Mock).mockResolvedValue(2);
    const onAgentsClosed = vi.fn();
    render(
      <StartupRoutingNotice routingOn onDismiss={vi.fn()} onAgentsClosed={onAgentsClosed} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Close running agents" }));
    expect(onAgentsClosed).not.toHaveBeenCalled(); // arming the confirm isn't acting
    fireEvent.click(screen.getByRole("button", { name: "Close agents" }));
    await waitFor(() => expect(onAgentsClosed).toHaveBeenCalledTimes(1));
  });

  it("does not notify onAgentsClosed when the close fails", async () => {
    (closeRunningAgents as Mock).mockRejectedValue("SIGTERM not permitted");
    const onAgentsClosed = vi.fn();
    render(
      <StartupRoutingNotice routingOn onDismiss={vi.fn()} onAgentsClosed={onAgentsClosed} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Close running agents" }));
    fireEvent.click(screen.getByRole("button", { name: "Close agents" }));
    await screen.findByText(/Couldn't close the running agents/);
    expect(onAgentsClosed).not.toHaveBeenCalled();
  });
});
