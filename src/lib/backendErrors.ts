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
 * Drain the buffer. Every failure goes to analytics; the first routing-down one
 * is returned for display.
 *
 * The raw message is classified frontend-side like any invoke rejection, so only
 * the classified title ever goes over the wire - the message itself stays on this
 * machine.
 *
 * Returns `null` when the buffer was empty or held nothing worth interrupting
 * for, which is the common case.
 */
export async function forwardBackendErrors(): Promise<ClassifiedError | null> {
  const errs = await drainBackendErrors().catch(() => []);
  let surfaced: ClassifiedError | null = null;
  for (const e of errs) {
    const context = backendErrorContext(e.context);
    trackError(e.message, context);
    if (!surfaced && ROUTING_DOWN_CONTEXTS.has(e.context)) {
      surfaced = classifyError(e.message, context);
    }
  }
  return surfaced;
}
