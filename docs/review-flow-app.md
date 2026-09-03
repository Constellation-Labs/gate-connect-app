# Flow validation: Figma canvas `Flows / App` (`116:30199`) vs the code

Scope: states and sequence only. The visual/token findings for this page are in
`docs/review-figma-dialogs-app.md` and are not repeated here.

Canvas: fileKey `9FrccCojXy0f8QD8Wm5Lln`, page `116:30199`, six labelled
sections split by `line` dividers, 15 top-level frames.

Code under examination (read-only): `src/components/gc/AppPane.tsx`,
`src/NewUiApp.tsx`, `src/components/gc/dialogs.tsx`, `src/lib/groups.ts`,
`src/lib/toolModels.ts`.

_(written incrementally; sections appended as each is measured)_

## Code map (established first, so the frames can be matched against it)

The model flow is a three-state overlay machine owned by `NewUiApp`:

- `src/NewUiApp.tsx:331-335` - `modelOverlay` is
  `{kind:"picker"; then:"activate"|"remember"} | {kind:"confirm-gate"; modelIds} | null`.
  `then` is the whole reason there are two picker entries: opened from the
  **Gate model** radio it is a step in switching and must continue into billing;
  opened from **Change model** while on App default it is a browse and only
  remembers.
- `src/NewUiApp.tsx:494-509` - `openModelChoice` (`"app" | "gate" | null`),
  `openModelIds`, `openModelId`. `null` is "the preference read has not landed",
  and it disables the radio group rather than defaulting to App default.
- `src/NewUiApp.tsx:532-560` - `saveModel(source, modelIds, acknowledgePaidUse)`,
  sets `modelBusy`, records `modelError` on failure.
- `src/NewUiApp.tsx:562-573` - `activateGateModel(ids)`: refuses an empty set,
  writes straight through if `paidAckUnix` is set, otherwise opens
  `confirm-gate`.
- `src/NewUiApp.tsx:2034-2059` - `ModelPickerDialog` render + `onSave`/`onDismiss`.
- `src/NewUiApp.tsx:2060-2092` - `UseGateModelDialog` render + its two exits.
- `src/NewUiApp.tsx:2219-2334` - the `AppPane` call, including the
  `multiProviderSlugs.has(view.slug) ? {} : {...}` spread that decides whether
  the model card exists at all.
- `src/components/gc/AppPane.tsx:226-249` - the card renders only when
  `onChooseModel && onChangeModel && onAddCredits` are all present.
- `src/components/gc/dialogs.tsx:554-1000` - `ModelPickerDialog`.
- `src/components/gc/dialogs.tsx:1003+` - `UseGateModelDialog`.

## Code map, continued: the picker and the confirmation

`ModelPickerDialog` (`dialogs.tsx:554-1000`) is **one dialog, multi-select, for
every tool**. There is no single-select variant anywhere in the file.

- `:597` `const [draft, setDraft] = useState<string[]>(selectedIds)` - a draft
  seeded from the stored set, so Cancel is a real cancel.
- `:706-752` `renderRow` - `role="checkbox"`, `onClick` toggles the id in the
  draft and does nothing else. **No row click closes the dialog.**
- `:773-796` the `Modal` footer: `secondary` "Cancel" -> `onDismiss`;
  `primary` "Save models" -> `onSave(draft)`, `disabled` when the draft is empty.
- `:672-690` the sort: `.sort((a,b) => a.vendor.localeCompare(b.vendor) || a.id.localeCompare(b.id))`,
  with a comment quoting the Figma annotation verbatim.
- `:886-894` "Unselect all (n)" clears the draft; hidden at zero.
- `:916-945` the AG-592 rescue list for chosen ids that have no row.
- `:798` `initialFocus={searchRef}`.

`UseGateModelDialog` (`dialogs.tsx:1003-1100`) - `secondary` "Keep App default",
`primary` "Use Gate credits", `onDismiss` = `onKeepAppDefault`, width 512.

## Q2 answered first, because it is the largest: OpenCode and the model card

