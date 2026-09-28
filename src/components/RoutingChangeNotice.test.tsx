import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent, waitFor } from "@testing-library/react";
import type { Mock } from "vitest";
import { RoutingChangeNotice, closedSummary } from "./RoutingChangeNotice";

vi.mock("../lib/api", () => ({ closeRunningAgents: vi.fn() }));
vi.mock("../lib/analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));
import { closeRunningAgents } from "../lib/api";
import { track, trackError } from "../lib/analytics";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const result = (closed: number, restarted: string[] = [], reopen_yourself: string[] = []) => ({
  closed,
  restarted,
  reopen_yourself,
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

describe("RoutingChangeNotice restart flow", () => {
  it("opens directly on the confirm step when startConfirming is set", () => {
    render(
      <RoutingChangeNotice routingOn startConfirming onDismiss={vi.fn()} />,
    );
    // No informational detour: the confirm copy and action are already up.
    expect(screen.getByText(/Desktop apps like Claude quit and open again/i)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Restart them" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Got it" })).toBeNull();
  });

  it("arms an inline confirm step first, and Cancel backs out without restarting", () => {
    renderNotice(true);
    fireEvent.click(screen.getByRole("button", { name: "Restart them…" }));
    // The confirm swaps the panel copy in place (the popover never stacks
    // dialogs) and says what comes back on its own and what does not.
    expect(screen.getByText(/Desktop apps like Claude quit and open again/i)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(closeRunningAgents).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Restart them…" })).toBeTruthy();
  });

  it("restarts on confirm and names what came back and what to start again", async () => {
    (closeRunningAgents as Mock).mockResolvedValue(result(3, ["Claude"], ["Claude Code", "Codex"]));
    renderNotice(true);
    fireEvent.click(screen.getByRole("button", { name: "Restart them…" }));
    fireEvent.click(screen.getByRole("button", { name: "Restart them" }));
    expect(
      await screen.findByText(
        "Restarted Claude. Start Claude Code and Codex again when you need them.",
      ),
    ).toBeTruthy();
    expect(track).toHaveBeenCalledWith("agents_closed", { count: 3, restarted: 1 });
    // The takeover ends with Done once the restart has run.
    expect(screen.getByRole("button", { name: "Done" })).toBeTruthy();
  });

  it("reports a single terminal tool without a stray plural", async () => {
    (closeRunningAgents as Mock).mockResolvedValue(result(1, [], ["Claude Code"]));
    renderNotice(true);
    fireEvent.click(screen.getByRole("button", { name: "Restart them…" }));
    fireEvent.click(screen.getByRole("button", { name: "Restart them" }));
    expect(
      await screen.findByText("Closed Claude Code. Start it again when you need it."),
    ).toBeTruthy();
  });

  it("says when no agents were running", async () => {
    (closeRunningAgents as Mock).mockResolvedValue(result(0));
    renderNotice(true);
    fireEvent.click(screen.getByRole("button", { name: "Restart them…" }));
    fireEvent.click(screen.getByRole("button", { name: "Restart them" }));
    expect(await screen.findByText(/Nothing was running\./)).toBeTruthy();
  });

  it("surfaces a failed close and stays on the confirm step for a retry", async () => {
    (closeRunningAgents as Mock).mockRejectedValue("SIGTERM not permitted");
    renderNotice(true);
    fireEvent.click(screen.getByRole("button", { name: "Restart them…" }));
    fireEvent.click(screen.getByRole("button", { name: "Restart them" }));
    expect(await screen.findByText(/SIGTERM not permitted/)).toBeTruthy();
    expect(trackError).toHaveBeenCalledWith("SIGTERM not permitted", "close_agents");
    // No count means no Done; the confirm button is still there to retry.
    expect(screen.queryByRole("button", { name: "Done" })).toBeNull();
    expect(screen.getByRole("button", { name: "Restart them" })).toBeTruthy();
  });

  it("dismisses via Done after a close", async () => {
    (closeRunningAgents as Mock).mockResolvedValue(result(2, ["Claude"], ["Codex"]));
    const onDismiss = renderNotice(true);
    fireEvent.click(screen.getByRole("button", { name: "Restart them…" }));
    fireEvent.click(screen.getByRole("button", { name: "Restart them" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Done" })).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("notifies onAgentsClosed only after a successful close", async () => {
    (closeRunningAgents as Mock).mockResolvedValue(result(2, ["Claude"], ["Codex"]));
    const onAgentsClosed = vi.fn();
    render(
      <RoutingChangeNotice routingOn onDismiss={vi.fn()} onAgentsClosed={onAgentsClosed} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Restart them…" }));
    expect(onAgentsClosed).not.toHaveBeenCalled(); // arming the confirm isn't acting
    fireEvent.click(screen.getByRole("button", { name: "Restart them" }));
    await waitFor(() => expect(onAgentsClosed).toHaveBeenCalledTimes(1));
  });

  it("does not notify onAgentsClosed when the close fails", async () => {
    (closeRunningAgents as Mock).mockRejectedValue("SIGTERM not permitted");
    const onAgentsClosed = vi.fn();
    render(
      <RoutingChangeNotice routingOn onDismiss={vi.fn()} onAgentsClosed={onAgentsClosed} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Restart them…" }));
    fireEvent.click(screen.getByRole("button", { name: "Restart them" }));
    await screen.findByText(/Couldn’t restart the running tools and apps/);
    expect(onAgentsClosed).not.toHaveBeenCalled();
  });
});

describe("RoutingChangeNotice destructive grammar", () => {
  it("moves the heading to the question on the confirm step", async () => {
    render(<RoutingChangeNotice routingOn={false} onDismiss={vi.fn()} />);
    const dialog = screen.getByRole("dialog");
    const titleId = dialog.getAttribute("aria-labelledby")!;
    expect(document.getElementById(titleId)?.textContent).toBe("Routing is off");

    fireEvent.click(screen.getByRole("button", { name: "Restart them…" }));
    // The heading is the aria-labelledby target; if it never moves, a screen
    // reader entering the confirm hears no change at all.
    expect(document.getElementById(titleId)?.textContent).toBe(
      "Restart the tools and apps that are running?",
    );
  });

  it("does not dress the destructive action as the encouraged one", () => {
    render(<RoutingChangeNotice routingOn={false} onDismiss={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Restart them…" }));
    const destroy = screen.getByRole("button", { name: "Restart them" });
    const cancel = screen.getByRole("button", { name: "Cancel" });
    expect(destroy.className).not.toContain("bg-gc-accent");
    expect(destroy.className).toContain("bg-gc-error-deep");
    // Cancel is a full button, not a text link, so the safe path is its equal.
    expect(cancel.className).toContain("w-full");
  });
});

describe("closedSummary", () => {
  it("says only what came back when nothing is left to start", () => {
    expect(closedSummary(result(1, ["Claude"]))).toBe("Restarted Claude.");
  });

  it("names every tool left to start again", () => {
    expect(closedSummary(result(3, [], ["Claude Code", "Codex", "OpenCode"]))).toBe(
      "Closed Claude Code, Codex, and OpenCode. Start them again when you need them.",
    );
  });

  it("falls back to a count when the names are missing", () => {
    expect(closedSummary(result(2))).toBe("Closed 2 apps.");
  });
});
