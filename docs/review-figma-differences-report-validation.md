# Validation of `gate-connect-figma-differences.html`

Source under validation: `~/Downloads/gate-connect-figma-differences.html`, dated
"Reviewed 9-10 September 2026", against source `77586f7`.

Validated 2026-09-10 against branch `feat/new-app-ui` at `5e2fee30`, and against
the Figma file `9FrccCojXy0f8QD8Wm5Lln` over MCP.

**Model-selection findings were excluded on request**: S-01, S-04, S-17, C-04,
C-05, C-14 and M-12 were not validated. M-13 was checked only for its trigger and
button text, because it is a billing acknowledgement rather than a picker.

## Method and what "validated" means here

Two halves per finding, checked separately.

- **App half.** Read at the reviewed revision (`git show 77586f70:<path>`) and at
  HEAD, so a mismatch can be told apart from drift. `Overview.tsx`, `metrics.tsx`,
  `base.tsx`, `Icon.tsx`, `tailwind.config.ts`, `AppPane.tsx`, `Onboarding.tsx`,
  `setup.tsx` and `SettingsPane.tsx` are byte-identical between the two, so their
  findings hold at both. `NewUiApp.tsx`, `TrayApp.tsx`, `dialogs.tsx`,
  `banners.tsx`, `Topbar.tsx`, `Tray.tsx`, `groups.ts`, `recovery.ts`, `reopen.ts`
  and `useRunningApps.ts` did drift.
- **Figma half.** 18 nodes resolved directly: `121:34782`, `864:3466`, `121:35174`,
  `130:48905`, `177:74332`, `143:70315`, `362:8700`, `437:158`, `202:80095`,
  `207:81491`, `209:84046`, `232:4370`, `212:84747`, `212:85100`, `744:37709`,
  `744:37758`, `177:79238`, plus fill variables on `706:10514`, `706:10605`,
  `864:3511`, `232:4448` and `143:70543`.

Geometry claims that source cannot settle exactly (rendered heights, wrap points)
were reconstructed from the box model plus the bundled Geist metrics. That model
reproduces five of the report's six app-side geometry figures to the pixel
(466, 549.84, 480, 480, 289, 290), which is what makes its one disagreement worth
raising rather than dismissing.

## Headline

**The report is substantially accurate and the measurement work behind it is
sound.** Of the 100 non-model findings checked, 88 are confirmed, 7 are imprecise
in a way that matters, 3 are wrong, 1 went stale after the reviewed revision, and
1 cannot be settled from source at all.
Every Figma value I re-resolved matched the report except one, and the card
heights, bar widths, badge widths, row steps and dialog frame sizes all came back
exactly as stated.

Three things qualify it:

1. **Every dialog y-position in the report was measured in a 720px-tall window,
   which the app cannot be.** The main window ships 1280x800 with a 1024x800 floor
   (`src-tauri/tauri.conf.json`, `MAIN_MIN_SIZE` at `src-tauri/src/lib.rs:1597`).
   D-01's y127 is exactly `(720 - 466) / 2`, and the scrim centres on the whole
   window (`AppShell.tsx:88`). At the shipped height the diagnostics dialog sits at
   y167 and the device-name heading at y330, not y290. The Settings frames in Figma
   genuinely are 1024x**720**, so the comparison is internally consistent, but its
   vertical offsets describe a window size no user can reach. The report's own
   preamble ("Figma is drawn at the minimum acceptable width and height") is wrong
   on the height. Onboarding is unaffected: that window really is 720 tall
   (`lib.rs:3322`, `.inner_size(1080.0, 720.0)`).
2. **A large share of the non-model findings are already open questions in this
   repo**, with sharper evidence, in `docs/figma-questions-for-design.md`. The
   report does not cite it. Map below.
3. **The report is measured against two generations of Figma frame at once** and
   does not always say which. `121:34782` is the retired 720px-content Overview
   (12 buckets, 48px bars); `864:3466` is the current 1280x800 one (976px content,
   944px chart, 24 buckets, 32px bars). Card heights are identical in both, so
   findings 03 to 06 are unaffected, but findings 02, 08 and 12 read as app defects
   when they are frame-generation differences that the app already resolved the
   newer way.

