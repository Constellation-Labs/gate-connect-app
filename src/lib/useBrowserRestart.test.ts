import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { useBrowserRestart } from "./useBrowserRestart";

describe("useBrowserRestart", () => {
  beforeEach(() => sessionStorage.clear());

  const render = (writes: number | null) =>
    renderHook(({ w }) => useBrowserRestart(w), { initialProps: { w: writes } });

  it("raises on a write, including the first reading", () => {
    expect(render(0).result.current[0]).toBe(false);
    sessionStorage.clear();
    expect(render(1).result.current[0]).toBe(true);
  });

  it("raises again only when the count goes up", () => {
    const { result, rerender } = render(0);
    rerender({ w: 2 });
    expect(result.current[0]).toBe(true);
    act(() => result.current[1]());
    rerender({ w: 2 });
    expect(result.current[0]).toBe(false);
    rerender({ w: 3 });
    expect(result.current[0]).toBe(true);
  });

  it("does not come back after a failed status read", () => {
    const { result, rerender } = render(2);
    act(() => result.current[1]());
    rerender({ w: null });
    rerender({ w: 2 });
    expect(result.current[0]).toBe(false);
  });

  it("does not come back after a webview reload", () => {
    const first = render(2);
    act(() => first.result.current[1]());
    first.unmount();
    expect(render(2).result.current[0]).toBe(false);
  });

  it("treats a count below the last one seen as a restarted app that wrote", () => {
    const first = render(3);
    act(() => first.result.current[1]());
    first.unmount();
    expect(render(1).result.current[0]).toBe(true);
  });
});
