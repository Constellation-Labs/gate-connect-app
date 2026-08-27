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
  oauthSignOut: vi.fn(),
  oauthStatus: vi.fn(),
  proxyEnable: vi.fn(),
  saveAccount: vi.fn(),
  setDeviceName: vi.fn(),
  setOrg: vi.fn(),
}));
vi.mock("./analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));
vi.mock("./oauthOffer", () => ({ markOAuthOfferSeen: vi.fn() }));

import {
  getAccount,
  oauthBeginLogin,
  oauthListOrgs,
  oauthSignOut,
  oauthStatus,
  proxyEnable,
  saveAccount,
  setDeviceName,
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
  billing_mode: "byok",
  org_id: "org-1",
  org_name: "Example Org",
  ...over,
});

const keyAccount = (over: Partial<Account> = {}): Account => ({
  gateway_base_url: "https://gw.example",
  has_api_key: true,
  auth_mode: "api_key",
  billing_mode: "byok",
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
  deviceNamed?: boolean;
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
      deviceNamed: initial.deviceNamed,
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
  (oauthSignOut as Mock).mockResolvedValue(undefined);
  (setDeviceName as Mock).mockResolvedValue(undefined);
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

  /**
   * Re-signing in must not repoint the install.
   *
   * The gateway used to be `useState(DEFAULT_GATEWAY_BASE_URL)`, seeded before
   * the account read landed and never synced, so this write sent production for
   * an account that had been on staging. That also picks the Cognito pool
   * `OAuthConfig::from_build_env` resolves, so the visible symptom was not a
   * wrong URL in Settings but a sign-in that failed before the browser opened.
   */
  it("re-signs in against the account's own gateway, not the build default", async () => {
    const { api } = harness({
      account: oauthAccount({ gateway_base_url: "https://gateway-staging.example" }),
      oauth: SIGNED_OUT,
    });

    await act(async () => {
      await api.current!.signIn();
    });

    expect(saveAccount).toHaveBeenCalledWith("https://gateway-staging.example", null);
    expect(saveAccount).not.toHaveBeenCalledWith(DEFAULT_GATEWAY_BASE_URL, null);
  });

  it("still uses the build default when there is no account to read one from", async () => {
    // First run on a fresh machine: nothing on disk, so the default is the only
    // answer, and the picker keeps overriding it.
    const { api } = harness({ account: null, oauth: null });
    expect(api.current!.gateway).toBe(DEFAULT_GATEWAY_BASE_URL);

    act(() => api.current!.setGateway("https://gateway-staging.example"));
    expect(api.current!.gateway).toBe("https://gateway-staging.example");

    await act(async () => {
      await api.current!.signIn();
    });
    expect(saveAccount).toHaveBeenCalledWith("https://gateway-staging.example", null);
  });

  /** The picker is an explicit choice and outranks the account it was made
   *  against, or selecting a server would appear to do nothing. */
  it("lets the picker override the account's gateway", () => {
    const { api } = harness({
      account: oauthAccount({ gateway_base_url: "https://gw.example" }),
      oauth: SIGNED_OUT,
    });
    expect(api.current!.gateway).toBe("https://gw.example");

    act(() => api.current!.setGateway("https://gateway-staging.example"));
    expect(api.current!.gateway).toBe("https://gateway-staging.example");
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

  it("escapes the org-picker dead end by dropping the session", async () => {
    // Signed in, no organization, no admin to ask. `Auth / Organizations` sends
    // this back to the sign-in choice rather than sideways into the key form:
    // an account with nothing to route for cannot go forward, and both drawn
    // affordances - "Go back" and "Use a different account" - mean this.
    const { api } = harness({
      account: oauthAccount({ org_id: null, org_name: null }),
      oauth: SIGNED_IN,
    });
    expect(api.current!.stage.kind).toBe("org-picker");

    (getAccount as Mock).mockResolvedValue(null);
    (oauthStatus as Mock).mockResolvedValue(SIGNED_OUT);
    await act(async () => {
      await api.current!.signOut();
    });

    expect(oauthSignOut).toHaveBeenCalled();
    // The gateway survives, and it is *the account's own* - not the build
    // default. This assertion used to name the default while the comment
    // claimed the gateway survived, which is the bug the two together hid:
    // signing out, then back in, rewrote a staging or local account with the
    // production URL.
    expect(saveAccount).toHaveBeenCalledWith("https://gw.example", null);
    expect(api.current!.stage).toEqual({ kind: "welcome", reauth: false });
  });

  it("opens the key route as its own pane, and goes back", () => {
    const { api } = harness({ account: null, oauth: null });

    act(() => api.current!.openApiKey());
    expect(api.current!.stage).toEqual({ kind: "api-key" });

    act(() => api.current!.closeApiKey());
    expect(api.current!.stage).toEqual({ kind: "welcome", reauth: false });
  });

  it("names the device after a sign-in, before confirming", async () => {
    const { api } = harness({ account: null, oauth: null, deviceNamed: false });

    await act(async () => {
      await api.current!.signIn();
    });
    expect(api.current!.stage).toEqual({ kind: "name-device" });

    act(() => api.current!.setDeviceNameDraft("Chad's Macbook Pro"));
    await act(async () => {
      await api.current!.nameDevice();
    });

    expect(setDeviceName).toHaveBeenCalledWith("Chad's Macbook Pro");
    // Does not wait on the preferences re-read that flips `deviceNamed`.
    expect(api.current!.stage).toEqual({ kind: "connected" });
  });

  it("lets naming be skipped, and does not ask again", async () => {
    const { api } = harness({ account: null, oauth: null, deviceNamed: false });

    await act(async () => {
      await api.current!.signIn();
    });
    expect(api.current!.stage).toEqual({ kind: "name-device" });

    act(() => api.current!.skipNaming());

    expect(setDeviceName).not.toHaveBeenCalled();
    expect(api.current!.stage).toEqual({ kind: "connected" });
  });

  it("never asks a returning user to name a machine that follows the hostname", () => {
    // `device_name` is null for everyone who never renamed, so deriving from it
    // alone would greet them with the pane on every launch.
    const { api } = harness({
      account: oauthAccount(),
      oauth: SIGNED_IN,
      deviceNamed: false,
    });
    expect(api.current!.stage).toEqual({ kind: "ready" });
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
