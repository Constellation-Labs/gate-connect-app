# Technical audit: the family panel and Home's ledger

**Target:** `src/screens/FamilyPanel.tsx`, `src/screens/Home.tsx`,
`src/screens/GroupMembers.tsx`
**Branch:** `design/per-family-panel` (staged, uncommitted) on `e9efb5d`
**Date:** 2026-08-10
**Conformance target on record:** WCAG 2.1 AA (DESIGN.md), light theme only,
one 380x620 room, text to 200% via the app's own control
**Method:** every number below was measured in the real render, not derived from
class names. Headless Chrome at the true 380x620 content box, Tauri v2 bridge
stubbed pre-boot via CDP `Page.addScriptToEvaluateOnNewDocument` (nothing
written into the working tree). 11 rendered views: Home and the family panel
across four data states (`mixed`, `healthy`, `untrusted`, `off`), plus both at
200% text and Home under `prefers-reduced-motion: reduce`. Composited contrast
over resolved alpha stacks for every text node, non-text contrast on every
switch track, `Accessibility.getFullAXTree` for names and descriptions, real
`Input.dispatchKeyEvent` Tab traversal, forced-focus outline reads, and
scroll/clip geometry per view. Plus the bundled detector and a production build.

---

## Audit Health Score

| # | Dimension | Score | Key Finding |
|---|-----------|-------|-------------|
| 1 | Accessibility | 3 | Two SC 1.4.4 clipping failures at 200%; everything else measured clean, including all contrast and every switch's reality description |
| 2 | Performance | 3 | Nothing user-visible in scope; no memoization anywhere in the three files, but no measurable cost either |
| 3 | Theming | 4 | Fully tokenized; the only literal in 1,400 lines sits inside an explanatory comment |
| 4 | Responsive Design | 2 | At 200% the member host identifier renders 51px of the 214px it needs, and one member name loses its third line |
| 5 | Implementation Integrity | 4 | Detector: 0 findings on all four changed UI files |
| **Total** | | **16/20** | **Good (address weak dimensions)** |

Not comparable to the 16/20 of 2026-08-04: that audit scoped `src/App.tsx` and
the 17 modules it composes, so its Theming score carried literals from files
outside this scope and its Responsive score predates both the rem type ramp and
this branch. Treat the dimension scores, not the total, as the signal.

---

## Implementation Integrity Verdict

**Pass.** The detector returns zero findings across `App.tsx`,
`FamilyPanel.tsx`, `GroupMembers.tsx` and `Home.tsx`. Nothing here is
interchangeable with an unrelated product: the panel's central abstraction is
the `desired` / `routed` split that DESIGN.md calls the product's signature
risk, and the measured AX tree shows it wired through to assistive tech at both
levels rather than asserted in a comment. Every switch on the panel carries an
`aria-describedby` resolving to its own reality label (family switch →
`Partly routed`; members → `Error`, `Not routed`, `Routed`), which is the one
thing a routing product must not get wrong and the one thing a generic settings
screen would not have.

One documentation inaccuracy, recorded as P3 below: comments in two files assert
that `pointer-events-none` removes content from the accessibility tree. The
measured tree disproves it.

---

## Executive Summary

- Audit Health Score: **16/20** (Good, address weak dimensions)
- Issues: **0 P0 · 2 P1 · 1 P2 · 3 P3**
- Top issues:
  1. **[P1]** At 200% text, the member identifier line clips the host to 51px of
     214px needed, three times per panel, and `line-clamp-2` cuts
     `Claude Desktop / Cowork` from 107px of text to 72px. SC 1.4.4.
  2. **[P1]** The footer credential promise truncates at 200% on every screen,
     which is the sentence carrying PRODUCT.md's core reassurance.
  3. **[P2]** The global `0.01ms` reduced-motion kill erases the popover's
     directional grammar, so a push and a pop become indistinguishable, which
     is precisely what DESIGN.md says direction exists to prevent.
- Recommended next: `/impeccable adapt` on the two P1s, then `/impeccable animate`
  for the reduced-motion alternative, then `/impeccable polish`.

---

## Detailed Findings by Severity

### [P1] The member identifier line loses its content at 200% text

**Location:** `src/screens/GroupMembers.tsx:348` (chip / host / Details row) and
`:307` (name / pill / switch row)
**Category:** Responsive / Accessibility
**WCAG:** 1.4.4 Resize Text (AA) — loss of content at 200%

Measured on `panel:200`, six clipped nodes:

| Node | Needs | Gets | Mechanism |
|---|---|---|---|
| `api.anthropic.com` (×3) | 214px | 51px, 51px, 127px | `truncate` |
| `Claude Desktop / Cowork` | 107px tall | 72px | `line-clamp-2` |
| `Hermes` | 94px | 80px | `line-clamp-2` |

