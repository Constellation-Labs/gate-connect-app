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
key on its hook where you can see it. The popover (360px wide, ~520px
tall) is that one room: white paper surfaces, a ledger's precision in the
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
- One room: everything fits a 360px popover; panels slide, nothing stacks.
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
display heading; inside the 360px popover the ramp tops out at Panel
Title.

### Named Rules
**The Mono Earns Its Place Rule.** Geist Mono appears only where identity
or precision matters: URLs, hosts, API key placeholders, workspace names,
section labels, status identifiers, tabular counts. Body copy, labels on
buttons, and descriptions are always sans. Mono is a signal, not a vibe.

## Layout

One room, 360px wide, ~520px tall on a typical screen. The header
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
update panel rises 16px over the room (240ms, same curve). The OS
reduced-motion preference collapses all of it to instant; that contract
is global and non-negotiable.

### Named Rules
**The One Room Rule.** Every screen works inside the 360px frame without
scrolling chrome. If a design needs a second window or a stacked modal,
it is the wrong design.

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
  8px horizontal padding.
- **Connected / Routed:** Success Wash background, Success Deep (#177a42)
  text, solid Success dot.
- **Partial:** Warning Wash background, ink-2 text, solid Warning dot.
  The honest third state for a system that is genuinely half-on (routing
  up, certificate untrusted).
- **Idle / Signed out / Not routed:** sunken (#eef0f6) background, ink-3
  text, ink-5 dot.
- Pills report system state truthfully; they are never decorative, and a
  pill on a tool row is a door (opens the tool detail), not a verdict.

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
  padded 14px sides / 12px top / 6px bottom. The ledger's column
  headings.

### Hint Banners (signature)
Inline, dismissible truth-telling: when routing state and running apps
disagree, a banner says so plainly.
- **Restart/Reopen hints:** highlight (#f6ffe3) fill, 6px radius, Seam,
  12px medium ink text with a bold imperative ("Restart your agent").
- **Stale-address notices:** sunken fill, error-colored icon and 11.5px
  text, trailing dismiss icon button.

## Do's and Don'ts

### Do:
- **Do** draw every card, input, and secondary button edge with the Seam
  stack (`shadow-border`), moving to Seam Hover on hover/focus-within.
- **Do** keep Gate Indigo scoped to affordance and live state: primary
  buttons, switch on-states, active icon tiles, links.
- **Do** set identifiers (URLs, hosts, workspace names, key placeholders,
  section labels) in Geist Mono at 10.5 to 11px; keep pills' text sans.
- **Do** honor `prefers-reduced-motion` for every animation; the global
  collapse in index.css is the contract.
- **Do** keep every screen inside the 360px popover; secondary surfaces
  slide as full panels with the 260ms directional grammar.
- **Do** meet WCAG 2.1 AA contrast on white; ink-3 (#55596f) is the
  floor for body-secondary text.

### Don't:
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
