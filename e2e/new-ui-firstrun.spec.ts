import { test, expect } from "./fixtures";

/**
 * First run and the two actions that return to it, in the new window UI.
 *
 * Same per-test opt-in as the other new-UI specs. `{ account: null }` is how a
 * spec asks for first run.
 *
 * The point of these: the setup stage is *derived* from account and OAuth state
 * rather than stored, so what needs proving is that each on-disk state puts the
 * right pane on screen - and that reset gets back here with nothing left over.
 */
const useNewUi = { gc: "gc.newUi" };

test.describe("new UI first run", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("no account lands on sign-in, not the app shell", async ({ boot }) => {
    const app = await boot({ account: null, oauth: { signed_in: false, email: null, expires_at_unix: 0 } });

    await expect(app.page.getByRole("heading", { name: "Gate Connect" })).toBeVisible();
    // No sidebar: there is nothing to navigate to yet.
    await expect(app.page.getByRole("navigation", { name: "Main" })).toHaveCount(0);
  });

  test("signing in saves the gateway first, then routes through the org picker", async ({
    boot,
  }) => {
    const app = await boot({
      account: null,
      oauth: { signed_in: false, email: null, expires_at_unix: 0 },
    });

    await app.page.getByRole("button", { name: "Continue with Gate account" }).click();

    // The account has to exist on disk for the sign-in to record OAuth against.
    await expect.poll(() => app.lastCall("save_account")).toMatchObject({ apiKey: null });
    // Two orgs in the default fixture, so this is a real choice.
    await expect(app.page.getByRole("heading", { name: "Choose an organization" })).toBeVisible();

    await app.page.getByRole("radio", { name: /Side Project/ }).click();
    await app.page.getByRole("button", { name: "Continue" }).click();

    await expect
      .poll(() => app.lastCall("set_org"))
      .toEqual({ orgId: "org-2", orgName: "Side Project" });
    // The machine is named before any app is chosen, which is the order the
    // Auth flow's own copy promises.
    await expect(app.page.getByRole("heading", { name: "Name this device" })).toBeVisible();
    await app.page.getByRole("button", { name: "Skip naming" }).click();
    await expect(app.page.getByRole("heading", { name: "You're connected" })).toBeVisible();
  });

  test("names the device between connecting and the confirmation", async ({ boot }) => {
    const app = await boot({
      account: null,
      oauth: { signed_in: false, email: null, expires_at_unix: 0 },
      orgs: [{ orgId: "org-1", name: "Only Org", slug: "only", role: "admin" }],
    });

    await app.page.getByRole("button", { name: "Continue with Gate account" }).click();

    const name = app.page.getByLabel("Device name");
    await expect(name).toBeVisible();
    // Refused while the field is empty: skipping is the way past, not an empty
    // Continue that would write a blank name and clear the override.
    await expect(app.page.getByRole("button", { name: "Continue" })).toBeDisabled();

    await name.fill("Studio Mac");
    await app.page.getByRole("button", { name: "Continue" }).click();

    await expect.poll(() => app.lastCall("set_device_name")).toEqual({ name: "Studio Mac" });
    await expect(app.page.getByRole("heading", { name: "You're connected" })).toBeVisible();
  });

  test("a single organization is chosen without asking", async ({ boot }) => {
    const app = await boot({
      account: null,
      oauth: { signed_in: false, email: null, expires_at_unix: 0 },
      orgs: [{ orgId: "org-1", name: "Only Org", slug: "only", role: "admin" }],
    });

    await app.page.getByRole("button", { name: "Continue with Gate account" }).click();

    await expect.poll(() => app.lastCall("set_org")).toEqual({
      orgId: "org-1",
      orgName: "Only Org",
    });
    await app.page.getByRole("button", { name: "Skip naming" }).click();
    await expect(app.page.getByRole("heading", { name: "You're connected" })).toBeVisible();
  });

  test("the API-key path reaches the app without an org", async ({ boot }) => {
    const app = await boot({
      account: null,
      oauth: { signed_in: false, email: null, expires_at_unix: 0 },
    });

    // The key is a destination of its own, not a form that unfolds under the
    // sign-in buttons.
    await app.page.getByRole("button", { name: "Use an API key" }).click();
    await expect(app.page.getByRole("heading", { name: "Use an API key" })).toBeVisible();
    await app.page.getByLabel("API key").fill("sk-gw-pasted");
    await app.page.getByRole("button", { name: "Connect and continue" }).click();

    await expect.poll(() => app.lastCall("save_account")).toMatchObject({
      apiKey: "sk-gw-pasted",
    });
    // The key route names the device too - the pane's copy says so.
    await app.page.getByRole("button", { name: "Skip naming" }).click();
    await expect(app.page.getByRole("heading", { name: "You're connected" })).toBeVisible();
  });

  test("the confirmation finishes the job it promises", async ({ boot }) => {
    // Routing is off in the fixture, so "Done" alone would leave a window that
    // says connected over apps carrying no traffic.
    const app = await boot({
      account: null,
      oauth: { signed_in: false, email: null, expires_at_unix: 0 },
      orgs: [{ orgId: "org-1", name: "Only Org", slug: "only", role: "admin" }],
    });

    await app.page.getByRole("button", { name: "Continue with Gate account" }).click();
    await app.page.getByRole("button", { name: "Skip naming" }).click();
    await app.page.getByRole("button", { name: "Turn on routing" }).click();

    await expect.poll(() => app.calls().then((c) => c.some((x) => x.cmd === "proxy_enable"))).toBe(
      true,
    );
    // And then the app itself.
    await expect(app.page.getByRole("navigation", { name: "Main" })).toBeVisible();
  });

  test("an expired session asks to sign in again rather than welcoming", async ({ boot }) => {
    const app = await boot({
      oauth: { signed_in: false, email: null, expires_at_unix: 0 },
    });

    await expect(app.page.getByRole("heading", { name: "Session expired" })).toBeVisible();
  });

  test("a signed-in account goes straight to the app", async ({ boot }) => {
    // Nobody who is already set up should meet a confirmation pane on launch.
    const app = await boot({});

    await expect(app.page.getByRole("navigation", { name: "Main" })).toBeVisible();
    await expect(app.page.getByRole("heading", { name: "You're connected" })).toHaveCount(0);
  });
});

