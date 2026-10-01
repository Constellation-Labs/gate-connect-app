import { describe, expect, it } from "vitest";
import type { Account } from "./api";
import type { Installation } from "./activity";
import { gatewaySawTrafficFromThisMachine, launchProps, sessionFacts } from "./analyticsSession";

const base: Account = {
  gateway_base_url: "https://gateway.example",
  has_api_key: true,
  auth_mode: "api_key",
  org_id: null,
  org_name: null,
} as unknown as Account;

describe("sessionFacts", () => {
  /** Round 5, L3: pasting a key over a Constellation sign-in leaves the old
   *  sign-in's org in `account.json`. That org is not the key's, and every
   *  milestone is grouped by whatever org is reported. */
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

const row = (over: Partial<Installation>): Installation => ({
  installId: "other",
  label: "other",
  current: false,
  lastSeenAt: "2026-09-30T00:00:00Z",
  requests: 3,
  ...over,
});

describe("gatewaySawTrafficFromThisMachine (review item 8)", () => {
  /** The response the reviewer asked about: the gateway echoes this read's own
   *  install id as the top-level `current`, and no row is this machine. */
  it("is false when only the echoed top-level id names this machine", () => {
    const raw = { current: "this-install", installations: [row({})] };
    expect(raw.current).not.toBeNull();
    expect(gatewaySawTrafficFromThisMachine(raw.installations)).toBe(false);
    expect(gatewaySawTrafficFromThisMachine([])).toBe(false);
  });

  it("is true for a row the gateway marks current", () => {
    expect(
      gatewaySawTrafficFromThisMachine([row({}), row({ installId: "me", current: true })]),
    ).toBe(true);
  });

  it("is false for a current row that says it saw no requests", () => {
    expect(gatewaySawTrafficFromThisMachine([row({ current: true, requests: 0 })])).toBe(false);
  });
});

describe("launchProps (review item 9)", () => {
  it("omits has_account when the account read failed", () => {
    const props = launchProps({ account: null, unread: true }, null);
    expect(props).not.toHaveProperty("has_account");
    expect(props).toEqual({ proxy_available: false, routing_on: false });
  });

  it("reports has_account when the read answered", () => {
    expect(launchProps({ account: null, unread: false }, { running: true })).toEqual({
      has_account: false,
      proxy_available: true,
      routing_on: true,
    });
    expect(launchProps({ account: base, unread: false }, null).has_account).toBe(true);
  });
});