**Impact:** at 200% the user cannot read which host a tool routes to, on the one
screen whose entire job is reporting per-tool routing truth. The host is the
identifier DESIGN.md says "cannot be shortened without lying". A member name
also loses its tail, and both the config-vs-proxy chip and the word "Details"
survive at full size beside the starved text, so the row spends its width on
its least informative parts.

**Root cause, and why the fix is known:** these two rows are the only ones in
the ledger surface without `flex-wrap` and an `em` basis. Home's `FamilyRow`
(`basis-[6em]`), the routing card (`basis-[8em]`) and this branch's own panel
control row (`basis-[8em]`) all use that rule and all measured **zero** clipped
nodes at 200%. The pattern is established, documented in DESIGN.md, and simply
not applied here.

**Pre-existing, not introduced.** The prior nested layout carried `pl-6` where
this now carries `px-3.5`, so the member row had ~10px *less* width before. This
branch improved the number without crossing the threshold, and this is the first
pass to measure it.

**Recommendation:** apply the same `flex-wrap` + `em`-basis rule so the pill and
switch drop to their own line at large text and the name and host keep the full
width. Consider dropping the visible `Details` word at large text, since the
row's accessible name already carries it.
**Suggested command:** `/impeccable adapt`

---

### [P1] The credential promise truncates at 200% on every screen

**Location:** the shared popover footer (rendered from `src/App.tsx`, outside the
three named files but present on both audited surfaces)
**Category:** Responsive / Accessibility
**WCAG:** 1.4.4 Resize Text (AA)

Measured on both `home:200` and `panel:200`: `Session in your system's secure
store` needs 360px, gets 301px, under a `truncate` class.

**Impact:** PRODUCT.md's first design principle is "credentials are the
product", and this line is where the popover states where the key lives. At the
text size a low-vision user needs, the reassurance is the thing that gets cut.
The prior audit recorded the same class of failure on this footer under Windows
platform text sizing; the rem ramp changed the mechanism but not the outcome.

**Recommendation:** allow the footer to wrap to two lines at large text rather
than truncating, or shorten to the store name alone once the label no longer
fits.
**Suggested command:** `/impeccable adapt`

---

### [P2] The global reduced-motion kill erases the panel's directional grammar

**Location:** `src/index.css:69-77`
**Category:** Accessibility / Performance
**WCAG:** related to 2.3.3 Animation from Interactions (AAA), but the defect
here is loss of orientation, not a conformance failure

The block sets `animation-duration: 0.01ms !important` and
`transition-duration: 0.01ms !important` on `*, *::before, *::after`. Measured:
**137 elements** at `1e-05s` under `prefers-reduced-motion: reduce`, against
**10** animated in the default state.

**Impact:** `SCREEN_DEPTH` in `App.tsx` exists so pushes and pops differ, and
DESIGN.md is explicit that in one 380px room "direction is the only navigational
metaphor available: without it a push and a pop look identical and the user
never builds a sense of where Settings is relative to Home." Under reduced
motion that metaphor is not softened, it is deleted: opening a family panel and
returning to Home become visually identical events. A user who set the OS
preference to avoid vestibular discomfort is the same user who then has no
orientation cue at all. DESIGN.md currently records the blanket collapse as "the
contract", so this is a design-system decision to revisit, not a stray line.

**Recommendation:** keep translation suppressed but preserve the state change.
A sub-100ms opacity cross-fade, or retaining the directional offset with the
duration floored rather than zeroed, both honour the preference while keeping
push and pop distinguishable. Update DESIGN.md's motion contract in the same
pass.
**Suggested command:** `/impeccable animate`

---

### [P3] The member disclosure button carries no reality description

**Location:** `src/screens/GroupMembers.tsx:291-297`
**Category:** Accessibility

Measured AX tree: `button 'Claude Code details'` has no `description`, while the
sibling `switch 'Route Claude Code through Gate'` has `desc='Error'` and Home's
equivalent `FamilyRow` button does carry an `aria-describedby`.

**Impact:** low. The pill text is present in the tree as adjacent `StaticText`
(`Error`, `Not routed`, `Routed`) and the switch beside it carries the state, so
nothing is unreachable. It is an inconsistency between two rows that are
otherwise deliberately built the same way, and Home's row has the description
precisely because a row can be the only carrier of state.

**Recommendation:** point the member row button at the same `member-state-*`
span the switch already uses. One attribute.
**Suggested command:** `/impeccable harden`

---

### [P3] Comments assert an accessibility mechanism the tree disproves

