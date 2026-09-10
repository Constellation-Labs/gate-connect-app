# Figma fidelity review, all flows - 2026-08-26

Read on a **Full seat** (`whoami`: `Constellation Network`, tier `org`, seat
`Full`), so this is the first pass not rationed by the 6-per-window View-seat
cap. Values below are node data from the REST API unless marked otherwise.

Scope: `Components / Sidenav`, `Flows / Onboarding`, `Flows / Setup`,
`Flows / Overview`, `Flows / App`, `Flows / Settings`.

> `get_metadata` with no `nodeId` still lists only `Components` and `Sidenav`,
> on a Full seat as on a View one. That blind spot was never about access; the
> Flows pages are reachable by node id and nothing else.

---

## Summary

| Flow | State | Action |
| --- | --- | --- |
| Settings | **Current.** Re-read and applied (`ee93fd37`). | none |
| Onboarding | **Current.** Re-read and applied (`bf94c807`, `754a512d`). | one open copy question |
| Overview | Sections unchanged since 2026-08-21; not re-read at value level. | spot-check |
| **Setup** | Was redrawn. **Applied.** | verify remaining panes |
| **App** | New section. **Sort applied; multi-select deferred.** | scope, then build |
| **Sidenav** | Six values moved. **Applied.** | none |

See "What was applied" at the foot of this file.

---

## Setup - redrawn, not tweaked

Every pane moved to a new shape. Read: sign-in (`202:80095`), org picker
(`451:7795`), name-device (`451:7906`). The last two carry ids in the 451
range, above the section rules - they are new frames, not edits of the old
ones.

**The card.** 496px wide, radius **16**, `shadow/lg`, 1px `base/border`. The
build has `max-w-[520px]`, `rounded-md` (8) and `shadow-base-sm`.

**One header archetype - see the correction below.** A 48px tile at radius 8
over a centred title and subtitle, with stacked full-width buttons under it.
`SetupHeader` drew a bare 40px hex mark with no tile at all.

**The sign-in card keeps stacked full-width buttons**, but at **44px tall and
radius 8**, primary carrying a 20px ArrowRight, and the `or` divider at a 19px
gap. Built: `h-9`, `rounded-sm`, no arrow, `gap-3`.

**Sign-in header specifically.** A 48x48 tile at radius 8 on a `#97C3FF`
hairline, mark at 24px, then the wordmark at **24px** (`Gate` `#203DE2`
SemiBold / `Connect` `#6B7280` SemiBold). Built: 20px, `blue-ribbon-800`
(`#1d37b6`) and default ink.

**Name-device field.** Label in `label/12`, input 44px at radius 8 on
`base/background` with a `base/primary` focus border and a trailing `CircleX`
clear button.

**Org rows.** Selected = 1px `base/primary` + `shadow/sm` + a 16px
`CircleCheck`. Unselected = a 16px radio circle (`base/background` fill,
`base/input` ring, `shadow/xs`).

**The progress rail is the onboarding rail** - 8px, track `base/background`
with a bottom hairline, fill `#7195FF` under a left-to-right black-to-white
64% wash. `SetupLayout` drew `h-1` on `gray-100` with two blue-ribbon stops -
**an inconsistency this session introduced**, the onboarding rail having been
corrected a few hours earlier and setup's not, so one element rendered two ways
one screen apart. Fixed.

**Two standing items, unchanged.** The topbar still draws **Ellipsis and
Minimize2** as 32px icon buttons - open question 5 (Minimize2 removed as an OS
duplicate) is contradicted for the fifth read running. And `Auth / Error
states` still carries **no check**, fifth read, so the setup-timeout dialog and
the device-name validation stay unbuilt.

---

## Correction: the bordered-footer redraw was read off unready frames

**Found by validating all 25 Setup frames**, after the first pass had read five.

The org picker and name-device panes were rebuilt with a left-aligned header and
a bordered action footer carrying `Back` and `Use a different account`. That was
read off `451:7795` and `451:7906` - the two highest-numbered frames on the page.
They do draw that treatment.

