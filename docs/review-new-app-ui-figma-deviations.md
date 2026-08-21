# Figma deviation audit: `feat/new-app-ui`

Audited 2026-08-20 against `plans/new-app-ui-figma.md` and the values it records
as sampled from `9FrccCojXy0f8QD8Wm5Lln`.

**No live Figma reads were made.** The MCP View seat allows 6 reads per month and
the plan records them as spent; the browser method (Layers panel + `shift+2`) was
not used either. So the reference here is the plan's own sampled-value tables plus
the two Figma captures committed under `src/assets/`. Where a finding depends on
a value nobody has sampled, it says so rather than asserting a mismatch.

**Superseded in part, 2026-08-21.** `121:33256` (`topsection` of
`overview-dimensions`: `topnav` over `banner/update` over
`banner/status-protected`) was read through the browser, so those two banners are
no longer inferred. What that closed is in §G, "Settled by the 2026-08-21 Figma
read"; the rows it touches say so.

Token hygiene is clean: no `bg-blue-*`/`text-blue-*` against the redefined OKLCH
ramp, no px font literals, no `purple` left where the drawn fill is violet.

---

## A. Deviations the plan documents, still accurate (no action)

These are decided, reasoned at the call site, and match what the plan says. Listed
so a later reader does not re-open them.

| Deviation | Where | Why |
| --- | --- | --- |
| Traffic lights dropped, space reserved | `gc/Topbar.tsx:10` | Window controls are the OS's |
| Minimize2 removed | `gc/Topbar.tsx:12`, glyph gone from `Icon.tsx` | Duplicated an OS control (open q5) |
| `Contact support` menu entry omitted | `gc/Topbar.tsx:19` | No support URL exists |
| `FamiliesPane` + its sidebar nav item | `gc/FamiliesPane.tsx:8`, `gc/Sidebar.tsx:146` | Flat app list cannot express family/chat-domain routing (open q3) |
| Empty / failed inventory states | `gc/Sidebar.tsx:253` | Figma draws no empty inventory (AG-560) |
| Diagnostics onboarding step | `gc/setup.tsx:563` | Figma draws no diagnostics step (AG-554) |
| `QuitDialog`, `RestoreDetailsDialog` | `gc/dialogs.tsx:985`, `:931` | Figma draws neither (AG-595, AG-569) |
| `ErrorBanner`, `RecoveryBanner` | `gc/banners.tsx:236`, `:292` | Figma draws neither |
| Certificate-trust `red` status tile tone | `gc/base.tsx:54` | Follows the drawn 50→200 / 300 / 700 pattern |
| `allow` action pill neutral | `gc/Overview.tsx:61` | Figma draws only the three enforcing actions |
| `Replace API key` second field says "New API key" | `gc/dialogs.tsx:639` | Drawn label is a copy-paste bug from the rename dialog |
| `Disconnect Gate` body copy corrected | `gc/dialogs.tsx:684` | Drawn copy describes Reset, not disconnect |
| Model ids rendered mono | `gc/dialogs.tsx:474` | Frame uses the UI face; every other identifier in the design is mono |
| Model picker search field omitted | `gc/dialogs.tsx:477` | Panel's top edge was unreadable in the only capture |
| `Sign-in method` row under the API key | `gc/SettingsPane.tsx:263` | Parity with the popover; not drawn |
| Onboarding step 2 keeps a platform-aware sub-heading | `screens/Onboarding.tsx:30` | Drawn copy is macOS-only |
| Status vocabulary: design's phrases over AG-562's | `lib/verdict.ts:9` | Figma is the source of truth for copy |
| Skeleton / loading silhouettes | `gc/base.tsx`, `gc/metrics.tsx:331` | No frame draws a loading state |

---

## B. Real deviations recorded nowhere in the plan

The plan is the shared roadmap, and these are missing from it. Each is defended in
code; the gap is that the roadmap does not know about them.

### B1. Recent activity lost two of its five drawn columns

The plan (line 769) records the drawn table as **Time / Status / Security /
Conversation / Action**, with a `View` button per row. What ships is **Time /
Status / Security / Model** plus a mono conversation reference.

- **Conversation → Model.** `lib/toolEventRow.ts:29-38`: the only human-readable
  conversation label the gateway holds is the user's own prompt, stored
  unredacted, and AG-574 excludes prompt text. The row identifies the request by
  what served it plus a conversation id instead.
- **Action column dropped.** `gc/AppPane.tsx:370`: the `View` button has no
  destination - nothing in `dashboard-web` filters by tool, machine or time.

Both are the right calls. Neither appears in the plan, and the plan's only mention
of AG-574 (line 1017) still says per-tool attribution "does not exist" - which is
no longer true, since `toolEvents.ts` / `useToolEvents.tsx` shipped it.

### B2. `InstallationPicker` is a control the design does not draw