**Location:** `src/screens/GroupMembers.tsx:298-306`, `src/screens/Home.tsx:729-734`,
and the same claim in `.impeccable/critique/` round 16
**Category:** Implementation Integrity (documentation)

Both files carry comments stating that putting the pill, count and exception in
`pointer-events-none` spans "also takes them out of the accessibility tree" /
"hid them from the accessibility tree". The measured tree contains every one of
them as `StaticText`: `Partly routed`, `1 of 3 routing`, `config file`,
`api.anthropic.com`, `Details`, and each member pill label.

**Impact:** none at runtime. The `aria-describedby` wiring the comments justify
is genuinely valuable, for the real reason that it binds reality to the control
that reports intent. But a false mechanism in a comment invites future work to
"fix" accessibility that was never broken, or to trust `pointer-events-none` as
a hiding primitive elsewhere, where it would silently not hide anything.

**Recommendation:** correct the stated reason to the accurate one: the stretch
button is a sibling with no text content, so its *accessible name* excludes the
row's text and needs `aria-label` plus a description; the text itself was always
exposed.
**Suggested command:** `/impeccable harden`

---

### [P3] Home scrolls 999px at 200%

**Location:** `src/screens/Home.tsx`
**Category:** Responsive

Measured `home:200`: `scrollHeight` 1555 against `clientHeight` 556. The panel
is lighter at 492px of scroll.

