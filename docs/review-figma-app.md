# Figma audit: `Flows / App` (116:30199) vs the built App pane

Read 2026-08-30 from the MCP metadata dump of `116:30199` plus four screenshots
(`665:19064`, `662:17782`, `661:15923`, `272:1718`). Five MCP calls total.
Geometry and copy below come from the metadata XML (layer names are the text
content) except where a screenshot is cited; nothing was guessed.

## Summary

The page has moved more since 2026-08-21 than any prior App-page sync. Two
structural changes and one whole new section: **Recent activity was redrawn
again** (`661:*`) - it gained a `Type` column and lost the `Message` column, so
the built table now differs from every frame on the page; and the file gained
**`App / Select multiple models (Opencode)`** (`671:19286`, frames `662:17579`,
`665:18498`, `671:19431`), which draws the Model selection card for OpenCode -
exactly the tool the build withholds the card from. The Model selection card
itself was rebuilt in the file (`683:*`, `670:*`) around a multi-model list with
a `3 of 14 models enabled` footer, and the picker dialog was redrawn twice
(`665:18400`, `665:19064`) with checkbox rows, new copy and a new footer. The
picker's *behaviour* (multi-select, save-on-confirm, provider sort) is already
built and correct; its chrome and copy are not. The stat trio, the Gate credits
row, `UseGateModelDialog` and the chart legend all still match. The App page's
copy of the Messages chart was **not** touched by the `706:*` redraw and still
draws bare `1..24` ticks - the build follows the newer Overview node, which is
the right call, but the file now disagrees with itself.

## Findings

