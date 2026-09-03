# Figma audit: `Banners ✅` component section (`744:37738`)

File `9FrccCojXy0f8QD8Wm5Lln`. Audited 2026-09-03 against
`src/components/gc/banners.tsx` and `src/components/gc/Topbar.tsx`.

**Authority note.** This canvas is a live component section - one of three
(`Menus 744:37691`, `Sidenav 408:15625`). The old Components page `113:16762`
is indeed empty, but the components were *moved*, not deleted. Where a
measurement here disagrees with a flow-frame instance, the component wins.

## Raw geometry from `get_metadata` (MEASURED)

```
744:37739 nav/topbar            1024x48
  744:37740 window-controls     60x12   @ 16,17.5   (3 x 12px ellipse, 24px pitch)
  744:37744 logo-wrapper        132.71x24 @ 437.65,11.5   -> centre x = 504.0
    744:37745 logo mark         20.71x24 @ 0,0
    744:37746 "Gate Connect"    104x24  @ 28.71,0   (gap 8)
  744:37747 toolbar-actions     76x32   @ 932,7.5   -> right edge 1008 (16px inset)
    744:37748 Button            32x32   @ 0,0
    744:37749 Button            32x32   @ 44,0      (gap 12)

744:37750 banner/update         1024x48
  744:37751 dot-matrix-light    1024x48 @ 0,0
  744:37752 update-text         186x20  @ 16,14
    744:37753 "Update available" 110x20
    744:37754 "- v0.5.0"         76x20  @ 110,0
  744:37755 banner-dismiss      93x24   @ 915,12  -> right edge 1008 (16px inset)
    744:37756 Button            61x24   @ 0,0     <-- 24px tall
    744:37757 Icon / X          16x16   @ 77,4    (gap 16 from button)

744:37758 banner/routing        1024x48
  744:37759 status-heading      249x32  @ 16,8
    744:37760 icon-wrapper      32x32   @ 0,0
      744:37761 Icon/ShieldCheck 16x16  @ 8,8
    744:37762 label             205x20  @ 44,6    (gap 12 from wrapper)
  744:37763 status-summary      148x20  @ 860,14  -> right edge 1008
    744:37764 "Routing"         51x20
    744:37765 "2 of 4 Apps"     97x20   @ 51,0

744:37766 banner/partly-routing 1024x48
  744:37767 status-heading      313x32  @ 16,8
    744:37768 icon-wrapper      32x32; 744:37769 Icon/ShieldBan 16x16 @ 8,8
    744:37770 label             269x20  @ 44,6
  744:37771 status-summary      184x20  @ 824,14 -> right edge 1008
    744:37772 "Partly routed"   86x20
    744:37773 "0 of 4 Apps"     98x20   @ 86,0

744:37774 banner/alert/multiple-apps  726x68
  744:37775 sidebar-menu-item   726x68
    744:37778 app-row-content   598x36  @ 16,16
      744:37779 icon-wrapper    36x36   @ 0,0
        744:37780 Icon/TriangleAlert 20x20 @ 8,8
      744:37781 status-summary  546x36  @ 52,0   (gap 16 from wrapper)
        744:37782 title         546x20  @ 0,0
        744:37783 body          546x16  @ 0,20
    744:37784 alert-actions     68x20   @ 638,24 -> right edge 706 (20px inset)
      744:37785 Switch          36x20   @ 0,0
      744:37786 Icon / X        20x20   @ 48,0   (gap 12)
    744:37787 icon-wrapper prev 20x20   @ -9,24  (9px overhang left)
      744:37788 Icon/ChevronLeft 12x12  @ 3,4
    744:37776 icon-wrapper next 20x20   @ 715,24 (9px overhang right)
      744:37777 Icon/ChevronRight 12x12 @ 5,4

744:37789 banner/alert/single-app 726x68  - same anatomy, no chevrons
744:37800 banner/alert/single-app 726x68  - second copy, different copy strings
```

_(report continues below; appended as measurements land)_

---

# Answers to the two questions

## 1. Is there a third button size? **Yes. MEASURED, and it is named.**

`744:37756` is an instance of the same `Button` component set (`685:20855`,
documented against `https://ui.shadcn.com/docs/components/button`). Figma
reports its variant node as:

> **`Variant=Outline, State=Default, Size=xs`** - node `685:20937`

So the set has **three** sizes, not two. `xs` is a first-class variant with
its own name, not a squashed instance. The set also publishes a variable
`height/h-6: 24` alongside the h36/h32 stops, which is corroborating: nobody
adds a height token for an accident.

Measured `xs` spec (from `get_design_context` on `744:37756`, every value a
bound variable):

