import type { Account, OAuthStatus } from "./api";
import type { SessionFacts } from "./analytics";
import { isSignedIn } from "./session";

/**
 * What the sign-in window tells the analytics seam about the session, from
 * what it has read (AG-960). One function for both shells, so the rule for
 * "unknown" cannot drift between them.
 *
 * `sessionUnknown` is the rule that matters. Nothing the seam does on a
 * sign-out may follow from a read that could not answer:
 * - an account read that REJECTED (`readAccount`'s `unread`: the account file
 *   or the secret store could not be read), which the screens draw as signed
 *   out but which says nothing about whether anyone is;
 * - for OAuth, a status read that failed (`oauth === null`) or a session whose
 *   state the identity provider or the secret store could not give
 *   (`session === "unavailable"`).
 *
 * The org is the account's own for OAuth. For an API key it is ONLY the org the
 * gateway resolved the key to (`apiKeyOrgId`, from the Overview's reading),
 * which is also the first moment the key was accepted: never the account
 * file's `org_id`, which a pasted key leaves behind from an earlier sign-in.
 */
export function sessionFacts({
  account,
  accountUnread,
  oauth,
  apiKeyOrgId = null,
}: {
  account: Account | null;
  accountUnread: boolean;
  oauth: OAuthStatus | null;
  apiKeyOrgId?: string | null;
}): SessionFacts {
  const authMode = account?.auth_mode ?? null;
  return {
    signedIn: isSignedIn(account, oauth),
    sessionUnknown:
      accountUnread || (authMode === "oauth" && (oauth === null || oauth.session === "unavailable")),
    authMode,
    sub: oauth?.sub ?? null,
    // An API key's org is the gateway's answer and nothing else. `org_id` in
    // the account file belongs to an OAuth sign-in, and a pasted key keeps the
    // file's old one, so reading it here would report a stale org as this key's.
    orgId: authMode === "api_key" ? apiKeyOrgId : (account?.org_id ?? null),
  };
}
