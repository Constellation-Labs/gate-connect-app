import { test, expect } from "./fixtures";

/**
 * Dashboard destinations follow the gateway this install actually talks to
 * (`lib/dashboard.ts`, 2026-09-07).
 *
 * The bug: `GATE_DASHBOARD_URL` and its three siblings were constants pinned to
 * `app.constellationgate.ai`, while the gateway is switchable at build time
 * (`VITE_GATE_DEFAULT_BASE_URL`) and at runtime (Settings -> Dev mode). So every
 * dashboard action on a staging install opened production - the user's traffic
 * in one environment and their key management in another, with nothing on
 * screen saying so. `pnpm app:local` defaults to staging, which made that the
 * normal developer state rather than an edge case.
 *
 * What these cover that the unit tests cannot: that the shells actually thread
 * the account's gateway into the opener, rather than deriving a correct URL
 * nobody passes on. Both were caught this way - the tray closed over a `null`
 * account from first render, which would have shipped as "this gateway has no
 * dashboard" on every click.
 *
 * The tray's own two cases live in `new-ui-tray.spec.ts`, beside the rest of
 * that surface.
 */
test.describe("dashboard links follow the active gateway", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => localStorage.setItem("gc.newUi", "1"));
  });

  test("the topnav opens the dashboard for the account's own environment", async ({
    boot,
  }) => {
    const app = await boot({
      account: { gateway_base_url: "https://gateway-staging.constellationgate.ai" },
    });

    await app.page.getByRole("button", { name: "More" }).click();
    await app.page.getByRole("menuitem", { name: "Visit dashboard" }).click();

    await expect.poll(() => app.lastCall("plugin:opener|open_url")).toMatchObject({
      url: "https://app-staging.constellationgate.ai/",
    });
  });

  test("Contact support opens the dashboard page the support widget lives on", async ({
    boot,
  }) => {
    // AG-598. Support has no route of its own: the control is a floating action
    // button in the dashboard's bottom-right corner, so Overview is the
    // destination (settled 2026-09-07, replacing an address that 404'd).
    const app = await boot({
      account: { gateway_base_url: "https://gateway-staging.constellationgate.ai" },
    });

    await app.page.getByRole("button", { name: "More" }).click();
    await app.page.getByRole("menuitem", { name: "Contact support" }).click();

    await expect.poll(() => app.lastCall("plugin:opener|open_url")).toMatchObject({
      url: "https://app-staging.constellationgate.ai/overview",
    });
  });

  test("production stays production", async ({ boot }) => {
    // The mapping is a rewrite of one host label, so it is worth pinning that
    // it is identity-shaped for the environment everybody ships to.
    const app = await boot({
      account: { gateway_base_url: "https://gateway.constellationgate.ai" },
    });

    await app.page.getByRole("button", { name: "More" }).click();
    await app.page.getByRole("menuitem", { name: "Visit dashboard" }).click();

    await expect.poll(() => app.lastCall("plugin:opener|open_url")).toMatchObject({
      url: "https://app.constellationgate.ai/",
    });
  });

  test("a gateway with no dashboard reports it and opens nothing", async ({ boot }) => {
    // A local dev gateway. Opening a guessed URL would be the same silent
    // wrong-environment failure the constants had; the banner is dismissible
    // and navigates nowhere, which is what AG-598 requires of a failed link.
    const app = await boot({
      account: { gateway_base_url: "http://localhost:3000" },
    });

    await app.page.getByRole("button", { name: "More" }).click();
    await app.page.getByRole("menuitem", { name: "Visit dashboard" }).click();

    await expect(app.page.getByText("This gateway has no dashboard")).toBeVisible();
    expect(await app.lastCall("plugin:opener|open_url")).toBeNull();
  });
});
