import React from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Mock } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import type { Account, OAuthStatus, Org } from "./api";
import { useSetup } from "./useSetup";
import { DEFAULT_GATEWAY_BASE_URL } from "./config";

vi.mock("./api", () => ({
  getAccount: vi.fn(),
  oauthBeginLogin: vi.fn(),
  oauthListOrgs: vi.fn(),
  oauthStatus: vi.fn(),
  proxyEnable: vi.fn(),
  saveAccount: vi.fn(),
  setOrg: vi.fn(),
}));
vi.mock("./analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));
vi.mock("./oauthOffer", () => ({ markOAuthOfferSeen: vi.fn() }));

import {
  getAccount,
  oauthBeginLogin,
  oauthListOrgs,
  oauthStatus,
  proxyEnable,
  saveAccount,
  setOrg,
} from "./api";
import { markOAuthOfferSeen } from "./oauthOffer";

const SIGNED_OUT: OAuthStatus = { signed_in: false, email: null, expires_at_unix: 0 };
const SIGNED_IN: OAuthStatus = {
  signed_in: true,
  email: "dev@example.com",
  expires_at_unix: 4102444800,
};

const oauthAccount = (over: Partial<Account> = {}): Account => ({
  gateway_base_url: "https://gw.example",
  has_api_key: false,
  auth_mode: "oauth",
  org_id: "org-1",
  org_name: "Example Org",
  ...over,
});

const keyAccount = (over: Partial<Account> = {}): Account => ({
  gateway_base_url: "https://gw.example",
  has_api_key: true,
  auth_mode: "api_key",
  org_id: null,
  org_name: null,
  ...over,
});

const ORGS: Org[] = [
  { orgId: "org-1", name: "Example Org", slug: "example", role: "admin" },
  { orgId: "org-2", name: "Other Org", slug: "other", role: "member" },
];

/** Drives the hook, re-rendering with whatever session the callbacks report -
 *  the container's job, and what makes the derived stage observable. */
function harness(initial: {
  loaded?: boolean;
  account?: Account | null;
  oauth?: OAuthStatus | null;
  proxyRunning?: boolean;
}) {
  const api: { current: ReturnType<typeof useSetup> | null } = { current: null };
  const onProxy = vi.fn();

  function Probe() {
    const [session, setSession] = React.useState<{
      account: Account | null;
      oauth: OAuthStatus | null;
    }>({ account: initial.account ?? null, oauth: initial.oauth ?? null });

    api.current = useSetup({
      loaded: initial.loaded ?? true,
      account: session.account,
      oauth: session.oauth,
      onSession: setSession,
      onProxy,
    });
    return null;
  }
  render(<Probe />);
  return { api, onProxy };
}


beforeEach(() => {
  vi.clearAllMocks();
  (saveAccount as Mock).mockResolvedValue(undefined);
  (oauthBeginLogin as Mock).mockResolvedValue(SIGNED_IN);
  (oauthListOrgs as Mock).mockResolvedValue(ORGS);
  (setOrg as Mock).mockResolvedValue(undefined);
  (proxyEnable as Mock).mockResolvedValue({ running: true });
  (getAccount as Mock).mockResolvedValue(oauthAccount());
  (oauthStatus as Mock).mockResolvedValue(SIGNED_IN);
});
afterEach(cleanup);

describe("useSetup: which stage", () => {
  it("shows nothing until the first read lands", () => {
    // A null account before the read is not the same as no account, and treating
    // them alike flashes sign-in at every signed-in user.
    const { api } = harness({ loaded: false });
    expect(api.current!.stage).toEqual({ kind: "loading" });
  });

  it("welcomes a user with no account", () => {
    const { api } = harness({ account: null, oauth: null });
    expect(api.current!.stage).toEqual({ kind: "welcome", reauth: false });
  });

  it("asks an OAuth account with a dead session to sign in again", () => {
    const { api } = harness({ account: oauthAccount(), oauth: SIGNED_OUT });
    expect(api.current!.stage).toEqual({ kind: "welcome", reauth: true });
  });

  it("does not claim a session expired for an API-key account", () => {
    // It never had one. `switch_gateway` clears the key, which is how this state
    // is reached.
    const { api } = harness({ account: keyAccount({ has_api_key: false }), oauth: null });
    expect(api.current!.stage).toEqual({ kind: "welcome", reauth: false });
  });

  it("sends a signed-in OAuth user with no org to the picker", () => {
    const { api } = harness({
      account: oauthAccount({ org_id: null, org_name: null }),
      oauth: SIGNED_IN,
    });
    expect(api.current!.stage).toEqual({ kind: "org-picker" });
  });

  it("never greets a user who was already signed in at launch", () => {
    // "Signed in" alone cannot tell a returning user from one who just finished
    // signing in, and confirming on every launch is a pane in the way.
    const { api } = harness({ account: oauthAccount(), oauth: SIGNED_IN });
    expect(api.current!.stage).toEqual({ kind: "ready" });
  });

  it("treats a stored key as signed in", () => {
    const { api } = harness({ account: keyAccount(), oauth: null });
    expect(api.current!.stage).toEqual({ kind: "ready" });
  });

  it("confirms a sign-in that happened here, then shows the app", async () => {
    const { api } = harness({ account: null, oauth: null });

    await act(async () => {
      await api.current!.signIn();
    });
    expect(api.current!.stage).toEqual({ kind: "connected" });

    act(() => api.current!.finish());
    expect(api.current!.stage).toEqual({ kind: "ready" });
  });
});

describe("useSetup: signing in", () => {
  it("saves the gateway before opening the browser flow", async () => {
    // The account has to exist on disk for the sign-in to record OAuth against.
    const { api } = harness({ account: null, oauth: null });

    await act(async () => {
      await api.current!.signIn();
    });

    expect(saveAccount).toHaveBeenCalledWith(DEFAULT_GATEWAY_BASE_URL, null);
    const saveOrder = (saveAccount as Mock).mock.invocationCallOrder[0];
    const loginOrder = (oauthBeginLogin as Mock).mock.invocationCallOrder[0];
    expect(saveOrder).toBeLessThan(loginOrder);
  });

  it("re-reads the session so the stage moves on its own", async () => {
    const { api } = harness({ account: null, oauth: null });

    await act(async () => {
      await api.current!.signIn();
    });

    expect(getAccount).toHaveBeenCalled();
    expect(api.current!.stage.kind).toBe("connected");
  });

  it("keeps the user on sign-in when the flow fails", async () => {
    (oauthBeginLogin as Mock).mockRejectedValue(new Error("browser closed"));
    const { api } = harness({ account: null, oauth: null });

    await act(async () => {
      await api.current!.signIn();
    });

    expect(api.current!.error).toBeTruthy();
    expect(api.current!.stage.kind).toBe("welcome");
    expect(api.current!.busy).toBe(false);
  });
});

describe("useSetup: the API-key path", () => {
  it("saves the key and answers the sign-in offer", async () => {
    // Choosing the key here is a deliberate decision; without marking the offer
    // seen it returns next launch and reverses it.
    (getAccount as Mock).mockResolvedValue(keyAccount());
    (oauthStatus as Mock).mockResolvedValue(SIGNED_OUT);
    const { api } = harness({ account: null, oauth: null });

    act(() => api.current!.setApiKey("  sk-gw-typed  "));
    await act(async () => {
      await api.current!.connectWithApiKey();
    });

    expect(saveAccount).toHaveBeenCalledWith(DEFAULT_GATEWAY_BASE_URL, "sk-gw-typed");
    expect(markOAuthOfferSeen).toHaveBeenCalled();
    expect(api.current!.stage.kind).toBe("connected");
  });

  it("does nothing for an empty key", async () => {
    const { api } = harness({ account: null, oauth: null });

    act(() => api.current!.setApiKey("   "));
    await act(async () => {
      await api.current!.connectWithApiKey();
    });

    expect(saveAccount).not.toHaveBeenCalled();
  });

  it("escapes the org-picker dead end to the key form", async () => {
    // Signed in, no organization, no admin to ask: the key form is the only way
    // forward, and derivation alone would keep returning to the picker.
    const { api } = harness({
      account: oauthAccount({ org_id: null, org_name: null }),
      oauth: SIGNED_IN,
    });
    expect(api.current!.stage.kind).toBe("org-picker");

    act(() => api.current!.useApiKeyInstead());

    expect(api.current!.stage).toEqual({ kind: "welcome", reauth: false });
    expect(api.current!.apiKeyOpen).toBe(true);
  });
});

describe("useSetup: choosing an organization", () => {
  it("picks the only organization without asking", async () => {
    (oauthListOrgs as Mock).mockResolvedValue([ORGS[0]]);
    const { api } = harness({
      account: oauthAccount({ org_id: null, org_name: null }),
      oauth: SIGNED_IN,
    });

    await act(async () => {
      await api.current!.loadOrgs();
    });

    expect(setOrg).toHaveBeenCalledWith("org-1", "Example Org");
  });

  it("waits for a choice when there are several", async () => {
    const { api } = harness({
      account: oauthAccount({ org_id: null, org_name: null }),
      oauth: SIGNED_IN,
    });

    await act(async () => {
      await api.current!.loadOrgs();
    });

    expect(setOrg).not.toHaveBeenCalled();
    expect(api.current!.orgs).toEqual(ORGS);
    expect(api.current!.selectedOrgId).toBe("org-1");

    act(() => api.current!.selectOrg("org-2"));
    await act(async () => {
      await api.current!.confirmOrg();
    });

    expect(setOrg).toHaveBeenCalledWith("org-2", "Other Org");
  });

  it("records an empty list rather than retrying forever", async () => {
    // The dead end the picker draws. `null` means unread, `[]` means read and
    // empty, and the container's effect only re-reads on `null`.
    (oauthListOrgs as Mock).mockResolvedValue([]);
    const { api } = harness({
      account: oauthAccount({ org_id: null, org_name: null }),
      oauth: SIGNED_IN,
    });

    await act(async () => {
      await api.current!.loadOrgs();
    });

    expect(api.current!.orgs).toEqual([]);
  });
});

describe("useSetup: finishing", () => {
  it("turns routing on and closes the confirmation", async () => {
    // Finishing with routing off would leave a window that says connected over
    // apps carrying no traffic.
    const { api, onProxy } = harness({ account: null, oauth: null });
    await act(async () => {
      await api.current!.signIn();
    });

    await act(async () => {
      await api.current!.turnOnRouting();
    });

    expect(proxyEnable).toHaveBeenCalled();
    expect(onProxy).toHaveBeenCalledWith({ running: true });
    expect(api.current!.stage.kind).toBe("ready");
  });

  it("stays on the confirmation when routing cannot start", async () => {
    (proxyEnable as Mock).mockRejectedValue(new Error("no engine"));
    const { api } = harness({ account: null, oauth: null });
    await act(async () => {
      await api.current!.signIn();
    });

    await act(async () => {
      await api.current!.turnOnRouting();
    });

    expect(api.current!.error).toBeTruthy();
    expect(api.current!.stage.kind).toBe("connected");
  });
});
