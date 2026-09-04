# Figma component library vs implementation

Audit of the Figma primitives against `src/components/gc/base.tsx`,
`Modal.tsx`, `banners.tsx`, `Icon.tsx` and `tailwind.config.ts`.
File `9FrccCojXy0f8QD8Wm5Lln`. Read-only: no source file was modified.

Repo is `tailwindcss 3.4.19`, so the v4 -> v3 shadow shift applies throughout.

## Headline: the Components page is empty

The audit target does not exist any more.

- `113:16762` resolves as a canvas named **Components** with **zero children**
  and 0x0 bounds.
- `113:16794` (`nav/sidebar/overview`, the known child in the brief) returns
  *node not found*.
- `get_metadata` with no nodeId lists exactly **one** page: `319:4686`
  **Design docs** - four text frames holding a generated token dump
  (`tokens.css`, `tokens.json`, `tailwind.tokens.js`, `design.md`).
- `search_design_system` returns **no** published components for Button,
  so nothing is exposed as a library either.

The component *sets* partly survive off-page and are still reachable by id:
`408:14253` (Switch) fetches fine, and `685:20855` / `685:20928` /
`685:20942` / `685:20856` (Button and its variants) come back only as
component-description references, never as fetchable nodes.

`base.tsx:52` already records this ("the file has since deleted [the]
Components page ... the live sources are the banner instances inside the flow
frames"), and that comment is correct. Everything below is therefore measured
off **live instances in the flow frames**, which is what CLAUDE.md's
"match what the frame renders" rule prefers anyway. The consequence worth
noting: there is no longer a component-set node to appeal to when an instance
and the set disagree, so the "component over instance" tiebreak written into
CLAUDE.md is no longer executable for Button.

The two token layers the file exposes are both intact and both were checked:
raw `tailwind colors/*` scales **and** the `base/*` semantics.

---

## Verified correct

All MEASURED unless noted.

1. **Card** - `base.tsx:37`. `rounded-md border border-base-border
   bg-base-card shadow-base-sm`. Frames `228:89333`, `228:89345`,
   `228:89517`, `661:17103` all draw radius **8**, 1px `base/border`
   **#e5e7eb**, `bg base/card` white, `shadow/sm`. The `shadow/sm` variable
   resolves to two drop shadows at `#00000014` (alpha 0x14 = 7.8%), offsets
   (0,1) blur 3 spread 0 and (0,1) blur 2 spread -1 - i.e. exactly
   `base-sm` at `tailwind.config.ts:308`. Byte-for-byte.

2. **BaseSwitch** - `base.tsx:132-139`. `408:14251` (on) / `408:14252` (off):
   36x20 track, `rounded-full`, 16px thumb at a 2px inset, on
   `blue-ribbon/700` **#203de2**, off `custom/outline`
   **rgba(163,163,163,0.5)**, thumb `base/background` **#f9fafb** with
   `shadow/lg`. Code's `bg-neutral-400/50` emits
   `rgb(163 163 163 / 0.5)` - the design's own value to the byte. The
   `translate-x-[18px]` on state is arithmetically right (36 - 16 - 2). The
   docstring's 36x20 claim is confirmed; the generated code's
   `border-2 border-transparent` is a no-op Figma stroke (the vertical maths,
   16px thumb centred in a 20px track, proves the inset is 2px).

3. **`base-btn`** (default / outline) - `tailwind.config.ts:279-283`. Instance
   `130:48311` of `685:20928`: h**36**, px 12 / py 10, radius **8**, 1px
   `base/input` **#d1d5db**, gap 8, `text/sm` 14/20 medium at -0.28px,
   `base/primary` label. Shadow = `shadow/xs` drop
   (`0 1px 2px rgba(0,0,0,0.05)`) plus
   `inset 0 4px 4px rgba(255,255,255,0.24)` and
   `inset 0 -4px 4px rgba(0,0,0,0.02)`. Exact match, all three stops.

4. **`base-btn-primary`** - `tailwind.config.ts:289-294`. Instance `130:48312`
   of `685:20856`: `linear-gradient(180deg, rgba(255,255,255,0.08),
   rgba(0,0,0,0.08))` over `rgb(32,61,226)`, border `alpha/80`
   rgba(255,255,255,0.2), label `base/primary-foreground` **#f9fafb**,
   drop `0 1px 3px 0 / 0 1px 2px -1px` at 8%, insets
   `0 4px 4px rgba(255,255,255,0.1)` (`alpha/90`) and
   `0 -4px 4px rgba(0,0,0,0.08)`. Exact match, and `Modal.tsx:288`
   reproduces the gradient and the `border-white/20` correctly.

5. **Dialog radius is 16px - confirmed.** `130:48278` (`card/organization`)
   draws `rounded-[16px]` with 1px `base/border` and `shadow/lg`.
   `Modal.tsx:195` is `rounded-2xl` (16px) + `shadow-base-lg`, and
   `base-lg` matches `shadow/lg` byte-for-byte. CLAUDE.md's claim that the
   measured 16 replaced a "locked" 12 is correct and the 12 is dead.

6. **Pane vs dialog button edge - CLAUDE.md's rule confirmed, 10/10.** Every
   pane-level button instance draws radius **4** on a `base/border` line
   (`228:89551`, `228:89560`, `661:17129/17143/17157/17171/17185`); every
   dialog button draws radius **8** on `base/input` (`130:48311`,
   `130:48312`). The worked example in CLAUDE.md is right.

7. **Radius scale** - `tailwind.config.ts:334-348` mirrors the Figma
   `radius/*` variables one-to-one: xs 2, sm 6, md 8, lg 10, xl 14,
   2xl 16, 3xl 24, 4xl 32, full 9999. `control: 4px` is a local addition
   with no named Figma stop, and it is justified: 4px is what all ten
   button instances and the status tiles actually draw.

8. **Shadow tokens** - `base-xs` = `shadow/xs`, `base-sm` = `shadow/sm`,
   `base-md` = `shadow/md`, `base-lg` = `shadow/lg`, all exact including the
   design's 8% alpha (not v3's 10%). The one-step v4->v3 remap is correctly
   absorbed. `base-2xs` matches `shadow/2xs`
   (`0 1px 0 0 rgba(0,0,0,0.05)`) and is correctly kept off buttons.

