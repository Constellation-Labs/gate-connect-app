# Figma audit: page `Sidenav` (408:15625) vs the app shell

File `9FrccCojXy0f8QD8Wm5Lln`, page **Sidenav** `408:15625`, read 2026-09-03.

Code audited: `src/components/gc/Sidebar.tsx`, `src/components/gc/Topbar.tsx`,
`src/components/gc/AppShell.tsx` (plus `BaseSwitch` in
`src/components/gc/base.tsx` and `BrandMark.tsx`, both reached from the rail).

## What the page actually contains

The whole page is six nodes:

| node | name | what it settles |
| --- | --- | --- |
| `408:14159` | `logo` | the eight 16px brand marks |
| `408:14180` | `logo-wrapper` | the 32px app tile |
| `408:14253` | `Switch` | the 36x20 switch, on and off |
| `434:129` | `sidebar-menu-item` | the app row, default and selected |
| `434:136` | `status-label` | protected / not-protected / not-routed |
| `437:161` | `sidebar` | the whole rail, in `page=overview` / `settings` / `app` |

**There is no topbar, chrome, banner or window-shell node anywhere on this
page.** So `Topbar.tsx` and `AppShell.tsx` are not audited by this page at all
(see "Could not determine"). Everything below is about `Sidebar.tsx`.

Internal disagreement, resolved by newest-node-id: the `page=overview` variant
carries pre-redraw internals (`408:*`, wrapper `688:23681`) and draws different
control padding from `page=settings` (`691:28*`) and `page=app` (`691:29*` /
`694:*`). I took the `691:*` pair throughout. Details under "Verified correct".

## Verified correct

All MEASURED unless marked. Pixel figures come from the 1:1 render of `437:161`
(864x734, no scaling).

**Rail container** - `Sidebar.tsx:205`
- 256px wide: left edge x=24, right edge x=279 in the set render, inclusive =
  256. Confirms the brief's note that 256 is current.
- 1px right border at x=279 measured `(229,231,235)` = `#e5e7eb` =
  `base/border`. `border-r border-base-border` is right.
- Body `#ffffff` = `base/card`. `bg-base-card` is right.

**Header and nav blocks** - `Sidebar.tsx:207`, `:211`
- `sidebar-header` `688:23681`/`691:28158` is 256x57 = 12px padding + 32px
  content + 12px + a 1px bottom rule; the rule measured `#e5e7eb` at y=80.
  `p-3` + `border-b border-base-border` is right.
- `sidebar-menu` `691:28255` is 256x85 = 12 + 28 + 4 + 28 + 12 + 1px rule;
  rule measured `#e5e7eb` at y=165. `p-3 gap-1 border-b` is right.

**Group list** - `Sidebar.tsx:235`
- `sidebar-group-list` `691:30028`: groups at x=12 (px-3), first at y=16 and
  last ending 16px short of the bottom (py-4), 16px between groups (gap-4).
  All three match.

**Org switcher** - `Sidebar.tsx:295`
- `691:28159` (newest): `sidebar-menu-button` at x=6, chevron right edge at
  226 of 232 -> 6px horizontal; content at y=8 of a 32px box -> 8px vertical.
  `px-1.5 py-2` is right.
- Radius 4 (`rounded-control`), 1px `base/input` border measured
  `(209,213,219)` = `#d1d5db` at x=36, white fill, `shadow/2xs`. All match.
- 16px icons, 8px gap, `label/12` in `base/foreground` `#030712`.

**NavItem** - `Sidebar.tsx:327`
- `691:28413` / `691:28364` (newest): `p-[6px]` uniform, `gap-[8px]`,
  `rounded-[4px]`. `p-1.5 gap-2 rounded-control` is right.
- Active: fill `#f9fafb` (measured `(249,250,251)`), 1px `#e5e7eb` border
  (measured at y=93), `shadow/2xs`, label `base/primary` `#203de2`. Code's
  `border-base-border bg-base-background text-base-primary shadow-base-2xs`
  matches to the byte. Figma names the fill `base/sidebar-primary-foreground`,
  the code names it `base.background`; same `#f9fafb`, no visual delta.

