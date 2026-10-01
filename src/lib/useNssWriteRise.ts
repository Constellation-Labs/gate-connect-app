import { useEffect, useRef } from "react";

/** Where the last write time this window raised a note for is kept. Local
 * rather than session storage: closing the window and opening it again is a
 * new webview, and a reopen note the user dismissed must not come back with
 * it. Safe across app restarts because the value is a time, not a per-process
 * count - a new process's first write is always later than anything stored. */
const SEEN_KEY = "gate.caNssWrittenAtSeen";

function readSeen(): number {
  try {
    const seen = Number(localStorage.getItem(SEEN_KEY));
    return Number.isFinite(seen) ? seen : 0;
  } catch {
    return 0;
  }
}

function writeSeen(writtenAt: number) {
  try {
    localStorage.setItem(SEEN_KEY, String(writtenAt));
  } catch {
    /* storage unavailable: a reload may raise the note again, nothing worse */
  }
}

/** Calls `onRise` when the backend has written the CA to a browser store since
 * the last write this window raised a note for (`ProxyState.ca_nss_written_at`
 * rising).
 *
 * The window's "quit and reopen" note rises on `ca_trusted` going true, which
 * misses the case the Ubuntu report was: the system anchor trusted long ago,
 * and a browser store written on a later enable - a Chromium database just
 * created, a Firefox profile seen for the first time. A browser only reads its
 * store at launch, so each write is news.
 *
 * Counts the first reading too, since the startup enable can write before the
 * window has asked for anything. A null reading (a failed status read) is
 * skipped rather than read as zero. */
export function useNssWriteRise(writtenAt: number | null, onRise: () => void) {
  const rise = useRef(onRise);
  rise.current = onRise;
  useEffect(() => {
    if (writtenAt === null || writtenAt <= readSeen()) return;
    writeSeen(writtenAt);
    rise.current();
  }, [writtenAt]);
}