**They contradict. The design gives OpenCode a model card; the code withholds
it.** This is not a reading of an ambiguous frame - `662:17579` renders the
OpenCode app pane with a complete **Model selection** card in it.

What `662:17579` draws, verified from the render:

- header `OpenCode` / `Protected - 2m ago`, switch `Off`
- stat tiles, Messages chart
- **Model selection** card: "Choose whether OpenCode or Gate selects the AI
  models for requests", two radio tiles - `App default` and **`Gate models`**
  (plural, with the description "Use a model selected in Gate AI"), Gate
  selected
- **Current Gate models** as a **two-column grid**: `Anthropic / gate/opus 5`
  and `Deepseek / gate/deepseek-v4-flash` on row one, `Moonshot /
  gate/kimi-k3` on row two - each with its own 16px vendor mark and vendor name
- a rule, then a footer row inside that block: **"3 of 14 models enabled"** on
  the left and a **"Choose models"** button on the right
- `Gate credits: $10.25 available` row with `Add credits`
- Recent activity table

The sidebar in the same frame puts OpenCode under **`OTHER TOOLS  1 of 2`**
alongside OpenClaw - i.e. the design agrees with `lib/groups.ts` that OpenCode
is a multi-provider tool, and still draws it a model card. So the disagreement
is specifically about the rule, not about the classification.

The code side:

- `src/lib/groups.ts:29` `MULTI_PROVIDER_ID = "any-provider"`; OpenCode, OpenClaw
  and Hermes land in that group because the catalog claims them for no provider.
- `src/NewUiApp.tsx:1064-1072` `multiProviderSlugs` is built from that group.
- `src/NewUiApp.tsx:2254-2256` `{...(multiProviderSlugs.has(view.slug) ? {} : {…})}`
  - for a multi-provider slug none of `modelChoice`, `onChooseModel`,
  `onChangeModel`, `gateModel`, `credits`, `plan`, `onAddCredits` are passed.
- `src/components/gc/AppPane.tsx:226` `{onChooseModel && onChangeModel && onAddCredits && (<ModelSelection …/>)}`
  - so the card is not rendered.

**Verdict: DRAWN-ONLY.** No code path produces `662:17579`, `665:18498` or
`671:19431` for OpenCode, because the pane those three frames start from cannot
exist for OpenCode. The `AppPane` doc comment argues the withholding is right
("what does this app use on Gate model has no single answer for them"); the
designer drew the opposite. One of the two has to give, and it is not
decidable by measurement. Raise it.

Three smaller divergences fall out of the same frame, and they apply to
**every** app pane, not just OpenCode:

1. The radio is labelled **"Gate models"** (plural) in `662:17579`;
   `AppPane.tsx:407` labels it `"Gate model"` (singular).
2. **Current Gate models is a two-column grid with per-entry vendor marks and
   vendor names.** `AppPane.tsx:492-499` renders a single-column `ul` of bare
   mono ids with the mark deliberately dropped for a set
   (`GateModel.ids` doc, `AppPane.tsx:44-49`). The "left to right using their
   provider" annotation (`671:19247`) is about **this grid**, not about the
   picker list - "left to right" only means anything on a horizontal flow.
3. The card's footer draws **"3 of 14 models enabled"** and a **"Choose
   models"** button. The code draws no count and labels the action
   **"Change model"** (`AppPane.tsx:480`).

## Q3: the sort rule - implemented, but on the wrong surface

The annotation `671:19247` reads "Current models will sort alphabetically, left
to right using their provider. Example. Anthropic > DeepSeek > Moonshot".

**"Current models" and "left to right" both name the app pane's Current Gate
models block, not the picker list.** The frames settle it:

- `662:17579` draws that block as a **two-column grid** flowing left to right:
  `Anthropic gate/opus 5` | `Deepseek gate/deepseek-v4-flash`, then
  `Moonshot gate/kimi-k3`. Alphabetical by provider in reading order.
