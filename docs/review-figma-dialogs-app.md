# Figma audit: dialogs on page `Flows / App` (116:30199)

File `9FrccCojXy0f8QD8Wm5Lln`. Code under audit:
`src/components/gc/dialogs.tsx` (`ModelPickerDialog`, `UseGateModelDialog`) and
`src/components/gc/Modal.tsx`.

Dialog roots on this page (measured, given):

| node | size | name |
| --- | --- | --- |
| `665:19064` | 600 x 588 | card/choose-model - **newest**, has a footer button row |
| `665:18400` | 600 x 536 | card/choose-model - older, no footer |
| `130:48278` | 512 x 362 | card/organization - the Gate-model confirmation |

Where the two picker frames disagree I take **`665:19064`** (higher id).

## Structure measured from the frames

`665:19064` (600x588), 25px card padding, 550px content column:

- `665:19065` header row 550x48: 44px icon tile (`665:19066`) with a 24px
  `Icon / Boxes`, 12px gap, text group at x=56; title 494x24, subtitle 494x24.
  A 24px `Icon / X` (`665:19132`) at x=551,y=25 - top-right, aligned to the
  card padding, not to the header row.
- 20px gap, then `665:19071` stack 550x418:
  - `665:19072` button-group h36: search `Input` 405x36 (16px search glyph at
    x=12, placeholder text x=36) + 12px gap + `Button` 133x36 (the provider
    filter, "All providers" with a chevron).
  - 16px gap, `665:19077` stack:
    - `665:19078` count row h16: "Showing 8 of 14 models・400+ in Gate AI"
      (225x16) at left; a right-hand group `682:20038` (x=478, 72x16, a 20x16
      count badge + "selected") that **does not render** in the frame.
    - 12px gap, `665:19081` settings-list 550x338: a bordered card, 9px inner
      padding, holding `665:19082` 532x320 = 8 rows of 40px, plus a 6px-wide
      rounded scrollbar (`683:20162`, h154).
- `665:19135` footer row at y=527, 550x36: **left-aligned** `Button` 133x36
  (`682:20043` = "Unselect all (4)"), then a right group at x=333: `Button`
  70x36 ("Cancel") + 16px gap + `Button` 131x36 ("Apply selections").

`665:18400` (600x536) is the same minus the footer row, and its count row draws
a plain "3 models selected" label (`665:18416`) in the right slot.


## Answers to the open questions

### 1. The model picker: which frame does the code match?

The two picker frames are identical from the search field down to the last row.
They differ in exactly three places, and the code lands on a different side of
each:

| | `665:18400` (older) | `665:19064` (newest) | code |
| --- | --- | --- | --- |
| title | "Choose a Gate model" | **"Choose Gate models"** | older |
| count-line right slot | "3 models selected" label (`665:18416`) | **empty** - `682:20038` carries `opacity-0` | holds "Unselect all (n)" |
| footer | none (card is 536 tall) | **`665:19135`**: "Unselect all (4)" left, Cancel + "Apply selections" right | Cancel + "Save models" right, no left slot |

So the code took `665:19064`'s *footer shape* and `665:19064`'s *"Unselect
all" content*, but put that content in `665:18400`'s *count-line slot* and kept
`665:18400`'s title. Taking `665:19064` throughout means: retitle to "Choose
Gate models", relabel the primary to "Apply selections", and move "Unselect all
(n)" out of the count line into the footer's left slot (`justify-between` on
`Modal`'s button row, which today is `justify-end`).

Structure and geometry otherwise verify: search 405px + 12px gap + 133px
provider filter on one h36 row; count line 12/16; a bordered card with 8px
padding holding 40px rows that scroll inside it; 16px vendor mark, 8px gap,
14/20 label, 20px checkbox at the right edge. Individual values that are off are
in **Mismatches** below.

