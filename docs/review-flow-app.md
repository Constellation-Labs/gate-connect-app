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

## Q1: the file specifies two picker behaviours; the code implements one, for the wrong population

Two annotations state the intent, and they sit under two different sections:

- `139:66782` (x=4528, under `139:66117`, section **App / Select a model**) -
  "When the user selects a model it automatically closes the modal and applies
  their selection."
- `671:19288` (x=2287, under `665:18498`, section **App / Select multiple models
  (Opencode)**) - "When the user selects a model (multiple) the modal stays open
  until the user confirms their selections. Then the modal will close"

This is not two readings of one annotation. **The two frames draw structurally
different dialogs**, and the difference is exactly the one the annotations
describe.

| | `665:18400` (single, in `139:66117`) | `665:19064` (multi, in `665:18498`) |
| --- | --- | --- |
| card | 600x**536** | 600x**588** |
| title | "Choose a Gate model" | "Choose Gate models" |
| subtitle | "Claude desktop will be able to use these models" | "OpenCode will be able to use these models" |
| rows checked | **1** (`gate/opus 5` `Icon / SquareCheck`; the other 7 `Icon / Square`) | **4** (`opus 5`, `deepseek-v4-flash`, `sonnet-5`, `kimi-k3`) |
| count slot | text `3 models selected` | badge frame `682:20038` = `4` + `selected` |
| **footer** | **none - the card ends at the list** | `665:19135`, a 550x36 row of **three** Buttons (133 / 70 / 131) |

The 52px height difference is exactly the footer row plus its gap. So the
single-model dialog **has no confirm control at all** - no Cancel, no Apply -
which is only coherent if a row click is itself the commit. The annotation and
the geometry agree.

(One drafting wrinkle, flagged rather than leaned on: `665:18400` says "3 models
selected" while drawing one checked row. It does not change the footer finding,
which is structural.)

### What the code has

**One dialog, and it is the multi one.** `dialogs.tsx:554-585` - there is no
`singleSelect`, `autoClose`, `maxSelect` or `selectionMode` prop anywhere in
`src/` (grepped; zero hits). Every caller gets the same component:

- `dialogs.tsx:706-752` `renderRow` toggles the draft and returns. No close.
- `dialogs.tsx:773-796` the footer is unconditional: Cancel -> `onDismiss`,
  "Save models" -> `onSave(draft)`.
- `dialogs.tsx:580-585` the `onSave` doc comment states the decision in so many
  words: "A set is not a sequence of independent clicks - AG-590 requires the
  final model not be removable without choosing another - so it is confirmed
  once rather than written per toggle, and Cancel is a real cancel."

So the single-model auto-close-and-apply behaviour is **implemented nowhere**,
and the code has a written reason for not implementing it (AG-590: you must not
be able to remove your last model, which a commit-per-click cannot enforce
without either rejecting the click or leaving the user with none).

### Which tools this actually lands on - and the sting

The populations are **inverted**.

- The auto-close annotation is drawn on a **Claude Desktop** frame - a
  single-provider tool. Every tool that can reach the picker in the shipped code
  is single-provider, because `NewUiApp.tsx:2254-2256` withholds
  `onChooseModel` from `multiProviderSlugs`.
- The stays-open annotation is drawn on an **OpenCode** frame. OpenCode,
  OpenClaw and Hermes (`groups.ts:29`, `NewUiApp.tsx:1064-1072`) are precisely
  the tools that **cannot open the picker at all** - see Q2.

Net: the code ships the confirm-footer flow to the population the file drew
auto-close for, and ships nothing to the population the file drew the
confirm-footer flow for. Neither annotation is honoured on the surface it was
written on.

**Verdict: DIVERGES, and it is the same root cause as Q2.** Deciding Q2
decides most of this: if multi-provider tools get a model card, the multi
behaviour has a home and the question narrows to "should single-provider tools
auto-close". If they do not, then the file's multi flow has no population at
all and `671:19288` is moot. Do not implement auto-close before AG-590 has an
answer for it - a click that both applies and closes cannot ask "you are
removing your last model" without contradicting one of the two annotations.

## `App / Main screens` (`130:56395`) - the three chrome states

