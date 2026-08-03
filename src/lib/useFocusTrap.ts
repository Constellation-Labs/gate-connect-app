import { useEffect, useRef } from "react";
import type { RefObject } from "react";

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** Dialog focus behavior for the full-popover takeovers: moves focus into the
 *  panel on mount, keeps Tab cycling inside it, fires `onEscape` on Escape,
 *  and hands focus back to the previously focused element on unmount. The
 *  popover never stacks dialogs, so one trap at a time is enough. */
export function useFocusTrap(
  ref: RefObject<HTMLElement>,
  onEscape?: () => void,
  /** Where focus lands on mount. Without it the trap takes the first
   * focusable in DOM order, which on every confirm panel here is the
   * destructive button - so a keyboard user who opened the panel with Enter
   * destroys something by pressing Enter again. Point this at the safe
   * choice whenever the primary action is `variant="danger"`. */
  initialFocus?: RefObject<HTMLElement>,
): void {
  // Keep the latest onEscape without re-running the trap effect (callers pass
  // inline closures).
  const escapeRef = useRef(onEscape);
  escapeRef.current = onEscape;

  useEffect(() => {
    const panel = ref.current;
    if (!panel) return;
    const previous = document.activeElement as HTMLElement | null;

    const focusables = () =>
      Array.from(panel.querySelectorAll<HTMLElement>(FOCUSABLE));
    (initialFocus?.current ?? focusables()[0] ?? panel).focus();

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && escapeRef.current) {
        e.stopPropagation();
        escapeRef.current();
        return;
      }
      if (e.key !== "Tab") return;
      const items = focusables();
      if (items.length === 0) {
        e.preventDefault();
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      const active = document.activeElement;
      if (e.shiftKey && (active === first || !panel.contains(active))) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && (active === last || !panel.contains(active))) {
        e.preventDefault();
        first.focus();
      }
    };

    panel.addEventListener("keydown", onKeyDown);
    return () => {
      panel.removeEventListener("keydown", onKeyDown);
      previous?.focus?.();
    };
  }, [ref]);
}