**Impact:** none; this is the documented contract ("content scrolls within the
body, header and footer never") and horizontal overflow is zero in every view at
every size. Recorded so the number exists for comparison, not as a defect.

---

## Patterns & Systemic Issues

**One layout rule, applied everywhere but the deepest row.** Both P1s and the
200% clipping are the same omission: the `flex-wrap` + `em`-basis pattern that
DESIGN.md establishes for a fixed window with no width breakpoints is applied to
the routing card, Home's family rows and this branch's panel control row, all of
which measure zero clipped nodes at 200%, and is missing from the member rows and
the footer, which are exactly the two places that clip. This is a coverage gap in
a known-good rule, not a design problem, which is why it is cheap to close.

**Text scaling is the only failing axis.** Across 11 views, every contrast
measurement passes, no view overflows horizontally, no heading level is skipped,
every control has a focus ring, and Escape restores focus correctly in all four
data states. Every defect found sits at 200% text. The project's own decision to
make text scaling its accessibility mechanism (a fixed non-resizable window
exposes no other route) means this axis deserves the same measurement discipline
the contrast ladder already gets.

---

## Positive Findings

- **Contrast: zero failures in 11 views.** Every text node passes its floor,
  measured over composited alpha stacks rather than nominal token values,
  including at 200% and under reduced motion.
- **Non-text contrast holds at both levels.** Switch tracks measure 3.36:1 off
  and 5.98:1 on, against the 3:1 SC 1.4.11 floor, on white and on the subtle
  fill an open row uses. The `#868c9e` off-track decision recorded in DESIGN.md
  is doing exactly what it was introduced to do.
- **Intent versus Reality is wired, not claimed.** Every switch in the panel
  resolves an `aria-describedby` to its own reality label. This is the product's
  signature risk and it is correct at family and member level, which round 16
  found unfixed at the member level.
- **Heading outline is unbroken and meaningful.** Home reads
  h1 → h2 Routing → h2 What routes through Gate → four h3 families; the panel
  reads h1 family → h2 per member. No skipped levels at any text size. The h2
  promotion this branch made to member names is what keeps the panel navigable.
- **Focus rings on all 16 controls** across both surfaces: 2px solid
  `rgb(62, 79, 234)`, verified by forced focus rather than by class presence.
- **No keyboard trap.** Tab cycles cleanly through 8 stops on Home and 7 on the
  panel; Escape returns to Home and restores focus to Home's h1 in all four data
  states. Back is one Shift+Tab from the panel's entry focus.
- **Tab-stop count fell from 14 to 7** on the panel, and overflow from 150px to
  0, because one family is on screen instead of four.
- **Fully tokenized.** The single colour literal across 1,400 lines of the three
  files sits inside a comment explaining a contrast decision.
- **No performance anti-patterns.** No `will-change`, no `transition-all`, no
  layout-property animation, no filter or blur effects.

---

## Recommended Actions

1. **[P1] `/impeccable adapt src/screens/GroupMembers.tsx`**: apply the
   `flex-wrap` + `em`-basis rule to the member name and identifier rows so the
   host stops rendering 51px of the 214px it needs at 200%, and
   `Claude Desktop / Cowork` stops losing its third line.
2. **[P1] `/impeccable adapt src/App.tsx`**: let the footer credential promise
   wrap instead of truncating at 200%, so the sentence that states where the key
   lives survives the text size a low-vision user needs.
3. **[P2] `/impeccable animate src/index.css`**: replace the blanket `0.01ms`
   collapse with a reduced-motion alternative that keeps push and pop
   distinguishable, and update DESIGN.md's motion contract to match.
4. **[P3] `/impeccable harden src/screens/GroupMembers.tsx`**: point the member
   disclosure button at the `member-state-*` span its own switch already uses,
   and correct the two comments that claim `pointer-events-none` hides content
   from the accessibility tree.
5. **[P3] `/impeccable polish src/screens/FamilyPanel.tsx src/screens/Home.tsx src/screens/GroupMembers.tsx`**:
   final pass once the above land.

---

## Revisions — 2026-08-10, all findings addressed

Every finding above was fixed in the same session and the identical measurement
pass was re-run against the same 11 views. Numbers below are re-measured, not
asserted.

| Finding | Before | After |
|---|---|---|
| [P1] Member row clipping at 200% | 6 clipped nodes on `panel:200` | **0** |
| [P1] Footer promise truncated at 200% | 1 clipped node, 301px of 360px | **0**, wraps to two lines |
| [P2] Reduced motion | 137 elements at `1e-05s` | **10**, and travel removed by property rather than by duration |
| [P3] Member row description | `button "Claude Code details"` with no description | points at `member-state-*`, same span its switch uses |
| [P3] Inaccurate a11y comments | 2 files asserting `pointer-events-none` hides content | corrected in both, with the real mechanism named |
| Contrast | 0 failures | **0 failures**, no regression |
| Horizontal overflow | 0 in every view | **0** |
| Tests / typecheck / build | green | **223 passing, `tsc` clean, build clean** |

**What changed, per finding.**

1. **Member rows** (`GroupMembers.tsx`): both rows took the `flex-wrap` plus
   `em`-basis rule the rest of the surface already used. The name column asks
   for `basis-[7em]` and the pill and switch became one `ml-auto` group so they
   wrap together rather than splitting the capsule from the control that
   changes it. The host identifier asks for `basis-[8em]`, so "Details" drops to
   a second line before the host gives up a character.
2. **Footer** (`App.tsx`): `truncate` removed from the credential sentence. It
   wraps to two lines at 200% and is unchanged at 100%, where it has always fit.
3. **Reduced motion** (`src/index.css`): the blanket duration collapse is gone.
   The guard now whitelists `transition-property` to the non-moving properties
   (colour, background, border, text-decoration, fill, stroke, opacity, shadow),
   so any `transform` transition is instant while feedback still eases; and the
   three travelling animations resolve to a new `gc-fade-in` at 120ms linear.
   Verified by computed style: under `reduce` the Switch thumb's
   `transition-property` excludes `transform` while its duration stays 0.15s,
   and a panel navigation computes `animation-name: gc-fade-in` at `0.12s`
   linear while the same element under `no-preference` computes
   `ob-slide-in-fwd` at `0.26s`. `animation-iteration-count` is no longer
   clamped, so the org picker's loading spinner spins again.
   DESIGN.md's Motion paragraph and its reduced-motion Do were rewritten to
   match; the contradictory legacy bullet was removed.
4. **Member row description and comments** (`GroupMembers.tsx`, `Home.tsx`):
   one attribute added, and both comments now state the accurate mechanism (an
   empty covering sibling has no accessible name of its own; the text was never
   hidden).

**Two numbers rose, both expected and both benign.** Scroll height at 200% went
from 999px to 1031px on Home and 492px to 742px on the panel, because wrapped
text is taller than truncated text. Horizontal overflow stayed at zero and the
body is the popover's designed scroll region, so this is the contract working
rather than a regression. CSS bundle grew 0.28 kB.

**Revised dimension scores:** Accessibility 3 → **4**, Responsive 2 → **4**.
Theming and Implementation Integrity unchanged at 4. Performance unchanged at 3
(untouched; no user-visible cost was found to fix). **Total 16/20 → 19/20,
Excellent.**

**One detector finding remains, adjudicated as a false positive:**
`design-system-color` `#000` at `src/index.css:210`. This is the pre-existing
`.gc-scroll-more` fold-cue `mask-image`, where the colour channel is never
painted and only the alpha stop matters. It is already recorded in
`.impeccable/critique/ignore.md`; my edit moved it from line 159 to 210 by
adding lines above it. Left unchanged, and not suppressed at the hook level
without an explicit call from the user. To quiet the hook for it:
`/impeccable hooks ignore-value design-system-color "#000" --shared --reason "mask-image alpha stop, never painted"`.
