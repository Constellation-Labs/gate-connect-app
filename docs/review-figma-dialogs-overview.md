# Figma audit: dialogs on `Flows / Overview` (116:26381)

File `9FrccCojXy0f8QD8Wm5Lln`. Read-only audit of the centred dialog frames on
the Overview flow page against `src/components/gc/Modal.tsx` and
`src/components/gc/dialogs.tsx`.


## Answers to the three open questions

### #1 - the 536 width is real (MEASURED)

`get_metadata` on both nodes directly:

| node | name | x | y | w | h |
| --- | --- | --- | --- | --- | --- |
| `694:33002` | card/organization | 255.6435546875 | 244.34130859375 | **536** | 232 |
| `694:33340` | card/organization | 255.6435546875 | 244.34130859375 | **536** | 232 |

So `ModalWidth`'s `536` member and `QuitSafeToCloseDialog`'s `width={536}`
(`src/components/gc/dialogs.tsx:1668`) are correct as drawn. **No change to
`Modal.tsx`.**

Why the earlier "no 536-wide frame anywhere" scan missed them: these two frames
are *not centred* in the 1024 flow frame. A 536 dialog centred in 1024 would
start at x=244; these start at 255.64, which is exactly where a **512** dialog
centred in 1024 starts (256, less the frame's own fractional offset). Their
right edge lands at 791.64, i.e. ~23.6px past the mirror of the left edge. A
scan keyed on "centred in 1024" therefore rejects them. The `ModalWidth`
docstring in `Modal.tsx:59-64` already describes exactly this geometry and calls
it a likely stretched edge worth a designer question - that description is
confirmed accurate, and it remains a designer question, not a code bug.

### #2 - dialog button radius: 8px IS drawn here (MEASURED)

`get_design_context` on both button groups on this page emits `rounded-[8px]`
on every instance, on a `base/input` (#d1d5db) hairline:

| node | button | emitted radius | border |
| --- | --- | --- | --- |
| `694:32469` | Cancel (outline), quit chooser | `rounded-[8px]` | `var(--base/input,#d1d5db)` |
| `694:32470` | Disconnect (primary), quit chooser | `rounded-[8px]` | `alpha/80` rgba(255,255,255,0.2) |
| `694:33509` | Cancel (outline), Safe-to-close | `rounded-[8px]` | `var(--base/input,#d1d5db)` |
| `694:33518` | Close Gate Connect (primary), Safe-to-close | `rounded-[8px]` | `alpha/80` rgba(255,255,255,0.2) |

So the answer is **plainly yes: 8px is drawn on this page**, on all four
instances, with no 4px anywhere. `Modal.tsx`'s `rounded-md` on the secondary,
middle and primary buttons (lines 255, 265, 283) is correct and must **not** be
flipped to `rounded-control`.

That means the Figma disagrees with *itself* across pages: the Settings-page
audit measured `rounded-[4px]` on all six of its dialog button instances, while
all four Overview dialog buttons draw 8. Both readings are real. The Overview
nodes are the newer ids (`694:*` vs Settings' `143:*`), and CLAUDE.md's
"dialogs draw 8" already encodes the 8, so 8 stays as the house rule for
`Modal.tsx`. The Settings frames' 4px is the outlier and is a designer
question, not a code change. Worth noting that both pages agree on the other
half of the rule - the `base/input` line - so the only contested value is the
radius.

**Cross-page tally, from the sibling reports in `docs/`** (their measurements,
not re-verified here): Settings draws 4 on all six of its dialog buttons
(`177:74577/8`, `143:70627/8`, `363:9037/8`); the App page is split - the model
picker's `682:20043`, `665:19136`, `665:19137` draw 4 while the model
confirmation's `130:48311`/`130:48312` draw **8** on `base/input`; Overview
draws 8 on all four. So the file is inconsistent within a page as well as
across pages, and `694:*` - this page's quit flow - is the newest drawing in the
file. That, plus CLAUDE.md naming 8, is why `Modal.tsx` stays at `rounded-md`.
The Settings report's instruction ("do not flip this on my word; either confirm
4 across the other dialog pages, or raise it with the designer") resolves the
first way: **4 is not confirmed across the other pages, so nothing changes.**

Two incidental confirmations from the same emit, both matching code:
- primary fill is the white-8%-to-black-8% vertical gradient over `#203de2`
  (`base/primary`), ink `base/primary-foreground #f9fafb`;
- outline button ink is `base/primary #203de2` on `base/card` white.

### #3 - focus order on destructive dialogs (MEASURED in code)

`Modal.tsx:182-186` wires it:

```
useFocusTrap(panelRef, onDismiss, primary?.destructive ? safeRef : initialFocus);
```

`safeRef` (line 173) is attached to the **secondary** button (line 252), so any
dialog whose primary is `destructive: true` opens focus on the safe choice.

Exactly one dialog on this page marks its primary destructive, and it is
correct:

- **`CloseAppsDialog`** (`src/components/gc/dialogs.tsx:400`, Figma `135:62601`
  / `130:58855`): primary `Yes, close apps` is `destructive: true`
  (line 420-424), so focus opens on the secondary `No, I will close later`.
  Verified against the frame: `135:62623` is the shadcn *Destructive* variant,
  fill `#dc2626`, ink `base/destructive-foreground #fef2f2`; `135:62622` is the
  Outline variant with `base/primary` ink. Code matches the frame.

The other Overview dialogs pass no `destructive` primary, and none of them puts
the primary first in DOM order, so the trap's first-focusable fallback lands
somewhere safe:

- `QuitDialog` - first focusable is the `Disconnect tools and quit` radio
  (the row the frame draws selected, and the one pilled `SAFEST`), not a button.
- `QuitSafeToCloseDialog`, `QuitLeftBehindDialog`, `ReviewConfigDialog`,
  `SwitchOrganizationDialog` - first focusable is the secondary button.
- `OrganizationSwitchedDialog`, `ChangeReadyDialog` - single `Done` primary,
  nothing destructive.

**One gap, and it is the interesting one.** `ApplyChangesDialog`
(`dialogs.tsx:362`, Figma `130:58427` / `135:62184`) deliberately inverts the
weighting: the frame draws `No, I will reopen later` as the filled primary and
`Yes, close affected apps` as the outline secondary, and the code follows that
(lines 379-381, with a comment saying why). But `Modal` keys its safe-focus
rule off `primary.destructive`, and here the destructive action is the
*secondary*. With no `destructive` flag and no `initialFocus`, the trap takes
the first focusable in DOM order - which is the secondary, i.e. the button that
closes the user's running apps. A keyboard user who opened this dialog with
Enter closes Codex by pressing Enter again: the precise failure principle 5 and
`useFocusTrap`'s own docstring (lines 15-19) describe.

This is a rule the frame cannot answer - the Figma draws button weighting, not
focus - so it is a code finding, not a Figma mismatch. The narrow fix is to
pass `initialFocus` at the `ApplyChangesDialog` call site pointed at the
primary; `Modal`'s current signature already supports it, since a
non-destructive primary lets `initialFocus` through. It needs a ref plumbed to
the primary button, which `Modal` does not currently expose - so this one is
worth raising rather than patching blind. Confidence: MEASURED in code,
INFERRED as a defect (no frame states the intent).

## Dialog inventory on this page

| Figma node | size | component | code width | tile drawn | tile in code |
| --- | --- | --- | --- | --- | --- |
| `130:55314` | 512x380 | `SwitchOrganizationDialog` :66 | 512 | 40 box / 20 glyph | `md` 40/20 ✓ |
| `130:55755` | 512x244 | `OrganizationSwitchedDialog` :248 | 512 | 40 box / **24** glyph | `lg` 44/24 ✗ |
| `130:57442` | 600x418 | `ReviewConfigDialog` :277 | 600 (default) | 44/24 | `lg` 44/24 ✓ |
| `130:58427`, `135:62184` | 600x318 | `ApplyChangesDialog` :362 | 600 (default) | 44/24 | `lg` 44/24 ✓ |
| `135:62601`, `130:58855` | 600x318 | `CloseAppsDialog` :400 | 600 (default) | 44/24 | `lg` 44/24 ✓ |
| `134:61659`, `135:63018` | 512x244 | `ChangeReadyDialog` :441 | 512 | 44/24 | `lg` 44/24 ✓ |
| `694:32272` | 600x428 | `QuitDialog` :1560 | 600 (default) | 44/24 | `lg` 44/24 ✓ |
| `694:33002`, `694:33340` | 536x232 | `QuitSafeToCloseDialog` :1651 | 536 | 32/20 | `sm20` 32/20 ✓ |

Every width in code matches its frame. Note `134:61659`/`135:63018` draw a
**44px** tile on a 512 dialog while `130:55314`/`130:55755` draw **40** at the
same width, so `Modal.tsx`'s "size tracks the dialog's width" heuristic does
not hold on this page; what holds is that each frame names its own tile, which
is what the `tile` prop is for.

## Verified correct

Measured against the frames, no change needed:

- **Card shell.** 24px padding (`p-6`; frames put content at x/y=25, which is
  24 plus the 1px border Figma draws inside), 24px header-to-body and
  body-to-buttons (`mt-6`; `694:32273` ends at 69 with `694:32279` at 93, and
  `694:32279` ends at 343 with the button group at 367), 12px button gap
  (`gap-3`), `bg-base-card` white on a `base/border` line.
- **Dialog title.** `heading/18`: Geist Medium 18/24, `base/foreground #030712`
  (`694:33007`). Code's `text-lg font-medium leading-6 text-base-foreground` is
  18/24 in rem. Nit only: it carries `tracking-heading` (-0.2px, the
  `heading/20` token) where `heading/18` at -1% is -0.18px. 0.02px; there is no
  `heading-18` stop and inventing one is not worth it.
- **Tone tile, success 32px** (`694:33004`): `rounded-[6px]` = this config's
  `rounded-sm`, `size-[32px]`, 1px `green/300 #86efac`, `shadow/2xs`, and a
  vertical `green/50 -> green/200` gradient. `TILE_SIZES.sm20` +
  `TONE_STYLES.success` reproduce all of it.
- **Buttons.** See open question #2 - radius, border, fill, gradient, ink and
  the destructive variant all match.
- **`ModalChoice`** (`694:32280` selected / `694:32456` unselected): radius 8
  (`rounded-md`), 12px padding, `base/card` fill, `shadow/sm` on **both**
  states with only the hairline moving `base/input` -> `base/primary`, title
  `heading/14` at `base/foreground`, 8px row gap. All as coded.
  The `SAFEST` pill is `green/200 #bbf7d0` on `green/800 #166534`,
  `rounded-[4px]`, `px-8 py-4`, `mono/label-12` at 0.72px - exactly the
  `rounded-control bg-green-200 ... py-1 ... tracking-label text-green-800`
  the component emits.
- **`ModalNote tone="info"`** (`694:32290`): `blue-ribbon/50 #ebf6ff` fill,
  `base/border` line, radius 8, 12px padding, `copy/14` at
  `blue-ribbon/900 #172563`, with two Medium runs inside a Regular sentence.
  Code matches, and `tailwind.config.ts` resolves both hexes exactly.
- **The second quit choice has no pill.** `694:32464` ("OPEN") is present in the
  frame but at `opacity-0`, i.e. a hidden leftover. Code correctly renders a
  pill only on the first row.

## Mismatches

Ordered roughly by how visible each is. All measurements are from
`get_design_context` on the named node unless marked otherwise; all Tailwind
values below were confirmed to emit by running this project's Tailwind over a
probe file (`rounded-control` -> 4px, `bg-amber-200` -> `#fde68a`,
`text-amber-800` -> `#92400e`, `bg-green-200` -> `#bbf7d0`,
`text-green-800` -> `#166534`, `bg-gray-50` = `bg-base-background` = `#f9fafb`,
`text-neutral-600` -> `#525252`, `text-base-muted-foreground` -> `#6b7280`).

### M1. `ModalNote` default (`muted`) is drawn as the `neutral` variant

- **Nodes:** `130:57459` (ReviewConfig), `135:62619` (CloseApps),
  `130:55762` (OrganizationSwitched). Three independent frames, identical.
- **Drawn:** fill `#f9fafb`, **1px `base/border #e5e7eb`**, radius 8,
  **12px padding**, `copy/14` Regular 14/20 at -0.14px.
- **Code:** `Modal.tsx:395`
  `rounded-md bg-gray-50 p-4 text-sm leading-5 text-neutral-600` - no border,
  16px padding, `#525252` ink.
- **Fix:** these dialogs' notes are exactly what `ModalNote` already implements
  as `tone="neutral"` (`Modal.tsx:400-410`: `border border-base-border p-3
  bg-base-background text-base-foreground`, and `bg-base-background` is the
  same `#f9fafb`). So either pass `tone="neutral"` at the Overview call sites,
  or change the `muted` default. Changing the default touches Settings and the
  reset dialog, so the call-site route is the safe one. The note bodies are
  `ReviewConfigDialog` (`dialogs.tsx:330`, `:341`),
  `ApplyChangesDialog` (`:390`), `CloseAppsDialog` (`:433`),
  `OrganizationSwitchedDialog` (`:265`), `ChangeReadyDialog` (`:458`).
- **Confidence:** MEASURED (border, padding, radius, fill). The ink is
  MEASURED but the file disagrees with itself: `130:57460` and `135:62620` say
  `base/foreground #030712`, `130:55764` says `base/muted-foreground #6b7280`.
  Neither is `neutral-600`, so the code is wrong either way; taking the newer
  id (`135:62620`) gives `base/foreground`, which is what `tone="neutral"`
  already draws.

### M2. Status pill palette, radius and vertical padding

- **Nodes:** `130:57457` `DETECTED` (amber), `135:62617` / `130:58442` /
  `130:58870` `OPEN` (green).
- **Drawn (both tones identically):** `amber/200 #fde68a` on
  `amber/800 #92400e`, `green/200 #bbf7d0` on `green/800 #166534`,
  `rounded-[4px]`, `px-[8px] py-[4px]`, `mono/label-12` 12/16 at 0.72px.
  Frame height 24px.
- **Code:** `Modal.tsx:304-305` `bg-amber-100 text-amber-900` /
  `bg-green-100 text-green-900`, and `Modal.tsx:366`
  `rounded-sm px-2 py-0.5` - one ramp step light on the fill, one step dark on
  the ink, 6px radius instead of 4, and 2px vertical padding instead of 4
  (20px tall instead of 24).
- **Fix:** `amber: "bg-amber-200 text-amber-800"`,
  `green: "bg-green-200 text-green-800"`, and `rounded-control px-2 py-1` on
  line 366. Mono face, size, leading and `tracking-label` (0.72px) are already
  right.
- **Confidence:** MEASURED.
- **Not a conflict:** `docs/review-figma-dialogs-app.md` measures the App
  page's PAYG pill (`130:48335`) as *no fill*, `base/border` line,
  `rounded-[2px]`, `base/foreground` ink. That is the `identity`-variant pill
  on a different frame, not this `PillTone` group; the fix above should stay
  scoped to `PILL_STYLES`' amber and green so it does not collide with whatever
  that report lands on.
- **Note for `Modal.tsx:434`:** `ModalChoice`'s comment calls its
  green-200/green-800 pill "a third pairing beside the Overview pills' 200/900".
  Measured, the Overview pills are **200/800** - the same pairing. That comment
  is wrong, and `ModalChoice` was right all along.

### M3. `ModalSubject` icon tile is 40px; the frame draws 36px

- **Node:** `130:57452` (`size-[36px]`), and the sibling wrappers
  `135:62611`, `130:58436`, `130:58864`, all 36x36 per `get_metadata`. The
  12px gap then puts the text group at x=48, which is what every frame shows.
- **Code:** `Modal.tsx:338` `size-10` (40px, border-box).
- **Fix:** `size-9`.
- **Confidence:** MEASURED on the box. Figma flattens this wrapper to a single
  exported SVG here (it contains the vendor mark), so its fill, border and
  radius could not be read *on this page* - but
  `docs/review-figma-dialogs-app.md` measured the same `ModalSubject` wrapper
  un-flattened on `130:48324`: **36px, `rounded-[4px]`, `base/border` line,
  `base/card` fill, glyph 20px**. Two pages therefore agree on 36. Taking that
  reading, the row also wants `rounded-control` in place of `rounded-sm` (6px)
  on line 338, and the glyph is 20px where `appIcon`'s fallback
  (`dialogs.tsx:54`) passes `size={16}` - the drawn Overview marks are 20px too
  (`135:62612`, `694:32283`, both 20x20 inset 8 in a 36 box). Both of those are
  the App report's call to make on its own frames; noted here as
  cross-confirmation of the 36.

### M4. `ModalSubject` description is `label/12`, not `copy/14`

- **Nodes:** `130:57456`, `135:62616`, `130:58441`, `130:58869`.
- **Drawn:** Geist **Medium 12/16**, `base/muted-foreground #6b7280`,
  tracking -0.12px (the `label/12` token).
- **Code:** `Modal.tsx:357` `truncate text-sm leading-5 text-neutral-600`
  - Regular 14/20 at `#525252`.
- **Fix:** `truncate text-base-xs font-medium leading-4 tracking-label-12
  text-base-muted-foreground` (all four emit; `tracking-label-12` is -0.12px).
  This is the same treatment `ModalChoice` already gives its own description
  line, minus the tracking - see M6.
- **Confidence:** MEASURED. The row title above it (`heading/14` Medium 14/20
  at `base/foreground`, letterSpacing **0**) matches the code exactly, so only
  the description moves.

### M5. `ModalOption` (the organization rows) diverges on six values

The only live frame for these rows is on this page. `Modal.tsx:493` cites
`451:7795` on `Flows / Setup` as its source; **that node id no longer exists in
the file** (`get_metadata` returns "node not found"), so the 8px-padding and
`shadow/xs`-when-unselected claims in that comment can no longer be checked
against anything. Measured on `130:55322` (selected) / `130:55330` /
`130:55338` (unselected):

| property | drawn | code (`Modal.tsx`) |
| --- | --- | --- |
| row padding | `p-[12px]` | `p-2` (8px), line 494 |
| shadow, unselected | `shadow/sm` | `shadow-base-xs`, line 497 |
| selected fill | `#f9fafb` | none (inherits the white card), line 496 |
| avatar radius | `rounded-[4px]` | `rounded-sm` (6px), line 502 |
| avatar fill / ink | `base/card` white on `base/foreground` | `bg-gray-100` on `text-neutral-700`, line 502 |
| meta line | Medium 12/16 at `base/muted-foreground`, -0.12px | `text-sm leading-5 text-neutral-600` (Regular 14/20 at `#525252`), line 510 |
| selected mark | 20px CircleCheck | `size={16}`, line 517 |
| unselected dot | 20px | `size-4` (16px), line 523 |
| row gap | 8px | `gap-3` (12px), `dialogs.tsx:101` |

Selected/unselected hairlines (`base/primary` / `base/input`), the avatar's
`base/border` line, the 36px avatar box, the 12px avatar-to-text gap, the name
line (`heading/14` Medium 14/20 at `base/foreground`) and the dot's
`bg-base-background` + `border-base-input` + `shadow/xs` are all correct.

- **Confidence:** MEASURED. Worth flagging to the reviewer that this row is
  shared with `SwitchGatewayDialog` (`dialogs.tsx:132`), which is not drawn on
  this page, so the change lands on an undrawn surface too.

### M6. `ModalChoice` description is missing its -0.12px tracking

- **Node:** `694:32287` / `694:32463`, `label/12` at `tracking-[-0.12px]`.
- **Code:** `Modal.tsx:453` `text-base-xs font-medium leading-4
  text-base-muted-foreground` - `base-xs` is a bare `fontSize` string with no
  letterSpacing tuple, so the span renders at `normal`.
- **Fix:** add `tracking-label-12`.
- **Confidence:** MEASURED, but it is 0.12px at 12px type. Cosmetic.

### M7. `OrganizationSwitchedDialog`'s tone tile is 40px, not 44px

- **Node:** `130:55757` - a **40x40** box at `rounded-[8px]` with a **24px**
  glyph, `green/300` line, `green/50 -> green/200` gradient, `shadow/2xs`.
- **Code:** `dialogs.tsx:256-263` passes no `tile`, and a non-neutral tone
  defaults to `lg` = `size-11` (44px) with a 24px glyph
  (`Modal.tsx:101`, `:177`).
- **Fix:** needs a `TILE_SIZES` pair that does not exist yet - 40px box, 24px
  glyph (`md` is 40/20, `lg` is 44/24). This is a third measured proof that the
  glyph does not track the box, which `Modal.tsx:77-90` already argues; the
  entry just has to be added and named from this frame.
- **Confidence:** MEASURED. 4px, so low severity, but it is the one dialog on
  the page whose tile is wrong and it also kills the "size tracks the dialog's
  width" heuristic: `134:61659` and `135:63018` are the same 512 width and draw
  44.

### M8. The `neutral` tone tile has no gradient

- **Node:** `451:8038` (the `SwitchOrganizationDialog` tile): 40px box,
  `rounded-[8px]`, `base/border` line, `shadow/2xs`, and a vertical
  black-4% -> white-4% overlay on white.
- **Code:** `Modal.tsx:49` `neutral: "border-base-border bg-base-card
  text-neutral-700"` - flat white. `Modal.tsx:36-40` documents the gradient for
  warning/success/danger and does not mention neutral, so this looks like the
  neutral case simply never got measured.
- **Fix:** `bg-gradient-to-b from-black/[0.04] to-white/[0.04]` over
  `bg-base-card`. Note the direction: the emitted gradient is 0deg, i.e. black
  at the **bottom**, which is the opposite of the toned tiles' light-to-dark.
- **Confidence:** MEASURED, but a 4% overlay on white is close to invisible.
  Worth raising, probably not worth shipping alone.

### M9. Body-block spacing in the quit chooser is 12/20, not 16/16

- **Node:** `694:32279` (the body stack). The label `694:32482` sits 12px above
  the choice group `694:32467`, and the choice group sits **20px** above the
  info note `694:32290` (164 -> 184 inside a 250-tall stack).
- **Code:** `Modal.tsx:246` gives every dialog body a uniform
  `flex flex-col gap-4` (16px), which the docstring says was measured on the
  Settings rename fields and the reset steps.
- **Fix:** none proposed. Honouring this needs per-dialog body spacing, which
  is a bigger change than the 4px it buys, and the two Settings frames the
  docstring cites do measure 16. Recording it so it is not re-discovered.
- **Confidence:** MEASURED (from `get_metadata` offsets), INFERRED as
  actionable.

## Copy differences

None. Every drawn string on this page matches the code, including the two
Overview-specific ones worth double-checking:

- `694:32287` "Restore saved configurations, turn routing off, then quit." and
  `694:32463` "Leave configurations pointed at Gate. Requests that depend on
  the local proxy may pause." match `dialogs.tsx:1614` / `:1621` verbatim.
- `694:33021` / `694:33348`, the two Safe-to-close branches, match
  `dialogs.tsx:1679-1681` verbatim.
- `130:57460` contains a typo Gate's own copy fixed - the frame reads
  "restored when you turn protection off, **disconnected** Gate Connect, or a
  complete reset", the code reads "disconnect Gate Connect, or do a complete
  reset" (`dialogs.tsx:349-350`). Fixing a drawn grammatical error is not a copy
  divergence of the kind CLAUDE.md reserves judgement for, so it is not raised
  as a third exception.
- `130:57456` also draws `hhtps://` in a sample URL. Placeholder content, not
  code.

The two decided exceptions (Replace API key's `New API key`, Disconnect Gate?'s
sentence) are not on this page. **No candidate third exception found here.**

## Could not determine

- **Dialog card radius and drop shadow.** Not measured on this page - no frame
  here exports the card root's own radius through `get_design_context` at a
  size worth fetching. `rounded-2xl` (16px) and `shadow-base-lg` are taken on
  CLAUDE.md's standing measurement, and `docs/review-figma-dialogs-app.md`
  independently reads `130:48278` as `rounded-[16px]`, `p-24`, `base/border`
  line, `shadow/lg` - which is exactly what `Modal.tsx:195` emits. Treated as
  correct, on someone else's measurement.
- **Tone-tile glyph ink.** Every icon comes back as a flattened `<img>`, so
  `text-amber-700` / `text-green-700` / `text-red-600` / `text-neutral-700` in
  `TONE_STYLES` cannot be checked from the frames. `Modal.tsx:38-39` already
  says this.
- **`ModalSubject`'s vendor-mark tile** fill, border and radius - same reason
  (see M3).
- **The scrim.** `bg-neutral-900/40` (`Modal.tsx:189`); the flow frames draw
  the dialog over a page, not over an exported overlay layer.

## Method and scope

Read-only on `src/` and `src-tauri/`. Every Figma value above came from
`get_metadata` (geometry) or `get_design_context` (emitted CSS) on the node id
named beside it, on page `Flows / Overview` (116:26381) of file
`9FrccCojXy0f8QD8Wm5Lln`. Nothing was inferred from a screenshot. Every Tailwind
class proposed as a fix was run through this project's own Tailwind (v3.4,
`tailwind.config.ts`) over a probe file and confirmed to emit CSS with the
expected value, since `dialogs.tsx` has shipped classes that emitted nothing
before.

Where the file contradicts itself the newer node id was taken and the choice is
stated inline (M1's note ink, and open question #2's radius).
