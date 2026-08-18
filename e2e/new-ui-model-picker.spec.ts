import { test, expect } from "./fixtures";

/**
 * Choosing which model an app runs on.
 *
 * There is no backend for model selection - no command, no Rust - so what these
 * tests pin is the part that exists: which overlay opens, that spending credits
 * is confirmed rather than taken on a radio click, and that the picker is honest
 * about having nothing to offer yet.
 */
const useNewUi = { gc: "gc.newUi" };

const tools = [
  {
    slug: "claude-code",
    name: "Claude Code",
    upstream_provider_name: "Anthropic",
    default_upstream_url: "https://gw.example/claude-code",
    requires_upstream_credential: false,
    status: { kind: "connected" as const },
  },
];

/** Open one app's pane, which is where model selection lives. */
async function openApp(app: { page: import("@playwright/test").Page }) {
  await app.page.getByRole("button", { name: "Claude Code" }).first().click();
}

test.describe("new UI model picker", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("Change model opens the picker", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true }, tools });
    await openApp(app);

    await app.page.getByRole("button", { name: "Change model" }).click();

    await expect(app.page.getByRole("heading", { name: "Choose a Gate model" })).toBeVisible();
  });

  test("the picker says it has nothing to offer rather than inventing models", async ({
    boot,
  }) => {
    // The design draws eleven `gate/...` ids. Shipping those before a gateway
    // reports them would put a fabricated catalogue in front of the user.
    const app = await boot({ proxy: { running: true, ca_trusted: true }, tools });
    await openApp(app);

    await app.page.getByRole("button", { name: "Change model" }).click();

    await expect(app.page.getByText("No models to choose from yet")).toBeVisible();
    // Inside the dialog: the pane behind it has its own App-default/Gate radios.
    await expect(app.page.getByRole("dialog").getByRole("radio")).toHaveCount(0);

    await app.page.getByRole("button", { name: "Cancel" }).click();
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
  });

  test("switching to a Gate model is confirmed, because it spends credits", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true }, tools });
    await openApp(app);

    await app.page.getByRole("radio", { name: /Gate model/ }).click();

    await expect(
      app.page.getByRole("heading", { name: /Use a Gate model for Claude Code\?/ }),
    ).toBeVisible();
  });

  test("keeping the app default leaves the choice where it was", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true }, tools });
    await openApp(app);

    await app.page.getByRole("radio", { name: /Gate model/ }).click();
    await app.page.getByRole("button", { name: "Keep App default" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    await expect(app.page.getByRole("radio", { name: /App default/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );
  });

  test("confirming moves the choice to the Gate model", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true }, tools });
    await openApp(app);

    await app.page.getByRole("radio", { name: /Gate model/ }).click();
    await app.page.getByRole("button", { name: "Use Gate credits" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    await expect(app.page.getByRole("radio", { name: /Gate model/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );
  });
});
