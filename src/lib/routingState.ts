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
   * Says "Gate Connect" since the 2026-09-29 redraw: every window banner
   * (`1390:14024`, `1374:7567`, `1401:16730`, `1404:18354`) writes the full
   * product name. This used to say "Gate" so the tray's 360px routing card
   * could draw the same sentence; the longest of these is 269px at 14px
   * Medium, which still fits beside that card's 36px tile.
   */
  headline: string;
  /** Grey is the drawn none state (`1390:14024`, a gray/50-to-200 tile with a
   *  `CircleOff` glyph), and the label beside it is muted rather than amber. */
  tone: "green" | "amber" | "grey";
  icon: "shieldCheck" | "shieldBan" | "circleOff";
}

const STATES: Record<RoutingStateKind, Omit<RoutingState, "kind">> = {
  routed: {
    label: "Routed",
    headline: "Gate Connect is protecting you",
    tone: "green",
    icon: "shieldCheck",
  },
  partly: {
    label: "Partly routed",
    headline: "Gate Connect is partly routing your apps",
    tone: "amber",
    icon: "shieldBan",
  },
  "none-routed": {
    label: "Not protected",
    headline: "Gate Connect is not routing your apps",
    tone: "amber",
    icon: "shieldBan",
  },
  "none-requested": {
    label: "None routed",
    // No fault is claimed: the user switched everything off, which is an
    // answer rather than a gap. A fraction DOES go beside it now - "0 of 8
    // Apps on" - since the denominator became availability and stopped being
    // the meaningless half of "0 of 0" (2026-09-23).
    headline: "No apps are routed",
    tone: "grey",
    icon: "circleOff",
  },
};

/**
 * Which state a pair of counts is in.
 *
 * Grey for "nothing switched on" and amber for the two failures. The grey is
 * drawn (`Overview/none-routed` 1390:13599 and `App/not-routing` 1340:21966,
 * 2026-09-29), which answers the old question 23: before it every unhappy
 * state was amber because no third tone existed. Whether that grey banner is
 * also meant for `none-routed` - switched on, and routing did not start - is
 * open (`plans/new-app-ui-figma.md`, design sync 2026-09-30, question 1), so that
 * state keeps its amber and its own sentence.
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