**But they sit above the page's first section rule.** The section labels and
their rules run:

| Section | Label y | Rule y |
| --- | --- | --- |
| `Auth / Connect with Gate ✅` | -219 | **-163** |
| `Auth / Connect with API key ✅` | 786 | 841 |
| `Auth / Organizations ✅` | 1813 | 1869 |
| `Auth / Error states` (no check) | 2895 | 2951 |

`451:7795` is at y=-1063 and `451:7906` at y=-1125. Both are **above -163**, so
neither falls under any section heading - and the file's convention is that the
check on a section heading is what marks a flow ready. Two frames floating above
every heading are the shape of work in progress, not of a spec.

Every frame *inside* a checked section draws the older, simpler treatment, and
all nine org-picker states agree with each other: centred header, stacked
full-width buttons, `Use a different account` as a link under the primary, and
**no `Back` button anywhere**. `231:2102` (empty), `229:90709` (one org),
`229:90782` (two), `229:90855` (three), `229:90928` (scrollable), `231:2271`
(timeout) - all the same shape. The name-device and API-key frames likewise.

**So the build before the redraw was right, and the redraw was wrong.** Both
panes are back to centred-and-stacked; `SetupFooter`, `FooterButton`, the
header's `row` mode and the `arrowLeft` glyph are gone with it.

**What survives, because the in-section frames do confirm it:** the 496px card
at radius 16 under `shadow/lg`, the 48px header tile, 44px buttons at radius 8
with the drawn right arrow, the `CircleX`-clearable 44px field, the 8px progress
rail, the org-row treatment, the 19px `or` divider, and the diagnostics copy.

**The lesson for next time:** on this file, position relative to the section
rules is part of the spec. A high node id means recent, not ready.

---

## App - a new section, and it is a feature

`App / Select multiple models (Opencode) ✅` is new since 2026-08-21, with
three frames (`App/OpenCode/gate-models-1..3`) and three rules written on the
canvas:

1. *"When the user selects a model it automatically closes the modal and
   applies their selection."* - what `ModelPickerDialog` already does.
2. *"When the user selects a model (multiple) the modal stays open until the
   user confirms their selections. Then the modal will close."* - **not built.
   OpenCode gets a multi-select picker with an explicit confirm.**
3. *"Current models will sort alphabetically, left to right using their
   provider. Example. Anthropic > DeepSeek > Moonshot."* - **not built.** The
   list is currently in gateway order.

This is a behaviour change with a per-tool branch, not a restyle. It needs
scoping before it needs code: what confirms, what happens to a partial
selection on dismiss, and whether "multiple" is OpenCode-only or
capability-driven.

Everything else on the page (`Main screens`, `Select a model`, `Table guide`,
`No data 1+ day state`, `Dimensions`) is unchanged and matches the build.

---

## Sidenav - six values moved

| Property | Drawn | Built |
| --- | --- | --- |
| Sidebar width | **256px** | 250px |
| `sidebar-content` gap | **24px** | 20px |
| `sidebar-menu-item` width | 224px | fills |
| Row padding | **4px** uniform | 6/4 |
| Row selected | `gray-50` fill, 1px `base/border`, `shadow/xs` | `neutral-100` fill, 1px `neutral-200` |
| Switch off track | **`rgba(163,163,163,0.5)`** | `base/input` `#d1d5db` |
| Switch track | 32 x 17.78, knob 14.22 | 36 x 20, knob 16 |

**The brand set changed membership.** `logo` now has eight variants: claude,
claudecode, codex, **moonshot**, openclaw, openai, opencode, openrouter.
Moonshot is new; **ChatGPT is gone**, having been there on 2026-08-23. Worth a
question - the rail still draws a ChatGPT row, so either the row loses its mark
or the mark was dropped by accident.

`status-label` confirms as built: Geist Medium 10/16, `Not protected` in
`amber-600`, `Protected` in `green-600`, the qualifier in `#6B7280`.

