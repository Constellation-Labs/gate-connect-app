# Figma audit: `Flows / Settings` (116:28963) vs `SettingsPane.tsx` + Settings dialogs

Read-only audit, 2026-09-03. File `9FrccCojXy0f8QD8Wm5Lln`.

Frames read: `Setting/update-1` (116:28970 and the newer 130:48905), `Setting/dimensions`
(191:79795, newest layout frame), `Setting/device name` (143:67141), `Setting/API key`
(177:74332, 177:74640), `Setting/Diagnotics report` (362:8700), `Setting/disconnect`
(143:70315), `Setting/reset` (177:73649, 177:73994).

Every value below came from `get_design_context` / `get_variable_defs` on the named node,
or from the metadata dump's own x/y/w/h. Percent letter-spacings are converted to px.
Every class proposed as a fix was run through this repo's Tailwind v3.4 config and
confirmed to emit CSS (`rounded-control` 4px, `rounded-sm` 6px, `rounded-md` 8px,
`rounded-2xl` 16px, `tracking-heading` -0.2px, `tracking-heading-16` -0.16px,
`tracking-button-xs` -0.12px, `shadow-base-xs`, `shadow-base-2xs`, `bg-gray-50` = #f9fafb).

---

## Verified correct

**Pane geometry.** `p-6` / `gap-6` (`191:79859` sits at x=24,y=24 in a 774 content area;
page-header to settings-list is 48-24 = 24; sections are 24 apart). Section `gap-3` =
the 12px between a 24px-tall section label and its card. `Card` `p-4` with `gap-4`
between rows: five of the six sections draw 16/16 either side of the rule, including the
newest (`361:8381` Diagnostics). Divider inset already documented at `SettingsPane.tsx:623`.

**Card chrome.** `rounded-md` + `border-base-border` + `shadow-base-sm` is an exact match
for `card/settings` (`rounded-[8px]`, `base/border` #e5e7eb, `shadow/sm` =
`0 1px 3px 0 rgba(0,0,0,.08), 0 1px 2px -1px rgba(0,0,0,.08)`).

**Page title.** `191:79862` resolves to `heading/20` = Geist Medium 20/**24** at -0.2px,
`base/foreground`. `SettingsPane.tsx:609`'s `text-xl font-medium leading-6
tracking-heading` is exact, and the pinned `leading-6` is right - the token export's 28 is
not what the frame draws.

**"Last 24 hours" is not a mismatch.** `191:79863` / `116:28976` carry `opacity-0`. It is
a hidden copy-paste of the Overview period label. The code correctly renders no
right-hand header text.

**Row internals.** Icon 20px at `base/foreground` #030712 (`191:79872` sampled, matching
the comment at `SettingsPane.tsx:656`). Icon-to-label gap 12. The `w-[189px]` label column
puts values at x=233, which is exactly the frame's `icon-label` 85 + `gap-[148px]`.
Description rows: label 14/20 Medium `base/foreground`, description 12/16 Regular
`base/muted-foreground`, both exact.

**Values are sans, not mono.** `116:28991` "MacBook Pro", `116:28999` "gc_a1b2c3d4",
`143:68381` "sk-gw-661b17\*\*\*" and `127:44762` all resolve to `copy/14` (Geist
**Regular** 14/20). The pane's `text-sm leading-5` is right; do not "fix" these to mono.

**Toggle group.** "On" is `copy/14` Regular `base/foreground`, 8px from a 36x20 switch -
`gap-2` and `text-sm leading-5 text-base-foreground` are exact.

**Pane action button** (`ActionButton`, `SettingsPane.tsx:743`) is an exact match to
`116:28992` / `361:8407`: h32, `rounded-[4px]`, `base/border` line, `base/primary` ink,
12px Medium at -0.12px, px-12, gap-6px, `shadow/xs` + the two insets that
`shadow-base-btn-sm` encodes. Destructive variant matches `127:44779` (fill #dc2626,
`base/destructive-foreground`, no border).

**Danger zone.** `130:48890` heading is `base/destructive` #dc2626 = `text-red-600`;
`130:48891` fill is `base/destructive-foreground` #fef2f2 = `bg-red-50`; border is
`rgba(220,38,38,0.4)` = `border-red-600/40`. The reset row's *label* stays
`base/foreground`, which the code does. All exact.

**Section order.** Device / Account / Connection / Startup / Diagnostics / About /
Danger zone matches the frame's y-order exactly (Diagnostics at y=800 between Startup and
About). All ten pane button labels match: Rename device, Copy ID, Upgrade plan (+ external
glyph), Change server, Replace key, Disconnect Gate, View report, Replay tutorial, Check
for updates, Review reset - confirmed against a render of `116:28977`.

**Dialog shell.** `rounded-2xl` (16), `p-6`, `gap`-equivalent 24 between blocks,
`border-base-border` / `border-base-destructive/40` for `edge="danger"` (`143:70617` draws
`rgba(220,38,38,0.4)`), and `shadow-base-lg` is a byte-exact match for `shadow/lg`
(`0 10px 15px -3px rgba(0,0,0,.08), 0 4px 6px -4px rgba(0,0,0,.08)`). Header gap 12.
Widths: rename/replace/disconnect 480, reset 544, diagnostics 600 (the default) - all
correct.

**Dialog button row.** `mt-6`, `gap-3`, `h-9`, secondary on `border-base-input`
(#d1d5db) with `base/primary` ink at 14px Medium -0.28px: exact. The filled primary's
`border border-white/20 bg-base-primary bg-gradient-to-b from-white/[0.08] to-black/[0.08]`
is an exact match for `363:9038` (border `alpha/80` rgba(255,255,255,.2), fill
`linear-gradient(180deg, white 8%, black 8%)` over #203de2). The destructive primary
(`143:70628`) draws no border, which the code also does.

**Dialog field label** = `label/12` (Geist Medium 12/16, `base/foreground`), field
`gap-2`: `Modal.tsx:558` is right. Read-only input drawn with no fill, no shadow, at
`opacity-60`: `Modal.tsx:578` is right.

**Reset dialog.** `heading/14` for "What happens next:" (Medium 14/20 at **0**% tracking -
so no tracking class is needed, and `text-sm font-medium leading-5` is exact). Step rows:
`rounded-[8px]`, fill #f9fafb, `base/border`, p-12, gap-12 inside, gap-8 between -
`rounded-md border-base-border bg-gray-50 p-3` + `gap-3` + `gap-2` all exact. Step title
14/20 Medium `base/foreground`: exact. Steps-to-checkbox 16: exact. The 44px danger tile
(`177:79233`) draws `rounded-[8px]` with a `red/300` #fca5a5 border and a red-50→red-200
gradient, which is precisely `lg: size-11 rounded-md` + `TONE_STYLES.danger`.

**Both dialog subtitles** resolve to `base/muted-foreground` #6b7280 at `copy/14`
(`177:73950`, `363:9033`), matching `Modal.tsx:227`. (Checked per the grey guard rather
than assumed.)

**Diagnostics report body is mono.** `363:9120` is `mono/body-14` (Geist Mono Regular
14/20 at 0 tracking), so `font-mono text-sm leading-5` at `dialogs.tsx:509` is right.

**Figma self-disagreements the code already resolves correctly**

- The Connection card (`127:44750`) draws 12px row gaps where Device, Account, Startup,
  Diagnostics and About all draw 16. Code's `gap-4` follows the five, including the
  newest node (`361:8381`). Correct as-is.
- `116:30083` draws "v0.1.4" as the Tutorial row's value; the render shows nothing there,
  so it is hidden. Code omits it. Correct.
- `127:44778` draws a masked key on the Active session row at `opacity-0`. Code renders no
  value there. Correct.
- The 32px danger tile (`143:70620`) borders `destructive/40` while the 44px one
  (`177:79233`) borders `red/300`. Code takes `red-300` for both, i.e. the newer node.
  Defensible; noted so it isn't rediscovered.

---

## Mismatches

Ordered by visual impact.

### 1. Reset dialog step descriptions are two sizes too big and the wrong grey

- **Figma** `177:73961` / `177:73968` / `177:73975`: 12px, line-height 16,
  `base/muted-foreground` **#6b7280**, tracking -0.12px, Geist **Medium** (the weight is
  set on the `text-group` parent `177:73959` and not overridden on the child, and the
  frame's style list names `label/12`).
- **Code** `src/components/gc/Modal.tsx:632`: `block text-sm leading-5 text-neutral-600`
  = 14px/20px at **#525252**.
- **Fix**: `block text-base-xs font-medium leading-4 tracking-button-xs text-base-muted-foreground`
- **Confidence**: MEASURED on size, leading and colour. The Medium weight is MEASURED from
  the emitted class but flagged: the *pane's* row descriptions are explicitly
  `Geist:Regular` `copy/12` (`361:8388`), so the step list being Medium may itself be a
  Figma slip. If you want one rule for both, drop `font-medium` and say so.

This is the loudest one: three lines of 14px grey where the frame draws 12px, inside a
544px dialog whose height the frame fixes at 448.

### 2. Diagnostics report text is drawn at the wrong ink

- **Figma** `363:9120`: `base/foreground` **#030712**.
- **Code** `src/components/gc/dialogs.tsx:509`: `text-neutral-700` = **#404040**.
- **Fix**: `text-base-foreground`
- **Confidence**: MEASURED. A full 316x550 block of mono text is the largest single area
  of ink in any Settings dialog, so a 4-stop-lighter grey reads as disabled output.

### 3. Every dialog button on this page draws radius 4, not 8

- **Figma**: `177:74577` + `177:74578` (Replace API key), `143:70627` + `143:70628`
  (Disconnect), `363:9037` + `363:9038` (Diagnostics) - all six emit `rounded-[4px]`.
  The dialogs' *inputs*, *checkbox* and two of their *tone tiles* are also 4.
- **Code** `src/components/gc/Modal.tsx:255`, `:265`, `:283`: `rounded-md` = 8px.
- **Fix**: `rounded-control` on all three dialog buttons - **but see below.**
- **Confidence**: MEASURED on the Figma side.

**This contradicts a standing line in `CLAUDE.md`** ("panes draw `rounded-control` (4px) on
a `base.border` line, **dialogs draw `rounded-md` (8px) on a `base.input` one**"). The
*edge* half of that rule is confirmed by these frames - dialog buttons do sit on
`base/input` #d1d5db while pane buttons sit on `base/border`. The *radius* half is not:
this page draws 4 in both places. I could not check the Overview/App dialogs (out of
scope), so the 8 may be real elsewhere and this may be a per-page split.

**Do not flip this on my word.** Either confirm 4 across the other dialog pages and then
change `Modal.tsx` plus the `CLAUDE.md` sentence together, or raise it with the designer
as a file inconsistency. Changing `Modal.tsx` alone would put the code at odds with its own
written contract.

### 4. `ModalField` input: radius 6 vs 4, and the editable field is missing its fill

- **Figma** `143:67467` (New device name), `177:74574` (New API key): `rounded-[4px]`,
  fill `custom/background-dark:input\30` = **#f9fafb**, `base/input` border, `shadow/xs`,
  h36, px-12.
- **Code** `src/components/gc/Modal.tsx:575`, `:580`: `rounded-sm` (6px) and
  `bg-base-card` (white).
- **Fix**: `rounded-control` on line 575; `bg-base-background` in place of `bg-base-card`
  on line 580.
- **Confidence**: MEASURED. The fill is the visible half - the frame's editable field is a
  faintly grey well inside a white dialog, and the code draws white-on-white with only the
  border separating them. The radius carries the same caveat as #3.

### 5. Reset dialog number tile: radius, numeral size and numeral ink

- **Figma** `177:73957` / `177:73958`: 36px box, `rounded-[4px]`, `base/card` fill,
  `base/border` line; the numeral is `heading/16` = **16px** Medium 24 at -0.16px in
  `base/foreground` **#030712**.
- **Code** `src/components/gc/Modal.tsx:624`: `size-9 rounded-sm ... text-sm font-medium
  text-neutral-700` = 6px radius, 14px numeral at #404040.
- **Fix**: `flex size-9 shrink-0 items-center justify-center rounded-control border border-base-border bg-base-card text-base font-medium leading-6 tracking-heading-16 text-base-foreground`
- **Confidence**: MEASURED (the numeral text node is 24px tall, and the class emits
  `text-[16px] tracking-[-0.16px]`).

### 6. Tone tile radius: 4 on the 32px tiles and on the *neutral* 44px tile

- **Figma**: `177:74563` (32, neutral) and `143:70620` (32, danger) are `rounded-[4px]`;
  `363:9029` (44, **neutral**, Diagnostics) is `rounded-[4px]`; `177:79233` (44, danger,
  Reset) is `rounded-[8px]`.
- **Code** `src/components/gc/Modal.tsx:93`, `:97`, `:101`: `sm`/`sm20` = `rounded-sm`
  (6), `lg` = `rounded-md` (8).
- **Fix**: `sm` and `sm20` → `size-8 rounded-control`. `lg` needs splitting, or accept the
  8: the two 44px tiles on this page disagree with each other and only the *newest* node
  (`363:9029`, Diagnostics) is 4.
- **Confidence**: MEASURED per node; the `lg` recommendation is INFERRED because the file
  contradicts itself and node age points the opposite way from tone.

### 7. Neutral tone tile is missing its 4% gradient

- **Figma** `177:74563` / `363:9029`: white plus
  `linear-gradient(rgba(0,0,0,.04) 0%, rgba(255,255,255,.04) 100%)`.
- **Code** `src/components/gc/Modal.tsx:49`: `bg-base-card` flat white.
- **Fix**: `border-base-border bg-base-card bg-gradient-to-t from-black/[0.04] to-white/[0.04] text-neutral-700`
  (note the frame's gradient runs dark-at-bottom to light-at-top, hence `to-t`).
- **Confidence**: MEASURED, very low visual impact (4% over white).

### 8. Disconnect dialog: header-to-body gap is 16, code gives 24

- **Figma** `164:73503` wraps the title row and the body paragraph in a stack with
  `gap-[16px]`; the dialog's own 24 gap then separates that block from the buttons. Frame
  height 198 = 25 + 32 + **16** + 40 + 24 + 36 + 25.
- **Code** `src/components/gc/Modal.tsx:245`: `mt-6` (24) unconditionally.
- **Fix**: none clean at the `Modal` level - the rename, replace-key, reset and diagnostics
  dialogs all draw 24 there and only Disconnect draws 16. Leave it, or give `Modal` an
  opt-in for the confirm-dialog shape. Recommend leaving it; the dialog renders 8px taller
  than the frame and nothing else moves.
- **Confidence**: MEASURED, low impact.

### 9. Section labels use `tracking-heading` (-0.2px) where the frame says -0.16px

- **Figma** `116:28979`, `127:44749`, `361:8380`, `130:48890`: all `heading/16` = Geist
  Medium 16/24 at -1% = **-0.16px**.
- **Code** `src/components/gc/SettingsPane.tsx:616`: `text-base font-medium leading-6
  tracking-heading`, and `tracking-heading` emits -0.2px (it is `heading/20`).
- **Fix**: `tracking-heading` → `tracking-heading-16`, matching what `AppPane.tsx:380`,
  `:661`, `Overview.tsx:163`, `:248` and `metrics.tsx:225` already do.
- **Confidence**: MEASURED. Sub-pixel per glyph (~0.4px across "Connection"), so this is
  consistency with the fix already landed on Overview/App rather than a visible defect.
  Size, weight, leading and colour are all already correct - this is the only token off.

### 10. Dialog titles use -0.2px where the frame says -0.18px

- **Figma** `177:74566`, `143:70623`, `363:9032`, `177:73949`: all `heading/18` = Geist
  Medium **18**/24 at -1% = **-0.18px**.
- **Code** `src/components/gc/Modal.tsx:222`: `text-lg font-medium leading-6
  tracking-heading` (-0.2px). `text-lg` is 18px and `leading-6` is 24, so size and leading
  are right.
- **Fix**: add `"heading-18": "-0.18px"` to `letterSpacing` in `tailwind.config.ts`
  (there is no -0.18px token today) and use `tracking-heading-18`.
- **Confidence**: MEASURED, sub-pixel. Bundle with #9 or skip both.

### 11. Diagnostics report `<pre>`: border token and missing shadow

- **Figma** `363:9034`: `base/input` **#d1d5db** border and `shadow/xs`.
- **Code** `src/components/gc/dialogs.tsx:509`: `border-base-border` (#e5e7eb), no shadow.
- **Fix**: `border-base-input shadow-base-xs` (radius 8 and the #f9fafb fill are already
  right).
- **Confidence**: MEASURED, low impact.

### 12. Scrim is `neutral-900/40`, the frame draws pure black at 40%

- **Figma** `143:67449` / `177:74560` / `362:8933` / `177:73877`: `base/black` at
  `opacity-40`.
- **Code** `src/components/gc/Modal.tsx:189`: `bg-neutral-900/40` (#171717 at 40%).
- **Fix**: `bg-black/40`.
- **Confidence**: MEASURED, barely visible.

### 13. 14px and 12px text across the pane carries no letter-spacing

`label/14` is -0.14px, `copy/14` is -0.14px, `copy/12` and `label/12` are -0.12px. The
pane's row labels (`SettingsPane.tsx:675`), values (`:708`), descriptions (`:679`), toggle
word (`:720`) and the dialog field label (`Modal.tsx:558`) all set size/leading/colour
correctly and no tracking. `tracking-button-xs` already emits -0.12px; there is no -0.14px
token.

- **Fix**: systemic and larger than this page. Recommend not touching it here; if it gets
  done, it wants a `letterSpacing` entry per step and a single sweep.
- **Confidence**: MEASURED, sub-pixel per glyph.

### 14. Reset checkbox is a native input, the frame draws a styled box

- **Figma** `I177:73976;46:68`: 16px box, `rounded-[4px]`, `base/input` border, #f9fafb
  fill, `shadow/xs`. Label is `copy/14` Regular `base/foreground`, 8px gap.
- **Code** `src/components/gc/Modal.tsx:660`: `<input type="checkbox">` with `size-4
  accent-blue-ribbon-700`; the label styling is correct.
- **Fix**: none proposed. The accessibility argument in the comment is sound and the
  browser default is close. Logged so it isn't reported as new.
- **Confidence**: MEASURED difference, deliberate implementation choice.

### 15. Report height

`363:9034` is a fixed 316px frame with `pt-16 pb-0`. Code caps at `max-h-72` = 288px
including padding. INFERRED that the frame's 316 is illustrative rather than a spec, since
the report's length is data-driven. No change recommended.

---

## Copy differences

Neither of `CLAUDE.md`'s two decided exceptions is reported below; both are intact in the
code and correctly annotated (`dialogs.tsx:1153` for `New API key`, `:1242` for the
Disconnect body).

### RAISE: a possible third "the drawn words describe something the action does not do"

- **Figma** `116:29086` / `363:10896`: the Startup card's single **Notifications** row is
  described as "Alert me when a request is blocked or flagged", with one switch.
- **Code** `src/components/gc/SettingsPane.tsx:406`: that row is described "Alert me about
  routing problems", and the blocked/flagged wording moved down to two new rows
  ("Blocked requests" `:427`, "Flagged requests" `:441`) plus a third, "Notification
  sound" `:455`. The frame draws none of the three.
- The code comment at `:400-405` gives the reasoning and it is the same shape as the two
  standing exceptions: one switch cannot honestly claim two different notification
  categories. **But `CLAUDE.md` does not record it**, and `CLAUDE.md` says explicitly "If
  you find a third of these, raise it rather than deciding it." It has already been
  decided in code. Per the standing instruction I am raising rather than resolving it:
  either add it to the `CLAUDE.md` exception list (it is the same argument, and the AG-594
  / AG-578 split is real work the frame predates), or take it to the designer so the frame
  grows the rows it is missing.

### Typo corrections (not semantic exceptions)

- **Diagnostics dialog subtitle.** `363:9033` reads "The state of this **installed**, as
  text you can hand to someone else". `dialogs.tsx:502` ships "this install". This is a
  spelling fix, not a claim change, and it is already annotated at `dialogs.tsx:501`. Different in
  kind from the two standing exceptions; I do not count it toward the "third".

### Placeholders

- **Rename device.** `143:67469` draws the placeholder **"Enter a device name"** in
  `base/muted-foreground`. `RenameDeviceDialog` (`dialogs.tsx:1140-1144`) passes no
  `placeholder` at all, so the New-device-name field is empty.
  **Fix**: add `placeholder="Enter a device name"`. MEASURED.
- **Replace API key.** `177:74575` draws **"Enter or paste your new key "** (with a
  trailing space). `dialogs.tsx:1199` ships `"sk-gw..."`.
  **Fix**: `placeholder="Enter or paste your new key"` if the file wins here; the drawn
  sentence is not a false claim, so the "file wins" default applies and I would take it.
  MEASURED.

### Mono on the API-key dialog fields

`ReplaceApiKeyDialog` passes `mono` on both fields (`dialogs.tsx:1192`, `:1198`). The frame
draws both in Geist **Regular** (`177:74571`, `177:74575` are `copy/14`), and so does the
pane's own API-key value row (`143:68381`). So the dialog's `font-mono` diverges from the
frame *and* from the surface that opens it, while agreeing with `CLAUDE.md`'s "mono for
identifiers". Worth one decision either way; I am not calling it a defect.

### Sections and rows the code adds

Help, Sign-in method, Gate certificate, Blocked/Flagged requests, Notification sound and
the "See what is collected" description link have no frame on this page. All are annotated
in `SettingsPane.tsx` as deliberate. Not reported as mismatches.

### Stale comment

`src/components/gc/SettingsPane.tsx:72` says "Diagnostics is the one row the Figma does not
draw." The file now draws a whole **Diagnostics** section (`361:8378`, and `363:10901` on
the newer frame) sitting between Startup and About, with both rows and copy matching the
code word for word ("Share diagnostic data" / "Send Gate errors and routing stats to help
fix problems. Never prompts or credentials." / "Diagnostics report" / "Everything Gate
knows about this install, as shareable text." / "View report"). The code is right; the
comment is 2026-08-era and should be deleted or rewritten.

---

## Could not determine

- **Whether dialog buttons draw 8px anywhere.** Finding #3 is decisive for this page's six
  instances but says nothing about the Overview/App/quit/model dialogs, and
  `CLAUDE.md` asserts 8 as a decided rule. Needs one more page read before anything moves.
  `685:20928` (the `Button` component's Outline/default variant) is on another page and
  `get_design_context` refused it ("invalid node selection"), so I could not check what the
  component itself says versus its instances.
- **Tone tile glyph colour.** The frames export the tile's fill and border but the icon
  comes back as an SVG asset with no resolvable fill, so `TONE_STYLES`' `text-red-600` /
  `text-neutral-700` on the glyph is unverifiable from this page. The existing comment at
  `Modal.tsx:38` already says this.
- **The 44px tone tile radius rule.** `363:9029` (neutral) is 4 and `177:79233` (danger) is
  8. Node age favours 4, tone favours reading it as "danger tiles are rounder", and neither
  is a rule the file states. Left to a designer question.
- **Step-description weight.** Emitted as Geist Medium via parent inheritance, and
  `label/12` is in the frame's style list, but the identical descriptions one card up in
  the pane are Regular `copy/12`. The file does not settle whether the reset steps mean
  Medium.
- **The Connection card's 12px gaps.** Genuinely drawn (`127:44763` at y=44 after a row
  ending at 32). Five other sections draw 16. Taken as a slip, not verified as one.