| property | Figma | token |
|---|---|---|
| height | 24px | `height/h-6` |
| padding | `py 4` / `px 10` | `spacing/1` / `spacing/2-5` |
| inner gap | 8px | `spacing/2` |
| radius | **4px** (literal, not a variable) | - |
| border | 1px `#d1d5db` | `border-width/border`, `base/input` |
| fill | `#ffffff` | `base/card` |
| label | Geist Medium 12/16, tracking -1% = **-0.12px** | `text-xs/leading-normal/medium` |
| label colour | `#203de2` | `base/primary` |
| shadow | `0 1px 2px rgba(0,0,0,.05)` drop + `inset 0 4px 6px rgba(255,255,255,.4)` + `inset 0 -4px 4px rgba(0,0,0,.04)` | `shadow/xs` + the moulding pair |

**`banners.tsx:55` is already correct on every one of those**, byte for byte:
`h-6` / `py-1 px-2.5` / `rounded-control` / `border-base-input` /
`bg-base-card` / `text-base-xs font-medium leading-4` /
`tracking-button-xs` (-0.12px) / `text-base-primary` /
`shadow-base-btn-sm`. And `shadow-base-btn-sm` in `tailwind.config.ts:284-288`
expands to exactly the three shadows above - so the token the config comment
labels "the `sm` size" is in fact the `xs` treatment. `sm` and `xs` evidently
share it.

**Consequences to raise:**

1. `CLAUDE.md:87` - "**Buttons have exactly two sizes**, from the `Button`
   component set" - is **wrong as written**. The set has `default` (h36),
   `sm` (h32) and `xs` (h24). Recommend amending to three, with `xs` scoped
   to "an action inside a 48px chrome strip", which is the only place the
   file uses it. **MEASURED / high confidence.**
2. `xs` also breaks the radius/edge rule the same file states. CLAUDE.md:92-94
   pairs radius 4 with a `base.border` line and radius 8 with `base.input`.
   `xs` draws **4px on `base.input`** - a third combination. The code already
   does this; the doc should say so rather than the code looking off-contract.
   **MEASURED.**
3. `tailwind.config.ts:284` comment attributes `base-btn-sm` to the `sm`
   size; it is the shared `sm`/`xs` treatment. Cosmetic, comment only.

## 2. `Topbar.tsx` "the components were deleted" - stale, and already fixed

The comment I was pointed at is **gone from disk as of this audit**: lines
9-13 now read "Drawn as `nav/topbar` (`744:37739`) and `topnav/menu`
(`744:37692`). Both live on the **Banners** and **Menus** component canvases -
the library was not deleted, it moved". A sibling audit landed that edit
mid-run. Nothing further to do on the comment; my measurements below confirm
its new claim - `744:37739` exists and renders.

`banners.tsx:11-13` still carries the same stale claim and **has not been
fixed**: "Those component frames lived on the Components page, which the file
has since emptied; the live sources are the banner instances inside the flow
frames." All five banner components are on this canvas. See Mismatches.

---

# Component vs flow instance

Three places where this canvas contradicts what the code says it read off a
flow frame. Per the standing rule, the component wins; each needs a decision
rather than a silent code edit.

### C1. `banner/routing` says **"Routing"**, the code says **"Routed"**

`744:37764` renders the literal string `Routing`. `banners.tsx:112` renders
`Routed`, and `banners.tsx:110-111` justifies it: *"'Routed', not 'Routing':
every routed frame on Flows/Overview reads `Routed · 4 of 4 Apps`
(re-read 2026-08-21)."*

So the component and every flow instance disagree on this word. Both readings
are documented; neither is a misreading. **MEASURED that they differ.** This
is a designer question, not a code fix - "Routed" is the better English for a
state and the instances back it, but the component is the authority and says
otherwise. Raise it; do not change the code on my word.

### C2. `banner/routing`'s green tile draws a **green/700** glyph, not 600

`744:37761` Icon/ShieldCheck exports with `stroke="#15803D"` =
`tailwind colors/green/700`. (Sampled from the exported SVG, which carries the
resolved colour.) `banner/partly-routing`'s `744:37769` Icon/ShieldBan exports
`stroke="#D97706"` = `amber/600`, and the alert's `744:37780`
Icon/TriangleAlert is `#D97706` too.

`base.tsx:60-66` sets both tones to a 600 glyph, and `base.tsx:29-32`
explicitly argues for 600 *from the amber instance*: "The icon step is 600, not
700: the ShieldBan inside the Overview frame's routing banner reads
`amber/600`." That reasoning is sound for amber and was generalised to green
without a green sample. The green component says **700**.

