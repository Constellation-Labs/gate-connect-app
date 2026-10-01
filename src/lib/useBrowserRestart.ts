import { useCallback, useEffect, useRef, useState } from "react";

/** Where the last `ca_nss_writes` reading is kept across a webview reload, so a
 * reload (dev HMR, or the webview being recreated) does not bring back a
 * notice the user already dismissed. Session-scoped: a new app process starts
 * its counter at zero, and a stale reading from an earlier one is handled
 * below rather than trusted. */
const SEEN_KEY = "gate.caNssWritesSeen";

function readSeen(): number {
  try {
    const seen = Number(sessionStorage.getItem(SEEN_KEY));
    return Number.isFinite(seen) ? seen : 0;
  } catch {
    return 0;
  }
}

function writeSeen(writes: number) {
  try {
    sessionStorage.setItem(SEEN_KEY, String(writes));
  } catch {
    /* storage unavailable: a reload may re-raise the notice, nothing worse */
  }
}

/** Whether Home's "quit and reopen your browsers" notice is up, from the
 * backend's count of browser-store writes (`ProxyState.ca_nss_writes`).
 *
 * A running browser keeps the store it opened at launch, so every write is
 * news until dismissed - including one on the first reading, since the startup
 * reconcile can write before any window has asked for anything. A null reading
 * (a failed status read) is skipped rather than read as zero, or the next good
 * reading would look like a fresh write. A count *below* the last one seen
 * means the app process restarted, so a non-zero count there is a write too. */
export function useBrowserRestart(writes: number | null): [boolean, () => void] {
  const [show, setShow] = useState(false);
  const seen = useRef<number | null>(null);
  useEffect(() => {
    if (writes === null) return;
    const before = seen.current ?? readSeen();
    if (writes > before || (writes < before && writes > 0)) setShow(true);
    seen.current = writes;
    writeSeen(writes);
  }, [writes]);
  const dismiss = useCallback(() => setShow(false), []);
  return [show, dismiss];
}
