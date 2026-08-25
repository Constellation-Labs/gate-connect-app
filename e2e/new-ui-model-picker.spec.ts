import { test, expect } from "./fixtures";

/**
 * Choosing which model an app runs on (AG-588).
 *
 * The choice persists in `preferences.json` on this install, keyed on tool slug.
 * It was briefly a gateway endpoint scoped to the organization; local means the
 * machine whose traffic it governs is the machine that holds it. What these specs pin is the
 * part a unit test cannot - the order of the overlays, and the two states the
 * card must not confuse:
 *
 *  - a model that is *remembered* versus one that is *served*, and
 *  - "we have not read the setting" versus "the setting is App default".
 *
 * The fake backend records consent only when moving to `gate`, exactly as the
 * real setter does, so a flow that stopped asking would fail here rather than
 * quietly start billing.
 */
const useNewUi = { gc: "gc.newUi" };

const tools = [
  {
    slug: "claude-code",
    name: "Claude Code",
    upstream_provider_name: "Anthropic",
    default_upstream_url: "https://gw.example/claude-code",
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

    // The X, not a Cancel button: single-select applies on click, so the Figma
    // gives the dialog no footer and the close control is the only exit.
    await app.page.getByRole("button", { name: "Close" }).click();
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

  test("does not re-ask once this install has accepted", async ({ boot }) => {
    // Once per install now, not once per organization: the acknowledgement is
    // stored beside the choice, and the choice is local. A second machine will
    // ask again - the trade recorded in `preferences.rs`.
    const app = await boot({
      ...base,
      toolModels: {
        catalogue,
        paidAckUnix: 1767330245,
        choices: { "claude-code": { source: "tool", model_ids: [catalogue[1].id] } },
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
        paidAckUnix: 1767330245,
        choices: { "claude-code": { source: "gate", model_ids: [catalogue[0].id] } },
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
        paidAckUnix: 1767330245,
        choices: { "claude-code": { source: "gate", model_ids: [catalogue[0].id] } },
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

});

/**
 * The picker as the Figma actually draws it (139:66117), and the multi-select
 * extension AG-590 asks for.
 *
 * The search field and provider filter are not decoration: the real catalogue
 * holds 344 models, not the eleven the frame draws, and a list that long is
 * unusable without them.
 */
test.describe("new UI model picker search and set", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  const many = [
    ...catalogue,
    { id: "openai/gpt-5", owned_by: "openai", name: "GPT-5" },
    { id: "deepseek/deepseek-v3", owned_by: "deepseek", name: "DeepSeek V3" },
  ];

  test("search narrows the list and says how many are showing", async ({ boot }) => {
    const app = await boot({ ...base, toolModels: { catalogue: many } });
    await openApp(app);
    await app.page.getByRole("radio", { name: /Gate model/ }).click();

    const dialog = app.page.getByRole("dialog");
    await expect(dialog.getByText("Showing 4 of 4 models")).toBeVisible();

    await dialog.getByRole("searchbox").fill("opus");
    await expect(dialog.getByText("Showing 1 of 4 models")).toBeVisible();
    await expect(dialog.getByRole("radio")).toHaveCount(1);
  });

  test("says so when a search matches nothing, rather than showing an empty list", async ({
    boot,
  }) => {
    const app = await boot({ ...base, toolModels: { catalogue: many } });
    await openApp(app);
    await app.page.getByRole("radio", { name: /Gate model/ }).click();

    const dialog = app.page.getByRole("dialog");
    await dialog.getByRole("searchbox").fill("nothing-matches-this");

    await expect(dialog.getByText("No model matches that search.")).toBeVisible();
    await expect(dialog.getByRole("radio")).toHaveCount(0);
  });

  test("the provider filter narrows to one vendor", async ({ boot }) => {
    const app = await boot({ ...base, toolModels: { catalogue: many } });
    await openApp(app);
    await app.page.getByRole("radio", { name: /Gate model/ }).click();

    const dialog = app.page.getByRole("dialog");
    await dialog.getByRole("combobox").selectOption("openai");

    await expect(dialog.getByText("Showing 1 of 4 models")).toBeVisible();
  });

  test("Edit set enables several models at once and states the count", async ({ boot }) => {
    const app = await boot({
      ...base,
      toolModels: {
        catalogue: many,
        paidAckUnix: 1787740800,
        choices: { "claude-code": { source: "gate", model_ids: [catalogue[0].id] } },
      },
    });
    await openApp(app);

    await app.page.getByRole("button", { name: "Edit set" }).click();
    const dialog = app.page.getByRole("dialog");
    await dialog.getByRole("checkbox", { name: "openai/gpt-5" }).click();

    await expect(dialog.getByText("2 models enabled")).toBeVisible();
    await dialog.getByRole("button", { name: "Save models" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    // The card keeps one row and says how many more, so the pane does not grow.
    await expect(app.page.getByText(/\+ 1 more enabled/)).toBeVisible();
  });

  test("refuses to remove the last enabled model", async ({ boot }) => {
    // AG-590: the final model cannot be removed without selecting another or
    // returning to App default. The dialog holds the line; the pane's radio is
    // the other way out.
    const app = await boot({
      ...base,
      toolModels: {
        catalogue: many,
        paidAckUnix: 1787740800,
        choices: { "claude-code": { source: "gate", model_ids: [catalogue[0].id] } },
      },
    });
    await openApp(app);

    await app.page.getByRole("button", { name: "Edit set" }).click();
    const dialog = app.page.getByRole("dialog");
    const only = dialog.getByRole("checkbox", { name: catalogue[0].id });

    // Asserted as state rather than by clicking: the row carries
    // `aria-disabled`, so a click is refused before it reaches the handler -
    // which is the point, and is also why Playwright will not click it.
    await expect(only).toHaveAttribute("aria-checked", "true");
    await expect(only).toHaveAttribute("aria-disabled", "true");
    await expect(only).toHaveAttribute("title", /at least one model/);
    await expect(dialog.getByText("1 model enabled")).toBeVisible();

    // And the way out the ticket names still works: enable a second, and the
    // first unlocks.
    await dialog.getByRole("checkbox", { name: "openai/gpt-5" }).click();
    await expect(only).not.toHaveAttribute("aria-disabled", "true");
  });
});
