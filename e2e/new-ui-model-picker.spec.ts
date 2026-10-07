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
const tools = [
  {
    slug: "claude-code",
    name: "CLI",
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

/** The card's heading over what the app's config holds: the frame's words
 *  (`1410:28145`), singular over a single row. */
const GATE_MODEL_HEADING = /^Current Gate models?$/;
const GATE_MODELS_HEADING = /^Current Gate models$/;

/**
 * Open one app's pane, which is where model selection lives.
 *
 * By the section's name, not the tool's: the rail draws one row per app now, so
 * the button is "Claude" and the `claude-code` tool behind it is what the pane
 * resolves to (`sectionMemberKeys` in `NewUiApp`). The tool's own `name` is no
 * longer drawn anywhere the rail can be clicked.
 */
async function openApp(app: { page: import("@playwright/test").Page }) {
  await app.page.getByRole("button", { name: "Claude" }).first().click();
}

/**
 * Put the app on a Gate model, which is the only branch that draws a balance.
 *
 * The card names credits only while Gate is the source: under App default the
 * app sends Gate nothing to bill. So every credits case has to make the switch
 * first, and it is the same three clicks the flow test at the top walks.
 */
async function switchToGateModel(app: { page: import("@playwright/test").Page }) {
  await app.page.getByRole("radio", { name: /Gate model/ }).click();
  await app.page.getByRole("dialog").getByRole("checkbox", { name: catalogue[0].id }).click();
  await app.page.getByRole("button", { name: "Apply selections" }).click();
  await app.page.getByRole("button", { name: "Use Gate credits" }).click();
  await expect(app.page.getByRole("dialog")).toHaveCount(0);
}

test.describe("new UI model picker", () => {
  test("says no model is chosen before one is", async ({ boot }) => {
    const app = await boot(base);
    await openApp(app);

    // Under App default there is no current Gate model to report, so the section
    // is absent rather than empty - and with it the control that would change a
    // model this app is not using.
    await expect(app.page.getByText(GATE_MODEL_HEADING)).toHaveCount(0);
    await expect(app.page.getByRole("button", { name: "Choose models" })).toHaveCount(0);
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

    await expect(app.page.getByRole("heading", { name: "Choose Gate models" })).toBeVisible();
    await expect(app.page.getByRole("dialog").getByRole("checkbox")).toHaveCount(2);
  });

  test("picking a model then confirming hands routing to Gate", async ({ boot }) => {
    const app = await boot({ ...base, toolModels: { catalogue } });
    await openApp(app);

    await app.page.getByRole("radio", { name: /Gate model/ }).click();
    await app.page.getByRole("dialog").getByRole("checkbox", { name: catalogue[0].id }).click();
    await app.page.getByRole("button", { name: "Apply selections" }).click();

    // The billing confirmation, because this organization has never accepted it.
    await expect(
      app.page.getByRole("heading", { name: /Use a Gate model for CLI\?/ }),
    ).toBeVisible();
    await app.page.getByRole("button", { name: "Use Gate credits" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    await expect(app.page.getByRole("radio", { name: /Gate model/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    // Served, so the section reports it.
    await expect(app.page.getByText(GATE_MODEL_HEADING)).toBeVisible();
    await expect(app.page.getByText(catalogue[0].id, { exact: true })).toBeVisible();
  });

  test("declining the billing keeps the model without serving it", async ({ boot }) => {
    // They declined the charge, not the choice. Remembering it is why a
    // preference may name a model while its source is still "tool".
    const app = await boot({ ...base, toolModels: { catalogue } });
    await openApp(app);

    await app.page.getByRole("radio", { name: /Gate model/ }).click();
    await app.page.getByRole("dialog").getByRole("checkbox", { name: catalogue[0].id }).click();
    await app.page.getByRole("button", { name: "Apply selections" }).click();
    await app.page.getByRole("button", { name: "Keep App default" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    await expect(app.page.getByRole("radio", { name: /App default/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    // Kept, and named by the radio that would put it to use. The "Current Gate
    // model" section stays away: nothing about it is current under App default.
    await expect(app.page.getByText(`Use ${catalogue[0].id}`)).toBeVisible();
    await expect(app.page.getByText(GATE_MODEL_HEADING)).toHaveCount(0);
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

  test("Choose models swaps the served model directly once Gate is serving", async ({ boot }) => {
    const app = await boot({
      ...base,
      toolModels: {
        catalogue,
        paidAckUnix: 1767330245,
        choices: { "claude-code": { source: "gate", model_ids: [catalogue[0].id] } },
      },
    });
    await openApp(app);

    await app.page.getByRole("button", { name: "Choose models" }).click();
    const swap = app.page.getByRole("dialog");
    await swap.getByRole("checkbox", { name: catalogue[1].id }).click();
    await swap.getByRole("checkbox", { name: catalogue[0].id }).click();
    await swap.getByRole("button", { name: "Apply selections" }).click();

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
    await expect(app.page.getByText(GATE_MODEL_HEADING)).toHaveCount(0);
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
  const many = [
    ...catalogue,
    { id: "openai/gpt-5", owned_by: "openai", name: "GPT-5", tags: ["tool-use"] },
    { id: "deepseek/deepseek-v3", owned_by: "deepseek", name: "DeepSeek V3", tags: ["tool-use"] },
  ];

  test("search narrows the list", async ({ boot }) => {
    const app = await boot({ ...base, toolModels: { catalogue: many } });
    await openApp(app);
    await app.page.getByRole("radio", { name: /Gate model/ }).click();

    const dialog = app.page.getByRole("dialog");
    await expect(dialog.getByRole("checkbox")).toHaveCount(4);

    await dialog.getByRole("searchbox").fill("opus");
    await expect(dialog.getByRole("checkbox")).toHaveCount(1);
  });

  test("counts the set as it is picked, against the limit", async ({ boot }) => {
    // `1410:31315`: "2 of 4 models selected". The count is of the choice, not
    // of the catalogue, so narrowing the list must not move it.
    const app = await boot({ ...base, toolModels: { catalogue: many } });
    await openApp(app);
    await app.page.getByRole("radio", { name: /Gate model/ }).click();

    const dialog = app.page.getByRole("dialog");
    // The set is stated by the checked rows now. `Unselect all` used to carry
    // the count here and design removed it on 2026-09-04: there is no
    // select-all to mirror it, and Cancel already starts the selection over.
    const checked = dialog.locator('[role="checkbox"][aria-checked="true"]');
    await expect(checked).toHaveCount(0);
    await expect(dialog.getByText("0 of 4 models selected")).toBeVisible();

    await dialog.getByRole("checkbox", { name: many[0].id }).click();
    await expect(checked).toHaveCount(1);

    await dialog.getByRole("checkbox", { name: many[1].id }).click();
    await expect(checked).toHaveCount(2);
    await expect(dialog.getByText("2 of 4 models selected")).toBeVisible();
    // Narrowing what is shown does not change what is chosen. One of the two is
    // filtered out of the list, so the *visible* checked count drops while the
    // draft does not.
    await dialog.getByRole("searchbox").fill("opus");
    await expect(dialog.getByRole("checkbox")).toHaveCount(1);
    await expect(dialog.getByText("2 of 4 models selected")).toBeVisible();
    await expect(dialog.getByText("No models enabled")).toHaveCount(0);
  });

  test("search matches the display name, with a space where the id has a dash", async ({
    boot,
  }) => {
    // "mistral large" found nothing: the search knew the id alone, and the id
    // is `mistralai/mistral-large` (alpha.13 feedback, 2026-10-07).
    const app = await boot({
      ...base,
      toolModels: {
        catalogue: [
          ...many,
          { id: "mistralai/mistral-large", owned_by: "mistralai", name: "Mistral Large", tags: ["tool-use"] },
          { id: "mistralai/mistral-small", owned_by: "mistralai", name: "Mistral Small", tags: ["tool-use"] },
        ],
      },
    });
    await openApp(app);
    await app.page.getByRole("radio", { name: /Gate model/ }).click();

    const dialog = app.page.getByRole("dialog");
    await dialog.getByRole("searchbox").fill("mistral large");
    await expect(dialog.getByRole("checkbox")).toHaveCount(1);
    await expect(dialog.getByRole("checkbox", { name: "mistralai/mistral-large" })).toBeVisible();

    // Words in any order, and the id's own spelling still works.
    await dialog.getByRole("searchbox").fill("large mistral");
    await expect(dialog.getByRole("checkbox", { name: "mistralai/mistral-large" })).toBeVisible();
    await dialog.getByRole("searchbox").fill("mistral-large");
    await expect(dialog.getByRole("checkbox", { name: "mistralai/mistral-large" })).toBeVisible();
  });

  test("opens with the applied models at the top, and holds that order while toggling", async ({
    boot,
  }) => {
    // The list is by provider then id, so Anthropic sits above OpenAI. An
    // applied GPT-5 opens on top (alpha.13 feedback, 2026-10-07: "keep the
    // selected ones on top"). Checking or clearing a row does not move it:
    // reordering on the live draft pulled rows out from under the pointer
    // (review, #427).
    const app = await boot({
      ...base,
      toolModels: {
        catalogue: many,
        paidAckUnix: 1787740800,
        choices: { "claude-code": { source: "gate", model_ids: ["openai/gpt-5"] } },
      },
    });
    await openApp(app);
    await app.page.getByRole("button", { name: "Choose models" }).click();

    const dialog = app.page.getByRole("dialog");
    const rows = dialog.getByRole("checkbox");
    await expect(rows).toHaveCount(4);
    await expect(rows.first()).toHaveAccessibleName("openai/gpt-5");
    await expect(rows.nth(1)).toHaveAccessibleName("anthropic/claude-opus-5");

    await dialog.getByRole("checkbox", { name: "anthropic/claude-opus-5" }).click();
    await expect(rows.first()).toHaveAccessibleName("openai/gpt-5");
    await expect(rows.nth(1)).toHaveAccessibleName("anthropic/claude-opus-5");

    await dialog.getByRole("checkbox", { name: "openai/gpt-5" }).click();
    await expect(rows.first()).toHaveAccessibleName("openai/gpt-5");
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

    await expect(dialog.getByRole("checkbox")).toHaveCount(1);
  });

  test("App default with a remembered set over the limit opens the picker to trim it", async ({
    boot,
  }) => {
    // Review on #388: the Gate radio used to save the remembered set as it was,
    // and the backend refused anything over four.
    const five = [
      ...many,
      { id: "moonshot/kimi-k3", owned_by: "moonshot", name: "Kimi K3", tags: ["tool-use"] },
    ];
    const app = await boot({
      ...base,
      toolModels: {
        catalogue: five,
        paidAckUnix: 1787740800,
        choices: { "claude-code": { source: "tool", model_ids: five.map((m) => m.id) } },
      },
    });
    await openApp(app);
    await app.page.getByRole("radio", { name: /Gate model/ }).click();

    const dialog = app.page.getByRole("dialog");
    await expect(dialog.getByText("5 of 4 models selected")).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Apply selections" })).toBeDisabled();
    expect(await app.lastCall("set_tool_model")).toBeNull();

    await dialog.getByRole("checkbox", { name: five[4].id }).click();
    await expect(dialog.getByRole("button", { name: "Apply selections" })).toBeEnabled();

    // Trimmed, Apply hands the app to Gate: the picker was opened to activate,
    // and billing was already accepted, so the four go straight through.
    await dialog.getByRole("button", { name: "Apply selections" }).click();
    await expect.poll(() => app.lastCall("set_tool_model")).toMatchObject({
      tool: "claude-code",
      source: "gate",
      modelIds: many.map((m) => m.id),
    });
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
  });

  test("stops at four models, and Clear selections starts over", async ({ boot }) => {
    const five = [
      ...many,
      { id: "moonshot/kimi-k3", owned_by: "moonshot", name: "Kimi K3", tags: ["tool-use"] },
    ];
    const app = await boot({ ...base, toolModels: { catalogue: five } });
    await openApp(app);
    await app.page.getByRole("radio", { name: /Gate model/ }).click();

    const dialog = app.page.getByRole("dialog");
    for (const m of five.slice(0, 4)) {
      await dialog.getByRole("checkbox", { name: m.id }).click();
    }
    await expect(dialog.getByText("4 of 4 models selected")).toBeVisible();
    await expect(dialog.getByRole("checkbox", { name: five[4].id })).toBeDisabled();

    await dialog.getByRole("button", { name: "Clear selections" }).click();
    await expect(dialog.getByText("0 of 4 models selected")).toBeVisible();
    await expect(dialog.getByRole("checkbox", { name: five[4].id })).toBeEnabled();
    await expect(dialog.getByRole("button", { name: "Apply selections" })).toBeDisabled();
  });

  test("Choose models enables several models at once and states the count", async ({ boot }) => {
    const app = await boot({
      ...base,
      toolModels: {
        catalogue: many,
        paidAckUnix: 1787740800,
        choices: { "claude-code": { source: "gate", model_ids: [catalogue[0].id] } },
      },
    });
    await openApp(app);

    await app.page.getByRole("button", { name: "Choose models" }).click();
    const dialog = app.page.getByRole("dialog");
    await dialog.getByRole("checkbox", { name: "openai/gpt-5" }).click();

    await dialog.getByRole("button", { name: "Apply selections" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    // The card lists the set as a grid (`1410:31957`), under the plural heading.
    await expect(app.page.getByText(GATE_MODELS_HEADING)).toBeVisible();
    const grid = app.page.getByRole("list", { name: /^Gate models for / });
    await expect(grid.getByRole("listitem")).toHaveCount(2);
    await expect(grid.getByText("openai/gpt-5", { exact: true })).toBeVisible();
  });

  test("refuses to save an empty set, which is where the last model is held", async ({ boot }) => {
    // AG-590: the final model cannot be removed without selecting another or
    // returning to App default. The line is held on the primary rather than on
    // the row, because what the ticket protects is the state that gets written,
    // not the draft.
    const app = await boot({
      ...base,
      toolModels: {
        catalogue: many,
        paidAckUnix: 1787740800,
        choices: { "claude-code": { source: "gate", model_ids: [catalogue[0].id] } },
      },
    });
    await openApp(app);

    await app.page.getByRole("button", { name: "Choose models" }).click();
    const dialog = app.page.getByRole("dialog");
    const only = dialog.getByRole("checkbox", { name: catalogue[0].id });

    await expect(only).toHaveAttribute("aria-checked", "true");

    // The row clears, and the dialog neither pretends the set is fine nor leaves
    // a disabled button with nothing beside it.
    await only.click();
    await expect(only).toHaveAttribute("aria-checked", "false");
    await expect(dialog.getByText("No models enabled")).toBeVisible();
    await expect(dialog.getByText(/needs at least one model/)).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Apply selections" })).toBeDisabled();

    // A *different* model unlocks it. Re-checking the one just cleared would
    // return the draft to the stored set, which the primary also refuses now -
    // an unchanged selection has nothing to apply (design, 2026-09-04).
    await dialog.getByRole("checkbox", { name: "openai/gpt-5" }).click();
    await expect(dialog.getByRole("button", { name: "Apply selections" })).toBeEnabled();
  });

  test("clearing the draft by hand leaves the stored set alone", async ({ boot }) => {
    const app = await boot({
      ...base,
      toolModels: {
        catalogue: many,
        paidAckUnix: 1787740800,
        choices: { "claude-code": { source: "gate", model_ids: [catalogue[0].id] } },
      },
    });
    await openApp(app);

    await app.page.getByRole("button", { name: "Choose models" }).click();
    const dialog = app.page.getByRole("dialog");
    await dialog.getByRole("checkbox", { name: "openai/gpt-5" }).click();

    await dialog.getByRole("checkbox", { name: catalogue[0].id }).click();
    await dialog.getByRole("checkbox", { name: "openai/gpt-5" }).click();
    await expect(dialog.getByText("No models enabled")).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Apply selections" })).toBeDisabled();

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
  test("shows the balance the way the design words it", async ({ boot }) => {
    const app = await boot({
      ...base,
      toolModels: {
        catalogue,
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
    await switchToGateModel(app);

    await expect(app.page.getByText("$10.25 available")).toBeVisible();
  });

  test("reads N/A when no balance was reported, not $0.00", async ({ boot }) => {
    // The default fixture reports none. Printing zero here would tell a funded
    // org their tools are about to stop.
    const app = await boot({ ...base, toolModels: { catalogue } });
    await openApp(app);
    await switchToGateModel(app);

    // The credits line specifically. A bare "N/A" used to be unambiguous only
    // because the stat tiles above it were stuck in skeletons: the default
    // `installations` fixture does not name this machine, so it is unattributed, and
    // those counters now correctly read "n/a" rather than promising a reading
    // that is not coming. Three more matches, none of them what this test is
    // about.
    await expect(app.page.getByText("Gate credits: N/A")).toBeVisible();
    await expect(app.page.getByText("$0.00 available")).toHaveCount(0);
  });

  test("says nothing about a balance while the app is on its own model", async ({ boot }) => {
    // The card drew the balance, "Add credits" and "Manage billing"
    // unconditionally, directly beneath the radio that had just said Gate is not
    // serving this app. Under App default there is nothing for Gate to bill.
    const app = await boot({
      ...base,
      toolModels: {
        catalogue,
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

    await expect(app.page.getByRole("radio", { name: /App default/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await expect(app.page.getByText("$10.25 available")).toHaveCount(0);
    await expect(app.page.getByText(/Gate credits:/)).toHaveCount(0);
    // And the App-default branch says what it does instead (408:25491).
    await expect(app.page.getByText(/Using .* model/)).toBeVisible();
    await expect(app.page.getByRole("button", { name: "Add credits" })).toHaveCount(0);

    // And it comes back with the branch that spends it.
    await switchToGateModel(app);
    await expect(app.page.getByText("$10.25 available")).toBeVisible();
  });

  test("says PAYG is off rather than showing money that cannot be spent here", async ({ boot }) => {
    const app = await boot({
      ...base,
      toolModels: {
        catalogue,
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
    await switchToGateModel(app);

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
  test("names the single model with its vendor, as the frame draws it", async ({ boot }) => {
    const app = await boot({ ...base, toolModels: { catalogue } });
    await openApp(app);

    await app.page.getByRole("radio", { name: /Gate model/ }).click();
    const dialog = app.page.getByRole("dialog");
    await dialog.getByRole("checkbox", { name: catalogue[0].id }).click();
    await dialog.getByRole("button", { name: "Apply selections" }).click();

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
    await dialog.getByRole("button", { name: "Apply selections" }).click();

    const confirm = app.page.getByRole("dialog");
    await expect(confirm.getByText(catalogue[0].id)).toBeVisible();
    await expect(confirm.getByText(catalogue[1].id)).toBeVisible();
    // The set replaced the old split presentation entirely.
    await expect(confirm.getByText(/Also enabled/)).toHaveCount(0);
  });

  test("says Gate writes the models into the tool's own config", async ({ boot }) => {
    const app = await boot({ ...base, toolModels: { catalogue } });
    await openApp(app);

    await app.page.getByRole("radio", { name: /Gate model/ }).click();
    const dialog = app.page.getByRole("dialog");
    await dialog.getByRole("checkbox", { name: catalogue[0].id }).click();
    await dialog.getByRole("button", { name: "Apply selections" }).click();

    // It said the app's own model preference was not changed, which stopped
    // being true once the choice was written into the app's config.
    await expect(app.page.getByText(/own model preference is not changed/)).toHaveCount(0);
    await expect(
      app.page.getByText(/Gate sets CLI's model to these in its own config/),
    ).toBeVisible();
    await expect(
      app.page.getByText(/restore your previous model/),
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

    await app.page.getByRole("button", { name: "Choose models" }).click();
    const dialog = app.page.getByRole("dialog");
    await expect(dialog.getByText("Unavailable")).toBeVisible();

    await dialog.getByRole("checkbox", { name: /retired-model/ }).click();
    await expect(dialog.getByText("Unavailable")).toHaveCount(0);
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
 * Every model the gateway lists is offered (team decision, 2026-10-07).
 *
 * The picker used to hold back models the compatibility table said the app
 * could not use, under a "N models are not shown" line with a "Show anyway"
 * link. The alpha.13 round asked for both to go: show them all, and drop the
 * copy. What is pinned here is that nothing is held back any more, whatever
 * the tags or the gateway's own verdicts say.
 */
test.describe("new UI model picker offers the whole catalogue", () => {
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
  /** Rows covering every state the old filter distinguished: verified,
   *  refuted, untagged for tools, and never tried. */
  const mixed = [
    {
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
    { id: "mistralai/mistral-large", owned_by: "mistralai", name: "Mistral Large", tags: ["tool-use"] },
  ];

  test("lists every model, with no held-back count and no override link", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: codexTools,
      toolModels: { catalogue: mixed },
    });
    await app.page.getByRole("button", { name: "Codex" }).first().click();
    await app.page.getByRole("radio", { name: /Gate model/ }).click();
    const dialog = app.page.getByRole("dialog");

    await expect(dialog.getByRole("checkbox")).toHaveCount(4);
    await expect(dialog.getByRole("checkbox", { name: "openai/gpt-3-5-turbo-instruct" })).toBeVisible();
    await expect(dialog.getByText(/not shown/)).toHaveCount(0);
    await expect(dialog.getByText(/cannot serve this app/)).toHaveCount(0);
    await expect(dialog.getByRole("button", { name: "Show anyway" })).toHaveCount(0);
  });
});

/**
 * Gate models live in the tool's own config now (Codex's `config.toml`,
 * Hermes's `config.yaml`), not on each request. Three consequences the card
 * has to get right: a write that changed a running tool's config offers the
 * same restart notice a routing write does; a tool moved off its Gate models
 * from inside itself is put back on App default, and the card says why; and
 * Hermes, multi-provider on the rail, gets the card because its config can
 * hold the set.
 */
test.describe("new UI Gate models in the tool's config", () => {
  const CODEX = {
    slug: "codex",
    name: "Codex",
    upstream_provider_name: "OpenAI",
    default_upstream_url: "https://gw.example/codex",
    requires_upstream_credential: false,
    status: { kind: "connected" as const },
  };
  const set = ["openai/gpt-5.6-terra", "mistralai/mistral-large"];
  const codexCatalogue = [
    { id: set[0], owned_by: "openai", name: "GPT-5.6 Terra", tags: ["tool-use"] },
    { id: set[1], owned_by: "mistralai", name: "Mistral Large", tags: ["tool-use"] },
  ];
  const onGate = {
    catalogue: codexCatalogue,
    paidAckUnix: 1787740800,
    choices: { codex: { source: "gate" as const, model_ids: set } },
  };

  test("the card leads with the model the tool's config starts on, and saves keep the stored order", async ({
    boot,
  }) => {
    // The user picked the SECOND model of the set in Codex's own picker. The card
    // shows that one first, but the stored order is the user's and every save
    // keeps it: reordering would move the default model, rewrite the config and
    // restart Codex's daemon with no choice made (review on #382).
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CODEX],
      toolModels: { ...onGate, configuredModel: { codex: set[1] } },
    });
    await app.page.getByRole("button", { name: "Codex" }).first().click();
    await expect(app.page.getByText(set[1]).first()).toBeVisible();

    await app.page.getByRole("radio", { name: /App default/ }).click();
    await expect.poll(() => app.lastCall("set_tool_model")).toMatchObject({
      tool: "codex",
      source: "tool",
      modelIds: set,
    });
  });

  test("App default keeps the whole set, not only its first model", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CODEX],
      toolModels: onGate,
    });
    await app.page.getByRole("button", { name: "Codex" }).first().click();

    await app.page.getByRole("radio", { name: /App default/ }).click();

    await expect.poll(() => app.lastCall("set_tool_model")).toMatchObject({
      tool: "codex",
      source: "tool",
      modelIds: set,
    });
    await expect(app.page.getByText("Use any of 2 Gate models")).toBeVisible();
  });

  test("offers to restart a running app after a model change rewrites its config", async ({
    boot,
  }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CODEX],
      toolModels: onGate,
      runningAgentNames: ["codex"],
    });
    await app.page.getByRole("button", { name: "Codex" }).first().click();

    await app.page.getByRole("radio", { name: /App default/ }).click();

    await expect(
      app.page.getByRole("heading", { name: "Apply changes to running apps" }),
    ).toBeVisible();
    // Worded for any change, since a model write raises it too.
    await expect(
      app.page.getByText("Your configuration is saved. One final step makes the change active"),
    ).toBeVisible();
  });

  test("stays silent after a model change when the app is not running", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CODEX],
      toolModels: onGate,
    });
    await app.page.getByRole("button", { name: "Codex" }).first().click();

    await app.page.getByRole("radio", { name: /App default/ }).click();

    await expect.poll(() => app.lastCall("set_tool_model")).not.toBeNull();
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
  });

  test("says so when the app was moved off its Gate models from inside it", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CODEX],
      toolModels: { ...onGate, left: { codex: "gpt-6-sol" } },
    });
    await app.page.getByRole("button", { name: "Codex" }).first().click();

    const notice = app.page
      .getByRole("status")
      .filter({ hasText: "You switched Codex to gpt-6-sol in Codex, so it is back on App default." });
    await expect(notice).toBeVisible();
    await expect(app.page.getByRole("radio", { name: /App default/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );

    await notice.getByRole("button", { name: "Dismiss" }).click();
    await expect(notice).toHaveCount(0);
  });

  test("draws the model card for Hermes", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [
        {
          slug: "hermes",
          name: "Hermes",
          upstream_provider_name: "Hermes",
          default_upstream_url: "https://gw.example/hermes",
          status: { kind: "connected" as const },
        },
      ],
    });
    await app.page.getByRole("button", { name: /^Hermes/ }).first().click();

    await expect(app.page.getByRole("heading", { name: "Model selection" })).toBeVisible();
  });
});
