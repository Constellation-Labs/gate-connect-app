import { test, expect } from "./fixtures";

/**
 * The routing verdict on screen: AG-562's rule that a status line reports
 * verified behaviour rather than what a config file says.
 *
 * What this covers that `lib/verdict.test.ts` cannot: that the sidebar actually
 * asks the backend for a verdict rather than deriving the line from
 * `Tool.status`, and that the switch stays on intent while the line moves. Those
 * two are the same bug `lib/groups.ts` documents, one level down.
 *
 * Opts into the new shell per-test, like the rest of the `new-ui-*` specs; the
 * suite default is the popover (`VITE_NEW_UI=0`).
 */
const useNewUi = { gc: "gc.newUi" };

/**
 * Where a verdict's reason is asserted, and why it is not the rail.
 *
 * A 250px row cannot fit "Not protected - Configuration update failed" and
 * truncates the reason mid-word, so the rail prints the coloured phrase alone
 * and the app pane's header carries the reason in full. A spec that wants the
 * phrase reads the row; one that wants the reason opens the pane.
 */
async function openApp(app: { page: import("@playwright/test").Page }, name: string) {
  await app.page.getByRole("button", { name }).first().click();
}

const connectedCodex = {
  slug: "codex",
  name: "Codex",
  upstream_provider_name: "OpenAI",
  default_upstream_url: "https://gw.example/codex",
  status: { kind: "connected" as const },
};

test.describe("new UI routing verdict", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("a connected app reads Protected only once the sweep confirms it", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [connectedCodex],
    });

    // `exact`, because the sidebar's own eyebrow reads "Protected apps".
    await expect(app.page.getByText("Protected", { exact: true })).toBeVisible();
    // The sweep is a real command, not a derivation from the tool list.
    await expect.poll(() => app.lastCall("routing_verdicts")).not.toBeNull();
  });

  /**
   * The behaviour the whole ticket turns on. The config still says connected -
   * nothing was written - but with the engine down the relay cannot carry
   * anything, so the row must not claim protection.
   */
  test("a connected app whose relay is down reads Not protected, and says why in the pane", async ({
    boot,
  }) => {
    const app = await boot({
      proxy: { running: false, ca_trusted: true },
      tools: [connectedCodex],
    });

    // The row's job is to stop claiming protection. Its reason is the pane's.
    await expect(app.page.getByText("Not protected").first()).toBeVisible();
    await expect(app.page.getByText("Protected", { exact: true })).toHaveCount(0);

    await openApp(app, "Codex");
    await expect(app.page.getByText("Connection problem")).toBeVisible();
  });

  /**
   * Intent and observation stay separate. A tool that cannot be verified is
   * still one the user asked to route, so its switch must read on - otherwise
   * clicking it turns off the setting they were trying to turn on.
   */
  test("a failing verdict does not move the switch", async ({ boot }) => {
    const app = await boot({
      proxy: { running: false, ca_trusted: true },
      tools: [connectedCodex],
    });

    // Waits for the sweep to have landed before reading the switch.
    await expect(app.page.getByText("Not protected").first()).toBeVisible();
    const sidebarSwitch = app.page.getByRole("switch", { name: "Codex" }).first();
    await expect(sidebarSwitch).toHaveAttribute("aria-checked", "true");
  });

  test("a tool whose process predates the routing change is told to reopen, in the pane", async ({
    boot,
  }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [connectedCodex],
      staleAgents: 1,
    });

    await openApp(app, "Codex");
    await expect(app.page.getByText("Reopen required")).toBeVisible();
  });

  test("a disconnected app reads Not routed", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [{ ...connectedCodex, status: { kind: "detected" as const } }],
    });

    // Scoped to the Codex row: the catalog's proxy domains are rail rows too
    // now, and while disabled they read "Not routed" as well.
    const row = app.page
      .getByRole("listitem")
      .filter({ has: app.page.getByRole("switch", { name: "Codex", exact: true }) });
    await expect(row.getByText("Not routed")).toBeVisible();
  });

  test("a drifted config keeps the design's own phrase", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [
        {
          ...connectedCodex,
          status: { kind: "drifted" as const, reason: "API base URL: https://api.openai.com/v1" },
        },
      ],
    });

    await expect(app.page.getByText("Config drifted")).toBeVisible();
  });
});
