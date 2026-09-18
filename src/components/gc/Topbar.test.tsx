import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { Topbar } from "./Topbar";

const noop = () => {};

/** Every prop, so a test can re-render with one of them changed - which is how
 *  the menu's focus-return is observed. */
function topbarProps(
  overrides: Partial<Parameters<typeof Topbar>[0]> = {},
): Parameters<typeof Topbar>[0] {
  return {
    menuOpen: false,
    onMenuToggle: noop,
    onMenuSelect: noop,
    ...overrides,
  };
}

function renderTopbar(overrides: Partial<Parameters<typeof Topbar>[0]> = {}) {
  return render(<Topbar {...topbarProps(overrides)} />);
}

afterEach(cleanup);

/**
 * The topbar's overflow menu. Every behaviour here is `TrayMenu`'s too, because
 * the two menus had diverged: the tray's closed on a click outside, on Escape
 * and under the arrow keys, and this one closed only by clicking the button
 * that had opened it. The interaction now has one home (`useRovingMenu`), and
 * these cases are the mirror of `Tray.test.tsx`'s - each surface is held to the
 * behaviour at its own wiring, so a menu that stops calling the hook, or calls
 * it wrong, fails here rather than shipping.
 */
describe("Topbar menu", () => {
  it("draws no menu and no scrim while it is closed", () => {
    // The scrim is a full-window click target that paints nothing, so a stray
    // one left mounted would swallow every click on the app with no visible
    // cause. It exists only while the menu does.
    const { container } = renderTopbar();

    expect(screen.queryByRole("menu")).toBeNull();
    expect(container.querySelector("div.fixed.inset-0")).toBeNull();
    expect(
      screen.getByRole("button", { name: "More" }).getAttribute("aria-expanded"),
    ).toBe("false");
  });

  it("carries all four drawn entries, in the drawn order", () => {
    renderTopbar({ menuOpen: true, onMenuSelect: vi.fn() });
    // `116:27225`, which draws Quit where the `topnav/menu` component
    // (`744:37692`) draws only the first three.
    expect(screen.getAllByRole("menuitem").map((el) => el.textContent)).toEqual([
      "Visit dashboard",
      "Contact support",
      "Read Gate docs",
      "Quit Gate Connect",
    ]);
  });

  it("reports the chosen action", () => {
    const onMenuSelect = vi.fn();
    renderTopbar({ menuOpen: true, onMenuSelect });

    fireEvent.click(screen.getByRole("menuitem", { name: "Read Gate docs" }));

    expect(onMenuSelect).toHaveBeenCalledWith("docs");
  });

  it("closes on a click outside, and that click does nothing else", () => {
    // The bug this fixes: the menu had one exit, the button that opened it. A
    // click elsewhere left it open *and* did whatever that click normally does,
    // so dismissing the menu could toggle the routing of an app row visible
    // beside it.
    const onMenuToggle = vi.fn();
    const onMenuSelect = vi.fn();
    const { container } = renderTopbar({ menuOpen: true, onMenuToggle, onMenuSelect });

    fireEvent.click(container.querySelector("div.fixed.inset-0") as HTMLElement);

    expect(onMenuToggle).toHaveBeenCalledTimes(1);
    expect(onMenuSelect).not.toHaveBeenCalled();
  });

  it("closes once on Escape from inside the panel, not twice", () => {
    // Escape is handled in two places - the panel, and a document listener for
    // when focus has left an open menu - so the panel stops propagation. Two
    // handlers on one key would call a *toggle* twice and leave the menu open.
    const onMenuToggle = vi.fn();
    const onMenuSelect = vi.fn();
    renderTopbar({ menuOpen: true, onMenuToggle, onMenuSelect });

    fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });

    expect(onMenuToggle).toHaveBeenCalledTimes(1);
    expect(onMenuSelect).not.toHaveBeenCalled();
  });

  it("closes on Escape after focus has left the open menu", () => {
    // The menu is a single tab stop, so Tab leaves it as a unit with the panel
    // still open and focus behind the scrim. Without the document handler that
    // state had no keyboard exit at all.
    const onMenuToggle = vi.fn();
    renderTopbar({ menuOpen: true, onMenuToggle });

    fireEvent.keyDown(document.body, { key: "Escape" });

    expect(onMenuToggle).toHaveBeenCalledTimes(1);
  });

  it("leaves Escape alone while the menu is closed", () => {
    // The document listener is registered only while the menu is open. A
    // permanent one would answer a key the window has other uses for, and the
    // handler is a toggle, so it would *open* the menu.
    const onMenuToggle = vi.fn();
    renderTopbar({ onMenuToggle });

    fireEvent.keyDown(document.body, { key: "Escape" });

    expect(onMenuToggle).not.toHaveBeenCalled();
  });

  it("focuses the first item when the menu opens, so the keyboard can reach it", () => {
    // `role="menu"` promises arrow-key navigation, and this menu had none. It
    // opens from a button, so focus stayed on that button and Tab walked into
    // the sidebar *behind* the panel rather than into it.
    renderTopbar({ menuOpen: true, onMenuSelect: vi.fn() });

    expect(document.activeElement?.textContent).toBe("Visit dashboard");
  });

  it("moves between items with the arrow keys, wrapping at both ends", () => {
    renderTopbar({ menuOpen: true, onMenuSelect: vi.fn() });
    const menu = screen.getByRole("menu");

    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(document.activeElement?.textContent).toBe("Contact support");
    fireEvent.keyDown(menu, { key: "ArrowUp" });
    expect(document.activeElement?.textContent).toBe("Visit dashboard");
    // Wrapping is what a menu does, and the last entry is the destructive one,
    // so arriving there by pressing Up once is deliberate rather than a slip.
    fireEvent.keyDown(menu, { key: "ArrowUp" });
    expect(document.activeElement?.textContent).toBe("Quit Gate Connect");
    fireEvent.keyDown(menu, { key: "Home" });
    expect(document.activeElement?.textContent).toBe("Visit dashboard");
    fireEvent.keyDown(menu, { key: "End" });
    expect(document.activeElement?.textContent).toBe("Quit Gate Connect");
  });

  it("is a single tab stop, so Tab leaves the menu rather than walking behind it", () => {
    // As four plain buttons every item was tabbable, so Tab past the last one
    // moved focus into the sidebar - visible beside the open menu and
    // click-blocked by the scrim, so the focus ring went where the pointer
    // could not follow.
    renderTopbar({ menuOpen: true, onMenuSelect: vi.fn() });
    const items = screen.getAllByRole("menuitem");

    expect(items.map((el) => el.getAttribute("tabindex"))).toEqual(["0", "-1", "-1", "-1"]);

    // The stop moves with the arrows: still exactly one, on the focused item.
    fireEvent.keyDown(screen.getByRole("menu"), { key: "End" });
    expect(items.map((el) => el.getAttribute("tabindex"))).toEqual(["-1", "-1", "-1", "0"]);
  });

  it("names the menu, so it does not announce as a bare menu", () => {
    // The mirror of `Tray.test.tsx`'s own case. A bare `role="menu"` announces
    // as just "menu"; named for the control that opens it, it reads
    // "More, menu".
    renderTopbar({ menuOpen: true, onMenuSelect: vi.fn() });

    expect(screen.getByRole("menu", { name: "More" })).toBeTruthy();
  });

  it("gives focus back to the trigger when it closes", () => {
    // Every exit unmounts the panel, so the focused element goes with it and
    // `activeElement` falls to `<body>` - the next Tab would restart from the
    // top of the window instead of the control the user was just on.
    const { rerender } = renderTopbar({ menuOpen: true, onMenuSelect: vi.fn() });
    expect(document.activeElement?.textContent).toBe("Visit dashboard");

    rerender(<Topbar {...topbarProps({ menuOpen: false, onMenuSelect: vi.fn() })} />);

    expect(document.activeElement?.getAttribute("aria-label")).toBe("More");
  });
});
