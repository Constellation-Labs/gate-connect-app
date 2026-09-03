# Questions for design, from the Figma audit

Every question below came out of comparing the file against the shipped app,
page by page. Each one is something the code cannot decide for itself: either
the file disagrees with itself, or it disagrees with the app and we need to know
which is right.

Node ids are given so each can be opened directly. Evidence for all of it is in
`docs/review-figma-*.md`.

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

**What we need.** Is sans the intention for identifiers, or are these frames
using the default face because the mono style was not applied? If sans is
intended for *some* identifiers and not others, what separates them?

---

## 2. Does the topnav menu have Quit, or Contact support?

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

**What we need.** Which of the two is current? And separately: is Contact
support meant to ship before there is an address for it to open?

---

## 3. Dialog buttons: 4px or 8px?

**What we see.** Both, in roughly equal measure.

| Where | Radius | Border |
| --- | --- | --- |
| Overview dialogs, four instances (`694:32469`, `694:32470`, `694:33509`, `694:33518`) | 8px | `base/input` |
| Gate-model confirmation (`130:48311`, `130:48312`) | 8px | `base/input` |
| Settings dialogs, all six instances | 4px | `base/input` |
| Model picker footer (`665:19135`), three buttons | 4px | `base/border` |

**Why it matters.** One button component serves all of these. We currently draw
8px on a `base/input` line in dialogs and 4px on a `base/border` line in panes,
which matches the Overview and confirmation frames exactly and contradicts
Settings and the picker footer. We cannot satisfy both, so today the Settings
dialogs and the picker footer render one step rounder than drawn.

**What we need.** Which is canonical for a dialog button? And is the picker
footer's `base/border` line deliberate, given every other dialog button sits on
`base/input`?

---

## 4. ANSWERED - there are at least four button sizes

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

## 5. Is the 536px dialog width intended?

**What we see.** The two quit confirmations (`694:33002`, `694:33340`) are
536px wide. Every other dialog is 480, 512, 544 or 600.

**Why it matters.** 536 reads like an edge that got dragged rather than a
number that was chosen: the left edge sits exactly where a **512** dialog
centred in 1024 would start, and the right edge lands about 24px past the
mirror of it. So the dialog is not centred, and 512 would be.

**What we need.** Keep 536, or is this meant to be 512? We have built 536 as
drawn either way.

---

## 6. Notifications: one row or three?

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

## 7. Model picker: confirm the current copy and the count slot

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

## 8. Two onboarding paragraphs the file does not draw

**What we see.** No frame draws step 1's paragraph about the key living in the
OS keychain, or step 3's closing "That's all there is to it."

**Why it matters.** The keychain sentence is the one place onboarding tells a
user where their credential is kept, which is the reassurance the whole product
turns on. We would rather not drop it silently, and we would rather not keep
undrawn copy without someone agreeing to it.

**What we need.** Keep both, keep the keychain line only, or drop both?

---

## 9. Apply changes: which button is the primary?

**What we see.** In the Apply-changes dialog the drawn weighting puts the
emphasis opposite to where the app puts it.

**Why it matters.** This is the dialog that closes the user's running apps.
Focus follows the primary, so as it stands a user who presses Enter twice
closes Codex. We deliberately open destructive dialogs with focus on the safe
button.

**What we need.** Confirm which action is the primary here. If closing apps is
the primary, we will keep the emphasis and move initial focus to the safe
button instead.

---

## 10. Should the onboarding window be 1024 wide?

**What we see.** The file annotates the app at **1024x720** (`App dimensions:
1024x720px`, plus `1024px` and `720px` dimension labels on
`Setting/dimensions`). Every frame on every page is 1024 wide.

**Why it matters.** The intro window ships at 1080x720, so onboarding is 56px
wider than everything drawn. Nothing in the file is drawn at 1080.

**What we need.** Should onboarding match at 1024, or is it deliberately wider?

Related, and easier: the file never states a **minimum** size. We have locked
the main window at 1024x720 so it can never render below a drawn size. Tell us
if you would rather it shrink further, and we will reason about the layout
below 1024.

---

## 11. Two small inconsistencies to confirm or rebind

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

## 12. "Routing" or "Routed" in the status banner?

**What we see.** The `banner/routing` **component** (`744:37758`) reads
**"Routing"**. Every routed frame on Flows/Overview reads **"Routed · 4 of 4
Apps"**, and that is what we ship.

**Why it matters.** Same shape as question 2: the component and the instances
disagree, and we are following the instances. It is one word on the banner that
sits above every screen, so we would rather it were deliberate.

**What we need.** Which word?

---

## 13. Is there meant to be a dot pattern behind the update banner?

**What we see.** The banner's `dot-matrix-light` layer (in `744:37750`) is an
**empty frame**. We rendered both the component and the flow instance
(`228:85974`) at 1:1 and sampled them: a flat gradient, no periodic pixels.

**Why it matters.** We draw a dot overlay there - 16% white on an 8px pitch -
and we cannot find a source for it. The gradient underneath it is confirmed
correct. Either the pattern was intended and is not rendering in the file, or
we invented it.

**What we need.** Should the dots be there? If yes we will keep ours and you
may want to restore the layer; if no we will remove them.

---

## 14. Does the chart tooltip head with "12" or "12:00"?

**What we see.** The `chart/tooltip` component (`744:37709`) heads with a bare
**"12"**. The redrawn chart axis (`706:*`) labels its ticks `HH:00`.

**Why it matters.** We render the tooltip heading through the same formatter as
the axis, so it reads "12:00". That was a deliberate call - one bucket phrased
two ways is worse than either - but the tooltip component is a newer node than
the axis, so the file's most recent word on it is the bare hour.

**What we need.** Should the tooltip match the axis at `HH:00`, or is the bare
hour intended there?

---

## 15. Is the Redacted chart series violet or purple?

**What we see.** Two nodes, two colours, for the same series.

| Node | Colour |
| --- | --- |
| Legend swatch `706:10096` | `violet/500` #8b5cf6 |
| Tooltip component `744:37728` | `purple/500` #a855f7 |

**Why it matters.** We ship violet/500, matching the legend. There is a
standing note in our own code warning that violet and purple are easy to
confuse here and not to "correct" one to the other by eye, so we have left it
alone rather than follow the newer node.

**What we need.** One colour for the series. While you are there: the Blocked
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