9. **letterSpacing** - all confirmed against `get_variable_defs`, with the
   percentage-to-px conversion applied: `mono/eyebrow` 8% at 12px ->
   `eyebrow: 0.96px`; `mono/label-12` 6% -> `label: 0.72px`;
   `heading/16` -1% -> `heading-16: -0.16px`;
   `text-sm/leading-normal/medium` -2% -> `button-sm: -0.28px`;
   `text-xs/leading-normal/medium` -1% -> `button-xs: -0.12px`.

10. **`BADGE_STYLES` colours** - `base.tsx:277-287`. Verified against the
    live badges `661:17123` (ERROR), `661:17179` (BLOCKED), `661:17151`
    (FLAGGED), `661:17137` (REDACTED), `661:17165` (ALLOW), and
    cross-checked against the CSS this repo's own Tailwind emits:

    | badge | Figma | emitted CSS | |
    |---|---|---|---|
    | ERROR / BLOCKED | red/100 #fee2e2 on red/700 #b91c1c | `rgb(254 226 226)` / `rgb(185 28 28)` | ok |
    | FLAGGED | amber/100 #fef3c7 on amber/700 #b45309 | `rgb(254 243 199)` / `rgb(180 83 9)` | ok |
    | REDACTED | violet/100 #ede9fe on violet/800 #5b21b6 | `rgb(237 233 254)` / `rgb(91 33 182)` | ok |
    | ALLOW | gray/100 #f3f4f6, text token unset | `rgb(243 244 246)` / `rgb(75 85 99)` | ok |

    All five byte-exact. `allow`'s `text-gray-600` is a deliberate,
    documented AA deviation and the Figma leaves ALLOW's text token unset,
    so nothing is being contradicted. The violet-not-purple call is right.

11. **Pill typography** - `base.tsx:242`. Geist Mono Medium 12/16 at 0.72px
    = `font-mono text-base-xs font-medium leading-4 tracking-label`. Correct.
    (Its *geometry* is not - see M1.)

