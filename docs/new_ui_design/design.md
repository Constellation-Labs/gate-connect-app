# Design Tokens & Variables (Light Mode)
# Gate Connect

Design tokens for the Gate Connect project, extracted from the
shadcn_ui kit - Gate Connect library. Light mode values only.
Use these tokens in code to stay in sync with the Figma source of truth.

Variable collections:
1. TailwindCSS — Primitives (spacing, colors, sizing, opacity)
2. Theme — Design tokens (semantic colors, typography, shadows, radius)
3. Mode — Semantic aliases with light/dark switching
4. Custom — Layout and heading presets (Desktop)

Generated files:
- tokens.css — CSS custom properties
- tokens.json — Structured JSON for any tool or agent
- tailwind.tokens.js — Extends Tailwind config

---

## Fonts
- Sans: Geist
- Serif: Georgia
- Mono: Geist Mono

## Font Weights
thin: 100 | extralight: 200 | light: 300 | normal: 400
medium: 500 | semibold: 600 | bold: 700 | extrabold: 800 | black: 900

---

## Brand Colors — Blue Ribbon
50: #ebf6ff  |  100: #dbecff  |  200: #bedcff
300: #97c3ff  |  400: #6e9dff  |  500: #4c79ff
600: #294dff  |  700: #203de2  |  800: #1d37b6
900: #172563  |  950: #101738