test.describe("new UI: the two ways back to first run", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("reset turns routing off before wiping, and lands on sign-in", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });

    await app.page.getByRole("button", { name: "Settings" }).click();
    await app.page.getByRole("button", { name: "Review reset" }).click();

    // Refused until the consequences are acknowledged.
    const reset = app.page.getByRole("button", { name: "Reset Gate Connect" });
    await expect(reset).toBeDisabled();
    await app.page.getByRole("checkbox").check();
    await reset.click();

    await expect.poll(() => app.calls().then((c) => c.some((x) => x.cmd === "clear_account"))).toBe(
      true,
    );
    const calls = await app.calls();
    const disable = calls.findIndex((c) => c.cmd === "proxy_disable");
    const clear = calls.findIndex((c) => c.cmd === "clear_account");
    expect(disable).toBeGreaterThanOrEqual(0);
    expect(disable).toBeLessThan(clear);

    await expect(app.page.getByRole("heading", { name: "Gate Connect" })).toBeVisible();
  });

  test("disconnect ends the session and keeps the account", async ({ boot }) => {
    const app = await boot({});

    await app.page.getByRole("button", { name: "Settings" }).click();
    await app.page.getByRole("button", { name: "Disconnect Gate" }).click();
    await app.page.getByRole("button", { name: "Yes, disconnect Gate" }).click();

    await expect
      .poll(() => app.calls().then((c) => c.some((x) => x.cmd === "oauth_sign_out")))
      .toBe(true);
    const calls = await app.calls();
    expect(calls.some((c) => c.cmd === "clear_account")).toBe(false);

    // Account intact, session gone: that is the reauth prompt, not a welcome.
    await expect(app.page.getByRole("heading", { name: "Session expired" })).toBeVisible();
  });

  test("an API-key account is offered reset but not disconnect", async ({ boot }) => {
    // It never had a session to end, and Replace key is its own row.
    const app = await boot({
      account: { has_api_key: true, auth_mode: "api_key", org_id: null, org_name: null },
    });

    await app.page.getByRole("button", { name: "Settings" }).click();

    await expect(app.page.getByRole("button", { name: "Disconnect Gate" })).toHaveCount(0);
    await expect(app.page.getByRole("button", { name: "Review reset" })).toBeVisible();
    await expect(app.page.getByRole("button", { name: "Replace key" })).toBeVisible();
  });

  test("a key account can move to a Gate account from Settings", async ({ boot }) => {
    // The popover has carried this since it shipped. The window had only the
    // one-time OAuth offer, and dismissing that once left no route to a Gate
    // account at all.
    const app = await boot({
      account: { has_api_key: true, auth_mode: "api_key", org_id: null, org_name: null },
    });

    await app.page.getByRole("button", { name: "Settings" }).click();
    await app.page.getByRole("button", { name: "Use a Gate account" }).click();

    await expect.poll(() => app.lastCall("oauth_begin_login")).not.toBeNull();
    // Not `save_account`: that would repoint the account at the default gateway
    // and drop the key the user still has.
    expect(await app.lastCall("save_account")).toBeNull();
  });

  test("a failed browser sign-in says so instead of going quiet", async ({ boot }) => {
    // The row called upgradeToOAuth through `void`, so a rejected browser flow
    // became an unhandled promise: the busy flag cleared and the pane went
    // silent, which is exactly what a button that does nothing looks like.
    const app = await boot({
      account: { has_api_key: true, auth_mode: "api_key", org_id: null, org_name: null },
      failures: { oauth_begin_login: "the sign-in window closed before it finished" },
    });

    await app.page.getByRole("button", { name: "Settings" }).click();
    await app.page.getByRole("button", { name: "Use a Gate account" }).click();

    const alert = app.page.getByRole("alert");
    await expect(alert).toBeVisible();
    // Not "Couldn't save your account": nothing was being saved.
    await expect(alert).toContainText(/complete sign-in/);

    // The hint says the details below help when reporting it, so they have to
    // actually be below it. The banner used to render none.
    await alert.getByText("Details", { exact: true }).click();
    await expect(alert).toContainText("the sign-in window closed before it finished");

    // And the row survives, so the user can try again.
    await expect(app.page.getByRole("button", { name: "Use a Gate account" })).toBeVisible();
  });

  test("an OAuth account is not offered a key to replace", async ({ boot }) => {
    // saveAccount with a key would flip auth_mode to api_key, quietly converting
    // the account behind a button that says "replace".
    const app = await boot({});

    await app.page.getByRole("button", { name: "Settings" }).click();

    await expect(app.page.getByRole("button", { name: "Replace key" })).toHaveCount(0);
    await expect(app.page.getByRole("button", { name: "Disconnect Gate" })).toBeVisible();
    // And no offer to switch to what it already is.
    await expect(app.page.getByRole("button", { name: "Use a Gate account" })).toHaveCount(0);
  });

  test("an OAuth account is not shown an API key at all", async ({ boot }) => {
    // The upgrade leaves the old key in the keychain, so `has_api_key` stays
    // true and Connection drew a masked key for a session a Cognito bearer
    // authenticates - naming the wrong credential on the one screen whose job
    // is to say where the credential lives.
    const app = await boot({ account: { has_api_key: true, auth_mode: "oauth" } });

    await app.page.getByRole("button", { name: "Settings" }).click();

    await expect(app.page.getByText("API key", { exact: true })).toHaveCount(0);
    // Replaced by what actually signs them in, rather than left blank.
    await expect(app.page.getByText("Gate account", { exact: true })).toBeVisible();
    await expect(app.page.getByRole("button", { name: "Disconnect Gate" })).toBeVisible();
  });

  test("a failed sign-in on the setup screen shows its details too", async ({ boot }) => {
    // The reported case: disconnect, then sign in again, and the browser never
    // opens. `ErrorBanner` had gained the expander; the setup screen had not, so
    // the one surface with no shell behind it promised details and showed none.
    const app = await boot({
      account: null,
      oauth: { signed_in: false, email: null, expires_at_unix: 0 },
      failures: { oauth_begin_login: "OAuth is not configured in this build" },
    });

    await app.page.getByRole("button", { name: "Continue with Gate account" }).click();

    const alert = app.page.getByRole("alert");
    await expect(alert).toBeVisible();
    await expect(alert).toContainText(/complete sign-in/);

    await alert.getByText("Details", { exact: true }).click();
    await expect(alert).toContainText("OAuth is not configured in this build");
  });

  test("re-signing in keeps the account's own gateway", async ({ boot }) => {
    // Disconnecting leaves the account on disk. The sign-in pane used to send
    // the *build default* here, so a staging install was silently repointed at
    // production - which also changes the Cognito pool the login resolves.
    const app = await boot({
      account: { gateway_base_url: "https://gateway-staging.example", auth_mode: "oauth" },
      oauth: { signed_in: false, email: null, expires_at_unix: 0 },
    });

    await app.page.getByRole("button", { name: "Continue with Gate account" }).click();

    await expect
      .poll(() => app.lastCall("save_account"))
      .toMatchObject({ baseUrl: "https://gateway-staging.example", apiKey: null });
  });
});

