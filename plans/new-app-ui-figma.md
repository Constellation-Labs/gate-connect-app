# New app UI: matching the Gate Connect Figma

Source: https://www.figma.com/design/9FrccCojXy0f8QD8Wm5Lln/Gate-Connect
File key `9FrccCojXy0f8QD8Wm5Lln`, single page `Components` (113:16762).

## What the Figma actually contains

The file has five pages, only one of which the MCP `get_metadata` call
reported: `Components`, `Flows` (containing `Overview`, `Settings`, `App`),
`Sandbox`, and `Icons`.

- **Overview** - dashboard: stat tiles, Messages bar chart, Policies table,
  Token savings table. Annotated **App dimensions: 1024x720px**.
- **Settings** - fully transcribed below.
- **App** - a per-app detail view, two sections (`App / Main screens` and
  `App / Select a model`). Selecting an app in the sidebar opens its own pane
  with per-app stats, a Messages chart, a model picker (App default vs Gate
  model) and a Recent activity table. This is a third screen type beyond
  Overview and Settings.
- **Sandbox** - scratch. Layers are named `Screenshot 2026-05-14 at 1:26:15 PM`,
  `overview-dimensions`, `mask`, plus duplicates. **Not a spec source; ignore.**
- **Icons** - glyph library.

### Settings screen (transcribed from `Flows / Settings`)

Sections, each a card with 8px radius; rows are label + value + trailing action.

| Section | Row | Value | Action |
| --- | --- | --- | --- |
| Device | Device | MacBook Pro | Rename |
| | Install ID | `gc_a1b2c3d4` | Copy |
| Account | Login ID | jdoe@acme.com | - |
| | Gate plan | Free | Upgrade plan |
| Connection | Gateway | Managed by Gate | - |
| | API key | `sk-gw*****************` | Replace key |
| | Active session | - | **Disconnect** (destructive) |
| Startup | Launch at login<br>"Keeps routing on after restart" | On | switch |
| | Notifications<br>"Alert me when a request is blocked or flagged" | On | switch |
| About | Tutorial | - | Replay tutorial |
| | Version | v0.1.4 | Check for updates |
| Danger zone | Reset Gate Connect<br>"Turn routing off, disconnect tools, remove this account or key, and start setup again." | - | **Review reset** (destructive) |

"Danger zone" is a red section heading; its card carries a red tint and border.

### Components page

Only component/state frames.

| Node | Name | Size |
| --- | --- | --- |
| 113:16763 | `nav/topbar` | 1024x48 |
| 113:16775 | `banner/update` | 1024x48 |
| 113:16786 | `banner/routing` | 1024x48 |
| 113:16889 | `banner/partly-routing` | 1024x48 |
| 113:16898 | `banner/alert/multiple-apps` | 726x68 |
| 113:16919 | `banner/alert/single-app` | 726x68 |
| 113:16794 | `nav/sidebar/overview` | 250x573 |
| 121:33326 | `nav/sidebar/settings` | 250x573 |
| 121:33421 | `nav/sidebar/settings` (row hover) | 250x573 |
| 116:17428 | `topnav/menu` | 224x114 |

## Reading the file without MCP quota

The Figma MCP server allows only **6 read calls per month** on a View seat.
When exhausted, read the file through the browser instead. Two things that do
NOT work: mouse-wheel `scroll` never reaches the WebGL canvas, and clicking the
canvas selects a nested child so `shift+2` zooms to the wrong thing.

What works: **click the frame in the Layers panel, then press `shift+2`**
(zoom to selection). Switch pages by clicking them in the Pages list, then
`shift+1` to fit. Allow ~10s after a page switch; the first
`Page.captureScreenshot` after navigation often times out at 30s, so wait and
retry once.


## The three forks that gated implementation (all RESOLVED)

Decided 2026-08-14: **the Figma wins on all three.** The app becomes a
1024x720 window, adopts the shadcn-flavoured token names, and CLAUDE.md's
Aesthetic Direction gets rewritten rather than obeyed. Detail kept below for
the record.

### 1. Window model: popover → window

