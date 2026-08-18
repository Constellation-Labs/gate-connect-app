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
    // An API-key account: the row is gated to accounts that have a key to replace.
    const app = await boot({
      account: { has_api_key: true, auth_mode: "api_key", org_id: null, org_name: null },
    });

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
    // broken one. Reset is wired now, so the Danger zone is back - see
    // new-ui-firstrun.spec.ts.
    const app = await boot({});

    await app.page.getByRole("button", { name: "Settings" }).click();
    await expect(app.page.getByRole("heading", { name: "Device" })).toBeVisible();

    for (const gone of ["Rename device", "Upgrade plan"]) {
      await expect(app.page.getByRole("button", { name: gone })).toHaveCount(0);
    }
    await expect(app.page.getByRole("switch", { name: "Notifications" })).toHaveCount(0);
    await expect(app.page.getByRole("heading", { name: "Danger zone" })).toBeVisible();
  });

  test("checking for updates reports back", async ({ boot }) => {
    // The fixture's updater says there is nothing to install. Silence on a
    // button the user just pressed reads as broken, so the row has to say so.
    const app = await boot({});

    await app.page.getByRole("button", { name: "Settings" }).click();
    await app.page.getByRole("button", { name: "Check for updates" }).click();

    await expect(app.page.getByText("You're on the latest version.")).toBeVisible();
    // And no banner, because there is no update.
    await expect(app.page.getByText(/Update available/)).toHaveCount(0);
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

/**
 * The Settings sections AG-594 asks for, and the two preference switches behind
 * them.
 *
 * What these cover that `SettingsPane.test.tsx` cannot: that the switches reach
 * the backend, and that a failed read renders Unavailable instead of an Off
 * switch - which is the difference between "you turned this off" and "we could
 * not tell", and the reason the failure flag is tracked separately from the
 * value.
 */
test.describe("new UI settings preferences", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("both preference switches read On before anything has been written", async ({
    boot,
  }) => {
    const app = await boot({});
    await app.page.getByRole("button", { name: "Settings" }).click();

    await expect(app.page.getByRole("switch", { name: "Routing health" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await expect(
      app.page.getByRole("switch", { name: "Share diagnostic data" }),
    ).toHaveAttribute("aria-checked", "true");
  });

  test("turning routing-health notifications off reaches the backend", async ({ boot }) => {
    const app = await boot({});
    await app.page.getByRole("button", { name: "Settings" }).click();

    const sw = app.page.getByRole("switch", { name: "Routing health" });
    await sw.click();

    await expect
      .poll(() => app.lastCall("set_routing_health_notifications"))
      .toEqual({ enabled: false });
    await expect(sw).toHaveAttribute("aria-checked", "false");
  });

  test("the diagnostics switch reaches the backend", async ({ boot }) => {
    const app = await boot({});
    await app.page.getByRole("button", { name: "Settings" }).click();

    await app.page.getByRole("switch", { name: "Share diagnostic data" }).click();

    await expect.poll(() => app.lastCall("set_share_diagnostics")).toEqual({ enabled: false });
  });

  /**
   * The rule this section exists to hold: an unreadable preference must not draw
   * an Off switch. Off is a claim about the user's setting.
   */
  test("a failed preferences read shows Unavailable, not an Off switch", async ({ boot }) => {
    const app = await boot({ failures: { get_preferences: "config directory unreadable" } });
    await app.page.getByRole("button", { name: "Settings" }).click();

    await expect(app.page.getByRole("switch", { name: "Routing health" })).toHaveCount(0);
    await expect(
      app.page.getByRole("switch", { name: "Share diagnostic data" }),
    ).toHaveCount(0);
    await expect(app.page.getByRole("button", { name: "Retry" }).first()).toBeVisible();
  });

  test("a failed launch-at-login read does not spread to the preference switches", async ({
    boot,
  }) => {
    const app = await boot({ failures: { launch_at_login_status: "unsupported" } });
    await app.page.getByRole("button", { name: "Settings" }).click();

    await expect(app.page.getByRole("switch", { name: "Launch at login" })).toHaveCount(0);
    // Different command, so it keeps its switch.
    await expect(app.page.getByRole("switch", { name: "Routing health" })).toBeVisible();
  });

  test("Settings carries the sections the criteria name", async ({ boot }) => {
    const app = await boot({});
    await app.page.getByRole("button", { name: "Settings" }).click();

    for (const heading of [
      "Device",
      "Account",
      "Connection",
      "Startup",
      "Notifications",
      "Diagnostics",
      "About",
      "Help",
    ]) {
      await expect(app.page.getByRole("heading", { name: heading })).toBeVisible();
    }
  });
});

/**
 * AG-603: "What is collected opens the field list WITHOUT changing the setting."
 *
 * The read-only half is the testable half here: whether the opt-out actually
 * stops PostHog is pinned in `lib/analytics.test.ts`, since the e2e build has no
 * analytics key and the channel no-ops entirely.
 */
test.describe("new UI: what diagnostics collects", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("the list opens and changes no setting", async ({ boot }) => {
    const app = await boot({});
    await app.page.getByRole("button", { name: "Settings" }).click();

    await app.page.getByRole("button", { name: "View list" }).click();

    const dialog = app.page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText("Sent", { exact: true })).toBeVisible();
    await expect(dialog.getByText("Never sent")).toBeVisible();
    // The whole point: opening the disclosure must not touch the preference.
    expect(await app.lastCall("set_share_diagnostics")).toBeNull();
    // ...and the switch is where it was.
    await app.page.getByRole("button", { name: "Close" }).click();
    await expect(
      app.page.getByRole("switch", { name: "Share diagnostic data" }),
    ).toHaveAttribute("aria-checked", "true");
  });

  test("it names prompts and credentials as never sent", async ({ boot }) => {
    // The claims this product's reassurance rests on, so they are asserted
    // rather than left to the copy drifting.
    const app = await boot({});
    await app.page.getByRole("button", { name: "Settings" }).click();
    await app.page.getByRole("button", { name: "View list" }).click();

    const dialog = app.page.getByRole("dialog");
    await expect(dialog.getByText(/Prompts or model responses/)).toBeVisible();
    await expect(dialog.getByText(/API keys, credentials/)).toBeVisible();
  });
});
