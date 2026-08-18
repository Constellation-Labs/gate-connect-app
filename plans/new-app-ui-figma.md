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

## Re-verification of the Flows pages, 2026-08-17

`Components` is unchanged. The three Flows pages have moved.

### Settings screen: four labels and one affordance changed

| Row | Was built as | Now reads |
| --- | --- | --- |
| Device | `Rename` | `Rename device` |
| Install ID | `Copy` | `Copy ID` |
| Gate plan | `Upgrade plan` | `Upgrade plan` + external-link glyph |
| Active session | `Disconnect` | `Disconnect Gate` |

Applied. The API key value also now shows a prefix rather than a full mask -
`sk-gw-661b17***…` instead of `sk-gw***…`. That is the shell's string to build,
not the pane's, and it is a decision about how much of a key to reveal.

### Four new Settings dialogs, all marked ready

- **Rename your device** - form: `Current device name` (readonly) over
  `New device name` (focused, with a clear button). No subtitle.
- **Replace API key** - same form shape. Its second field is labelled
  **"New device name"**, copy-pasted from the rename dialog. Design bug.
- **Disconnect Gate?** - red tone, body as a plain paragraph rather than a
  tinted note, primary `Yes, disconnect Gate`.
- **Reset Gate Connect** - red tone, subtitle, a `What happens next:` list of
  three numbered steps, and a **checkbox** gating the destructive primary.

### Elsewhere

- Overview: the org-switch frames are now properly named (`overview-switch org`
  x3, where only two generically-named ones existed before).
- App: a new **choose model modal**, distinct from the model-switch
  confirmation. `Change model` currently goes nowhere.

### Built 2026-08-17

`Modal.tsx` gained the four things it lacked: a `danger` tone
(`red-100` tile, `red-600` icon), `ModalField`, `ModalSteps`, `ModalCheckbox`.
`ModalButton` gained `disabled` so the checkbox can actually refuse the primary,
and `Modal` gained `initialFocus` so form dialogs open on the field being
edited rather than the read-only one above it. Glyph added: `circleX`.

`dialogs.tsx` gained `RenameDeviceDialog`, `ReplaceApiKeyDialog`,
`DisconnectGateDialog`, `ResetGateConnectDialog`.

Deliberate deviation: `ReplaceApiKeyDialog` labels its second field
**"New API key"**, not the drawn "New device name". Shipping the design's label
would put a wrong word on the one screen where the user handles a credential.

Still unbuilt: the App page's **choose model** dialog. It is only known from a
frame caption; nothing about its contents has been read, so building it would be
invention.

## Branching model

`feat/new-app-ui` is the **integration base** for the design work, not a branch
heading for `main`. PR #151 stays open as work in progress; every PR that
implements a piece of the design targets this branch rather than `main`.

    git fetch origin
    git switch -c feat/wire-routing-actions origin/feat/new-app-ui

Consequences worth keeping in mind:

- **Keep it green.** Downstream branches inherit whatever is broken here, so a
  red base costs everyone. CI covers macOS, Windows and Linux Rust plus the
  frontend; all eight checks were green as of 2026-08-17.
- **Merge `main` in periodically.** The longer this diverges, the worse the
  eventual reconciliation, and every downstream branch inherits the drift.
- **The popover stays until routing works.** It is the only surface that can
  change what is routed, so it is load-bearing, not dead weight. The e2e suite
  is pinned to it (`VITE_NEW_UI=0` in `playwright.config.ts`).
- **This plan is the shared roadmap.** "Still to do" below is the queue that
  downstream PRs draw from; keep it current rather than tracking work elsewhere.
- **Green CI here means less than it looks**, though less so than it did.
  Routing, Settings and the routing verdict are wired and carry their own e2e
  specs (`new-ui-routing.spec.ts`, `new-ui-settings.spec.ts`,
  `new-ui-verdict.spec.ts`), which opt into the new shell per-test through
  localStorage. Everything listed under "Still to do" is
  still covered by nothing.

## How the designer marks readiness (from Chad, 2026-08-14)

- **A white check mark in a section title above a flow means that flow is
  ready.** Sections without one are still moving.
- **Every main frame is named for the step it represents**, deliberately so MCP
  reads and this work can tell which state a frame is. Trust the names.
