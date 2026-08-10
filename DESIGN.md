---
name: Gate Connect
description: Menu-bar popover that points AI dev tools at a Constellation Gate gateway and keeps the key in the OS keychain.
colors:
  gate-indigo: "#3e4fea"
  gate-indigo-deep: "#2a38cb"
  indigo-wash: "rgba(62,79,234,0.08)"
  indigo-wash-strong: "rgba(62,79,234,0.14)"
  gatehouse-navy: "#002a5f"
  surface: "#ffffff"
  subtle: "#f8f9fc"
  sunken: "#eef0f6"
  highlight: "#f6ffe3"
  line: "#e8eaef"
  line-strong: "#d4d7e3"
  switch-off: "#868c9e"
  ink: "#0f1222"
  ink-2: "#2a2d3f"
  ink-3: "#55596f"
  ink-4: "#7a7f93"
  ink-5: "#a1a6bb"
  success: "#2ecc71"
  success-wash: "rgba(46,204,113,0.14)"
  success-deep: "#177a42"
  warning: "#f39c12"
  warning-deep: "#a25f02"
  warning-wash: "rgba(243,156,18,0.12)"
  error: "#e74c3c"
  error-deep: "#c0392b"
  error-wash: "rgba(231,76,60,0.12)"
typography:
  title:
    fontFamily: "Geist, ui-sans-serif, system-ui, sans-serif"
    fontSize: "14px"
    fontWeight: 600
    letterSpacing: "-0.01em"
  panel-title:
    fontFamily: "Geist, ui-sans-serif, system-ui, sans-serif"
    fontSize: "17px"
    fontWeight: 600
    letterSpacing: "-0.01em"
  body:
    fontFamily: "Geist, ui-sans-serif, system-ui, sans-serif"
    fontSize: "13.5px"
    fontWeight: 400
    lineHeight: 1.45
  body-small:
    fontFamily: "Geist, ui-sans-serif, system-ui, sans-serif"
    fontSize: "12.5px"
    fontWeight: 400
    lineHeight: 1.4
  caption:
    fontFamily: "Geist, ui-sans-serif, system-ui, sans-serif"
    fontSize: "11.5px"
    fontWeight: 400
    lineHeight: 1.4
  label:
    fontFamily: "Geist Mono, ui-monospace, SFMono-Regular, Menlo, monospace"
    fontSize: "10.5px"
    fontWeight: 500
    letterSpacing: "0.08em"
rounded:
  sm: "6px"
  tile: "9px"
  card: "10px"
  lg: "12px"
  pill: "48px"
spacing:
  xs: "6px"
  sm: "8px"
  md: "12px"
  lg: "14px"
components:
  button-accent:
    backgroundColor: "{colors.gate-indigo}"
    textColor: "#ffffff"
    rounded: "{rounded.sm}"
    height: "40px"
    padding: "0 16px"
  button-accent-hover:
    backgroundColor: "{colors.gate-indigo-deep}"
  button-secondary:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    rounded: "{rounded.sm}"
    height: "40px"
    padding: "0 16px"
  input:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    rounded: "{rounded.sm}"
    height: "36px"
    padding: "0 12px"
  card:
    backgroundColor: "{colors.surface}"
    rounded: "{rounded.card}"
    padding: "14px"
  pill-connected:
    backgroundColor: "{colors.success-wash}"
    textColor: "{colors.success-deep}"
    rounded: "{rounded.pill}"
    padding: "4px 8px"
---

# Design System: Gate Connect

## Overview

**Creative North Star: "The Gatehouse"**

Gate Connect is a small warm room at the edge of something big. A calm
keeper checks your credentials, waves your traffic through, and keeps the
key on its hook where you can see it. The popover (380 x 620, fixed) is
that one room: white paper surfaces, a ledger's precision in the
mono identifiers, and a single house color (Gate Indigo) that marks what
is live and actionable. The emotional target is the reassuring
gatekeeper: the user should close the popover thinking "good, that's
handled."

The system is ink-on-paper carried by shadows instead of lines. Elevated
surfaces (cards, inputs, buttons) never wear a solid 1px border; a
three-stop shadow stack draws their seam. Hairline borders exist only as
structural dividers between the popover's fixed zones. Controls are quiet
and precise: state changes are legible, never theatrical, and every
status pill reports system state truthfully.

