import type { UpstreamCoverage, Verdict, VerdictNextAction, VerdictReason } from "./api";
import { governingMembers } from "./groups";
import type { Group, GroupMember } from "./groups";
import type { AppStatus, SidebarApp } from "../components/gc/Sidebar";

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
 * Two of `VerdictReason`'s five are absent on purpose, because each maps to a
 * coloured phrase of its own and repeating it as a suffix would print the same
 * fact twice: `configuration_changed` is the design's "Config drifted", and
 * `reopen_required` is "Reopen to finish". */
const REASON_SUFFIX: Record<
  Exclude<VerdictReason, "configuration_changed" | "reopen_required">,
  string
> = {
  // Deliberately not "Config drifted": the file Gate wrote is intact, and
  // sending someone to re-apply it would be sending them to fix the one thing
  // that is already right.
  configuration_overridden: "Configuration overridden",
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
  opts: { writeFailed?: boolean; coverage?: UpstreamCoverage | null } = {},
): AppStatus {
  // Outranks the sweep. The sweep describes the state on disk, which after a
  // failed write is the state from *before* the user acted - true, and not the
  // thing they need to know. What they need to know is that their click did not
  // land.
  if (opts.writeFailed) return { kind: "not-protected", detail: WRITE_FAILED_DETAIL };
  if (!verdict) return { kind: "not-protected", detail: "Checking" };
  switch (verdict.state) {
    case "on": {
      // Routed, and Gate can see it - unless the provider it routes TO is one
      // Gate does not intercept, which is a question the sweep never asks.
      // The verdict answers "is this tool pointed at Gate"; coverage answers
      // "and does Gate look at where it is pointed". Both must be yes before
      // a row may say Protected. AG-932.
      //
      // Only this arm. Every other state is already amber and already more
      // urgent than this: a drifted or unrouted tool has a bigger problem
      // than an uninspected provider, and stacking the two would bury it.
      const uninspected = uninspectedHosts(opts.coverage);
      if (uninspected) return { kind: "not-inspected", detail: uninspected };
      return { kind: "protected" };
    }
    case "off":
      // The design draws this one with its suffix already: "Not routed - Off".
      return { kind: "not-routed", detail: "Off" };
    case "needs_attention":
      if (verdict.reason === "configuration_changed") return { kind: "drifted" };
      // Its own phrase rather than an amber negative. The configuration landed;
      // what has not happened is a process restart, and this is the one
      // `needs_attention` reason where nothing has gone wrong at all.
      if (verdict.reason === "reopen_required") return { kind: "reopen" };
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

/**
 * The hosts Gate is not looking at, as the short phrase a rail row has room
 * for, or `undefined` when it is looking at all of them.
 *
 * Both halves of the coverage count. `unknown` is the irremediable one - no
 * catalog entry claims that host - and `switched_off` is a domain whose switch
 * is off, which AG-930's dialog offers to fix at the moment a tool is
 * connected. The row still has to say it, because the dialog fires once and
 * the switch can be flipped afterwards from somewhere else: removing and
 * re-trusting a certificate reset one to off hours after the fact, which is
 * how this was found.
 *
 * One host plus a count. The rail is 250px and a list truncates mid-word; the
 * app pane has the room to name them all if it ever needs to.
 */
function uninspectedHosts(
  coverage: UpstreamCoverage | null | undefined,
): string | undefined {
  if (!coverage) return undefined;
  // `flatMap`, because a switched-off entry is a catalog ROW and one row can
  // claim several hosts - #327 keyed these by slug for exactly that reason, so
  // a caller cannot name the same row twice or flip the same switch twice.
  // Naming hosts is still right here: the row already says the app, and the
  // host is the part the person recognises from their own config.
  const hosts = [
    ...coverage.switched_off.flatMap((entry) => entry.hosts),
    ...coverage.unknown,
  ];
  if (hosts.length === 0) return undefined;
  return hosts.length === 1 ? hosts[0] : `${hosts[0]} +${hosts.length - 1}`;
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

/**
 * The status line for a whole section, which is what a rail row is now.
 *
 * A section spans mechanisms - a config tool and two intercepted hosts under
 * one switch - so its line has to answer for all of them at once:
 *
 * 1. The first member with a line that is not plain routing, in DRAW order. A
 *    section that says "Protected" while one of its surfaces is drifted is
 *    making the claim principle 6 forbids, so anything drifted, not-protected
 *    or mid-reopen outranks the count below it.
 * 2. Otherwise, routing if every member the switch governs is routing.
 * 3. Otherwise "Partly protected - 2 of 3" if some are, which is the state an
 *    app switch makes reachable and a per-surface ledger never had to describe.
 * 4. Otherwise off.
 *
 * A mid-reopen member joining rule 1 is what makes the common case legible: a
 * group switch writes one config and enables two hosts, the hosts route at once
 * and the tool waits on a restart, so the honest line is the one naming the
 * restart rather than the count. It outranks rule 3 by sitting in rule 1 at all,
 * which is also why this is not a severity ladder - see below.
 *
 * Rule 1 is draw order and not a severity ladder, which the doc here used to
 * claim: a member still being swept reads `not-protected / "Checking"`, so drawn
 * first it speaks over a sibling that is genuinely drifted. `groupSummary` in
 * `lib/groups.ts` has the explicit ladder for callers that need one. Rule 1 also
 * never sees an untrusted certificate on a proxy member, which
 * `proxyMemberStatus` reports as `not-routed / "Blocked"` rather than as an
 * exception - that is a row nobody has managed to route yet, not a row that has
 * gone wrong.
 *
 * Reads {@link governingMembers} for on/off and every member for exceptions: a
 * section is not "off" because its session surface is off, and a section that
 * has nothing but session surfaces is described by them, because otherwise it is
 * described by nothing and reports "Off" over traffic it is carrying.
 */
export function sectionStatus(
  group: Group,
  statusBySlug: Map<string, SidebarApp>,
): AppStatus | null {
  if (group.members.length === 0) return null;
  // A config member's line is the sweep's, which the rail already computed; a
  // proxy member's is its own state. Taking the tool's from the map keeps the
  // section and the tool it contains from ever disagreeing. Named for what it
  // is rather than `appFor`, which is a lookup FUNCTION in both shells.
  const lines = group.members.map((m) => ({
    member: m,
    status: m.kind === "config" ? statusBySlug.get(m.key)?.status : proxyMemberStatus(m),
  }));
  const known = lines.filter(
    (l): l is { member: GroupMember; status: AppStatus } => l.status !== undefined,
  );
  if (known.length === 0) return null;

  const exception = known.find(
    (l) =>
      l.status.kind === "drifted" ||
      l.status.kind === "not-protected" ||
      l.status.kind === "reopen",
  );
  if (exception) {
    // No suffix on a reopen, on any section.
    //
    // This used to name the member - "Reopen to finish - CLI" - on the
    // reasoning that a section's heading is the app and a reopen is one
    // program inside it. `Tool.name` is a surface label, so what the rail
    // actually drew was the row's state and the row's TYPE in one line, which
    // design rejected on 2026-09-21: "we're mixing CLI and On/Off ... the type
    // cannot live in the same label."
    //
    // Dropping the suffix rather than swapping it for the product name, which
    // was the first attempt here and the wrong one. The instruction was to
    // drop the type, and the product name is no better in the slot: on the
    // one-member sections it repeats the row's own heading, which is exactly
    // why the old code special-cased them, and on the multi-member ones the
    // remedy is the same whichever member it is - reopen the app this row
    // names. The pane still says which program, where there is room.
    return exception.status;
  }

  const governed = governingMembers(group.members);
  const routing = governed.filter((m) => m.routed).length;
  if (governed.length > 0 && routing === governed.length) return { kind: "protected" };
  // Reachable only when no member's own line is already an exception - the find
  // above returns those - so in practice this is the mixed state where every
  // member is either routing or plainly off. The count is the useful half: it
  // tells the user how much of the app is covered, which "partly" alone does
  // not, and it is a reading rather than an adverb.
  if (routing > 0)
    return { kind: "partly-protected", detail: `${routing} of ${governed.length}` };
  return { kind: "not-routed", detail: group.switchDesired > 0 ? "Blocked" : "Off" };
}