`gc/InstallationPicker.tsx`, rendered into the Overview header at
`NewUiApp.tsx:1757`. It is required by AG-572 AC 1 and it self-hides below two
installations, so it is well behaved - but its own doc comment argues only about
`<select>` versus a hand-rolled menu and never says the Figma has no such control.
The word "InstallationPicker" does not appear in the plan at all.

### B3. The tokens-saved stat tile is now a button

`gc/Overview.tsx:120` passes `onSelectTokensSaved`, so the third stat tile scrolls
to the Token savings section (`gc/metrics.tsx:112`). The plan's Phase 5 inventory
describes the stat trio as three read-only figures divided by `border-l`. Whether
the Figma draws the tile as interactive was never sampled; the comment cites
AG-572, not a frame.

### B4. The certificate-trust dialog is not listed as an undrawn surface

`NewUiApp.tsx:1436` says it outright: "Not in the Figma: the new design has no
certificate surface." The plan mentions this modal only in passing, at line 1458,
as the second half of a modal-stacking argument about AG-564 - never in the
undrawn-surface inventory beside `FamiliesPane` and the quit dialog.

### B5. `SettingsPane`'s own comment understates what it adds

`gc/SettingsPane.tsx:64` reads "Diagnostics is the one row the Figma does not
draw." The pane now carries nine sections against the drawn six: **Notifications**,
**Diagnostics** and **Help** are all additions, and Diagnostics is a section, not
a row. The plan's AG-594 section does record the 6→8 growth, so this is a stale
in-code comment rather than an undocumented decision - but it is the comment a
reader lands on first.

### B6. Model picker: dropdown redrawn as a centred overlay

`gc/dialogs.tsx:466` states the design draws a dropdown anchored to `Change model`
and explains why it ships as a modal-positioned popover (focus trap, escape
handling, the drawn placement being legible at one zoom only). The plan records
that `ModelPickerDialog` was built but not that the anchoring was abandoned.

---

## C. Where the plan is now stale about its own numbers

Not code bugs - the plan should be corrected so the next reader does not "fix" the
code back to it.

- **Topbar left spacer: plan says 60px, code says 32px.** Phase 4 records a "60px
  `aria-hidden` spacer" holding the lockup at the drawn 504px of 1024.
  `gc/Topbar.tsx:48` is `w-8` (32px), mirroring the right-hand button cluster so
  the lockup is dead centre. The comment there explains it and open question 5
  agrees; Phase 4's text was never updated.
- **Phase 4 lists `minimize2` among added glyphs.** It is gone from `Icon.tsx`.
- **Phase 3 names a `banners/` directory.** It is a single `banners.tsx`.
- **"MCP never sees the `Flows` pages" is wrong as stated** (line ~1043). It never
  *lists* them: `get_metadata` with no `nodeId` returns top-level pages only, by
  design. Every read tool takes `fileKey` + `nodeId` parsed out of a
  `?node-id=177-79237` URL, so `Flows / Onboarding` (`177:79237`) and
  `Flows / Auth` (`177:79238`) were addressable by MCP all along. The browser
  method is still the right default on a View seat, but for quota reasons, not
  reachability.

---

## D. Values still inferred, never sampled

Unchanged since the plan recorded them. Worth one browser pass together, since
they are all one screen apart.

| Value | Where | Status |
| --- | --- | --- |
| Danger-zone card `red-50` fill, `red-200` border | `gc/SettingsPane.tsx:530` | Inferred from the red section heading |
| App-page status/security pills at the 200 stop | `gc/AppPane.tsx:34` | Not sampled - the App page draws them too small |
| `StatusPill` "Off" fill `neutral-100` | `gc/Overview.tsx:309` | No frame draws an off pill |
| Sidebar right border `black/8%` | `gc/Sidebar.tsx:133` | Not `base.border #e5e7eb`; no sampled value recorded either way |
| Update banner bottom border `black/20` | `gc/banners.tsx:29` | **Settled 2026-08-21**: the frame draws 1px `base/border` #e5e7eb. Fixed |
| Gate-model option icon | `gc/AppPane.tsx:213` | Still the `layers` placeholder; the design draws brand art |

---

## E. Priority finding: the tutorial teaches a control that was deleted

Not a token or a layout question, and the one item here that a user meets.

Onboarding step 2 ships the drawn copy verbatim
(`screens/Onboarding.tsx:120-123`): "Click the Gate Connect icon to open the
compact popover for a quick status check, **or expand it to the full desktop app**
for more details, alerts, and controls", under a note reading "Open the desktop app
for detail. **Collapse to a popover** for a fast status check."

There is no collapse control. `minimize2` was removed under open question 5 and
the glyph is gone from `Icon.tsx`; the popover shell is reachable only through
`gcNewUi(false)` in devtools. So the first-run tutorial instructs a new user to
use an affordance the shipping default UI does not have, on the step whose whole
job is telling them where the app lives.

