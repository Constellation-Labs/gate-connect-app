import type { Config } from "tailwindcss";

/**
 * Constellation Gate design tokens — mirrors
 * `gate/packages/frontend-ui/src/cg/tokens.css` so Connect speaks the same
 * visual language as the dashboard and admin apps. Locked rules from the
 * design.md contract:
 * - ink-900 is primary (NOT blue).
 * - Neutral ink ramp uses OKLCH chroma 0 (no blue tint).
 * - Canvas surface is warm off-white #ecece7.
 * - Geist + Geist Mono.
 *
 * The `brand` indigo palette is kept ONLY to back the existing logo tile.
 * Do not introduce new indigo surfaces — use ink-900 for primary instead.
 *
 * EXCEPTION: the `gc` group below is the Gate Connect menu-bar popover palette
 * from the Claude Design handoff (the indigo-forward "Gate Connect — Prototype").
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

        // Ink — pure neutral grays (OKLCH chroma 0). Tonal anchor for the
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

        // Brand blue ramp — secondary accent, used sparingly. Primary
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
          ink: "#0f1222",
          "ink-2": "#2a2d3f",
          "ink-3": "#55596f",
          "ink-4": "#7a7f93",
          "ink-5": "#a1a6bb",
          navy: "#002a5f",
          success: "#2ecc71",
          warning: "#f39c12",
          error: "#e74c3c",
          menubar: "#181a30",
        },
      },
      boxShadow: {
        // Shadow-as-border — cards never get a solid 1px border per
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
      },
      borderRadius: {
        // Map default to cg-sm (6px) since most surfaces want the cg
        // everyday radius. Modal sits on 12px (`rounded-xl`).
        DEFAULT: "0.375rem",
        // Gate Connect popover radii (prototype --r-lg / --r-pill).
        "gc-lg": "12px",
        "gc-pill": "48px",
      },
    },
  },
  plugins: [],
} satisfies Config;
