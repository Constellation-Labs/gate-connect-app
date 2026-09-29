import { test, expect } from "./fixtures";

/**
 * The window shell's quit. There is no dialog: `quit_app` puts every tool back
 * on its own settings itself (macOS and Windows) and says so in a
 * notification, so the menu's Quit goes straight to it with tools routed.
 *
 * Opts into the new shell per-test; the suite default is the popover.
 */
const useNewUi = { gc: "gc.newUi" };

const connectedTools = [
  {
    slug: "claude-code",
    name: "CLI",
    upstream_provider_name: "Anthropic",
    default_upstream_url: "https://api.anthropic.com",
    status: { kind: "connected" as const },
  },
  {
    slug: "codex",
    name: "CLI",
    upstream_provider_name: "OpenAI",
    default_upstream_url: "https://api.openai.com",
    status: { kind: "connected" as const },
  },
];

test.describe("new UI quit", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("with tools routed, the menu quits without asking", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: connectedTools,
    });

    await app.page.getByRole("button", { name: "More" }).click();
    await app.page.getByRole("menuitem", { name: "Quit Gate Connect" }).click();

    await expect.poll(() => app.lastCall("quit_app")).not.toBeNull();
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
  });
});
