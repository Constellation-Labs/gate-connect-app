import type { Verdict, VerdictNextAction, VerdictReason } from "./api";
import type { GroupMember } from "./groups";
import type { AppStatus } from "../components/gc/Sidebar";

/**
 * Turning a backend routing verdict into the status line the design draws.
 *
 * Two vocabularies meet here and neither one was free to change:
 *
 * - **The design draws four phrases** - "Protected", "Not protected", "Config
 *   drifted", "Not routed" - and the Figma is the source of truth for copy.
 * - **AG-562 specifies three states** (On / Off / Needs attention) each carrying
 *   one reason from a closed set of five.
 *
 * Rather than pick a winner, the state maps onto the design's phrase and the
 * reason rides in the grey suffix the design already has a slot for ("Protected
 * - 2m ago", "Not routed - Off"). The reason strings are the ticket's own words,
 * so nothing here is invented. The remaining conflict - whether the coloured
 * phrase should read "On" or "Protected" - is a copy decision for the designer,
 * raised on AG-561/562 rather than settled in this file.
 */

/**
 * The status line for a tool whose last config write failed.
 *
 * Deliberately *not* one of `VerdictReason`'s five. Those are derived from
 * evidence - a config on disk, a relay that answers, a process older than the
 * last change - and a failed write leaves none: nothing was written, so there is
 * nothing for the sweep to find. The frontend remembers it for the session
 * instead (`useRouting`'s `writeFailures`), which is why it arrives as a separate
 * argument rather than as a sixth reason.
 *
 * AG-564 and AG-568 both name this state; AG-562's list of five does not include
 * it. Raised on those tickets rather than smuggled into the enum.
 */
const WRITE_FAILED_DETAIL = "Configuration update failed";

/** The grey suffix for a reason: the ticket's own name for it, verbatim.
 *
 * `configuration_changed` is absent on purpose - it maps to the design's own
 * "Config drifted" phrase, so repeating it as a suffix would print the same
 * fact twice. */
const REASON_SUFFIX: Record<Exclude<VerdictReason, "configuration_changed">, string> = {
  // Deliberately not "Config drifted": the file Gate wrote is intact, and
  // sending someone to re-apply it would be sending them to fix the one thing
  // that is already right.
  configuration_overridden: "Configuration overridden",
  reopen_required: "Reopen required",
  connection_problem: "Connection problem",
  access_problem: "Access problem",
  verification_failed: "Verification failed",
};

/** Button label for the one action a reason offers. Straight from AG-562's list
 * ("Reopen tool, Apply Gate configuration, Retry check, Sign in, Reconnect"), so
 * the control and the ticket say the same thing. */
export const NEXT_ACTION_LABEL: Record<VerdictNextAction, string> = {
  apply_gate_configuration: "Apply Gate configuration",
  show_conflicting_config: "Show conflicting file",
  reopen_tool: "Reopen tool",
  reconnect: "Reconnect",
  sign_in: "Sign in",
  retry_check: "Retry check",
};

/**
 * The status line for one app.
 *
 * `undefined` means the sweep has not answered yet, and it deliberately does
 * **not** fall back to the config-derived line. Reading "Protected" off a config
 * file is the exact claim AG-562 rules out ("a switch or saved configuration
 * does not produce On"), so an unanswered row says it is still checking instead.
 * That costs a moment of amber on load, which is the honest trade.
 */
export function verdictStatus(
  verdict: Verdict | undefined,
  opts: { writeFailed?: boolean } = {},
): AppStatus {
  // Outranks the sweep. The sweep describes the state on disk, which after a
  // failed write is the state from *before* the user acted - true, and not the
  // thing they need to know. What they need to know is that their click did not
  // land.
  if (opts.writeFailed) return { kind: "not-protected", detail: WRITE_FAILED_DETAIL };
  if (!verdict) return { kind: "not-protected", detail: "Checking" };
  switch (verdict.state) {
    case "on":
      return { kind: "protected" };
    case "off":
      // The design draws this one with its suffix already: "Not routed - Off".
      return { kind: "not-routed", detail: "Off" };
    case "needs_attention":
      if (verdict.reason === "configuration_changed") return { kind: "drifted" };
      return {
        kind: "not-protected",
        detail: verdict.reason ? REASON_SUFFIX[verdict.reason] : undefined,
      };
    case "not_installed":
      // Not shown in the sidebar at all - the ledger lists what could route
      // today - but a verdict for one must map to something rather than throw.
      return { kind: "not-protected" };
  }
}

/** Index a sweep by slug, so a row can look itself up. */
export function verdictsBySlug(verdicts: Verdict[]): Map<string, Verdict> {
  return new Map(verdicts.map((v) => [v.slug, v]));
}

/**
 * The status line for a proxy-routed member, shared by the window shell's rail,
 * the tray popover and the family panel so no two surfaces can phrase one
 * domain two ways. No verdict exists for these - the sweep is per tool - so
 * observation is the domain's own state: carrying traffic, switched on but
 * blocked (master off, certificate untrusted), or off.
 */
export function proxyMemberStatus(m: GroupMember): AppStatus {
  return m.routed
    ? { kind: "protected" }
    : { kind: "not-routed", detail: m.desired ? "Blocked" : "Off" };
}
