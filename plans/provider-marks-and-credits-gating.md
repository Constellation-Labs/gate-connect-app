# Provider marks for Gate models, and gating the credits row

## Context

Two defects in the App pane's **Model selection** card and the model picker.

**1. Provider marks never render.** Every surface that draws a model id already
has a slot for the provider's mark, and every one of them falls back to a cube.
`GateModelOption.logo` (`src/components/gc/dialogs.tsx:899`) and `GateModel.logo`
(`src/components/gc/AppPane.tsx:52`) are both optional `ReactNode`s that **no
call site has ever passed**. `NewUiApp.tsx:3481-3494` builds the pane's model
object as `{ vendor, ids }` with no `logo`; `NewUiApp.tsx:3192` passes the
catalogue straight through, and `lib/toolModels.ts`'s `GateModel` has no logo
field to pass. The activity table draws a grey one-letter monogram instead
(`VendorMark`, `AppPane.tsx:365-379`), whose own comment says "this repo carries
no provider logos ... swap this for the real SVGs when they land".

Logged as **M16** in `docs/review-figma-dialogs-app.md:414-423`.

**2. The credits row shows under App default.** The `Gate credits: ` / `<Plan>
plan` `InfoRow` (`AppPane.tsx:568-597`) renders unconditionally, including while
the app is on its own model and Gate is spending nothing. It sits below a radio
that has just said Gate is not serving this app.

Outcome: a model row identifies its provider at a glance, and the balance
appears only when there is a balance being spent.

## What the frames actually draw (measured 2026-09-16)

The nodes supplied settle every open question, including one this plan had
queued for design.

**`665:18400` `card/choose-model`** - the row mark is a bare **16px** frame, 8px
before the id, no wrapper (`665:18420`: mark at x=0 w=16, text at x=24). That is
what `dialogs.tsx:1124` already lays out. The marks are **full colour**:

| Frame layer | Fill |
|---|---|
| `anthropic 2` (`665:18421`) | `#E8704E` |
| `deepseek-color 1` (`671:19272`) | `#4D6BFE` |
| `alibaba-color 1` (`665:18457`) | `#FF6003` |
| `moonshot 1` (`671:19259`) | black |

Each is one path in one colour: "full colour" means each mark carries its own
brand colour, not that any mark is multi-coloured. The raw exports also contain
a `fill="white"` rect, which is the clip region rather than a visible layer and
is dropped when vendoring.

**This kills the colour question.** `docs/review-figma-dialogs-app.md` M16 reads
the layer names as a mono/colour split (`anthropic 2` vs `deepseek-color 1`), but
`anthropic 1` (`130:48325`, the confirmation) and `anthropic 2` are **both
`#E8704E`**. The "-color" suffix is a lobe-icons filename, not a variant choice.
Every drawn model mark is colour; nothing goes to design.

**`130:48278` `card/organization`** - the confirmation draws its mark at **20px**
(`130:48325`) inside a 36px `icon-wrapper`, not 16px. Same asset, same colour.

**`408:14159` `logo` / `408:14180` `logo-wrapper`** - this is the **tool** set,
not the provider set: 8 variants (claude, claudecode, codex, openai, opencode,
openrouter, openclaw, moonshot), every one exported at **`#F9FAFB` monochrome**
because it sits on the rail's dark 32px tile. That is exactly what
`BrandMark.tsx` renders today via `currentColor`, and it is the reason provider
marks need a **separate** file rather than more `BrandName` entries - see below.

**`qwen` takes the Alibaba mark.** `665:18457` draws `alibaba-color 1` beside
`gate/qwen3-6-35b-a3b`, though lobe-icons publishes a distinct `qwen-color`.
CLAUDE.md: the file wins. Follow the frame.

**The fallback is the one already in the code.** Figma's `Icon / Boxes`
(`665:18402`, `130:48281`) is the repo's `<Icon name="cube" />`. No new glyph.

## What the catalogue actually holds

Measured against `https://gateway-staging.constellationgate.ai/v1/models`,
2026-09-16: **413 models across 50 vendor namespaces**, long-tailed - the top 9
are 90% of rows, 23 namespaces hold one model each. The namespaces are not clean:
`x-ai`/`xai`, `z-ai`/`zai`, `moonshot`/`moonshotai` and
`bytedance`/`bytedance-seed` all appear as separate `owned_by` values for the
same company, so an alias map is required, not optional.

With the table below, **~96% of catalogue rows resolve to a mark**. The rest
(sakana, rekaai, writer, perceptron, thinkingmachines, sao10k, thedrummer,
undi95, gryphe, mancer, meituan, anthracite-org, cognitivecomputations,
inclusionai) have no published mark and take the cube, as decided.

## Change 1: a `ProviderMark` component

**New file: `src/components/gc/ProviderMark.tsx`.**