`src-tauri/tauri.conf.json` today is a 380×620 `resizable: false`,
`decorations: false`, `alwaysOnTop: true`, `skipTaskbar: true` menubar
popover. The design is 1024 wide with **macOS traffic lights** at
top-left (113:16764, three 12px ellipses) and a **Minimize2** button at
top-right (113:16774).

That is a regular application window, not a popover. Deciding this
rewrites:

- `tauri.conf.json` - width, `resizable`, `alwaysOnTop`, `skipTaskbar`
- `src-tauri/src/lib.rs` - transparent window + CALayer `cornerRadius`
- `src/components/LinuxTitleBar.tsx` - already draws its own chrome
- `pinPopover` / `unpinPopover` in `src/lib/api.ts` - popover-only concepts

Open sub-question: what does Minimize2 do? Plausibly "collapse back to
the menubar popover", which would mean keeping *both* shells.

### 2. Design system: bespoke `cg/` → shadcn/ui

The returned tokens are shadcn CSS variable names on the default Tailwind
palette:

```
--base/card        white
--base/background  #f9fafb
--base/border      #e5e7eb
--base/input       #d1d5db
--base/primary     #203de2   ← blue-ribbon/700
--base/muted-foreground #6b7280
tailwind-colors/neutral/{50,600,900}
tailwind-colors/amber/600  #d97706   "Not protected"
green                      #16a34a   "Protected"
```

The Button frames link to `ui.shadcn.com/docs/components/button`. The repo
has **no** shadcn, radix, cva, tailwind-merge, clsx, or lucide dependency,
and runs a bespoke OKLCH ink system in `tailwind.config.ts` + `.impeccable/design.json`.

### 3. CLAUDE.md conflicts

The project's own `CLAUDE.md` Aesthetic Direction contradicts the design
on four locked points:

| CLAUDE.md says | Figma does |
| --- | --- |
| "Primary is ink-900, **never blue**" | `--base/primary #203de2` on active nav, switches, links |
| "Brand indigo is reserved for the logo glyph only" | Wordmark "Gate" in `#1d37b6`, blue switches |
| "**Shadow-as-border.** No 1px solid borders on cards" | 1px `--base/border` on sidebar, org switcher, topbar, nav item |
| "12px modal radius **LOCKED**" | 4px and 8px radii throughout |
| Status pill is "the single most important pixel" | Per-app toggle **Switch**, pill gone |
| Light surface, ink-on-paper | `banner/update` is dot-matrix navy |

If the Figma wins, CLAUDE.md's Aesthetic Direction section needs rewriting
in the same PR, otherwise every future session fights the design.

## Copy / model changes

- "Protected" / "Not protected" replaces the connected/routed vocabulary.
- Relative timestamps per app: "Protected - 2m ago", "- 25s ago".
- Sidebar counter `PROTECTED APPS  4/4` (mono eyebrow, 12px, tracking 1.2px).
- Banner right-rail: `Routing · 2 of 4 Apps` / `Partly routed · 0 of 4 Apps`.
- Alert copy: "Codex isn't protected / It's config changed outside Gate, so
  its traffic isn't routed. Reconnect to restore protection."
- `topnav/menu`: Visit dashboard, Contact support, Read Gate docs.

## Reuse inventory

Already in `src/components/gc/`, reusable:

- `ui.tsx` - `Button`, `IconButton`, `Switch`, `CardButton`, `SectionLabel`
- `Icon.tsx` - 16 glyphs: shieldCheck, layers, settings, chevronRight,
  caretRight, chevronLeft, cube, key, copy, eye, eyeOff, check, refresh,
  trash, info, x, plus, search, logOut, book
- `PopHeader.tsx`, `ConstellationHexMark.tsx`

New glyphs the design needs (lucide names): `UsersRound`, `ChevronsUpDown`,
`LayoutDashboard`, `Settings2`, `ShieldBan`, `TriangleAlert`, `Headset`,
`BookOpenText`, `SquareArrowOutUpRight`, `Ellipsis`, `Minimize2`.

## Navigation rewrite

