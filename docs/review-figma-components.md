# Figma audit: shared components (`Components` + `Components / Sidenav`)

Branch `review/figma-audit`, read 2026-08-30. Figma file `9FrccCojXy0f8QD8Wm5Lln`.
12 MCP calls (2 of them errors, 1 a screenshot download).

## Summary

The rail matches the file almost exactly - `sidebar` (`437:161`) and
`sidebar-menu-item` (`434:128`) were re-read node by node and every padding,
radius, gap, colour and font in `Sidebar.tsx` lands on the drawn value, which is
what the 2026-08-28 commits `3de578bd` / `77589b28` claimed. Two things moved
that the build has not followed. **The `Switch` component set draws a 36x20
track with a 16px knob; `BaseSwitch` renders 32x18 with a 14px knob** - the
2026-08-26 measurement that produced those numbers was taken off an instance
scaled 1.125x, and 36/1.125, 20/1.125, 16/1.125 and 2/1.125 are exactly
32, 17.78, 14.22 and 1.78. And **`mono/eyebrow` now resolves to 8% tracking**,
which `Sidebar.tsx` hardcodes as `0.96px` while the shared `tracking-eyebrow`
token still says 1.2px, so one variable renders two ways across the app.
Structurally, **the `Components` page is now empty**: `topnav/menu` `116:17428`
is deleted, the `113:*` banners were already known deleted, and `nav/topbar`
`113:16763` is gone with them - four built components still cite those node ids
in their docstrings. The `Button` set `685:20855` could not be read at all this
pass (see below). New in the file: a `brand=moonshot` mark with no counterpart
in `BrandMark.tsx`.

## Findings

