import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import { useGateModels } from "./toolModels";

vi.mock("./api", () => ({
  gateCredits: vi.fn(),
  gateModelCatalogue: vi.fn(),
  setToolModel: vi.fn(),
  toolModelPreferences: vi.fn(),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ onFocusChanged: () => Promise.resolve(() => undefined) }),
}));
import { gateModelCatalogue } from "./api";

/**
 * The hook, not the adapter - `toolModels.test.ts` covers `adaptModels`.
 *
 * `enabled` used to be the picker's visibility, and closing and reopening the
 * picker was what retried a failed read and what, across a sign-out, read the
 * next account's catalogue. It is constant for a signed-in session now, so both
 * have to be deliberate: a failure must not retry itself, `reload` must retry
 * it, and a new credential must drop the catalogue and read again.
 */

/** One catalogue read the test resolves by hand. */
function pending() {
  let resolve!: (text: string) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<string>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const catalogue = (...ids: string[]) =>
  JSON.stringify({ data: ids.map((id) => ({ id, owned_by: id.split("/")[0], name: id })) });

let latest: ReturnType<typeof useGateModels>;
function Probe({ enabled, credential }: { enabled: boolean; credential: string }) {
  latest = useGateModels(enabled, credential);
  return null;
}

const read = vi.mocked(gateModelCatalogue);
const flush = () => act(() => Promise.resolve());

beforeEach(() => read.mockReset());
afterEach(cleanup);

describe("useGateModels", () => {
  it("reads once, and does not retry a failure on its own", async () => {
    const first = pending();
    read.mockReturnValueOnce(first.promise);
    const view = render(<Probe enabled credential="a" />);
    await act(async () => {
      first.reject(new Error("offline"));
      await first.promise.catch(() => undefined);
    });

    expect(latest.failure).not.toBeNull();
    expect(latest.models).toBeNull();
    // A re-render with nothing changed is what a failure used to be followed by
    // in the old picker-toggle world. It must not be a second request now.
    view.rerender(<Probe enabled credential="a" />);
    await flush();
    expect(read).toHaveBeenCalledTimes(1);
  });

  it("retries through reload, which is what opening the picker calls", async () => {
    read.mockRejectedValueOnce(new Error("offline"));
    render(<Probe enabled credential="a" />);
    await flush();
    expect(latest.failure).not.toBeNull();

    read.mockResolvedValueOnce(catalogue("anthropic/claude-opus-5"));
    await act(async () => latest.reload());
    await flush();

    expect(read).toHaveBeenCalledTimes(2);
    expect(latest.failure).toBeNull();
    expect(latest.models?.map((m) => m.id)).toEqual(["anthropic/claude-opus-5"]);
  });

  it("forgets the catalogue when the credential changes, and reads the new one", async () => {
    read.mockResolvedValueOnce(catalogue("anthropic/claude-opus-5"));
    const view = render(<Probe enabled credential="a" />);
    await flush();
    expect(latest.models).toHaveLength(1);

    const second = pending();
    read.mockReturnValueOnce(second.promise);
    view.rerender(<Probe enabled credential="b" />);
    await flush();
    // Dropped at once, not when the new read lands: the rows of the new account
    // must not be labelled from the old one's catalogue in the meantime.
    expect(latest.models).toBeNull();
    expect(read).toHaveBeenCalledTimes(2);

    await act(async () => second.resolve(catalogue("openai/gpt-6-luna")));
    expect(latest.models?.map((m) => m.id)).toEqual(["openai/gpt-6-luna"]);
  });

  it("drops a read still in flight for the previous credential", async () => {
    const first = pending();
    const second = pending();
    read.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const view = render(<Probe enabled credential="a" />);
    view.rerender(<Probe enabled credential="b" />);
    await flush();

    // The old gateway answers late. Its catalogue is not this account's.
    await act(async () => first.resolve(catalogue("anthropic/claude-opus-5")));
    expect(latest.models).toBeNull();

    await act(async () => second.resolve(catalogue("openai/gpt-6-luna")));
    expect(latest.models?.map((m) => m.id)).toEqual(["openai/gpt-6-luna"]);
  });

  it("reads nothing while disabled", async () => {
    render(<Probe enabled={false} credential="a" />);
    await flush();
    expect(read).not.toHaveBeenCalled();
    expect(latest.models).toBeNull();
  });
});