Three frames, one axis: what sits between the topbar and the pane. All three
draw the same app (Claude Desktop, Gate model selected) and the same routing
banner, so the section is a test of the banner stack, not of the pane.

| Frame | Caption (`y=1519`) | Stack above `container` | `container` y |
| --- | --- | --- | --- |
| `116:31920` | "App w/ Gate model selected" | topbar, `banner/status-protected` | 96.25 |
| `116:30204` | "…+ update available" | topbar, `banner/update` (`124:36163`), `banner/status-protected` | 144.25 |
| `116:30663` | "…+ update available + alert" | as above, **plus an in-pane alert card** (`116:30678`) | 144.25 |

**The chrome stack is IMPLEMENTED and the order matches exactly.**
`AppShell.tsx:90-101` renders `{update && <UpdateBanner/>}` then an
unconditional `<RoutingBanner/>`, which is the drawn 48/48/48 sequence and the
drawn conditionality (the update banner is the one that comes and goes).
`banners.tsx:106,115` produce both routing phrasings, and all three frames draw
the "partly" one ("Gate Connect is partly routing your apps" / "Partly routed"),
which is `allProtected === false`.

### The alert card is the divergence

`116:30678` is **not** chrome: it sits inside `main-nav` at `y=0`, above the KPI
rail, i.e. exactly the `{alert}` slot at `AppPane.tsx:220`. It draws:

- `Icon / TriangleAlert`
- title **"Claude Desktop isn't protected"**
- body **"Routing is set to off. Reconnect to restore protection."**
- an `Icon / X` dismiss (`116:30690`) and **both** paging chevrons
  (`116:30680` right, `116:30692` left)
- and the frame's own header reads "Not protected - 2m ago" with the switch
  drawn **Off**, so the drawn cause is unambiguous: the **master is off**.

The code has that copy, verbatim, and it is in the wrong place.

- `src/lib/notices.ts:46-56` - `case "master-off"` returns exactly
  ``title: `${name} isn't protected` `` and
  `body: "Routing is set to off. Reconnect to restore protection."`, citing the
  same drawn banner.
- But `buildNotices` feeds **only the Overview**: `NewUiApp.tsx:2465-2496`
  renders `notice` -> `AlertBanner` inside the `<Overview>` branch of the
  ternary that starts at `NewUiApp.tsx:2219`.
- The **`AppPane`** alert slot (`NewUiApp.tsx:2357-2368`) is a different
  expression. The only banner it can produce is `driftAlert`
  (`NewUiApp.tsx:1644-1665`), whose cause is `status.kind === "drifted"` and
  whose copy is "Reconnect to restore protection" / "…config changed outside
  Gate…".

So: **the app pane produces an alert for drift and for nothing else.** The one
alert cause the section actually draws - master off - never appears on an app
pane. A user who turns routing off sees the correct sentence on Overview and a
bare, alert-free app pane on the screen that names the app the sentence is
about. **DIVERGES.**

Which causes each surface produces, laid out, since the split is the finding:

| Cause (`GroupMember.attention`) | Overview alert | App pane alert | Drawn on an app pane? |
| --- | --- | --- | --- |
| `master-off` | yes (`notices.ts:46`) | **no** | **yes, `116:30663`** |
| `drifted` | yes (`notices.ts:64`) | yes (`NewUiApp.tsx:1644`) | no frame on this canvas |
| `needs-trust` | yes (`notices.ts:57`) | no | no frame |
| `error` | yes (`notices.ts:84`) | no | no frame |

### A second-order bug the frame exposes

`driftAlert` is built from `drifted[0]` - the first drifted tool **anywhere**
(`NewUiApp.tsx:1640-1643` filters `tools`, not the open one). It is then handed
to whichever app pane is open. So with Codex drifted and Claude Desktop's pane
open, Claude Desktop's pane draws a card headed "Reconnect to restore
protection" whose body names **Codex**. The frame's alert is about the app whose
pane it is; the code's is about an unrelated app. Not something the file can
decide - it is a plain scoping miss - so it is recorded here rather than raised
with design.

The drawn card also carries **both** paging chevrons on a single-app alert.
`AlertBanner`'s `paging` prop is only passed when `drifted.length > 1`
(`NewUiApp.tsx:1655-1662`), and even then the handlers are `noop` with a comment
saying so. Inert as drawn, inert in code, for different reasons.