A sibling of `BrandMark.tsx`, deliberately **not** an extension of it. The
measurement above is the argument: the `logo` set is monochrome `#F9FAFB` so the
dark tile can colour it, which is why `BrandMark` renders
`<svg fill="currentColor">` (`BrandMark.tsx:13-14`, `Sidebar.tsx:626-633`).
Provider marks are fixed multi-fill colour on a white row. Merging them would
either break the rail or push a colour-mode flag through a component with no
other reason for one. `anthropic` and `openai` therefore exist in both files with
different treatments; that is the point, and the header comment should say so.

### Inline paths, matching every other vector component in the repo

All four existing vector components inline their path data in a typed `Record`
and carry provenance in a header comment - `Icon.tsx` (139 paths),
`BrandMark.tsx` (11), `ConstellationHexMark.tsx` (1), `GateAiLogoMark.tsx` (9).
There are no SVG files on disk, no asset imports and no icon dependency
anywhere in the tree. `ProviderMark` follows that, for three reasons:

1. **`GateAiLogoMark.tsx` is the exact precedent** - a full-colour,
   Figma-exported brand mark with fixed fills, inlined, provenance in the
   header. Same shape of problem, already solved here.
2. **It is also the React/SVG default for icon systems**: styleable,
   accessible, no extra request, no decode, no FOUC. `<img>` is for content
   images and gives up CSS control over the graphic entirely.
3. **Measured: there is no performance case for the alternative.** 413 rows in
   Chromium, real staging vendor distribution, median of 15 runs with layout and
   paint forced - inline `<svg>` 15.7ms / 3072 nodes / 416KB markup, `<img>`
   with Vite-inlined data URIs 16.1ms / 2525 nodes / **529KB** (base64 is ~33%
   larger than the source and repeats per row). `<img>` with asset URLs is
   16.1ms / 102KB but trades the bytes for requests. Render time is a wash.

**Drop the `clipPath` wrappers when vendoring**, as `BrandMark.tsx:49-51`
already documents ("the svg element clips to its viewBox on its own"). That
removes every `id` from the markup, which matters at this scale:
`GateAiLogoMark` needs `useId` to stop two instances capturing each other's
gradient defs, and a 413-row list drawing ~34 marks would hit that hazard hard.
With the clips dropped, the four Figma marks carry no ids at all.

**Store the viewBox per mark.** The Figma exports are `0 0 16 16`; lobe-icons is
`0 0 24 24`. Keeping each mark's own box avoids rewriting coordinates by hand,
which is the step most likely to introduce a silent shape error.

```tsx
/**
 * Full-colour provider marks for Gate model rows (Figma `card/choose-model`
 * 665:18400 at 16px; the confirmation's `130:48325` at 20px).
 *
 * Colour, not `currentColor` - see BrandMark.tsx, the monochrome counterpart
 * for the rail's dark app tiles, whose own `logo` set (408:14159) exports at
 * #F9FAFB for that reason. The two files overlap on anthropic and openai on
 * purpose: different surface, different treatment.
 *
 * anthropic, deepseek, alibaba and moonshot are exported from the Figma file
 * (node ids and fills below); every other mark is vendored from
 * lobehub/lobe-icons (MIT), the set the designer built the frames from. Marks
 * remain their owners' trademarks, used here to identify the models they name.
 */

type Mark = { viewBox: string; paths: JSX.Element };
const PATHS: Record<ProviderName, Mark> = { ... };

export function ProviderMark({ name, size = 16 }: { name: ProviderName; size?: number })

/** The mark for a vendor namespace, or undefined for one with none - callers
 *  keep their cube fallback. */
export function providerMarkFor(vendor: string, size?: number): JSX.Element | undefined
```

`providerMarkFor` lowercases its argument and resolves through
`PROVIDER_BY_VENDOR`. Keep that table in this file, exactly as `BRAND_BY_SLUG`
lives in `BrandMark.tsx` - "the branding tables stay in the branding file"
(`BrandMark.tsx:126-128`). Unlike `BrandMark`, the `<svg>` carries **no `fill`**
of its own; each path keeps the one it was exported with.

**If this file outgrows itself**, the fix is `vite-plugin-svgr` applied to
`Icon`, `BrandMark` and `ProviderMark` *together* - one pattern across all of
them, not a bespoke one for the newest file. Do not adopt it for this file
alone.

### Sources for the four drawn marks

The four the frames draw are exported from the Figma file, because those are the
pixels design signed off. Their path data is in the **appendix at the end of this
document** - transcribed rather than linked, because `get_design_context` returns
asset URLs Figma keeps for only 7 days.

