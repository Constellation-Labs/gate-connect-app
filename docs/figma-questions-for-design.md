# Questions for design, from the Figma audit

Every question below came out of comparing the file against the shipped app.
Each one is something the code cannot decide for itself: either the file
disagrees with itself, or it disagrees with the app and we need to know which
is right.

Two passes are behind this. The first compared values - type, colour, spacing,
radius - page by page (`docs/review-figma-*.md`). The second compared **states
and sequence**: which screens exist, in what order, and whether the code can
reach each one (`docs/review-flow-*.md`). Questions 2 and 3 come from the
second pass and are the two largest here, because they are about whether a
capability exists at all rather than what it looks like.

Node ids are given so each can be opened directly. Question 6 is kept as
ANSWERED rather than deleted, because closing it changed a rule we had
written down.

---

## 1. Are identifiers mono or sans?

**What we see.** The model picker draws model ids in **Geist Medium sans**
(`665:18400`, `665:19064`). On Settings, four more identifiers are drawn sans
too. That is six identifiers drawn sans and none drawn mono.

**Why it matters.** The app renders every identifier in Geist Mono - model ids,
URLs, hosts, keys, install ids, versions. It is a deliberate rule ("mono is a
signal, not a vibe") and it is load-bearing: mono is how a user tells a
machine-readable value from prose. If the file means sans, that rule goes and a
lot of screens change at once.

**We asked whether these were slips. They are not - the file is consistent.**
We resolved four identifier classes node by node and every one is sans:

| Drawn value | Node | Style |
| --- | --- | --- |
| model ids | `665:18400`, `665:19064` | Geist Medium, sans |
| an API key | `143:68381` (`sk-gw-661b17…`) | `copy/14`, Geist Regular |
| a version | `116:30083` (`v0.1.4`) | `copy/14`, Geist Regular |
| four more Settings identifiers | see `docs/review-figma-settings.md` | sans |

Zero identifier *values* are drawn mono anywhere we have looked. Mono is used,
and used deliberately, but only for **eyebrows** (`mono/eyebrow`) and **status
pill labels** (`mono/label-12`) - which is exactly half of what our rule
claims.

So this is no longer "did the mono style get missed": the file has a position
and holds it consistently. It is now a decision about whether to adopt it.

**What we need.** Confirm that identifier *values* are meant to be sans, with
mono reserved for eyebrows and pill labels. If so we will rewrite the rule and
the change is large but mechanical.

---

## 2. Does OpenCode get a model card at all?

**What we see.** `App / Select multiple models (Opencode)` is a three-frame
flow, and `662:17579` renders the OpenCode pane with a **complete Model
selection card**: the radio pair with `Gate models` selected, a two-column
`Current Gate models` grid (`Anthropic gate/opus 5`, `Deepseek
gate/deepseek-v4-flash`, `Moonshot gate/kimi-k3`, each with its vendor mark), a
`3 of 14 models enabled` footer and a `Choose models` button. The two frames
after it are the picker open and the pane with four models chosen.

Notably, the sidebar in that same frame files OpenCode under `OTHER TOOLS`, so
the design **agrees** with us that it is a multi-provider tool - and still draws
it a model card.

**Why it matters.** We withhold the card entirely for multi-provider tools.
OpenCode, OpenClaw and Hermes route whichever of their several configured
providers Gate covers, so "which model does this app use on Gate" has no single
answer for them - that is the reasoning in the code, and it is why no
`onChooseModel` is passed and no card renders. If it is wrong, all three of
those frames are currently unreachable and a shipped capability is missing. If
it is right, three frames want deleting.

This is the largest single disagreement the audit found, and it is not
decidable by measurement.

**What we need.** Should a multi-provider tool be able to pick Gate models? If
yes, we need to know what the choice means when the tool asks for a provider
Gate is not serving.

---

## 3. Which model-picker behaviour is correct?

**What we see.** Two annotations, on two different sections, specifying
different things:

- `139:66782` (single, on a Claude Desktop frame): "When the user selects a
  model it automatically closes the modal and applies their selection."
- `671:19288` (multi, on the OpenCode frame): "When the user selects a model
  (multiple) the modal stays open until the user confirms their selections.
  Then the modal will close"

The frames back both up structurally: the single dialog (`665:18400`) is 536
tall and **has no footer at all** - no Cancel, no Apply - which only works if a
row click is itself the commit. The multi one (`665:19064`) is 588 tall, and the
52px difference is exactly the footer row of three buttons.

**Why it matters.** We ship one dialog, the multi one, to everybody. So neither
annotation is honoured on the surface it was written on: the auto-close was
drawn on a single-provider tool that gets the confirm footer, and the
confirm-footer flow was drawn on OpenCode, which cannot open the picker at all
(question 2).

And the two cannot simply both be built, because AG-590 requires that the last
enabled model not be removable without choosing another. A click that both
applies and closes cannot stop to say "that was your last model" - it has
already closed.

**What we need.** One behaviour, or a rule for when each applies. If auto-close
is wanted for single-provider tools, we also need to know what should happen
when the click would leave the tool with no model.

---

## 4. ANSWERED - the menu has both, and the component is unlinked

**Found first, so this question could be asked at all.** We had this down as
"the component library has been deleted", because the old Components page
(`113:16762`) resolves with zero children. That was wrong: it moved into
**Banners** (`744:37738`), **Menus** (`744:37691`) and **Sidenav**
(`408:15625`). Only the `Button` set (`685:20855`) is still unreachable for us.
Good news, and it immediately produced a real conflict.

**What we see.** The `topnav/menu` **component** (`744:37692`) draws three rows
- Visit dashboard, **Contact support**, Read Gate docs - and no Quit. The
**instance** in the Overview flow (`116:27225`) draws four rows including Quit.
Both are 224px wide; the component is 114px tall for three rows, the instance
146px for four.

**Why it matters.** The rule we work to says the component wins over an
instance, which would mean removing Quit and adding Contact support. We have
done the opposite: we ship Quit, because the flow frame added it, and we omit
Contact support, because the support URL 404s and a menu entry that opens a
broken page is worse than an absent one. So we are currently following the
instance over the component and would rather that were a decision than an
accident.

**Largely answered, by how the file is wired.** `topnav/menu` has **no
instances anywhere**. Every menu in the file - `116:27225` on Overview and
`744:38192` on the Tray page - is a detached `frame`, not an `instance` of
`744:37692`. (The metadata does distinguish the two: the Banners canvas is full
of real `<instance>` nodes.)

So the component is not the source of anything. Nothing is linked to it,
which is why it never gained Quit when the flow copies did, and why
"component beats instance" cannot arbitrate here - there is no instance
relationship to arbitrate. The copies are what is being maintained, and the
code follows them. That part is settled.

**Answered 2026-09-04 on both halves.**

**Quit stays**, because the component is not the source of anything (above).

**Contact support now ships**, URL notwithstanding. It is drawn in the
component and in the flow copy (`116:27225`, four items), and it is in the menu
pointing at `GATE_SUPPORT_URL` even though that address 404s today. The
argument it overruled - that an entry opening a broken page is worse than an
absent one, since the user cannot tell "not built" from "broken" - is recorded
on `TopnavAction` so it is not re-litigated.

`SettingsPane` keeps its Support row omitted, which is a different call: no
Settings frame draws one.

Still open, as housekeeping: if `744:37692` is meant to be the library
component it needs relinking, or deleting so it stops looking authoritative.

---

## 5. Dialog buttons: 4px or 8px?

**What we see.** Both, in roughly equal measure.

| Where | Radius | Border |
| --- | --- | --- |
| Overview dialogs, four instances (`694:32469`, `694:32470`, `694:33509`, `694:33518`) | 8px | `base/input` |
| Gate-model confirmation (`130:48311`, `130:48312`) | 8px | `base/input` |
| Settings dialogs, all six instances | 4px | `base/input` |
| Model picker footer (`665:19135`), three buttons | 4px | `base/border` |

**This is now a narrower question than we first asked.** We can read each
instance's variant name, and it turns out these are not different buttons being
styled differently - they are **the same variant of the same component**.
`143:70627` (Settings, 4px) and `694:32469` (Overview, 8px) both report
`Variant=Outline, State=Default, Size=default`, both point at master
`685:20928`, and both draw identical padding (10/12), border (`base/input`),
shadow and type. The **only** difference between them is the radius.

Two instances of one variant cannot legitimately differ in radius. So at least
one of them carries a **local override** - somebody nudged a corner on a
detached copy - rather than the file expressing two rules.

Counting what we can reach: `Size=default` appears at 4px nine times (the six
Settings dialog buttons and the three in the picker footer) and at 8px six
times (the four Overview ones and the two on the Gate-model confirmation). The
other sizes are `icon` 4px, `xs` 4px, `sm` 8px.

**What we need.** Which radius does the `Button` component actually define for
`Size=default`? We cannot open `685:20928` over our integration to check. Once
you tell us, the answer also tells us which instances need resetting in the
file - and we will make the code follow the component rather than the majority.

Related, same cause: the picker footer's outline button sits on `base/border`
where every other dialog button sits on `base/input`. Likely the same kind of
override; worth checking while you are in there.

---

## 6. ANSWERED - there are at least four button sizes

Left here for the record rather than deleted, because it changes what we build
to. We asked whether a 24px button was real. It is, and it is named: Figma
reports `744:37756` as **`Variant=Outline, State=Default, Size=xs`**
(`685:20937`), and the set publishes `height/h-6: 24`. So the set has at least
`default`, `sm`, `xs` and `icon`, and our "exactly two sizes" rule was wrong.
`banners.tsx` already matched `xs` on every property.

No question for you here. We settled it by reading each instance's variant
name, which works even though the set node itself (`685:20855`) will not open
for us - if that page could be shared it would save us guessing in future.

---

## 7. Is the 536px dialog width intended?

**What we see.** The two quit confirmations (`694:33002`, `694:33340`) are
536px wide. Every other dialog is 480, 512, 544 or 600.

**Why it matters.** 536 reads like an edge that got dragged rather than a
number that was chosen: the left edge sits exactly where a **512** dialog
centred in 1024 would start, and the right edge lands about 24px past the
mirror of it. So the dialog is not centred, and 512 would be.

**What we need.** Keep 536, or is this meant to be 512? We have built 536 as
drawn either way.

---

## 8. Notifications: one row or three?

**What we see.** `116:29086` draws a single row: "Alert me when a request is
blocked or flagged".

**Why it matters.** The app splits that into three rows - notify on blocked,
notify on flagged, and a sound toggle - because the backend gates those
preferences separately and one switch cannot express "blocked but not flagged".
So the drawn sentence describes something the single control would not actually
do.

**What we need.** Should Settings show the one drawn row, or the three the
preferences support? If one, we need to know what it writes when a user only
wants blocked alerts.

---

## 9. Model picker: confirm the current copy and the count slot

**What we see.** The two picker frames differ, and the app has ended up taking
some of each.

- Title: `665:18400` says "Choose a Gate model", the newer `665:19064` says
  "Choose Gate models". The app says the older one.
- Primary button: the frame says "Apply selections", the app says "Save
  models".
- The "N selected" badge (`682:20038`) is set to **opacity 0** in the newer
  frame, and the count appears instead as a ghost button at the left of the
  footer. The app still draws a count in the older frame's slot.

**Why it matters.** The picker is the one screen where a user commits money to a
model, so its wording matters more than most. And a hidden layer is ambiguous:
opacity 0 usually means retired, but not always.

**What we need.** Confirm the newer title, subtitle and "Apply selections", and
confirm the "N selected" badge is retired in favour of the footer count.

---

## 10. Two onboarding paragraphs the file does not draw

**What we see.** No frame draws step 1's paragraph about the key living in the
OS keychain, or step 3's closing "That's all there is to it."

**Why it matters.** The keychain sentence is the one place onboarding tells a
user where their credential is kept, which is the reassurance the whole product
turns on. We would rather not drop it silently, and we would rather not keep
undrawn copy without someone agreeing to it.

**Partly answered by the file itself.** The keychain paragraph is not
invented copy: hidden frame `212:85283` is a **retired fifth onboarding step**
("How to connect with Gate", eyebrow `3 of 4`, Config apps / Proxy apps), and
its body is exactly that paragraph. So when the step was removed its content
was folded into step 1 rather than dropped. That is a reason to keep it, but it
is still your call whether it belongs there.

**What we need.** Keep both, keep the keychain line only, or drop both?

---

## 11. WITHDRAWN - the file and the code already agree

Left in place because we asked it and the answer corrects us, not you.

We reported that the Apply-changes dialog's drawn weighting was the opposite of
the app's. It is not. `130:58448` is `Variant=Default` - the filled primary,
`base/primary` with the gradient - and its label is **"No, I will reopen
later"**. `130:58447` is `Variant=Outline` and reads **"Yes, close affected
apps"**. So the file makes the *safe* action the primary, which is a deliberate
inversion of the usual arrangement, and the code has done exactly that all
along, with a comment saying so.

The real defect was ours and is fixed: our focus rule only redirected away from
a destructive **primary**, so a destructive **secondary** matched nothing and
focus fell to the first button in the panel - which is the secondary. Enter
therefore landed on "Yes, close affected apps". The rule now covers both
arrangements.

**Nothing needed from you.**

---

## 12. ANSWERED - 1080 is the intended onboarding width

**What we see.** The file annotates the app at **1024x720** (`App dimensions:
1024x720px`, plus `1024px` and `720px` dimension labels on
`Setting/dimensions`). Every frame on every page is 1024 wide.

**Why it matters.** The intro window ships at 1080x720, so onboarding is 56px
wider than everything drawn. Nothing in the file is drawn at 1080.

**Answered 2026-09-04: 1080 is right.** The intro window is deliberately wider
than the 1024 every flow frame is drawn at, and `lib.rs`'s 1080x720 stays as it
is. Recorded because it looks exactly like drift and an audit will find it
again.

Still open, and smaller: the file never states a **minimum** size. We have
locked the main window at 1024x720 so it can never render below a drawn size.
Tell us if you would rather it shrink further and we will reason about the
layout below 1024.

---

## 13. Two small inconsistencies to confirm or rebind

Neither changes much on screen, but both make the file ambiguous to measure.

- **`label/14` is used at two different trackings.** The tray master-card title
  (`744:38097`) draws it at 0%, while the footer org name (`744:38190`) and the
  CLI card title (`735:37344`) draw it at -1%. We have followed each node
  literally, so one title is untracked and its neighbours are not.
- **The tray master and CLI cards resolve an older variable layer.** They come
  back as `base/foreground` #0a0a0a and `color/muted-foreground` #737373, while
  the rows immediately beside them resolve #030712 and #6b7280. The difference
  is invisible, but it means two cards on one screen are bound to different
  token sets.

**What we need.** Are these intentional? If not, rebinding them in the file
would let us keep measuring node by node without second-guessing.

---

## 14. "Routing" or "Routed" in the status banner?

**What we see.** The `banner/routing` **component** (`744:37758`) reads
**"Routing"**. Every routed frame on Flows/Overview reads **"Routed · 4 of 4
Apps"**, and that is what we ship.

**Why it matters.** Same shape as question 4: the component and the instances
disagree, and we are following the instances. It is one word on the banner that
sits above every screen, so we would rather it were deliberate.

**What we need.** Which word?

---

## 15. ANSWERED - the dot pattern is real

**What we see.** The banner's `dot-matrix-light` layer (in `744:37750`) is an
**empty frame**. We rendered both the component and the flow instance
(`228:85974`) at 1:1 and sampled them: a flat gradient, no periodic pixels.

**Why it matters.** We draw a dot overlay there - 16% white on an 8px pitch -
and we cannot find a source for it. The gradient underneath it is confirmed
correct. Either the pattern was intended and is not rendering in the file, or
we invented it.

**Answered 2026-09-04: the pattern is visible in Figma, so our dots stay.**

Worth recording how we got this wrong, because the file cannot be used to check
it: `dot-matrix-light` reads as an **empty frame** over the API, and rendering
both the component and the instance at 1:1 and sampling the pixels finds a flat
gradient with no periodic variation. Whatever carries the pattern does not
survive the export. Noted in `banners.tsx` too, so the next audit does not
delete the dots on the same evidence.

---

## 16. Does the chart tooltip head with "12" or "12:00"?

**What we see.** The `chart/tooltip` component (`744:37709`) heads with a bare
**"12"**. The redrawn chart axis (`706:*`) labels its ticks `HH:00`.

**Why it matters.** We render the tooltip heading through the same formatter as
the axis, so it reads "12:00". That was a deliberate call - one bucket phrased
two ways is worse than either - but the tooltip component is a newer node than
the axis, so the file's most recent word on it is the bare hour.

**What we need.** Should the tooltip match the axis at `HH:00`, or is the bare
hour intended there?

---

## 17. Is the Redacted chart series violet or purple?

**What we see.** Two nodes, two colours, for the same series.

| Node | Colour |
| --- | --- |
| Legend swatch `706:10096` | `violet/500` #8b5cf6 |
| Tooltip component `744:37728` | `purple/500` #a855f7 |

**Why it matters.** We ship violet/500, matching the legend. There is a
standing note in our own code warning that violet and purple are easy to
confuse here and not to "correct" one to the other by eye, so we have left it
alone rather than follow the newer node.

**And the tiebreak now points somewhere, which it did not when we asked.**
Since finding the component canvases, "component beats instance" is executable
again - and `chart/tooltip` (`744:37708`) IS a component, on the Menus canvas,
while the legend swatch lives inside a flow frame. By the rule, purple/500
wins. We have deliberately NOT applied that, because our own code carries a
standing warning that violet and purple are easy to confuse here and not to
swap one for the other by eye, and because a chart series changing colour is
the kind of thing a person should agree to.

**What we need.** One colour for the series - and if it is purple, we would
like to retire that warning at the same time. While you are there: the Blocked
swatch is `red/400` in both nodes and we had it at `red/500`, which we have now
fixed - worth confirming red/400 is right.

---

## For information: things we found and fixed without asking

So the list above is not mistaken for the whole audit. All of these were
measured off the file and the app now matches it: card heading sizes and the
20/24 line-height, card rule insets, the pane header's icon frame and status
line, the stat figure's weight and its delta, the feed badge's fill and radius,
empty-state type, the tray's menu radii, shadow and glyph sizes, the CLI card's
padding, the onboarding step art width and its footer checkbox, three
letter-spacing steps that had no token, and the activity table's whole column
set.

Two things we corrected in ourselves rather than in the file: a checkbox we had
made 6px where the file draws 1.667px, and a claim that no 536px frame existed,
which was our own measuring error.
