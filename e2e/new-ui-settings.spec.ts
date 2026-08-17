import { test, expect } from "./fixtures";

/**
 * The new window UI's Settings actions and org switcher, against the same fake
 * backend the popover suite uses.
 *
 * Same per-test opt-in as `new-ui-routing.spec.ts`: the suite is pinned to the
 * popover, and `newUiEnabled()` reads localStorage before the build-time
 * default.
 *
 * What this covers that `lib/useSettingsActions.test.tsx` cannot: that the row
 * is wired to the action at all, that the dialog the design specifies is the one
 * that opens, and that confirming it reaches the backend.
 */
const useNewUi = { gc: "gc.newUi" };

test.describe("new UI settings", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("the launch-at-login switch reaches the backend", async ({ boot }) => {
    const app = await boot({});

    await app.page.getByRole("button", { name: "Settings" }).click();
    const launch = app.page.getByRole("switch", { name: "Launch at login" });
    await expect(launch).toHaveAttribute("aria-checked", "false");

    await launch.click();

    await expect.poll(() => app.lastCall("set_launch_at_login")).toEqual({ enabled: true });
    await expect(launch).toHaveAttribute("aria-checked", "true");
  });

  test("replacing the API key saves against the current gateway", async ({ boot }) => {
    const app = await boot({
      account: {
        gateway_base_url: "https://gw.example",
        has_api_key: true,
        auth_mode: "api_key",
      },
    });

    await app.page.getByRole("button", { name: "Settings" }).click();
    await app.page.getByRole("button", { name: "Replace key" }).click();

    await expect(app.page.getByRole("heading", { name: "Replace API key" })).toBeVisible();
    // Nothing is written until the primary is pressed.
    expect(await app.lastCall("save_account")).toBeNull();

    await app.page.getByLabel("New API key").fill("sk-gw-replacement");
    await app.page.getByRole("button", { name: "Replace key" }).last().click();

    await expect.poll(() => app.lastCall("save_account")).toEqual({
      baseUrl: "https://gw.example",
      apiKey: "sk-gw-replacement",
    });
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
  });

  test("cancelling the key dialog writes nothing", async ({ boot }) => {
    const app = await boot({});

    await app.page.getByRole("button", { name: "Settings" }).click();
    await app.page.getByRole("button", { name: "Replace key" }).click();
    await app.page.getByLabel("New API key").fill("sk-gw-typed");
    await app.page.getByRole("button", { name: "Cancel" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    expect(await app.lastCall("save_account")).toBeNull();

    // Reopening starts empty: a credential should not survive a cancel.
    await app.page.getByRole("button", { name: "Replace key" }).click();
    await expect(app.page.getByLabel("New API key")).toHaveValue("");
  });

  test("switching organization picks the current one and confirms the change", async ({
    boot,
  }) => {
    const app = await boot({});

    await app.page.getByRole("button", { name: /Constellation Labs/ }).click();

    await expect(app.page.getByRole("heading", { name: "Switch organization" })).toBeVisible();
    await app.page.getByRole("radio", { name: /Side Project/ }).click();
    await app.page.getByRole("button", { name: "Switch organization" }).last().click();

    await expect
      .poll(() => app.lastCall("set_org"))
      .toEqual({ orgId: "org-2", orgName: "Side Project" });
    // Confirmation rather than a silent close: the org decides what gets billed.
    await expect(app.page.getByRole("heading", { name: "Organization switched" })).toBeVisible();

    await app.page.getByRole("button", { name: "Done" }).click();
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
  });

  test("no picker opens when there is only one organization", async ({ boot }) => {
    // A question with one answer.
    const app = await boot({
      orgs: [{ orgId: "org-1", name: "Constellation Labs", slug: "constellation", role: "admin" }],
    });

    await app.page.getByRole("button", { name: /Constellation Labs/ }).click();

    // The list is read - the click is not ignored - and then nothing opens.
    await expect
      .poll(() => app.calls().then((c) => c.some((x) => x.cmd === "oauth_list_orgs")))
      .toBe(true);
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
  });

  test("replaying the tutorial opens the onboarding window", async ({ boot }) => {
    const app = await boot({});

    await app.page.getByRole("button", { name: "Settings" }).click();
    await app.page.getByRole("button", { name: "Replay tutorial" }).click();

    await expect
      .poll(() => app.lastCall("open_onboarding_window"))
      .toEqual({ source: "settings" });
  });

  test("rows with no backend show no control", async ({ boot }) => {
    // Hidden rather than dead: the user cannot tell an inert control from a
    // broken one, and on the Danger zone card cannot tell it from "already done".
    const app = await boot({});

    await app.page.getByRole("button", { name: "Settings" }).click();
    await expect(app.page.getByRole("heading", { name: "Device" })).toBeVisible();

    for (const gone of ["Rename device", "Upgrade plan", "Check for updates", "Review reset"]) {
      await expect(app.page.getByRole("button", { name: gone })).toHaveCount(0);
    }
    await expect(app.page.getByRole("switch", { name: "Notifications" })).toHaveCount(0);
    await expect(app.page.getByRole("heading", { name: "Danger zone" })).toHaveCount(0);
  });

  test("the diagnostics report can be copied", async ({ boot, context }) => {
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    const app = await boot({});

    await app.page.getByRole("button", { name: "Settings" }).click();
    await app.page.getByRole("button", { name: "View report" }).click();
    await app.page.getByRole("button", { name: /Copy report/ }).click();

    // The copy hands over exactly what the dialog showed.
    const copied = await app.page.evaluate(() => navigator.clipboard.readText());
    expect(copied).toContain("Gate Connect");
  });
});