| What | Figma (node id + drawn value) | Built | Verdict |
| --- | --- | --- | --- |
| Switch track and knob | `408:14253` set, variants `408:14251` / `408:14252`, and as instanced in the row `434:128`: `w-[36px] h-[20px] rounded-[9999px] px-[2px] border-2` transparent, knob `size-[16px]` `bg-[base/background,#f9fafb]` under `shadow/lg`. On = `justify-end`, so knob x = 18; off = x = 2 | `base.tsx:120` `h-[18px] w-8`, knob `size-3.5` (14px), `translate-x-4` / `translate-x-[2px]`. Comment cites "32 x 17.78 with a 14.22 knob at 1.78 inset" | **DRIFT** |
| Switch off-track fill | `custom/outline` `#a3a3a380` (`437:161` vars) | `bg-neutral-400/50` | OK |
| Eyebrow tracking | `437:161` vars: `mono/eyebrow: Font(family: "Geist Mono", style: Medium, size: 12, weight: 500, lineHeight: 16, letterSpacing: 8)` = 0.96px at 12px | `tailwind.config.ts` `letterSpacing.eyebrow: "1.2px"` ("10% tracking") and `eyebrow-14: "1.4px"`. `Sidebar.tsx:246` overrides with `tracking-[0.96px]`; `metrics.tsx:165`, `metrics.tsx:423`, `Onboarding.tsx:368,371` still render 1.2 / 1.4px | **DRIFT** |
| `Components` page contents | `get_metadata` on `113:16762` returns the canvas with **zero children**. `116:17428` (`topnav/menu`) returns "node ID was not found in the file" | `Topbar.tsx:6-7` cites `nav/topbar` `113:16763` and `topnav/menu` `116:17428`; `banners.tsx:5-9` cites `banner/update` / `banner/routing` / `banner/partly-routing` / `banner/alert/*`; `base.tsx:46` cites `113:16788` / `113:16891`; `base.tsx:88` cites `113:16827`; `Sidebar.tsx:7` cites `nav/sidebar/overview` | **DRIFT** (docs) |
| `brand=moonshot` | `logo` `408:14159` and `logo-wrapper` `408:14180` each carry eight marks including `moonshot` (`433:120` / `433:123`, a newer id batch than openclaw's `430:*`) | `BrandMark.tsx:16-24` has seven `BrandName`s, no moonshot. `rg -i "moonshot|kimi"` over `src/` and `src-tauri/src` is empty | **DRIFT** (low; no such provider ships yet) |
| `brand=chatgpt` | Does **not** exist and never did - the set draws `openai` only | `BRAND_BY_SLUG` maps `chatgpt` and `chatgpt-apps` to `openai` | OK (corrects the plan's 2026-08-23 note, which listed ChatGPT among eight marks) |
| Rail width | `437:158/159/160` at 256x686 | `Sidebar.tsx:200` `w-[256px]` | OK |
| Rail sections | `sidebar-header` `688:23681` 256x57 (inner 232x32 on 12px pad); `sidebar-menu` `408:15634` 256x85 (items 232x28, 12px pad, 4px gap); both closed by a 1px edge | `Sidebar.tsx:202-219`, `border-b border-base-border p-3` on both | OK |
| Group list geometry | `408:15642`: groups at x=12, first at y=16, group gap 16, label-to-menu 8, row stride 48 on 44px rows | `Sidebar.tsx:224` `px-3 py-4`, `gap-4`, `gap-2`, `gap-1` | OK |
| Selected row | `434:128`: `p-[6px] rounded-[4px] gap-[16px]`, bg `base/sidebar-primary-foreground #f9fafb`, border `base/border #e5e7eb`, `shadow/xs` | `Sidebar.tsx:466-470` `p-1.5 rounded-control gap-4 bg-base-background border-base-border shadow-base-xs` (same hex) | OK |
| App tile | `logo-wrapper` `408:14175`: `p-[8px] rounded-[4px]`, border `rgba(255,255,255,0.24)`, `linear-gradient(180deg, rgba(255,255,255,0.24), rgba(0,0,0,0.24))` over black, 16px mark = 32px tile | `Sidebar.tsx:478-489`, identical | OK |
| Row name / status | Name Geist Medium 12/16, `base/primary` when selected. `StatusLabel` `434:130`: **10px** Geist Medium/16, `#d97706` phrase + `#6b7280` " - 2m ago" | `Sidebar.tsx:492-501` `text-base-xs` name, `text-base-2xs font-medium leading-4` status, `text-amber-600` / `text-base-muted-foreground` | OK |
| Eyebrow counter | `408:15645/15646` etc: `mono/body-12` Geist Mono **Regular** 12/16 ls 0, right-aligned at x=188, `base/muted-foreground`; reads `1 of 2` / `3 of 3` / `1 of 1` / `1 of 2` | `Sidebar.tsx:254-257`, derived from the rows | OK |
| Nav / header horizontal pad | `page=overview` draws 8 (icon at x=8, chevron right edge at 232-8) | 6px (`px-1.5` / `p-1.5`), taken from the settings and app variants | **RECORDED** (`Sidebar.tsx:284-285`, `:313-317`) |
| `OPENCODE` eyebrow over the OpenRouter row | `408:15690` "OPENCode" over `408:15691` "1 of 1" and a lone OpenRouter row, all three variants | Family label comes from `upstream_provider_name` = OpenRouter | **DESIGN BUG** (raised 2026-08-23, still unfixed in the file) |
| Quit menu entry | `751:38274` carries `Icon / SquareArrowOutUpRight` `751:38278` like the other three | `Topbar.tsx:132-140` omits the glyph, red ink, `LogOut` | **RECORDED** (`Topbar.tsx:35-38`) |
| Contact support | Drawn, `116:27231` | Omitted | **RECORDED** (`Topbar.tsx:18-27`) |
| `ModalWidth` docs | 536 is drawn (`694:33002`, `694:33340`) and typed | `Modal.tsx:18` header still reads "the file draws 480, 512, 544 and 600"; `CLAUDE.md` says "typed to those four" | **DRIFT** (docs, cosmetic) |
| `scroll-indicator` | `607:14444`: 6px x 154px rounded rect at x=249 inside the rail | `overflow-y-auto`, native scrollbar | OK |
| Icon stroke weight | Not read this pass | `Icon.tsx:456` `stroke = 1.75` (house value carried from `cg/ui.tsx`); Lucide's own default is 2 | unverified |

## Drawn with no counterpart in code

- **`brand=moonshot`** (`433:120` / `433:123`) - see table. The only genuinely new
  thing on the Sidenav page since the 2026-08-23 read.
- **`image 1`** (`408:15902`, 831x900) - the pasted screenshot of the retired
  `FamiliesPane`. Reference material the designer keeps, not a spec. No action.
  (It was 931x990 in the 2026-08-23 note, so it has been re-pasted since.)

## Built with no drawn source

- **`drifted` status.** `status-label` `434:136` draws exactly three states:
  `not-protected` (`434:130`), `protected` (`434:132`), `not-routed` (`434:134`).
  `Sidebar.tsx:116-121` carries a fourth, "Config drifted" in amber. Nothing in
  the set specifies its ink or phrasing, though the alert banner's drift copy is
  drawn elsewhere.
- **`MasterCard`** (`Sidebar.tsx:338`) and the **inventory states**
  (`Sidebar.tsx:396`). The three `sidebar` variants draw header, nav and group
  list and nothing else. Both are documented as deliberate at their definitions.
- `ErrorBanner`, `RecoveryBanner`, `ErrorDetails` (`banners.tsx:251`, `:305`,
  `:211`), `Skeleton`, `EmptyNote` - already recorded as undrawn.

## Could not read

- **`Button` component set `685:20855`.** Both `get_metadata` and
  `get_screenshot` return *"This is an invalid node selection. Ask the user to
  select a node from a visible page on the canvas."* That is a different error
  from the "node ID was not found in the file" that the deleted `116:17428`
  gives, so the node still resolves but sits on a page this MCP session cannot
  reach. The 2026-08-26 reproduction (two sizes at h36/10-12 and h32/8-12, the
  drop-plus-inset moulding behind `shadow-base-btn*`, the two `letterSpacing`
  tokens) is therefore **unverified this pass**, neither confirmed nor
  contradicted. Someone with the file open should select it and re-run, or paste
  a node URL.
- The Components page's former `nav/topbar` was not read because the page is
  empty; whatever replaced it lives inside the flow frames, which are out of
  this audit's scope.

## Ranked DRIFT

1. **Switch geometry.** 36x20 / 16px knob drawn, 32x18 / 14px built - an 11%
   undersize on the single most-repeated control in the app (every rail row, the
   master card, the alert banner). `base.tsx`'s own header docstring at line 88
   already says "36x20 track, 16px thumb"; only the code below it disagrees. Knob
   travel also changes: on sits at 18px, not 16.
2. **`letterSpacing.eyebrow` 1.2px vs the drawn 0.96px.** Cosmetic per site but
   it is one Figma variable rendered two different ways inside one build, with a
   hardcoded workaround at the only site that follows the file.
3. **Stale node ids in four docstrings.** No pixels move, but each is a pointer
   the next audit follows to a deleted node.
4. **`brand=moonshot` missing.** Inert until a Moonshot provider ships.
5. **`Modal.tsx` header and CLAUDE.md still say four dialog widths.** One line
   each.