The plan flags the conflict ("either the tutorial copy is stale or dropping
Minimize2 was wrong; the designer should say which") and left both sides alone.
That was defensible while the copy was unbuilt. It shipped on 2026-08-20, so the
conflict is now live and one of the two has to move:

- keep the removal and cut the two expand/collapse sentences, or
- restore a collapse control and honour the tutorial.

The first is a two-line edit and loses nothing the design was promising elsewhere;
the second reopens a resolved question. Recommend the first, and raise the copy
change with the designer rather than the control.

---

## F. Drawn but unbuilt (gaps, not deviations)

For completeness; all already tracked in the plan.

- `Auth / Error states` - the setup-timeout dialog and device-name validation. No
  ready mark on the section, verified twice.
- The App page's **loading** half of `App / No data 1+ day state`. Nothing can be
  loading until the endpoint exists.
- The Overview's own `overview-loading` frame was never opened; whether its
  no-data copy differs from the App page's is unverified.
- Per-app brand logos - four SVGs unexported, `AppRow` falls back to an initial.
- `PROTECTED APPS 4/4` reads 4/4 in every frame regardless of switch state (open
  question 1); the sidebar computes protected-over-total.

---

## G. Per-screen token sweep, 2026-08-20

Added on the instruction that **every screen should match**. This pass ignores
the flagged items above and instead walks every new-UI surface mechanically,
looking for one semantic thing rendered two ways. It needs no Figma access: an
internal contradiction is evidence on its own.

Surfaces swept: `Topbar`, `Sidebar`, `banners`, `Overview`, `metrics`, `AppPane`,
`SettingsPane`, `FamiliesPane`, `Modal`, `base`, `setup`, `screens/Onboarding`.

### Dismissed by verification (recorded so they are not re-raised)

- **No bare `rounded`** in any new-UI surface. `borderRadius.DEFAULT` is 6px, the
  cg-era everyday radius, so a bare `rounded` would silently render the popover
  value. The only two hits are in `gc/ui.tsx`, which is popover-era by design.
- **No bare `shadow`.** The apparent hit was the word `shadow/sm` inside a comment
  in `base.tsx`.
- **`text-base-muted` does not exist and is not used.** It was an artefact of the
  sweep's own regex truncating `text-base-muted-foreground`.

### Confirmed drift

| # | Finding | Where | Needs |
| --- | --- | --- | --- |
| G1 | **Two names for one ground.** `base.background` is `#f9fafb`, which *is* `gray-50`. Both `bg-base-background` and `bg-gray-50` are in use, while the pane ground is `bg-gray-100`. Three names, two colours. | `base.tsx`, `Onboarding`, `Sidebar`, `AppPane`, `SettingsPane`, `Modal`, `setup` | A rule, not a value. Zero visual change. |
| G2 | **Two names for primary.** `base.primary` is `#203de2`, which *is* `blue-ribbon-700`. `Onboarding` uses both in one file. | `Onboarding`, `Modal`, `setup`, `base.tsx` | A rule, not a value. Zero visual change. |
| G3 | **Modal tone icons sit on two different stops.** `text-red-600` for danger, but `text-amber-700` / `text-green-700` for warning and success. The plan records `red-600` as sampled; the other two were never recorded. | `Modal.tsx` | **Partly answered 2026-08-21**: the ShieldBan in the routing banner reads `amber/600`, and all three `StatusTile` tones moved to 600. `Modal.tsx` is a different node and still sits on 700. |
| G4 | **Modal tone tiles are a flat `100`; `StatusTile` is a `50 -> 200` gradient with a `300` border.** Two treatments for what may be one component in the design. | `Modal.tsx` vs `base.tsx` | One Figma read. |
| G5 | **Border tokens used as backgrounds.** `bg-base-border` and `bg-base-input` back what are presumably dividers, rails and switch tracks. Visually plausible, semantically wrong: changing a border colour would move a rail. | `setup.tsx`, `Onboarding`, `base.tsx` | A rule, plus confirmation the rail colour really is the border colour. |
| G6 | **Four hand-rolled alpha borders** where every other border in the app is `base-border` or `base-input`: `black/[0.08]` (sidebar right edge), `black/20` (update banner bottom), `white/[0.24]` (app tiles), `white/40`. | `Sidebar`, `banners`, `AppPane` | **Two settled 2026-08-21** and fixed: `black/20` is `base/border`, and `white/40` was the `Update` button's border, which the frame draws as `base/input` on a white face. `black/[0.08]` (sidebar) and `white/[0.24]` (app tiles) are still open. |
| G7 | **Neutral stops in use exceed the stops ever verified.** In use: 50, 100, 400, 500, 600, 700, 900. Recorded as sampled: 50, 600, 900. So 100, 400, 500 and 700 are all unchecked. | every surface | A spot check, most cheaply as a variables dump. |
| G8 | **Onboarding still carries popover-era styling.** `rounded-[28px]` is off-system entirely; `rounded-[2px]` duplicates `rounded-sm`; and `shadow-[0_14px_34px_rgba(0,42,95,0.5)]` is a navy glow built on `#002a5f`, the `NAVY` constant from the popover-era `ConstellationHexMark`. The design draws this screen inside the new window shell, so these are very likely leftovers from the port rather than drawn values. | `screens/Onboarding.tsx` | One Figma read to confirm, then deletion. |

### Settled by the 2026-08-21 Figma read

`121:33256`, read through the browser (Layers panel click, then `shift+2`, values
off the properties panel). The routing banner needed nothing: fixed 1024x48,
padding 8/16, `base/card`, 1px bottom `base/border`, tile and right rail all
already exact. The update banner needed four, and `StatusTile` a fifth.

| Value | Drawn as | Was | Now |
| --- | --- | --- | --- |
| `Update` button | `Button` variant Outline, size xs: `base/card` fill, 1px `base/input`, `base/primary` label, padding 4/10, the domed three-shadow recipe | `white/40` border and `white` label on transparent, `px-2` | A white face, the same recipe `OutlineIconButton` already carried |
| Version run | one Geist Mono 400 node reading `- v0.5.0`, `base/white` | dash in sans at `white/50`, version mono at `white/80`, both inheriting weight 500 | one mono 400 run at full white, dash inside it |
| Banner bottom border | 1px `base/border` #e5e7eb | `black/20` | `base-border` |
| Dismiss glyph | `base/primary-foreground` #f9fafb | `white/80` | `base-primary-foreground`, a name the token export already carried with no call site |
| `StatusTile` icon | ShieldBan `amber/600` #d97706 | `amber-700` | 600 on all three tones |

Geometry that was already right, recorded so nobody measures it twice: the `x`
and `shieldBan` glyphs both render the drawn vector exactly at `size={16}` (this
repo's `x` path spans 12 of 24 viewBox units, so 16px yields the drawn 8x8), and
the tile is 28px.

Left open deliberately: the frame draws no hover for the dismiss glyph, and at
`base/primary-foreground` the glyph has no colour headroom, so its hover became a
10% white scrim on a hit area grown with a negative margin. That is a choice, not
a sampled value.

### What this changes about the ask

G1, G2 and the `rounded-[2px]` half of G8 are **pure hygiene with no visual
effect** - canonicalising two names for one hex. They can be done blind, but they
do not make any screen match anything; they only stop the next reader from
treating two names as two intentions.

G3, G4, G6, G7 and the rest of G8 are **possible real mismatches**, and none of
them can be settled from the code. Combined with §D, the outstanding value count
is eleven, not five.

That count predates the 2026-08-21 read, which closed three of them: the update
banner's bottom border, `white/40`, and the tile icon stop.

---

## Appendix: why Linux-only window behaviour lives in Rust

Not a Figma finding. Recorded here because it was worked out twice during this
branch, both times to decide where a platform-specific window setting should go,
and the reasoning otherwise survives nowhere: `tauri.conf.json` is JSON and
cannot hold a comment.

**The trap.** Tauri supports `tauri.linux.conf.json`, merged over the base config
for Linux builds. It looks like the obvious place to change one window field per
platform. It is not, because `tauri-utils/src/config/parse.rs:180` merges with
`json_patch::merge`, which is **JSON Merge Patch (RFC 7396)** - and under that
spec a patch value that is not an object *replaces* the target. An array is not
an object, so arrays are never merged element-wise.

`app.windows` is an array holding one window object with thirteen fields. So this,
which reads like it flips a single flag:

```json
{ "app": { "windows": [ { "visible": true } ] } }
```

replaces the entire array with a one-element array carrying only `visible`.
Everything else is gone, **`label` included**, so `get_webview_window("main")`
returns `None` everywhere and the app breaks on the first run. This is not a
subtle drift risk; it fails immediately.

Done properly, all thirteen fields get copied into the Linux file, and the cost
becomes drift instead: change `width`, `resizable` or `decorations` in the base
config later and it silently will not apply on Linux, because the array was
replaced rather than merged. Nothing warns you.

**So the rule is:** `tauri.conf.json` stays the single definition of the window,
and anything that must differ on one platform is applied in Rust after the window
exists. The live example is the decoration repair in `src-tauri/src/lib.rs`
(`map_maximized_for_decorations` / `restore_after_repair`), which is Linux-gated
in code rather than in config for exactly this reason.

Two candidates were rejected on these grounds during this branch: a Linux-only
`decorations: false`, and a Linux-only `visible: true`.
