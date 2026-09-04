# Figma audit: Tray page (694:34005) vs `src/components/gc/Tray.tsx`

Read 2026-09-03. File `9FrccCojXy0f8QD8Wm5Lln`, page `↳ Tray ✅` (694:34005).

## Scope and what maps to what

The page draws four artboards:

| Node | Name | Size | What it is |
| --- | --- | --- | --- |
| 694:34006 | `Connect/partial` | 400x700 | amber master card, groups from the newest `738:*` label nodes |
| 694:34167 | `Connect/routing` | 400x700 | green master card, custom scroll indicator, older compact `Other tools` rows |
| 744:38073 | `Connect/menu` | 400x699.36 | same as routing, plus the footer overflow menu open |
| 734:36992 | `Connect/full frame` | 400x1066 | a scrolled-out overview, not a window state |

All of it maps to `src/components/gc/Tray.tsx` (presentational) plus
`src/TrayApp.tsx` (state), with two shared imports doing real drawing work:
`StatusTile` / `BaseSwitch` from `src/components/gc/base.tsx` and
`OutlineIconButton` from `src/components/gc/Topbar.tsx`.

**`src/components/gc/PopHeader.tsx` is not on this surface.** It is imported
only by `src/screens/Home.tsx` and `src/screens/Success.tsx`, the retiring
popover screens. Nothing on the Tray page draws it, and nothing in this report
touches it or `gc/*`.

Two frames are 400x700 with a 64px header; `Connect/full frame` is the outlier
at 56px because it is a 1066-tall overview. `src-tauri/tauri.conf.json` (400 x
700, `resizable: false`, `decorations: false`) matches the two real frames, and
`h-screen` + `h-16` header + `h-14` footer reproduces the drawn 64 / 580 / 56
split exactly.

---

## Verified correct

Measured off the frames and confirmed in code, so these do not need re-checking:

- **Shell geometry.** Header 64 (`h-16`), footer 56 (`h-14`), content inset 16
  (`px-4 pt-4`), master-card-to-list gap 20 (`gap-5`, drawn 98 - 78), group gap
  16 (`gap-4`, drawn 174 - 158), label-to-card gap 8 (`gap-2`, drawn y=28).
- **Header lockup.** Logo 23.33x27.16 (`height={27}`), gap 9.975 (`gap-2.5`),
  wordmark Geist SemiBold 16/24 at -0.16px, `#002554` / `#3646e7`, inter-word
  gap 2.138 (`gap-[2px]`). Tray.tsx:111-118 is exact, including the two raw
  hexes, which are genuinely not `base.*` or ramp tokens (694:34020/21).
- **"Expand app" button.** h32, `rounded-[8px]` on `base/input`, px12, gap8,
  icon 16, label `text/xs` 12/16 at -0.12px on `base/primary #203de2`, drop
  `shadow/xs` plus `inset 0 4px 6px rgba(255,255,255,.4), inset 0 -4px 4px
  rgba(0,0,0,.04)`. Tray.tsx:120-127 matches on every one, and
  `shadow-base-btn-sm` is that inset pair to the byte. Radius 8 is right here:
  this is chrome drawing the `Button` component's own radius, not a pane.
- **Master card.** `rounded-[8px]`, `p-[12px]`, `border green/300 #86efac` /
  `amber/300 #fcd34d`, tile 36 at `rounded-[4px]` with a 50 -> 200 vertical
  gradient, 300 border and a 20px 600-ink glyph, text stack `gap-[2px]`, title
  `label/14` Medium 14/20. Tray.tsx:193-207 plus `StatusTile`/`TILE_TONES`
  reproduce all of it.
- **The master card's missing switch is correct.** 744:38099 carries
  `opacity-0`, and because the text block is a fixed 162.45px inside a
  `justify-between` row, deleting the invisible switch changes nothing
  visually. The docstring's reading holds.
- **App rows.** Card `rounded-[8px]` + 1px `base/border` + `shadow/xs`
  (`shadow-base-xs` is `0 1px 2px 0 rgba(0,0,0,.05)`, which is Figma
  `shadow/xs` exactly), rows `p-[8px]` with a `border-b base/border` divider,
  row gap 16, content gap 12, tile 32 at `rounded-[4px]`, app name `label/12`
  Medium 12/16 on `base/foreground`.
