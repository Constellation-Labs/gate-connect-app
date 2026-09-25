/**
 * Switch between the new 1024x720 window UI and the legacy menu-bar popover.
 *
 * **The new UI is the default.** The popover is the escape hatch now, not the
 * other way round, so set `VITE_NEW_UI=0` at build time or run
 * `gcNewUi(false)` in devtools to get back to it. That matters while the new
 * shell's routing actions are still inert: the popover is the only surface that
 * can actually change what is routed.
 *
 * Runtime rather than build-time so the fallback does not need a rebuild.
 *
 * This whole module goes away once the popover screens are deleted.
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
  return import.meta.env.VITE_NEW_UI !== "0";
}

export function setNewUiEnabled(on: boolean): void {
  try {
    localStorage.setItem(KEY, on ? "1" : "0");
  } catch {
    // Same reasoning: a flag that cannot persist is not worth an exception.
  }
}

// Deliberately not dev-only. The popover is the fallback for a shell whose
// routing actions are still inert, so getting back to it must not require a
// rebuild.
(window as unknown as Record<string, unknown>).gcNewUi = (on = true) => {
  setNewUiEnabled(on);
  location.reload();
};
