import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent, RefObject } from "react";

/**
 * The keyboard and focus behaviour a `role="menu"` panel owes its users.
 *
 * Extracted because the app draws that panel twice - `TrayMenu` in the
 * popover's footer, `TopnavMenu` in the window's topbar - and the two are
 * deliberately NOT one component: they draw different shadows, paddings, glyph
 * sizes and corners off different frames. What is not different is any of this,
 * and the copies proved it by diverging. The topbar's menu shipped with
 * `role="menu"` promising arrow-key navigation it did not have, no dismissal
 * but the button that opened it, and every item in the tab order, while the
 * tray's copy had all three - because the fix was made on one surface and not
 * the other. So the frames win on everything they draw, and the behaviour has
 * one home.
 *
 * The caller supplies the markup and the scrim; this supplies the panel ref,
 * the roving tab stop and the key handler.
 */
export function useRovingMenu(
  /** The button the menu opened from. Focus returns to it when the panel
   *  unmounts - every exit unmounts it, and with the focused element gone
   *  `activeElement` falls to `<body>`, so the next Tab would restart from the
   *  top of the surface rather than from the control the user was just on. */
  triggerRef: RefObject<HTMLButtonElement> | undefined,
  /** Close without choosing: Escape from inside the panel. A click outside is
   *  the caller's scrim, and goes through the same handler. */
  onDismiss: () => void,
) {
  const panel = useRef<HTMLDivElement>(null);
  /**
   * Which item is the menu's single tab stop (roving tabindex).
   *
   * `role="menu"` is meant to be ONE stop, with the arrows moving inside it. As
   * plain buttons every item was in the tab order, so Tab past the last one
   * walked into the surface behind the open menu - visible beside it, and
   * click-blocked by the scrim, so the focus ring went somewhere the pointer
   * could not follow.
   */
  const [current, setCurrent] = useState(0);

  /**
   * Focus the first item when the menu opens.
   *
   * Without it the menu is unreachable by keyboard: it opens from a button, so
   * focus stays on that button and Tab walks into the surface *behind* the menu
   * rather than into it. `role="menu"` promises arrow-key navigation, so the
   * roles were describing behaviour that did not exist.
   */
  useEffect(() => {
    panel.current?.querySelector<HTMLButtonElement>("[role='menuitem']")?.focus();
    // One cleanup covers every exit: Escape, the scrim and selecting an item all
    // unmount the panel.
    const trigger = triggerRef;
    return () => trigger?.current?.focus();
  }, [triggerRef]);

  /**
   * Arrow keys move between items, Escape closes, Home/End jump.
   *
   * Wrapping at both ends, which is what a menu does and what a listbox does
   * not. `stopPropagation` on Escape so the shell's document-level handler does
   * not also fire on the same key - both surfaces keep one, for the case where
   * Tab has moved focus out of a still-open menu, and two handlers on one key
   * would call a *toggle* twice and leave the menu open. `preventDefault` on
   * the arrows because the content behind the menu scrolls: without it Down
   * would move the selection *and* scroll what is underneath.
   */
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      onDismiss();
      return;
    }
    const keys = ["ArrowDown", "ArrowUp", "Home", "End"];
    if (!keys.includes(e.key)) return;
    const items = Array.from(
      panel.current?.querySelectorAll<HTMLButtonElement>("[role='menuitem']") ?? [],
    );
    if (items.length === 0) return;
    e.preventDefault();
    const at = items.indexOf(document.activeElement as HTMLButtonElement);
    const next =
      e.key === "Home"
        ? 0
        : e.key === "End"
          ? items.length - 1
          : e.key === "ArrowDown"
            ? (at + 1 + items.length) % items.length
            : (at - 1 + items.length) % items.length;
    setCurrent(next);
    items[next]?.focus();
  };

  return { panel, current, onKeyDown };
}
