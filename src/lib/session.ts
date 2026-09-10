import type { Account, OAuthStatus } from "./api";

/**
 * What counts as signed in, shared by both shells.
 *
 * Extracted from `App.tsx` rather than reimplemented in `NewUiApp`: if the two
 * disagree, one surface shows the app while the other shows sign-in against the
 * same account, and which one is right is not obvious from either. The rule is
 * subtle enough to be worth exactly one copy - an OAuth account needs a live
 * session *and* a picked org, because the gateway rejects requests without one.
 */
export function isSignedIn(account: Account | null, oauth: OAuthStatus | null): boolean {
  if (!account) return false;
  if (account.auth_mode === "oauth") return (oauth?.signed_in ?? false) && !!account.org_id;
  return account.has_api_key;
}

/** An OAuth session that's authenticated but hasn't picked an org yet - the
 *  one state that routes to the org picker rather than sign-in or home. */
export function needsOrg(account: Account | null, oauth: OAuthStatus | null): boolean {
  return account?.auth_mode === "oauth" && (oauth?.signed_in ?? false) && !account.org_id;
}
