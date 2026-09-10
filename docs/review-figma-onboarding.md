# Figma audit: Onboarding flow (and locating Auth)

Read 2026-09-03 against file `9FrccCojXy0f8QD8Wm5Lln`.

Pages audited:

- **`Flows / Onboarding`** = canvas `177:79237`, layer name `↳ Onboarding ✅`.
  Four window frames: `232:4370` (welcome, x=0), `212:84747` (1 of 3),
  `212:85100` (2 of 3), `212:85391` (3 of 3), plus one hidden frame
  `212:85283` and three exported art frames (`604:13757`, `604:13806`,
  `604:14040`).
- **`Flows / Auth`** = canvas **`177:79238`**, layer name `↳ Setup ✅`. Found
  (see "Could not determine" for how, and why the page listing does not show
  it). Four captioned sections: `Auth / Connect with Gate` ✅ (frames at
  y=-56), `Auth / Connect with API key` ✅ (y=939), `Auth / Organizations` ✅
  (y=1934), `Auth / Error states` **no check** (y=3024).

Code audited: `src/screens/Onboarding.tsx`, `src-tauri/src/lib.rs`, and - as a
scope correction - `src/components/gc/setup.tsx`, which is where the new-UI Auth
flow actually lives.

Every class proposed below was run through the project's own
`tailwind.config.ts` and confirmed to emit CSS at the stated value
(`rounded-control` 4px, `rounded-xs` 2px, `tracking-heading` -0.2px,
`tracking-heading-16` -0.16px, `tracking-button-xs` -0.12px,
`shadow-base-btn-sm`, `h-8`, `gap-1.5`, `text-sm`).

---

## Verified correct

The Onboarding screen is in good shape. These were measured and match.

**Welcome frame (`232:4370`)**

- Hero tile `240:4501`: 96px, `rounded-[16px]`, `blue-ribbon/300` border, white
  fill, `shadow/md`, and the inset pair
  `inset 0 4px 8px rgba(255,255,255,0.4), inset 0 -4px 8px rgba(151,195,255,0.24)`.
  Code reproduces all six values exactly (`Onboarding.tsx:51-57`), hex mark at
  the drawn 56px.
- Title `240:4503`: `heading/32` = Geist **Medium** 32/36, letterSpacing -4%
  = **-1.28px**, `Welcome to` in `base/foreground` + ` Gate Connect` in
  `#203de2`. Code: `text-base-3xl font-medium leading-9 tracking-heading-32`
  with a `text-base-primary` span. Exact.
- Subtitle `240:4505`: `heading/14` Medium 14/20 letterSpacing 0,
  `base/muted-foreground`. Exact. Copy is `Created by Constellation Network`,
  which the code now matches (`plans/new-app-ui-figma.md` still records the old
  "Constellation Gate AI" wording; that is stale).
- Divider `241:4511`: `base/input`, full 540 width. Exact.
- Body `240:4508`: `copy/16` on `base/foreground`, drawn as
  `<p>…</p><p></p><p>…</p>` - an **empty paragraph** between the two, i.e. a
  24px blank line. Code's `space-y-6` is exactly right, and
  `Click Next to get started.` is Medium in both.
- Container 540px wide; code `max-w-[540px]` for `index === 0`. Exact.

**Progress rail (`402:13840`)**

- 8px, `border-b base/border`, track `#f9fafb`, fill `#7195ff`.
- Fill widths across the four frames are 256 / 512 / 768 / 1024 of 1024 =
  25 / 50 / 75 / 100%. Code's `(step / total) * 100` with `total = 4` produces
  exactly that.

**Step cards (`212:84769`, `212:85210`, `212:85407`)**

- Card: 640 wide, `rounded-[16px]`, `border base/border`, `bg base/card`,
  `p-[24px]`, `shadow/lg`. Code `rounded-2xl border border-base-border
  bg-base-card p-6 shadow-base-lg` - and `shadow-base-lg` in
  `tailwind.config.ts` is character-for-character `shadow/lg`. Exact.
- Spacing: 24px between card sections, 8px header→body, 12px eyebrow→title,
  16px art→note. Code `gap-6` / `gap-2` / `gap-3` / `gap-4`. Exact.
- Eyebrow `212:84894`: `mono/eyebrow` Geist Mono Medium 12/16 at 8%
  (= `tracking-eyebrow` 0.96px), uppercase, `base/muted-foreground`, both
  labels on one `justify-between` row. Exact.
- Title `212:84890`: **20px SemiBold, leading 24, tracking -0.2px**,
  `#030712`. Code `text-xl font-semibold leading-6 tracking-heading` is right.
  Worth noting: this is *not* the `heading/20` token (which is Medium 20/28) -
  the frame draws a raw SemiBold at 24 leading, and the code matches the frame.
  That is the correct call under CLAUDE.md.
