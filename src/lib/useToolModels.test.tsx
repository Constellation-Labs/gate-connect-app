import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import { useToolModels } from "./toolModels";

vi.mock("./api", () => ({
  toolModelPreferences: vi.fn(),
  setToolModel: vi.fn(),
  gateModelCatalogue: vi.fn(),
}));
// The focus re-read listens on the Tauri window. Captured so a test can play a
// blur and a return; jsdom has no Tauri runtime to fire them.
const focus: { handler: ((e: { payload: boolean }) => void) | null } = { handler: null };
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    onFocusChanged: (h: (e: { payload: boolean }) => void) => {
      focus.handler = h;
      return Promise.resolve(() => undefined);
    },
  }),
}));
import { setToolModel, toolModelPreferences } from "./api";

/**
 * The hook, not the adapter - `toolModels.test.ts` covers the pure part.
 *
 * What is risky here is the state machine: a write has to be followed by a
 * re-read rather than a local patch, a failed read has to drop the previous
 * *setting* (unlike a measurement, which stays true), and a failure has to come
 * back as a value the caller can branch on rather than as a throw.
 */
const readCall = toolModelPreferences as unknown as ReturnType<typeof vi.fn>;
const writeCall = setToolModel as unknown as ReturnType<typeof vi.fn>;

function payload(
  source: "tool" | "gate",
  modelIds: string[],
  paidAckUnix: number | null = null,
  configured: Record<string, unknown> = {},
) {
  return {
    tools: { codex: { source, model_ids: modelIds } },
    paid_ack_unix: paidAckUnix,
    configured,
  };
}

/** Codex's config as the backend reports it after putting it back on its own
 *  model, on the one read that found the change. */
function leftTo(model: string | null) {
  return {
    codex: { state: "not_applied", model: null, left_gate_models: true, left_to_model: model },
  };
}

function harness() {
  const seen: ReturnType<typeof useToolModels>[] = [];
  function Probe() {
    seen.push(useToolModels(true, "cred"));
    return null;
  }
  render(<Probe />);
  return { seen, latest: () => seen[seen.length - 1] };
}

const flush = () => act(async () => {});

beforeEach(() => {
  readCall.mockReset();
  writeCall.mockReset();
  focus.handler = null;
});
afterEach(cleanup);

type SaveResult = Awaited<ReturnType<ReturnType<typeof useToolModels>["save"]>>;

