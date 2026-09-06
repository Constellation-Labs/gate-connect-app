import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Mock } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import { useRunningApps } from "./useRunningApps";

vi.mock("./api", () => ({
  runningAgents: vi.fn(),
  closeRunningAgents: vi.fn(),
  routingVerdicts: vi.fn(),
}));
vi.mock("./analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));

import { closeRunningAgents, routingVerdicts, runningAgents } from "./api";

const agent = (slug: string, name: string, pid: number, stale = true) => ({
  slug,
  name,
  can_reopen: false,
  pid,
  started_at_unix: 1000,
  predates_routing: stale,
});

const verdict = (
  slug: string,
  state: "on" | "off" | "needs_attention",
  reason: string | null = null,
) => ({
  slug,
  state,
  reason,
  next_action: null,
  route_in_use: null,
  requested_route: null,
});

function harness() {
  const api: { current: ReturnType<typeof useRunningApps> | null } = { current: null };
  const onError = vi.fn();
  function Probe() {
    api.current = useRunningApps({
      onError,
      nameFor: (slug) => (slug === "codex" ? "Codex" : "Claude Code"),
    });
    return null;
  }
  render(<Probe />);
  return { api, onError };
}

/** Walk to the confirmation, which is the only place closing is possible. */
async function toConfirm(api: { current: ReturnType<typeof useRunningApps> | null }) {
  await act(async () => {
    await api.current!.offerAfterChange();
  });
  act(() => api.current!.goToConfirm());
}

/** The slugs a stage is about, which is all most assertions here care about. */
function slugsOf(api: { current: ReturnType<typeof useRunningApps> | null }) {
  const stage = api.current!.stage;
  return stage ? stage.tools.map((t) => t.slug) : null;
}

function stagesOf(api: { current: ReturnType<typeof useRunningApps> | null }) {
  const stage = api.current!.stage;
  return stage ? stage.tools.map((t) => t.stage) : null;
}

beforeEach(() => {
  vi.clearAllMocks();
  (runningAgents as Mock).mockResolvedValue({
    scanned_names: ["claude", "codex"],
    agents: [agent("codex", "codex", 10)],
  });
  (closeRunningAgents as Mock).mockResolvedValue(1);
  (routingVerdicts as Mock).mockResolvedValue([
    verdict("codex", "needs_attention", "reopen_required"),
  ]);
});
afterEach(cleanup);

describe("useRunningApps: when to say anything", () => {
  it("offers to close what is running", async () => {
    const { api } = harness();

    await act(async () => {
      await api.current!.offerAfterChange();
    });

    expect(api.current!.stage?.kind).toBe("offer");
    expect(slugsOf(api)).toEqual(["codex"]);
    // The tool's own name, not the process name: the reader knows it as Codex.
    expect(api.current!.stage!.tools[0].name).toBe("Codex");
  });

  it("says nothing when nothing is running", async () => {
    // A dialog about no apps is a dialog about nothing.
    (runningAgents as Mock).mockResolvedValue({ scanned_names: ["codex"], agents: [] });
    const { api } = harness();

    await act(async () => {
      await api.current!.offerAfterChange();
    });

    expect(api.current!.stage).toBeNull();
  });

  it("stays silent when the scan fails", async () => {
    // This sequence offers to kill processes, so guessing wrong means offering
    // to close apps that may not be open.
    (runningAgents as Mock).mockRejectedValue(new Error("scan failed"));
    const { api, onError } = harness();

    await act(async () => {
      await api.current!.offerAfterChange();
    });

    expect(api.current!.stage).toBeNull();
    expect(onError).not.toHaveBeenCalled();
  });

  it("counts two processes of one app as one app", async () => {
    // The pid is not something the reader can act on, and "codex, codex" reads
    // as a bug.
    (runningAgents as Mock).mockResolvedValue({
      scanned_names: ["codex"],
      agents: [
        agent("codex", "codex", 10),
        agent("codex", "codex", 11),
        agent("claude-code", "claude", 12),
      ],
    });
    const { api } = harness();

    await act(async () => {
      await api.current!.offerAfterChange();
    });

    expect(slugsOf(api)).toEqual(["codex", "claude-code"]);
  });

  it("carries the two routes the verdict established", async () => {
    // "Reopen required" without them does not say what reopening would change.
    (routingVerdicts as Mock).mockResolvedValue([
      {
        ...verdict("codex", "needs_attention", "reopen_required"),
        route_in_use: "https://api.openai.com",
        requested_route: "https://gate.example/v1",
      },
    ]);
    const { api } = harness();

    await act(async () => {
      await api.current!.offerAfterChange();
    });

    expect(api.current!.stage!.tools[0]).toMatchObject({
      routeInUse: "https://api.openai.com",
      requestedRoute: "https://gate.example/v1",
    });
  });

  it("opens the step even when the sweep will not answer", async () => {
    // The routes are the nice half; the fact that a running tool is on an old
    // route comes from the scan, and losing the step because a probe was slow
    // would be worse than drawing it without them.
    (routingVerdicts as Mock).mockRejectedValue(new Error("no sweep"));
    const { api } = harness();

    await act(async () => {
      await api.current!.offerAfterChange();
    });

    expect(api.current!.stage?.kind).toBe("offer");
    expect(api.current!.stage!.tools[0].routeInUse).toBeNull();
  });
});

describe("useRunningApps: closing takes two answers", () => {
  it("does not signal anything from the offer", async () => {
    const { api } = harness();

    await act(async () => {
      await api.current!.offerAfterChange();
    });
    act(() => api.current!.goToConfirm());

    expect(api.current!.stage?.kind).toBe("confirm");
    expect(closeRunningAgents).not.toHaveBeenCalled();
  });

  it("backs out to the offer without closing anything", async () => {
    const { api } = harness();
    await toConfirm(api);

    act(() => api.current!.goBack());

    expect(api.current!.stage?.kind).toBe("offer");
    expect(slugsOf(api)).toEqual(["codex"]);
    expect(closeRunningAgents).not.toHaveBeenCalled();
  });

  it("walks away entirely, leaving the saved config alone", async () => {
    // The config was already written; this sequence only decides whether the
    // running processes are restarted now or later.
    const { api } = harness();
    await act(async () => {
      await api.current!.offerAfterChange();
    });

    act(() => api.current!.dismiss());

    expect(api.current!.stage).toBeNull();
    expect(closeRunningAgents).not.toHaveBeenCalled();
  });

  it("closes only from the confirmation, and then waits for the user", async () => {
    // Gate cannot reopen a terminal tool, so a closed one lands on the stage
    // that says the next move is theirs - not on a claim that it is routing.
    (runningAgents as Mock)
      .mockResolvedValueOnce({
        scanned_names: ["codex"],
        agents: [agent("codex", "codex", 10)],
      })
      .mockResolvedValue({ scanned_names: ["codex"], agents: [] });
    const { api } = harness();
    await toConfirm(api);

    await act(async () => {
      await api.current!.closeApps();
    });

    expect(closeRunningAgents).toHaveBeenCalled();
    expect(api.current!.stage?.kind).toBe("work");
    expect(stagesOf(api)).toEqual(["awaiting_reopen"]);
  });

  it("cannot close from the offer stage", async () => {
    const { api } = harness();
    await act(async () => {
      await api.current!.offerAfterChange();
    });

    await act(async () => {
      await api.current!.closeApps();
    });

    expect(closeRunningAgents).not.toHaveBeenCalled();
  });

  it("reports a close that failed rather than reporting nothing", async () => {
    // The apps are still open on their old route, which is a result the flow
    // has to account for - and the row's own action restarts the conversation
    // for that tool alone.
    (closeRunningAgents as Mock).mockRejectedValue(new Error("permission denied"));
    const { api, onError } = harness();
    await toConfirm(api);

    await act(async () => {
      await api.current!.closeApps();
    });

    expect(onError).toHaveBeenCalled();
    expect(stagesOf(api)).toEqual(["close_failed"]);
    expect(api.current!.stage!.tools[0].error).toContain("permission denied");
    expect(api.current!.busy).toBe(false);
  });
});

/**
 * What the flow concludes after the close, which is the half AG-566 AC 8 is
 * about: it is the *reopen* that gets verified, never the config on its own.
 */
describe("useRunningApps: following a tool back", () => {
  it("does not call a closed tool verified, however healthy its config reads", async () => {
    // `verdict_for` will happily answer `on` for a tool that is not running:
    // its config carries Gate's values, the relay answers and the session is
    // valid. Nothing has read that file, so this must not report it applied.
    (runningAgents as Mock)
      .mockResolvedValueOnce({
        scanned_names: ["codex"],
        agents: [agent("codex", "codex", 10)],
      })
      .mockResolvedValue({ scanned_names: ["codex"], agents: [] });
    (routingVerdicts as Mock).mockResolvedValue([verdict("codex", "on")]);
    const { api } = harness();
    await toConfirm(api);

    await act(async () => {
      await api.current!.closeApps();
    });

    expect(stagesOf(api)).toEqual(["awaiting_reopen"]);
  });

  it("verifies once a new process is up", async () => {
    (runningAgents as Mock)
      .mockResolvedValueOnce({
        scanned_names: ["codex"],
        agents: [agent("codex", "codex", 10)],
      })
      // Reopened: a process that started after the change.
      .mockResolvedValue({
        scanned_names: ["codex"],
        agents: [agent("codex", "codex", 44, false)],
      });
    (routingVerdicts as Mock).mockResolvedValue([verdict("codex", "on")]);
    const { api } = harness();
    await toConfirm(api);

    await act(async () => {
      await api.current!.closeApps();
    });

    expect(stagesOf(api)).toEqual(["routing"]);
  });

  it("reads a reopened tool that is off as verified too", async () => {
    // Routing turned off is a change like any other: the tool comes back on its
    // own settings, and that reading is the proof the change landed.
    (runningAgents as Mock)
      .mockResolvedValueOnce({
        scanned_names: ["codex"],
        agents: [agent("codex", "codex", 10)],
      })
      .mockResolvedValue({
        scanned_names: ["codex"],
        agents: [agent("codex", "codex", 44, false)],
      });
    (routingVerdicts as Mock).mockResolvedValue([verdict("codex", "off")]);
    const { api } = harness();
    await toConfirm(api);

    await act(async () => {
      await api.current!.closeApps();
    });

    expect(stagesOf(api)).toEqual(["not_routed"]);
  });

  it("moves one row without touching another", async () => {
    // AG-566 AC 10: retrying one tool repeats nothing for the others.
    (runningAgents as Mock).mockResolvedValue({
      scanned_names: ["codex", "claude"],
      agents: [agent("codex", "codex", 10), agent("claude-code", "claude", 11)],
    });
    const { api } = harness();
    await toConfirm(api);
    await act(async () => {
      await api.current!.closeApps();
    });

    act(() => api.current!.markStage("codex", "applying"));

    expect(api.current!.stage!.tools.map((t) => [t.slug, t.stage])).toEqual([
      ["codex", "applying"],
      // Untouched by the row above: still where the watch left it.
      ["claude-code", "closing"],
    ]);
  });
});

/**
 * Which processes the offer names.
 *
 * Offering to close Claude because someone switched Codex asks to kill work the
 * change never touched - and it is the kind of thing a user reads as the app not
 * understanding what it just did.
 */
describe("useRunningApps: which tool it is talking about", () => {
  it("asks only about the tools whose configs were written", async () => {
    const { api } = harness();

    await act(async () => {
      await api.current!.offerAfterChange(["codex"]);
    });

    expect(runningAgents).toHaveBeenCalledWith(["codex"]);
  });

  it("asks about every tool when the caller names none", async () => {
    // The master toggle: every routed tool is on its old route, so all of them
    // are fair game.
    const { api } = harness();

    await act(async () => {
      await api.current!.offerAfterChange();
    });

    expect(runningAgents).toHaveBeenCalledWith(undefined);
  });

  it("closes exactly the set it offered", async () => {
    // The regression this guards: the offer was scoped to one tool and the
    // close was not, so confirming a Codex change SIGTERMed a running `claude`
    // that nothing had reconfigured and the user was never shown.
    const { api } = harness();
    await act(async () => {
      await api.current!.offerAfterChange(["codex"]);
    });
    act(() => api.current!.goToConfirm());

    await act(async () => {
      await api.current!.closeApps();
    });

    expect(closeRunningAgents).toHaveBeenCalledWith(["codex"]);
  });

  it("keeps the filter across a trip back to the offer", async () => {
    // Backing out and confirming again must not widen the set.
    const { api } = harness();
    await act(async () => {
      await api.current!.offerAfterChange(["codex"]);
    });
    act(() => api.current!.goToConfirm());
    act(() => api.current!.goBack());
    act(() => api.current!.goToConfirm());

    await act(async () => {
      await api.current!.closeApps();
    });

    expect(closeRunningAgents).toHaveBeenCalledWith(["codex"]);
  });

  it("closes the rows on screen, not the filter it was raised with", async () => {
    // The master toggle offers every running tool and passes no filter, and the
    // close must still name exactly the rows the user agreed to.
    (runningAgents as Mock).mockResolvedValue({
      scanned_names: ["codex", "claude"],
      agents: [agent("codex", "codex", 10), agent("claude-code", "claude", 11)],
    });
    const { api } = harness();
    await toConfirm(api);

    await act(async () => {
      await api.current!.closeApps();
    });

    expect(closeRunningAgents).toHaveBeenCalledWith(["codex", "claude-code"]);
  });
});
