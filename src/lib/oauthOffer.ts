/** Whether the one-time "sign in instead of pasting a key" offer has been
 * answered.
 *
 * The offer exists for installs that predate OAuth, or that took the key path
 * before deciding. It must never reach someone who just chose the key
 * deliberately on FirstRun - and gating on the tour does not achieve that,
 * because the tour window opens and completes *before* FirstRun renders. So
 * FirstRun stamps this itself when a key connects.
 *
 * A storage failure reads as answered: nobody should be trapped in a
 * recurring offer. */
const SEEN_KEY = "gc.oauth-offer.v1.seen";

export function hasSeenOAuthOffer(): boolean {
  try {
    return localStorage.getItem(SEEN_KEY) === "1";
  } catch {
    return true;
  }
}

export function markOAuthOfferSeen(): void {
  try {
    localStorage.setItem(SEEN_KEY, "1");
  } catch {
    /* noop */
  }
}
