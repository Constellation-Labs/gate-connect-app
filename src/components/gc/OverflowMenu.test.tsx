import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { OverflowMenu } from "./OverflowMenu";

const noop = () => {};

afterEach(cleanup);

/**
 * The drawn values `OverflowMenu` exists to hold in one place. Behaviour is
 * covered at each surface's wiring (`Topbar.test.tsx`, `Tray.test.tsx`); these
 * are the visual fixes that came with merging the two copies, so a class that
 * drifts back fails here rather than on screen.
 */
describe.each(["topbar", "tray"] as const)("OverflowMenu (%s)", (surface) => {
  it("draws no rule under its rows", () => {
    // `shadow/2xs` renders as nothing in Figma on a row with no fill, and as
    // a 1px line in CSS.
    render(<OverflowMenu surface={surface} onSelect={noop} onDismiss={noop} />);
    for (const item of screen.getAllByRole("menuitem")) {
      expect(item.className).not.toContain("shadow");
    }
  });

  it("pads the panel 8px, as both frames draw", () => {
    render(<OverflowMenu surface={surface} onSelect={noop} onDismiss={noop} />);
    expect(screen.getByRole("menu").className.split(" ")).toContain("p-2");
  });

  it("gives Quit label/12's tracking, like every other row", () => {
    render(<OverflowMenu surface={surface} onSelect={noop} onDismiss={noop} />);
    const quit = screen.getByRole("menuitem", { name: "Quit Gate Connect" });
    expect(quit.querySelector(".tracking-label-12")).not.toBeNull();
  });
});