`src/App.tsx` uses a depth-ranked push/pop stack (`Screen` union +
per-screen `depth`) because "the popover is one room". The design replaces
that with a **persistent sidebar + swapped content pane**. The depth
metaphor and the slide animations become dead code.

Screens affected: `Home.tsx` (972 lines), `Settings.tsx` (825),
`GroupMembers.tsx` (660) - plus their tests (`Home.test.tsx` 822 lines,
`Settings.test.tsx` 310, `GroupMembers.test.tsx` 406) will break on any
DOM restructure.

## Verified token values

Sampled from Figma's properties panel (readable on a View seat, no MCP quota).
The design uses **Tailwind's default palette** throughout, so most of these need
no new tokens - the config uses `extend`, leaving `neutral`/`amber`/`green`/`gray`
intact.

Primary ramp (needs config entries, `blue-ribbon`):

| Stop | Hex | Used by |
| --- | --- | --- |
| 700 | `#203DE2` | switches, active nav, links |
| 800 | `#1D37B6` | "Gate" wordmark, update banner gradient start |
| 900 | `#172563` | update banner gradient end |

Status tile - one shape, two palettes. 28x28, `rounded-base` (4px), 4px padding,
1px border, vertical gradient from the 50 stop to the 200 stop, icon stroked at
the 700 stop:

| Variant | Gradient | Border | Icon |
| --- | --- | --- | --- |
| protecting | `green/50 #F0FDF4` -> `green/200 #BBF7D0` | `green/300 #86EFAC` | `green/700 #15803D` |
| partly routing | `amber/50 #FFFBEB` -> `amber/200 #FDE68A` | `amber/300 #FCD34D` | `amber/700` |

Banner geometry:

| Banner | Size | Padding | Fill | Bottom border |
| --- | --- | --- | --- | --- |
| `banner/update` | 1024 x hug 48 | 12 / 16 | gradient 800 -> 900 + dot matrix | 1px |
| `banner/routing` | 1024 x 48 | 8 / 16 | `base/card` #FFFFFF | 1px `base/border` #E5E7EB |
| `banner/partly-routing` | 1024 x 48 | 8 / 16 | `base/card` | 1px `base/border` |

All are `flex-row`, `justify-between`.

## Implementation plan

### Phase 1 - Token layer (DONE)

`tailwind.config.ts`: `base.*` semantic colors, `blue-ribbon` ramp,
`shadow-base-{2xs,xs,lg}`, `rounded-base`, `tracking-eyebrow`,
`text-base-{2xs,xs}` in rem. Add `blue-ribbon.900` when the update banner lands.

### Phase 2 - Sidebar (DONE)

`src/components/gc/Sidebar.tsx` - `Sidebar`, `SidebarView`, `SidebarApp`,
plus private `OrgSwitcher`, `NavItem`, `AppRow`, `AppSwitch`.
`Icon.tsx` gained `usersRound`, `chevronsUpDown`, `layoutDashboard`, `settings2`.

### Phase 3 - Banners (DONE)

New `src/components/gc/banners/`:

- `StatusTile` - the shared 28x28 gradient tile, variant `protecting | partly`.
- `RoutingBanner` - white, status tile + message, right rail
  `Routing · 2 of 4 Apps` (green) or `Partly routed · 0 of 4 Apps` (amber).
  Replaces nothing; this is new.
- `UpdateBanner` - the navy gradient + dot matrix, "Update available - v0.5.0",
  outline `Update` button, dismiss X. Retargets the slim-banner branch already
  inside `UpdatePanel.tsx` (which today also owns a startup takeover).
- `AlertBanner` - amber card, 36x36 tile, `TriangleAlert`, title + body, a
  switch and a dismiss X. Two variants: `single-app`, and `multiple-apps` which
  adds prev/next chevrons to page between apps.

New glyphs needed: `shieldBan`, `triangleAlert`, `refreshCw`. `shieldCheck` and
`x` already exist.

### Phase 4 - Topbar and window chrome (DONE)

