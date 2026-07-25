import { useEffect, useRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** Fires `onReopen` on each blur → focus edge of the current window.
 *
 * The tray reveals the popover with show() + set_focus(), so a focus gained
 * after a prior blur marks a genuine return to the window; the initial launch
 * never fires (no blur has happened yet), and refocus noise without an
 * intervening blur is ignored. This is the one place that encodes the
 * tray-reopen detection - App's popover_opened tracking and UpdatePanel's
 * update re-check both hang off it, so a change to the tray's show/focus
 * behavior lands in a single spot. */
export function useWindowReopen(onReopen: () => void): void {
  // Latest-callback ref so the listener registers once but never calls a
  // stale closure.
  const callback = useRef(onReopen);
  callback.current = onReopen;

  useEffect(() => {
    let blurred = false;
    const unlisten = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (!focused) {
        blurred = true;
        return;
      }
      if (blurred) {
        blurred = false;
        callback.current();
      }
    });
    return () => {
      void unlisten.then((f) => f()).catch(() => {});
    };
  }, []);
}
