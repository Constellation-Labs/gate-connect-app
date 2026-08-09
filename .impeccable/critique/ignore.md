# Accepted detector findings

Findings the mechanical detector reports that are documented exceptions,
not drift. Critique runs drop matches silently.

Matching is by file + value, not line, so these survive the code moving.
Line numbers are recorded as of 2026-08-04 (re-verified after that day's
polish pass, which shifted the Onboarding ones by four lines) and are a
navigation aid only.

- `design-system-color` on `src/screens/Onboarding.tsx:35`, `:116` and
  `:120`: the `#000` literals sit inside `mask-image` alpha gradients,
  where the color channel is never painted; only opacity matters. Two fade
  the platform mockups on tour step 3; the third fades the step 2 hero,
  whose capture is cropped mid-card.
- `design-system-color` `#000` on `src/index.css:159`: the same case one
  layer down. `.gc-scroll-more` is the fold cue on the body's scroll
  container, and its `mask-image` gradient uses `#000` as the opaque stop.
  Nothing paints it; the gradient only decides where content fades. Added
  2026-08-04 with the layout pass.
- ~~`design-system-font-size` 27px on `src/screens/Onboarding.tsx:219`~~:
  retired 2026-08-07. The literal became the `gc-display` token when the type
  ramp moved to rem, so the rule no longer fires and the exception matched
  nothing. The heading itself is unchanged, and DESIGN.md's Typography
  section still documents why the tour may exceed the popover's ramp.
- `design-system-color` `oklch(0.165 0 0)` on `tailwind.config.ts:188` and
  `:192`: this is `ink.800` (defined at line 71) and it only ever appears
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
