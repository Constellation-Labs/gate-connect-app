/**
 * How much of what the user asked for is actually routed, in one vocabulary
 * (AG-913).
 *
 * The topbar banner and the tray's routing card answer the same question, and
 * they used to answer it in different words and with different state models.
 * The banner said "Gate Connect is partly routing your apps" where the card
 * said "Partially routed"; worse, each drew a distinction the other lost. The
 * banner folded "you asked for three and none are routed" into "partly", which
 * is not partly. The card folded "you asked for nothing" into "Not protected",
 * which reports a fault the user caused on purpose - the exact claim the
 * banner's own `nothingRequested` branch was added to stop making.
 *
 * Four states, so neither loss survives. One module, so the two surfaces cannot
 * drift apart again: this is the same argument `lib/groups.ts` and
 * `lib/reopen.ts` make one level up, and AG-890 is the bug that argument is
 * about.
 *
 * **The STATE is judged on intent; the FRACTION is not.** `requested` is what
 * the user switched on, not what exists on the machine - see the note on
 * `desiredApps` in `NewUiApp`. A row nobody turned on is not a gap, and a
 * banner that can never go green is decoration rather than a status, so the
 * tone and the headline keep that denominator.
 *
 * The printed fraction does not. It reads "N of M tools on": apps switched
 * ON, over every app on the rail (2026-09-23).
 *
 * **It is coverage, not outcome, and it does NOT match the group eyebrow
 * counters.** Those read routed-over-group (`Tray.tsx`, `Sidebar.tsx`), which
 * is what `Components / Sidenav` draws. So the card and the eyebrows measure
 * two different things on one screen, and moving the denominator to
 * availability fixed only half of that: with two apps on and one routed, the
 * card reads "2 of 8 tools on" over groups reading "1 of 3" and "0 of 5" - the
 * denominators now add up and the numerators do not.
 *
 * That is deliberate rather than settled. The word "on" is the whole of what
 * marks the difference, which is thin, and whether the eyebrows should count
 * intent too is a design question rather than something to decide here - it
 * would deviate from the drawn counter. Raised with the deviation this file
 * already owes them.
 *
 * What the split buys: the fraction answers a question the headline does not -
 * how much of this machine is routed at all - rather than restating the
 * headline in digits, which is what routed-over-requested did.
 */
export type RoutingStateKind =
  /** Everything asked for is routed. */
  | "routed"
  /** Some of it is. */
  | "partly"
  /** None of it is, but something was asked for. */
  | "none-routed"
  /** Nothing was asked for, so there is nothing to report a gap about. */
  | "none-requested";

export interface RoutingState {
  kind: RoutingStateKind;
  /**
   * The short phrase: the banner's pill, and the card's heading.
   *
   * "Routed", not "Routing" - every routed frame on Flows/Overview reads
   * `Routed · 4 of 4 Apps` (re-read 2026-08-21).
   */
  label: string;
  /**
   * The sentence, for a surface with room for one.
   *
   * Says "Gate", not "Gate Connect". The tray's routing card is 360px wide and
   * the longest of these has to fit beside a 36px tile; the card's own frame
   * already writes "Gate is protecting you". Taking the narrow surface's
   * product name is what lets both draw one sentence, which is the whole point
   * of this module.
   */
  headline: string;
  tone: "green" | "amber";
  icon: "shieldCheck" | "shieldBan";
}

const STATES: Record<RoutingStateKind, Omit<RoutingState, "kind">> = {
  routed: {
    label: "Routed",
    headline: "Gate is protecting you",
    tone: "green",
    icon: "shieldCheck",
  },
  partly: {
    label: "Partly routed",
    headline: "Gate is partly routing your apps",
    tone: "amber",
    icon: "shieldBan",
  },
  "none-routed": {
    label: "Not protected",
    headline: "Gate is not routing your apps",
    tone: "amber",
    icon: "shieldBan",
  },
  "none-requested": {
    label: "None routed",
    // No fault is claimed: the user switched everything off, which is an
    // answer rather than a gap. A fraction DOES go beside it now - "0 of 8
    // Apps on" - since the denominator became availability and stopped being
    // the meaningless half of "0 of 0" (2026-09-23).
    headline: "No apps are set to route",
    tone: "amber",
    icon: "shieldBan",
  },
};

/**
 * Which state a pair of counts is in.
 *
 * Amber for all three unhappy states because there is no third tone drawn and
 * picking one by eye is the thing this repo is told not to do - see question 23
 * in `docs/figma-questions-for-design.md`. The words are the part that was
 * making a false claim, so the words are the part that changed.
 */
export function routingState(routed: number, requested: number): RoutingState {
  const kind: RoutingStateKind =
    requested <= 0
      ? "none-requested"
      : routed >= requested
        ? "routed"
        : routed > 0
          ? "partly"
          : "none-routed";
  return { kind, ...STATES[kind] };
}

/**
 * Whether a fraction belongs beside the state, given how many apps the rail is
 * showing.
 *
 * The fraction counts **apps switched on, out of apps available** - not routed
 * out of requested, which is what the headline and tone above answer. The two
 * were the same ratio until 2026-09-23 and disagreed with the group eyebrow
 * counters beside them, which have always divided by the whole group: a card
 * reading "2 of 2" sat above groups reading "1 of 3" and "1 of 5", the same
 * word "of" over two different populations.
 *
 * So this now takes the count the denominator is drawn from rather than the
 * state. A denominator of nothing is still suppressed - "0 of 0" is a ratio
 * with both halves meaningless - but "0 of 8" is not, and it used to be hidden
 * along with it, which is why a rail full of apps could report no fraction at
 * all.
 */
export function showsFraction(available: number): boolean {
  return available > 0;
}
