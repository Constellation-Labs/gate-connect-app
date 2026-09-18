import { drainBackendErrors } from "./api";
import { backendErrorContext, classifyError, type ClassifiedError } from "./errors";
import { trackError } from "./analytics";

/**
 * The backend's buffered failures, drained into the analytics seam and - for the
 * ones that mean routing is down - handed back so a human sees them too.
 *
 * Lifted out of `App.tsx` so both shells share one copy. The window shell had no
 * drain at all, which reintroduced exactly the bug the popover's version was
 * written to fix: a buffered failure produced zero pixels of UI, in the one app
 * whose first principle is reassurance through transparency, on the one error
 * class the user cannot discover any other way. A second implementation would
 * have been a second chance to drift.
 */

/**
 * Contexts that mean **routing is not working right now**, as opposed to a
 * one-off that failed and can be retried.
 *
 * These are the failures a user cannot find any other way: they happen in the
 * Rust layer, often before the webview exists (the startup auto-enable runs
 * before either shell mounts), so nothing on screen would otherwise change.
 */
const ROUTING_DOWN_CONTEXTS = new Set([
  "restore_routing",
  "provider_restore",
  "provider_reconcile",
]);

/**
 * Drain this window's buffer. The first routing-down failure is returned for
 * display; whether the batch also goes to analytics is the caller's to say.
 *
 * The raw message is classified frontend-side like any invoke rejection, so only
 * the classified title ever goes over the wire - the message itself stays on this
 * machine.
 *
 * `reportToAnalytics` is not a preference, it is a de-duplication, and it has to
 * be decided by the caller because only the caller knows which shell it is. The
 * Rust buffer queues a copy of every failure per webview label so the two shells
 * cannot race over one take (`PENDING_BACKEND_ERRORS`), and both shells are
 * mounted from launch - so a drain that always reported would emit two
 * `error_shown` events and two `captureException` records per failure, one of
 * them from a hidden webview where nothing was shown. Display wants both copies;
 * analytics wants one. `main` is the reporter because it is always mounted,
 * so nothing is lost by the tray staying quiet.
 *
 * Returns `null` when the buffer was empty or held nothing worth interrupting
 * for, which is the common case.
 */
export async function forwardBackendErrors({
  reportToAnalytics,
}: {
  reportToAnalytics: boolean;
}): Promise<ClassifiedError | null> {
  const errs = await drainBackendErrors().catch(() => []);
  let surfaced: ClassifiedError | null = null;
  for (const e of errs) {
    const context = backendErrorContext(e.context);
    if (reportToAnalytics) trackError(e.message, context);
    if (!surfaced && ROUTING_DOWN_CONTEXTS.has(e.context)) {
      surfaced = classifyError(e.message, context);
    }
  }
  return surfaced;
}
