import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Mock } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import { useRunningApps } from "./useRunningApps";

vi.mock("./api", () => ({ runningAgents: vi.fn(), closeRunningAgents: vi.fn() }));
vi.mock("./analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));

import { closeRunningAgents, runningAgents } from "./api";

const agent = (name: string, pid: number) => ({
  name,
  pid,
  started_at_unix: 1000,
  predates_routing: true,
});

function harness() {
  const api: { current: ReturnType<typeof useRunningApps> | null } = { current: null };
  const onError = vi.fn();
  function Probe() {
    api.current = useRunningApps({ onError });
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

beforeEach(() => {
  vi.clearAllMocks();
  (runningAgents as Mock).mockResolvedValue({
    scanned_names: ["claude", "codex"],
    agents: [agent("codex", 10)],
  });
  (closeRunningAgents as Mock).mockResolvedValue(1);
});
afterEach(cleanup);

describe("useRunningApps: when to say anything", () => {
  it("offers to close what is running", async () => {
    const { api } = harness();

    await act(async () => {
      await api.current!.offerAfterChange();
    });

    expect(api.current!.stage).toEqual({ kind: "offer", apps: ["codex"] });
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
      agents: [agent("codex", 10), agent("codex", 11), agent("claude", 12)],
    });
    const { api } = harness();

    await act(async () => {
      await api.current!.offerAfterChange();
    });

    expect(api.current!.stage).toEqual({ kind: "offer", apps: ["codex", "claude"] });
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

    expect(api.current!.stage).toEqual({ kind: "offer", apps: ["codex"] });
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

  it("closes only from the confirmation, then reports what it closed", async () => {
    const { api } = harness();
    await toConfirm(api);

    await act(async () => {
      await api.current!.closeApps();
    });

    expect(closeRunningAgents).toHaveBeenCalled();
    expect(api.current!.stage).toEqual({ kind: "done", apps: ["codex"] });
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

  it("finishes even when nothing was signalled", async () => {
    // Closed between the probe and the confirmation. The apps are gone either
    // way, which is what the user was after.
    (closeRunningAgents as Mock).mockResolvedValue(0);
    const { api } = harness();
    await toConfirm(api);

    await act(async () => {
      await api.current!.closeApps();
    });

    expect(api.current!.stage?.kind).toBe("done");
  });

  it("stays on the confirmation when closing fails", async () => {
    // The apps are still open, so the user should be able to retry or back out.
    (closeRunningAgents as Mock).mockRejectedValue(new Error("permission denied"));
    const { api, onError } = harness();
    await toConfirm(api);

    await act(async () => {
      await api.current!.closeApps();
    });

    expect(onError).toHaveBeenCalled();
    expect(api.current!.stage?.kind).toBe("confirm");
    expect(api.current!.busy).toBe(false);
  });
});

/**
 * Which processes the offer names.
 *
 * Offering to close Claude because someone switched Codex asks to kill work the
 * change never touched - and it is the kind of thing a user reads as the app not
 * understanding what it just did.
 */
describe("offerAfterChange scope", () => {
  it("asks only for the tools that changed", async () => {
    const { api } = harness();
    (runningAgents as Mock).mockResolvedValue({ scanned_names: [], agents: [] });

    await act(async () => {
      await api.current!.offerAfterChange(["codex"]);
    });

    expect(runningAgents as Mock).toHaveBeenCalledWith(["codex"]);
  });

  it("asks for everything when nothing is named, which is the master toggle", async () => {
    // A master toggle moved every tool's route, so every running tool is stale.
    const { api } = harness();
    (runningAgents as Mock).mockResolvedValue({ scanned_names: [], agents: [] });

    await act(async () => {
      await api.current!.offerAfterChange();
    });

    expect(runningAgents as Mock).toHaveBeenCalledWith(undefined);
  });
});
