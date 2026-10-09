import type { UpstreamCoverage, Verdict, VerdictReason } from "./api";
import { governingMembers } from "./groups";
import type { Group, GroupMember } from "./groups";
import type { AppStatus, SidebarApp } from "../components/gc/Sidebar";

/**
 * Turning a backend routing verdict into the status line the design draws.
 *
 * Two vocabularies meet here and neither one was free to change:
 *
 * - **The design draws three phrases** - "Protected", "Not protected", "Not
 *   routed" (434:136) - and the Figma is the source of truth for copy.
 * - **AG-562 specifies three states** (On / Off / Needs attention) each carrying
 *   one reason from a closed set of five.
 *
 * Rather than pick a winner, the state maps onto the design's phrase and the
 * reason rides in `detail`, which the app pane draws as a card (`statusNote` in
 * `NewUiApp`). The reason strings are the ticket's own words, so nothing here
 * is invented. The remaining conflict - whether the coloured
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
export const WRITE_FAILED_DETAIL = "Configuration update failed";

/** The reason on a row that has no verdict yet. Not a fault, so the pane draws
 *  no card for it; see `statusNote` in `NewUiApp`. */
export const CHECKING_DETAIL = "Checking";

/** The reason behind a `not-protected` for each verdict reason: the ticket's own
 *  name for it, verbatim, except the two the design already had a phrase for. */
export const REASON_DETAIL: Record<VerdictReason, string> = {
  configuration_changed: "Config drifted",
  // The configuration landed; what has not happened is a process restart, so
  // the traffic is still on the old route.
  reopen_required: "Reopen to finish",
  // Deliberately not "Config drifted": the file Gate wrote is intact, and
  // sending someone to re-apply it would be sending them to fix the one thing
  // that is already right.
  configuration_overridden: "Configuration overridden",
  connection_problem: "Connection problem",
  access_problem: "Access problem",
  verification_failed: "Verification failed",
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
  // land. Callers pass it only for a failed turn-off: a tool whose turn-on
  // failed was never routed, and "Not protected" would claim it had been.
  if (opts.writeFailed) return { kind: "not-protected", detail: WRITE_FAILED_DETAIL };
  if (!verdict) return { kind: "not-protected", detail: CHECKING_DETAIL };
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
      const uninspected = uninspectedDetail(opts.coverage);
      if (uninspected) return { kind: "not-protected", detail: uninspected };
      return { kind: "protected" };
    }
    case "off":
      return { kind: "not-routed" };
    case "needs_attention":
      return {
        kind: "not-protected",
        detail: verdict.reason ? REASON_DETAIL[verdict.reason] : undefined,
      };
    case "not_installed":
      // Not shown in the sidebar at all - the ledger lists what could route
      // today - but a verdict for one must map to something rather than throw.
      return { kind: "not-protected" };
  }
}

/**
 * Why Gate is not looking at some of the hosts, or `undefined` when it is
 * looking at every one.
 *
 * Both halves of the coverage count. `unknown` is the irremediable one - no
 * catalog entry claims that host - and `switched_off` is a domain whose switch
 * is off, which AG-930's dialog offers to fix at the moment a tool is
 * connected. The row still has to say it, because the dialog fires once and
 * the switch can be flipped afterwards from somewhere else: removing and
 * re-trusting a certificate reset one to off hours after the fact, which is
 * how this was found.
 *
 * Every host, not one plus a count: the rail prints no reason at all now, and
 * the pane card that does has the room.
 */
function uninspectedDetail(
  coverage: UpstreamCoverage | null | undefined,
): string | undefined {
  if (!coverage) return undefined;
  // `flatMap`, because a switched-off entry is a catalog ROW and one row can
  // claim several hosts - #327 keyed these by slug for exactly that reason, so
  // a caller cannot name the same row twice or flip the same switch twice.
  // Naming hosts is right here: the pane already says the app, and the host
  // is the part the person recognises from their own config.
  //
  // Two sentences, not one list, because the two halves have different
  // remedies: a switched-off provider is one click away (`upstreamFix`), an
  // unknown one is not fixable from here at all. "Routed, not inspected" said
  // both the same way, in plumbing words.
  const off = coverage.switched_off.flatMap((entry) => entry.hosts);
  const sentences: string[] = [];
  if (off.length > 0) {
    const whose =
      coverage.switched_off.length === 1 ? "its provider is" : "their providers are";
    sentences.push(`Gate can’t see requests to ${off.join(", ")} while ${whose} turned off.`);
  }
  if (coverage.unknown.length > 0) {
    sentences.push(`Gate can’t inspect requests to ${coverage.unknown.join(", ")}.`);
  }
  return sentences.length > 0 ? sentences.join(" ") : undefined;
}

