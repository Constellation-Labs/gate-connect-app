import { useEffect, useRef } from "react";
import type { RefObject } from "react";

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** Dialog focus behavior for the full-popover takeovers: moves focus into the
 *  panel on mount and again on each step change, keeps Tab cycling inside it,
 *  fires `onEscape` on Escape, and hands focus back to the previously focused
 *  element on unmount. The popover never stacks dialogs, so one trap at a time
 *  is enough. */
export function useFocusTrap(
  ref: RefObject<HTMLElement>,
  onEscape?: () => void,
  /** Where focus lands on mount and after each step change. Without it the
   * trap takes the first focusable in DOM order, which on every confirm panel
   * here is the destructive button - so a keyboard user who opened the panel
   * with Enter destroys something by pressing Enter again. Point this at the
   * safe choice whenever the primary action is `variant="danger"`. */
  initialFocus?: RefObject<HTMLElement>,
  /** Changes whenever the panel swaps its contents for a new step. Without it
   * the trap was a mount-time affair: when the focused control unmounted on a
   * step change, focus fell to `document.body`, and from there Escape did
   * nothing and Tab walked into the page behind the dialog. */
  resetKey?: unknown,
): void {
  // Keep the latest onEscape without re-running the trap effect (callers pass
  // inline closures).
  const escapeRef = useRef(onEscape);
  escapeRef.current = onEscape;

  // Key handling binds once per mount. It must NOT re-run on a step change:
  // its cleanup restores focus to whatever was focused before the dialog
  // opened, and doing that mid-dialog would eject the user from the panel.
  useEffect(() => {
    const panel = ref.current;
    if (!panel) return;
    const previous = document.activeElement as HTMLElement | null;

    const focusables = () => Array.from(panel.querySelectorAll<HTMLElement>(FOCUSABLE));

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && escapeRef.current) {
        e.stopPropagation();
        escapeRef.current();
        return;
      }
      if (e.key !== "Tab") return;
      const items = focusables();
      // A panel can legitimately have nothing focusable mid-operation (the
      // update panel while installing, the quit confirm while busy). Swallow
      // Tab rather than let it walk into the page behind the dialog.
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

    // On `document`, not the panel. The panel only receives keys while focus
    // is inside it, which is exactly the condition that fails after a step
    // change; the `!panel.contains(active)` re-entry branch above was already
    // written and could never run.
    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("keydown", onKeyDown, true);
      previous?.focus?.();
    };
  }, [ref]);

  // Place focus on mount, and re-place it whenever the step changes.
  useEffect(() => {
    const panel = ref.current;
    if (!panel) return;
    const focusables = Array.from(panel.querySelectorAll<HTMLElement>(FOCUSABLE));
    // The panel itself is the last fallback, so focus never lands on
    // `document.body`. That needs it to be programmatically focusable.
    if (!panel.hasAttribute("tabindex")) panel.setAttribute("tabindex", "-1");

    // Try each candidate and verify it actually took focus. `initialFocus`
    // routinely points at a control that is `disabled` for the current step -
    // the quit confirm disables all three buttons while it works - and
    // `.focus()` on a disabled element is a silent no-op, so an unchecked
    // chain leaves focus on body precisely when the panel has nothing to
    // offer.
    for (const candidate of [initialFocus?.current, focusables[0], panel]) {
      if (!candidate) continue;
      candidate.focus();
      if (document.activeElement === candidate) break;
    }
  }, [ref, initialFocus, resetKey]);
}