**App row** - `Sidebar.tsx:475`
- `434:127`/`434:128` and `691:30034`/`691:30035`: `gap-[16px]`,
  `p-[var(--spacing/1-5,6px)]`, `rounded-[4px]`, 44px tall, 4px between rows,
  8px from the eyebrow. `gap-4 p-1.5 rounded-control` + `gap-1` + `gap-2` all
  right.
- Selected: `#f9fafb` fill (measured), `#e5e7eb` border (measured at y=206),
  `shadow/xs`. Default: `base/card` white, no border. Code reserves a
  transparent border at rest so rows do not shift; documented and harmless.
- Name: `label/12` medium, `base/primary` when selected else `base/foreground`.

**Logo tile** - `Sidebar.tsx:489`
- `408:14175` / `logo-wrapper`: `p-[8px]` around a 16px mark = 32px,
  `rounded-[4px]`, 1px `rgba(255,255,255,0.24)` border, and
  `linear-gradient(180deg, rgba(255,255,255,0.24) 0%, rgba(0,0,0,0.24) 100%)`
  over black. Code reproduces all five values exactly, 24% overlays included.

**Status line** - `Sidebar.tsx:121-126`, `:507-509`
- `434:136`: 10px Geist Medium, 16px leading, no tracking. `text-base-2xs`
  emits `0.625rem` = 10px; `font-medium leading-4` right.
- Protected `#16a34a` = `tailwind colors/green/600` -> `text-green-600` emits
  `rgb(22 163 74)`. Not protected and Not routed `#d97706` =
  `tailwind colors/amber/600` -> `text-amber-600` emits `rgb(217 119 6)`.
- Suffix `#6b7280` = `base/muted-foreground`, rendered as `label` + `" "` +
  `"- 2m ago"`. Code's `<span> - {suffix}</span>` produces the identical
  string. `text-base-muted-foreground` right.

**Eyebrow and counter** - `Sidebar.tsx:255`, `:263`
- `691:30030`: row is `justify-between`, `w-full`, both children 12px/16 in
  `base/muted-foreground` `#6b7280`.
- Eyebrow `mono/eyebrow` = Geist Mono Medium 12/16 at `letterSpacing: 8`
  (percent) = 0.96px, uppercase. `tracking-eyebrow` compiles to
  `letter-spacing: 0.96px` exactly. Right.
- Counter `mono/body-12` = Geist Mono Regular 12/16 at 0 tracking.
  `font-mono font-normal` with no tracking class. Right.

**Switch** - `base.tsx:132-139`
- `408:14251` (on) / `408:14252` (off): track 36x20, radius full, knob 16px.
  Knob node `408:14248` sits at x=18 in the on symbol; pixel scan of the
  1:1 render puts the knob at 18..33 (on) and 2..17 (off) inside the track.
  Code's `translate-x-[18px]` / `translate-x-[2px]` on a `h-5 w-9` track with
  a `size-4` knob is exact.
- On track `#203de2` (`blue-ribbon/700`); off track `custom/outline`
  `#a3a3a380`, measured `(82,82,82)` composited over the black page, which is
  `#a3a3a3` at 50%. `bg-blue-ribbon-700` / `bg-neutral-400/50` right.
- Knob `#f9fafb` (measured `(249,250,251)`) under `shadow/lg` = two stops at
  `#00000014`, 10/15/-3 and 4/6/-4. `bg-base-background shadow-base-lg` right.

**Shadow token mapping** (compiled and diffed)
- `shadow/2xs` = 0 1px 0 0 `#0000000d` -> `.shadow-base-2xs` emits
  `0 1px 0 0 rgba(0,0,0,0.05)`. Match.
- `shadow/xs` = 0 1px 2px 0 `#0000000d` -> `.shadow-base-xs` emits
  `0 1px 2px 0 rgba(0,0,0,0.05)`. Match.

