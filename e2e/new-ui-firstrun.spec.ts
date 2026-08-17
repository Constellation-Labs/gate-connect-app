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

    await expect(app.page.getByRole("heading", { name: "Welcome to Gate Connect" })).toBeVisible();
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

    await app.page.getByRole("button", { name: "Sign in with Constellation" }).click();

    // The account has to exist on disk for the sign-in to record OAuth against.
    await expect.poll(() => app.lastCall("save_account")).toMatchObject({ apiKey: null });
    // Two orgs in the default fixture, so this is a real choice.
    await expect(app.page.getByRole("heading", { name: "Choose an organization" })).toBeVisible();

    await app.page.getByRole("radio", { name: /Side Project/ }).click();
    await app.page.getByRole("button", { name: "Continue" }).click();

    await expect
      .poll(() => app.lastCall("set_org"))
      .toEqual({ orgId: "org-2", orgName: "Side Project" });
    await expect(app.page.getByRole("heading", { name: "You're connected" })).toBeVisible();
  });

  test("a single organization is chosen without asking", async ({ boot }) => {
    const app = await boot({
      account: null,
      oauth: { signed_in: false, email: null, expires_at_unix: 0 },
      orgs: [{ orgId: "org-1", name: "Only Org", slug: "only", role: "admin" }],
    });

    await app.page.getByRole("button", { name: "Sign in with Constellation" }).click();

    await expect.poll(() => app.lastCall("set_org")).toEqual({
      orgId: "org-1",
      orgName: "Only Org",
    });
    await expect(app.page.getByRole("heading", { name: "You're connected" })).toBeVisible();
  });

  test("the API-key path reaches the app without an org", async ({ boot }) => {
    const app = await boot({
      account: null,
      oauth: { signed_in: false, email: null, expires_at_unix: 0 },
    });

    await app.page.getByRole("button", { name: "Use a Gate API key instead" }).click();
    await app.page.getByLabel("Gate API key").fill("sk-gw-pasted");
    await app.page.getByRole("button", { name: "Connect" }).click();

    await expect.poll(() => app.lastCall("save_account")).toMatchObject({
      apiKey: "sk-gw-pasted",
    });
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

    await app.page.getByRole("button", { name: "Sign in with Constellation" }).click();
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

    await expect(app.page.getByRole("heading", { name: "Welcome to Gate Connect" })).toBeVisible();
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

  test("an OAuth account is not offered a key to replace", async ({ boot }) => {
    // saveAccount with a key would flip auth_mode to api_key, quietly converting
    // the account behind a button that says "replace".
    const app = await boot({});

    await app.page.getByRole("button", { name: "Settings" }).click();

    await expect(app.page.getByRole("button", { name: "Replace key" })).toHaveCount(0);
    await expect(app.page.getByRole("button", { name: "Disconnect Gate" })).toBeVisible();
  });
});
