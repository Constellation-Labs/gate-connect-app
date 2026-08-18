import { test, expect } from "./fixtures";

/**
 * The window shell's quit flow: AG-596's three outcomes, and its rule that Gate
 * Connect "does not claim cleanup completed".
 *
 * The popover already had this (`QuitConfirm`); the new shell had nothing at all,
 * so a tray Quit went unanswered there. What this covers that the unit tests
 * cannot: that the deferred quit actually reaches this window, that the dialog
 * outranks the other overlays, and that a partial teardown stops the exit.
 *
 * Opts into the new shell per-test; the suite default is the popover.
 */
const useNewUi = { gc: "gc.newUi" };

const connectedTools = [
  {
    slug: "claude-code",
    name: "Claude Code",
    upstream_provider_name: "Anthropic",
    default_upstream_url: "https://api.anthropic.com",
    requires_upstream_credential: false,
    status: { kind: "connected" as const },
  },
  {
    slug: "codex",
    name: "Codex",
    upstream_provider_name: "OpenAI",
    default_upstream_url: "https://api.openai.com",
    requires_upstream_credential: false,
    status: { kind: "connected" as const },
  },
];

test.describe("new UI quit", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("a deferred quit raises the dialog with all three outcomes", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: connectedTools,
      pendingQuitTools: ["Claude Code", "Codex"],
    });

    const dialog = app.page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Disconnect tools and quit" })).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Quit without disconnecting" })).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Cancel" })).toBeVisible();
  });

  test("disconnecting puts the tools back and then quits", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: connectedTools,
      pendingQuitTools: ["Claude Code", "Codex"],
    });

    await app.page.getByRole("button", { name: "Disconnect tools and quit" }).click();

    await expect.poll(() => app.lastCall("disconnect_tools_for_quit")).not.toBeNull();
    await expect.poll(() => app.lastCall("quit_app")).not.toBeNull();
  });

  test("quitting without disconnecting touches no config", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: connectedTools,
      pendingQuitTools: ["Claude Code"],
    });

    await app.page.getByRole("button", { name: "Quit without disconnecting" }).click();

    await expect.poll(() => app.lastCall("quit_app")).not.toBeNull();
    expect(await app.lastCall("disconnect_tools_for_quit")).toBeNull();
  });

  /**
   * The behaviour this ticket turns on. The teardown succeeds overall but leaves
   * Codex on Gate's settings, so the app must not exit while implying otherwise.
   */
  test("a tool left on Gate is named, and the app does not quit", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: connectedTools,
      pendingQuitTools: ["Claude Code", "Codex"],
      quitLeftBehind: ["Codex"],
    });

    await app.page.getByRole("button", { name: "Disconnect tools and quit" }).click();

    await expect(app.page.getByText(/Couldn’t put Codex back/)).toBeVisible();
    expect(await app.lastCall("quit_app")).toBeNull();
    // Retrying only retouches what failed; quitting stays available.
    await expect(app.page.getByRole("button", { name: "Try again" })).toBeVisible();
    await expect(app.page.getByRole("button", { name: "Quit anyway" })).toBeVisible();
  });

  test("cancelling leaves the window open with nothing changed", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: connectedTools,
      pendingQuitTools: ["Claude Code"],
    });

    await app.page.getByRole("button", { name: "Cancel" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    expect(await app.lastCall("quit_app")).toBeNull();
    expect(await app.lastCall("disconnect_tools_for_quit")).toBeNull();
  });
});