**Scroll indicator** - `607:14444`, `729:36963`, `729:36976`
- `base/sidebar-ring` `#9ca3af`, radius 999, 6px wide, **`opacity: 0` in all
  three variants**. The design draws it hidden, so the code's native
  `overflow-y-auto` scrollbar is not a deviation. This one was worth checking:
  it looks like an unimplemented affordance and is not one.

Every class I checked emits CSS. Compiled `Sidebar.tsx`, `Topbar.tsx`,
`AppShell.tsx` and `base.tsx` through the v3.4 CLI and confirmed non-empty
rules for all 21 `base-*` / `blue-ribbon-*` / `tracking-*` / `rounded-control`
utilities they use. No repeat of the `rounded-base` class of bug.

## Mismatches

Ordered by visual impact. The page is in good shape; there is exactly one real
type deviation and one latent asset gap.

### 1. `label/12` tracking is missing on every 12px sans label in the rail

- **Nodes**: `691:28162` (org name), `691:28415` and `691:28366` (nav labels),
  `I691:30034;408:15410` and `I691:30035;408:15399` (app names).
- **Measured**: all five resolve the `label/12` style, Geist Medium 12/16 at
  `letterSpacing: -1` (percent) = **-0.12px**. `get_design_context` renders it
  literally as `tracking-[-0.12px]` on each of those text nodes.
- **Code**: `Sidebar.tsx:299` (org name), `Sidebar.tsx:327` (NavItem),
  `Sidebar.tsx:501` (app name), and `Sidebar.tsx:351` / `:372` (MasterCard,
  undrawn but same style) all use `text-base-xs` with **no tracking class**.
  Verified from the compiled output: `.text-base-xs { font-size: 0.75rem }`
  and nothing else, so the fontSize token carries no letter-spacing.
- **Fix**: either add `tracking-button-xs` at those call sites - that token is
  already `-0.12px`, just misnamed for this use - or bake
  `letterSpacing: "-0.12px"` into the `base-xs` entry of
  `tailwind.config.ts`'s `fontSize` so every `label/12` site inherits it. The
  second is the correct shape and has repo-wide reach, so it is a call for the
  owner rather than a drop-in edit.
- **Confidence**: MEASURED. **Impact**: low. -0.12px per character at 12px is
  under a pixel across a whole label. It is reported because it is the only
  measured type deviation on the page and it affects four drawn labels, not
  because it will be seen.

### 2. `brand=moonshot` has no mark in the code

- **Nodes**: `433:120` (16px, in the `logo` set) and `433:123` (32px, in
  `logo-wrapper`).
- **Measured**: the sets draw **eight** brands - claude, claudecode, codex,
  moonshot, openclaw, openai, opencode, openrouter.
- **Code**: `BrandMark.tsx:17-24` declares **seven** `BrandName`s; moonshot is
  absent, and `BRAND_BY_SLUG` (`BrandMark.tsx:88-101`) has no key for it. A
  Moonshot row would fall through `brandMarkFor` to the tile's initial.
- **Fix**: add the path and slug when a Moonshot surface ships.
- **Confidence**: MEASURED, but **latent, not visible**. `rg -in 'moonshot|kimi'`
  over `src/`, `src-tauri/` and `crates/` finds no tool or proxy-domain slug,
  so no row can reach the gap today. Nothing to do now.

### 3. Brand marks are vendored rather than exported from the file

- **Nodes**: `408:14155` (claude), `408:14157` (claudecode), `408:14154`
  (codex), `408:14158` (openai), `408:14958` (openrouter), `408:14156`
  (opencode), `430:107` (openclaw).
- **Code**: `BrandMark.tsx:1-15` says the paths come from lobehub/lobe-icons
  as a stopgap "until the designer exports the drawn marks". They are
  exportable now: each of those nodes returns an SVG asset from
  `download_assets` on `408:14159`.
- **Fix**: pull the seven (eight, with moonshot) SVGs and replace the vendored
  paths, keeping `fill="currentColor"`.
- **Confidence**: INFERRED on the visual delta. Both sets are the same official
  logos, so the difference is likely nil to sub-pixel. This is a provenance
  and design-to-code-hygiene item, not a look item, and the file already
  documents the decision.

