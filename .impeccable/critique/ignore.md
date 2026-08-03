# Accepted detector findings

Findings the mechanical detector reports that are documented exceptions,
not drift. Critique runs drop matches silently.

- `design-system-color` on `src/screens/Onboarding.tsx:105` and `:109`:
  the `#000` literals sit inside `mask-image` alpha gradients, where the
  color channel is never painted; only opacity matters.
- `design-system-font-size` 27px on `src/screens/Onboarding.tsx:182`: the
  onboarding tour renders in its own larger window and uses a 27px display
  heading, documented in DESIGN.md's Typography section. The popover ramp
  tops out at Panel Title (17px).
