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

**There are two header archetypes, not one.** Read after the first draft of
this review, which had over-generalised from the two stepped panes:

- **Centred** - a 48px tile at radius 8 over a centred title and subtitle, with
  stacked full-width buttons below. Sign-in, the key form, the diagnostics step.
- **Row** - a 32px tile beside the title, subtitle left-aligned beneath, paired
  with the action footer. The stepped panes: org picker, name this device.

Both stack at 12px. `SetupHeader` centred everything and drew a bare 40px hex
mark with no tile at all.

**The panes have a bordered action footer.** A row with `padding: 16` and a 1px
top border, outside the padded body:

| Pane | Left | Right |
| --- | --- | --- |
| Org picker | `Use a different account` (Link variant, underlined) | `Back` + ArrowLeft, `Continue` + ArrowRight |
| Name this device | `Skip naming` (Link variant) | `Continue` + ArrowRight |

The build stacks full-width buttons inside the card body and has no `Back` and
no `Use a different account` anywhere. Note the drawn footer buttons are **hug
width**, radius 4 - unlike the sign-in card's.

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

**Setup.** The rail is the intro's rail. The card is 496 / r16 / `shadow-base-lg`
and unpadded, so a footer can span it. `SetupHeader` grew the two archetypes
above and a `HeaderTile`. New `SetupBody`, `SetupFooter` and `FooterButton`
primitives; `PrimaryButton` and `SecondaryButton` moved to 44px at radius 8, the
primary taking the drawn right arrow; `TextField` to a 44px r8 input on
`base/background` with a `CircleX` clear button; `OrDivider` to the drawn 19px
gap. Org rows tightened to 8px padding, a 36px tile, 16px marks and the drawn
two-tier elevation. Diagnostics copy now reads "Opt-in" and "routing stats",
with the never-shared sentence in Medium as drawn.

**Sidenav.** 256px rail, 24px content rhythm, 4px row padding, and the selected
row moved from `neutral-100`/`neutral-200` to `base/background` on `base/border`
under `shadow/xs`. `BaseSwitch` is 32x18 with a 14px knob and a `neutral-400/50`
off track - within a hair of the `base/input` it replaced, so no contrast change.

**App.** The model list sorts by provider then id, per the rule written on the
new section. The provider filter sorts too.

**One judgement recorded at the call site:** the org picker draws 44px footer
buttons and the name-device pane draws 36px, on the same bar. Both are explicit,
so `FooterButton` takes a `tall` flag and follows each frame rather than
averaging them.

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
3. **The rest of Setup's frames** - 26 exist; five were read. The connected pane
   and the many state variants are inferred from the archetypes, not read.
4. **`Auth / Error states`** - still no check, fifth read. Stays unbuilt.
5. **Minimize2** - drawn in every Setup topbar, removed from the build under
   open question 5. Sixth read; the designer should settle it.
6. **ChatGPT lost its brand mark** while Moonshot gained one, but the rail still
   draws a ChatGPT row.
7. **Onboarding step 4's closing sentence**, which the frame does not draw.
