import { test, expect } from "./fixtures";

const NO_ACCOUNT = {
  account: null,
  oauth: { signed_in: false, email: null, expires_at_unix: 0 },
} as const;

/** Sign-in, both paths, all the way to Home. Each screen here has unit tests
 *  of its own; what only this suite sees is App handing one to the next -
 *  first run to org picker to success - off state it re-reads from the
 *  backend rather than off what the previous screen told it. */
test.describe("sign in", () => {
  test("Constellation sign-in routes through the org picker to Home", async ({ boot }) => {
    const app = await boot(NO_ACCOUNT);

    await app.page.getByRole("button", { name: /Sign in with Constellation/ }).click();

    // The account is persisted before the browser flow starts, so the backend
    // has somewhere to record the auth mode.
    await expect
      .poll(() => app.lastCall("save_account"))
      .toEqual({ baseUrl: "https://gateway.constellationgate.ai", apiKey: null });

    // Signed in, no org yet: the picker, not Home.
    await expect(app.page.getByText("Side Project")).toBeVisible();
    await app.page.getByText("Constellation Labs").click();

    await expect(app.page.getByRole("heading", { name: /connected/i })).toBeVisible();
    expect(await app.lastCall("set_org")).toEqual({ orgId: "org-1", orgName: "Constellation Labs" });

    // Routing is off and there is a proxy subsystem, so Success offers the
    // last step rather than claiming to be routing.
    await app.page.getByRole("button", { name: "Turn on routing" }).click();
    await expect(app.routingSwitch).toHaveAttribute("aria-checked", "true");
    expect((await app.state()).proxy.running).toBe(true);
  });

  test("the API key path saves the key and skips the org picker", async ({ boot }) => {
    const app = await boot(NO_ACCOUNT);

    await app.page.getByRole("button", { name: "Use an API key instead" }).click();
    await app.page.getByPlaceholder("sk-gw-").fill("sk-gw-test-key");
    await app.page.getByRole("button", { name: "Connect with key" }).click();

    await expect(app.page.getByRole("heading", { name: /connected/i })).toBeVisible();
    expect(await app.lastCall("save_account")).toEqual({
      baseUrl: "https://gateway.constellationgate.ai",
      apiKey: "sk-gw-test-key",
    });
    // A key account has no session to pick an org for.
    expect(await app.lastCall("set_org")).toBeNull();

    await app.page.getByRole("button", { name: "Not now" }).click();
    await expect(app.page.getByRole("heading", { name: "Routing" })).toBeVisible();
  });

  test("a failed sign-in explains itself and stays on first run", async ({ boot }) => {
    const app = await boot({
      ...NO_ACCOUNT,
      failures: { oauth_begin_login: "sign-in timed out waiting for the browser redirect" },
    });

    await app.page.getByRole("button", { name: /Sign in with Constellation/ }).click();

    // Classified, not raw, and the screen is still usable.
    await expect(app.page.getByText(/timed out|couldn|try again/i).first()).toBeVisible();
    await expect(app.page.getByRole("button", { name: /Sign in with Constellation/ })).toBeEnabled();
  });

  test("the key form is reachable from the org picker's dead end", async ({ boot }) => {
    // Signed in, but the account is on no organization at all: the picker is
    // the only screen that can say so, and a key is the only way forward.
    const app = await boot({ account: { org_id: null, org_name: null }, orgs: [] });

    await expect(app.page.getByRole("button", { name: /API key/i }).first()).toBeVisible();
  });
});

/** Sign-out returns to the sign-in prompt, not to first run: the account and
 *  everything routed through it stay put for the next sign-in. */
test.describe("sign out", () => {
  test("keeps the account and lands back on the sign-in prompt", async ({ boot }) => {
    const app = await boot();

    await app.openSettings();
    await app.page.getByRole("button", { name: "Sign out" }).click();

    await expect(app.page.getByRole("heading", { name: /Welcome/ })).toBeVisible();
    const state = await app.state();
    expect(state.oauth.signed_in).toBe(false);
    expect(state.account).not.toBeNull();
  });
});