| What | Figma (node + drawn value) | Built | Verdict |
| --- | --- | --- | --- |
| Model card withheld from multi-provider tools | Whole new section `App / Select multiple models (Opencode)` (`671:19286`); `662:17579` / `665:18498` / `671:19431` all draw the Model selection card on an **OpenCode** pane, subtitle `Choose whether OpenCode or Gate selects the AI models for requests` (`662:17786`), options `App default` / `Gate models` (`662:17801`) | `src/NewUiApp.tsx:2164` spreads `{}` for `multiProviderSlugs`, so OpenCode gets no card at all; `AppPane.tsx:252` renders `ModelSelection` only when all three handlers arrive | **DRIFT** |
| Recent activity: new `Type` column | `661:15923` (`App / Table guide`) and every instance (`661:16231`, `661:16449`, `661:16994`, `662:17826`, `665:18768`): header `Time` / `Type` / `Security` / `Model` / `Action`. `Type` is a 20px coloured glyph + label - `Injection` (red ShieldAlert), `PII` (green UserRound), `Credential` (violet KeyRound). Cell x=148, w=124 | `AppPane.tsx:638-659` renders `Time` / `Security` / `Model` / `Message` / `Action`. No `type` field exists on `ActivityEntry` (`src/lib/toolEventRow.ts:16-55`), so this needs a gateway field before it can be drawn | **DRIFT** |
| Recent activity: `Message` column removed | No `Message` column in any `661:*` instance; the row is Time / Type / Security / Model / Action only, and the Model cell carries the display name (`Claude Opus 5`), not a session reference | `AppPane.tsx:653-655` header, `:726-739` cell (title + mono `reference`). Pinned by `AppPane.test.tsx:213` | **DRIFT** |
| Model card body: the enabled set, not one row | `662:17782` / `683:20164`: one bordered box holding a 2-column grid of every enabled model (vendor over mono id - `Anthropic`/`gate/opus 5`, `Deepseek`/`gate/deepseek-v4-flash`, `Moonshot`/`gate/kimi-k3`), then a divider row `3 of 14 models enabled` (`670:19163`) with a `Choose models` button. Heading `Current Gate models` (`662:17806`) | `AppPane.tsx:452-481`: one `InfoRow` for `gateModel` with a `Change model` button; `alsoEnabled` drives only the heading's plural (`:453`). No count line, no set | **DRIFT** |
| Picker dialog title | `665:19069` `Choose Gate models` (multi); `665:18405` `Choose a Gate model` (single) | `dialogs.tsx:634` always `Choose a Gate model` | **DRIFT** |
| Picker dialog subtitle | `665:19070` `OpenCode will be able to use these models`; `665:18406` `Claude desktop will be able to use these models` | `dialogs.tsx:635` `` `${appName} may use any model you enable here` ``. The 2026-08-21 recorded deviation (drawn subtitle "Claude Desktop uses on Gate model" was not a sentence) is **obsolete** - the redraw is a sentence, so the file wins again | **DRIFT** |
| Picker primary button | `665:19137` reads `Apply selections` (screenshot) | `dialogs.tsx:643` `Save models` | **DRIFT** |
| Picker footer: bulk-clear control | `682:20043` at the footer's left edge, primary-blue link style, `Unselect all (4)` (screenshot of `665:19064`) | Nothing. Footer is Cancel + primary only | **DRIFT** |
| Picker row control shape | `665:19088` / `665:19094` etc.: `Icon / SquareCheck` and `Icon / Square`, 20px - square checkboxes. Same in the single-app frame `665:18424` / `665:18430` | `dialogs.tsx:818-829` draws `circleCheck` / an empty `rounded-full` - radio shapes on a `role="checkbox"`. The comment at `dialogs.tsx:531-535` ("139:66117 draws the radios of the single-model era") is stale: `139:66117` now contains `665:18400`, which draws checkboxes | **DRIFT** |
| Unavailable counter casing | `228:89343` and `272:1728` render lowercase **`n/a`** (confirmed by screenshot of `272:1718`) | `metrics.tsx:94` `const UNAVAILABLE = "N/A"`. `plans/new-app-ui-figma.md` recorded `N/A` from this frame; that was a transcription slip, not a design change | **DRIFT** (cosmetic) |
| Recent activity empty copy | `272:1982` `No recent activity in the last 24hrs` | `AppPane.tsx:632-636` `No recent messages`, with a reasoned comment (`:626-631`) that the feed outlives the 24h window. The argument is sound but the deviation is **not recorded in the plan** - the plan (line ~2365) still states the drawn copy is what ships | **DRIFT** (or record it) |
| `Gate model` vs `Gate models` option label | Inconsistent within Claude Desktop's own frames: `Gate model` at `683:20450`, `683:20507`, `683:20393`, `134:60786`, `154:71289`, `130:56194`, `408:25487`, `408:25722`; `Gate models` at `683:20288`, `683:20280`, `683:20272` | `AppPane.tsx:423` always `Gate model`; heading pluralises on `alsoEnabled` (`:453`) | **DESIGN BUG** |
| `App/OpenCode/gate-models-2` names the wrong app | `665:18498` is an OpenCode frame, but its card reads `Choose whether Claude Desktop or Gate selects the AI model for requests` (`665:18704`) and labels the option `Gate model` (`665:18719`) while its own heading reads `Current Gate models` (`671:19361`). Copy-pasted from the Claude Desktop frame | n/a | **DESIGN BUG** |
| Single-app picker is drawn multi-select | `665:18400` (inside `139:66117`, the `App / Select a model` section) is titled `Choose a Gate model` yet draws checkboxes and a count line `3 models selected` (`665:18416`), with the plural subtitle `…these models`. Its frame annotation `139:66782` still says "When the user selects a model it automatically closes the modal", contradicting `671:19288` ("the modal stays open until the user confirms") | Build follows the multi-select behaviour (`dialogs.tsx:559-563`), which is the newer annotation and the right one | **DESIGN BUG** |
| `N selected` chip on the count line | `682:20038` / `682:20040` / `682:20041` exist in the tree (`4` + `selected`) but do **not** render - the screenshot of `665:19064` shows the count line with nothing to its right | Built has no chip; it states the set in a `ModalNote` (`dialogs.tsx:838-848`) | **DESIGN BUG** (hidden node; nothing to build) |
| Row `View` button height | `661:16257` and every sibling: 68x24. The `Button` component set has only `default` (36) and `sm` (32) | `AppPane.tsx:742-749` uses `h-8` (32) | **DESIGN BUG** (out-of-set size; build is right to snap to `sm`) |
| `Type` glyph colour | `661:16248` etc. render red / green / violet per category (screenshot of `661:15923`), against CLAUDE.md's "row icons are `base.foreground`" | n/a (column not built) | **DESIGN BUG** - flag when building the column; the frame wins per the standing rule, but it contradicts the file's own icon rule |
| Messages chart on this page | `116:30231` / `662:17610` are pre-redraw copies: 24 columns, bare numeric ticks `1`..`24` (`116:30242`…`116:30403`). The `706:*` redraw did not reach this page | `metrics.tsx:81` `hourTick` prints `HH:00` everywhere. Build follows the newer node | **DESIGN BUG** (file disagrees with itself; build is right) |
| Chart legend | `116:30404`: full-width rule, `Total messages` / `Blocked` / `Flagged` / `Redacted`, item pitch 24px (0→132→222→312 over 108/66/66 widths), swatch gap 8px (12px swatch, label at x=20) | `metrics.tsx:334-341` `gap-6`, `size-3` swatch, `gap-2`, `border-t … pt-4`, `text-base-foreground` | **OK** |
| Stat trio | `121:35714` / `228:89333` / `272:1718`: `Messages` / `Blocked/flagged` / `TOKENS SAVED`; no-data reads `0` / `0` / `n/a` with the `+$1.05` delta layer hidden | `metrics.tsx:117-131`; no delta beside the unavailable reading | **OK** (except the casing row above) |
| Gate credits row | `683:20466` / `663:18379`: CreditCard glyph, `Gate credits:` + `$10.25 available`, 110px action button with external glyph | `AppPane.tsx:483-495` | **OK** |
| `UseGateModelDialog` | `130:48278` (512 wide): `Use a Gate model for Claude Desktop?`, `Your next requests will use Constellation Gate PAYG credits`, vendor-over-id row (`686:23565`) with `PAYG` pill, credits block, `Claude Desktop's own model preference is not changed. You can return to App default at any time.` | `dialogs.tsx:886-949` verbatim, at `width={512}` | **OK** |
| Security pill set | `661:16251` etc.: `ERROR` / `REDACTED` / `FLAGGED` / `ALLOW` / `BLOCKED`, 24px tall, 8/4 padding | `AppPane.tsx:79-89` + `Pill` (`:799-818`), with the recorded `gray-600` contrast substitution for `ALLOW` | **RECORDED** |
| `・400+ in Gate AI` on the count line | `665:19079` / `665:18415` `Showing 8 of 14 models・400+ in Gate AI` (renders as `·`) | `dialogs.tsx:719-726` deliberately drops the clause until per-tool filtering exists, with the reason at the call site | **RECORDED** |
| Canonical model ids vs drawn `gate/…` | `665:19087` etc. draw `gate/opus 5`, `gate/kimi-k3` | `dialogs.tsx:525-529` keeps real `provider/model` ids | **RECORDED** |

## Notes

- `plans/new-app-ui-figma.md` has **no record of AG-589 / AG-590 / AG-592** or of
  the multi-model picker work that `dialogs.tsx` cites, and no sync entry for
  this page after 2026-08-21. The picker's behaviour was built against a read
  the plan never captured, which is why several of the drifts above look like
  regressions and are really just an unrecorded partial migration.
- Nothing on this page was unreadable. The one thing I could not settle from the
  file is what feeds a `Type` value: no gateway field in `toolEventRow.ts`
  carries a guardrail category, so DRIFT 2 is blocked on data, not on markup.