12. **StatusTile** - `base.tsx:83`. `228:89324`: `bg-gradient-to-b`
    from `amber/50` #fffbeb to `amber/200` #fde68a, border `amber/300`
    #fcd34d, `rounded-[4px]`, p 8. Matches `TILE_TONES.amber` +
    `rounded-control` exactly. The docstring's note about `228:89662`
    reading 3.5px is confirmed - that node is a 0.875-scaled instance
    (border 0.875px, radius 3.5px), so 4 is the true value.

13. **banners.tsx update button** - `banners.tsx:55`. `334:805`: h**24**,
    px 10, py 4, radius 4, 1px `base/input`. Code is
    `h-6 px-2.5 py-1 rounded-control border-base-input`. Exact match - and
    note this is a *third* button size the design contract does not name
    (see M4).

14. **ModalField input** - `Modal.tsx:579-580`. `base/input` border with
    `shadow-base-xs`, read-only variant transparent at 60% with no shadow.
    Consistent with `shadow/xs` and with what `228:89523` draws
    (`p-12`, radius 4, `base/input`).

15. **Every class checked emits CSS.** 40 candidate classes
    (`rounded-control`, all four `shadow-base-btn*`, all six `tracking-*`,
    the `base-*` colours and font sizes, the badge ramps) were compiled
    through the project's own `tailwind.config.ts`; **none** was a
    no-op. No repeat of the `rounded-base` dead-class bug.

16. **Raw-scale layer is sound.** The config uses `extend`, so Tailwind's
    own gray / amber / violet / red / neutral ramps stay untouched and match
    the Figma `tailwind colors/*` variables byte-for-byte (verified in the
    emitted CSS above). Only `blue` is redefined as OKLCH, which is exactly
    why the `chart.*` group exists and why `chart.messages` carries
    `#60a5fa` by hand. Nodes that use a raw scale **on purpose**: the badges
    above, the status tiles (`amber/50-200-300`), the kpi eyebrow
    (`gray/600` on `228:89335`), the app status line (`neutral/500`,
    `neutral/600` on `116:30213`), the switch on-track
    (`blue-ribbon/700`). Nodes that use the semantic layer: cards, buttons,
    inputs, all body ink. Both layers are legitimate and the config mirrors
    both.

---

## Mismatches

Ordered by how many surfaces inherit them.

### M1 - `Pill` geometry is wrong in three properties

- **Node:** `661:17123`, `661:17137`, `661:17151`, `661:17165`, `661:17179`
  (the five live `status-badge` nodes in `table/recent-activity` `661:17103`)
- **Measured:** `px-[8px] py-[4px] rounded-[4px]` -> 24px tall
- **Code:** `src/components/gc/base.tsx:242` -
  `rounded-xs px-1.5 py-0.5` -> 2px radius, 6px/2px padding, 20px tall
- **Fix:** `rounded-control px-2 py-1`
- **Confidence:** MEASURED. Also confirmed visually against a render of
  `661:17103`: the badges are visibly pill-padded and softly rounded, not
  tight 2px rectangles.

Inherited by every row of the per-app recent-activity table and the live
security feed - the most-repeated element in the app. The radius is wrong
by 2px *and* the badge is 4px shorter than drawn, so rows sit tighter than
the design.

Note: `base.tsx:250` cites `272:3150` as the source, and that node **no
longer exists** in the file. The `661:*` nodes are far newer and are what
the frame renders now, so they win under the brief's newest-id rule.

### M2 - `base-btn-sm` uses the wrong inset highlight for 7 of its 10 call sites

- **Nodes:** `base/border` buttons `228:89551`, `228:89560`,
  `661:17129`, `661:17143`, `661:17157`, `661:17171`, `661:17185`
  vs `base/input` buttons `228:89649`, `228:89650`, `334:805`