- Body: `copy/14` on `base/foreground`. Size and colour correct.
- Note strip: `rounded-[8px]`, `border base/border`, `px-[16px] py-[12px]`,
  12/16 text on `base/foreground`, 16px icon on `base/muted-foreground`. Code
  matches. Icons match per step: `ShieldCheck` / `MonitorSmartphone` /
  `BellDot`.
- Note-strip icon gap: **the file disagrees with itself.** Step 1
  (`212:85066`, text at x=41) draws 8px; steps 2 and 3 (`212:85265`,
  `231:3500`, text at x=45) draw 12px. Taking the newest node ids
  (`231:3500`/`231:3502` > `212:85265` > `212:85066`) and the 2-of-3 majority,
  12px wins - so code's `gap-3` is correct and step 1's frame is the outlier.
- Footer `232:4445`: `bg base/card`, `border-t base/border`, `px-[24px]
  py-[12px]`, checkbox left, 220px button pair at `gap-[12px]`. Exact,
  including the fixed `w-[220px]`.
- Primary button `232:4449`: `rounded-[8px]`, border `alpha/80`
  (white 20%), `linear-gradient(180deg, rgba(255,255,255,0.08),
  rgba(0,0,0,0.08))` over `#203de2`, `base/primary-foreground` #f9fafb, 14px
  Medium at -0.28px, and a shadow stack identical to `shadow-base-btn-primary`.
  Exact.
- `Previous` is drawn at `opacity-0` on the welcome frame only, exactly as the
  code comment claims; `invisible` preserves the same 220px alignment.
- `Get started` is the last step's primary label (verified by render). Exact.
- Step 3's card is drawn shorter (452 vs 472) purely because its body is 2 lines
  rather than 3; the code's natural height does this for free.
- The three note-strip sentences are verbatim matches.

**Chrome**

- 48px topbar, centred lockup, and **no top-right actions**: `toolbar-actions`
  (`232:4383`) exists in the layer tree but renders as a 1x1 transparent PNG on
  every Onboarding frame. Code omitting it is correct.
- `Gate` in `blue-ribbon-800`, `Connect` in `neutral-600`: `232:4375` resolves
  both `tailwind colors/blue-ribbon/800` and `tailwind colors/neutral/600` as
  real variables, so `text-neutral-600` here is deliberate, not a stray grey.

---

## Mismatches

### 1. Step art is capped 52px narrower than the column it sits in
**MEASURED.** Highest visual impact of anything here.

- Figma `604:13804` (and `710:36505`, `710:36133`): the art is `w-full` at
  `aspect-[1770/660]`, filling the whole 590px content column - flush with the
  title above it and the note strip directly below it.
- Code `src/screens/Onboarding.tsx:78`, `:97`, `:124` -
  `mx-auto block h-auto w-full max-w-[540px]`.
- Effect: inside the 640px card at `p-6` the column is 592px, so the art renders
  540px with a 26px gap on each side, visibly narrower than the note strip
  beneath it, and ~200px tall against the drawn 220px.
- Fix: drop `max-w-[540px]` from the three step `<img>` elements. Keep it on the
  welcome container at `:308`, where 540 is the measured width.

### 2. Footer checkbox label is 12px grey; the frame draws 14px ink
**MEASURED** (via `get_design_context` on `232:4445`, not just variable names).

- Figma `232:4446`: `copy/14` = Geist Regular **14/20**, tracking -0.14px,
  colour **`base/foreground` #030712**. The instance is 206x20, and 206px only
  fits that string at 14px.
- Code `Onboarding.tsx:405` - `text-base-xs leading-4 text-neutral-600` =
  12px/16px on #525252.
- This is the case the grey guard is for: the node resolves to a `base/*`
  semantic token and the code uses a raw neutral.
- Fix: `text-sm leading-5 text-base-foreground`.

### 3. "Show me where Gate Connect lives" is drawn as a pane `sm` button
**MEASURED.** This is CLAUDE.md's own worked example, drawn exactly as the file
describes it.

Figma `267:5083`: 242x**32**, `rounded-[4px]`, border **`base/border`**
#e5e7eb, `px-[12px] py-[8px]`, `gap-[6px]`, label **12px** Medium /16 at
**-0.12px** on `base/primary`, drop `shadow/xs` plus
`inset 0 4px 4px rgba(255,255,255,0.4), inset 0 -4px 4px rgba(0,0,0,0.04)`.

Code reuses `IntroButton` (`Onboarding.tsx:211`), which is the footer's
`default` button: `h-9` (36), `rounded-md` (8px), `border-base-input`,
`gap-2` (8), `text-sm` (14px), `tracking-button-sm` (-0.28px),
`shadow-base-btn`.

