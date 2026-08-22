import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import { useToolModels } from "./toolModels";

vi.mock("./api", () => ({
  toolModelPreferences: vi.fn(),
  setToolModel: vi.fn(),
  gateModelCatalogue: vi.fn(),
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

function payload(source: "tool" | "gate", modelIds: string[], firstPaidAckAt: string | null = null) {
  return JSON.stringify({
    generatedAt: "2026-08-22T10:00:00.000Z",
    org: { orgId: "org-1", name: "Acme" },
    preferences: [{ platformId: "codex", source, modelIds, updatedAt: "2026-08-22T10:00:00.000Z" }],
    firstPaidAckAt,
  });
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
});
afterEach(cleanup);

type SaveResult = Awaited<ReturnType<ReturnType<typeof useToolModels>["save"]>>;

describe("useToolModels", () => {
  it("reads once and exposes the preferences by platform", async () => {
    readCall.mockResolvedValue(payload("gate", ["openai/gpt-5"], "2026-01-01T00:00:00.000Z"));
    const h = harness();
    await flush();

    expect(readCall).toHaveBeenCalledTimes(1);
    expect(h.latest().view?.byPlatform.get("codex")?.source).toBe("gate");
    expect(h.latest().view?.firstPaidAckAt).toBe("2026-01-01T00:00:00.000Z");
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
    writeCall.mockResolvedValue("{}");
    const h = harness();
    await flush();
    expect(readCall).toHaveBeenCalledTimes(1);

    readCall.mockResolvedValue(payload("gate", ["openai/gpt-5"], "2026-08-22T10:00:01.000Z"));
    await act(async () => {
      await h.latest().save("codex", "gate", ["openai/gpt-5"], true);
    });

    expect(readCall).toHaveBeenCalledTimes(2);
    expect(h.latest().view?.byPlatform.get("codex")?.source).toBe("gate");
    expect(h.latest().view?.firstPaidAckAt).toBe("2026-08-22T10:00:01.000Z");
  });

  it("passes the acknowledgement through, and defaults it to withheld", async () => {
    readCall.mockResolvedValue(payload("tool", []));
    writeCall.mockResolvedValue("{}");
    const h = harness();
    await flush();

    await act(async () => {
      await h.latest().save("codex", "tool", []);
    });
    expect(writeCall).toHaveBeenCalledWith("codex", "tool", [], false);
  });

  it("returns a write failure rather than throwing, so the caller can branch", async () => {
    // `needs_paid_ack` is the case this exists for: it is not an error to report
    // but the signal to raise the billing dialog.
    readCall.mockResolvedValue(payload("tool", []));
    writeCall.mockRejectedValue('{"code":"needs_paid_ack","message":"show the confirmation"}');
    const h = harness();
    await flush();

    // Held on an object rather than in a `let`: TypeScript cannot see the
    // assignment inside `act`'s callback and narrows a bare local to `never`.
    const got: { failure: SaveResult } = { failure: null };
    await act(async () => {
      got.failure = await h.latest().save("codex", "gate", ["openai/gpt-5"]);
    });

    expect(got.failure?.code).toBe("needs_paid_ack");
    // No re-read: nothing was stored, so the held view is still current.
    expect(readCall).toHaveBeenCalledTimes(1);
  });

  it("keeps a role refusal's own sentence, which no code carries", async () => {
    readCall.mockResolvedValue(payload("tool", []));
    writeCall.mockRejectedValue(
      '{"code":"rejected","message":"Your role can view this organization\'s model settings but not change them."}',
    );
    const h = harness();
    await flush();

    // Held on an object rather than in a `let`: TypeScript cannot see the
    // assignment inside `act`'s callback and narrows a bare local to `never`.
    const got: { failure: SaveResult } = { failure: null };
    await act(async () => {
      got.failure = await h.latest().save("codex", "gate", ["openai/gpt-5"]);
    });

    expect(got.failure?.message).toContain("not change them");
  });
});