---

## Overview - unchanged at section level

All five sections still carry the check and the frame inventory is the same as
the 2026-08-21 read. Three screenshots dated 2026-08-26 09:19 were pasted onto
the page; they are reference images, not frames, and specify nothing.

**Not re-read at value level this pass.** The shared components mean it inherits
the Sidenav row changes above; whether its own cards moved is unverified.

---

## Settings and Onboarding - current

Both were re-read from node data earlier today and the findings applied. No
section changed since. One item is still open, and it is a question rather than
a defect:

- **Onboarding step 4** closes with *"That's all there is to it. Sign in and
  your first app is one toggle away."* The frame draws the subtitle and then
  the illustration, nothing between. Not a recorded deviation, so it is
  probably ours. Keep or cut is a copy call.

---

## What was applied

Items 1-3 of the original order, plus the App sort rule. Verified by
screenshotting the built panes and diffing them against `get_screenshot` of the
same frames.

**Setup.** The rail is the intro's rail. The card is 496 / r16 /
`shadow-base-lg`. `SetupHeader` gained a `HeaderTile`; `SetupBody` is the padded
region. `PrimaryButton` and `SecondaryButton` moved to 44px at radius 8, the
primary taking the drawn right arrow; `TextField` to a 44px r8 input on
`base/background` with a `CircleX` clear button; `OrDivider` to the drawn 19px
gap. Org rows tightened to 8px padding, a 36px tile, 16px marks and the drawn
two-tier elevation. Diagnostics copy now reads "Opt-in" and "routing stats",
with the never-shared sentence in Medium as drawn. The footer treatment was
reverted - see the correction above.

**Sidenav.** 256px rail, 24px content rhythm, 4px row padding, and the selected
row moved from `neutral-100`/`neutral-200` to `base/background` on `base/border`
under `shadow/xs`. `BaseSwitch` is 32x18 with a 14px knob and a `neutral-400/50`
off track - within a hair of the `base/input` it replaced, so no contrast change.

**App.** The model list sorts by provider then id, per the rule written on the
new section. The provider filter sorts too.

## Still open

1. **App multi-select** - the OpenCode picker that stays open until confirmed.
   **Resolved for now by matching `main`: a multi-provider tool gets no model
   card at all.** `main` has no model UI whatsoever - OpenCode appears there
   only as a routing target, and `lib/groups.ts` has called it a tool that talks
   to "several providers, not one model family" since before this branch. So the
   question the new pane was asking had no answer it could record: `ModelChoice`
   is `"app" | "gate"`, a binary, and `GATE_MODELS` is `[]` because no gateway
   endpoint reports models yet. Withholding the card is the honest state until
   both exist. Keyed on `buildGroups`' own "Other tools" membership, so it
   cannot disagree with the rail; pinned in `AppPane.test.tsx`.

   The "OpenCode-only or capability-driven" question answered itself in the
   code: **capability-driven**, on the multi-provider family. What still needs
   the designer, when the endpoint lands, is whether the picker's confirm
   subsumes the PAYG cost confirmation or precedes it.
2. **Overview at value level** - never read this pass, only its section list.
3. ~~The rest of Setup's frames~~ - **all 25 validated**, by pulling every frame
   through `GET /v1/images` in one call and reading them as contact sheets. That
   is what caught the correction above. The `Auth / Error states` frames draw a
   **Setup timeout** dialog ("Gate Connect timed out will trying to process your
   request. Would you like to try your request again, or go back to the setup
   start?", `Go back` / `Retry`) - still unbuilt, still unchecked.
4. **`Auth / Error states`** - still no check, fifth read. Stays unbuilt.
5. **Minimize2** - drawn in every Setup topbar, removed from the build under
   open question 5. Sixth read; the designer should settle it.
6. **ChatGPT lost its brand mark** while Moonshot gained one, but the rail still
   draws a ChatGPT row.
7. **Onboarding step 4's closing sentence**, which the frame does not draw.