/**
 * AG-554 / AG-603: the diagnostic-data step, between the sign-in confirmation and
 * Overview.
 *
 * Consent belongs before collection, and `lib/analytics.ts` starts PostHog at
 * launch - so what this step buys is a person who has actually been asked. The
 * stage is derived from the stored answer rather than remembered in UI state,
 * which is why a reload cannot skip it.
 */
test.describe("new UI: the diagnostic-data step", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  const unanswered = {
    account: {
      gateway_base_url: "https://gw.example",
      has_api_key: true,
      auth_mode: "api_key" as const,
    },
    preferences: {
      routing_health_notifications: true,
      share_diagnostics: true,
      share_diagnostics_recorded: false,
    },
  };

  test("an unanswered install is asked before it reaches the app", async ({ boot }) => {
    const app = await boot(unanswered);

    await expect(app.page.getByRole("heading", { name: "Help fix problems" })).toBeVisible();
    // The switch defaults on, and the primary is Continue rather than Accept:
    // leaving it alone is a real answer.
    await expect(
      app.page.getByRole("switch", { name: "Share diagnostic data" }),
    ).toHaveAttribute("aria-checked", "true");
    await expect(app.page.getByRole("button", { name: "Continue" })).toBeVisible();
  });

  test("it lists what is and is not collected", async ({ boot }) => {
    const app = await boot(unanswered);

    await expect(app.page.getByText("Sent", { exact: true })).toBeVisible();
    await expect(app.page.getByText("Never sent")).toBeVisible();
    await expect(app.page.getByText(/Prompts or model responses/)).toBeVisible();
  });

  /** Leaving the default in place is an answer, so Continue must record it - or the
   * next launch would ask again. */
  test("Continue records an unchanged choice and opens the app", async ({ boot }) => {
    const app = await boot(unanswered);

    await app.page.getByRole("button", { name: "Continue" }).click();

    await expect.poll(() => app.lastCall("set_share_diagnostics")).toEqual({ enabled: true });
    await expect(app.page.getByRole("button", { name: "Settings" })).toBeVisible();
  });

  test("switching it off records the refusal, and still opens the app", async ({ boot }) => {
    const app = await boot(unanswered);

    await app.page.getByRole("switch", { name: "Share diagnostic data" }).click();
    await app.page.getByRole("button", { name: "Continue" }).click();

    await expect.poll(() => app.lastCall("set_share_diagnostics")).toEqual({ enabled: false });
    await expect(app.page.getByRole("button", { name: "Settings" })).toBeVisible();
  });

  test("an install that already answered is not asked again", async ({ boot }) => {
    const app = await boot({
      ...unanswered,
      preferences: { ...unanswered.preferences, share_diagnostics_recorded: true },
    });

    await expect(app.page.getByRole("heading", { name: "Help fix problems" })).toHaveCount(0);
    await expect(app.page.getByRole("button", { name: "Settings" })).toBeVisible();
  });
});
