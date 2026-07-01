/**
 * First-launch tour state. A single "seen" flag in webview localStorage
 * (which survives Tauri restarts) gates the welcome tour so it shows exactly
 * once. The key is versioned so a materially-changed tour can be re-shown by
 * bumping the suffix.
 */

const SEEN_KEY = "gc.tour.v2.seen";

export function hasSeenTour(): boolean {
  try {
    return localStorage.getItem(SEEN_KEY) === "1";
  } catch {
    // Treat storage failures as "already seen" so we never trap the user in a
    // tour that can't record completion.
    return true;
  }
}

export function markTourSeen(): void {
  try {
    localStorage.setItem(SEEN_KEY, "1");
  } catch {
    /* noop - best effort */
  }
}