- `671:19431` (the same pane after the picker) draws four:
  `Anthropic gate/opus 5` | `Anthropic gate/sonnet 5`,
  `Deepseek gate/deepseek-v4-flash` | `Moonshot gate/kimi-k3`. Alphabetical by
  provider in reading order, with a provider's own models adjacent.
- The **picker list** in `665:18498` is emphatically **not** provider-sorted:
  `gate/opus 5`, `gate/opus 4.8`, `gate/deepseek-v4-flash`, `gate/fable 5`,
  `gate/deepseek-v4-flash-0731`, `gate/sonnet-5`, `gate/qwen3-6-35b-a3b`,
  `gate/kimi-k3`. Two Deepseek ids are split by an Anthropic one.

Where the code puts it:

- `dialogs.tsx:685-690` sorts the **picker list** by
  `vendor.localeCompare || id.localeCompare`, and the comment quotes this
  annotation as its authority. The rule is real; it has been applied to the one
  list the file draws unsorted.
- The **pane** does not sort. `NewUiApp.tsx:507` `openModelIds = openPref?.modelIds ?? []`
  is the stored order, and `AppPane.tsx:492-499` maps it straight into a
  single-column `<ul>`. So the surface the annotation names has no sort at all.

**Verdict: DIVERGES.** The requirement is honoured, on the opposite surface from
the one it was written on. Worth noting that sorting the picker is harmless and
arguably an improvement; not sorting the pane list is the actual miss.

## A code state nothing reaches: `modelOverlay.then === "remember"`

`NewUiApp.tsx:331-335` carries a two-valued `then`, and the comment above it
(`:320-330`) explains the distinction carefully: opened from **Change model
while on App default** the picker is a browse and only remembers.

That entry does not exist.

- `AppPane.tsx:480` is the only call site of `onChangeModel`, and it sits inside
  the `{gateActive && (…)}` block that opens at `AppPane.tsx:455`. `gateActive`
  is `choice === "gate"` (`AppPane.tsx:376`), and `choice` is
  `modelChoice` = `openModelChoice` (`NewUiApp.tsx:2257`).
- So whenever "Change model" is clickable, `openModelChoice === "gate"`, and the
  ternary at `NewUiApp.tsx:2313-2315`
  (`then: openModelChoice === "gate" ? "activate" : "remember"`) can only ever
  produce `"activate"`.
- The other picker entry, `NewUiApp.tsx:2288`, hard-codes `then: "activate"`.
- Therefore `NewUiApp.tsx:2056` (`else void saveModel("tool", ids)`) is
  unreachable.

The design agrees there is no such entry: `121:35174` ("App w/ App default
selected") draws **no Change model button** and no Current-Gate-model block
under App default. So this is dead code following a decision the pane already
made, not a missing screen. It is worth deleting or the comment is worth
correcting, because as written it describes a flow the app does not have.

## `121:35174` - App default: one drawn row the code has not got

Under App default the frame draws, in the Current-model slot, an info row:

  glyph (the app's own mark) | **Using Claude Desktop model**
  "Gate protects requests, then leaves model choice to Claude Desktop. No Gate
  credits used."

and **no Gate credits row at all**.

- Code: `AppPane.tsx:455-503` renders nothing in that slot under App default.
  The comment there argues the case ("a section headed Current describing
  nothing current"), which is a fair argument against the *earlier* content,
  but the frame's row is not that: it is a statement about App default, and it
  is the sentence that tells the user no credits are spent. **MISSING.**
- Code: the credits `InfoRow` at `AppPane.tsx:505-533` is outside the
  `gateActive` guard, so it is drawn under App default too. The frame does not
  draw it there. **DIVERGES** (code state with no frame).
- The Gate radio's subtitle in this frame is "Use a model selected in Gate AI",
  matching `AppPane.tsx:424`'s no-model fallback; the code replaces it with
  "Use <id>" / "Use any of N Gate models" once a model is remembered, which no
  frame draws. That deviation is documented at `AppPane.tsx:410-421` and is
  defensible; noted so nobody reports it as new.