`src/components/gc/Topbar.tsx` - `Topbar`, `TopnavMenu`, `TopnavAction`, plus
private `WindowControls`, `TrafficLight`, `OutlineIconButton`. Glyphs added:
`ellipsis`, `minimize2`, `squareArrowOutUpRight`, `headset`, `bookOpenText`.
Reuses the existing `ConstellationHexMark` at 24px - its aspect ratio (0.8628)
matches the Figma lockup's 20.708x24 exactly.

**Window controls are the system's, not ours.** The design draws its own
traffic lights (and draws them green / yellow / red, the reverse of macOS);
decided 2026-08-14 to drop them and let the OS render real ones. The topbar
keeps a 60px `aria-hidden` spacer where they sat, so the brand lockup holds its
drawn position (504px of 1024) and never slides underneath them.

This makes the Phase 9 window config load-bearing, because today
`tauri.conf.json` sets `decorations: false` and no controls exist at all:

- **macOS** - `decorations: true` with `titleBarStyle: "Overlay"` and
  `hiddenTitle: true`, so the OS paints the traffic lights over our topbar.
- **Windows** - system controls sit top **right**, not left, where our Ellipsis
  and Minimize2 buttons already are. The 60px left spacer is wasted there and
  the right cluster will collide. Needs a platform-conditional layout.
- **Linux** - `LinuxTitleBar.tsx` already draws its own chrome; reconcile.

### Phase 5 - Overview pane (DONE)

`src/components/gc/Overview.tsx` - `Overview` plus the types the 24-hour
backend will need to satisfy (`OverviewStats`, `MessagesBucket`, `Policy`,
`PolicyAction`, `Saving`); private `StatTiles`, `Stat`, `MessagesChart`,
`PolicyTable`, `SavingsTable`, `StatusPill`, `ManageLink`. `Card` went into
`base.tsx` since the Settings pane uses the same surface.

Chart is hand-rolled from stacked divs, no chart library: the mark is four
rectangles, and hand-rolling keeps the series on design tokens. Bars are the
design's 20px and distribute across the card, so 24 buckets or 6 both work.

Verified against Figma: pane `gray/100`, card Fill 726 / r8 / `#E5E7EB` /
`shadow/sm`, bars 20px, legend swatches 12x12 r2.

Measured values worth keeping:

- pane padding 24, gap between cards 16
- stat tiles are one card of three columns divided by `border-l`
- action pill label is `mono/label-12` (Geist Mono Medium 12/16, 6% tracking)
- pane title and section captions are `heading/20` (Geist Medium 20/24, -1%)
- window frame radius is 8px, not the popover's 12px

Two values were **inferred, not sampled**: the action pill backgrounds
(`red/amber/purple-100`) and the On pill (`green-100`). Text sits at the 900
level in Figma and 100/900 is the standard Tailwind badge pairing, matching the
50 -> 200 tile gradients, but they are worth a spot check.

### Phase 6 - Settings pane (DONE)

`src/components/gc/SettingsPane.tsx` - `SettingsPane` plus `SettingsSection`,
`SettingsRow`, `SettingsAction`; private `Row`, `ActionButton`. Named
`SettingsPane`, not `Settings`, so it does not read as a drop-in for
`screens/Settings.tsx`, which stays until the shell swap.

Rows are uniform enough across all six sections that the pane takes a
declarative section/row model rather than a prop per field. Reuses `Card` and
`BaseSwitch`.

Button vocabulary, read off the design:

- **outline + blue label** for Rename, Copy, Upgrade plan, Replace key,
  Replay tutorial, Check for updates
- **filled red** for the two destructive ones, Disconnect and Review reset
- Startup rows pair an "On"/"Off" label with the switch
- values that are identifiers render mono: install ID, API key, version

Nine glyphs added: `monitorSmartphone`, `idCard`, `user`, `receipt`, `globe`,
`link`, `power`, `bell`, `codeXml`.

The danger card's `red-50` fill and `red-200` border are **inferred** from the
red section heading, not sampled.

Still outstanding: `screens/Settings.tsx` (825 lines) and its 310-line test are
untouched - they get retired in Phase 9, not here.

### Phase 7 - App detail pane (DONE)