## Correct but worth stating

Things that look like findings and are not.

- **The overview variant disagrees with the other two on control padding, and
  the code follows the newer pair.** Overview (`408:15629`, `408:15635`, under
  wrapper `688:23681`) draws the org switcher at 8px uniform and the nav item
  at 8px horizontal / 6px vertical. Settings (`691:28159`, `691:28413`) and app
  (`691:29773`, `691:29774`) both draw 6/8 and 6 uniform. Code takes 6/8 and 6
  uniform, i.e. the `691:*` reading. Higher ids win, and the comments at
  `Sidebar.tsx:292-294` and `:322-326` already record exactly this. Correct as
  written; do not "fix" it toward the overview frame.
- **`base/sidebar-primary-foreground` vs `base.background`.** The Figma names
  the active-nav and selected-row fill `base/sidebar-primary-foreground`; the
  code writes `bg-base-background`. Both are `#f9fafb`. Zero visual delta. An
  alias token would improve traceability and change nothing on screen.
- **Three nav items in code, two in the frame.** `Security events` is an
  undrawn addition (AG-578), documented at `Sidebar.tsx:18-21`. Not a
  deviation.
- **`drifted` has no drawn variant.** `434:136` draws not-protected, protected
  and not-routed only. `STATUS_TEXT.drifted` at amber-600 is a consistent
  extension of the drawn family, not a mismatch.
- **`MasterCard` (`Sidebar.tsx:347`) and `InventoryState` (`:405`) are drawn
  nowhere**, on this page or in the set. Their values cannot be audited here.
  One observation for the owner rather than a finding: `InventoryState` uses
  `text-sm`, `leading-5` and `text-neutral-600` where the rest of the new UI
  uses the `base-*` ramp and `base/muted-foreground`. No frame settles it, so
  this is a consistency note only (INFERRED), and `text-neutral-600` is a real
  Figma variable elsewhere in the file.

## Could not determine

- **`Topbar.tsx` and `AppShell.tsx` are entirely unaudited by this page.** The
  Sidenav page contains no topbar, menu, banner or window-shell node; the six
  nodes it does contain are listed at the top. `Topbar.tsx:9-13` says the
  `nav/topbar` and `topnav/menu` component nodes were deleted from the file and
  the live sources are the instances inside the flow frames (the Overview
  page's `116:27225` for the menu). Auditing the 48px strip, the 224px overflow
  menu, the wordmark and the shell's banner stack needs that page, not this
  one.
  - Specifically not resolved: `text-neutral-600` on the "Connect" half of the
    wordmark (`Topbar.tsx:66`) and `text-neutral-500` on the external-link
    glyph (`Topbar.tsx:134`). `tailwind colors/neutral/500` and `/600` are real
    variables in this file and are used deliberately elsewhere, so these may
    well be correct. I did not resolve their nodes and make no claim.
- **Hover states.** No hover frame exists on this page. The app row's
  `hover:border-base-border hover:bg-base-background` (`Sidebar.tsx:478`) is
  inferred from the selected state rather than read off a frame. Relatedly,
  `437:158` resolves `tailwind colors/gray/100` (`#f3f4f6`) and
  `tailwind colors/neutral/200` (`#e5e5e5`) among its variables and I could not
  locate the node that consumes either - most likely a hover or pressed state
  on a component the variant instantiates but does not render. If a hover
  treatment is ever drawn, `gray/100` is where I would look first.
- **Focus rings.** Nothing on the page draws one. `base/sidebar-ring`
  (`#9ca3af`) exists as a variable but its only consumer here is the
  zero-opacity scroll indicator. The code's
  `focus-visible:outline-base-primary` is unsettled by the file.
- **The rail's own scroll behaviour.** The set draws the indicator hidden and
  all three variants are 686px tall with a 544px list, so the frames never show
  an overflowing rail. Whether the header and nav are meant to stay pinned is
  the code's own reading (`Sidebar.tsx:232-235`), consistent with the indicator
  being parented to the list region, but not proven by a frame.