## Errors to correct

| Ref | Problem | Evidence |
|---|---|---|
| C-10 | **Figma evidence is inverted.** The report says `177:74332` carries copied rename-field wording and treats "New API key" as the app's accepted correction. `177:74332` draws `Current API key` (`177:74569`) and `New API key` (`177:74573`), identical to the app. The slip is in the FILLED state `177:74640`. | Resolved the node; `CLAUDE.md` records this trap explicitly, including that a "newest node wins" tiebreak points the wrong way here |
| C-13 | **Wrong mechanism.** "Time formatting follows the chart axis" and "locale may differ" are both untrue. Tooltip and axis use two different formatters on purpose: `hourHeading` gives `07:00` (`metrics.tsx:103`, called at `:465`), `hourTick` gives `07` (`:88`, called at `:331`), and the docstring at `:92-102` argues for the split. No locale API is on either path; `bucket.label` is `String(new Date(b.hour).getHours())` (`activity.ts:263`), a timezone dependency. The Figma half is right: `744:37709` is a text node named `12`. | Resolved both |
| D-02 | **Height wrong and cause wrong.** 480 width confirmed (`dialogs.tsx:1567`). The height reconstructs to **226**, not 206: the body is 866.6px of 14px Geist in a 430px column, so two lines are arithmetically impossible (2 x 430 = 860) and greedy wrap gives three. 206 is what the *drawn* copy (669.2px, two lines) produces in the app's box. The dialog also moves in 20px line steps, never 8. Real delta against the 480x198 frame is 28px: 20 for the extra line plus D-14's 8px gap. | Frame size 480x198 confirmed at `143:70617` |
| D-15 | **Wrong, and wrong the same way C-10 is.** The cited `232:4448` is `Variant=Outline, State=**Disabled**, Size=default` carrying **`opacity=0`**: an invisible placeholder on the Welcome frame, which has no previous step. Its `#FAFAFA` is the Disabled variant's fill. The Previous button the frames actually render (`232:4359`, on the intro steps) is `#FFFFFF`, which is what the app draws. No fix needed; the app was already right. | Resolved both instances |
| Left-menu inventory | Six errors. **(a)** "Topbar: lockup, routing count, More menu" - `Topbar` has no routing count at either revision; it takes `{menuOpen, onMenuToggle, onMenuSelect}` and `grep -n routing src/components/gc/Topbar.tsx` is empty. The counts live on sidebar group eyebrows (`Sidebar.tsx:317-320`) and in the tray (`Tray.tsx:48`). **(b)** "Other tools" is not fed by `catalog.rs:351-390`: the fallback takes unplaced *tools* only (`groups.ts:557-564`), never a domain. **(c)** The table omits an OpenRouter group, which the rail does draw (`provider.rs:137-148`, supported and not staging-gated). **(d)** `notices.ts:130-160` is not the Error > Recovery > Reopen precedence: `buildNotices` orders `master-off, needs-trust, drifted, error` with error **last**; the banner precedence is carried by the `NewUiApp` chain alone. **(e)** The four status strings are `STATUS_TEXT` in `Sidebar.tsx:177-180`, not `verdict.ts:84-119`. **(f)** Range slips: Experimental members are at `groups.ts:79/89` not 68-70; the inventory card is 461-503 not 463-505. | |
| M-10 | **Explanation nearly inverted.** "No registry tool reports `can_reopen=true`" is false, and it is not why M-10 is rare. M-10 is gated on `allVerified`; Claude Desktop and ChatGPT are the only `can_reopen = true` rows (`lib.rs:1809`, `:1825`) and both have `verifiable = false`, so they can never reach it. | |
| I-02 | **STALE at HEAD, plus a mislabel.** The buttons were renamed "Reopen tool"/"Reopen {tool}" to **"Close tool"/"Close {tool}"** (`banners.tsx:207`, `:274`). Also, "Routing didn't finish coming back" belongs to `RecoveryBanner` (`banners.tsx:597`), and `banners.tsx:160` is `ReopenAlert`, not `ReopenBanner` (`:229`). | |
| M-01 | Button inventory incomplete: the primary relabels to **"Continue"** when radio 2 is selected (`dialogs.tsx:2283-2291`). The report does note the label parenthetically in its prose and then contradicts itself in the state table. | |
| L-01 | The only `VITE_NEW_UI=0` setter is `playwright.config.ts:55`, not "the pixel-review harness". Substance (no shipped build sets it) holds. | |
| C-02 | Misquote by one character: the app renders `That’s` with U+2019 (`Onboarding.tsx:135`). | |
| D-16 | Understated, not wrong. There is one recipe, `h-9 rounded-md` (`Modal.tsx:282/296/315`), so **every** dialog button is 8px, not "several". No dialog button is 4px anywhere; `rounded-control` appears only on model-picker rows. | Design conflict itself confirmed and already tracked as question 5 |
| 02 (Overview) | Mechanism right, numbers retired. 16px versus 17px inset holds (CSS border-box eats 2px against Figma's inside stroke). 688/686 is the old 720px card. At the shipped 1280 shell the card content is **942px**, and `864:3474` draws 976 with a 944px chart. | Resolved both frames |
| Step numbering | The report uses two incompatible schemes in one document. D-03/D-04/D-05 follow Figma's on-screen "N of 3" eyebrow (`212:84747` is `1 of 3`, `212:85100` is `2 of 3`), while B-03 lists the app's four array steps including Welcome. C-01 then attributes the config/proxy/key-storage copy to "step 1", where the Welcome step's body (`Onboarding.tsx:63-64`) contains none of it; that copy is at `:85`, on the next step. | `232:4370` draws the Welcome body verbatim as the app renders it |
| C-01 evidence | The mitigating half ("Moved copy, not wholly invented", citing retired frame `212:85283`) **cannot be re-verified**: MCP reports that node id is not in the file. `docs/figma-questions-for-design.md:294-299` describes the same node as a retired fifth step, so the report probably inherited the citation rather than re-resolving it. The claim may still be true; the evidence is gone. | |

Smaller citation slips, string correct in every case: C-01 (83 to 85), C-02 (132 to
135), C-06 (475 to 482), C-08 (1581 to 1599), C-09 (`dialogs.tsx` to
`useSettingsActions.ts:350`), C-10 (1529 to 1546), C-12 (130 to 126), C-13
(285 points at a `className`), S-19 (`Sidebar.tsx:293` is the inventory card, not
the availability filter), M-04 (`useRouting.ts:259` is M-05's line; drift is
262-267), M-08 (`TrayApp.tsx:875` is the render site, not the tray card at `:818`,
and `:386`/`:910` are omitted), and off-by-one-or-two opens on M-01, M-06, M-13,
M-27, B-02, B-04.

## Confirmed

**Structural, S-02 to S-20 excluding model items: 16 of 16 app-side claims
confirmed.** Spot-highlights: `AppPane.tsx:482` is the `gateActive &&` guard and the
credits row at `:527-538` does sit outside it (S-02, S-03); `InstallationPicker.tsx:30`
is `if (installations.length < 2) return null` (S-13); `onChangeGateway` is
unconditional at `NewUiApp.tsx:1818` and the only `import.meta.env.DEV` gate is the
setup picker at `:2299`, so S-15's "dev-only is stale" correction is right;
SettingsPane really does have three Diagnostics rows (`:493`, `:518`, `:530`) where
Figma draws two.

Figma-side confirmations for the structural set: `437:158` (`page=overview` sidebar
symbol) draws **Overview and Settings only**, with no Security nav item and no
master/shell card, which is positive evidence for S-05 and S-10; `130:48905`,
`177:74332`, `143:70315` and `362:8700` all draw a two-row Diagnostics section, no
Help section and no certificate row (S-07, S-12), and one Notifications row
(S-11); `202:80095`'s topbar draws Ellipsis and Minimize2 as `base.primary` icon
buttons, which is exactly S-16's premise (screenshot resolved); `177:79238` is the
canvas `↳ Setup ✅` whose section labels read `Auth / Connect with Gate ✅`,
`Auth / Connect with API key ✅`, `Auth / Organizations ✅` and
**`Auth / Error states`** with no check, which is S-18's claim precisely.

**Copy: C-03, C-06 (as behaviour), C-07, C-08, C-09 (as behaviour), C-10, C-11,
C-12 confirmed**, and all 18 setup and onboarding strings match character for
character under literal `grep -F`. C-11's typo finding is exact on both sides: the
frame draws "The state of this installed" (`363:9033`) and the app says "install"
(`dialogs.tsx:769`).

**Dialog inventory: 23 of the 26 modals in scope, plus B-01, B-02, B-04 and I-01,
confirmed**, including the harder conditional claims. M-15's key claim is right and worth
keeping: the caller only opens the report when `outstanding > 0`
(`NewUiApp.tsx:1128`), while the dialog carries an `outstanding > 0` branch
(`dialogs.tsx:2179`), so "Every tool is back on its own settings" is dead code from
those call sites. M-23 is right that Change gateway ships. M-11's 3000/10000ms poll
intervals check out.

**Overview measurements: 22 of 24 confirmed**, with the app-side values re-derived
independently rather than taken on trust. Two inherited defaults do most of the
work and are worth recording: Tailwind preflight's `html { line-height: 1.5 }` is
never overridden on this pane, and `text-base-2xs`/`text-base-xs` carry no
line-height in their tuples, so the 10px axis tick renders at 15px and the 12px
spans at 18px. That is findings 11, 13 and 15. Finding 21's arithmetic is exact:
`Icon.tsx` sets `stroke = 1.75` on a 24-unit viewBox, drawn into `size={12}`, so
0.875px. Finding 22 lands on 47.84px to the byte. Finding 19 is the one item source
cannot settle: inline-block versus inline-flex is the right cause and the direction
is right, but the magnitude depends on how the browser synthesizes a flex baseline,
and an OFF pill with no icon aligns exactly.

The report's cumulative drift table is internally coherent: every step is one 16px
`gap-4` (0+86, 102+258, 376+300, 692+250 = 942), and 942 - 904 = 38 = 2 + 26 + 6 + 4,
the sum of the per-card deltas. Its Figma column matched `121:34782` exactly
(84/232/294/246 at y0/100/348/658, total 904).

The accessibility note is correct as stated: `#d97706` is 3.19:1 and `#16a34a` is
3.30:1 against white, both under 4.5:1, and both match Figma.

## What the report adds that this repo did not already have

- **Finding 10 is its best contribution.** The bars draw `red/500` `#ef4444` in
  both frame generations (`706:10516`, `864:3513`), while the legend swatch in
  the same frame is `red/400` `#f87171` (`706:10605`, `864:3602`), as is the
  tooltip component (`744:37718`). The app renders one token, `chart.blocked`
  `#f87171`, for bars, legend and tooltip alike, so its legend is exact and its
  bars are one step light. `tailwind.config.ts` records moving this from red/500
  to red/400 on the strength of the swatch and the tooltip; neither pass had read
  the bar rectangles.
  Checked further before acting: **only Blocked splits.** Bar and swatch agree on
  `blue/400`, `amber/400` and `violet/500` for the other three series
  (`864:3597`), so this is one node's slip rather than a rule that bars run a
  step hot, and picking a side by eye is exactly what this repo does not do with
  a series colour. Raised as question 19 instead of patched.
- Finding 09, the stack order: the app draws blue, red, amber, violet bottom to
  top (`SERIES` plus `flex-col-reverse`) where the frames draw blue, violet,
  amber, red (`706:10515` renders red/500, amber/400, violet/500, blue/400 top
  down). Fixed.
- The icon and badge arithmetic (findings 20, 21, 22) and the row-step and
  badge-gap pair (17, 18).
- D-13, the scrim tint: `bg-neutral-900/40` is `#171717` at 40% because `neutral`
  is not redefined in this repo, where `143:70543` resolves
  `tailwind colors/base/black`.
- D-08, D-09 and D-10, the onboarding control recipes: the locate button is
  `Variant=Outline, Size=sm` (`267:5083`, 32 tall, 8/12 padding, 6px gap, radius
  4, 12px label) where the app was drawing the footer's `Size=default` recipe;
  the gap above it is 16px, not 24; the `Checkbox` component's box is radius 4,
  not 2. All three fixed. D-15, the fourth of that group, is the report sampling a
  hidden node - see above.
- Finding 01, the 16px versus 24px header gap.
- Finding 24, which turned out to be a token error rather than a call-site one:
  `Size=sm` draws `inset 0 4px **4px** rgba(255,255,255,0.4)` (`267:5083` and
  `121:35058`, both named `Size=sm`), where `base-btn-sm` held the `xs`
  treatment's 6px blur. Every `h-8` button in the app was wearing the `xs`
  highlight. Fixed by splitting the token.
- The 3.19:1 and 3.30:1 status-label contrast measurement.

## What it duplicates

`docs/figma-questions-for-design.md` already carries, with node-level evidence:

| Report | Repo question |
|---|---|
| S-07, and the "third Diagnostics row" note | 18, "The Diagnostics section now has three rows, and none of the third is drawn" |
| S-11 | 8, "Notifications: one row or three?" |
| C-01, C-02 | 10, "Two onboarding paragraphs the file does not draw" |
| C-12 | 14, "'Routing' or 'Routed' in the status banner?" |
| C-13 | 16, "Does the chart tooltip head with '12' or '12:00'?" |
| D-16 | 5, "Dialog buttons: 4px or 8px?" |
| D-17 | 17, "Is the Redacted chart series violet or purple?" |
| Accepted note, onboarding width | 12, ANSWERED, 1080 is intended |
| Accepted note, dot pattern | 15, ANSWERED, the pattern is real |
| Accepted note, More-menu Quit | 4, ANSWERED, the menu has both |
| Accepted note, button sizes | 6, ANSWERED, at least four sizes |

Question 17 is also sharper than D-17: it records that "component beats instance"
now points at `purple/500`, and that the repo deliberately has not applied it.

## Fixes applied

Every finding below was confirmed on both sides before it was touched. Tokens
were resolved to literals, and each Figma value was re-read off the node named in
the code comment rather than taken from the report.

| Ref | Change | File |
|---|---|---|
| 01 | Header to first content item 16px to 24px, via `mb-2` on the header on top of the pane's `gap-4`. The 24 is one gap, not the pane's rhythm, so it cannot come from `gap-4`. | `Overview.tsx` |
| 07 | Chart plot `h-28` (112px) to `h-[5.5rem]` (88px). In rem so `useTextScale` still carries it. Also closes 04, whose 26px was this plus padding. | `metrics.tsx` |
| 09 | New `STACK` constant orders the bar bottom to top as total, redacted, flagged, blocked. `SERIES` keeps the legend's and tooltip's order, which is the frame's own. | `metrics.tsx` |
| 11 | Axis tick gets `leading-4`. `text-base-2xs` carries no line-height, so it was inheriting preflight's 1.5 and boxing at 15px against the drawn 16. Family stays sans, see below. | `metrics.tsx` |
| 13 | Reporting period `text-base-xs` to `text-sm` (14/20), which is `copy/14` and a 20px-tall text node in both frames. | `Overview.tsx` |
| 14 | Stat-tile eyebrow ink `base-muted-foreground` to `text-gray-600` `#4b5563`, the raw ramp the tile's own variable names. | `metrics.tsx` |
| 15 | Both tables' headers to `font-medium leading-4`, which is `label/12`. The `font-normal` they carried was undoing preflight's bold and landing on 400. | `Overview.tsx` |
| 16 | Both tables' row labels to `font-medium tracking-heading-14`, which is `heading/14` at 0% - a named style with no tracking, so it needs a token rather than `text-sm`'s own -0.14px. | `Overview.tsx` |
| 17 | Closed by 19: with the action pill no longer inline-block, the row measures 49px against the drawn 48, which is the border-outside-the-box gap CLAUDE.md says not to contort markup for. | `Overview.tsx` |
| 18 | Policies status column `w-24` to `w-[110px]`: the frame right-aligns a 50px badge to the card's inner edge and puts the action badge 60px before it, on all three rows. | `Overview.tsx` |
| 19 | Action pill `inline-block` to `inline-flex items-center`, matching `StatusPill`, so the two pills share a baseline. | `Overview.tsx` |
| 20, 22 | ON glyph 12px to 14px. Both current policies cards draw 14, and 14 is what makes the badge measure the drawn 50: 8 + 14 + 4 + 15.84 + 8. | `Overview.tsx` |
| 24 | Split the button shadow: `base-btn-sm` now holds the measured `Size=sm` treatment (`inset 0 4px 4px rgba(255,255,255,0.4)`) and a new `base-btn-xs` holds the 6px-blur `xs` one. `banner/update`'s h-6 dismiss is the only `xs` call site and now names it. | `tailwind.config.ts`, `banners.tsx` |
| D-08 | `IntroButton` takes a `size`; the locate control uses `sm` (h-8, radius 4, 12px label, 6px gap) instead of the footer pair's `default`. | `Onboarding.tsx` |
| D-09 | Locate control's `mt-6` to `mt-4`. | `Onboarding.tsx` |
| D-10 | Intro checkbox `rounded-xs` (2px) to `rounded-control` (4px). | `Onboarding.tsx` |
| D-12 | Dialog title gets a new `tracking-heading-18` (-0.18px). It had been borrowing `tracking-heading`, which is `heading/20`'s -1% at a size no dialog draws. | `tailwind.config.ts`, `Modal.tsx` |
| D-13 | Scrim `bg-neutral-900/40` to `bg-black/40`. | `Modal.tsx` |

New tokens: `letterSpacing.heading-18`, `letterSpacing.heading-14`,
`boxShadow.base-btn-xs`. `tsc --noEmit` clean, 964 tests pass, and the five files
Prettier flags were already flagged before these edits.

Two of this repo's own records were corrected as part of the work: the stale
"locked the main window at 1024x720" line in question 12, which is plausibly what
sent the audit to a 720-tall viewport, and two new questions for design, 19 (the
Blocked red) and 20 (the dialog body gap).

**`CLAUDE.md` now has one stale claim as a result**, left for a person to change:
its Button section says `shadow-base-btn-sm` "expands to exactly the three shadows
above - so the token the config comment labels 'the sm size' is in fact the `xs`
treatment. `sm` and `xs` evidently share it." They do not share it. The `xs`
instance (`744:37756`) blurs the white inset 6px and the `sm` instances
(`267:5083`, `121:35058`) blur it 4px, which is why the token is now split.

## Not fixed, and why

- **02, 03, 23.** The CSS-border-outside-the-padding-box effect, which CLAUDE.md
  already says not to contort markup for. 02's own numbers are also retired: 688
  and 686 belong to the 720px card, where the shipped shell draws 976 and 942.
- **08, 12.** The app already follows the newer 24-bucket design. The report's
  48px bar and `HH:00` labels are the retired 12-bucket frame, which its own notes
  say.
- **10.** Only Blocked splits bar from swatch, so it is one node's slip. Question
  19.
- **11, the mono half.** The frames draw the axis tick in Geist Mono, and design
  settled on 2026-09-04 that mono marks an eyebrow or a pill label and nothing
  else. A dense row of 24 ticks is neither. Only the line-height was applied.
- **21.** The app-side arithmetic is right (1.75 on a 24-unit viewBox at 12px is
  0.875px), but the Figma side is not checkable over this integration: the glyph
  comes back as `IMAGE-SVG` with no stroke weight. The size fix takes it to
  1.02px. Whether it should be 1.5px needs either a design answer or a
  per-call `strokeWidth`, which is new API for a hairline.
- **D-01 to D-07.** Consequences of copy and height decisions, and their vertical
  offsets were measured in a 720-tall window the app cannot be.
- **D-02, D-15.** Wrong, as above.
- **D-11.** Compositing; the Figma effect stack was not re-resolved.
- **D-14.** The shared 24px gap is what the diagnostics (`363:9028`) and
  replace-key (`177:74562`) dialogs draw. Only the disconnect dialog draws 16, and
  it is also the only one of the three with a 32px header tile, so this is either
  a rule or a drag. Question 20.
- **D-16, D-17.** Recorded design conflicts. Repo policy is not to pick a winner,
  and both are already questions 5 and 17.
- **S-01 to S-20, C-01 to C-14.** Either the app is already right (C-03, C-08,
  C-10, C-11, C-12 and the whole setup string list), or the report's own
  recommendation is to agree something with design first: adding or retiring a
  product surface (S-05 to S-09, S-14, S-15), a visibility rule that changes what
  the pane claims about billing (S-02, S-03), a grouping the backend gates
  separately (S-11), or copy that is a decision rather than a defect (C-01, C-02).
  Two smaller ones worth naming as unrequested-but-known: the enabled Previous
  button draws radius 4 and a `base/border` line where the app gives it `rounded-md`
  and `base/input` (`232:4359`), and the reporting period's ink is `gray/600` in the
  frame where the app leaves it `base-muted-foreground`.

## Claims I could not settle

- Every "no matching approved reference identified" negative (S-06, S-08, S-09,
  S-14, S-15, M-03, M-04 to M-07, M-14 to M-17, M-23, M-26, M-27, B-01). MCP lists
  exactly one top-level page for this file ("Design docs", `319:4686`), so I cannot
  enumerate the file to prove an absence. The report's own hedge ("'no reference
  found' does not prove a feature was unauthorized") is the right posture.
- Anything native: the Windows alpha.4 captures, the 30 rendered comparison pairs,
  and the 16-scenario isolated run.
- D-11, the progress-bar compositing: confirmed that the app uses layered
  `linear-gradient` with no `mix-blend-mode` anywhere in `src/` and no track inset,
  but the Figma effect stack was not re-resolved.
- Finding 19's exact magnitude, as above.

## Code findings that came out of the validation

1. **`dialogs.tsx:2199` renders the literal word "cube".** `ModalSubject`'s `icon`
   is `ReactNode` and is rendered raw (`Modal.tsx:354`, `:378`), and this call site
   passes the string `"cube"` instead of `<Icon name="cube" size={16} />`. So every
   per-tool row in the teardown report shows "cube" in its 40px tile. Present at
   both revisions. The report flagged this as "a separate implementation bug" and it
   is real. Note the neighbouring string icons at `:626` and `:2178` are correct:
   those are `Modal`'s own `icon`, typed `IconName` and rendered through `<Icon>`.
2. **The chart is 2px short of its own arithmetic.** `CLAUDE.md` derives the 1280
   width from 24 bars at 32px on an 8px gap, which needs 944, and `864:3474` draws
   exactly that. The app's chart row content box is 942px, and with `flex-shrink: 1`
   the bars silently compress rather than overflow. The 6px webkit scrollbar in
   `index.css` takes a further 6px off every card on a pane that always scrolls.
3. **M-01's info note names a surface that does not exist off macOS.** "You can
   safely minimize this app to the menu bar" is hardcoded at `dialogs.tsx:2320`,
   while `Onboarding.tsx:18-27` already has `whereItLives(platform)` returning
   "system tray at the bottom right" on Windows and "top bar at the top right" on
   Linux.
4. **The two Quit entrances diverge on Linux.** `request_quit` calls `app.exit(0)`
   before computing any tool list on non-macOS/Windows (`lib.rs:3778-3779`, with a
   comment recording that removing the gate was already a regression once), so tray
   Quit never raises the chooser there, while More > Quit is pure TypeScript and
   does.
5. **M-01's radio focus is incidental.** `QuitDialog` passes no `initialFocus` and
   neither button is `destructive`, so `useFocusTrap` (`Modal.tsx:199-206`) falls to
   `focusables[0]`, which is the first radio only because the dialog renders no
   close button. Adding `closeButton` would silently move focus off it.
6. **`whereItLives` has no `case "macos"`.** macOS shares the `default:` arm with
   `"unknown"`, which is what `usePlatform` returns on the first async tick
   (`platform.ts:29-30`), unlike `secretStoreName`, which does have a macOS case.
7. One stale line in this repo's own docs, and plausibly the origin of the report's
   720-tall viewport: the tail of `docs/figma-questions-for-design.md`'s answered
   question 12 still says "we have locked the main window at 1024x720".

## Working notes

Per-lens detail, with the quoted code for every verdict, is in the session
scratchpad: `validate-structural.md`, `validate-copy.md`, `validate-modals.md`,
`validate-visual.md`, `validate-overview.md`.