**Six things the code draws that no frame on this page draws.** Listed so
nobody hunts for them: the loading skeletons, the "could not list its models"
note, the "no models to choose from" note, the set-aside notice with "Show
anyway", the unavailable-model rescue list, and the trailing credits
`ModalNote`. All are AG-590 / AG-592 requirements post-dating both frames. They
are not mismatches, and nothing here argues against them.

### 2. Dialog button radius: is 8px drawn anywhere on this page? **Yes.**

- `130:48311` "Keep App default" and `130:48312` "Use Gate credits" on the
  Gate-model confirmation both emit **`rounded-[8px]`** on a
  **`var(--base/input,#d1d5db)`** line. That is CLAUDE.md's dialog rule, both
  halves, exactly as written.
- `665:19076`, the picker's provider-filter `Button`, also emits
  **`rounded-[8px]`** on **`base/input`**.
- But the picker's own footer (`665:19135`) emits **`rounded-[4px]`** on all
  three buttons, and its outline button `665:19136` sits on
  **`var(--base/border,#e5e7eb)`**, not `base/input`.

**Verdict.** `Modal.tsx`'s `rounded-md` + `border-base-input` (`:255`, `:283`)
is correct for `130:48278` and wrong for `665:19135`. The file draws both
numbers, so **do not flip `Modal.tsx`** - the Settings audit's decision to leave
it alone was right for a different reason than it gave. Running tally of dialog
button instances measured so far: Settings 6/6 at 4px on `base/input`; this page
2 at 8px on `base/input` (the confirmation), 3 at 4px (the picker footer, the
outline one on `base/border`). 8px survives on exactly one dialog. Either the
picker needs a per-dialog radius escape on `Modal`, or this goes to the
designer as a file inconsistency - it is not resolvable by measurement, because
both readings are measured.

### 3. Focus order on the confirmation: honoured, but by accident

`UseGateModelDialog` (`dialogs.tsx:1032-1043` (the `Modal` at `:1034`)) passes neither `initialFocus` nor
`destructive: true`, so `Modal.tsx:182-186` hands `useFocusTrap` an `undefined`
target and the trap falls to its second candidate, `focusables[0]`
(`src/lib/useFocusTrap.ts:116`). The dialog has no close button and no input, and
`Modal` renders `secondary` before `primary` (`:251` before `:278`), so
`focusables[0]` **is** "Keep App default". It also gets `data-initial-focus`, so
the ring is visible.

The safe outcome holds. The mechanism principle 5 names is not what produces it,
and it would regress silently if the button order changed or any focusable were
added above the footer. **Do not "fix" this by marking the primary
`destructive`**: `130:48312` is drawn as a filled blue primary with the
white/black 8% gradient, and `destructive` would repaint it `red-600`. If the
invariant is wanted explicitly, pass `initialFocus` at a ref on the secondary.
No change is required on the evidence here.

The picker passes `initialFocus={searchRef}` (`:798`) and opens on the search
field, which is right for a 344-row catalogue and contradicts no frame.

### 4. The row checkbox remap: wrong, and worse than what it replaced

The frame does not draw a bordered box. `665:19088` / `665:19094` are 20x20
instances of `Icon / SquareCheck` / `Icon / Square`, and their exported SVGs
give the geometry exactly:

```
M15.8333 2.5H4.16667C3.24619 2.5 2.5 3.24619 2.5 4.16667V15.8333 ...
stroke="#D1D5DB" stroke-width="1.5"
```

- the visible square is inset to 2.5..17.5, i.e. **15x15 inside a 20px slot**
- corner radius **1.667px** (4.16667 - 2.5)
- stroke **1.5px**; `#D1D5DB` = `base/input` unchecked, `#203DE2` =
  `base/primary` checked, tick drawn in the same stroke
  (`M7.5 10 L9.16667 11.6667 L12.5 8.33333`)

