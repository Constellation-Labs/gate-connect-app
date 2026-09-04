import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";

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
 *  list is the thing the picker's empty state exists to avoid.
 *
 *  They carry `tool-use` because real catalogue rows do. A row with no tags at
 *  all is a terse gateway rather than a typical one, and leaving these bare made
 *  every model read as untagged, which is a different test than the ones below
 *  mean to be running. */
const catalogue = [
  { id: "anthropic/claude-opus-5", owned_by: "anthropic", name: "Claude Opus 5", tags: ["tool-use"] },
  {
    id: "anthropic/claude-sonnet-5",
    owned_by: "anthropic",
    name: "Claude Sonnet 5",
    tags: ["tool-use"],
  },
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

    // Under App default there is no current Gate model to report, so the section
    // is absent rather than empty - and with it the control that would change a
    // model this app is not using.
    await expect(app.page.getByText("Current Gate model")).toHaveCount(0);
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
    await expect(app.page.getByRole("dialog").getByRole("checkbox")).toHaveCount(0);

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
    await expect(app.page.getByRole("dialog").getByRole("checkbox")).toHaveCount(2);
  });

  test("picking a model then confirming hands routing to Gate", async ({ boot }) => {
    const app = await boot({ ...base, toolModels: { catalogue } });
    await openApp(app);

    await app.page.getByRole("radio", { name: /Gate model/ }).click();
    await app.page.getByRole("dialog").getByRole("checkbox", { name: catalogue[0].id }).click();
    await app.page.getByRole("button", { name: "Save models" }).click();

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
    // Served, so the section reports it.
    await expect(app.page.getByText("Current Gate model")).toBeVisible();
    await expect(app.page.getByText(catalogue[0].id, { exact: true })).toBeVisible();
  });

  test("declining the billing keeps the model without serving it", async ({ boot }) => {
    // They declined the charge, not the choice. Remembering it is why a
    // preference may name a model while its source is still "tool".
    const app = await boot({ ...base, toolModels: { catalogue } });
    await openApp(app);

    await app.page.getByRole("radio", { name: /Gate model/ }).click();
    await app.page.getByRole("dialog").getByRole("checkbox", { name: catalogue[0].id }).click();
    await app.page.getByRole("button", { name: "Save models" }).click();
    await app.page.getByRole("button", { name: "Keep App default" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    await expect(app.page.getByRole("radio", { name: /App default/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    // Kept, and named by the radio that would put it to use. The "Current Gate
    // model" section stays away: nothing about it is current under App default.
    await expect(app.page.getByText(`Use ${catalogue[0].id}`)).toBeVisible();
    await expect(app.page.getByText("Current Gate model")).toHaveCount(0);
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
    const swap = app.page.getByRole("dialog");
    await swap.getByRole("checkbox", { name: catalogue[1].id }).click();
    await swap.getByRole("checkbox", { name: catalogue[0].id }).click();
    await swap.getByRole("button", { name: "Save models" }).click();

    // No second confirmation: billing was accepted when the switch was made.
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    await expect(app.page.getByText(catalogue[1].id, { exact: true })).toBeVisible();
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
    // Remembered, and still named - by the radio, not by a section claiming it is
    // current.
    await expect(app.page.getByText(`Use ${catalogue[0].id}`)).toBeVisible();
    await expect(app.page.getByText("Current Gate model")).toHaveCount(0);
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
    { id: "openai/gpt-5", owned_by: "openai", name: "GPT-5", tags: ["tool-use"] },
    { id: "deepseek/deepseek-v3", owned_by: "deepseek", name: "DeepSeek V3", tags: ["tool-use"] },
  ];

  test("search narrows the list and says how many are showing", async ({ boot }) => {
    const app = await boot({ ...base, toolModels: { catalogue: many } });
    await openApp(app);
    await app.page.getByRole("radio", { name: /Gate model/ }).click();

    const dialog = app.page.getByRole("dialog");
    await expect(dialog.getByText("Showing 4 of 4 models")).toBeVisible();

    await dialog.getByRole("searchbox").fill("opus");
    await expect(dialog.getByText("Showing 1 of 4 models")).toBeVisible();
    await expect(dialog.getByRole("checkbox")).toHaveCount(1);
  });

  test("counts the set as it is picked, beside the count of what is shown", async ({ boot }) => {
    // The two numbers answer different questions - how much of the catalogue is
    // on screen, and how much of it you have chosen - so the frame puts them at
    // either end of the same line and this checks they move independently.
    const app = await boot({ ...base, toolModels: { catalogue: many } });
    await openApp(app);
    await app.page.getByRole("radio", { name: /Gate model/ }).click();

    const dialog = app.page.getByRole("dialog");
    // Nothing picked yet, so there is nothing to unselect and the control is away.
    await expect(dialog.getByRole("button", { name: /Unselect all/ })).toHaveCount(0);

    await dialog.getByRole("checkbox", { name: many[0].id }).click();
    await expect(dialog.getByRole("button", { name: "Unselect all (1)" })).toBeVisible();

    await dialog.getByRole("checkbox", { name: many[1].id }).click();
    await expect(dialog.getByRole("button", { name: "Unselect all (2)" })).toBeVisible();
    // Narrowing what is shown does not change what is chosen.
    await dialog.getByRole("searchbox").fill("opus");
    await expect(dialog.getByRole("button", { name: "Unselect all (2)" })).toBeVisible();
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
    await expect(dialog.getByRole("checkbox")).toHaveCount(0);
  });

  test("the provider filter narrows to one vendor", async ({ boot }) => {
    const app = await boot({ ...base, toolModels: { catalogue: many } });
    await openApp(app);
    await app.page.getByRole("radio", { name: /Gate model/ }).click();

    const dialog = app.page.getByRole("dialog");
    await dialog.getByRole("combobox").selectOption("openai");

    await expect(dialog.getByText("Showing 1 of 4 models")).toBeVisible();
  });

  test("Change model enables several models at once and states the count", async ({ boot }) => {
    const app = await boot({
      ...base,
      toolModels: {
        catalogue: many,
        paidAckUnix: 1787740800,
        choices: { "claude-code": { source: "gate", model_ids: [catalogue[0].id] } },
      },
    });
    await openApp(app);

    await app.page.getByRole("button", { name: "Change model" }).click();
    const dialog = app.page.getByRole("dialog");
    await dialog.getByRole("checkbox", { name: "openai/gpt-5" }).click();

    await expect(dialog.getByRole("button", { name: "Unselect all (2)" })).toBeVisible();
    await dialog.getByRole("button", { name: "Save models" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    // The card keeps the single row Figma 228:89517 draws; only the heading
    // turns plural, which is the minimum that stops it naming one model when
    // two are enabled.
    await expect(app.page.getByText("Current Gate models")).toBeVisible();
  });

  test("refuses to save an empty set, which is where the last model is held", async ({ boot }) => {
    // AG-590: the final model cannot be removed without selecting another or
    // returning to App default. The line is held on Save rather than on the row -
    // the row has to be clearable for "Unselect all" to mean anything, and what
    // the ticket protects is the state that gets written, not the draft.
    const app = await boot({
      ...base,
      toolModels: {
        catalogue: many,
        paidAckUnix: 1787740800,
        choices: { "claude-code": { source: "gate", model_ids: [catalogue[0].id] } },
      },
    });
    await openApp(app);

    await app.page.getByRole("button", { name: "Change model" }).click();
    const dialog = app.page.getByRole("dialog");
    const only = dialog.getByRole("checkbox", { name: catalogue[0].id });

    await expect(only).toHaveAttribute("aria-checked", "true");
    await expect(dialog.getByRole("button", { name: "Unselect all (1)" })).toBeVisible();

    // The row clears, and the dialog neither pretends the set is fine nor leaves
    // a disabled button with nothing beside it.
    await only.click();
    await expect(only).toHaveAttribute("aria-checked", "false");
    await expect(dialog.getByText("No models enabled")).toBeVisible();
    await expect(dialog.getByText(/needs at least one model/)).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Save models" })).toBeDisabled();

    // Nothing was written: the stored set is what it was until Save says otherwise.
    await dialog.getByRole("checkbox", { name: "openai/gpt-5" }).click();
    await expect(dialog.getByRole("button", { name: "Save models" })).toBeEnabled();
  });

  test("Unselect all empties the draft without touching the stored set", async ({ boot }) => {
    const app = await boot({
      ...base,
      toolModels: {
        catalogue: many,
        paidAckUnix: 1787740800,
        choices: { "claude-code": { source: "gate", model_ids: [catalogue[0].id] } },
      },
    });
    await openApp(app);

    await app.page.getByRole("button", { name: "Change model" }).click();
    const dialog = app.page.getByRole("dialog");
    await dialog.getByRole("checkbox", { name: "openai/gpt-5" }).click();
    await expect(dialog.getByRole("button", { name: "Unselect all (2)" })).toBeVisible();

    await dialog.getByRole("button", { name: "Unselect all (2)" }).click();
    await expect(dialog.getByRole("button", { name: /Unselect all/ })).toHaveCount(0);
    await expect(dialog.getByRole("button", { name: "Save models" })).toBeDisabled();

    // Cancel is a real cancel: the pane still names the model that was stored.
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(app.page.getByText(catalogue[0].id, { exact: true })).toBeVisible();
  });
});

/**
 * The credits line on the model card (Figma 228:89517).
 *
 * The number sits beside the control that starts spending it, so the three
 * states it can be in have to stay apart: a real balance, a balance nobody
 * reported, and PAYG being switched off entirely.
 */
test.describe("new UI model card credits", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("shows the balance the way the design words it", async ({ boot }) => {
    const app = await boot({
      ...base,
      toolModels: {
        credits: {
          plan: "pro",
          paygEnabled: true,
          balanceCents: 1025,
          lowBalanceThresholdCents: 500,
          autoTopupArmed: false,
        },
      },
    });
    await openApp(app);

    await expect(app.page.getByText("$10.25 available")).toBeVisible();
  });

  test("reads N/A when no balance was reported, not $0.00", async ({ boot }) => {
    // The default fixture reports none. Printing zero here would tell a funded
    // org their tools are about to stop.
    const app = await boot(base);
    await openApp(app);

    await expect(app.page.getByText("N/A")).toBeVisible();
    await expect(app.page.getByText("$0.00 available")).toHaveCount(0);
  });

  test("says PAYG is off rather than showing money that cannot be spent here", async ({ boot }) => {
    const app = await boot({
      ...base,
      toolModels: {
        credits: {
          plan: "pro",
          paygEnabled: false,
          balanceCents: 4200,
          lowBalanceThresholdCents: 500,
          autoTopupArmed: false,
        },
      },
    });
    await openApp(app);

    await expect(app.page.getByText("Not enabled")).toBeVisible();
    await expect(app.page.getByText("$42.00 available")).toHaveCount(0);
  });
});

/**
 * The confirmation dialog (Figma 130:48278), and what it does with a set.
 *
 * This is where someone agrees to be billed, so what it lists is the whole
 * commitment - not the first model with the rest summarised somewhere else.
 */
test.describe("new UI Gate model confirmation", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("names the single model with its vendor, as the frame draws it", async ({ boot }) => {
    const app = await boot({ ...base, toolModels: { catalogue } });
    await openApp(app);

    await app.page.getByRole("radio", { name: /Gate model/ }).click();
    const dialog = app.page.getByRole("dialog");
    await dialog.getByRole("checkbox", { name: catalogue[0].id }).click();
    await dialog.getByRole("button", { name: "Save models" }).click();

    const confirm = app.page.getByRole("dialog");
    await expect(confirm.getByText("anthropic", { exact: true })).toBeVisible();
    await expect(confirm.getByText(catalogue[0].id)).toBeVisible();
    // Exact: the subtitle also says "PAYG", and this is asserting the pill.
    await expect(confirm.getByText("PAYG", { exact: true })).toBeVisible();
  });

  test("lists every model when several are enabled", async ({ boot }) => {
    // The charge covers all of them, so all of them are stated before it is
    // accepted - AG-590. An "and 2 others" would not be stating them.
    const app = await boot({ ...base, toolModels: { catalogue } });
    await openApp(app);

    await app.page.getByRole("radio", { name: /Gate model/ }).click();
    const dialog = app.page.getByRole("dialog");
    await dialog.getByRole("checkbox", { name: catalogue[0].id }).click();
    await dialog.getByRole("checkbox", { name: catalogue[1].id }).click();
    await dialog.getByRole("button", { name: "Save models" }).click();

    const confirm = app.page.getByRole("dialog");
    await expect(confirm.getByText(catalogue[0].id)).toBeVisible();
    await expect(confirm.getByText(catalogue[1].id)).toBeVisible();
    // The set replaced the old split presentation entirely.
    await expect(confirm.getByText(/Also enabled/)).toHaveCount(0);
  });

  test("says the tool's own preference is untouched", async ({ boot }) => {
    const app = await boot({ ...base, toolModels: { catalogue } });
    await openApp(app);

    await app.page.getByRole("radio", { name: /Gate model/ }).click();
    const dialog = app.page.getByRole("dialog");
    await dialog.getByRole("checkbox", { name: catalogue[0].id }).click();
    await dialog.getByRole("button", { name: "Save models" }).click();

    await expect(
      app.page.getByText(/own model preference is not changed/),
    ).toBeVisible();
  });
});

/**
 * AG-592: a Gate model that stops working says so, and can be recovered from.
 *
 * The catalogue is the definition of "available" - `/v1/models` returns only
 * what the gateway will actually serve - so a chosen id missing from it is
 * precisely a model Gate can no longer serve.
 */
test.describe("new UI model needs attention", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  const funded = {
    plan: "pro",
    paygEnabled: true,
    balanceCents: 1025,
    lowBalanceThresholdCents: 500,
    autoTopupArmed: false,
  };

  test("highlights a chosen model the catalogue no longer offers", async ({ boot }) => {
    const app = await boot({
      ...base,
      toolModels: {
        catalogue,
        credits: funded,
        paidAckUnix: 1787740800,
        choices: { "claude-code": { source: "gate", model_ids: ["anthropic/retired-model"] } },
      },
    });
    await openApp(app);

    await expect(app.page.getByText(/no longer available from Gate/)).toBeVisible();
    // The rule the ticket is emphatic about.
    await expect(app.page.getByText(/will not pick a replacement/)).toBeVisible();
  });

  test("stays quiet when one chosen model is still servable", async ({ boot }) => {
    // Gate uses a model the user chose, which is not a substitution.
    const app = await boot({
      ...base,
      toolModels: {
        catalogue,
        credits: funded,
        paidAckUnix: 1787740800,
        choices: {
          "claude-code": { source: "gate", model_ids: ["anthropic/retired-model", catalogue[0].id] },
        },
      },
    });
    await openApp(app);

    await expect(app.page.getByText(/no longer available from Gate/)).toHaveCount(0);
  });

  test("says nothing about a model remembered under App default", async ({ boot }) => {
    const app = await boot({
      ...base,
      toolModels: {
        catalogue,
        credits: funded,
        choices: { "claude-code": { source: "tool", model_ids: ["anthropic/retired-model"] } },
      },
    });
    await openApp(app);

    await expect(app.page.getByText(/no longer available/)).toHaveCount(0);
  });

  test("highlights an empty balance", async ({ boot }) => {
    const app = await boot({
      ...base,
      toolModels: {
        catalogue,
        credits: { ...funded, balanceCents: 0 },
        paidAckUnix: 1787740800,
        choices: { "claude-code": { source: "gate", model_ids: [catalogue[0].id] } },
      },
    });
    await openApp(app);

    await expect(app.page.getByText(/no Gate credits left/)).toBeVisible();
  });

  test("lets an unavailable model be removed, which nothing else could", async ({ boot }) => {
    // A model absent from the catalogue renders no row, so without listing it
    // there is no checkbox to clear and no way out of the selection.
    const app = await boot({
      ...base,
      toolModels: {
        catalogue,
        credits: funded,
        paidAckUnix: 1787740800,
        choices: {
          "claude-code": { source: "gate", model_ids: ["anthropic/retired-model", catalogue[0].id] },
        },
      },
    });
    await openApp(app);

    await app.page.getByRole("button", { name: "Change model" }).click();
    const dialog = app.page.getByRole("dialog");
    await expect(dialog.getByText("Unavailable")).toBeVisible();

    await dialog.getByRole("checkbox", { name: /retired-model/ }).click();
    await expect(dialog.getByRole("button", { name: "Unselect all (1)" })).toBeVisible();
  });
});

/**
 * Feedback after a save, and after a Gate model breaks the tool.
 *
 * Both come from the same complaint: the app was silent when it should not have
 * been. Choosing a model while on App default only remembers it, and the only
 * sign was "not in use" in small grey text; and a Gate model the tool could not
 * be served with simply broke the tool with nothing on screen.
 */
test.describe("new UI model feedback", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  const funded = {
    plan: "pro",
    paygEnabled: true,
    balanceCents: 1025,
    lowBalanceThresholdCents: 500,
    autoTopupArmed: false,
  };

  test("names the chosen model on the option that would use it", async ({ boot }) => {
    // Saving a model while on App default used to change nothing visible.
    const app = await boot({
      ...base,
      toolModels: {
        catalogue,
        credits: funded,
        choices: { "claude-code": { source: "tool", model_ids: [catalogue[0].id] } },
      },
    });
    await openApp(app);

    await expect(app.page.getByText(`Use ${catalogue[0].id}`)).toBeVisible();
  });

  test("falls back to the generic line when nothing is chosen", async ({ boot }) => {
    const app = await boot({ ...base, toolModels: { catalogue, credits: funded } });
    await openApp(app);

    await expect(app.page.getByText("Use a model selected in Gate AI")).toBeVisible();
  });
});

/**
 * Only the models this app can actually be served with (AG-590, AG-729).
 *
 * The case that cost a real prompt on staging: Codex was offered `gpt-4o`, which
 * carries the `tool-use` tag and still cannot serve it, because Codex sends
 * freeform tools. The provider's refusal - `Missing required parameter:
 * 'tools[0].custom'` - is not something a user can act on, so the picker
 * answers first.
 *
 * AG-729 split the answer into three. Only a model MEASURED failing is held
 * back; a model nobody ever tried is offered below an "Unverified" divider,
 * because the old boolean called that a refusal and quietly shrank the
 * catalogue to the one family anybody had swept.
 */
test.describe("new UI model picker compatibility", () => {
  const codexTools = [
    {
      slug: "codex",
      name: "Codex",
      upstream_provider_name: "OpenAI",
      default_upstream_url: "https://gw.example/codex",
      requires_upstream_credential: false,
      status: { kind: "connected" as const },
    },
  ];
  /**
   * Rows as a gateway serving AG-729's `tool_shapes` sends them, covering all
   * three states: verified, refuted, and never tried.
   */
  const mixed = [
    {
      // In KNOWN_GOOD: a real Codex request was run against this one. Shape
      // evidence alone no longer promotes a model, so a fixture that wants a
      // VERIFIED row has to name a pairing somebody actually ran.
      id: "openai/gpt-5.6-terra",
      owned_by: "openai",
      name: "GPT-5.6 Terra",
      tags: ["tool-use"],
      tool_shapes: { freeform: { verdict: "works", checked: "2026-08-28" } },
    },
    {
      id: "openai/gpt-4o",
      owned_by: "openai",
      name: "GPT-4o",
      tags: ["tool-use"],
      tool_shapes: { freeform: { verdict: "fails", checked: "2026-08-28" } },
    },
    { id: "openai/gpt-3-5-turbo-instruct", owned_by: "openai", name: "Instruct", tags: ["vision"] },
    // Nothing known about this one. It must be OFFERED, below the divider.
    { id: "mistralai/mistral-large", owned_by: "mistralai", name: "Mistral Large", tags: ["tool-use"] },
  ];

  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  /** Open Codex's picker. Four tests below need the same three clicks. */
  const openPicker = async (app: { page: Page }) => {
    await app.page.getByRole("button", { name: "Codex" }).first().click();
    await app.page.getByRole("radio", { name: /Gate model/ }).click();
    return app.page.getByRole("dialog");
  };

      test("counts only the measured failures as held back", async ({ boot }) => {
    // The count line covers only what was measured failing. Anything offered is
    // visibly in the list, so calling it "not shown" would be false.
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: codexTools,
      toolModels: { catalogue: mixed },
    });
    const dialog = await openPicker(app);

    await expect(dialog.getByText(/2 models are not shown/)).toBeVisible();
  });

  test("names the reason when every held-back model shares one", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: codexTools,
      // Only the freeform refusal remains, so there is one sentence worth saying.
      toolModels: { catalogue: mixed.filter((m) => m.id !== "openai/gpt-3-5-turbo-instruct") },
    });
    const dialog = await openPicker(app);

    await expect(dialog.getByText(/1 model is not shown/)).toBeVisible();
    // The copy no longer names a family, which stops being true the moment the
    // verdict table grows. It states what was measured.
    await expect(dialog.getByText(/verified to reject/)).toBeVisible();
  });

  test("treats an older gateway's silence as offered, not as a refusal", async ({ boot }) => {
    // No `tool_shapes` anywhere: the local fallback answers, and everything
    // outside the families it knows about is OFFERED rather than hidden.
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: codexTools,
      toolModels: {
        catalogue: [
          { id: "openai/gpt-5.6-terra", owned_by: "openai", name: "GPT-5.6 Terra", tags: ["tool-use"] },
          { id: "mistralai/mistral-large", owned_by: "mistralai", name: "Mistral Large", tags: ["tool-use"] },
        ],
      },
    });
    const dialog = await openPicker(app);

    // Both offered. Nothing was measured failing, so nothing is held back, and
    // a shape nobody has a verdict on is not a refusal.
    await expect(dialog.getByRole("checkbox")).toHaveCount(2);
    await expect(dialog.getByText(/not shown/)).toHaveCount(0);
  });

  test("lets the user overrule it, because the rule will date", async ({ boot }) => {
    // The freeform-tool rule is empirical. A model that starts working would be
    // unreachable if this were a hard filter, with no way to tell us so.
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: codexTools,
      toolModels: { catalogue: mixed },
    });
    await app.page.getByRole("button", { name: "Codex" }).first().click();
    await app.page.getByRole("radio", { name: /Gate model/ }).click();

    const dialog = app.page.getByRole("dialog");
    await dialog.getByRole("button", { name: "Show anyway" }).click();

    await expect(dialog.getByRole("checkbox")).toHaveCount(4);
    await expect(dialog.getByRole("checkbox", { name: "openai/gpt-4o" })).toBeVisible();

    // The sentence follows the override. Saying "not shown" here would
    // contradict both the rows on screen and the "Hide them" control beside it,
    // and nothing asserted this state before, which is how it drifted.
    await expect(dialog.getByText(/not shown/)).toHaveCount(0);
    await expect(dialog.getByText(/cannot serve this app/)).toBeVisible();
  });

    });