- **The status line really is 10px.** 738:37562 and 738:37570 both resolve to
  `text-[10px]` / `leading-[16px]` Medium, with `amber/600 #d97706` or
  `green/600 #16a34a` on the phrase and `#6b7280` on the ` - 3d ago` suffix.
  `text-base-2xs` (10px) at Tray.tsx:268 and `STATUS_TEXT`'s `text-green-600` /
  `text-amber-600` are right. The 16px node height was the trap here; the token
  is 10/16, not 12/16.
- **Group eyebrow.** `mono/eyebrow` at 14, not 12: Geist Mono Medium 14/20 at
  8% (1.12px) uppercase on `base/muted-foreground #6b7280` (738:37554). The
  docstring's claim is confirmed against the newest nodes.
- **Group count.** `mono/body-12` Geist Mono Regular 12/16 at 0% on
  `base/muted-foreground`. `font-mono text-base-xs font-normal leading-4`
  matches.
- **Switch.** 36x20, 16px knob at a 2px inset, on `blue-ribbon/700 #203de2`,
  off `custom/outline rgba(163,163,163,.5)`, knob `base/background` under
  `shadow/lg`. `BaseSwitch` is exact, `bg-neutral-400/50` included.
- **Footer.** Icon/Users 20px, gap 8, org name `label/14` Medium 14/20 on
  `base/foreground`, 32px ellipsis button on the right.
- **Quit carries no external-link glyph.** 744:38212 exists in the metadata but
  the rendered frame shows nothing there, and the LogOut asset's stroke is
  `#DC2626` = `base/destructive` = `text-red-600`. Tray.tsx:404-412 is right on
  both counts, and the docstring's explanation of the discrepancy is accurate.
- **Menu container** width 224 (`w-56`), radius 8 (`rounded-md`), 1px
  `base/border`, right edge at 384 (`right-4`), items 32 tall at `px-[6px]`,
  icon-to-label gap 8, external glyph 12px at 6px from the right edge, label
  `label/12` on `base/foreground`. Item order dashboard / docs / quit matches
  once Contact support is dropped.
- **Collapsed "Not installed"** draws ChevronDown (738:37382) at 20px, matching
  the unrotated default at Tray.tsx:320-324.
- **CLI card copy** is the frame's own, word for word (735:37345), and its
  `label/14` + `copy/12` sizes, weights and leadings are right.
- **The 6px scroll indicator** the frames draw (694:34280) is matched by the
  global `::-webkit-scrollbar { width: 6px }` in `src/index.css:51`.
- **Every class Tray.tsx uses emits CSS.** Ran the project's own
  `tailwind.config.ts` over a probe containing all of them:
  `rounded-control` 4px, `rounded-sm` 6px, `rounded-md` 8px, `rounded-xs` 2px,
  `text-base-2xs`, `text-base-xs`, `shadow-base-xs/-md/-lg/-btn-sm`,
  `tracking-button-xs`, `tracking-label`, `tracking-eyebrow-14`,
  `divide-base-border`, `bg-blue-ribbon-700` all resolve. No dead classes on
  this surface.

### Deliberate deviations, checked rather than re-reported

- **No third row line.** The frames draw a 48px three-line `app-info`
  (name / status / "345 messages · 23 alerts") in a 64px row; the code draws
  two lines in a 48px row. Documented at Tray.tsx:32-38 and still correct on
  the data argument. One correction to the docstring's *evidence*, though: it
  cites the compact two-line `Other tools` rows in `Connect/routing`
  (694:34266, 40px, `p-[4px]`, gap 8, 32x17.78 switch) as proof the design
  draws a two-line row. Those are the *older* nodes. The newest group nodes
  (738:37615 in `Connect/partial`, 735:37291 in the full frame) give `Other
  tools` the same 64px three-line row as everything else, so per the
  newest-wins rule the drawn row is uniformly the three-liner. The code's 48px
  row is a hybrid (the three-liner's `p-2` / gap-3 / 36x20 switch, minus a
  line) rather than either drawn row. That is a reasonable landing spot and
  needs no change, but the justification should be restated.
