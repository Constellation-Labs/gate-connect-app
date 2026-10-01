import { useEffect, useRef } from "react";

/** Where the last `ca_nss_writes` reading is kept across a webview reload, so a
 * reload (dev HMR, or the webview being recreated) does not raise again a note
 * the user already dismissed. Session-scoped: a new app process starts its
 * counter at zero, and a stale reading from an earlier one is handled below
 * rather than trusted. */
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
    /* storage unavailable: a reload may raise the note again, nothing worse */
  }
}

/** Calls `onRise` when the backend has written the CA to another browser store
 * (`ProxyState.ca_nss_writes` going up).
 *
 * The window's "quit and reopen" note rises on `ca_trusted` going true, which
 * misses the case the Ubuntu report was: the system anchor trusted long ago,
 * and a browser store written on a later enable - a Chromium database just
 * created, a Firefox profile seen for the first time. A browser only reads its
 * store at launch, so each write is news.
 *
 * Counts the first reading too, since the startup enable can write before the
 * window has asked for anything. A null reading (a failed status read) is
 * skipped rather than read as zero, or the next good reading would look like a
 * fresh write. A count *below* the last one seen means the app process
 * restarted, so a non-zero count there is a write too. */
export function useNssWriteRise(writes: number | null, onRise: () => void) {
  const seen = useRef<number | null>(null);
  const rise = useRef(onRise);
  rise.current = onRise;
  useEffect(() => {
    if (writes === null) return;
    const before = seen.current ?? readSeen();
    seen.current = writes;
    writeSeen(writes);
    if (writes > before || (writes < before && writes > 0)) rise.current();
  }, [writes]);
}
