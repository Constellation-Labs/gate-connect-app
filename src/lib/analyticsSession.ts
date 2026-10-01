import type { Account, AccountReading, OAuthStatus } from "./api";
import type { Installation } from "./activity";
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

/**
 * Whether the gateway reports proxied traffic from THIS machine: the
 * `first_request_proxied { source: "gateway" }` signal where the relay cannot
 * report (Linux).
 *
 * Read from the `installations` rows, never from the response's top-level
 * `current`. In the gateway (gate repo,
 * `apps/gateway-proxy/src/activity/activity.controller.ts`, the installations
 * route) the top-level `current` is only the `X-Gate-Install-Id` header of the
 * read itself echoed back, so it is set on the first Overview load after
 * sign-in, before anything has been proxied. The rows come from
 * `gateway_requests`, proxied traffic, and a row is `current: true` only when
 * it is this install and it has traffic in the window. A row that also says how
 * many requests it saw must say at least one.
 */
export function gatewaySawTrafficFromThisMachine(installations: readonly Installation[]): boolean {
  return installations.some(
    (row) => row.current === true && (typeof row.requests !== "number" || row.requests > 0),
  );
}

/**
 * `app_launched`'s props, from the launch's first reads. `has_account` is
 * left out when the account read REJECTED (the account file or the secret store
 * could not be read): that launch did not learn whether there is an account,
 * and reporting `false` would count a keychain error as a launch with none.
 */
export function launchProps(
  account: AccountReading,
  proxy: { running: boolean } | null,
): Record<string, boolean> {
  return {
    ...(account.unread ? {} : { has_account: account.account !== null }),
    proxy_available: proxy !== null,
    routing_on: proxy?.running ?? false,
  };
}
