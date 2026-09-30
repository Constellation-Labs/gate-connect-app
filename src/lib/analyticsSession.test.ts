import { describe, expect, it } from "vitest";
import type { Account } from "./api";
import { sessionFacts } from "./analyticsSession";

const base: Account = {
  gateway_base_url: "https://gateway.example",
  has_api_key: true,
  auth_mode: "api_key",
  org_id: null,
  org_name: null,
} as unknown as Account;

describe("sessionFacts", () => {
  /** Round 5, L3: pasting a key over a Constellation sign-in leaves the old
   *  sign-in's org in `account.json`. That org is not the key's, and the core
   *  spends the install id on whatever org is reported. */
  it("reports only the gateway's org for an API key, never a stale account org", () => {
    const stale = { ...base, org_id: "org-from-an-old-sign-in" };
    expect(sessionFacts({ account: stale, accountUnread: false, oauth: null }).orgId).toBeNull();
    expect(
      sessionFacts({ account: stale, accountUnread: false, oauth: null, apiKeyOrgId: "org-key" })
        .orgId,
    ).toBe("org-key");
  });

  it("reports the account's own org for OAuth", () => {
    const oauthAccount = { ...base, auth_mode: "oauth", has_api_key: false, org_id: "org-a" } as unknown as Account;
    expect(
      sessionFacts({ account: oauthAccount, accountUnread: false, oauth: null, apiKeyOrgId: "x" })
        .orgId,
    ).toBe("org-a");
  });
});