Code (`dialogs.tsx:741`, `:748`) draws a 20x20 span, 1px border, **`rounded-sm`
= 6px**, with a 14px `check` glyph inside. Verified against the project's own
tailwind build: `rounded-base` emits nothing, `rounded-sm` emits `6px`,
`rounded-xs` emits `2px`, `rounded-control` emits `4px`.

So the remap is wrong twice over: 6px is ~3.6x the drawn radius, and the box is
20px where the drawing is 15px. Worse, the dead `rounded-base` rendered **0px**,
which was nearer 1.667 than 6 is - this remap moved away from the frame. Nearest
faithful fix without adding a glyph: `rounded-xs` on a `size-[15px]` box centred
in a `size-5` slot. Faithful fix: add lucide `square` / `square-check` to
`Icon.tsx` and render the instance, which is what the frame does.

The colours the remap left alone are **correct** (sampled from a 1:1 render of
`665:19082`: checked stroke exactly `#203de2`, unchecked exactly `#d1d5db`).

**The other five `rounded-base` remaps in the same commit (`1fe71a5`):**

| site | remapped to | frame | verdict |
| --- | --- | --- | --- |
| selected row `:724` | `rounded-md` 8px | `665:19083` `rounded-[4px]` | wrong, want `rounded-control` |
| checked box `:741` | `rounded-sm` 6px | 1.667px | wrong (above) |
| unchecked box `:748` | `rounded-sm` 6px | 1.667px | wrong (above) |
| "Unselect all" `:889` | `rounded-md` 8px | `682:20043` `rounded-[4px]` | wrong, want `rounded-control` |
| "Show anyway" `:909` | `rounded-sm` 6px | not drawn | undetermined; focus-ring only |
| "Unavailable" row `:926` | `rounded-md` 8px | not drawn (AG-592) | fine; matches the list card's 8px |

## Verified correct

- Dialog card: `rounded-[16px]`, 24px padding, `base/border` line, `shadow/lg`
  (`130:48278`) = `Modal.tsx:195` `rounded-2xl border-base-border p-6
  shadow-base-lg`.
- Header gap 12px, title 18/24 Medium `-0.18px` `base/foreground`, subtitle
  14/20 `base/muted-foreground` (`130:48283`/`:48284`) = `Modal.tsx:221`,
  `:226`. `tracking-heading` is `-0.2px` against a drawn `-0.18px`; not worth
  moving.
- The close X: 24px glyph, ink sampled `#6b7280` = `base/muted-foreground`
  (`665:19132`) = `Modal.tsx:206-209`.
- Row list card: 8px padding on a `base/border` line (`665:19081`) =
  `dialogs.tsx:955` `p-2 border-base-border`. Rows h40, `p-2`, `gap-2` between a
  16px mark and the label - all exact.
- Selected row fill: `#f9fafb` on a `base/border` line = `bg-gray-50
  border-base-border` (`:724`). Unselected: no fill (`:725`).
- Search field: `base/input` line, `shadow/xs`, h36, 8px gap, 16px search glyph
  (`665:19073`) = `dialogs.tsx:832`, except radius/fill/padding below.
- Provider filter: `base/input` line, `base/foreground` ink, Medium 14/20, h36
  (`665:19076`) = `dialogs.tsx:852`, except radius/padding below.
- Count line size and step: 12/16 (`665:19079`) = `text-base-xs leading-4`
  (`:874`). The `・` separator matches the drawn one.
- Confirmation copy is exact end to end: title, subtitle, "Gate credits:",
  "Keep App default", "Use Gate credits" and the closing sentence all match
  `130:48278` verbatim.
- Confirmation width 512 (`Modal` `width={512}`, `dialogs.tsx:1043`) and picker
  width 600 (default) both match their frames.
- The picker's `closeButton` is the right affordance: `665:19064` draws the X
  and `665:18400` draws it too.

## Mismatches

Ordered by how visible they are. Every "measured" row is from the emitted CSS of
a `get_design_context` call or from a pixel sampled off a 1:1 render.