- **`SecurityCard`** (Tray.tsx:455-483) is drawn nowhere on this page. Undrawn
  addition per AG-578, wired at TrayApp.tsx:373-378.
- **`SignedOutNote`** is drawn nowhere. Undrawn, documented.
- **Contact support omitted** from the menu (744:38198 draws it). Documented.
- **Quit flow.** CLAUDE.md records the chooser (694:32272) and confirmations
  (694:33002 / 694:33340) as redrawn and built. Those nodes are **not on this
  page** - the Tray page's only quit affordance is the menu row, which is
  present and correct. Nothing to verify here; it belongs to the dialogs audit.

---

## Mismatches

Ordered by visual impact.

### 1. Menu item glyphs are drawn 14px, coded 16px

- **Figma** 744:38195 / 38200 / 38205 / 38210: `size-[14px]` on every menu row
  icon (LayoutDashboard, Headset, BookOpenText, LogOut).
- **Code** `src/components/gc/Tray.tsx:398` and `:410`: `<Icon ... size={16} />`.
- **Fix** `size={14}` in both.
- **Confidence** MEASURED. Beside a 12px label a 16px glyph reads a step too
  loud, and the row is only 32px tall, so the 2px shows.

### 2. Menu rows and the footer ellipsis button carry the wrong radius

- **Figma** menu row 744:38193 etc: `rounded-[4px]`. Footer button 694:34124
  (default state) and 744:38191 (pressed): `rounded-[8px]`.
- **Code** `src/components/gc/Tray.tsx:395` and `:408` use `rounded-sm`, which
  this config defines as **6px**, not 4. `src/components/gc/Topbar.tsx:107`
  (`OutlineIconButton`, which Tray.tsx:160 renders) also uses `rounded-sm` 6px
  where the frame draws 8.
- **Fix** menu rows -> `rounded-control` (4px); footer button ->
  `rounded-md` (8px).
- **Confidence** MEASURED. Note the blast radius: `OutlineIconButton` is
  exported from `Topbar.tsx` and the topbar draws the same button, so this one
  belongs to whoever owns the topbar audit rather than to a tray-only edit.

### 3. Menu drop shadow is one step too heavy

- **Figma** 744:38192 carries `shadow/md`: `0 4px 6px -1px #00000014, 0 2px 4px
  -2px #00000014`. That is `shadow-base-md` in `tailwind.config.ts:312` to the
  byte.
- **Code** `src/components/gc/Tray.tsx:387`: `shadow-base-lg`
  (`0 10px 15px -3px, 0 4px 6px -4px`).
- **Fix** `shadow-base-lg` -> `shadow-base-md`.
- **Confidence** MEASURED.

### 4. "Not installed" count is drawn 14px, coded 12px, and its gap is 12 not 4

- **Figma** 738:37377: the count `8` (738:37381) is `mono/body-14`, Geist Mono
  Regular **14/20** on `base/muted-foreground`, and the count-to-chevron gap is
  `gap-[12px]`. The group counts elsewhere are `mono/body-12`, so this is a
  genuine per-node difference, not a stale node.
- **Code** `src/components/gc/Tray.tsx:319`: `text-base-xs ... leading-4`
  (12/16). `:318`: `gap-1` (4px).
- **Fix** count -> `text-sm leading-5`; wrapper gap -> `gap-3`.
- **Confidence** MEASURED. The count currently sits visibly smaller than the
  14px eyebrow it is paired against, and 4px crowds the chevron.

### 5. Menu vertical offset and padding

