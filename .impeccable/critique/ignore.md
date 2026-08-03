# Accepted detector findings

Findings the mechanical detector reports that are documented exceptions,
not drift. Critique runs drop matches silently.

Matching is by file + value, not line, so these survive the code moving.
Line numbers are recorded as of 2026-08-03 and are a navigation aid only.

- `design-system-color` on `src/screens/Onboarding.tsx:31`, `:112` and
  `:116`: the `#000` literals sit inside `mask-image` alpha gradients,
  where the color channel is never painted; only opacity matters. Two fade
  the platform mockups on tour step 3; the third fades the step 2 hero,
  whose capture is cropped mid-card.
- `design-system-font-size` 27px on `src/screens/Onboarding.tsx:215`: the
  onboarding tour renders in its own larger window and uses a 27px display
  heading, documented in DESIGN.md's Typography section. The popover ramp
  tops out at Panel Title (17px).
- `design-system-color` `oklch(0.165 0 0)` on `tailwind.config.ts:186` and
  `:190`: this is `ink.800` (defined at line 71) and it only ever appears
  inside `color-mix(in oklch, ... 4-9%, transparent)`, so it is never
  painted as a color, only as a seam alpha. The four shadow strings that
  contain it are reproduced verbatim in DESIGN.md's Shadows section. The
  detector builds its allowlist from DESIGN.md's YAML frontmatter only, so
  it cannot see the prose, and its alpha exemption parses the bare literal
  rather than the enclosing `color-mix()`. Reported on 2 of the 4
  occurrences; the palette definition at line 71 is never flagged.
- `overused-font` on `index.html:56`: Geist is the locked typeface for this
  product, named in both CLAUDE.md and DESIGN.md. The rule is a taste
  heuristic about a popular face, not a defect here.
