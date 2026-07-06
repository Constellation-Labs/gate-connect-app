/**
 * First-launch onboarding state. A single "seen" flag in webview localStorage
 * (which survives Tauri restarts) gates the welcome intro so it shows exactly
 * once. The key is versioned so a materially-changed intro can be re-shown by
 * bumping the suffix (v3: the window-sized onboarding flow replaced the
 * in-popover tour).
 */

const SEEN_KEY = "gc.tour.v3.seen";

export function hasSeenTour(): boolean {
  try {
    return localStorage.getItem(SEEN_KEY) === "1";
  } catch {
    // Treat storage failures as "already seen" so we never trap the user in a
    // tour that can't record completion.
    return true;
  }
}

/** Persist the "do not show this intro again" choice in either direction. */
export function setTourSeen(seen: boolean): void {
  try {
    if (seen) {
      localStorage.setItem(SEEN_KEY, "1");
    } else {
      localStorage.removeItem(SEEN_KEY);
    }
  } catch {
    /* noop - best effort */
  }
}

export function markTourSeen(): void {
  setTourSeen(true);
}