- **Figma** 744:38192 sits with its bottom edge 52px above the window bottom
  (y 501.36 + h 146 = 647.36 in a 699.36-tall frame, ~648 unscaled) and pads
  `p-[8px]` (the metadata's x=9 is 1px border drawn inside plus 8px padding).
- **Code** `src/components/gc/Tray.tsx:387`: `bottom-12` (48px, putting the
  bottom edge at 652) and `p-[9px]`.
- **Fix** `bottom-[52px]` (or `bottom-13` if the scale gains a stop) and `p-2`.
- **Confidence** MEASURED for both, 4px and 1px respectively. Low impact; group
  these with the two above into one menu pass.

### 6. Missing letter-spacing on five text nodes

Every one of these resolves a Figma type token with non-zero tracking, and the
call site carries none. The config already has the values
(`tracking-button-xs` = -0.12px).

| Figma node | Token | Drawn tracking | Code |
| --- | --- | --- | --- |
| 738:37561 (app name) | `label/12` | -0.12px | `Tray.tsx:241` |
| 744:38196 (menu label) | `label/12` | -0.12px | `Tray.tsx:399` |
| 744:38190 / 694:34123 (org name) | `label/14` | -0.14px | `Tray.tsx:156` |
| 735:37344 (CLI title) | `label/14` | -0.14px | `Tray.tsx:359` |
| 744:38098 (master sub-line) | `copy/12` | -0.12px | `Tray.tsx:202` |

- **Fix** add `tracking-button-xs` to the two `label/12` and the `copy/12`
  sites; the two `label/14` sites need a -0.14px stop, which the scale does not
  yet name.
- **Confidence** MEASURED. Sub-pixel per glyph, cumulative over a line.
- **Caveat** the file contradicts itself on `label/14`: the master card title
  (744:38097) resolves it at letterSpacing **0**, the footer and CLI card at
  **-1%**. The master card title is correctly untracked in code; the other two
  are not.

### 7. App tile gradient alpha is 0.28, coded 0.24

- **Figma** 738:37559: `linear-gradient(180deg, rgba(255,255,255,0.28) 0%,
  rgba(0,0,0,0.28) 100%)` over black, 1px `rgba(255,255,255,0.24)` border.
- **Code** `src/components/gc/Tray.tsx:283`: both stops at `0.24`. The border
  at `:280` is correct at 0.24.
- **Fix** `0.24` -> `0.28` in the two gradient stops.
- **Confidence** MEASURED. Barely visible, but it is a two-character fix and
  the border/gradient values being different in Figma is exactly the kind of
  thing that got collapsed by accident.

### 8. CLI card padding and shadow

- **Figma** 735:37341: `pl-[12px] pr-[8px] py-[12px]`, and **no drop shadow**
  at all - its variable set carries only `shadow/lg` (the switch knob's) and
  the generated context emits no shadow on the card div, unlike the group cards
  which do emit `shadow/xs`.
- **Code** `src/components/gc/Tray.tsx:357`: `p-3` (12 uniform) and
  `shadow-base-xs`.
- **Fix** `p-3 pr-2`, and drop `shadow-base-xs`.
- **Confidence** MEASURED on the padding; MEASURED-with-a-caveat on the shadow,
  since an absent shadow is inferred from its absence in the generated context
  rather than read positively. Both are low impact, and leaving the shadow on
  arguably reads better next to the bordered-and-shadowed group cards above it.

### 9. Eyebrow rows align baselines where the frames align tops

- **Figma** 738:37553 and 738:37377 are both `items-center` / `items-start`
  with label and count top-aligned at y=0 (label box 20 tall, count box 16 or
  20).
- **Code** `src/components/gc/Tray.tsx:223` and `:313`: `items-baseline`.
- **Fix** `items-baseline` -> `items-center`.
- **Confidence** MEASURED, ~2px on the group rows. Cosmetic.

### 10. Header and footer rules use the neutral border, not `base/border`

- **Figma** 694:34007, 694:34120 and 744:38187 all bind their divider to
  `color/border` = **#e5e5e5** (neutral-200). The group and CLI cards bind
  `base/border` = #e5e7eb.
- **Code** `src/components/gc/Tray.tsx:109` and `:155`: `border-base-border`
  (#e5e7eb).
- **Fix** none recommended. #e5e5e5 vs #e5e7eb is 2 and 6 counts of G and B on
  a 1px line, below any threshold worth a token split, and it is part of the
  variable-mode drift in the next item.
- **Confidence** MEASURED, no visible effect. Recorded so the next auditor does
  not re-derive it.

### 11. The master card and CLI card are bound to a stale variable layer

Not a code fix; a question for the designer.

Within the newest frame (744:38073), the sidebar rows resolve
`base/foreground` = #030712, `base/muted-foreground` = #6b7280,
`base/background` = #f9fafb, `base/border` = #e5e7eb - the gray family
CLAUDE.md locks. The master card (744:38091) and CLI card (735:37341) in the
same frames resolve `base/foreground` = **#0a0a0a**, `base/border` =
**#e5e5e5**, and use `color/muted-foreground` = **#737373** instead of
`base/muted-foreground` - the neutral family. `base/background` even resolves
#fafafa on one and #f9fafb on the other.

- **Affected code** `Tray.tsx:201-203` (master title and sub-line),
  `:359-362` (CLI title and body), all on `base-*` tokens.
- **Recommendation** leave the code on `base.*`. CLAUDE.md's standing decision
  ("Ink is `base.foreground #030712`, not `neutral-900`") already resolved this
  family question, the deltas are imperceptible (#737373 vs #6b7280), and
  switching two cards onto `neutral-500`/`neutral-950` to chase a stale library
  mode would put the tray's only two prose cards out of step with every other
  surface. Worth raising with design so the master-card component gets rebound.
- **Confidence** MEASURED on the values, INFERRED on the cause.

### 12. Menu rows drop a drawn `shadow/2xs`

- **Figma** 744:38193 etc each carry `shadow/2xs` (`0 1px 0 0 rgba(0,0,0,.05)`)
  as a drop shadow.
- **Code** `src/components/gc/Tray.tsx:395`, `:408`: no shadow.
- **Fix** none recommended. A 1px black-5% lip under each row of a popover menu
  is almost certainly inherited from the `sidebar-menu-item` component rather
  than intended here, and CLAUDE.md explicitly warns off flat `shadow-base-2xs`
  as a treatment.
- **Confidence** MEASURED on the value, INFERRED that it is an artifact.

### 13. Footer button's bottom inset is drawn at 4%, coded at 6%

- **Figma** 694:34124: `inset 0 -4px 4px 0 rgba(0,0,0,0.04)`, which is what
  `shadow-base-btn-sm` already spells (`tailwind.config.ts:284-288`).
- **Code** `src/components/gc/Topbar.tsx:107` hardcodes the same stack with
  `rgba(0,0,0,0.06)` instead of using the token.
- **Fix** replace the arbitrary shadow with `shadow-base-btn-sm`.
- **Confidence** MEASURED. Invisible; worth it only because it removes a
  hand-copied duplicate of an existing token. Shared component, same caveat as
  item 2.

### Nit, no visual effect

`Tray.tsx:226` and `:315` use `tracking-[1.12px]` where
`tailwind.config.ts:362` already names `tracking-eyebrow-14` as exactly that
value, for exactly this call site. Same output; the token is traceable.

---

## Could not determine

- **The scroll indicator's fill.** 694:34280 / 744:38186 is a 6px x 154
  rounded rect at x=357. Width matches `index.css`'s 6px webkit scrollbar, but
  I did not resolve the rect's fill, so I cannot say whether the `ink-300`
  thumb is right. It is also unclear whether the design intends a custom
  always-visible indicator or is just drawing the platform scrollbar.
- **The expanded "Not installed" state.** Only the collapsed header is drawn
  (738:37377, and a hidden variant 738:37545 whose chevron points up and whose
  count reads "8 tools" rather than "8"). The expanded row anatomy at
  Tray.tsx:327-343 has no frame to check against.
- **Whether the master card's `color/*` bindings are intentional** (item 11).
  Needs a designer.
- **Whether the tray's `SecurityCard` should carry `shadow-base-sm`**
  (Tray.tsx:465) when every drawn sibling card on the surface carries
  `shadow/xs`. Undrawn, so there is nothing to measure, but it is the only
  card on the tray with a heavier shadow than its neighbours.
- **The scroll region's bottom padding.** The frames run the scroll area flush
  to the footer (98 + 482 = 580); `Tray.tsx:137` adds `pb-4`. Whether that is a
  deviation or just a scroll affordance the static frame cannot express, I
  cannot tell from the file.
