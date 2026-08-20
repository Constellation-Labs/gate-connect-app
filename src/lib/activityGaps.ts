import type { ActivityFailure, UnavailableReason } from "./activity";

/**
 * What the Overview says when a reading is missing, and what it offers to do
 * about it (AG-576).
 *
 * The ticket's requirement is narrow and worth restating: a metric that cannot
 * be shown must **name its cause and offer a matching action**, and must never
 * render a zero, because a zero is a real reading that says "no traffic".
 *
 * Two vocabularies feed in and they are not the same thing:
 *
 * - [`ActivityFailure`] is the client's account of never getting an answer.
 * - `UnavailableReason` is the gateway's account of a section it answered but
 *   could not fill - a role that may not see security events, a policy set
 *   nobody has configured.
 *
 * Neither has anything to do with whether routing is switched on. That
 * conflation is the one AG-576 names: sending a user to check their routing
 * switch because the gateway is unreachable is sending them to fix something
 * that was never broken.
 *
 * **Only actions that do real work appear here.** `lib/notices.ts` argues the
 * same point for routing notices: an offered action the user cannot complete is
 * worse than none, because it converts "I cannot see this" into "I tried to fix
 * this and it did nothing". So there is no "Sign in": `useSetup` owns that flow
 * and the shell only routes to it when there is no usable credential at all,
 * which is not a state this banner is ever drawn in. Those causes point at the
 * dashboard, which does work.
 *
 * "Switch organization" is the near miss. The sidebar's switcher is wired now,
 * so the action would work - but no reason maps to it cleanly: `no_org` is the
 * candidate, and an account with no organization has nothing to switch between.
 * Offering it belongs with the rest of AG-576's action vocabulary rather than
 * being smuggled in here.
 */

export type GapActionKind =
  | "retry"
  | "diagnostics"
  | "dashboard"
  | "api-keys"
  | "docs";

export interface GapAction {
  kind: GapActionKind;
  label: string;
}

export interface GapNotice {
  /** What is missing. A metric name, or the whole reading. */
  subject: string;
  /** Why, in one sentence, in the user's terms rather than the gateway's. */
  cause: string;
  /** Most likely to help first. May be empty when nothing the user can reach
   *  would change the outcome, which is itself honest. */
  actions: GapAction[];
}

const RETRY: GapAction = { kind: "retry", label: "Try again" };
const DIAGNOSTICS: GapAction = { kind: "diagnostics", label: "View diagnostics" };
const DASHBOARD: GapAction = { kind: "dashboard", label: "Visit dashboard" };
const API_KEYS: GapAction = { kind: "api-keys", label: "Manage API keys" };
const DOCS: GapAction = { kind: "docs", label: "Read Gate docs" };

/** The whole reading is missing, because the fetch never landed. */
export function failureNotice(failure: ActivityFailure): GapNotice {
  switch (failure.code) {
    case "offline":
      return {
        subject: "Activity",
        // Says which end is silent. "Check your connection" would be a guess:
        // the gateway itself can be down while the user's network is fine.
        cause: "Gate Connect could not reach the gateway from this machine.",
        actions: [RETRY, DIAGNOSTICS],
      };
    case "signed_out":
      return {
        subject: "Activity",
        cause: "There is no live session to read your activity with.",
        actions: [RETRY, DASHBOARD],
      };
    case "no_org":
      return {
        subject: "Activity",
        cause: "No organization is selected, so there is nothing to report on.",
        actions: [DASHBOARD],
      };
    case "rejected":
      return {
        subject: "Activity",
        cause: "The gateway refused this credential.",
        actions: [API_KEYS, DOCS],
      };
    case "gateway":
      return {
        subject: "Activity",
        cause: "The gateway answered, but could not produce a reading.",
        actions: [RETRY, DIAGNOSTICS],
      };
    default:
      return {
        subject: "Activity",
        cause: "Gate Connect could not read your activity.",
        actions: [RETRY, DIAGNOSTICS],
      };
  }
}

/** One section of an otherwise good reading is missing. */
export function sectionNotice(section: string, reason: UnavailableReason): GapNotice {
  switch (reason) {
    case "connectivity":
      return {
        subject: section,
        cause: "This part of the reading could not be fetched.",
        actions: [RETRY],
      };
    case "access":
      // No action on purpose. A role is granted by somebody else, so every
      // button here would be a dead end dressed up as a remedy.
      //
      // Nothing raises this today: the gateway declines a section for a role it
      // cannot resolve *at all*, which is `attribution` below, and every caller
      // it can resolve gets at least their own slice. Kept because the reason is
      // part of AG-576's published taxonomy and the first section that is
      // genuinely role-gated will send it; unhandled, it would fall through to a
      // cause that blames the wrong thing.
      return {
        subject: section,
        cause: "Your role in this organization cannot see this. An owner or admin can.",
        actions: [],
      };
    case "attribution":
      // Not phrased as a permission problem, because it is not one and an admin
      // cannot grant anything that would fix it. The remedy is a credential that
      // belongs to a person: signing in, or a key issued to this account rather
      // than to a machine.
      return {
        subject: section,
        cause: "The credential in use has no user attached, so Gate cannot tell whose activity this is.",
        actions: [API_KEYS, DOCS],
      };
    case "not_configured":
      return {
        subject: section,
        cause: "Nothing is set up for this yet, so there is nothing to report.",
        actions: [DASHBOARD],
      };
    case "definition_pending":
      return {
        subject: section,
        cause: "This measure is not defined yet.",
        actions: [],
      };
  }
}