/**
 * The card's button for an uninspected app: the provider domains to turn on,
 * or `undefined` when none is switched off and there is nothing to click.
 *
 * `also` discloses the wider reach. A provider domain is not Hermes' own:
 * turning `anthropic` on inspects Claude Code's traffic too, and
 * `provider::reconcile_enabled` reads it as licence to connect Claude Code at
 * the next launch. The connect-time gate refuses to do that silently for
 * exactly this reason (`hermesProviderDomains`), so a button that does it has
 * to say so before the click.
 */
export function upstreamFix(
  coverage: UpstreamCoverage | null | undefined,
): { slugs: string[]; label: string; also?: string } | undefined {
  if (!coverage || coverage.switched_off.length === 0) return undefined;
  const one = coverage.switched_off.length === 1;
  const tools = [...new Set(coverage.switched_off.flatMap((entry) => entry.tools))];
  return {
    slugs: coverage.switched_off.map((entry) => entry.slug),
    label: one ? "Turn it on" : "Turn them on",
    also:
      tools.length > 0
        ? `Turning ${one ? "it" : "them"} on also routes ${tools.join(", ")}.`
        : undefined,
  };
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
    : m.desired
      ? { kind: "not-protected", detail: "Blocked" }
      : { kind: "not-routed" };
}

/**
 * The status line for a whole section, which is what a rail row is now.
 *
 * A section spans mechanisms - a config tool and two intercepted hosts under
 * one switch - so its line has to answer for all of them at once:
 *
 * 1. The first member that is not protected, in DRAW order. A section that says
 *    "Protected" while one of its surfaces is drifted is making the claim
 *    principle 6 forbids, so any member's reason outranks the count below it.
 * 2. Otherwise, routing if every member the switch governs is routing.
 * 3. Otherwise "Not protected" with "Partly protected: 2 of 3" as its reason if
 *    some are, which is the state an app switch makes reachable and a
 *    per-surface ledger never had to describe.
 * 4. Otherwise off, or blocked if the switch is on.
 *
 * Rule 1 is draw order and not a severity ladder: a member still being swept
 * reads `not-protected / "Checking"`, so drawn first it speaks over a sibling
 * that is genuinely drifted. `groupSummary` in `lib/groups.ts` has the explicit
 * ladder for callers that need one.
 *
 * Reads {@link governingMembers} for on/off and every member for exceptions: a
 * section is not "off" because its session surface is off, and a section that
 * has nothing but session surfaces is described by them, because otherwise it is
 * described by nothing and reports "Off" over traffic it is carrying.
 */
/**
 * The partly protected card's words, for the surfaces that are off.
 *
 * A function of its own because no section today has two off surfaces that
 * govern it, so the plural can only be reached from a test.
 */
export function partlyProtectedCopy(
  appName: string,
  off: string[],
): { title: string; body: string; label: string } {
  const one = off.length === 1;
  const names = one ? off[0] : `${off.slice(0, -1).join(", ")} and ${off[off.length - 1]}`;
  return {
    title: `${appName} isn’t fully protected`,
    body: `${names} ${one ? "isn’t" : "aren’t"} routed through Gate, so ${one ? "its" : "their"} traffic goes straight to the provider.`,
    // "Route", not "Turn on": beside an app's name, "Turn on" reads as
    // launching it. The body has just named the surface, so "it".
    label: one ? "Route it" : "Route them",
  };
}

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

  const exception = known.find((l) => l.status.kind === "not-protected");
  if (exception) return exception.status;

  const governed = governingMembers(group.members);
  const routing = governed.filter((m) => m.routed).length;
  if (governed.length > 0 && routing === governed.length) return { kind: "protected" };
  // Reachable only when no member's own line is already an exception - the find
  // above returns those - so in practice this is the mixed state where every
  // member is either routing or plainly off. The count is the useful half: it
  // tells the user how much of the app is covered, which "partly" alone does
  // not, and it is a reading rather than an adverb.
  if (routing > 0)
    return {
      kind: "not-protected",
      detail: `Partly protected: ${routing} of ${governed.length}`,
      partly: true,
    };
  return group.switchDesired > 0
    ? { kind: "not-protected", detail: "Blocked" }
    : { kind: "not-routed" };
}