- **Hidden layers were stripped and 2,000+ layers renamed** with Figma's own
  agent, because MCP struggles with hidden layers. Reads after 2026-08-14 are
  against the cleaned file.
- **The designs are frozen as of 2026-08-14**, with one exception: dialog
  states are still being worked on. Do not chase dialog changes until the
  designer says they are done - `Modal.tsx` and `dialogs.tsx` are the parts most
  likely to move.

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

- `tauri.conf.json`: **DONE 2026-08-17.** 1024x720, `resizable: true`,
  `decorations: true`, `alwaysOnTop` and `skipTaskbar` off. `visible: false`
  is unchanged: the tray still owns the first show.
- **The new UI is now the default** (`newUi.ts`); the popover is the fallback,
  reachable with `gcNewUi(false)` or a `VITE_NEW_UI=0` build.
- **Still popover-shaped on the Rust side.** `show_popover` calls
  `anchor_under_tray`, so the window is positioned under the tray icon rather
  than where the user left it, and `set_activation_policy(Accessory)` keeps it
  out of the dock. Both are wrong for a 1024x720 window and neither is changed
  yet - unverified Rust edits were not worth guessing at.
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

## Still to do

The queue downstream PRs draw from, in the order that unblocks the most.

1. **First run, disconnect and reset: DONE.** `lib/useSetup.ts` derives the
   stage from account and OAuth state rather than storing it, which is what makes
   reset work - clearing the account is the whole handoff, with no separate "show
   first run" flag to disagree with what is on disk. Two things cannot be derived
   and are flagged in the hook: whether the confirmation pane has been seen (a
   returning user must not be greeted by it) and the org-picker dead end's escape
   to the key form. `isSignedIn` / `needsOrg` moved to `lib/session.ts` so both
   shells cannot drift on what counts as signed in.
2. **Updates: DONE.** The mechanism moved to `lib/useUpdate.ts` and both shells
   use it - the opposite call from `useRouting` and `useSettingsActions`, because
   nothing here was tangled with the popover's shape and what it holds is a
   sequence whose ordering is load-bearing: the relaunch mark lands after the
   download (quitting mid-download is a genuine exit that must keep its cleanup)
   and before `install()` (on Windows the installer exits from inside that call).
   A second copy could drift into a botched update. The window uses the banner
   `AppShell` already had a slot for; the popover keeps its takeover-then-banner
   escalation. Settings' "Check for updates" reports its result in the version
   row, since silence on a pressed button reads as broken.
3. **The running-apps sequence: DONE.** `lib/useRunningApps.ts` runs it after a
   config write that actually happened - `setAppRouted` now reports that, so a
   declined review or a failed write never reaches it. Deliberately not part of
   `useRouting`: that hook's prompt is a *gate* that blocks a write until
   answered, and this is the opposite, a sequence that follows a write and can be
   walked away from without changing what was saved. Nothing is signalled without
   two answers, and a failed scan stays silent rather than defaulting to showing
   (the popover defaults the other way, but it is choosing whether to show
   advice; this offers to kill processes). The e2e fixture gained
   `runningAgentNames`, since it stubbed only the count probes.
4. **The family master cascade: DONE.** The rules moved to `cascadeTargets` in
   `lib/groups.ts` and both shells use them: chat members never ride a family
   switch, a drifted config is never adopted by one, and members already in the
   target state are left alone. The action is `setFamilyRouted` in `useRouting`,
   which trusts the certificate ahead of the loop (a member's connect
   auto-enables the engine, so the system dialog must not be sprung from member
   three) and names the members that failed rather than reporting "couldn't
   connect this tool".

   Two things this turned up. The new UI was driving the family switch from
   `Group.desired` instead of `cascadeDesired` - the exact bug `groups.ts`
   documents, where a chat member switched on alone leaves the master stuck on.
   And a config member's `desired` is derived from `connected`, so a **drifted
   member is outside the family switch in both directions**; the row and the
   alert card are the ways back. That is pre-existing shared behaviour, so it is
   pinned by a test rather than changed here.
