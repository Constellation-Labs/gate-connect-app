import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useNssWriteRise } from "./useNssWriteRise";

describe("useNssWriteRise", () => {
  beforeEach(() => localStorage.clear());

  const mount = (writtenAt: number | null, onRise = vi.fn()) => {
    const hook = renderHook(({ w }) => useNssWriteRise(w, onRise), {
      initialProps: { w: writtenAt },
    });
    return { ...hook, onRise };
  };

  it("fires on a write, including the first reading", () => {
    expect(mount(0).onRise).not.toHaveBeenCalled();
    expect(mount(1_000).onRise).toHaveBeenCalledOnce();
  });

  it("fires again only when a later write lands", () => {
    const { rerender, onRise } = mount(0);
    rerender({ w: 1_000 });
    rerender({ w: 1_000 });
    expect(onRise).toHaveBeenCalledOnce();
    rerender({ w: 2_000 });
    expect(onRise).toHaveBeenCalledTimes(2);
  });

  it("does not fire after a failed status read", () => {
    const { rerender, onRise } = mount(1_000);
    onRise.mockClear();
    rerender({ w: null });
    rerender({ w: 1_000 });
    expect(onRise).not.toHaveBeenCalled();
  });

  it("does not fire again in a new window for a write it already showed", () => {
    mount(1_000).unmount();
    expect(mount(1_000).onRise).not.toHaveBeenCalled();
  });

  it("fires for a restarted app's first write, whatever an earlier one showed", () => {
    // A count would restart at 1 and sit below an earlier process's 3; a time
    // is always later.
    mount(3_000).unmount();
    expect(mount(5_000).onRise).toHaveBeenCalledOnce();
  });
});