- **Measured:** the file has **two** `sm` mouldings, and the split is
  perfect across all ten instances - it is a variant distinction, not the
  file disagreeing with itself:

  | border token | top inset highlight | instances |
  |---|---|---|
  | `base/border` | `inset 0 4px **4px** 0 rgba(255,255,255,0.4)` | 7 (incl. all five newest, `661:*`) |
  | `base/input` | `inset 0 4px **6px** 0 rgba(255,255,255,0.4)` | 3 |

  The bottom lip is `inset 0 -4px 4px 0 rgba(0,0,0,0.04)` and the drop is
  `shadow/xs` in both, so only the highlight blur differs.
- **Code:** `tailwind.config.ts:287` - one token at **6px**, applied to both
  kinds. So the three `base/input` call sites (`banners.tsx:55`,
  `Tray.tsx:123`, `Tray.tsx:436`) are right, and the seven `base/border`
  ones (`Overview.tsx:347`, `AppPane.tsx:634`, `AppPane.tsx:802`,
  `AppPane.tsx:820`, `SecurityPane.tsx:206`, `Sidebar.tsx:443`,
  `SettingsPane.tsx:746`) are getting a highlight 2px softer than drawn.
- **Fix:** keep `base-btn-sm` at 6px for the `base/input` buttons and add a
  4px sibling for the `base/border` pane buttons. If one token must serve
  both, set it to **4px** - that is both the majority and the newest nodes.
- **Confidence:** MEASURED on the values, 10/10 on the correlation.
  **Low visual severity** - 2px of blur on an inset highlight is close to
  invisible at size. Reported because the token layer is meant to be
  traceable, not because the buttons look wrong.

### M3 - `EmptyNote` tile radius and sentence type

- **Node:** `228:89721` (container), `228:89725` (tile), `228:89716` (text)
- **Measured:**
  - tile: `rounded-[4px]`, `size-[36px]`, `bg base/card` white,
    1px `base/border`
  - sentence: Geist **Medium 16/24, tracking -0.16px** (`heading/16`) at
    `base/muted-foreground`
  - container: `gap-[12px]`, `pb-[24px]`, **no top padding**
- **Code:** `src/components/gc/base.tsx:211` - tile `rounded-sm`
  (**6px**); `base.tsx:215` - `text-sm leading-5` (**14/20, regular**);
  `base.tsx:208` - `gap-3 py-6`
- **Fix:** `rounded-control` on the tile;
  `text-base leading-6 font-medium tracking-heading-16` on the `<p>`;
  `pb-6` rather than `py-6`. `gap-3` is already correct at 12px.
- **Confidence:** MEASURED. The radius and the 14-vs-16px type are both
  visible; the missing `font-medium` is the most noticeable of the three.

Inherited by 8 call sites across `Overview`, `AppPane`, `SecurityPane` and
`metrics`. Worth noting the sampled node is the one call site that is
otherwise perfect: `metrics.tsx:235` renders "No messages sent in the last
24hrs" with `icon="chartColumn"`, matching `228:89716`'s copy and
`228:89730`'s BarChart glyph exactly.

Minor, same component: `base.tsx:201` calls `messageCircleX` "the design's
own" default, but the node the docstring cites (`228:89721`) draws
**BarChart**. Every call site passes an explicit icon, so nothing renders
wrong - the comment is just inaccurate about its own citation.

### M4 - the two-size button rule is incomplete (documentation, not code)

- **Nodes:** `334:805` (h24, `base/input`, banner) and `661:17129` /
  `17143` / `17157` / `17171` / `17185` (h24, `base/border`, table-row
  "View" - the newest button nodes in the file)
- **Measured:** h**24**, px 10, py 4, radius 4. A third geometry.
- **Contract:** CLAUDE.md says "Buttons have exactly two sizes ... `default`
  (h36, 10/12) and `sm` (h32, 8/12)".
- `banners.tsx:55` already implements the h24 `base/input` one exactly, so
  the **code is ahead of the doc** here.
- **But** the table-row buttons drift the other way:
  `AppPane.tsx:802` and `SecurityPane.tsx:206` draw "View" at
  `h-8 px-3 gap-1.5` where `661:17129` draws `h24 px-2.5 py-1`. That is a
  MEASURED mismatch - 8px too tall and 2px too wide on padding - on the
  newest button nodes in the file.