### M1. The header tone tile is one step too small, and the wrong glyph

- **Figma** `665:19066` and `130:48280`: **44px** box, `rounded-[8px]`,
  `base/border` line, `shadow/2xs`, **24px** glyph, glyph ink sampled `#6b7280`
  = `base/muted-foreground`, and the box carries a vertical gradient (white 4%
  at the top, black 4% at the bottom - sampled `#fefefe` top / `#f5f5f5`
  bottom).
- **Code**: neither dialog passes `tile`, so `Modal.tsx:177` resolves
  `tone === "neutral"` to `"md"` = `size-10 rounded-md` with a **20px** glyph
  (`TILE_SIZES`, `Modal.tsx:99`). `TONE_STYLES.neutral` (`:49`) is a flat
  `bg-base-card` with `text-neutral-700` ink.
- **Fix**: pass `tile="lg"` on both dialogs (`dialogs.tsx:776` and `:1035`) -
  `lg` is already `size-11 rounded-md` / glyph 24, exactly the drawn pair. Then
  give `TONE_STYLES.neutral` the drawn gradient and `text-base-muted-foreground`
  in place of `text-neutral-700`. `Modal.tsx:38-40` says the frames "export the
  tile but not the glyph's own fill" - they do not, but the render does, and it
  is `#6b7280`.
- **Note**: `TONE_STYLES.neutral` and `md` are shared with undrawn dialogs, so
  the gradient/ink change is wider than this page. The `tile="lg"` half is
  local and safe.
- MEASURED (geometry from CSS, ink and gradient from pixels).

### M2. The header glyph is `boxes`, not `layers`

- **Figma** `665:19067` / `130:48281`: `Icon / Boxes` - three cubes stacked in a
  triangle. Confirmed on a 6x crop of the render.
- **Code** `dialogs.tsx:776` and `:1035`: `icon="layers"`, which is lucide
  `layers` (three flat stacked plates). `Icon.tsx` has no `boxes`.
- **Fix**: add lucide `boxes` to `Icon.tsx` and use it on both dialogs.
- MEASURED.

### M3. The row label is mono `neutral-900`; the frame draws sans `base/foreground`

- **Figma** `665:19087` and every sibling: style `label/14` - Geist **Medium**
  14/20 at `-0.14px`, ink `base/foreground` **#030712** (darkest pixel sampled
  in the row: exactly `(3,7,18)`). `686:23567` on the confirmation draws the
  model id the same way.
- **Code** `dialogs.tsx:731`: `font-mono text-sm leading-5 text-neutral-900`
  (`#171717`).
- **The ink half is unambiguous**: `text-neutral-900` is wrong, and this settles
  the prior review's flag. `dialogs.tsx:983` ("No models enabled") is the same
  class inside a `ModalNote`; no frame draws that note, but CLAUDE.md's ink rule
  and every measured heading on this page say `base/foreground`. Change both to
  `text-base-foreground`.
- **The mono half is a decision, not a slip - and I am not deciding it.**
  `GateModelOption.id`'s doc comment (`dialogs.tsx:517-520`) argues the frame's
  sans is "a slip rather than a decision since every other identifier in the
  design is mono". That premise is **false as measured**:
  `docs/review-figma-settings.md:46` found `116:28991` (device name),
  `116:28999` (install id), `143:68381` (masked key) and `127:44762` all resolve
  to `copy/14` Geist Regular, and told the next reader not to "fix" them to
  mono. With the two model-id nodes here, that is six identifiers drawn sans and
  none drawn mono. So the design consistently draws identifiers in the UI face,
  and CLAUDE.md's principle 4 ("Mono ... model ids") is the outlier. **Raise
  it**: either principle 4 narrows to the places a frame actually draws mono
  (the status pills, the eyebrow, the `mono/label-12` badges) or the designer
  confirms the sans is the slip. Do not flip `font-mono` off on my measurement
  alone.