5. **The per-app model picker: UI DONE, no backend exists.** `App / Select a
   model` was finally read (browser, "App w/ choose model modal open"): the
   design draws a **dropdown anchored to Change model**, one row per model, the
   current one first and outlined, listing eleven `gate/...` ids across
   Anthropic, DeepSeek, Qwen, Kimi and OpenAI. Built as `ModelPickerDialog`; the
   choice is confirmed through `UseGateModelDialog` because switching to a Gate
   model spends PAYG credits.

   **There is no model backend at all** - no Tauri command, no Rust, nothing to
   persist to. The choice is session state and says so at the call site, and the
   picker ships with an empty list rather than the drawn ids: shipping those
   would put a fabricated model catalogue in front of the user, which is the same
   argument the zeroed metrics make. What it needs is an endpoint reporting the
   gateway's models and somewhere to record the per-app selection.

   Two details of the frame could not be read: the top edge was cut off, so
   whether the panel carries a search field is unknown (omitted), and the drawn
   ids render in the UI face rather than mono. Mono was used, since CLAUDE.md
   names model ids explicitly and every other identifier in the design is mono.
6. **Truthful routing status (AG-562): DONE.** Status lines no longer come from
   `Tool.status`. `crates/core/src/routing_health.rs` computes a verdict from
   three inputs - the integration's config state, a new loopback health check
   that the relay answers itself (`/__gate/health`, 204, never forwarded), and
   whether the tool's process predates the last routing change - and
   `routing_verdicts` reports it per tool. `lib/verdict.ts` maps that onto the
   four phrases the design draws, carrying the ticket's reason in the grey suffix
   the design already has a slot for.

   `Status` was deliberately **not** rewritten. It describes config on disk and is
   threaded through every integration, `provider.rs` and their tests; the verdict
   is a layer on top, which is also what keeps intent and observation separate one
   level below where `groups.ts` documents the same rule.

   Three things this turned up. The probes run **once per sweep**, not per tool -
   they ask about shared infrastructure (the relay port, the account's session), so
   per-tool calls would let two rows in one refresh disagree. `SessionProbe::Unavailable`
   must map to `Verification failed` rather than `Access problem`, or an offline
   machine accuses a perfectly good credential. And a tool with no known process
   name (OpenClaw, Hermes) has *unobservable* staleness rather than none, so it can
   still read Protected but will never be told to reopen.

   **Two conflicts raised rather than resolved in code.** AG-562 specifies the
   words On / Off / Needs attention; the Figma draws Protected / Not protected /
   Config drifted / Not routed. The design won the coloured phrase and the ticket
   won the reason, because the Figma is the source of truth for copy - the
   remaining question is for the designer on AG-561. Separately, AG-564/566/570/596
   all assert Gate "does not restore user-authored values", but
   `integrations/claude_code.rs` saves `previousEnv` and restores it on disconnect,
   and `groups.ts:312` documents that behaviour too. The code is right; the copy
   was not written.

   What this does **not** do: prove a tool sent traffic. Nothing attributes
   requests to a tool. That needs a per-tool segment in the relay path
   (`/<tool>/<domain-slug>` instead of `/<domain-slug>`), which would also give
   AG-574 its per-tool counts.
7. **Metrics.** Overview and `AppPane` render zeros against `EMPTY_STATS`
   pending the 24-hour endpoint. Open question 7 (whether `total` double-counts)
   should be settled in that response shape, not in the chart.
8. **Retire the popover.** `App.tsx`, `screens/`, `gc/ui.tsx`, the `gc.*`
   palette, `pinPopover` / `unpinPopover`, and `VITE_NEW_UI=0` all go together.
   Item 1 was the blocker and is done, so what remains is repointing the e2e
   suite at the new shell and deleting; the popover's own specs go with it.

Rows the design draws that have no backend command at all: device rename and
notifications, plus plan upgrade, which has no billing URL to open. They render
their value and omit their control.

Two rows are gated on the account's auth mode rather than hidden outright:
**Replace key** appears only for an API-key account (on an OAuth account
`saveAccount` with a key would flip `auth_mode`, quietly converting the account
behind a button that says "replace"), and **Disconnect Gate** only for an OAuth
account, which is the only one with a session to end.

## Copy corrections to raise with the designer

Two dialogs say something the action does not do. Both are implemented correctly
and the reason is recorded at the component:

- **Replace API key** labels its second field "New device name", copy-pasted from
  the rename dialog. Shipped as "New API key": the drawn label would put a wrong
  word on the one screen where the user handles a credential.
- **Disconnect Gate** says protection turns off, apps stop routing and the key is
  removed from the keychain. That describes Reset, a separate row on the same
  screen. Implemented as ending the session, and the body copy corrected, rather
  than shipping two destructive actions that claim the same consequences.

## Drift repair (AG-568)

The review dialog and the drift *gate* were already built (`ReviewConfigDialog`,
`useRouting.ts` refusing to adopt a drifted config silently). Two things were
missing, and one of them was a correctness bug.

**A failed write left the row lying.** `connect_tool` failing sent the error to a
transient banner, and the status line - the thing next to the switch the user
just clicked - carried on describing the state from before the click. It was
*true*, since nothing was written, and useless. `useRouting` now remembers which
slugs failed (`writeFailures`), and the row reads "Configuration update failed".

That state is deliberately **not** a sixth `routing_health::Reason`. The Rust
reasons are derived from evidence - a config on disk, a relay that answers, a
process older than the last change - and a failed write leaves none of that to
probe. It is session state, cleared the moment a write for that slug succeeds, and
it arrives at `verdictStatus` as a separate argument for exactly that reason.

Worth noting: **AG-562's list of five reasons is incomplete.** AG-564 and AG-568
both name "Configuration update failed" as a status. Raised on those tickets
rather than smuggled into the enum.

**The dialog showed what Gate found but not what it would write.** Approving an
overwrite without seeing the replacement is approving a value you cannot see, on
the one screen where the user hands their tool's routing to us. `ProxyState`
gained `relay_base_url` (non-secret - it is already written verbatim into every
config-routed tool's own file), and the dialog shows it. With no relay port bound
the row is omitted rather than guessed at.

Still open on the ticket: the per-failure action set ("Retry, Use tool defaults,
Documentation, Diagnostics, or Contact support based on the failure"). The sidebar
row has a switch and no room for a second control, and the switch *is* the retry -
but a documentation or diagnostics link per failure needs the per-app pane, and
Contact support needs a URL that does not exist. Also open: "last completed check
or routed request" in the summary, which needs the activity endpoint.


## Settings sections and preferences (AG-594)

There was **no preferences store anywhere**. `account.rs` was the only config, and
it holds a credential and an identity, so the notification and diagnostics
choices had nowhere to live - which is why `SettingsPane` had a `notifications`
prop that the shell never passed. New `crates/core/src/preferences.rs`: a small
JSON file next to `account.json`, deliberately separate so a preference change
does not rewrite the file holding the key prefix, and so clearing the account on
reset does not take the preferences with it.

**Every preference defaults to on, and a missing field loads as on.** That is
what lets a switch read On before anything has been written, rather than showing
Off and inviting the user to "fix" a setting that was never off.

Sections went from 6 to the 8 the criteria name that can be built: Device,
Account, Connection, Startup, Notifications, Diagnostics, About, Help, plus the
Danger zone. Notifications and Diagnostics moved out of Startup and About
respectively.

### Rows deliberately not built, and why

- **Blocked-event, flagged-event and sound switches.** The criteria list four
  notification switches. The app fires exactly two notifications, both about
  routing (an expired session; a quit that could not put a tool back), and both
  now ride the one `Routing health` switch. Blocked and flagged notifications
  need the live security-event feed (AG-578), which does not exist. A switch for
  an event that cannot arrive tells the user they turned something off.
- **The permission row.** `tauri-plugin-notification` hardcodes
  `PermissionState::Granted` on desktop - `desktop.rs` returns it unconditionally,
  the state is only real on mobile. A permission row built on that would report
  "granted" on a Mac with notifications denied. Real detection needs per-platform
  native work (UNUserNotificationCenter on macOS).
- **Contact support.** There is no support URL anywhere in the app. `GATE_DOCS_URL`
  exists, so Documentation is wired; support is omitted rather than pointed at an
  invented address. The topnav's Contact support entry is dead for the same reason
  (AG-598).
- **Send diagnostics now, and the diagnostic reference.** No upload path exists;
  that is AG-603.

### Unavailable rows

`SettingsRow` gained `unavailable: { onRetry }`. The shell now tracks *whether a
read failed* separately from the value it failed to produce:
`launch?.enabled ?? false` collapsed "off" and "could not be read" into one Off
switch, which is a claim about the user's setting they cannot distinguish from one
they made. Wired for launch-at-login and for the preferences pair, which share one
read and so fail together - a failed preferences read leaves launch-at-login's
switch alone, and a test pins that.

## Refreshing the inventory (AG-558)

Detection ran on backend events only, so a tool installed while the window was
open stayed invisible until something unrelated repainted the sidebar. The
"Protected apps" eyebrow gained a small refresh control that re-reads tools and
proxy state and re-runs the routing sweep.

Provisional: the Figma draws no refresh control. It is 20px in a 12px eyebrow with
an `aria-label` rather than visible text, and no spinner - the scan is fast enough
that one would only flash, so `aria-busy` plus the disabled state is the signal.
`refreshing` is deliberately separate from `routingBusy`: that one guards a
*write*, and refusing to re-read during a toggle would be the wrong coupling.

The rest of AG-558 is not buildable and is documented on the ticket: every
integration returns `requires_upstream_credential() == false`, so "installed but
unavailable" cannot occur; "installed but unsupported" needs detection of tools
Gate has no integration for; "incomplete installation" needs per-integration
probes; and the per-entry request counts need per-*tool* attribution, which the
in-flight `feat/activity-overview-client` does not provide (it is per-installation
- `activity.ts` has no tool or slug in it).


## An empty inventory is not a failed one (AG-560)

`listTools().catch(() => [])` turned a failed read into an empty array, so a device
Gate could not scan rendered exactly like a device with no AI apps on it - blank
list, "0/0" count, no explanation. `InventoryState` in `Sidebar.tsx` now tells the
two apart:

- **`none`** - the scan completed and found nothing. Carries the scan time, which
  is what makes it an answer rather than a shrug, plus Refresh.
- **`failed`** - the scan could not complete. Amber, says Gate does not know what
  is installed and that nothing was changed, and offers Try again.
- **`ok`** - there are rows, and the rows speak for themselves.

Before the first scan lands the state is `ok`, deliberately: "no apps detected" is
a claim, and nothing has checked yet.

The eyebrow's refresh control (AG-558) hides while the card is up, since the card
carries its own and two controls for one action in a 250px rail is one too many.

Not built, and recorded on the ticket: "a detected but unsupported tool remains
visible" needs detection of tools Gate has no integration for; "a known but absent
tool may provide an installation action" is optional in the criteria and would put
uninstalled tools in a rail the Figma draws as installed apps only; and the
model-control gating would remove the picker shipped in #159.

## The diagnostics switch now gates something (AG-603)

The "Share diagnostic data" switch added for AG-594 recorded a preference that
**nothing read**, while PostHog collected regardless. `lib/analytics.ts` is a real
channel - initialised at boot in `main.tsx`, capturing a closed set of event names,
a filtered prop allowlist, classified error titles and two coarse super-properties
- and it had no user opt-out at all. A switch that implies control it does not have
is worse than no switch.

Now:

- **`initAnalytics` reads consent before constructing the client.** An install that
  opted out never creates it. Consent is checked *before* `posthog.init`, not
  after, because opting out afterwards would still have put the device on the wire
  first.
- **A failed consent read means do not collect.** `preferences::load()` is
  infallible in Rust, so the only path here is the IPC failing - and consent that
  cannot be confirmed is not consent.
- **`setAnalyticsConsent` applies a change immediately.** Off opts the live client
  out; on starts it if this session never did (the opted-out install) and opts back
  in otherwise. Called from the Settings switch *before* the write, so a failed
  write cannot leave the client sending after the user said no.
- **`initAnalytics` is now async and not awaited** in `main.tsx`: blocking first
  paint on an IPC round trip would trade a visible delay for a few milliseconds of
  telemetry.

A read-only "What is collected" list opens from Settings without touching the
setting, as the criteria require. Its contents are written from what
`analytics.ts` actually sends, **not** from the ticket's field list - that list
describes an upload that does not exist (installation name, verification state,
event-delivery state, notification permission), and describing it would be
describing something Gate does not do, on the one screen whose job is telling the
truth about what leaves the machine.

Still open on the ticket: the onboarding Diagnostic data step (shared with AG-554),
"Send diagnostics now" and the diagnostic reference (no upload path exists), and
scoping the choice to the selected organization.


## The diagnostic-data onboarding step (AG-554 / AG-603)

Consent before collection: `lib/analytics.ts` starts PostHog at launch, so what
this step buys is a person who has actually been asked. It sits between the
sign-in confirmation and Overview.

**Derived, not remembered.** `preferences.json` gained
`share_diagnostics_recorded`, and `useSetup` returns the `diagnostics` stage while
it is false. That keeps the guarantee the hook already had - stage comes from what
is on disk - so a reload mid-setup cannot skip the step, and answering it is what
dismisses it. `undefined` (the read still in flight) is deliberately *not*
unanswered, or the step would flash at someone who answered months ago.

`share_diagnostics_recorded` defaults to **false**, including on installs written
before the field existed. Those see the step once, which is the point: sharing
defaults to on, and a default nobody was asked about is not consent. It is also why
this is a separate field rather than inferring an answer from `share_diagnostics` -
the default and a deliberate "yes" are the same value and must not be the same
fact.

Continue records the **displayed** value whether or not it changed, because leaving
the default in place is still an answer; treating it as unanswered would ask again
next launch. The same command backs the Settings switch, so changing it there also
counts as answering.

The sent / never-sent lists are shared with the Settings disclosure
(`CollectedDataLists`, one copy, framed by whatever `Wrapper` each caller passes).
Two copies would drift, and these are the claims the product's reassurance rests
on - the moment the onboarding promise and the Settings disclosure disagree,
neither can be trusted.

Layout is provisional; the Figma draws no diagnostics step.

### A harness bug this turned up

`e2e/install.ts`'s `get_preferences` returned the *live* `state.preferences`
object, which the write stubs mutate in place. So `setPrefs` received an identical
reference, React skipped the re-render, and a preference change was invisible to
anything derived from it. Real IPC serialises every response; the stub now returns
a copy. Worth remembering for the other handlers that still return `state.x`
directly.

Still open on AG-554: the device-derived installation-name suggestion (device
rename has no backend command), and distinguishing cancellation from expiration
from failure in the browser sign-in.
## Interrupted routing, explained and resumable (AG-570)

Three changes, in three PRs.

**1. The window shell had no backend-error drain.** `drainBackendErrors` existed only
in `App.tsx`, so a failure buffered Rust-side went to telemetry and nowhere else -
including `report_backend_error("provider_restore", ...)`, which fires on both
restore passes in `proxy_enable`. Routing could fail to come back and the window
said nothing. `forwardBackendErrors` moved to `lib/backendErrors.ts` and both shells
import it; the popover's behaviour is unchanged.

**2. The snapshots were already a record of unfinished work.** `restore_all`
re-attempts each recorded entry, keeps failures in the file, and clears it only once
everything is back. Nothing read them for display. `provider::pending_restore()`
reports what they owe, `resume_restore` calls `restore_all` and returns **what is
still outstanding** rather than unit, and `RecoveryBanner` names it with Resume now
and Finish later. Amber, not red: nothing is lost, and resuming retries exactly what
failed.

**3. A per-entry journal** (`crates/core/src/recovery.rs`) records what the restore
*did*, as opposed to what is left. Written as it goes and seeded before the first
attempt, because an interrupted operation is the one worth describing and a journal
written on completion would be missing for every case it serves. `RestoreDetailsDialog`
shows it read-only - AG-570 requires that reviewing changes no state.

Details worth keeping:

- **One journal for the whole restore.** Both passes seed it up front; two writers
  would clobber each other's file, which they briefly did.
- **Outcomes come from the control flow, never from an error message.** The restore
  already branches on not-installed, unknown-slug and signed-out, so classifying
  those costs nothing and cannot drift.
- **Dropped entries owe nothing.** `NotInstalled` and `Unknown` are settled, not
  outstanding; reporting them would ask for action nobody can take.
- **A journal is an explanation, never a blocker.** A missing or corrupt one reads
  as absent and the restore still runs. Its fields are `#[serde(default)]`-backed
  for the same reason - a unit test caught that they were not, which would have
  silently dropped the explanation on any older file.
- **Known shortcoming:** only the tool pass can report `DeferredSignedOut`.
  `enable_skipping` has no signed-out branch, so a provider that cannot re-enable
  for want of an account lands on `WriteFailed`. Pinned by a test so it stays
  visible.

Still open: the recovery action lives in the shell banner, not the tray; per-tool
"last verified route" and "check result" in the summary would need the verdict sweep
to persist its results; and item 8's default-writing result list exists for the quit
path only (#162), not yet for sign-out and reset.
## Quit and teardown in the window shell (AG-596)

The popover has carried the three-way quit since it shipped
(`components/QuitConfirm.tsx`: disconnect-and-quit / quit-without-disconnecting /
cancel, with focus on cancel). The window shell had **nothing**: no
`quit-requested` listener and no dialog, so a tray Quit raised in the new UI went
unanswered. Fixed by porting the flow, not reinventing it - `QuitDialog` in
`dialogs.tsx` shares the popover's copy so a user who has seen one is not asked
to work out whether the other means something different.

`Modal` gained an optional **`middle` action**. Three buttons because the
outcomes genuinely differ: collapsing "quit without disconnecting" into the
primary would hide the consequence that makes it a separate choice, and
collapsing cancel would leave no way out. It is the only dialog that needs it.

The larger change is behavioural, and it applies to **both** shells.
`provider::snapshot_and_disable_everything` swallowed per-tool disconnect
failures into `eprintln!` and returned `Ok(())`, so a quit that left a tool
pointing at a relay about to die reported success and fired a notification
saying "your tools are back on their own settings". It now returns the display
names of what it could not put back:

- `disconnect_tools_for_quit` returns `Vec<String>` rather than `()`.
- The quit notification only claims a clean teardown when there was one.
- Both shells stop, name the tools, and offer retry / quit-anyway rather than
  exiting quietly. AG-596 is explicit: Gate Connect "does not claim cleanup
  completed".

Not covered here, and still open on the ticket: the reset result's equivalent
listing ("if reset leaves a credential, configuration, or routed tool, Gate
Connect lists it"). Reset's checkbox gate and confirmation are built; enumerating
*residue* after the fact needs the same treatment `snapshot_and_disable_everything`
just got, applied to the reset path.

The restore-vs-AC conflict recorded against AG-564/566/570/596 is unchanged by
this work: `claude_code.rs` does restore prior values, so no copy here claims
otherwise.
## Naming the file Gate rewrites (AG-564)

`Integration::config_location()` is new: the file each integration edits, as a
display string, implemented for Claude Code, Codex, OpenCode, OpenClaw and Hermes
and defaulting to `None` for the environment channel, which writes machine-wide
settings rather than a file of its own. It reaches the UI on `ToolDto` and the
drift review now names the file before asking the user to approve a rewrite.

Two things this is deliberately *not*:

- **Not a probe.** Reading the location creates nothing. A test pins that,
  because a call that materialised a config directory would make `status()`
  report a tool as present because the UI had looked at it.
- **Not a secret.** It is a path in the user's own home directory, and the point
  of showing it is that they can go and read it - which is the reassurance
  CLAUDE.md's first design principle asks for, and stronger than any sentence
  about what Gate does or does not touch.

### The confirmation this ticket asks for, and why it is not here

AG-564 asks for a confirmation before *every* turn-on: "Before routing is turned
on, Gate Connect states that it will replace the tool's routing configuration
with Gate values", with **Use Gate routing** and **Cancel**. It was built, and
then removed before shipping, for reasons worth recording:

1. **The design draws a bare switch.** The Figma has no confirmation on this flow,
   and AG-563 - this ticket's design counterpart - is still In review, so the
   designer has an opinion in flight that this work cannot see. CLAUDE.md makes the
   Figma the source of truth for exactly this kind of question.
2. **It stacks two modals on a first turn-on.** The gate lands before
   `ensureCaTrusted`, so the first tool a user routes would raise "Route X through
   Gate?" and then "Trust the Gate certificate?" back to back.
3. **The criteria are written per tool and say nothing about the cascade.**
   `setFamilyRouted` calls `connectTool` directly rather than through
   `setAppRouted`, so a family switch would have escaped the gate anyway - which is
   just as well, since a modal per member would make that switch unusable. But
   that means the ticket's model and the product's shape disagree.

The warning does exist for the case where something of the user's is actually at
risk of being overwritten: a drifted config, which has had the review dialog all
along. What a general confirmation would add is coverage of the *clean* case,
where `integrations/codex.rs` still replaces the user's own `model_provider` (and
stashes it for disconnect). That is a real gap, and a real design decision -
raised on AG-564 and AG-563 rather than settled here.
