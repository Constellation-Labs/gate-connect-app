/**
 * Dev switch between the shipping menu-bar popover and the new 1024x720 window
 * UI, so both can exist in one build while the new one is finished.
 *
 * Runtime rather than build-time, because the point is to click through the new
 * shell against real data without rebuilding, and to get back out of it when
 * something is unfinished. The Vite variable only supplies the default.
 *
 * From devtools:
 *
 *     gcNewUi(true)    // switch to the new shell and reload
 *     gcNewUi(false)   // back to the popover
 *
 * This whole module goes away with the popover in Phase 9.
 */

const KEY = "gc.newUi";

export function newUiEnabled(): boolean {
  try {
    const stored = localStorage.getItem(KEY);
    if (stored !== null) return stored === "1";
  } catch {
    // Storage can be unavailable (disabled, quota, exotic webview). Fall
    // through to the build-time default rather than taking the app down over a
    // dev flag.
  }
  return import.meta.env.VITE_NEW_UI === "1";
}

export function setNewUiEnabled(on: boolean): void {
  try {
    localStorage.setItem(KEY, on ? "1" : "0");
  } catch {
    // Same reasoning: a flag that cannot persist is not worth an exception.
  }
}

if (import.meta.env.DEV) {
  (window as unknown as Record<string, unknown>).gcNewUi = (on = true) => {
    setNewUiEnabled(on);
    location.reload();
  };
}
