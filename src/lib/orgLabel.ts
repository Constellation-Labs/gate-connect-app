import type { Account } from "./api";

/**
 * What to call the signed-in organization when nothing names it.
 *
 * Deliberately blunt. Every user belongs to an org, so the org always loads in
 * practice and this string is the "we have not been told yet" state rather than
 * a real condition. That is also why it must not be replaced by something
 * cleverer: the setup pane used to fall back to the gateway URL, which reads as
 * an answer and is not one.
 */
export const NO_ORG = "No organization";

/**
 * One name for the organization, for every surface that shows it.
 *
 * Three surfaces resolved this independently and gave three different answers
 * from the same state. `Account.org_name` is populated in OAuth mode only - it
 * is the org the user *picked* - so on an api-key account it is always null and
 * each chain fell through to its own idea of a fallback:
 *
 * - the window's rail reached for the activity reading's `orgName` and showed
 *   the real name;
 * - the tray had no second source and announced "No organization" in its
 *   footer, on the one line a user would check to see which org their traffic
 *   bills to;
 * - the setup pane fell back to the gateway URL, then to "Gate".
 *
 * The fix is one chain, not three. `reading` is whatever name the caller can
 * see - the window's live overview, the tray's cached one - and the two are the
 * same figure from the same endpoint, so agreeing on the order is what makes
 * the surfaces agree on the answer.
 */
export function orgLabel(
  account: Pick<Account, "org_name"> | null | undefined,
  reading?: string | null,
): string {
  return account?.org_name || reading || NO_ORG;
}
