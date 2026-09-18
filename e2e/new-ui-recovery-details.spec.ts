import { test, expect } from "./fixtures";

/**
 * What the window does with the tray's "Review details" (AG-890).
 *
 * `new-ui-tray.spec.ts` covers the tray half: the press reaches
 * `request_recovery_details` rather than a bare reveal. This is the other end
 * of that handover, which nothing exercised - and which is where the ticket's
 * symptom was, the popover's link landing on Settings.
 */
const useNewUi = { gc: "gc.newUi" };

const pendingRestore = {
  providers: [],
  tools: [{ slug: "opencode", name: "OpenCode" }],
};

test.describe("the tray's Review details, in the window", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("opens the routing details for the unfinished run", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      pendingRestore,
    });

    await app.emit("recovery-details-requested");

    await expect(
      app.page.getByRole("heading", { name: "What happened to routing" }),
    ).toBeVisible({ timeout: 10_000 });
  });

  /**
   * The ticket's symptom. `recovery_summary` failing used to be indistinguishable
   * from "nothing pending", and the `else` branch navigated to Settings - a page
   * with nothing on it about routing recovery, reached with no word about why.
   *
   * Refusing silently is the documented behaviour of this listener's other two
   * refusal paths, and the tray's own card is still there when the user gets
   * back to it.
   */
  test("does not answer with Settings when it has nothing to show", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      pendingRestore,
      failures: { recovery_summary: "could not read the restore journal" },
    });

    await app.emit("recovery-details-requested");

    await expect(
      app.page.getByRole("heading", { name: "Diagnostics" }),
    ).toHaveCount(0);
    await expect(
      app.page.getByRole("heading", { name: "What happened to routing" }),
    ).toHaveCount(0);
  });

  /**
   * A read that fails must not wipe a summary this window already holds. That
   * asymmetry is what let the tray keep showing an unfinished run the window
   * had silently forgotten, which is the first half of AG-890.
   */
  test("keeps the run it already read when a later read fails", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      pendingRestore,
    });

    // Let the first successful read land.
    await app.emit("recovery-details-requested");
    await expect(
      app.page.getByRole("heading", { name: "What happened to routing" }),
    ).toBeVisible({ timeout: 10_000 });
    await app.page.getByRole("button", { name: "Close" }).click();

    // Now every further read fails, and something re-reads.
    await app.patch({ failures: { recovery_summary: "journal unreadable" } });
    await app.emit("proxy-state-changed");
    await app.emit("recovery-details-requested");

    await expect(
      app.page.getByRole("heading", { name: "What happened to routing" }),
    ).toBeVisible({ timeout: 10_000 });
  });
});
