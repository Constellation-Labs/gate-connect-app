import type { Config } from "tailwindcss";

/**
 * Constellation Gate design tokens - mirrors
 * `gate/packages/frontend-ui/src/cg/tokens.css` so Connect speaks the same
 * visual language as the dashboard and admin apps. Locked rules from the
 * design.md contract:
 * - ink-900 is primary (NOT blue).
 * - Neutral ink ramp uses OKLCH chroma 0 (no blue tint).
 * - Canvas surface is warm off-white #ecece7.
 * - Geist + Geist Mono.
 *
 * The `brand` indigo palette is kept ONLY to back the existing logo tile.
 * Do not introduce new indigo surfaces - use ink-900 for primary instead.
 *
 * EXCEPTION: the `gc` group below is the Gate Connect menu-bar popover palette
 * from the Claude Design handoff (the indigo-forward "Gate Connect - Prototype").
 * It is namespaced so it backs ONLY the Connect popover redesign and never
 * leaks into the shared cg ink/brand ramps above.
 */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      fontFamily: {
        sans: [
          "Geist",
          "ui-sans-serif",
          "system-ui",
          "-apple-system",
          "Segoe UI",
          "sans-serif",
        ],
        mono: [
          "Geist Mono",
          "ui-monospace",
          "SFMono-Regular",
          "Menlo",
          "monospace",
        ],
      },
      colors: {
        canvas: "#ecece7",

        // Legacy logo tile only.
        brand: {
          50: "#eef2ff",
          100: "#e0e7ff",
          200: "#c7d2fe",
          300: "#a5b4fc",
          400: "#818cf8",
          500: "#6366f1",
          600: "#4f46e5",
          700: "#4338ca",
          800: "#3730a3",
          900: "#312e81",
          950: "#1e1b4b",
        },

        // Ink - pure neutral grays (OKLCH chroma 0). Tonal anchor for the
        // whole UI; previously a Tailwind slate ramp with a blue cast.
        ink: {
          50: "oklch(0.985 0 0)",
          100: "oklch(0.96 0 0)",
          200: "oklch(0.91 0 0)",
          300: "oklch(0.82 0 0)",
          400: "oklch(0.68 0 0)",
          500: "oklch(0.53 0 0)",
          600: "oklch(0.38 0 0)",
          700: "oklch(0.26 0 0)",
          800: "oklch(0.165 0 0)",
          900: "oklch(0.09 0 0)",
          950: "oklch(0.045 0 0)",
        },

        // Brand blue ramp - secondary accent, used sparingly. Primary
        // affordances stay on the ink ramp.
        blue: {
          50: "oklch(0.97 0.02 268.85)",
          100: "oklch(0.94 0.04 268.85)",
          200: "oklch(0.89 0.075 268.85)",
          300: "oklch(0.81 0.13 268.85)",
          400: "oklch(0.7 0.18 268.85)",
          500: "oklch(0.58 0.215 268.85)",
          600: "oklch(0.47 0.232 268.85)",
          700: "oklch(0.345 0.224 268.85)",
          800: "oklch(0.275 0.175 268.85)",
          900: "oklch(0.215 0.13 268.85)",
          950: "oklch(0.145 0.085 268.85)",
        },

        success: {
          50: "oklch(0.982 0.018 155.826)",
          100: "oklch(0.962 0.044 156.743)",
          200: "oklch(0.925 0.084 155.995)",
          300: "oklch(0.871 0.15 154.449)",
          400: "oklch(0.792 0.209 151.711)",
          500: "oklch(0.723 0.219 149.579)",
          600: "oklch(0.627 0.194 149.214)",
          700: "oklch(0.527 0.154 150.069)",
          800: "oklch(0.448 0.119 151.328)",
          900: "oklch(0.393 0.095 152.535)",
          950: "oklch(0.266 0.065 152.934)",
        },
        warning: {
          50: "oklch(0.987 0.022 95.277)",
          100: "oklch(0.962 0.059 95.617)",
          200: "oklch(0.924 0.12 95.746)",
          300: "oklch(0.879 0.169 91.605)",
          400: "oklch(0.828 0.189 84.429)",
          500: "oklch(0.769 0.188 70.08)",
          600: "oklch(0.666 0.179 58.318)",
          700: "oklch(0.555 0.163 48.998)",
          800: "oklch(0.473 0.137 46.201)",
          900: "oklch(0.414 0.112 45.904)",
          950: "oklch(0.279 0.077 45.635)",
        },
        danger: {
          50: "oklch(0.971 0.013 17.38)",
          100: "oklch(0.936 0.032 17.717)",
          200: "oklch(0.885 0.062 18.334)",
          300: "oklch(0.808 0.114 19.571)",
          400: "oklch(0.704 0.191 22.216)",
          500: "oklch(0.637 0.237 25.331)",
          600: "oklch(0.577 0.245 27.325)",
          700: "oklch(0.505 0.213 27.518)",
          800: "oklch(0.444 0.177 26.899)",
          900: "oklch(0.396 0.141 25.723)",
          950: "oklch(0.258 0.092 26.042)",
        },

        // ── New app UI (Figma "Gate Connect", file 9FrccCojXy0f8QD8Wm5Lln). ──
        // Names mirror the Figma variables 1:1 (`--base/card` -> `base.card`)
        // so a token can be traced back to the design without guessing. The
        // design is shadcn-flavoured on the default Tailwind palette, so
        // neutral/amber/green/gray come from Tailwind itself (this config uses
        // `extend`, so those ramps are untouched and already exact matches:
        // neutral-900 #171717, neutral-500 #737373, amber-600 #d97706,
        // green-600 #16a34a, gray-100 #f3f4f6).
        //
        // This supersedes the `gc` group below and the ink-primary rule in the
        // header note: the design makes blue-ribbon the primary. `gc` stays
        // until the popover screens are migrated off it.
        base: {
          card: "#ffffff",
          background: "#f9fafb",
          border: "#e5e7eb",
          input: "#d1d5db",
          primary: "#203de2",
          "muted-foreground": "#6b7280",
        },

        // Primary ramp. Only the stops the design actually specifies:
        // 700 backs switches, active nav and links; 800 is the "Gate" wordmark
        // and the update banner's gradient start; 900 is its gradient end.
        // Fill in the rest from Figma rather than interpolating.
        "blue-ribbon": {
          700: "#203de2",
          800: "#1d37b6",
          900: "#172563",
        },

        // Messages chart series (Figma legend swatches, sampled individually).
        // Named for the series rather than the hue for two reasons: the meaning
        // is what call sites care about, and three of the four are Tailwind
        // defaults while `blue` is REDEFINED as an OKLCH ramp further up this
        // file - so `bg-blue-400` would silently render the wrong colour.
        // Levels are the design's own and are not uniform: 400, 400, 400, 500.
        chart: {
          messages: "#60a5fa", // tailwind blue/400
          blocked: "#f87171", // tailwind red/400
          flagged: "#fbbf24", // tailwind amber/400
          redacted: "#a855f7", // tailwind purple/500
        },

        // ── Gate Connect popover palette (Claude Design handoff). ──
        // Indigo-forward; scoped to the Connect popover only. See header note.
        gc: {
          accent: "#3e4fea",
          "accent-ink": "#2a38cb",
          "accent-wash": "rgba(62,79,234,0.08)",
          "accent-wash-2": "rgba(62,79,234,0.14)",
          page: "#f4f5f9",
          surface: "#ffffff",
          subtle: "#f8f9fc",
          sunken: "#eef0f6",
          highlight: "#f6ffe3",
          line: "#e8eaef",
          "line-strong": "#d4d7e3",
          // The switch's off track. SC 1.4.11 wants 3:1 for a component state
          // indicator, and it has to clear that on every surface the switch
          // lands on - not just white. #8b91a6 measured 3.13:1 on white but
          // 2.98:1 on `subtle`, which is what an expanded or hovered member row
          // uses, so the most-used control in the app failed exactly where the
          // user was interacting with it. #868c9e is 3.36:1 on white and
          // 3.19:1 on subtle.
          "switch-off": "#868c9e",
          ink: "#0f1222",
          "ink-2": "#2a2d3f",
          "ink-3": "#55596f",
          "ink-4": "#7a7f93",
          "ink-5": "#a1a6bb",
          navy: "#002a5f",
          success: "#2ecc71",
          // Text-on-wash partners for the status colors: dark enough to hold
          // WCAG AA (4.5:1) at pill size on their washes over white.
          "success-deep": "#177a42",
          "success-wash": "rgba(46,204,113,0.14)",
          warning: "#f39c12",
          // Same role as success-deep / error-deep: dark enough to carry meaning
          // on its own wash. The status dots use the deep variants so the dot
          // clears SC 1.4.11's 3:1 instead of relying on the label beside it.
          "warning-deep": "#a25f02",
          "warning-wash": "rgba(243,156,18,0.12)",
          error: "#e74c3c",
          // Text-weight red, mirroring success-deep: #e74c3c is 3.8:1 on
          // white, which is fine for a dot or an icon and short of AA for the
          // words next to it. Dots and washes keep `error`; anything the user
          // reads uses this.
          "error-deep": "#c0392b",
          "error-wash": "rgba(231,76,60,0.12)",
          menubar: "#181a30",
        },
      },
      boxShadow: {
        // Shadow-as-border - cards never get a solid 1px border per
        // design.md §1 / §8. The three-stop stack carries the seam:
        // 1) hairline color outline
        // 2) bottom inset stroke (gives the row a "card" feel)
        // 3) ambient drop for elevation
        border:
          "0 0 0 1px color-mix(in oklch, oklch(0.165 0 0) 6%, transparent), 0 1px 0 0 color-mix(in oklch, oklch(0.165 0 0) 6%, transparent), 0 2px 4px -1px color-mix(in oklch, oklch(0.165 0 0) 4%, transparent)",
        "border-hover":
          "0 0 0 1px color-mix(in oklch, oklch(0.165 0 0) 9%, transparent), 0 1px 0 0 color-mix(in oklch, oklch(0.165 0 0) 8%, transparent), 0 2px 6px -1px color-mix(in oklch, oklch(0.165 0 0) 6%, transparent)",
        popup:
          "0 0 0 1px color-mix(in oklch, oklch(0.165 0 0) 4%, transparent), 0 4px 16px -2px color-mix(in oklch, oklch(0.165 0 0) 8%, transparent)",
        popover:
          "0 0 0 1px color-mix(in oklch, oklch(0.165 0 0) 8%, transparent), 0 12px 32px -8px color-mix(in oklch, oklch(0.165 0 0) 22%, transparent), 0 4px 12px -2px color-mix(in oklch, oklch(0.165 0 0) 10%, transparent)",
        // Gate Connect popover card drop (prototype --shadow-md).
        "gc-md": "0 2px 4px rgba(10,10,10,0.04), 0 8px 24px rgba(10,10,10,0.06)",

        // New app UI (Figma `shadow/2xs`, `shadow/xs`, `shadow/lg`). Namespaced
        // rather than overriding Tailwind's `shadow-lg`, which the popover
        // screens still use.
        // The design names these on Tailwind v4's scale, where everything
        // shifted one step down (v4 `shadow-xs` is v3 `shadow-sm`). This repo is
        // on v3.4, so Figma `shadow/sm` is v3's DEFAULT `shadow`, not `shadow-sm`.
        // Spelled out here so the mapping does not have to be re-derived.
        "base-2xs": "0 1px 0 0 rgba(0,0,0,0.05)",
        "base-xs": "0 1px 2px 0 rgba(0,0,0,0.05)",
        "base-sm": "0 1px 3px 0 rgba(0,0,0,0.1), 0 1px 2px -1px rgba(0,0,0,0.1)",
        "base-lg":
          "0 10px 15px -3px rgba(0,0,0,0.08), 0 4px 6px -4px rgba(0,0,0,0.08)",
      },
      borderRadius: {
        // Map default to cg-sm (6px) since most surfaces want the cg
        // everyday radius. Modal sits on 12px (`rounded-xl`).
        DEFAULT: "0.375rem",
        // Gate Connect popover radii (prototype --r-lg / --r-pill).
        "gc-lg": "12px",
        "gc-pill": "48px",
        // New app UI: 4px on inputs, nav items and icon tiles. 8px rows use
        // Tailwind's `rounded-lg`, which is already 8px.
        base: "4px",
      },
      letterSpacing: {
        // New app UI `mono/eyebrow`: Geist Mono Medium 12/16 at 10% tracking.
        eyebrow: "1.2px",
        // `mono/label-12`: same face at 6% - the action pills (BLOCK/FLAG/REDACT).
        label: "0.72px",
        // `heading/20`: Geist Medium 20/24 at -1% - pane titles and captions.
        heading: "-0.2px",
      },
      fontSize: {
        // The popover's type ramp, in rem against a 16px root.
        //
        // These were 138 `text-[Npx]` literals and zero rem, which is why
        // nothing in the app could be made larger: px is absolute, so a user
        // who raises their text size saw no change at all (measured: root
        // 16px -> 32px left a 13.5px heading at 13.5px). Expressed in rem, the
        // whole ramp scales from one variable, which is what `useTextScale`
        // drives.
        //
        // Eleven steps, not the six DESIGN.md names, because eleven is what the
        // code actually uses and this pass must not change how anything looks at
        // 100%. The six canonical names keep their DESIGN.md meaning; the five
        // in-between steps are named for their role and marked here as the
        // consolidation candidates they are. Collapsing 11 onto 6 changes
        // appearance and belongs to a typesetting pass, not to this one.
        //
        // Divide by 16 to read the px value back: 0.84375rem * 16 = 13.5px.
        //
        // Font size only, deliberately no line-height tuple. Tailwind would emit
        // `line-height` alongside `font-size` for each of these, which would
        // change computed leading at every call site that does not already carry
        // a `leading-*` utility. The existing leading is correct and this pass is
        // a units change, not a typesetting one.
        "gc-label": "0.65625rem", // 10.5px - mono section labels, identifiers
        "gc-micro": "0.6875rem", // 11px   - row exception lines
        "gc-caption": "0.71875rem", // 11.5px - captions, hints, banner copy
        "gc-caption-lg": "0.75rem", // 12px   - inline banner actions
        "gc-body-sm": "0.78125rem", // 12.5px - takeover body, inline links
        "gc-body-md": "0.8125rem", // 13px   - row titles, member names
        "gc-body": "0.84375rem", // 13.5px - buttons, inputs, sentence copy
        "gc-title-sm": "0.875rem", // 14px   - secondary headings
        "gc-title": "0.90625rem", // 14.5px - panel titles, wordmark
        "gc-panel-title": "1.0625rem", // 17px   - takeover headings
        "gc-display": "1.6875rem", // 27px   - onboarding window only

        // New app UI. In rem for the same reason as the ramp above: px would
        // opt these out of `useTextScale` entirely.
        "base-2xs": "0.625rem", // 10px - app row status line
        "base-xs": "0.75rem", // 12px - label/12 and the mono eyebrow
      },
    },
  },
  plugins: [],
} satisfies Config;