describe("useToolModels", () => {
  it("reads once and exposes the preferences by platform", async () => {
    readCall.mockResolvedValue(payload("gate", ["openai/gpt-5"], 1767225600));
    const h = harness();
    await flush();

    expect(readCall).toHaveBeenCalledTimes(1);
    expect(h.latest().view?.byTool.get("codex")?.source).toBe("gate");
    expect(h.latest().view?.paidAckUnix).toBe(1767225600);
  });

  it("drops the previous reading when a re-read fails", async () => {
    // Deliberately unlike the activity pane, which keeps its last numbers. A
    // measurement stays true after the network dies; a *setting* does not, and
    // showing a stale one invites the user to toggle a switch whose current
    // position nobody knows.
    readCall.mockResolvedValueOnce(payload("gate", ["openai/gpt-5"]));
    const h = harness();
    await flush();
    expect(h.latest().view).not.toBeNull();

    readCall.mockRejectedValueOnce('{"code":"offline","message":"no route"}');
    act(() => h.latest().reload());
    await flush();

    expect(h.latest().view).toBeNull();
    expect(h.latest().failure?.code).toBe("offline");
  });

  it("re-reads after a write instead of patching the map it holds", async () => {
    // The write can change something it was not asked to - the org's paid
    // acknowledgement - and another machine may have changed a different platform
    // since this view was taken. Only a re-read can be trusted.
    readCall.mockResolvedValue(payload("tool", []));
    writeCall.mockResolvedValue(true);
    const h = harness();
    await flush();
    expect(readCall).toHaveBeenCalledTimes(1);

    readCall.mockResolvedValue(payload("gate", ["openai/gpt-5"], 1787740801));
    await act(async () => {
      await h.latest().save("codex", "gate", ["openai/gpt-5"], true);
    });

    expect(readCall).toHaveBeenCalledTimes(2);
    expect(h.latest().view?.byTool.get("codex")?.source).toBe("gate");
    expect(h.latest().view?.paidAckUnix).toBe(1787740801);
  });

  it("passes the acknowledgement through, and defaults it to withheld", async () => {
    readCall.mockResolvedValue(payload("tool", []));
    writeCall.mockResolvedValue(true);
    const h = harness();
    await flush();

    await act(async () => {
      await h.latest().save("codex", "tool", []);
    });
    expect(writeCall).toHaveBeenCalledWith("codex", "tool", [], false);
  });

  it("returns a write failure rather than throwing, so the caller can branch", async () => {
    // A local write can still fail - a read-only home, a full disk - and the
    // caller has to be able to say so rather than silently show the old value.
    readCall.mockResolvedValue(payload("tool", []));
    writeCall.mockRejectedValue("writing preferences.json: permission denied");
    const h = harness();
    await flush();

    // Held on an object rather than in a `let`: TypeScript cannot see the
    // assignment inside `act`'s callback and narrows a bare local to `never`.
    const got: { result: SaveResult | null } = { result: null };
    await act(async () => {
      got.result = await h.latest().save("codex", "gate", ["openai/gpt-5"]);
    });

    expect(got.result?.failure?.message).toContain("permission denied");
    // Nothing was written, so there is nothing to restart for.
    expect(got.result?.applied).toBe(false);
    // No re-read: nothing was stored, so the held view is still current.
    expect(readCall).toHaveBeenCalledTimes(1);
  });

  it("keeps the failure's own sentence, which no code carries", async () => {
    readCall.mockResolvedValue(payload("tool", []));
    writeCall.mockRejectedValue("unknown tool slug \"hermes-2\"");
    const h = harness();
    await flush();

    // Held on an object rather than in a `let`: TypeScript cannot see the
    // assignment inside `act`'s callback and narrows a bare local to `never`.
    const got: { result: SaveResult | null } = { result: null };
    await act(async () => {
      got.result = await h.latest().save("codex", "gate", ["openai/gpt-5"]);
    });

    expect(got.result?.failure?.message).toContain("unknown tool slug");
  });

  it("says whether the tool's config was rewritten, and only a literal true counts", async () => {
    // `true` is the cue for the restart notice. `false` - Gate does not manage
    // this tool's config right now - stores the choice and moves nothing on
    // disk, and so does anything that is not a boolean: offering to close a
    // running app on a guess is what `offerAfterChange` exists to avoid.
    readCall.mockResolvedValue(payload("tool", []));
    const h = harness();
    await flush();

    const got: { results: SaveResult[] } = { results: [] };
    for (const answer of [true, false, undefined]) {
      writeCall.mockResolvedValueOnce(answer);
      await act(async () => {
        got.results.push(await h.latest().save("codex", "gate", ["openai/gpt-5"], true));
      });
    }

    expect(got.results.map((r) => r.applied)).toEqual([true, false, false]);
    expect(got.results.every((r) => r.failure === null)).toBe(true);
  });

  it("re-reads when the window comes back, not on the first focus", async () => {
    // The user changes the model inside the tool, in another window. Coming
    // back is when the tool's config has to be read again.
    readCall.mockResolvedValue(payload("gate", ["openai/gpt-5"]));
    harness();
    await flush();
    expect(readCall).toHaveBeenCalledTimes(1);

    // A focus with no blur before it is the mount's own, already read.
    act(() => focus.handler?.({ payload: true }));
    await flush();
    expect(readCall).toHaveBeenCalledTimes(1);

    act(() => focus.handler?.({ payload: false }));
    act(() => focus.handler?.({ payload: true }));
    await flush();
    expect(readCall).toHaveBeenCalledTimes(2);
  });

  it("holds a tool leaving Gate models across later reads, until dismissed", async () => {
    // The backend reports it once, on the read that found it and put the tool
    // back on App default. The next read - a focus a second later - says
    // nothing, and the notice must not vanish with it.
    readCall.mockResolvedValueOnce(payload("tool", ["openai/gpt-5"], null, leftTo("gpt-6-sol")));
    const h = harness();
    await flush();
    expect(h.latest().leftGateModels.get("codex")).toBe("gpt-6-sol");
    expect(h.latest().view?.configured.get("codex")?.leftGateModels).toBe(true);

    readCall.mockResolvedValueOnce(payload("tool", ["openai/gpt-5"]));
    act(() => h.latest().reload());
    await flush();
    expect(h.latest().leftGateModels.has("codex")).toBe(true);

    act(() => h.latest().dismissLeft("codex"));
    expect(h.latest().leftGateModels.has("codex")).toBe(false);
  });

  it("keeps the report of a tool that named no model, as null", async () => {
    readCall.mockResolvedValueOnce(payload("tool", [], null, leftTo(null)));
    const h = harness();
    await flush();

    expect(h.latest().leftGateModels.has("codex")).toBe(true);
    expect(h.latest().leftGateModels.get("codex")).toBeNull();
  });

  it("clears the report on the next save for that tool, and only that tool", async () => {
    readCall.mockResolvedValueOnce(
      payload("tool", [], null, {
        ...leftTo("gpt-6-sol"),
        hermes: { state: "not_applied", model: null, left_gate_models: true, left_to_model: null },
      }),
    );
    const h = harness();
    await flush();
    expect(h.latest().leftGateModels.size).toBe(2);

    readCall.mockResolvedValue(payload("gate", ["openai/gpt-5"]));
    writeCall.mockResolvedValue(true);
    await act(async () => {
      await h.latest().save("codex", "gate", ["openai/gpt-5"], true);
    });

    expect(h.latest().leftGateModels.has("codex")).toBe(false);
    expect(h.latest().leftGateModels.has("hermes")).toBe(true);
  });

  it("keeps a drift report from a read that a newer one overtook", async () => {
    // Two reads in flight - a quick double focus. The FIRST carries the one
    // report of Codex leaving Gate models; the second, which resolves first
    // and wins the view, does not (review on #382).
    let resolveFirst: (v: unknown) => void = () => undefined;
    readCall
      .mockResolvedValueOnce(payload("gate", ["openai/gpt-5"], 1))
      .mockImplementationOnce(() => new Promise((r) => (resolveFirst = r)))
      .mockResolvedValueOnce(payload("tool", ["openai/gpt-5"], 1));
    const h = harness();
    await flush();

    act(() => {
      h.latest().reload();
      h.latest().reload();
    });
    await flush();
    expect(h.latest().view?.byTool.get("codex")?.source).toBe("tool");

    await act(async () => {
      resolveFirst(payload("tool", ["openai/gpt-5"], 1, leftTo("gpt-6-sol")));
    });
    expect(h.latest().leftGateModels.get("codex")).toBe("gpt-6-sol");
    // And the stale view did not replace the newer one.
    expect(h.latest().view?.configured.get("codex")).toBeUndefined();
  });
});

