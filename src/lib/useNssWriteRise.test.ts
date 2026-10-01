import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useNssWriteRise } from "./useNssWriteRise";

describe("useNssWriteRise", () => {
  beforeEach(() => sessionStorage.clear());

  const mount = (writes: number | null, onRise = vi.fn()) => {
    const hook = renderHook(({ w }) => useNssWriteRise(w, onRise), {
      initialProps: { w: writes },
    });
    return { ...hook, onRise };
  };

  it("fires on a write, including the first reading", () => {
    expect(mount(0).onRise).not.toHaveBeenCalled();
    sessionStorage.clear();
    expect(mount(1).onRise).toHaveBeenCalledOnce();
  });

  it("fires again only when the count goes up", () => {
    const { rerender, onRise } = mount(0);
    rerender({ w: 2 });
    rerender({ w: 2 });
    expect(onRise).toHaveBeenCalledOnce();
    rerender({ w: 3 });
    expect(onRise).toHaveBeenCalledTimes(2);
  });

  it("does not fire after a failed status read", () => {
    const { rerender, onRise } = mount(2);
    onRise.mockClear();
    rerender({ w: null });
    rerender({ w: 2 });
    expect(onRise).not.toHaveBeenCalled();
  });

  it("does not fire after a webview reload", () => {
    mount(2).unmount();
    expect(mount(2).onRise).not.toHaveBeenCalled();
  });

  it("treats a count below the last one seen as a restarted app that wrote", () => {
    mount(3).unmount();
    expect(mount(1).onRise).toHaveBeenCalledOnce();
  });
});