- MEASURED (ink); MEASURED but needs a decision (face).

### M4. Both picker rows are one radius step too loose

- **Figma**: selected `665:19083` **`rounded-[4px]`**; unselected `665:19089`
  **`rounded-[8px]`**.
- **Code** `dialogs.tsx:724-725`: `rounded-md` (8px) selected, `rounded-lg`
  (10px) unselected.
- **Fix**: `rounded-control` and `rounded-md`. The comment at `:720-722` has the
  direction right ("tightens its radius against the looser one the other rows
  carry") and both values one step wide.
- MEASURED.

### M5. The list card is 10px; the frame draws 8px

- **Figma** `665:19081`: `rounded-[8px]`.
- **Code** `dialogs.tsx:955`: `rounded-lg` = 10px.
- **Fix**: `rounded-md`.
- MEASURED.

### M6. "Unselect all" is in the wrong slot, at the wrong radius

- **Figma** `682:20043`: a ghost `Button` in the **footer row**, left-aligned
  (`665:19135` is `justify-between`), h36, px-16 py-10, `rounded-[4px]`,
  transparent, `base/primary` ink, Geist Medium 14/20 at **letterSpacing 0**
  (style `heading/14`). The count-line slot that used to hold "3 models
  selected" is `opacity-0` in this frame.
- **Code** `dialogs.tsx:885-893`: rendered inside the count row's right edge,
  `-my-1 rounded-md px-2 py-1`, h ~28.
- **Fix**: move it to a footer slot and make the footer `justify-between`;
  `rounded-control`, `px-4 py-2.5`, `h-9`. This needs a new `Modal` slot (a
  `footerLeft`), which is a real change rather than a class swap.
- The comment at `:878-884` cites `682:20043` correctly but describes it as
  taking the count-line slot; it does not - it takes the footer's left slot, and
  the count-line slot is hidden.
- MEASURED.

### M7. The count line is `base/foreground`, not muted

- **Figma** `665:19079`: `copy/12` Regular 12/16 `-0.12px`, ink
  **`base/foreground` #030712**.
- **Code** `dialogs.tsx:875`: `text-base-muted-foreground`.
- **Fix**: drop the muted class - the wrapper's `text-base-xs leading-4` is
  already right; add `text-base-foreground`.
- MEASURED. (This is the second half of the pattern the prompt flagged:
  `text-neutral-*` is not automatically wrong here, but *muted* on the count
  line is.)

### M8. The search field: radius, fill, padding, placeholder ink

- **Figma** `665:19073`: **`rounded-[4px]`**, fill
  `custom/background-dark:input\30` = **#f9fafb**, `base/input` line,
  `shadow/xs`, h36, **px-12**, placeholder `base/muted-foreground` **#6b7280**.
- **Code** `dialogs.tsx:832`, `:845`: `rounded-sm` (6px), `bg-base-card`
  (white), `px-2.5` (10px), `placeholder:text-neutral-500` (#737373).
- **Fix**: `rounded-control bg-base-background px-3`, and
  `placeholder:text-base-muted-foreground`.
- **Cross-check**: `docs/review-figma-settings.md` found `ModalField` drawn the
  same way (4px, #f9fafb fill) on `143:67467` / `177:74574`. Two pages agree, so
  this is the house input, not a one-off.
- MEASURED.

### M9. The provider filter: radius and padding

- **Figma** `665:19076`: `rounded-[8px]`, px-12, and the `Button`'s inset
  moulding (`shadow/xs` drop + inset 4px white 24% / -4px black 2%) - i.e.
  `shadow-base-btn`, not `shadow-base-xs`.
- **Code** `dialogs.tsx:852`: `rounded-sm` (6px), `px-2.5`, `shadow-base-xs`.
- **Fix**: `rounded-md px-3 shadow-base-btn`.
- MEASURED. (This is a native `<select>`, so the drawn 20px chevron is the
  browser's; leave that alone.)

### M10. `ModalSubject`'s icon wrapper is 40px at radius 6; the frame draws 36px at radius 4

- **Figma** `130:48324`: **36px**, **`rounded-[4px]`**, `base/border` line,
  `base/card` fill, **20px** glyph centred.
- **Code** `Modal.tsx:338`: `size-10` (40px) `rounded-sm` (6px); and
  `dialogs.tsx:1047` passes a **16px** fallback glyph.
- **Fix**: `size-9 rounded-control` on `Modal.tsx:338`, and `size={20}` at
  `dialogs.tsx:1047`.
- **Caveat**: `ModalSubject` is shared with the drift-review and quit dialogs on
  other pages. This measurement is `130:48278`'s only. Check the sibling frames
  before moving the shared component, or take a per-instance size.
- MEASURED for this frame; the shared-component decision is not.

### M11. `ModalSubject`'s identity label is 10px; the frame draws 12px

- **Figma** `686:23566` "Anthropic": `label/12` - Medium **12**/16 `-0.12px`
  `base/muted-foreground`.
- **Code** `Modal.tsx:346`: `text-base-2xs` = **10px** (verified in the tailwind
  build: `0.625rem`).
- **Fix**: `text-base-xs font-medium`.
- MEASURED. Same caveat as M10 about the shared component.

### M12. The PAYG pill is a bordered 2px chip, not a filled 6px one

- **Figma** `130:48335`: **no fill**, `base/border` line, px-8 py-4,
  **`rounded-[2px]`**, Geist Mono Medium **12**/16 at `0.72px` (6%), ink
  **`base/foreground`**.
- **Code**, single-model path: `Modal.tsx:366` via `PILL_STYLES.neutral`
  (`:306`) = `bg-gray-100 text-neutral-700`, `rounded-sm` (6px), `px-2 py-0.5`.
- **Code**, multi-model path `dialogs.tsx:1068`: bordered (right shape) but
  `rounded-sm`, `text-base-2xs` (10px), `text-neutral-700`.
- **Fix**: the drawn chip is `rounded-xs border border-base-border bg-transparent
  px-2 py-1 font-mono text-base-xs font-medium tracking-label
  text-base-foreground`. `PILL_STYLES` is shared with the Overview/App status
  pills, which are a different component - do **not** change `PILL_STYLES`; give
  the model row its own chip, or extend `PillTone` with the drawn one.
  `dialogs.tsx:1068` only needs `rounded-xs`, `text-base-xs` and
  `text-base-foreground`.
- MEASURED. `Modal.tsx:366`'s `uppercase` is harmless (the drawn label is
  already caps).

### M13. The credits block: missing border, wrong internal gap, two inverted weights, grey ink

- **Figma** `130:48302`: `#f9fafb` fill **on a `base/border` line**, p-12,
  `rounded-[8px]`, column **gap-8**. Inner icon wrapper `130:48340` **36px
  `rounded-[4px]`** with a 20px glyph. "Gate credits:" (`130:48343`) is
  **Medium** 14/20 at letterSpacing 0, ink **`base/foreground`**.
  "$10.25 available" (`130:48345`) is **Regular** 14/20 `-0.14px`, ink
  **`base/foreground`**. The closing sentence (`130:48349`) is Regular 14/20,
  ink **`base/foreground`**.
- **Code** `dialogs.tsx:1078-1095`: no border; `mt-3` (12px) where the frame has
  8px; `size-9 rounded-sm` on the icon wrapper (36px right, 6px wrong);
  "Gate credits:" is Regular `text-neutral-600` (`:1086`); the balance is
  **Medium** `base-foreground` (`:1089`) - the two weights are swapped; the
  closing sentence is `text-neutral-600` (`:1093`).
- **Fix**: add `border border-base-border`, `mt-2`, `rounded-control` on the
  wrapper, `font-medium text-base-foreground` on the label, drop `font-medium`
  from the balance, and `text-base-foreground` on the sentence.
- MEASURED.

### M14. Picker block gaps

- **Figma** `665:19064`: header -> body **20px** (`665:19065` ends at 48,
  `665:19071` starts at 68), body -> footer **16px** (`665:19160` ends at 511,
  `665:19135` at 527). Inside the body: search -> count 16px, count -> list
  **12px**.
- **Code** `Modal.tsx:246`, `:249`: `mt-6` (24px) for both card gaps, `gap-4`
  (16px) for every child gap.
- **Note**: `130:48278` draws 24/24, so `Modal`'s defaults are right for the
  confirmation and 4px/8px loose on the picker. Low value, and it needs a
  per-dialog override to fix. Listed for completeness, not recommended.
- MEASURED (from node coordinates), low priority.

### M15. The list has no drawn scrollbar

- **Figma** `683:20162`: a 6px-wide pill (`rounded-[999px]`) in
  `base/sidebar-ring` **#9ca3af**, inside the card's right padding.
- **Code** `dialogs.tsx:957-959`: plain `overflow-y-auto`, so the platform
  scrollbar.
- **Fix**: optional. A `scrollbar-width: thin` / `::-webkit-scrollbar` rule
  would get closer, but the frame's is a drawn rectangle rather than a spec, and
  the app ships on three platforms' native scrollbars.
- MEASURED, INFERRED as to whether it is worth doing.

### M16. Vendor marks are never supplied

- **Figma**: every row draws the provider's own 16px mark (`anthropic 2`,
  `deepseek-color 1`, `alibaba-color 1`, `moonshot 1`), and `130:48324` draws a
  20px `anthropic 1`.
- **Code**: `GateModelOption.logo` is optional and **no call site passes it**
  (`NewUiApp.tsx:2035`), nor does `vendorLogo` (`:2061`), so every row and the
  confirmation draw the `cube` fallback.
- Already acknowledged at `dialogs.tsx:526` ("Falls back to a cube while the
  marks are unexported"). Flagged as an open gap, not a defect.
- MEASURED.

## Copy differences

Neither of CLAUDE.md's two decided exceptions occurs in these dialogs. I found
no candidate for a third; the differences below are plain wording drift where
the frame says something the code could say.

1. **Picker title.** `665:19069` "Choose Gate models"; `dialogs.tsx:777`
   "Choose a Gate model". The plural is the newer frame and the correct one now
   that the control is a multi-select - the singular is a leftover of the
   single-model era the code itself documents at `:549-552`. Take the frame.
2. **Picker subtitle.** `665:19070` "OpenCode **will be able to use** these
   models"; `dialogs.tsx:778` "`${appName}` **may use any model you enable
   here**". Same claim, different words, and neither frame draws the code's.
   Take the frame: `` `${appName} will be able to use these models` ``.
3. **Picker primary button.** `665:19137` "Apply selections";
   `dialogs.tsx:786` "Save models". Take the frame.
4. **Count line's third clause.** `665:19079` draws "400+ in Gate AI";
   `dialogs.tsx:877` emits the real catalogue size. That is the figure rule
   (principle 6) applied correctly - a measured number where the frame drew a
   placeholder. **Not a copy difference to fix.**
5. **Drawn model ids.** The frames draw a `gate/...` namespace no catalogue
   serves. Already reasoned about at `dialogs.tsx:539-543`. **Not a copy
   difference to fix.**

## Could not determine

- **Whether the picker's 4px footer buttons or the confirmation's 8px ones are
  the intent.** Both are measured from the emitted CSS of the same-generation
  nodes in the same file. This is a file inconsistency, not a reading error, and
  it is the one thing on this page a measurement cannot settle. See answer 2.
- **The face for model ids** (M3): measured as sans in six places across two
  pages, but CLAUDE.md names model ids in the mono list and the code carries an
  explicit contrary decision. Needs a person.
- **Whether `ModalSubject`'s 36px/4px wrapper and 12px identity label** (M10,
  M11) hold on the drift-review and quit frames, which are on other pages. One
  page's measurement is not evidence for a shared component.
- **The 20px provider-filter chevron** (`I665:19076;267:4103`): the code uses a
  native `<select>`, so the glyph is the platform's. Not comparable.
- **The unselected row's absent border.** The frame gives it no border at all,
  so its content box is 1px wider than the selected row's; the code adds
  `border-transparent` to keep the labels aligned. A deliberate and better
  choice than the frame's; recorded so it is not read as drift.

## RAW MEASUREMENTS (kept for the record)

### Picker footer `665:19135` (get_design_context, emitted CSS)
- `682:20043` "Unselect all (4)" - ghost: transparent bg, **`rounded-[4px]`**, no
  border, h36, px-16 py-10, gap-6, ink `base/primary #203de2`,
  Geist Medium 14/20, **letterSpacing 0** (style `heading/14`).
- `665:19136` "Cancel" - outline: **`rounded-[4px]`**, border
  **`var(--base/border,#e5e7eb)`**, bg `base/card`, h36, px-12 py-10,
  `shadow/xs` drop + inset 4px white 24% / -4px black 2%, ink `base/primary`,
  14/20 tracking -0.28px.
- `665:19137` "Apply selections" - primary: **`rounded-[4px]`**, border
  `alpha/80 rgba(255,255,255,0.2)`, gradient white 8% -> black 8% over
  rgb(32,61,226), `shadow/sm` 1+2, ink `base/primary-foreground #f9fafb`,
  14/20 tracking -0.28px, 1px text-shadow.
- Row: `justify-between`; right group gap 16px.

### Confirmation `130:48278` (get_design_context, emitted CSS)
- card: `rounded-[16px]`, p-24, column gap-24, `items-end`, border
  `base/border`, `shadow/lg`.
- header gap 12; `130:48280` icon tile **44px**, `rounded-[8px]`, border
  `base/border`, bg white + vertical gradient black 4% -> white 4%,
  `shadow/2xs`, glyph `Icon / Boxes` **24px**.
- title `130:48283` 18/24 tracking -0.18 Medium `base/foreground`;
  subtitle `130:48284` 14/20 tracking -0.14 Regular `base/muted-foreground`.
- subject row `130:48322`: border `base/border`, p-12, `rounded-[8px]`;
  icon wrapper `130:48324` **36px**, **`rounded-[4px]`**, border `base/border`,
  bg `base/card`, glyph **20px**.
- `686:23565` text group: "Anthropic" 12/16 Medium tracking -0.12
  `base/muted-foreground`; "gate/opus 5" **Geist Medium (sans, not mono)**
  14/20 tracking -0.14 `base/foreground`.
- PAYG pill `130:48335`: **no fill**, border `base/border`, px-8 py-4,
  **`rounded-[2px]`**, Geist Mono Medium 12/16 tracking 0.72px (6%),
  ink **`base/foreground`**.
- credits block `130:48302`: bg #f9fafb, **border `base/border`**, p-12,
  `rounded-[8px]`, column **gap-8**.
  - inner icon wrapper `130:48340` 36px **`rounded-[4px]`** border
    `base/border` bg `base/card`, glyph 20px.
  - "Gate credits:" `130:48343` **Medium** 14/20 letterSpacing 0
    **`base/foreground`**.
  - "$10.25 available" `130:48345` **Regular** 14/20 tracking -0.14
    **`base/foreground`**.
  - closing sentence `130:48349` Regular 14/20 tracking -0.14
    **`base/foreground`**.
- buttons `130:48311` / `130:48312`: **`rounded-[8px]`**, outline border
  **`var(--base/input,#d1d5db)`**, gap-8 inside, px-12, group gap 16.