## Semantic Colors (Light Mode)
background: gray-50 (#f9fafb)
foreground: gray-950 (#030712)
primary: blue-ribbon-700 (#203de2)
primary-foreground: gray-50 (#f9fafb)
secondary: gray-200 (#e5e7eb)
secondary-foreground: gray-900 (#111827)
destructive: red-600 (#dc2626)
destructive-foreground: red-50 (#fef2f2)
muted: gray-100 (#f3f4f6)
muted-foreground: gray-500 (#6b7280)
accent: gray-100 (#f3f4f6)
accent-foreground: gray-900 (#111827)
popover: white (#ffffff)
popover-foreground: gray-900 (#111827)
card: white (#ffffff)
card-foreground: gray-950 (#030712)
border: gray-200 (#e5e7eb)
input: gray-300 (#d1d5db)
ring: gray-400 (#9ca3af)

## Chart Colors (Project Override)
1 — Total messages: blue-400 (#60a5fa)
2 — Blocked: red-500 (#ef4444)
3 — Flagged: amber-400 (#fbbf24)
4 — Redacted: violet-500 (#8b5cf6)

## Sidebar Colors
background: gray-50  |  foreground: gray-950
primary: gray-950  |  primary-foreground: gray-50
accent: gray-100  |  accent-foreground: gray-900
border: gray-200  |  ring: gray-400

## Alpha Values
5: 0.95  |  10: 0.9  |  20: 0.8  |  30: 0.7  |  40: 0.6
50: 0.5  |  60: 0.4  |  70: 0.3  |  80: 0.2  |  90: 0.1

## Custom Colors
outline: rgba(163, 163, 163, 0.5)
outline-10: rgba(163, 163, 163, 0.1)
destructive-20: rgba(220, 38, 38, 0.2)
destructive-40: rgba(220, 38, 38, 0.4)

## Mode-Switching Custom Variables (light → dark)
These variables resolve differently per mode. Light values shown.
destructive→destructive/60: destructive (#dc2626)
destructive→destructive/70: destructive (#dc2626)
destructive→destructive/90: destructive (#dc2626)
destructive/40→destructive/60: rgba(220, 38, 38, 0.4)
outline/10→outline/20: rgba(163, 163, 163, 0.1)
blue-500→blue-600: blue-500 (#3b82f6)
background→input/30: background (#f9fafb)
background→calendar/30: background (#f9fafb)
input→input/80: input (#d1d5db)
accent→calendar/50: accent (#f3f4f6)
accent→input/50: accent (#f3f4f6)
ring→input-dark: ring (#9ca3af)
border→input-dark: border (#e5e7eb)

---

## Typography Scale
xs:   12px / 16px line-height
sm:   14px / 20px
base: 16px / 24px
lg:   18px / 28px
xl:   20px / 28px
2xl:  24px / 32px
3xl:  30px / 36px
4xl:  36px / 40px
5xl:  48px / 48px
6xl:  60px / 60px
7xl:  72px / 72px
8xl:  96px / 96px
9xl:  128px / 128px

## Headings (Desktop — 4. Custom collection)
XL:  Geist Bold 48px/48px tracking 0
LG:  Geist Bold 36px/40px tracking 0
MD:  Geist Bold 30px/36px tracking 0
SM:  Geist Bold 24px/32px tracking 0

---

## Spacing
0: 0px  |  px: 1px  |  0.5: 2px  |  1: 4px  |  1.5: 6px
2: 8px  |  2.5: 10px  |  3: 12px  |  3.5: 14px  |  4: 16px
5: 20px  |  6: 24px  |  7: 28px  |  8: 32px  |  9: 36px
10: 40px  |  11: 44px  |  12: 48px  |  14: 56px  |  16: 64px
20: 80px  |  24: 96px  |  28: 112px  |  32: 128px  |  36: 144px
40: 160px  |  44: 176px  |  48: 192px  |  52: 208px  |  56: 224px
60: 240px  |  64: 256px  |  72: 288px  |  80: 320px  |  96: 384px

## Border Radius
none: 0px  |  xs: 2px  |  sm: 6px  |  md: 8px
lg: 10px  |  xl: 14px  |  2xl: 16px  |  3xl: 24px
4xl: 32px  |  full: 9999px

## Border Width
0: 0px  |  default: 1px  |  2: 2px  |  3: 3px
4: 4px  |  5: 5px  |  6: 6px  |  7: 6px  |  8: 8px

## Opacity
0: 0  |  5: 0.05  |  10: 0.1  |  15: 0.15  |  20: 0.2
25: 0.25  |  30: 0.3  |  35: 0.35  |  40: 0.4  |  45: 0.45
50: 0.5  |  55: 0.55  |  60: 0.6  |  65: 0.65  |  70: 0.7
75: 0.75  |  80: 0.8  |  85: 0.85  |  90: 0.9  |  95: 0.95
100: 1.0

---

## Shadows
2xs: 0 1px 0 0 rgba(0,0,0,0.05)
xs:  0 1px 2px 0 rgba(0,0,0,0.05)
sm:  0 1px 3px 0 rgba(0,0,0,0.08), 0 1px 2px -1px rgba(0,0,0,0.08)
md:  0 4px 6px -1px rgba(0,0,0,0.08), 0 2px 4px -2px rgba(0,0,0,0.08)
lg:  0 10px 15px -3px rgba(0,0,0,0.08), 0 4px 6px -4px rgba(0,0,0,0.08)
xl:  0 20px 25px -5px rgba(0,0,0,0.1), 0 8px 10px -6px rgba(0,0,0,0.1)
2xl: 0 25px 50px -12px rgba(0,0,0,0.25)

## Inset Shadows
2xs: inset 0 1px 0 0 rgba(0,0,0,0.05)
xs:  inset 0 1px 1px 0 rgba(0,0,0,0.05)
sm:  inset 0 2px 4px 0 rgba(0,0,0,0.05)

## Drop Shadows
xs:  0 1px 1px rgba(0,0,0,0.05)
sm:  0 1px 2px rgba(0,0,0,0.15)
md:  0 3px 3px rgba(0,0,0,0.12)
lg:  0 4px 4px rgba(0,0,0,0.15)
xl:  0 9px 7px rgba(0,0,0,0.1)
2xl: 0 25px 25px rgba(0,0,0,0.15)

## Blur
xs: 4px  |  sm: 8px  |  md: 12px  |  lg: 16px
xl: 24px  |  2xl: 40px  |  3xl: 64px

---

## App Viewport
Fixed width: 1024px (desktop app, single breakpoint)

## Layout (Desktop — 4. Custom collection)
Container padding-x: 24px (spacing-6)
Section padding-y: 96px (spacing-24)
Section title gap XL: 24px | LG: 20px | MD: 20px | SM: 16px