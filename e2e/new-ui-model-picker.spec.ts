import { test, expect } from "./fixtures";

/**
 * Choosing which model an app runs on (AG-588).
 *
 * There is a backend now: preferences persist per organization on the gateway,
 * keyed on the platform id it derives per request. What these specs pin is the
 * part a unit test cannot - the order of the overlays, and the two states the
 * card must not confuse:
 *
 *  - a model that is *remembered* versus one that is *served*, and
 *  - "we have not read the setting" versus "the setting is App default".
 *
 * The fake backend enforces the gateway's own refusals (a first paid selection
 * needs an acknowledgement; `gate` needs a model), so a flow that stopped asking
 * would fail here rather than quietly start billing.
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

/** Two real ids from the staging catalogue. Not invented: a fabricated model
 *  list is the thing the picker's empty state exists to avoid. */
const catalogue = [
  { id: "anthropic/claude-opus-5", owned_by: "anthropic", name: "Claude Opus 5" },
  { id: "anthropic/claude-sonnet-5", owned_by: "anthropic", name: "Claude Sonnet 5" },
];

const base = { proxy: { running: true, ca_trusted: true }, tools };

/** Open one app's pane, which is where model selection lives. */
async function openApp(app: { page: import("@playwright/test").Page }) {
  await app.page.getByRole("button", { name: "Claude Code" }).first().click();
}