- Amber tone: `text-amber-600` **correct, confirmed twice.**
- Green tone: design `green-700` (#15803d), code `green-600` (#16a34a).
  **MEASURED. Medium-high confidence** - one green sample, but it is the
  component's own and it is unambiguous.

### C3. `banner/update`'s dot matrix: the component draws **no dots**

`744:37751 dot-matrix-light` is an **empty 1024x48 frame** - `get_design_context`
emits `<div className="relative size-full" />` with no fill, no image, no child.
I rendered `744:37750` at 1:1 and sampled it: the strip is a clean horizontal
gradient with ±1/255 noise and **no periodic light pixels anywhere**
(row scan y=10 x300-340 and column scan x=300 y4-44 are both flat).

Measured gradient endpoints, which do confirm the code:
`left #16244 62 -> (22,36,98)`, `right -> (24,44,139)`. Predicted from the
component's two layers - `linear-gradient(270deg, blue-ribbon/800 @50%,
blue-ribbon/900 @50%)` over solid `blue-ribbon/900 #172563` - that is
`(23,37,99)` at the left and `(26,46,140)` at the right. Match within
interpolation error. **`banners.tsx:35` is correct**: `bg-blue-ribbon-900
bg-gradient-to-l from-blue-ribbon-800/50 to-blue-ribbon-900/50`
(`bg-gradient-to-l` *is* 270deg).

`banners.tsx:41-43` adds a `radial-gradient(circle, rgba(255,255,255,0.16) 1px,
transparent 1px)` at `8px 8px`, described at line 22-23 as approximating "the
design's dot matrix". The component named `dot-matrix-light` renders nothing,
so either the raster was stripped when the component was cut, or the flow
instance carries an override. **This one I did not fully settle** - see "Could
not determine". The code's overlay is a reasonable standing choice either way;
0.16 white at an 8px pitch is not measured from anything on this canvas.

---

# Verified correct

Everything below matched the component exactly. Listed so a later pass does
not re-litigate it.

**`banner/update` (`744:37750`)** - `border-b base/border`; `px-16`;
`justify-between`; the `bg-blue-ribbon-900` + `to-l 800/50 -> 900/50` gradient
(C3); `text-shadow` = `shadow/2xs` `0 1px 0 rgba(0,0,0,.05)`; "Update
available" as Geist Medium 14/20 white; `- v0.5.0` as Geist **Mono Regular**
14/20 untracked (`mono/body-14`), dash inside the mono run exactly as
`744:37754` draws it; 16px gap between button and X (`gap-4`); the 16px
Icon/X at **`#F9FAFB` = `base/primary-foreground`**, which is what
`banners.tsx:63` uses - correct, and notably *not* white.

**The `xs` Button (`744:37756`)** - every property. See Q1.

**`banner/routing` / `banner/partly-routing`** - `bg-base/card`;
`border-b base/border`; `px-16 py-8` (`px-4 py-2`); `gap-12` heading
(`gap-3`); 32px tile at radius 4/3.5 -> `rounded-control`; 16px glyph;
tile gradient `50 -> 200` with a `300` border on both tones; the heading in
Geist Medium 14/20 `base/foreground` at letterSpacing **0** (`heading/14`) -
so the code correctly carries *no* tracking class there; `Partly routed` in
`amber/600`; the count in Geist **Regular** at `base/muted-foreground`
`#6b7280`; the `·` being that node's own `list-disc` marker in the same
`base/muted-foreground`, which `banners.tsx:114-116` already documents.
Copy strings "Gate Connect is protecting you" and "Gate Connect is partly
routing your apps" match character for character.

**`banner/alert/*`** - `bg-amber/50`, `border amber/300`; `pl-16 pr-20 py-16`
(`py-4 pl-4 pr-5`); outer `gap-24` (`gap-6`); `app-row-content gap-16`
(`gap-4`); 36px amber tile at radius 4 with a 20px `amber/600`
TriangleAlert - `StatusTile size={36}` is right, glyph colour right; title
Geist Medium 14/20 `base/foreground` untracked (`heading/14`); body 12/16
`gray/600 #4b5563` (`text-gray-600` - a raw Tailwind grey, and it is what the
file draws); `alert-actions gap-12` (`gap-3`); 20px Icon/X; the chevron pucks
as 20px `rounded-[999px]` white discs on a `base/input` border with
`shadow/xs` = `0 1px 2px rgba(0,0,0,.05)` (`shadow-base-xs`) holding a 12px
chevron. Copy: "Codex isn't protected" / "Routing is set to off. Reconnect to
restore protection." and the second single-app variant's "Reconnect to
restore protection" / "This app's config changed outside Gate, so its traffic
isn't routed."

**`nav/topbar` (`744:37739`)** - `bg-base/card`; `border-b base/border`;
`px-16`; `logo-wrapper gap-8` (`gap-2`); the wordmark as Geist **SemiBold**
16/24 at tracking **-0.16px** with "Gate" `#1d37b6` (`blue-ribbon-800`) and
"Connect" `#525252` (`neutral-600`) - `Topbar.tsx:72-74` is exact;
`toolbar-actions gap-12` (`gap-3`); the icon button as 32x32,
`rounded-[4px]`, 1px `base/input`, `bg-base/card`, 16px glyph, drop
`shadow/xs`.

All classes I assert or recommend were run through the project's own tailwind
build and confirmed to emit CSS: `tracking-label-14`, `tracking-label-12`,
`tracking-button-xs`, `tracking-heading-16`, `rounded-control`, `rounded-md`,
`text-base-xs`, `shadow-base-btn-sm`, `shadow-base-xs`, `text-green-700`.