`src/components/gc/AppPane.tsx` - `AppPane` plus `ModelChoice`, `GateModel`,
`ActivityEntry`, `ActivityStatus`, `ActivitySecurity`; private `ModelSelection`,
`ModelOption`, `InfoRow`, `RecentActivity`, `Pill`.

The stat card and Messages chart are identical to the Overview's, so they moved
to `src/components/gc/metrics.tsx` (`StatTiles`, `MessagesChart`, `UsageStats`,
`MessagesBucket`) and both panes import them. `OverviewStats` was renamed
`UsageStats` in the move - nothing consumed it yet.

Structure: header (app tile, name, protected status, On + switch), stat tiles,
Messages chart, Model selection, Recent activity.

Model selection is a two-option radio group (App default / Gate model), the
selected one carrying a `base-primary` border and a `circleCheck`. Below it,
"Current Gate model" with vendor + mono model id and a `Change model` button,
then Gate credits with `Add credits`.

Recent activity is Time / Status / Security / Conversation / Action, with mono
uppercase pills - status `success | error`, security
`allow | flagged | redacted | blocked` - and a `View` button per row.

Glyphs added: `circleCheck`, `creditCard`.

The **Gate model option's icon is a placeholder** (`layers`). The design draws a
three-node cluster that reads as brand art rather than a lucide glyph;
`ModelOption` takes a `ReactNode` so the real mark can be passed without
inventing one.

### Phase 8 - Modals (DONE)

`src/components/gc/Modal.tsx` - `Modal`, `ModalSubject`, `ModalNote`,
`ModalOption`, plus `ModalTone`, `ModalButton`, `PillTone`.

All seven dialogs in the file are drawn from one template, so this is one
component with slots rather than seven near-copies:

    tone tile + title + subtitle
    optional subject card (icon, name, description, status pill)
    optional note block
    right-aligned secondary / primary buttons

600px wide, 12px radius, centred over a scrim. Reuses the repo's existing
`useFocusTrap`, pointing initial focus at the *secondary* button whenever the
primary is destructive - the hazard that hook's own comment calls out.

The seven, and how they map onto the template:

| Dialog | Tone | Subject pill | Buttons |
| --- | --- | --- | --- |
| Switch organization | neutral | (radio list instead) | Cancel / Switch organization |
| Organization switched | success | - | Done |
| Review `<app>` configuration | warning | `DETECTED` amber | Keep existing config / Replace config and protect |
| Apply changes to running apps | warning | `OPEN` green | Close affected apps / I will reopen later |
| Close affected apps now? | warning | `OPEN` green | Go back / **Close `<app>`** (red) |
| Change is ready | success | - | Done |
| Use a Gate model for `<app>`? | neutral | `PAYG` | Keep App default / Use Gate credits |

Note that "Apply changes to running apps" makes the *less* destructive option
primary: `I will reopen later` is the filled button, `Close affected apps` the
outline one.

`src/components/gc/dialogs.tsx` holds the seven concrete dialogs -
`SwitchOrganizationDialog`, `OrganizationSwitchedDialog`, `ReviewConfigDialog`,
`ApplyChangesDialog`, `CloseAppsDialog`, `ChangeReadyDialog`,
`UseGateModelDialog`. Copy lives with them rather than in the shell, so it stays
next to the design it came from and the shell supplies only names and handlers.

`ModalSubject` grew a `variant`: `subject` names a thing and describes it (bold
name over grey detail), `identity` inverts that for the model row where the
vendor is a quiet label above a mono model id. `Modal` grew `subtitleTone`,
because the Gate-model dialog is the one place the design states its cost
consequence in `base-primary` rather than grey.

`OrganizationSwitchedDialog`'s copy was read at low resolution and is the least
certain of the seven.

### Phase 9 - Shell swap (PARTIALLY DONE)

`src/components/gc/AppShell.tsx` composes chrome, banners, sidebar and a dialog
slot. The open pane arrives as `children`: routing is a one-line switch on
`view.kind` at the call site, and folding it in would force the shell to accept
every pane's data. The dialog is a slot for the same reason - the shell owns
*that* a dialog covers the window, the caller owns which one.

