import { test, expect } from "./fixtures";

/**
 * The tray popover (window label `tray`, Figma `Flows / Tray`, built
 * 2026-08-28): the compact quick-status surface the tray icon toggles beside
 * the full window.
 *
 * Reached by pointing the fake `getCurrentWindow()` label at "tray" - the
 * surface is picked per window, not by the `gc.newUi` shell flag, so no
 * localStorage opt-in is involved. What these pin is the seam the component
 * tests cannot see: whether the controls are connected to the backend at all.
 */
test.describe("tray popover", () => {
  test("a row switch routes its tool through the same dispatch as the rail", async ({
    boot,
  }) => {
    const app = await boot({
      windowLabel: "tray",
      // Engine on and certificate trusted, so the toggle reaches the config
      // write without raising the trust dialog first.
      proxy: { running: true, ca_trusted: true },
    });

    await app.page.getByRole("switch", { name: "Claude Code", exact: true }).click();

    await expect.poll(() => app.lastCall("connect_tool")).toMatchObject({
      slug: "claude-code",
    });
  });

  test("Expand app reveals the main window", async ({ boot }) => {
    const app = await boot({ windowLabel: "tray" });

    await app.page.getByRole("button", { name: "Expand app" }).click();

    await expect.poll(() => app.lastCall("reveal_popover")).not.toBeNull();
  });

  test("the menu's Quit goes through the tray-menu quit path", async ({ boot }) => {
    const app = await boot({ windowLabel: "tray" });

    await app.page.getByRole("button", { name: "More" }).click();
    await app.page.getByRole("menuitem", { name: "Quit Gate Connect" }).click();

    // `request_app_quit` defers to the main window's three-way dialog when
    // config-routed tools are still managed; the tray never raises its own.
    await expect.poll(() => app.lastCall("request_app_quit")).not.toBeNull();
  });

  test("signed out, it hands over instead of painting empty groups", async ({ boot }) => {
    const app = await boot({ windowLabel: "tray", account: null });

    await expect(
      app.page.getByRole("heading", { name: "Sign in to get started" }),
    ).toBeVisible();

    await app.page.getByRole("button", { name: "Open Gate Connect" }).click();
    await expect.poll(() => app.lastCall("reveal_popover")).not.toBeNull();
  });
});