- Fix: give the locate button an `sm` variant -
  `h-8 rounded-control border-base-border gap-1.5 text-base-xs
  tracking-button-xs shadow-base-btn-sm`. Leave the footer pair alone: they
  measure h36 / `rounded-[8px]` / `base/input`, which the code already matches.

### 4. Two body paragraphs that no frame draws
**MEASURED** as to what the frames contain; the decision is the designer's.

- Step 1 `212:84775` is a single 3-line block, 590x60: two sentences joined by a
  hard line break and nothing else. Code `Onboarding.tsx:85` adds a third
  paragraph - "For Claude Code and Codex, Gate Connect points the app's own
  config at your gateway … Your Gate key stays in `<keychain>`, not a plain
  file." That is 4+ extra lines and pushes the card past the drawn 472px.
- Step 3 `212:85415` is a single 2-line block, 590x40. Code `:135` adds
  "That's all there is to it. Sign in and your first app is one toggle away."
- Structural, same area: the frame joins its two sentences with a `<br>` inside
  one paragraph, while the code renders `sub` and `body[0]` as separate `<p>`s
  inside `gap-2`, inserting an 8px gap the frame does not have.
- CLAUDE.md names exactly two decided copy exceptions and says to **raise** a
  third rather than decide it. This is that: the extra prose is good, useful
  copy (and the keychain sentence serves Design Principle 1), but it is not
  drawn. Flagging, not fixing.

### 5. `Next` carries a right arrow the frames do not draw
**MEASURED**, two independent ways.

- `get_design_context` expands both footer Button instances in `232:4445` to
  background + label + inset overlay, with no icon child.
- The rendered frames show `Next` and `Get started` centred with no glyph.
- Code `Onboarding.tsx:464` - `{!last && <Icon name="arrowRight" size={16} />}`.
- Same pattern in Auth: `src/components/gc/setup.tsx:453` passes `arrow` (also
  `:505`, `:555`, `:637`), and its comment at `:200` says "The drawn primary
  carries the Button component's right arrow". The Auth frame `202:80095`
  renders "Continue with Gate account" with **no** arrow.
- The `Button` component set is on a page MCP will not open, so I cannot confirm
  whether the component itself carries one. But CLAUDE.md is explicit that where
  the component set and a pane instance disagree, the frame wins.
- Fix: drop the arrow, or get the designer to settle the component set. Given it
  affects five call sites across two flows, worth asking rather than
  unilaterally removing.

### 6. Step 2's locate button gap, and a card that jumps between steps
**MEASURED** for the gap, **INFERRED** for the jump.

- Figma: card `212:85210` at y=124, h=472 (bottom 596); button `267:5083` at
  y=**612** - a **16px** gap. The card sits at the same y=124 as steps 1 and 3.
- Code `Onboarding.tsx:392` - `mt-6` = 24px.
- The card and the button also live in one vertically-centred block, so on step
  2 the card rides roughly 28px higher than on steps 1 and 3, making it jump as
  the user pages through. In the Figma the card holds still and the button hangs
  below the centred box.
- Fix: `mt-4`. The jump needs the button taken out of the centred measurement.

### 7. Welcome body tracking is -0.2px; drawn -0.16px
**MEASURED.** `240:4508` is `copy/16`, letterSpacing -1% of 16px = **-0.16px**.
Code `Onboarding.tsx:330` uses `tracking-heading`, which emits **-0.2px**.
`tracking-heading-16` exists and emits -0.16px.

- Fix: `tracking-heading-16`.

### 8. Checkbox box radius is 2px; drawn 4px
**MEASURED.** `I232:4446;46:68` is `rounded-[4px]`. Code `Onboarding.tsx:417`
uses `rounded-xs`, verified to emit 2px.

- Fix: `rounded-control`.

### 9. `Previous` fill is white; drawn `neutral/50`
**MEASURED**, low impact. `232:4448` fills `tailwind colors/neutral/50`
#fafafa; code `Onboarding.tsx:214` uses `bg-base-card` (#ffffff). One tint step
against a white footer.

### 10. The onboarding window is 1080x720; every frame is 1024x720
**MEASURED** on the numbers, **INFERRED** that 1024 is intended.

- `src-tauri/src/lib.rs:2439` - `.inner_size(1080.0, 720.0)`,
  `.min_inner_size(760.0, 560.0)`.
- All four Onboarding frames are 1024x720, the Setup frames are 1024x720, and
  the app's own main window is `MAIN_MIN_SIZE = (1024.0, 720.0)`
  (`lib.rs:1442`). CLAUDE.md's Surface section says 1024x720.
- 56px wider than drawn and than the rest of the app. Content is centred, so the
  visible effect is extra side margin - but `min_inner_size` also lets this
  window shrink to 760x560, a size no frame draws and where the 640px card plus
  `p-6` will start colliding with the viewport.