## `App / No data 1+ day state` (`228:89770`) - and one frame that is not a no-data frame

Three frames under this label. They are **not** three variants of one state; they
are three different states, and the section label only fits two of them.

### `408:25991` is a LOADING frame, not a no-data frame

All three KPI value slots are empty frames named `Frame 143` (208x28) with no
text child. Rendered (`408:26497` at 1:1, 726x90): each draws a **rounded grey
skeleton bar**. The chart below it (`408:26510`) is 24 bars all at exactly
`height="88.554"` - a flat placeholder, not data.

So this frame is the *pending* state, filed under a no-data heading. Worth
knowing, because it is the only frame on this canvas that draws a skeleton, and
it settles a question the design contract says was inferred rather than drawn:
CLAUDE.md's principle 6 says "a value still in flight draws a `Skeleton`" and
notes the loading state was "inferred from this rule rather than from a frame".
**It is drawn, here.** `AppPane.tsx:219` -> `StatTiles pending` and
`MessagesChart pending`, driven by `NewUiApp.tsx:2241-2246`. **IMPLEMENTED**,
and the inference was right.

It is also the only frame besides `121:35174` that draws the App-default
"Using Claude Desktop model / Gate protects requests, then leaves model choice to
Claude Desktop. No Gate credits used." row (`408:26223-26224`) - so that row is
drawn **twice** on this canvas and is still missing from the code. It
strengthens the `121:35174` finding above rather than adding a new one.

### `228:89241` and `272:1623` are the real no-data frames

Both draw the same emptied pane, and differ only in the feed:

| | `228:89241` | `272:1623` |
| --- | --- | --- |
| alert | drift card, "Reconnect to restore protection" / "This app's config changed outside Gate…" | same |
| Messages | **`0`** (`228:89336`) | **`0`** (`272:1721`) |
| Blocked/flagged | **`0`** (`228:89339`) | **`0`** (`272:1724`) |
| Tokens saved | **`n/a`** + `+$1.05` (`228:89343-44`) | **`n/a`** + `+$1.05` |
| chart | empty state: `Icon / BarChart` + "No messages sent in the last 24hrs" | same |
| Recent activity | **full table, 5 rows, 384 tall** | empty state: `Icon / Activity` + "No recent activity in the last 24hrs", 172 tall |

### Does this match `lib/activity.ts`'s null-vs-zero rule?


**Answered.** The rule is implemented, and the frames agree with it on two of
three counters while drawing one pairing the code cannot produce.

`lib/activity.ts:299-311` is explicit: a counter the gateway ANSWERED keeps its
value including zero (`state === "ok" ? (value ?? 0) : null`), and a counter it
declined becomes `null`. `metrics.tsx:113-116` then renders `null` as
`UNAVAILABLE`, which is the lowercase `n/a` at `metrics.tsx:97` - the spelling
these very frames settled (`228:89343`, `272:1728`).

So for the two no-data frames:

| Drawn | Means | Code produces it? |
| --- | --- | --- |
| Messages `0` | the section was read and the answer was zero | Yes - `requestsRouted.state === "ok"`, value 0 |
| Blocked/flagged `0` | same | Yes |
| Tokens saved `n/a` | the section was NOT read | Yes - `saved.state !== "ok"` |

That is principle 6 working exactly as written: two real zeroes and one absent
reading, on the same card, told apart.

### The one thing the code cannot draw: `n/a` beside `+$1.05`

Both frames pair the `n/a` percent with a `+$1.05` delta
(`228:89344`, and the same in `272:1623`). **That combination is unreachable.**
`tokensSavedPercent` and `tokensSavedAmount` are gated on the *same*
`saved.state === "ok"` (`activity.ts:301-311`):

- state ok -> percent is a number, so the tile shows `38%`, never `n/a`
- state not ok -> `tokensSavedAmount` is `null`, so `Stat` gets
  `delta={undefined}` and draws nothing beside the `n/a`

Nor can a null `fraction` produce it: `Math.round((saved.fraction ?? 0) * 100)`
is `0`, a reading, not `n/a`.