Theme is light only. Confirmed rejections: no generic AI aesthetic
(purple/blue gradients, neon on dark, sparkles, chatbot chrome), no
enterprise SaaS dashboard look (heavy cards, bright blue CTAs), no
dev-tool brutalism (all-mono UI, ASCII chrome, all-caps everything).

**Key Characteristics:**
- One room: everything fits the 380px popover; panels slide, nothing stacks.
- Shadow-as-border seams on every elevated surface.
- Gate Indigo marks affordance and live state; neutrals do everything else.
- Geist for voice, Geist Mono for identity and system state.
- Quiet, precise controls; reassurance through transparency.

## Colors

A blue-cast neutral ramp on white paper, with one working accent and
small, wash-backed status colors.

### Primary
- **Gate Indigo** (#3e4fea): the house color. Accent buttons, the on-state
  of switches, active icon tiles, inline links, and the "Connect" half of
  the wordmark. **Gate Indigo Deep** (#2a38cb) is its hover/pressed state.
  **Indigo Wash** (rgba(62,79,234,0.08)) and **Indigo Wash Strong**
  (rgba(62,79,234,0.14)) back active icon tiles and selected rows; the
  accent never floods a surface.

### Secondary
- **Gatehouse Navy** (#002a5f): the Constellation hex mark and the "Gate"
  half of the wordmark. Identity only; never a control color.

### Neutral
- **Ink** (#0f1222): primary text and headings on white.
- **Ink 2** (#2a2d3f): strong secondary text, hover state of icon buttons.
- **Ink 3** (#55596f): body-secondary, row descriptions, section labels,
  idle pill text. The AA floor: the smallest ink that may carry real text.
- **Ink 4** (#7a7f93): placeholders, muted icons, and incidental mono
  identifiers only; never sentence copy or labels.
- **Ink 5** (#a1a6bb): faintest text, idle status dots.
- **Surface** (#ffffff): the popover card and every elevated surface.
- **Subtle** (#f8f9fc): hover fill for icon buttons and quiet rows.
- **Sunken** (#eef0f6): recessed wells: idle pills, inline notices,
  disabled tiles.
- **Highlight** (#f6ffe3): the one warm tint; reserved for restart/reopen
  hint banners.
- **Line** (#e8eaef) / **Line Strong** (#d4d7e3): structural dividers.
  Never card borders, and never the off-state switch track - see Switch.

### Status
- **Success** (#2ecc71) with **Success Wash** (rgba(46,204,113,0.14)) and
  **Success Deep** (#177a42) for pill text: the connected state. Success
  Deep is tuned to hold 4.5:1 on the wash over white at pill size.
- **Warning** (#f39c12) with **Warning Wash** (rgba(243,156,18,0.12)):
  degraded or partial states. Warning is never text; on a wash the text is
  ink-2 and the dot carries the color.
- **Error** (#e74c3c) with **Error Wash** (rgba(231,76,60,0.12)):
  failures and destructive actions. Same rule: dot and icon in the color,
  body-sized text in ink.

### Named Rules
**The Provisional Indigo Rule.** The Gate Indigo palette is the incumbent
identity of this popover (from the Claude Design handoff) but it is
provisional. The wider Constellation Gate family rule is ink-primary,
never blue, and a future pass may fold the popover back to it. Extend the
existing indigo usage patterns faithfully; do not invent new indigo
surface types, and do not bleed the `gc-*` palette into shared cg
components.

**The Wash-First Rule.** Status and accent colors touch surfaces only as
washes (8 to 14% alpha) with a solid dot or text in the full color.
Solid accent fills are reserved for the primary button and the switch
on-state.

## Typography

**Body Font:** Geist (with ui-sans-serif, system-ui fallback), weights 400
to 600, `ss01` on, `text-wrap: pretty`. Bundled locally via @fontsource;
the app never fetches fonts at runtime.
**Label/Mono Font:** Geist Mono (with ui-monospace, Menlo fallback),
weights 400 to 500, bundled the same way.

**Character:** one contemporary grotesque doing the talking, one mono
doing the record-keeping. The pairing reads like a well-kept logbook:
friendly sentences, exact entries.

### Hierarchy
- **Panel Title** (600, 17px, tracking -0.01 to -0.02em): takeover and
  full-screen moment headings (quit confirm, update ready, welcome,
  connected). The largest type in the popover.
- **Title** (600, 14 to 14.5px, tracking -0.01 to -0.02em): panel titles
  and the wordmark.
- **Row Title** (600, 13.5px): card and row headings ("Routing",
  tool names).
- **Body** (400 to 500, 13 to 13.5px): buttons, inputs, and sentence copy.
- **Body Small** (400 to 500, 12.5px): takeover body copy and inline
  action links (Replace key, Sign out, Cancel).
- **Caption** (400 to 500, 11.5 to 12px, ink-3): row descriptions, hints,
  and banner copy.
- **Label** (mono, 500, 10.5px, 0.08em, uppercase): section labels.
  Mono also carries, in lowercase, the workspace sub-label, URLs, hosts,
  key placeholders, and pill-adjacent identifiers at 10.5 to 11px.

The onboarding tour lives in its own larger window and may use a 27px
display heading; inside the 380px popover the ramp tops out at Panel
Title.

### Units, and the eleven-versus-six gap
Every size above is expressed in **rem against a 16px root**, never in px.
That is what makes the ramp scalable: px is absolute, so before this the
whole app was immune to a user's text-size setting (measured: root 16px to
32px left a 13.5px heading at 13.5px).

The `fontSize` tokens in `tailwind.config.ts` are the implementation, and
there are **eleven of them against the six names above**: `gc-label`,
`gc-micro`, `gc-caption`, `gc-caption-lg`, `gc-body-sm`, `gc-body-md`,
`gc-body`, `gc-title-sm`, `gc-title`, `gc-panel-title`, `gc-display`.
Eleven is what the code actually uses. The six names here keep their
meaning and remain the design intent; the five in-between steps are
consolidation candidates, and collapsing them changes how the product looks
at 100%, so it belongs to a typesetting pass rather than to the units change
that introduced them. Until that happens, this section describes the target
and the tokens describe the state.

Tokens carry **font size only, no line-height tuple**. Tailwind would
otherwise emit `line-height` beside every `font-size` and change computed
leading at each call site that does not already carry a `leading-*` utility.

### Text scaling
The popover scales its own type: five steps from 100% to 200%, driven by
`useTextScale`, with the control in Settings and a Cmd/Ctrl plus-minus
shortcut. WCAG 2.1 SC 1.4.4 asks for 200% without loss of content or
function, and a fixed, non-resizable menubar window exposes no other way to
get there.

**Not webview zoom**, though that would be one call. The app root is
`h-full w-full`, so the CSS layout width *is* the window width: zooming a
380px window to 200% lays the page out at 190 CSS px, and "Waiting on
routing" is a ~130px pill. Zoom shrinks the room while it grows the type.
Scaling the rem root grows type inside a composition that keeps its shape.

Five steps rather than a slider, because a slider implies a precision the
vertical budget cannot honour.

### Named Rules
**The Mono Earns Its Place Rule.** Geist Mono appears only where identity
or precision matters: URLs, hosts, API key placeholders, workspace names,
section labels, status identifiers, tabular counts. Body copy, labels on
buttons, and descriptions are always sans. Mono is a signal, not a vibe.

**The Rem Rule.** No `text-[Npx]` literal in product code. A new size is a
new token or an existing one; a px literal is a size that silently opts out
of scaling, and one of those is enough to break a screen at 200%.

**Type size is the breakpoint.** The popover has no width breakpoints and
cannot get them: it is one fixed window. Where a layout must change at large
text, it keys off the type itself in `em` rather than a media query. The
header is the worked example: `flex-wrap` with `basis-[8em]` asks for eight
characters' worth of room at whatever the current size is, so the pill group
wraps beneath the wordmark at 200% instead of colliding with it.

## Layout

One room, 380px wide and 620px tall, fixed and non-resizable. The header
(wordmark, workspace, connection pill, gear) and any footer are fixed;
only the body scrolls, on 6px ink-ramp scrollbars. Secondary surfaces
(tool detail, settings, forms) slide in as full-popover panels, never as
nested dialogs or stacked modals.

Spacing sits on a 4px grid with a 14px (p-3.5) room padding as the base
rhythm: cards are padded 14px, stacked with 10px gaps, and internal
row gaps run 8 to 12px. Density is high but breathable; asymmetric
spacing (e.g. header 14px top / 8px bottom) is used where the rhythm
calls for it.

Motion follows one grammar: direction-aware slides. Onboarding steps
slide 28px horizontally (260ms, cubic-bezier(0.2, 0.7, 0.3, 1)); the
update panel rises 16px over the room (240ms, same curve).

**Reduced motion removes travel, not feedback.** The OS preference used to
collapse everything to 0.01ms, which measured 137 elements frozen against
10 animated normally and took the directional grammar with it: in one
380px room direction is the only navigational metaphor there is, so
zeroing every duration made a push and a pop the same event on screen for
the one user who asked for less motion. The contract now splits the two.
Nothing transitions a property that moves (the guard whitelists colour,
background, border, fill, stroke, opacity and shadow, so the Switch
thumb's 18px slide arrives instantly while a hover still eases), and the
three travelling animations cross-fade in place instead at 120ms linear.
Direction is genuinely given up under the preference, since no animation
conveys it without moving; it stays recoverable from the static layout,
because a panel wears a back chevron and its own h1 and Home wears
neither. Infinite animations are no longer clamped to one iteration: the
only one is the org picker's loading spinner, and a frozen spinner does
not reduce motion, it removes the instruction to wait.

### Named Rules
**The One Room Rule.** Every screen works inside the 380px frame without
scrolling chrome. If a design needs a second window or a stacked modal,
it is the wrong design.

**The Vertical Budget.** The window is **380 x 620** (`tauri.conf.json`,
fixed: `resizable: false`, no runtime resize). The body's scroll viewport is
**587px** on macOS and Windows (620 less the 33px pinned credential strip)
and 555px on Linux, whose custom title bar takes another 32px. Measure
against those numbers rather than the frame; the Linux case is the one that
catches a composition out.

Home's daily state must fit with no overflow **at 100% text**: header,
routing card, ledger heading, four 44px family rows and the dashboard link
measure **443.4px, leaving 143.6px (24.5%) with nothing scrolling**. Above
100% the body scrolls, and that is the intended trade: SC 1.4.4 asks for no
loss of content or function, not for everything to stay above the fold, and
scrolling a body is neither. What must not happen at any size is content
colliding or clipping, which is what "type size is the breakpoint" in
Typography exists to prevent.

That 24.5% is not a target and not yet a verdict. The screen is no longer
*empty with its content hidden behind a door*, which is what the rows were
brought back to fix, but a quarter of the primary surface is still unspent
and the next composition pass should decide deliberately what it is for.

**Correction, 2026-08-07.** This section previously said 360 x 520 with a
487px viewport, and so did every other size reference in this file and in
PRODUCT.md. The real window has been 380 x 620 throughout. Every measurement
taken before this date, including the figures in the round-15 and round-16
critique snapshots under `.impeccable/critique/`, was captured in a 360 x 520
harness built from the wrong number: 20px too narrow and 100px too short.
Those readings are directionally useful and numerically wrong, and the
emptiness figures in particular are understated, because a shorter frame
makes a screen look fuller than it is.

States that add chrome are allowed to scroll, and should:
the certificate ceremony, a stale-address banner or two exception sentences
each earn their height, and the fold cue (`gc-scroll-more`) exists for
exactly that. What is not allowed is the inverse, which is what this rule
was written after: a third of the primary screen empty, nothing scrolling,
and the list it was supposed to hold parked behind a door.

**Blockers outrank inventory.** Anything that explains why traffic is not
flowing, and carries the fix, sits directly under the routing card and above
the list. Ordering the ledger first pushed the certificate card and its only
Trust button below the fold in the one state where nothing routes.

## Elevation & Depth

Depth is structural, not atmospheric: the shadow stack IS the border.
Elevated surfaces are white sitting on white, and their edge is drawn by
a three-stop stack: a hairline outline, a bottom inset stroke that gives
the row its card feel, and a faint ambient drop. Hover raises contrast
within the same stack rather than lifting the element.

### Shadow Vocabulary
- **Seam** (`box-shadow: 0 0 0 1px color-mix(in oklch, oklch(0.165 0 0) 6%, transparent), 0 1px 0 0 color-mix(in oklch, oklch(0.165 0 0) 6%, transparent), 0 2px 4px -1px color-mix(in oklch, oklch(0.165 0 0) 4%, transparent)`):
  the resting edge of every card, input, and secondary button
  (`shadow-border`).
- **Seam Hover** (`box-shadow: 0 0 0 1px color-mix(in oklch, oklch(0.165 0 0) 9%, transparent), 0 1px 0 0 color-mix(in oklch, oklch(0.165 0 0) 8%, transparent), 0 2px 6px -1px color-mix(in oklch, oklch(0.165 0 0) 6%, transparent)`):
  the same seam, slightly darker and deeper, for hover and focus-within
  (`shadow-border-hover`).
- **Popup** (`box-shadow: 0 0 0 1px color-mix(in oklch, oklch(0.165 0 0) 4%, transparent), 0 4px 16px -2px color-mix(in oklch, oklch(0.165 0 0) 8%, transparent)`):
  small floating elements inside the room.
- **Popover** (`box-shadow: 0 0 0 1px color-mix(in oklch, oklch(0.165 0 0) 8%, transparent), 0 12px 32px -8px color-mix(in oklch, oklch(0.165 0 0) 22%, transparent), 0 4px 12px -2px color-mix(in oklch, oklch(0.165 0 0) 10%, transparent)`):
  the popover window itself against the desktop.
- **Card Drop** (`box-shadow: 0 2px 4px rgba(10,10,10,0.04), 0 8px 24px rgba(10,10,10,0.06)`):
  the prototype's softer ambient drop (`shadow-gc-md`), used where a card
  floats free of the seam language.

### Named Rules
**The Seam Rule.** Elevated surfaces never wear a solid 1px border; the
seam stack draws their edge. Hairline borders (`line`, #e8eaef) are
permitted only as structural dividers between the popover's fixed zones
(header/footer separators), never around cards, inputs, or buttons.

## Shapes

Soft rectangles on a tight radius scale: 6px is the everyday radius
(buttons, inputs, notices), 9px for icon tiles, 10px for cards and rows,
12px for the popover window and modal-grade panels (locked), and a 48px
pill for status capsules and switches. Corners grow with the surface's
importance; nothing is fully square, nothing but pills and dots is round.
The Tauri window itself is transparent with 12px rounded corners; the
white card fills it edge to edge.

## Components

### Buttons
Quiet and precise: color states, not size or shadow theatrics.
- **Shape:** everyday radius (6px), 40px tall, 16px horizontal padding,
  13.5px medium Geist, 8px icon gap.
- **Accent:** Gate Indigo (#3e4fea) fill with white text; hover and
  active deepen to Gate Indigo Deep (#2a38cb).
- **Secondary:** white surface with ink text, wearing the Seam; hover
  moves to Seam Hover.
- **Disabled:** 45% opacity, pointer events off.
- **Icon buttons:** 28px square, radius 6px, ink-3 glyph; hover fills
  subtle (#f8f9fc) and darkens the glyph to ink-2.
- **The safe option is its equal.** In any takeover that offers a
  destructive or irreversible action, Cancel is a full secondary button of
  the same width and height as the buttons it sits with, never a text link.
  A takeover that puts initial focus on Cancel (because Enter on an unread
  panel should not decide the outcome) and then renders it as 12.5px text
  makes the faintest control on the panel the one the panel points at.
- **`sm` (32px) is for a button embedded in something else** - an inline
  banner, an expanded row, an inline confirm pair - where a 40px control
  would outweigh what it sits in. The rule is about the container, not
  about the window.

### Switch
- **Style:** 38x22px pill track; on-state Gate Indigo, off-state
  Switch Off (#868c9e). 18px white thumb slides 16px and carries an
  11px indigo check when on. The check is the confirmation: state is
  readable without color.
- **Why not Line Strong:** the off track is a component state indicator
  under SC 1.4.11, so it needs 3:1 against every surface it sits on.
  #d4d7e3 is 1.44:1 on white; #868c9e is 3.36:1 on white and 3.19:1 on
  Subtle, which is what an expanded or hovered row uses.

### Status Pills
- **Style:** 48px-radius capsule, 11px medium text, 6px status dot,
  8px horizontal padding, and a 1px seam ring tinted from the state's own
  hue (see the ladder below).
- **Connected / Routed:** Success Wash background, Success Deep (#177a42)
  text, solid Success dot, Success Deep ring at 30%.
- **Partial:** Warning Wash background, ink-2 text, solid Warning dot,
  Warning Deep ring at 45%. The honest third state for a system that is
  genuinely half-on (routing up, certificate untrusted).
- **Error:** Error Wash background, ink-2 text, solid Error dot, Error
  Deep ring at 65%. A family dark *because something failed* must not
  borrow the grey it uses for a switch the user set.
- **Idle / Signed out / Not routed:** sunken (#eef0f6) background, ink-3
  text, ink-3 dot, ink-4 ring at 45%.
- **Needs trust / Set up elsewhere:** Warning Wash, ink-2 text, Warning
  Deep dot and ring at 45%. Both are "half-on for a reason outside this
  row", so they share the Partial rung.
- **Waiting on routing:** sunken background, ink-2 text (not ink-3),
  ink-3 dot, ink-4 ring at 45%. Sunken because nothing is flowing; ink-2
  because the user did not ask for this.
- **The Pill Seam Ladder.** A wash at 8 to 14% alpha measures 1.09 to
  1.16:1 against the row it sits on, so the capsule was invisible as an
  object: the words floated in tinted air beside a switch track reading
  5.98:1, and reality lost the row to intent by a factor of 5.4 on the
  element this product calls the most important pixel on the screen. The
  ring is what makes a pill a thing. It is weighted by severity rather
  than applied evenly: error reads about 3:1 as a bordered chip, and
  Routed stays the quietest of the set at about 1.5:1, because four green
  pills on a healthy launch should read as "that's handled" and not as a
  wall of edges.
- **One table, every level.** The ladder is a rule for the component, not
  for one caller. It first shipped on the family pill alone, which left the
  member rows - the one place in the app where an intent control and a
  reality report sit side by side and are *allowed to disagree* - at 1.08:1
  beside a 5.68:1 switch, i.e. the exact measurement the ring was introduced
  to fix, unfixed. There is now one skin table keyed by label, shared by the
  family pill, the member pill and the header pill. Two copies of a severity
  ladder would drift.
- **Not a conformance fix.** The pill's state is carried by its text
  (4.63 to 4.84:1) and its dot, both of which already pass, so the ring is
  a weight decision and does not need to reach 3:1 at every level.
- A ring, not a border: solid 1px borders are what this system draws with
  box-shadow instead.
- Pills report system state truthfully and are never decorative. On a family
  panel a pill sits beside a switch that reports intent, and the two are
  allowed to disagree; on Home a pill carries its row alone.

### Ledger Rows (Home)
The list of model families is Home's primary content, not a panel behind a
door. PRODUCT.md's second principle puts it there and the vertical budget
allows it: parked behind a door, Home measured 33% empty with nothing
scrolling.
- **Anatomy:** family name (13px medium ink, `truncate`), status pill,
  stroked chevron. No switch and no expander: those are the family panel's
  job, and keeping them off this row is what stops Home re-crowding.
- **Height:** 44px at rest. A row with an exception grows by its sentence
  (11px, two lines maximum, error in Error Deep and everything quieter in
  ink-2).
- **Order:** exception-first, error before needs-trust before drifted, with
  the healthy tail holding catalog order via a stable sort.
- **Card-owned states never print on a row.** The master being off and the
  certificate both belong to the card above, which is also the thing that
  can fix them; printed per row they repeat one sentence up to four times
  directly under the card that just said it.
- **Depth grammar:** the row navigates (stroked chevron) and the member level
  drops the glyph for the word "Details". Two depths, two affordances, no glyph
  doing two jobs. There was a third for as long as the panel listed every
  family and expanded one in place (filled caret, rotating); the panel is
  about one family now, so nothing is left to open in place and the caret
  retired with the accordion.
- **Every door leads somewhere different.** Four rows carrying four chevrons
  into one panel that varied only by which family arrived expanded is a
  promise the destination did not keep, and three of that panel's four
  visible rows were a copy of the screen the user had just left. A row's
  chevron opens that row's family.
- **Headings:** the group heading is an h2 and family names are h3 beneath
  it on Home; on a family panel, whose title is the h1, the members are h2.
  Four families and six pills must be navigable by heading.

### Family Panel
One family per panel, opened from its row on Home. The panel's h1 is the
family's own name, not the question Home heads its rows with.
- **Anatomy:** a control row directly under the header carrying the label
  "Route through Gate" (the h1 already said which family), the count on a
  second line, then the family pill and the family switch pushed right;
  then the group-level banners; then the members.
- **The count, not the exception.** Every exception `groupSummary` can name
  (error, needs-trust, master-off, drifted) has a banner below it with the
  remedy attached, so printing the summary sentence here too would state one
  fact twice and put the shorter, unactionable copy first. The count is the
  half the banners do not carry, and it is the one screen where the
  denominator is itemized directly beneath it.
- **No family name below the h1, no shell-environment switch.** The switch
  that sets `HTTPS_PROXY` spans every family at once, so a panel about one
  family is the one place it cannot live; Home carries it.
- **Members are the list.** `role="list"` belongs to the members here, since
  there are no families on this screen to enumerate.
- **`flex-wrap` with an `em` basis** on the control row's text column, the
  same rule the routing card and the ledger rows use: at 200% the pill and
  switch drop to their own line rather than starving the label.

### Cards / Rows
- **Corner Style:** 10px radius.
- **Background:** white surface; leading 36px icon tile at 9px radius,
  Indigo Wash + indigo glyph when the feature is live, sunken + ink-4
  glyph when off.
- **Shadow Strategy:** Seam at rest, Seam Hover on hover (see Elevation).
- **Border:** none, ever.
- **Internal Padding:** 14px, with 12px gap between tile, text block, and
  trailing controls (switch, chevron).

### Inputs / Fields
- **Style:** white surface, 6px radius, 36px tall, 12px horizontal
  padding, 13px ink text, wearing the Seam; optional leading icon in
  ink-4.
- **Focus:** the wrapper moves to Seam Hover on focus-within; no glow,
  no accent ring.
- **Placeholder:** ink-3. It carries real instruction ("Enter new sk-gw-… key"), and ink-4 is 4.0:1 on white.

### Section Labels
- **Style:** Geist Mono, 10.5px, 500, uppercase, 0.08em tracking, ink-3,
  padded 14px sides / 12px top / 6px bottom. Structural chrome for a long
  scrolling list of unrelated sections: Settings uses three.
- **Not for a sentence.** Home's ledger heading is sentence-case sans
  (11.5px medium ink-3), because "What routes through Gate" is a sentence
  and the mono rule below forbids mono for sentence copy. Mono earns its
  place on identifiers, not on prose that happens to label something.

### Hint Banners (signature)
Inline, dismissible truth-telling: when routing state and running apps
disagree, a banner says so plainly.
- **Restart/Reopen hints:** highlight (#f6ffe3) fill, 6px radius, Seam,
  12px medium ink text with a bold imperative ("Restart your agent").
- **Stale-address notices:** sunken fill, error-colored icon and 11.5px
  text, trailing dismiss icon button.
- **Group-level blockers:** wash fill keyed to severity, the colour on the
  icon and the sentence in ink (the Wash-First rule), and the remedy as an
  `sm` accent button in the banner itself.
- **Every blocking member state gets one, at group level.** Master-off,
  error, drifted and needs-trust each announce themselves where the family
  is named, because that is the level whose sentence the user reads. The
  certificate was the last one to get this: it was named on the family row
  and then explained nowhere, with its remedy two disclosures down.
- **One remedy per cause.** Where a banner offers the fix, the member rows
  beneath it must not repeat it. There is a single machine-wide
  certificate, so a Trust button inside a member could only ever be the
  second or third copy of the banner's, and two expanded members put three
  identical buttons on screen for one action.
- **An icon may not contradict the button beside it.** A confirm step that
  destroys something takes `info` in Warning Wash, not `shieldCheck` in
  Indigo Wash: a shield 40px from a red danger button says "protected"
  while the button says "destroy".

### Intent versus Reality
The product's signature risk is a setting that is on while nothing flows,
so the two are separate fields (`desired`, `routed`) and are allowed to
disagree on screen. The switch reports intent; a pill or a note reports
what is actually happening.
- Anything that reports intent must not paint itself in live-state indigo
  when it cannot be live. The shell-environment channel's stored choice
  survives routing being turned off, so the switch has to answer for reading
  "on" over a channel that cannot carry anything.
- Use the existing vocabulary for the condition. "Waiting on routing" is
  what the member pill says for exactly this state; a second phrasing for
  one condition is a second thing to learn.
- **Which sentence reports reality depends on the screen.** On a family
  panel, whose subject is one family, the reality sentence is the panel's
  own. On Home the master card already reports "Off · N waiting" once and
  countably, so nothing further down repeats it: the shell-environment
  switch points `aria-describedby` at that line instead of printing a second
  copy 190px below it. This is the same rule that keeps `master-off` off the
  ledger rows, applied to the one control on Home that is not a row.
- Wire it to assistive tech: a switch that can read "on" over something
  broken points `aria-describedby` at the sentence that reports reality, so
  one control speaks both channels.
- **The shell-environment switch is a line, not a card, and sits below the
  ledger.** It routes every command-line tool at once, so it belongs to no
  family and cannot live on a family panel. Its first stay on Home failed
  geometrically: 66px under the master switch, same 38x22 track, same indigo,
  which said by proximity that changing git and curl was routing's equal. The
  ledger between them and the absence of card weight are what make the second
  stay different. It is withheld entirely when no family is installed, since
  an empty ledger restores exactly that adjacency.

## Do's and Don'ts

### Do:
- **Do** draw every card, input, and secondary button edge with the Seam
  stack (`shadow-border`), moving to Seam Hover on hover/focus-within.
- **Do** keep Gate Indigo scoped to affordance and live state: primary
  buttons, switch on-states, active icon tiles, links.
- **Do** set identifiers (URLs, hosts, workspace names, key placeholders,
  section labels) in Geist Mono at 10.5 to 11px; keep pills' text sans.
- **Do** honor `prefers-reduced-motion` by removing travel rather than
  feedback: no transition on a property that moves, and a 120ms cross-fade
  where a slide would have been. See the Motion paragraph above; the old
  blanket-duration collapse is not the contract any more. The rule that has
  not changed: every animation must answer to the preference somehow.
- **Do** keep every screen inside the 380px popover; secondary surfaces
  slide as full panels with the 260ms directional grammar.
- **Do** meet WCAG 2.1 AA contrast on white; ink-3 (#55596f) is the
  floor for body-secondary text.
- **Do** name things the way the user would. The UI's nouns are tools and
  apps: a group label must be something someone could point at on their own
  machine, never the name of the filter that built it ("Agent harnesses"
  was the label on `filter(t => !claimed.has(t.slug))`).
- **Do** give an interactive element a hit area of at least 24px even when
  its visible box is smaller, using `before:-inset-*` or matched
  `inline-block` padding and negative margin so the layout does not move.

### Don't:
- **Don't** state one fact in more than one place on the same screen. A
  header pill, a card sub-line and a row that all report the same fault
  make the user reconcile three vocabularies before learning which tool
  broke; whichever element can also be acted on is the one that keeps it.
- **Don't** put a solid 1px border on a card, input, or button; hairlines
  are for the popover's fixed structural seams only.
- **Don't** introduce new indigo surface types or leak the `gc-*` palette
  into shared cg components (The Provisional Indigo Rule).
- **Don't** use gradients, neon accents, sparkles, or dark chrome; the
  generic AI aesthetic is a locked anti-reference.
- **Don't** set body copy or button labels in mono, or use all-caps
  outside the 10.5px mono section labels.
- **Don't** stack modals or open second windows; one room, sliding
  panels.
- **Don't** use the em dash character anywhere in UI copy.