Nothing below is done yet, and all of it changes what ships:

- `tauri.conf.json`: 380x620 non-resizable -> 1024x720; drop `alwaysOnTop`
  and `skipTaskbar`; decide `resizable`. Turn `decorations` back on so the OS
  draws the window controls (see Phase 4 for the per-platform detail) - without
  this the app has no close/minimise affordance at all.
- `src-tauri/src/lib.rs`: the transparent-window + CALayer `cornerRadius` work
  is popover-specific.
- `App.tsx`: replace the depth-ranked push/pop `Screen` stack with
  sidebar + content pane. The `depth` map and slide animations become dead code.
- Retire `pinPopover` / `unpinPopover` from `lib/api.ts`.

### Phase 10 - Tests and docs

`Home.test.tsx` (822), `Settings.test.tsx` (310), `GroupMembers.test.tsx` (406)
will all break on DOM restructure. Rewrite CLAUDE.md's Aesthetic Direction in
the same PR.

## Open questions

1. **`PROTECTED APPS 4/4`** reads 4/4 in every frame while switch states vary,
   sitting above two off switches with "2 of 4 Apps" beside it. Either it means
   something other than protected-count, or it was never bound. The sidebar
   currently computes protected-over-total.
2. **App brand logos** - no marks in the repo; `AppRow` falls back to an
   initial. Four SVGs need exporting (`claude-color`, `claudecode`, `codex`,
   `opencode`).
3. **Groups: RESOLVED 2026-08-16 - they keep a home.** The design lists apps
   flat, which cannot express routing's real shape: families (Claude, OpenAI,
   OpenRouter, plus the multi-provider "Other tools" bucket) own a master
   switch, and their members route either through a tool's own config file or
   through the local proxy - the latter being the chat domains, which the drawn
   UI does not show at all. `FamiliesPane` gives them a third sidebar
   destination. **Not in the Figma**; expect it to be redrawn.
4. **Diagnostics: RESOLVED 2026-08-16.** A row under About opens
   `DiagnosticsDialog`, which shows the report before offering to copy it.
   `buildSettingsSections` owns the row, and a test pins it so it cannot be
   dropped silently.
5. **Minimize2: RESOLVED 2026-08-16 - removed.** With window controls coming
   from the OS, a second minimise affordance was a duplicate. The topbar now
   carries only the overflow menu, and the brand lockup is genuinely centred
   rather than sitting at the design's 504px, an offset that existed only
   because a second button balanced it.
6. **Onboarding / FirstRun / OrgPicker / Success: RESOLVED 2026-08-16 - built
   provisionally.** `setup.tsx` gives them `SetupLayout` (chrome plus one
   centred card, no sidebar) and `WelcomePane`, `OrgPickerPane`,
   `ConnectedPane`. Built from the design's own vocabulary rather than invented
   wholesale, but **not in the Figma** and expected to be redrawn.
7. **Does the chart's `total` series double-count?** The Figma legend calls the
   blue series "Total messages" but stacks it *underneath* blocked, flagged and
   redacted. Read literally the bar sums to more than the total. `MessagesBucket`
   currently treats the four as additive, so `total` means "everything not
   otherwise accounted for". Worth settling in the 24-hour backend's response
   shape rather than in the component: if the API really returns a grand total,
   the chart should subtract rather than stack.
8. **Sidebar app states: RESOLVED 2026-08-16.** `SidebarApp` now carries an
   `AppStatus` union (`protected` / `not-protected` / `drifted` /
   `not-routed`), and separately an `on` boolean. The split is deliberate and
   `lib/groups.ts` explains why: observed routing drives the status line, user
   intent drives the switch. Conflating them made the switch destructive, since
   an enabled-but-unrouted member rendered off and clicking it turned the
   setting off rather than on.
9. **Tailwind version skew.** The design names shadows on Tailwind v4's scale
   (v4 `shadow-xs` == v3 `shadow-sm`); the repo is on v3.4.17. The `base-*`
   shadow tokens absorb the mapping, but any new value read off Figma needs
   shifting one step before use.