So either the mock pairs a placeholder percent with a leftover amount, or the
design intends the saved *amount* to survive a percent the gateway declined -
which would need the endpoint to report the two independently, and it does not.
Worth a designer's word before anyone "fixes" the tile to show both; the code's
behaviour is the defensible one under principle 6, because a figure with no
reading behind it is the thing that rule exists to prevent.

## Screen inventory

Consolidated from the sections above. Verdicts marked *(caption)* are read from
the section label and the frame's chrome rather than from an independent
frame-by-frame diff - the sections above carry the measured detail.

| Figma frame | Code path | Verdict |
| --- | --- | --- |
| `154:70994` `App / Dimensions` | none - annotation artboard, 1024x720 callouts | N/A, not a state |
| `661:15923` `App / Table guide` | `AppPane.tsx` activity table (columns audited in `docs/review-figma-dialogs-app.md`) | N/A, guide artboard |
| `116:31920` main, gate model | `AppShell.tsx:90-101` chrome + `AppPane` | IMPLEMENTED |
| `116:30204` main, + update | `{update && <UpdateBanner/>}` then `<RoutingBanner/>` | IMPLEMENTED |
| `116:30663` main, + update + alert | chrome IMPLEMENTED; the in-pane `master-off` alert is not - `notices.ts:46` feeds Overview only | **DIVERGES** |
| `121:35174` App default | `openModelChoice === "app"` | **DIVERGES** - missing the "Using Claude Desktop model / …No Gate credits used." row; code draws a credits row the frame does not |
| `127:43501` model switch modal open *(caption)* | `modelOverlay.kind === "picker"` (`NewUiApp.tsx:2034`) | IMPLEMENTED |
| `130:55863` gate model selected *(caption)* | `openModelChoice === "gate"` | IMPLEMENTED |
| `139:66117` single-model picker (`665:18400`) | `ModelPickerDialog` | **DIVERGES** - drawn with no footer and auto-close; code ships the confirm footer |
| `662:17579` OpenCode pane + model card | none - `multiProviderSlugs` withholds every model prop | **DRAWN-ONLY** |
| `665:18498` OpenCode picker (`665:19064`) | none - unreachable for OpenCode | **DRAWN-ONLY** |
| `671:19431` OpenCode, four models chosen | none - unreachable for OpenCode | **DRAWN-ONLY** |
| `408:25991` "no data" row, actually LOADING | `pending` -> `StatTiles`/`MessagesChart` skeletons | IMPLEMENTED - and it is the frame that finally *draws* the loading state CLAUDE.md had recorded as inferred |
| `228:89241` no data, feed populated | zeroes + `n/a` per above; activity table with rows | IMPLEMENTED except the `n/a`+delta pairing |
| `272:1623` no data, feed empty | same, plus `activity.length === 0` -> `EmptyNote` | IMPLEMENTED except the `n/a`+delta pairing |

## Dead ends

1. **`modelOverlay.then === "remember"` is unreachable**, so `NewUiApp.tsx:2056`
   (`else void saveModel("tool", ids)`) can never run - "Change model" only
   exists inside `gateActive`. Dead code following a decision the pane already
   made; the design agrees there is no such entry. Delete it or correct the
   comment, which currently describes a flow the app does not have.
2. **Three drawn screens with no way in** - the OpenCode trio, for the reason in
   Q2. Not a dead end in the code's own terms; a dead end in the file's.
3. No code state with no way out was found on this canvas. Every model overlay
   has an explicit dismiss (`onDismiss` on both dialogs), `modelBusy` gates
   re-entry during a write, and `modelError` renders in the pane rather than
   behind anything.

## Could not determine

- **Whether the designer wired these frames as a prototype.** The canvas has no
  connector nodes and this MCP surface does not report reactions, so every
  sequence above is read from the code and the frame content.
- **Whether the `App / No data 1+ day state` label is meant to cover the
  loading frame** (`408:25991`) or whether that frame was filed there by
  accident. It draws skeletons, not emptied readings, which is a different
  state from its two neighbours.
- **What `665:18400`'s "3 models selected" is doing** above a single checked
  row. It does not affect the footer finding, which is structural, but the
  count and the checkboxes disagree within one frame.
- **Whether the single-model auto-close is still wanted** given AG-590. A click
  that both applies and closes cannot also ask "you are removing your last
  model", so the two annotations and that requirement cannot all three hold.