Each is a **single path, single fill, on a `0 0 16 16` viewBox**: `anthropic`
`#E8704E`, `deepseek` `#4D6BFE`, `alibaba` `#FF6003`, `moonshot` black. No
gradients, no multi-layer colour. ("Full colour" means each mark carries its own
brand colour, not that any one mark is multi-coloured - the `fill="white"` in
the raw exports is the clip region's rect, which is dropped.)

**The confirmation's 20px mark is not a fifth asset.** Its export is the same
path scaled uniformly by 1.25 (`9.218` -> `11.5225`, `16` -> `20`), verified
coordinate by coordinate, so size is a viewBox concern.

### Sources for the rest, and the alias table

`packages/static-svg/icons/<asset>.svg` in lobehub/lobe-icons (906 assets, MIT).
Prefer the `-color` variant where one exists.

| `owned_by` | asset | | `owned_by` | asset |
|---|---|---|---|---|
| `qwen` | **Figma `alibaba`** | | `perplexity` | `perplexity-color` |
| `openai` | `openai` | | `aion-labs` | `aionlabs-color` |
| `google` | `google-color` | | `cohere` | `cohere-color` |
| `mistralai` | `mistral-color` | | `ibm-granite` | `ibm` |
| `meta-llama` | `meta-color` | | `inception` | `inception` |
| `deepseek` | **Figma** | | `kwaipilot` | `kwaipilot-color` |
| `anthropic` | **Figma** | | `microsoft` | `microsoft-color` |
| `z-ai`, `zai` | `zhipu-color` | | `morph` | `morph-color` |
| `minimax` | `minimax-color` | | `poolside` | `poolside-color` |
| `amazon` | `aws-color` | | `relace` | `relace` |
| `moonshotai`, `moonshot` | **Figma** | | `stepfun` | `stepfun-color` |
| `nvidia` | `nvidia-color` | | `upstage` | `upstage-color` |
| `tencent` | `tencent-color` | | `ai21` | `ai21` |
| `x-ai`, `xai` | `grok` | | `baidu` | `baidu-color` |
| `bytedance-seed`, `bytedance` | `bytedance-color` | | `nousresearch` | `nousresearch` |
| `xiaomi` | `xiaomimimo` | | `arcee-ai` | `arcee-color` |
| `inference-net` | `inference` | | | |

Drop unreferenced `<defs>`/clip wrappers the way `BrandMark.tsx:49-51` already
documents for `openclaw`, and keep per-path fills. Do **not** rewrite
coordinates to a common viewBox: store each mark's own box (see above), since
hand-rescaling is the step most likely to introduce a silent shape error.

## Change 2: wire the marks, and delete the dead `logo` props

Resolve the mark **at the render site**, from the `vendor` the component already
holds, rather than plumbing a `ReactNode` down from `NewUiApp`. `NewUiApp`
derives the vendor by `id.split("/")[0]` in two places already (`:3209`, `:3489`)
purely because the catalogue is only loaded while the picker is open - adding a
third derivation there to build a `logo` node repeats that workaround for no
gain. So **remove `logo?: ReactNode`** from `GateModelOption` (`dialogs.tsx:899`)
and `GateModel` (`AppPane.tsx:52-53`), and the unused `vendorLogo` prop from
`UseGateModelDialog` (`dialogs.tsx:1430`, `:1450`). None has ever been passed;
the review doc already flags them as dead.

| File:line | Today | After |
|---|---|---|
| `dialogs.tsx:1124` (picker row) | `model.logo ?? <Icon name="cube" size={16} />` | `providerMarkFor(model.vendor) ?? <Icon name="cube" size={16} />` |
| `dialogs.tsx:1470` (`UseGateModelDialog`) | `vendorLogo ?? <Icon name="cube" size={16} />` | `providerMarkFor(vendor, 20) ?? <Icon name="cube" size={20} />` - the frame's 20px |
| `AppPane.tsx:540` (Current Gate model) | `gateModel.logo ?? <Icon name="cube" size={16} />` | `providerMarkFor(gateModel.vendor) ?? ...` |
| `AppPane.tsx:868` (`VendorMark`, activity) | grey letter monogram | `providerMarkFor(provider) ?? <Icon name="cube" size={16} />` |

`VendorMark` (`AppPane.tsx:347-379`) keeps its two other behaviours verbatim: the
empty `size-4` spacer when `provider` is null, and the `sr-only` provider name
beside the `aria-hidden` glyph. Only the glyph changes; rewrite the doc comment,
which currently asserts the repo has no provider logos.

The set case is unchanged and deliberate: `AppPane.tsx:537` and
`dialogs.tsx:1478-1480` both draw no mark for a multi-model set, because one
glyph cannot stand for several vendors.

**Measured, pre-existing, not fixed here.** `ModalSubject`'s tile
(`Modal.tsx:427-430`) is `size-10` at `rounded-sm` (6px in this repo's scale);
the frame's `icon-wrapper` (`130:48324`) is **36px at 4px** on a white card
ground. It is a shared component across several dialogs, so it is outside these
two asks - record it in `docs/review-figma-dialogs-app.md` as a new measured
item.

## Change 3: gate the credits row on the Gate branch

`src/components/gc/AppPane.tsx`, `ModelSelection`.

The card already computes `const gateActive = choice === "gate";` at `:439`, and
`:524` opens a `{gateActive && (...)}` block holding the "Current Gate model"
heading and its row. Move the credits `InfoRow` and its `div` wrapper
(`:569-597`) **inside that same block**, after the model row. No new state, no
new condition - `gateActive` is false for both App default and `choice === null`,
which is exactly the decided behaviour.

Add a comment in the spirit of the one at `:517-523`: the balance is what the
Gate branch spends, so a card describing the app's own model has no business
naming it, and "Add credits" / "Manage billing" act on a billing relationship
this app is not using.

## Tests

**`src/components/gc/AppPane.test.tsx`**
- `:503` "reads N/A for a credit balance nothing reports" boots on the default
  `modelChoice="app"` (`pane()` at `:44-62`) and asserts the credits line. It
  must pass `modelChoice: "gate", gateModel: model`.
- The two plan tests (`:475`, `:483`) already pass `modelChoice: "gate"` - no change.
- New: credits row absent under App default; absent under `modelChoice={null}`;
  present under `gate`.
- New: the Current Gate model row renders a mark for a known vendor, the cube for
  an unknown one.

**`src/components/gc/dialogs.picker.test.tsx`** - fixtures at `:25-27` are
`{id, vendor, tags}`, so removing `logo` needs no fixture change. Add a case
asserting the `anthropic/...` row carries a mark and a `sao10k/...` row the cube.

**`e2e/new-ui-model-picker.spec.ts`** - the whole `new UI model card credits`
describe (`:427-485`, three tests) boots on default preferences, which is App
default, and asserts the credits line is visible. All three now need the app
switched to a Gate model first. `:121` already performs that switch ("Use Gate
credits") and is the pattern to lift into a helper alongside `openApp` (`:59`).
`:628` (`no Gate credits left`) is the attention line, not this row - verify it
is reached on the Gate branch.

**New: `src/components/gc/ProviderMark.test.tsx`** - alias resolution
(`x-ai`/`xai` to one mark, `moonshot`/`moonshotai` to one), `qwen` resolving to
the Alibaba mark, case insensitivity, and `undefined` for an unknown vendor.

## Docs to update

- `docs/review-figma-dialogs-app.md` - M16 closes; record that the mono/colour
  reading was wrong (both anthropic layers are `#E8704E`) and add the
  `ModalSubject` tile item above.
- `CLAUDE.md` - the "Row icons" bullet gains a third case: model rows carry
  full-colour provider marks, a per-surface rule like the other two.
- `plans/new-app-ui-figma.md` - migration status.
- No new entry in `docs/figma-questions-for-design.md`: the nodes answered it.

## Verification

1. `pnpm test` - the unit suites above.
2. `pnpm test:e2e -- new-ui-model-picker` - the rewritten credits describe.
3. `pnpm build` (runs `tsc --noEmit`) - catches any remaining `logo=` passer.
4. `pnpm app:local` (staging, the gateway this catalogue was measured against):
   open the Claude pane and confirm the credits row is **absent** under App
   default; open **Change model** and confirm the 413-row list draws real marks
   for qwen / openai / google / anthropic / deepseek and a cube for `sao10k`;
   select an Anthropic model and confirm the confirmation dialog draws the 20px
   mark; accept, and confirm the credits row appears and the Current Gate model
   row carries the mark; confirm the Recent activity table's model column draws
   marks rather than letters.
   No keychain path is touched, so `app:local` is sufficient.
5. Compare the four Figma-sourced marks against `665:18418` at 16px and
   `130:48324` at 20px. The lobe-icons ones cannot be checked this way - eyeball
   them at 16px, since several `-color` assets are multi-path and may need
   simplifying. This is the one part that no test covers.

## Appendix: the four Figma-exported marks

Transcribed here because `get_design_context` returns asset URLs Figma keeps
for only 7 days, and these are the shapes design signed off. Copy them into
`ProviderMark.tsx`; this appendix can be deleted once it exists.

Each is a **single path, single fill, `0 0 16 16`** - no gradients, no ids, no
clip needed. Re-export from `665:18418` / `130:48324` only if the frames change.

### `anthropic` - Figma 665:18421 (`anthropic 2`), viewBox `0 0 16 16`, fill="#E8704E" fillRule="evenodd" clipRule="evenodd"

```
M9.218 2.34668H11.62L16 13.3333H13.598L9.218 2.34668ZM4.37933 2.34668H6.89067L11.2707 13.3333H8.82133L7.926 11.026H3.34467L2.44867 13.3327H0L4.38 2.34801L4.37933 2.34668ZM7.134 8.98601L5.63533 5.12468L4.13667 8.98668H7.13333L7.134 8.98601Z
```

### `deepseek` - Figma 671:19272 (`deepseek-color 1`), viewBox `0 0 16 16`, fill="#4D6BFE"

```
M15.832 2.988C15.6627 2.90533 15.5893 3.06333 15.4907 3.144C15.4567 3.17 15.428 3.204 15.3993 3.23467C15.1513 3.49933 14.862 3.67267 14.484 3.652C13.9313 3.62133 13.4593 3.79467 13.042 4.21733C12.9533 3.696 12.6587 3.38533 12.2107 3.18533C11.976 3.08133 11.7387 2.978 11.574 2.752C11.4593 2.59133 11.428 2.412 11.3707 2.236C11.334 2.12933 11.2973 2.02067 11.1753 2.00267C11.042 1.982 10.99 2.09333 10.938 2.18667C10.7293 2.568 10.6487 2.988 10.6567 3.41333C10.6747 4.37067 11.0787 5.13333 11.882 5.67533C11.9733 5.73733 11.9967 5.8 11.968 5.89067C11.9133 6.07733 11.848 6.25867 11.7907 6.446C11.754 6.56533 11.6993 6.59067 11.5713 6.53933C11.1388 6.35346 10.746 6.08642 10.414 5.75267C9.84267 5.20067 9.32667 4.59133 8.68267 4.114C8.53345 4.00374 8.38024 3.89901 8.22333 3.8C7.56667 3.162 8.31 2.638 8.482 2.576C8.662 2.51067 8.544 2.288 7.96267 2.29067C7.38133 2.29333 6.84933 2.48733 6.17133 2.74667C6.07058 2.78527 5.96692 2.81581 5.86133 2.838C5.22775 2.71862 4.57977 2.6957 3.93933 2.77C2.68267 2.91 1.67933 3.50467 0.941333 4.51867C0.0546666 5.73733 -0.154 7.12267 0.101333 8.56667C0.37 10.0893 1.14733 11.35 2.34133 12.3353C3.58 13.3573 5.006 13.858 6.63333 13.762C7.62133 13.7053 8.722 13.5727 9.96267 12.522C10.276 12.678 10.604 12.74 11.1493 12.7867C11.5693 12.826 11.9733 12.7667 12.286 12.7013C12.776 12.5973 12.742 12.1433 12.5653 12.0607C11.1287 11.3913 11.444 11.664 11.1567 11.4433C11.8873 10.5793 12.9873 9.682 13.418 6.77467C13.4513 6.54333 13.4227 6.398 13.418 6.21133C13.4153 6.098 13.4413 6.05333 13.5713 6.04067C13.932 6.00325 14.2819 5.89568 14.6013 5.724C15.532 5.21533 15.908 4.38067 15.9967 3.37933C16.01 3.226 15.994 3.06867 15.832 2.988ZM7.72067 12C6.328 10.9053 5.65267 10.5447 5.374 10.56C5.11267 10.576 5.16 10.874 5.21733 11.0687C5.27733 11.2607 5.35533 11.3927 5.46467 11.5613C5.54067 11.6727 5.59267 11.8387 5.38933 11.9633C4.94067 12.2407 4.16133 11.87 4.12467 11.852C3.21733 11.3173 2.458 10.612 1.924 9.64733C1.408 8.71867 1.108 7.72267 1.05867 6.65933C1.04533 6.402 1.12067 6.31133 1.37667 6.26467C1.71274 6.20035 2.05708 6.19157 2.396 6.23867C3.81733 6.44667 5.02667 7.082 6.04133 8.088C6.62 8.66133 7.058 9.346 7.50933 10.0153C7.98933 10.726 8.50533 11.4033 9.16267 11.958C9.39467 12.1527 9.57933 12.3007 9.75667 12.4093C9.222 12.4693 8.33 12.4827 7.72067 12ZM8.38733 7.70667C8.38722 7.67355 8.39517 7.6409 8.41049 7.61155C8.42582 7.58219 8.44806 7.557 8.4753 7.53817C8.50254 7.51933 8.53395 7.50741 8.56683 7.50343C8.5997 7.49945 8.63306 7.50354 8.664 7.51533C8.70344 7.52948 8.73748 7.55558 8.76138 7.59C8.78529 7.62442 8.79785 7.66543 8.79733 7.70733C8.79742 7.7344 8.79213 7.76122 8.78175 7.78623C8.77137 7.81123 8.75612 7.83392 8.73688 7.85297C8.71765 7.87202 8.69481 7.88705 8.66971 7.89718C8.64461 7.90732 8.61774 7.91235 8.59067 7.912C8.5638 7.91209 8.53719 7.90682 8.51238 7.8965C8.48758 7.88618 8.46508 7.87102 8.44621 7.8519C8.42733 7.83278 8.41246 7.81009 8.40247 7.78515C8.39248 7.76021 8.38689 7.73353 8.38733 7.70667ZM10.4607 8.77067C10.3273 8.82467 10.1947 8.87133 10.0673 8.87733C9.87577 8.88401 9.68779 8.82418 9.53533 8.708C9.35267 8.55467 9.222 8.46933 9.16733 8.20267C9.14849 8.07235 9.15209 7.93977 9.178 7.81067C9.22467 7.59267 9.17267 7.45267 9.01867 7.326C8.894 7.222 8.73467 7.19333 8.56 7.19333C8.5002 7.18986 8.44211 7.17202 8.39067 7.14133C8.31733 7.10533 8.25733 7.01467 8.31467 6.90267C8.33333 6.86667 8.42133 6.77867 8.44267 6.76267C8.68 6.628 8.954 6.672 9.20667 6.77333C9.44133 6.86933 9.61867 7.04533 9.874 7.29467C10.1347 7.59533 10.182 7.67867 10.3307 7.904C10.448 8.08067 10.5547 8.262 10.6273 8.46933C10.672 8.59933 10.6147 8.70533 10.4607 8.77067Z
```

### `alibaba` - Figma 665:18457 (`alibaba-color 1`), viewBox `0 0 16 16`, fill="#FF6003" fillRule="evenodd" clipRule="evenodd"

```
M16 9.34267C14.1333 10.3507 12.2533 11.2733 10.1607 11.692C9.694 11.7847 9.17667 11.7847 8.70267 11.7207C8.25067 11.664 8.02467 11.266 8.18533 10.8473C8.33867 10.4573 8.54267 10.06 8.80533 9.73333C9.37333 9.02333 10.0147 8.37733 10.5907 7.67467C10.9424 7.24404 11.2594 6.78616 11.5387 6.30533C11.7427 5.96467 11.648 5.55267 11.2907 5.38267C10.6927 5.09133 10.0513 4.886 9.41733 4.666C9.344 4.63733 9.23467 4.71533 9.08933 4.76533C9.27133 4.928 9.40267 5.04867 9.58467 5.212C7.704 5.532 5.92467 5.98533 4.198 6.61067C4.19 6.646 4.176 6.674 4.18267 6.68867C4.438 7.07867 4.32133 7.37667 3.94933 7.618C3.80492 7.71249 3.67699 7.83006 3.57067 7.966C4.708 8.29933 5.72133 8.108 6.69067 7.476C6.63267 7.39133 6.574 7.31333 6.516 7.228C6.88 7.292 7.09867 7.49067 7.128 7.80267C7.13533 7.874 7.092 7.94467 7.07 8.016C7.01867 7.95867 6.95333 7.90267 6.91 7.838C6.88 7.79533 6.87267 7.746 6.85133 7.67467C5.69933 8.44133 4.46 8.63333 3.09667 8.236C3.09667 8.50533 3.082 8.73267 3.104 8.95267C3.11867 9.144 3.03867 9.22933 2.864 9.32867C2.47 9.57 2.06133 9.81867 1.73333 10.138C1.34 10.528 1.486 11.004 2.018 11.2027C2.62267 11.4293 3.25733 11.436 3.89133 11.3587C4.642 11.266 5.378 11.1453 6.19533 11.032C5.24 11.4787 4.29933 11.792 3.30867 11.9187C2.61533 12.012 1.92333 12.0613 1.238 11.8627C0.254 11.586 -0.184 10.862 0.0713333 9.88933C0.312 8.98067 0.895333 8.25667 1.522 7.58933C3.61467 5.368 6.26067 4.24 9.322 4.01267C10.0367 3.96333 10.7587 4.05533 11.4287 4.35333C12.3693 4.77933 12.7633 5.68 12.304 6.58933C12.0053 7.19267 11.5893 7.746 11.174 8.29267C10.766 8.832 10.3067 9.32867 9.87667 9.84667C9.75267 10.0027 9.636 10.1667 9.54133 10.3433C9.35867 10.6767 9.48267 10.8967 9.86933 10.862C10.678 10.7833 11.5027 10.72 12.2827 10.5213C13.4267 10.23 14.542 9.81867 15.672 9.45667C15.7887 9.428 15.898 9.38533 16 9.34333V9.34267Z
```

### `moonshot` - Figma 671:19259 (`moonshot 1`), viewBox `0 0 16 16`, fill="black" fillRule="evenodd" clipRule="evenodd"

```
M0.701346 11.2775L7.06068 12.9788C7.05212 13.431 7.06547 13.8833 7.10068 14.3342L11.0713 15.3962C9.89366 15.8823 8.61689 16.08 7.34735 15.9728L7.22735 15.9622L7.19801 15.9595L7.14201 15.9535L7.07935 15.9468C7.04443 15.9426 7.00954 15.9382 6.97468 15.9335L6.90335 15.9242L6.83001 15.9135C6.75875 15.9031 6.68763 15.8918 6.61668 15.8795L6.58868 15.8742L6.53868 15.8655L6.46735 15.8522L6.42068 15.8422L6.35868 15.8295L6.30868 15.8188L6.24535 15.8055L6.18068 15.7902L6.11801 15.7755L6.07268 15.7642L6.01401 15.7495L5.95401 15.7335L5.89068 15.7168L5.83601 15.7015L5.76335 15.6815L5.72201 15.6682L5.66601 15.6515L5.60401 15.6328L5.53401 15.6102L5.49535 15.5975L5.44201 15.5802L5.38201 15.5595L5.33801 15.5435C5.32822 15.5402 5.31844 15.5369 5.30868 15.5335L5.26335 15.5168L5.19601 15.4922L5.15801 15.4775L5.10468 15.4575L5.04668 15.4342L4.98801 15.4108L4.93535 15.3895L4.87201 15.3628L4.83001 15.3442L4.78801 15.3262C4.77889 15.3222 4.76977 15.3182 4.76068 15.3142L4.71668 15.2942L4.64801 15.2628L4.61335 15.2468L4.54935 15.2162L4.50801 15.1962L4.45201 15.1695L4.39468 15.1402L4.33268 15.1088L4.29801 15.0908L4.22935 15.0542L4.19135 15.0342L4.15268 15.0128C4.14243 15.0071 4.13221 15.0013 4.12201 14.9955L4.05935 14.9602L4.01935 14.9375L3.98535 14.9175L3.93735 14.8902L3.88268 14.8568L3.82068 14.8195L3.78601 14.7982L3.73001 14.7628L3.68935 14.7368L3.63668 14.7035L3.59001 14.6722L3.55468 14.6488C3.54265 14.6409 3.53065 14.6329 3.51868 14.6248L3.48935 14.6048L3.46001 14.5848C3.4511 14.5786 3.44221 14.5724 3.43335 14.5662L3.39535 14.5395L3.34468 14.5035L3.29868 14.4702L3.24935 14.4342L3.21201 14.4062L3.16135 14.3682L3.11068 14.3288L3.05335 14.2842L3.02335 14.2608L2.98068 14.2262L2.93135 14.1862L2.87201 14.1375L2.84135 14.1115L2.81068 14.0855C2.80109 14.0773 2.79154 14.0691 2.78201 14.0608L2.75201 14.0342L2.71135 13.9988L2.66468 13.9575L2.61935 13.9175L2.57801 13.8788L2.53335 13.8375L2.49801 13.8042L2.43935 13.7482C2.41723 13.7267 2.39523 13.7052 2.37335 13.6835L2.35401 13.6648L2.32668 13.6368L2.28068 13.5902L2.24735 13.5562L2.21401 13.5208C2.17593 13.4818 2.13859 13.442 2.10201 13.4015L2.04868 13.3428L2.00735 13.2962L1.96001 13.2428L1.93201 13.2102L1.89668 13.1688L1.85801 13.1235L1.82735 13.0862C1.82132 13.0789 1.81532 13.0715 1.80935 13.0642L1.77935 13.0275L1.73535 12.9728L1.70801 12.9382L1.67468 12.8955L1.66135 12.8788C1.27989 12.3844 0.957694 11.8469 0.701346 11.2775ZM0.0213457 7.41483L7.59001 9.4395C7.4633 9.88084 7.35894 10.3283 7.27735 10.7802L14.4887 12.7095C14.1304 13.2 13.7178 13.6484 13.2587 14.0462L0.438012 10.6155L0.427346 10.5848L0.404012 10.5155C0.392673 10.4816 0.381561 10.4476 0.370679 10.4135L0.366012 10.3982C0.314698 10.235 0.268674 10.0703 0.228012 9.90417L0.208012 9.82017L0.196012 9.76683L0.182012 9.70217L0.170012 9.64817L0.158012 9.58817L0.146679 9.53217L0.134679 9.4695C0.117346 9.3755 0.101346 9.28083 0.0873457 9.1855L0.0760124 9.10683L0.0686791 9.0515L0.0600124 8.9835C0.0555487 8.94775 0.0513264 8.91197 0.0473457 8.87617L0.0440124 8.84483C-0.00579682 8.36977 -0.0133821 7.89123 0.0213457 7.41483ZM1.08335 3.9815L9.04868 6.11217C8.80335 6.5155 8.57868 6.93283 8.37535 7.36217L15.9053 9.37683C15.8107 9.9235 15.66 10.4515 15.46 10.9535L7.76001 8.8935L0.0826791 6.84017L0.0926791 6.7735L0.0980124 6.74083L0.104679 6.69617L0.114679 6.63817L0.126679 6.57283C0.144012 6.47417 0.164012 6.37617 0.185346 6.27817L0.204012 6.1955L0.217346 6.13883L0.233346 6.07417C0.248012 6.01417 0.263346 5.95417 0.280012 5.8955L0.298679 5.8275L0.314012 5.77217L0.334012 5.7055L0.350679 5.65083L0.370679 5.58683L0.388012 5.53217L0.408679 5.46883C0.581316 4.95098 0.806956 4.45234 1.08201 3.98083L1.08335 3.9815ZM4.04468 1.04817L11.568 3.06017C11.171 3.4205 10.7953 3.80359 10.4427 4.2075L15.658 5.60283C15.836 6.17083 15.9527 6.7655 16 7.3795L1.40401 3.4755L1.43401 3.43217L1.45201 3.4055L1.47868 3.36883L1.50935 3.3255L1.54601 3.27483L1.58201 3.22683L1.62468 3.1695L1.65801 3.12617L1.69601 3.0775L1.73268 3.03083L1.77268 2.9815L1.80935 2.9355L1.85268 2.88417L1.88868 2.84017L1.93268 2.78883L1.96801 2.74883L2.01601 2.69417L2.05135 2.65417L2.09601 2.60483L2.13201 2.56617L2.18068 2.51417L2.21935 2.47417L2.26135 2.4295L2.37335 2.31617L2.44001 2.25083L2.47935 2.2135L2.53001 2.16617C2.98947 1.73558 3.49783 1.36036 4.04468 1.04817ZM8.01135 0.000166667H8.07601L8.13068 0.000833333L8.17668 0.0015L8.21268 0.00283333L8.25801 0.00416667L8.28868 0.00483333L8.33935 0.00683333L8.37068 0.00816667L8.41068 0.0101667L8.44668 0.0115L8.50468 0.0148333L8.57468 0.0195L8.67068 0.0268333L8.72935 0.0315L8.75868 0.0341667L8.81001 0.0395L8.86468 0.0448333L8.89601 0.0481667L8.96401 0.0561667L8.99735 0.0601667L9.06935 0.0695L9.12335 0.0761667L9.15135 0.0801667L9.19468 0.0868333L9.33268 0.108167L9.37935 0.116167L9.42268 0.1235L9.51601 0.140833L9.57735 0.152833L9.65068 0.1675L9.68135 0.174167L9.73135 0.184833L9.75868 0.1915L9.80001 0.200167L9.82801 0.206833L9.87135 0.216833L9.90401 0.224833L9.95135 0.236167L10.0153 0.252167L10.09 0.272167L10.1653 0.292167L10.2407 0.3135L10.274 0.3235L10.3207 0.336833L10.3727 0.352833L10.4213 0.368167L10.4547 0.378833L10.488 0.3895L10.5387 0.406167L10.6047 0.428167L10.6727 0.452167L10.7047 0.4635L10.7473 0.478833L10.8093 0.5015L10.8827 0.528833L10.96 0.558833L11.0267 0.5855L11.058 0.598833L11.098 0.614833L11.1253 0.626833L11.1673 0.644167L11.194 0.656167L11.232 0.672833L11.3053 0.704833L11.372 0.7355L11.4213 0.758833L11.4713 0.782833L11.5113 0.8015L11.5727 0.832167L11.6333 0.862167L11.7013 0.896833L11.7367 0.9155L11.7693 0.932833L11.8 0.948833L11.84 0.970833L11.8673 0.9855L11.902 1.00483L11.9607 1.03817L12.0313 1.07817L12.0893 1.11217L12.1273 1.13483L12.1627 1.15617L12.2267 1.1955L12.2853 1.23217L12.3507 1.2735L12.3747 1.2895L12.4173 1.31683L12.4733 1.35417L12.5 1.37217L12.5413 1.40017L12.5827 1.42883L12.598 1.44017C12.634 1.46483 12.67 1.49017 12.7053 1.51617L12.7607 1.55617L12.804 1.58817L12.8413 1.61683L12.8987 1.66017L12.9533 1.70283L12.98 1.72283L13.0133 1.75017L13.0707 1.79617L13.1233 1.8395L13.18 1.88683C13.6547 2.28683 14.082 2.74217 14.4527 3.24083L4.81468 0.662833L4.85601 0.644833L4.89935 0.626167L4.95335 0.6035L5.01068 0.580167C5.08601 0.550167 5.16201 0.520167 5.23801 0.492833L5.30201 0.4695L5.36401 0.4475L5.42001 0.4275L5.48401 0.406833C5.54201 0.386833 5.60135 0.368167 5.66001 0.350167L5.72068 0.332167L5.77801 0.3155L5.84601 0.2955L5.90268 0.280167L5.96935 0.262833L6.02668 0.246833L6.08668 0.2315L6.14735 0.216833L6.21068 0.202167L6.27068 0.188833L6.33601 0.174833L6.39668 0.1615L6.46001 0.1495L6.52135 0.1375L6.58801 0.1255L6.64868 0.114833L6.71401 0.1035L6.77535 0.0941667L6.84001 0.0841667L6.90135 0.0755L6.96935 0.0668333L7.03001 0.0588333L7.10001 0.0508333L7.16001 0.0441667L7.23001 0.0375C7.29201 0.0308333 7.35401 0.0255 7.41668 0.0215L7.48735 0.0161667L7.54735 0.0128333L7.62068 0.00883333L7.68268 0.00616667L7.74935 0.0035L7.81401 0.00216667L7.88001 0.000833333L8.01135 -0.0005V0.000166667Z
```