- Fix: `.inner_size(1024.0, 720.0)`.

### 11. Progress fill misses `mix-blend-overlay`
**MEASURED**, negligible - listing it only so the next reader does not re-derive
it.

`402:13843` paints `#7195ff` and lays the black→white 64% gradient over it at
`mix-blend-mode: overlay`. Code `Onboarding.tsx:186` composites the gradient
normally. Computed: identical at the left end (#29365C both ways), #BAD9FF
(overlay) vs #CCD9FF (normal) at the right. Not worth a change. The drawn track
also carries `inset 0 -2px 2px rgba(255,255,255,0.48), inset 0 2px 2px
rgba(0,0,0,0.04)`, which the code omits.

### 12. Step 2's first sentence is rewritten per platform
Copy divergence, looks deliberate and defensible - raising it per CLAUDE.md.

- Figma `232:3544`: "Gate Connect stays open in your menu bar, so it is always
  easy to access while you work."
- Code `whereItLives()` (`Onboarding.tsx:20-29`): "Gate Connect lives in the
  menu bar at the top right of your screen.", with Windows ("system tray at the
  bottom right") and Linux ("top bar at the top right") variants.
- The second sentence matches the frame verbatim.
- The drawn sentence is macOS-only vocabulary and this app ships Windows and
  Linux, so adapting it is the right instinct. But it is a third copy exception
  beyond the two CLAUDE.md has decided, and the rewrite also drops the "always
  easy to access while you work" reassurance in favour of a location statement.

---

## Could not determine

**The Auth page - found, but not the way the brief expected.** It is canvas
`177:79238`, and its layer name is `↳ Setup ✅`, not "Auth". The four section
captions inside it are named `Auth / …`, which is where the "Auth page" name
comes from. `Auth / Error states` carries no green check, so per the plan doc it
is still moving and I did not audit it.

**`get_metadata` with no `nodeId` is unreliable for this file.** It reports
exactly one page - `319:4686: Design docs` - even though `116:26381` (Overview),
`116:28963` (Settings), `116:30199` (App), `177:79237` (Onboarding) and
`177:79238` (Setup/Auth) all resolve when queried directly. Probing the sibling
ids in the brief identified Overview/Settings/App but not Auth; I got
`177:79238` from `plans/new-app-ui-figma.md:133`, which records it. Worth noting
for the next audit: the plan doc is the reliable index of node ids for this file,
not the MCP page listing.

**The `Button` component set.** `685:20855` / `685:20856` return "invalid node
selection" - they sit on a page MCP will not open. So the arrow question in
mismatch 5 cannot be closed from the component side, only from the frames.

**The hero tile's inset shadows** are not variable-bound, so I can confirm them
only as raw effect values (which the code matches exactly), not as tokens.

**The collapse button in the topbar.** The `toolbar-actions` node renders empty
on every Onboarding frame, but the same node *does* render on the Auth frames
(an ellipsis and a collapse glyph, visible in `202:80095`). Whether that
collapse control should exist is `plans/new-app-ui-figma.md` open question 5 and
is still unresolved; nothing in these two pages settles it.

**Scope corrections to the brief.**

- `src/components/InstallationPicker.tsx` does not exist.
  `src/components/gc/InstallationPicker.tsx` does, but it is the Overview pane's
  installation `<select>` (AG-572) and has no Onboarding or Setup frame. Out of
  scope for these two flows.
- `src/components/OAuthOffer.tsx` is a popover-shell takeover on the legacy
  `gc.*` ink system. Its strings ("Sign in instead of pasting a key", "Keep
  using my API key", "You can switch either way later") appear **nowhere** in
  either page, so there is no frame to audit it against. It cannot be checked
  against this Figma.
- `src/screens/FirstRun.tsx` and `src/screens/OrgPicker.tsx` are likewise
  `gc.*`-only popover screens reached from `App.tsx`.
- **The new-UI Auth flow is `src/components/gc/setup.tsx`** (`WelcomePane`,
  `ApiKeyPane`, `NameDevicePane`, `OrgPickerPane`, `ConnectedPane`), which the
  brief did not name. Its copy now matches the drawn strings the plan doc listed
  as missing: "Continue with Gate account", "Use an API key", "Name this
  device", and the sign-in subtitle verbatim ("Sign in once, then choose which AI
  apps route through Gate. Claude, Codex, OpenCode, and supported apps keep
  working normally while Gate handles protection underneath."). The plan doc's
  Auth gap table at `plans/new-app-ui-figma.md:133-175` is therefore stale.
  A full measured audit of the 24 Setup frames is a separate pass and was not
  attempted here beyond the sign-in frame `202:80095`.