- **Fix:** two parts. (a) CLAUDE.md should name the third size. (b) the two
  table "View" buttons should take h24/4-10. Per CLAUDE.md's own standing
  instruction I am **raising (a) rather than deciding it**, since a third
  size changes a rule the whole app is measured against.
- **Confidence:** MEASURED on the geometry; the doc change is a judgement
  call for you.

### M5 - filled-primary label has no text shadow

- **Node:** `130:48312`, label `I130:48312;37:925`
- **Measured:** the label carries `shadow/2xs` as a **text** shadow -
  `0 1px 0 rgba(0,0,0,0.05)`
- **Code:** `Modal.tsx:288` has none. (`banners.tsx:45` does do this, with
  an arbitrary `[text-shadow:0_1px_0_rgba(0,0,0,0.05)]`, so the pattern
  already exists in the repo.)
- **Fix:** add the same arbitrary property to the filled-primary button.
  Tailwind 3.4 has no `text-shadow` utility, so it has to be arbitrary.
- **Confidence:** MEASURED, very low severity.

---

## Could not determine

- **Skeleton** (`base.tsx:172`, `rounded-sm bg-gray-200`). **No frame draws
  a loading placeholder.** CLAUDE.md is explicit that the loading state is
  inferred from the "a figure is a measurement" rule rather than from a
  frame, and the Figma draws only the no-traffic case. Nothing to check
  against. Aside: `gray-200` is `#e5e7eb`, coincidentally identical to
  `base/border`; there is no Figma token for a skeleton fill, so the value
  is unanchored either way.

- **`base-btn-destructive`** (`tailwind.config.ts:297-302`). **No
  destructive button instance appears in any frame I could reach**, and the
  Button set is not fetchable. Its distinctive choice - a `red-700`-at-50%
  bottom lip rather than black - is the repo's own reasoning recorded in the
  comment, not a measurement. Unverified in both directions: I can neither
  confirm nor contradict it.

- **`heading/24` and `heading/14` have no `letterSpacing` token.**
  `heading/24` is Geist Medium 24/28 at -1% (-0.24px) and `heading/14` is
  Medium 14/20 at **0%**. Neither is in `tailwind.config.ts`'s
  `letterSpacing` group; `228:89336` (the kpi figure) carries an inline
  `tracking-[-0.24px]`. This is a token-layer gap outside my assigned files,
  flagged rather than measured against call sites. `heading/14` at 0% is the
  one to watch: every other heading step is -1%, so applying
  `tracking-heading` (-0.2px) to a 14px heading anywhere would be wrong.

- **Component-set values in general.** With `685:20855` unfetchable, any
  claim about what the *set* specifies (as opposed to what an instance
  draws) now rests on `plans/new-app-ui-figma.md:958`, which is a code
  artifact rather than the file. Where that plan and an instance disagree,
  there is no longer a way to adjudicate from Figma.

---

## Summary

The primitive layer is in good shape. Colour, shadow, radius-scale and
typography tokens are byte-exact against the Figma variables across both the
semantic `base/*` layer and the raw `tailwind colors/*` layer, the four
button mouldings are faithfully reproduced (three of four confirmed exactly),
the 16px dialog radius and the pane-4 / dialog-8 button rule are both
confirmed, and no class in the group is a silent no-op.

Three real drifts, in priority order: **`Pill`'s geometry** (M1 - three
properties, every table row, and its cited source node is deleted),
**`EmptyNote`'s tile radius and sentence type** (M3 - 8 call sites), and
**`base-btn-sm`'s highlight blur** (M2 - correct for 3 call sites, 2px soft
for 7, low visual impact but the token should be split). Plus one contract
gap worth your decision: the Figma draws a **third button size** (h24/4-10)
that CLAUDE.md's "exactly two sizes" does not name, which the banner already
implements and the two table "View" buttons do not.

The finding that outranks all of them: **the Components page is gone**, so
`113:*` citations in the code are dangling and the component-set tiebreak in
CLAUDE.md can no longer be executed. Comments citing `113:*` and `272:3150`
should be repointed at the live instances (`661:*`, `408:*`, `228:*`, `130:*`)
before the next audit has to rediscover this.