test.describe("new UI model picker", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("says no model is chosen before one is", async ({ boot }) => {
    const app = await boot(base);
    await openApp(app);

    await expect(app.page.getByText("No Gate model chosen yet")).toBeVisible();
    // No dead control: there is nothing to change yet.
    await expect(app.page.getByRole("button", { name: "Change model" })).toHaveCount(0);
  });

  test("the picker says it has nothing to offer rather than inventing models", async ({ boot }) => {
    // The design draws eleven `gate/...` ids. A gateway with no platform
    // provider accounts genuinely offers none, and saying so is the honest
    // answer - shipping the drawn ids would be a fabricated catalogue.
    const app = await boot(base);
    await openApp(app);

    await app.page.getByRole("radio", { name: /Gate model/ }).click();

    await expect(app.page.getByText("No models to choose from yet")).toBeVisible();
    await expect(app.page.getByRole("dialog").getByRole("radio")).toHaveCount(0);

    await app.page.getByRole("button", { name: "Cancel" }).click();
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
  });

  test("choosing Gate model with nothing chosen picks a model first", async ({ boot }) => {
    // Gate cannot serve a model nobody selected - the gateway refuses it and its
    // schema refuses it - so the picker has to come before the switch.
    const app = await boot({ ...base, toolModels: { catalogue } });
    await openApp(app);

    await app.page.getByRole("radio", { name: /Gate model/ }).click();

    await expect(app.page.getByRole("heading", { name: "Choose a Gate model" })).toBeVisible();
    await expect(app.page.getByRole("dialog").getByRole("radio")).toHaveCount(2);
  });

  test("picking a model then confirming hands routing to Gate", async ({ boot }) => {
    const app = await boot({ ...base, toolModels: { catalogue } });
    await openApp(app);

    await app.page.getByRole("radio", { name: /Gate model/ }).click();
    await app.page.getByRole("dialog").getByRole("radio", { name: catalogue[0].id }).click();

    // The billing confirmation, because this organization has never accepted it.
    await expect(
      app.page.getByRole("heading", { name: /Use a Gate model for Claude Code\?/ }),
    ).toBeVisible();
    await app.page.getByRole("button", { name: "Use Gate credits" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    await expect(app.page.getByRole("radio", { name: /Gate model/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await expect(app.page.getByText(catalogue[0].id)).toBeVisible();
    // Served, so no "not in use" qualifier.
    await expect(app.page.getByText(/not in use/)).toHaveCount(0);
  });

  test("declining the billing keeps the model without serving it", async ({ boot }) => {
    // They declined the charge, not the choice. Remembering it is why a
    // preference may name a model while its source is still "tool".
    const app = await boot({ ...base, toolModels: { catalogue } });
    await openApp(app);

    await app.page.getByRole("radio", { name: /Gate model/ }).click();
    await app.page.getByRole("dialog").getByRole("radio", { name: catalogue[0].id }).click();
    await app.page.getByRole("button", { name: "Keep App default" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    await expect(app.page.getByRole("radio", { name: /App default/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    // Kept, and marked as not in force - the distinction the card exists to make.
    await expect(app.page.getByText(catalogue[0].id)).toBeVisible();
    await expect(app.page.getByText(/not in use/)).toBeVisible();
  });

  test("a second app does not re-ask once the organization has accepted", async ({ boot }) => {
    // AG-588 words the confirmation as once per organization, so an org that has
    // already accepted goes straight through.
    const app = await boot({
      ...base,
      toolModels: {
        catalogue,
        firstPaidAckAt: "2026-01-02T03:04:05.000Z",
        preferences: [{ platformId: "claude-code", source: "tool", modelIds: [catalogue[1].id] }],
      },
    });
    await openApp(app);

    await app.page.getByRole("radio", { name: /Gate model/ }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    await expect(app.page.getByRole("radio", { name: /Gate model/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );
  });

  test("Change model swaps the served model directly once Gate is serving", async ({ boot }) => {
    const app = await boot({
      ...base,
      toolModels: {
        catalogue,
        firstPaidAckAt: "2026-01-02T03:04:05.000Z",
        preferences: [{ platformId: "claude-code", source: "gate", modelIds: [catalogue[0].id] }],
      },
    });
    await openApp(app);

    await app.page.getByRole("button", { name: "Change model" }).click();
    await app.page.getByRole("dialog").getByRole("radio", { name: catalogue[1].id }).click();

    // No second confirmation: billing was accepted when the switch was made.
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    await expect(app.page.getByText(catalogue[1].id)).toBeVisible();
    await expect(app.page.getByRole("radio", { name: /Gate model/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );
  });

  test("switching back to App default keeps the model, unserved", async ({ boot }) => {
    const app = await boot({
      ...base,
      toolModels: {
        catalogue,
        firstPaidAckAt: "2026-01-02T03:04:05.000Z",
        preferences: [{ platformId: "claude-code", source: "gate", modelIds: [catalogue[0].id] }],
      },
    });
    await openApp(app);

    await app.page.getByRole("radio", { name: /App default/ }).click();

    // Switching off is not confirmed - nothing is being spent.
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    await expect(app.page.getByRole("radio", { name: /App default/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await expect(app.page.getByText(catalogue[0].id)).toBeVisible();
    await expect(app.page.getByText(/not in use/)).toBeVisible();
  });

  test("offers no choice at all when the setting could not be read", async ({ boot }) => {
    // The principle 2 case. An org that HAD switched to a Gate model must not see
    // App default selected because a read failed - clicking Gate model would look
    // like a change when it is the first thing anyone said.
    const app = await boot({
      ...base,
      failures: { tool_model_preferences: '{"code":"offline","message":"no route to host"}' },
    });
    await openApp(app);

    for (const name of [/App default/, /Gate model/]) {
      const radio = app.page.getByRole("radio", { name });
      await expect(radio).toHaveAttribute("aria-checked", "false");
      await expect(radio).toBeDisabled();
    }
    await expect(app.page.getByText(/could not read this app's model setting/i)).toBeVisible();
  });

  test("withholds the control for an app the gateway cannot identify", async ({ boot }) => {
    // Hermes is the real case: nothing in the gateway's registry detects it, so
    // its requests are unattributed and no preference could ever be applied.
    const app = await boot({
      ...base,
      tools: [{ ...tools[0], platform_id: null }],
    });
    await openApp(app);

    await expect(
      app.page.getByText(/cannot identify Claude Code on a request/i),
    ).toBeVisible();
    await expect(app.page.getByRole("radio", { name: /Gate model/ })).toHaveCount(0);
  });
});
