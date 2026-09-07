/**
 * Sending one diagnostics report on demand (AG-603).
 *
 * Its own module, and deliberately not a function inside `lib/analytics.ts`,
 * because it breaks that module's one invariant on purpose. Everything there is
 * gated on `enabled` - the consent switch - and the criterion for this path is
 * the opposite: **Send uploads one report whether automatic collection is On or
 * Off.** Pressing Send *is* the consent, for this report and nothing else.
 *
 * That is why nothing here touches posthog-js. `opt_out_capturing` persists an
 * opt-out flag that the SDK checks on every `capture`, so a report sent through
 * the shared client would be silently dropped for exactly the users most likely
 * to be asked for one. Temporarily opting in and back out would work and is
 * worse: the window it opens is a window in which the *automatic* queue can also
 * drain, which turns "send one report" into "send everything you were holding".
 * A direct POST to the capture API sends one event and nothing else, and leaves
 * the consent flag untouched in both directions.
 *
 * The destination is PostHog because that is where this install's other events
 * already are: a report filed under the same distinct id lands beside the event
 * stream it explains, which is the whole reason a support thread wants one. The
 * gateway would be the other candidate and has no ingest route, no table and no
 * admin surface to read one back from.
 */
import { POSTHOG_HOST, POSTHOG_KEY_VALUE } from "./config";
import { analyticsId } from "./analytics";
import { installId } from "./api";

/**
 * Crockford's alphabet minus `I`, `L`, `O` and `U`: a reference gets read down a
 * phone line and typed into a support form by someone who did not generate it,
 * and those four are the characters that come back wrong.
 */
const ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/**
 * How long a send may hang before it is called a failure.
 *
 * `fetch` has no default timeout, and the dialog refuses to be dismissed while a
 * request is in flight - so without a ceiling here, one unanswered socket traps
 * the user in a dialog with no way out. 15s is the same ceiling
 * `gateway_api.rs` puts on a control-plane call, for the same reason.
 */
const SEND_TIMEOUT_MS = 15_000;

/**
 * A fresh reference for one report. `GC-XXXX-XXXX` - 40 bits, which is far more
 * than the collision arithmetic needs and short enough to read aloud.
 *
 * Minted here rather than returned by the server because the capture API answers
 * `{"status": 1}` and nothing else. That is not the compromise it looks like: the
 * reference's job is to be *searchable*, and a value we generated and sent as a
 * property is exactly as searchable as one we were handed back.
 */
export function newDiagnosticReference(): string {
  const bytes = new Uint8Array(8);
  crypto.getRandomValues(bytes);
  const body = Array.from(bytes, (b) => ALPHABET[b % ALPHABET.length]).join("");
  return `GC-${body.slice(0, 4)}-${body.slice(4)}`;
}

/**
 * Whether this build has anywhere to send a report.
 *
 * The shell hides the Settings row when this is false rather than letting the
 * user press a button that can only fail. It is false in every dev build -
 * `pnpm app:local` sets no `VITE_POSTHOG_KEY` - which is the common case for the
 * people most likely to press it by accident.
 */
export function canSendDiagnostics(): boolean {
  return POSTHOG_KEY_VALUE !== "";
}

/**
 * Which id to file the report under.
 *
 * The live analytics id first, so the report joins the events it explains. An
 * install that opted out never started the client and has no such id, so it
 * falls back to the install id - the same value the gateway sees on
 * `x-gate-install-id`, which is what lines the report up with the org's traffic.
 *
 * Last resort is the reference itself. `distinct_id` is required by the capture
 * API, and a report that could not be sent because neither id could be read
 * would be a report lost to a field nobody will ever query.
 */
async function distinctId(reference: string): Promise<string> {
  const id = analyticsId();
  if (id.kind === "id") return id.value;
  return installId().catch(() => reference);
}

/**
 * Upload one report. Resolves with the reference to show the user; throws with a
 * message the dialog classifies and offers Retry on.
 *
 * Nothing here catches: unlike `track`, a failure has somewhere to be shown, and
 * swallowing it would leave the user looking at a success state for a report that
 * never arrived.
 */
export async function sendDiagnosticReport(report: string): Promise<string> {
  if (!POSTHOG_KEY_VALUE) {
    throw new Error("This build has no diagnostics destination configured.");
  }
  const reference = newDiagnosticReference();
  const response = await fetch(`${POSTHOG_HOST}/i/v0/e/`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    signal: AbortSignal.timeout(SEND_TIMEOUT_MS),
    body: JSON.stringify({
      api_key: POSTHOG_KEY_VALUE,
      event: "diagnostic_report",
      distinct_id: await distinctId(reference),
      properties: {
        diagnostic_reference: reference,
        // The rendered report, verbatim - the same text the View report dialog
        // shows and the Copy report button copies. Sending a different, quieter
        // version would mean the screen that exists to say what leaves the
        // machine is describing something else.
        report,
        // Keep the anonymous posture the shared client is configured for
        // (`person_profiles: "identified_only"`): this event must not be what
        // finally creates a person profile for an install that never identified.
        $process_person_profile: false,
      },
    }),
  });
  if (!response.ok) {
    throw new Error(`The diagnostics service returned ${response.status}.`);
  }
  return reference;
}
