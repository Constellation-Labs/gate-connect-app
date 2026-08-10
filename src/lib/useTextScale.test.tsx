import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import { TEXT_SCALES, applyTextScale, readStoredScale, useTextScale } from "./useTextScale";

const KEY = "gc.text-scale.v1";

function Harness() {
  const { scale } = useTextScale();
  return <div data-testid="scale">{scale}</div>;
}

function rootPx(): number {
  return Number.parseFloat(document.documentElement.style.fontSize);
}

/** The accelerators listen on `window`, so drive them there. */
function press(key: string, meta = true) {
  act(() => {
    window.dispatchEvent(
      new KeyboardEvent("keydown", { key, metaKey: meta, ctrlKey: !meta, bubbles: true }),
    );
  });
}

beforeEach(() => {
  localStorage.clear();
  document.documentElement.style.fontSize = "";
});

afterEach(() => {
  cleanup();
  localStorage.clear();
  document.documentElement.style.fontSize = "";
});

describe("readStoredScale", () => {
  it("defaults to 100% with nothing stored", () => {
    expect(readStoredScale()).toBe(1);
  });

  it("reads a stored step back", () => {
    localStorage.setItem(KEY, "1.5");
    expect(readStoredScale()).toBe(1.5);
  });

  it("snaps an off-scale value to the nearest step", () => {
    // A value from a future build, a hand-edited store, or a step that was
    // removed. Anything is better than an arbitrary root size.
    localStorage.setItem(KEY, "1.7");
    expect(readStoredScale()).toBe(1.75);
    localStorage.setItem(KEY, "1.62");
    expect(readStoredScale()).toBe(1.5); // 0.12 below 1.5 beats 0.13 below 1.75
    localStorage.setItem(KEY, "4");
    expect(readStoredScale()).toBe(2); // never past the top step
  });

  it("falls back to 100% on garbage", () => {
    localStorage.setItem(KEY, "not-a-number");
    expect(readStoredScale()).toBe(1);
  });
});

describe("applyTextScale", () => {
  it("scales the rem root, which is what the whole ramp is authored against", () => {
    applyTextScale(1);
    expect(rootPx()).toBe(16);
    applyTextScale(2);
    expect(rootPx()).toBe(32);
  });
});

describe("useTextScale", () => {
  it("applies the stored scale on mount", () => {
    localStorage.setItem(KEY, "1.5");
    render(<Harness />);
    expect(rootPx()).toBe(24);
  });

  it("steps up and down with the platform accelerator, and persists", () => {
    const { getByTestId } = render(<Harness />);
    expect(getByTestId("scale").textContent).toBe("1");

    press("+");
    expect(getByTestId("scale").textContent).toBe("1.25");
    expect(rootPx()).toBe(20);
    expect(localStorage.getItem(KEY)).toBe("1.25");

    // `=` is the unshifted key that carries `+` on most layouts.
    press("=");
    expect(getByTestId("scale").textContent).toBe("1.5");

    press("-");
    expect(getByTestId("scale").textContent).toBe("1.25");
  });

  it("works with Ctrl as well as Cmd, so Windows and Linux are not locked out", () => {
    const { getByTestId } = render(<Harness />);
    press("+", false);
    expect(getByTestId("scale").textContent).toBe("1.25");
  });

  it("ignores the keys without the modifier, so typing + in a field is safe", () => {
    const { getByTestId } = render(<Harness />);
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "+", bubbles: true }));
    });
    expect(getByTestId("scale").textContent).toBe("1");
  });

  it("clamps at both ends instead of wrapping", () => {
    const { getByTestId } = render(<Harness />);
    for (let i = 0; i < 8; i++) press("+");
    expect(getByTestId("scale").textContent).toBe(String(TEXT_SCALES[TEXT_SCALES.length - 1]));
    expect(rootPx()).toBe(32);
    for (let i = 0; i < 8; i++) press("-");
    expect(getByTestId("scale").textContent).toBe("1");
    expect(rootPx()).toBe(16);
  });

  it("resets to 100% on modifier-0", () => {
    localStorage.setItem(KEY, "2");
    const { getByTestId } = render(<Harness />);
    expect(getByTestId("scale").textContent).toBe("2");
    press("0");
    expect(getByTestId("scale").textContent).toBe("1");
    expect(rootPx()).toBe(16);
    expect(localStorage.getItem(KEY)).toBe("1");
  });

  it("reaches 200%, which is what WCAG 1.4.4 asks for", () => {
    expect(TEXT_SCALES).toContain(2);
  });
});
