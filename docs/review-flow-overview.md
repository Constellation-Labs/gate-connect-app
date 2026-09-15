# Flow validation: `Flows / Overview` (canvas `116:26381`)

File `9FrccCojXy0f8QD8Wm5Lln`. This is a **states-and-sequence** audit, not a
pixel audit: the visual/token findings live in
`docs/review-figma-dialogs-overview.md` and are not repeated here. Read-only on
`src/` and `src-tauri/`.

**Frame list verified** against `get_metadata` on the canvas. The 21 frame ids
in the brief are exactly the 21 `<frame>` children of `116:26381` - none missing,
none extra. One correction: the brief calls it "23 top-level frames", but the
list it gives has 21 entries and the canvas has 21 frames. The other top-level
children are 17 loose `<text>` annotation labels and 8 `<line>` rules, which is
probably where the 23 came from.

## The canvas is five labelled flows, not 21 loose frames

The loose text labels are the designer's own row headings, and they are what
resolves the "duplicate frame" question. Taken from the label's `y` against
each frame row's `y`:

| label node | text | row `y` | frames on that row |
| --- | --- | --- | --- |
| `130:55858` | Overview / Main screens ✅ | 0 | `116:26405`, `116:26815`, `116:27241`, `116:27647`, `116:28041`, `191:79324` |
| `130:56971` | Overview / Dimensions ✅ | -1090 | `121:32875` |
| `727:36961` | Overview / Loading state ✅ | -1090 | `228:85602` |
| `130:55856` | Overview / Switching an organization ✅ | 1664 | `143:67827`, `130:54908`, `130:55349` |
| `130:59009` | Overview / Turn routing ON for an app that **has config drift** ✅ | 2757 | `130:57032`, `130:58021`, `130:58449`, `134:61253` |
| `135:63155` | Overview / Turn routing ON for an app that **was OFF** ✅ | 3792 | `135:61790`, `135:62207`, `135:62624` |
| `694:33533` | Overview / Turn routing ON for an app that was OFF ✅ *(mislabelled)* | 4837 | `694:31955`, `694:32488`, `694:33023` |

Two things fall out of this immediately:

1. **`overview-dimensions` (`121:32875`) is an annotation artboard, not a
   state.** It sits under its own "Dimensions" heading beside the loose
   `121:33871` "App dimensions: 1024x720px" text and three measurement `<line>`
   nodes (`121:33874`, `121:33875`, `121:33876`) drawing the 720px and 1024px
   callouts. Nothing in code corresponds to it and nothing should.
   **`overview-loading` (`228:85602`) is a real state**, under its own
   "Loading state ✅" heading.
2. **The `apply changes` / `close apps` / `change ready` duplicates are not
   sequential steps.** Each pair is the *same dialog drawn twice*, once in the
   drift row (`130:58021` / `130:58449` / `134:61253`) and once in the
   was-OFF row (`135:61790` / `135:62207` / `135:62624`). The drift row is
   four steps because it opens with `overview-review config`; the was-OFF row
   is the same flow minus that first step. The code models exactly this: the
   drift prompt is `useRouting`'s gate and the three-stage tail is
   `useRunningApps`, which runs after *any* successful config write regardless
   of whether drift was reviewed.
3. **The last row's heading is wrong in the file** - `694:33533` says "Turn
   routing ON for an app that was OFF" over three `overview-quit` frames.
   Taking the frames over the label.

## 1. Screen inventory

Every code path below was opened. "Trigger" is the state that produces the
screen, not a description of the button that leads to it.

### Main screens (row `y=0`)

The designer labelled each of these with a loose text node, so the state matrix
is the file's own words rather than my reading of the pixels.

| Figma frame | drawn state (from its own label) | code path + trigger | verdict |
| --- | --- | --- | --- |
| `116:26405` Overview/partly-routed-1 | update banner + `banner/alert/multiple-apps` + Partly routed | `AppShell.tsx:90-101` (topbar, `UpdateBanner` on `NewUiApp.tsx:1852` `update.available && !updateDismissed`, `RoutingBanner`); alert is `NewUiApp.tsx:2467` `notice && <AlertBanner …>` with `paging` at `:2481` when `notices.length > 1`; pane is `Overview.tsx:106` | IMPLEMENTED |
| `116:26815` …/menu-open | same, plus `topnav/menu` open | `Topbar.tsx:141` `TopnavMenu`, on `menuOpen` (`NewUiApp.tsx:234`, toggled at `:1849`) | DIVERGES - see §3.1 |
| `116:27241` Overview/partly-routed-2 | update banner + `banner/alert/single-app` | same alert component; `notices.length === 1` so `paging` is `undefined` (`NewUiApp.tsx:2481`) | IMPLEMENTED |
| `116:27647` Overview/routed-1 | update banner, no alert, "Routed" | `notices` empty -> no banner (`NewUiApp.tsx:2467`); `banners.tsx:91` `allProtected` branch | IMPLEMENTED (with a caveat, §5) |
| `116:28041` Overview/routed-2 | no update banner, no alert | `update.available` false **or** `updateDismissed` true (`NewUiApp.tsx:1852`, dismiss at `:1858`) | IMPLEMENTED |
| `191:79324` overview-chart tooltip | `chart/tooltip` on hover; also `banner/alert/multiple-apps`, "3 of 4 Apps" | `metrics.tsx:210` `const [hovered, setHovered] = useState<number|null>(null)`, set at `:264` `onMouseEnter`, rendered `:280-286` `<ChartTooltip …>` | IMPLEMENTED |

### Annotation and loading (row `y=-1090`)

| Figma frame | verdict |
| --- | --- |
| `121:32875` overview-dimensions | **DRAWN-ONLY - annotation artboard, not a state.** Its contents are byte-for-byte the same set as `116:26405` (`banner/update` + `banner/alert/multiple-apps` + `banner/status-protected` + "6 of 8 Apps"). It exists to carry the measurement callouts that sit *outside* it: `121:33871` "App dimensions: 1024x720px", `121:33872` "720px", `121:33873` "1024px", and the `<line>` rules `121:33874`-`121:33877`. Nothing in code corresponds to it and nothing should. |
| `228:85602` overview-loading | **IMPLEMENTED, and it is a real state.** Screenshotted: chrome, sidebar and banners draw real data; only the three stat tiles, the Messages chart and the tables are skeletonised. That is exactly the scope of `Overview.tsx`'s `pending` prop (`:92-94`, threaded to `StatTiles`, `MessagesChart`, `PolicyTable`, `SavingsTable`), driven by `NewUiApp.tsx:2439` `pending={activity.view === null && activity.failure === null}`. |

### Switching an organization (row `y=1664`)

The three frames are **two alternative states of step one plus step two**, not
three steps. Measured from the dialog subtrees:

| Figma frame | dialog node | what it draws | code path + trigger | verdict |
| --- | --- | --- | --- | --- |
| `143:67827` (x=0) | `143:68237` 520x356 | Switch organization; CircleCheck on the **third** row (`143:68316`, Chad's organization) and, screenshotted, the **primary drawn disabled** (pale) | `dialogs.tsx:66` `SwitchOrganizationDialog`, at `NewUiApp.tsx:2119` on `settings.prompt.kind === "switch-org"`; `selectedId` defaults to `account.org_id` (`useSettingsActions.ts:214`) and the primary carries `disabled: currentId !== undefined && selectedId === currentId` (`dialogs.tsx:94`) - i.e. **the dialog as it opens, on the org already in use** | IMPLEMENTED |
| `130:54908` (x=1120) | `130:55314` 512x380 | same dialog, CircleCheck on the **first** row (`130:55329`, Acme Engineering) and the **primary drawn enabled** (solid `base/primary`) | same component; `selectedId` moved by `selectOrg` (`useSettingsActions.ts:226`, wired at `NewUiApp.tsx:2123`), which un-disables the primary - i.e. **after the user picks a different org** | IMPLEMENTED |
| `130:55349` (x=2235) | `130:55755` 512x244 | "Organization switched" / "Gate Connect is now using Acme Engineering", single 58px Done | `dialogs.tsx:248` `OrganizationSwitchedDialog`, at `NewUiApp.tsx:2146` on `prompt.kind === "org-switched"`, set by `confirmSwitchOrg` **after** `setOrg` + `getAccount` resolve (`useSettingsActions.ts:230-245`) | IMPLEMENTED |

**The disabled button settles which frame is which**, and it is measured rather
than inferred: I screenshotted both. `143:67827` draws `Switch organization`
greyed out beside the selected `Chad's organization`; `130:55314` draws it solid
blue beside the selected `Acme Engineering`. That is exactly the rule at
`dialogs.tsx:94` - the primary is disabled while the selection is still the org
the device already uses - so the two frames are the **before and after of
picking**, and `143:67827` is unambiguously the freshly-opened state. The
confirmation agrees: `130:55761` says the app is *now using* Acme Engineering,
the org selected in the enabled frame.

Entry point exists and is on this pane: `AppShell`'s sidebar org row
(`NewUiApp.tsx:1862-1865` `onSwitchOrg`), so drawing this flow over the Overview
is right.

### Turn routing ON for an app that has config drift (row `y=2757`)

| Figma frame | dialog node | code path + trigger | verdict |
| --- | --- | --- | --- |
| `130:57032` overview-review config | `130:57442` 600x418 "Review Codex configuration" | `dialogs.tsx:277` `ReviewConfigDialog`, at `NewUiApp.tsx:1949` on `routing.prompt?.kind === "drift"`; the prompt is raised by `useRouting.ts:235-241` `if (!force && tool?.status.kind === "drifted") await ask({kind:"drift", …})` inside `setAppRouted` | IMPLEMENTED, plus two undrawn blocks (§2) |
| `130:58021` overview-apply changes | `130:58427` 600x318 "Apply changes to running apps?" (subject = Codex) | `dialogs.tsx:362` `ApplyChangesDialog`, at `NewUiApp.tsx:2018` on `runningApps.stage?.kind === "offer"`; stage set by `useRunningApps.ts:73` after `runningAgents(slugs)` returns a non-empty list, called from `routeApp` (`NewUiApp.tsx:1023`) only when the write actually landed | IMPLEMENTED |
| `130:58449` overview-close apps | `130:58855` 600x318 "Close affected apps now?" | `dialogs.tsx:400` `CloseAppsDialog`, at `NewUiApp.tsx:2024` on `stage.kind === "confirm"`, from `goToConfirm` (`useRunningApps.ts:80`) | IMPLEMENTED |
| `134:61253` overview-change ready | `134:61659` 512x244 "Change is ready" / "Codex closed successfully" | `dialogs.tsx:441` `ChangeReadyDialog`, at `NewUiApp.tsx:2030` on `stage.kind === "done"`, from `closeApps` (`useRunningApps.ts:99`) | IMPLEMENTED |

### Turn routing ON for an app that was OFF (row `y=3792`)

Same three tail frames, redrawn naming OpenCode instead of Codex. Measured
side by side: `135:62184` vs `130:58427`, `135:62601` vs `130:58855`,
`135:63018` vs `134:61659` are the same dialog at the same size with the same
copy, differing only in the app name.

| Figma frame | verdict |
| --- | --- |
| `135:61790` overview-apply changes | IMPLEMENTED - the same `ApplyChangesDialog`. Reached without the drift dialog because `useRouting.ts:235`'s gate does not fire on a non-drifted tool. |
| `135:62207` overview-close apps | IMPLEMENTED - same `CloseAppsDialog`. |
| `135:62624` overview-change ready | IMPLEMENTED - same `ChangeReadyDialog`. Note the frame contradicts itself: its subtitle `154:71516` still reads "Codex closed successfully" while its body `135:63027` reads "Open OpenCode …". Code takes the app it actually closed (`closedLabel`, `NewUiApp.tsx:2567`), which is right. |

**So the duplicate frames are not sequential steps: they are one flow drawn
twice, once with a drift-review prologue and once without.** The code models
precisely that split - the drift prompt is `useRouting`'s *gate* (it blocks the
write) while the three-stage tail is `useRunningApps`, which runs after any
successful write regardless. `useRunningApps.ts:18-21` states that distinction
in as many words.

### Quit (row `y=4837`)

| Figma frame | dialog node | code path + trigger | verdict |
| --- | --- | --- | --- |
| `694:31955` overview-quit | `694:32272` **600x428** chooser: "Quit Gate Connect?" / "3 protected apps are still routed through Gate", two `ModalChoice` rows, `SAFEST` pill on the first, Cancel (70) + primary (97) | `dialogs.tsx:1562` `QuitDialog`, at `NewUiApp.tsx:1938` on `quit?.kind === "choose"` | IMPLEMENTED |
| `694:32488` overview-quit | `694:33002` **536x232** "Safe to close Gate Connect", note `694:33021` "Tools are disconnected and their previous settings are restored. Setup will be waiting…" | `dialogs.tsx:1653` `QuitSafeToCloseDialog` with `disconnected: true`, at `NewUiApp.tsx:1931` on `quit?.kind === "confirm"` | IMPLEMENTED |
| `694:33023` overview-quit | `694:33340` **536x232**, same title, note `694:33348` "Routing settings were left in place. Some tools may need Gate Connect running…" | same component with `disconnected: false` | IMPLEMENTED |

The two 536 frames are the **two branches of step two**, not two steps: same
size, same position, same title, differing only in the one note. Both frames
also carry a hidden subtitle (`694:33008` / `694:33346`, `hidden="true"`), and
the code passes no subtitle - correct, and an instance of the opacity/hidden
trap the brief warns about.

## 2. States the code can produce that the flow does not draw

All reachability judgements below are from the call sites named, not from
guessing.

### 2.1 A whole dialog: `QuitLeftBehindDialog` (AG-596's failure branch)

- **Code:** `dialogs.tsx:1698`, rendered at `NewUiApp.tsx:1923` on
  `quit?.kind === "left-behind"`.
- **Trigger:** `runDisconnect` (`NewUiApp.tsx:911-925`) branches on the return
  of `disconnectToolsForQuit()`:
  `failed.length > 0 ? {kind:"left-behind", tools: failed} : {kind:"confirm", disconnected:true}`.
  The Rust side returns exactly that list (`src-tauri/src/lib.rs:2901`, with the
  contract stated at `:2894-2899`: "Returns the display names of any tools it
  could **not** return to their own settings … the caller must not report the
  quit as tidy, and must not quit without saying so").
- **Undrawn:** confirmed. The canvas has three `overview-quit` frames and all
  three are accounted for (chooser + two confirm branches). There is no fourth.
- **Reachable:** yes, on any partial teardown.
- **Verdict: the AG-596 branch exists and is correct, and the dialog is still
  undrawn.** This is the one undrawn state that carries a written requirement,
  so it is the one worth a frame.

### 2.2 The certificate gate sits *inside* both drawn routing rows

- **Code:** `NewUiApp.tsx:1959-1988`, an inline `Modal` on
  `routing.prompt?.kind === "trust"` ("Trust the Gate certificate?").
- **Trigger:** `useRouting.ts:179-191` `ensureCaTrusted`, awaited by
  `setAppRouted` at `:243` - i.e. **after** the drift gate and **before**
  `connectTool`. So in the drift row it lands between
  `overview-review config` and `overview-apply changes`; in the was-OFF row it
  lands before `overview-apply changes`.
- **Reachable:** yes, and on the highest-traffic path there is - the first time
  a user turns routing on with the CA untrusted.
- The code says so itself at `:1960-1962` ("Not in the Figma: the new design
  has no certificate surface, and connecting cannot proceed without one"). The
  sibling `untrust` modal (`:1990-2014`) is undrawn too but is reached from
  Settings, not from this pane.

### 2.3 `ReviewConfigDialog` ships two blocks the frame does not draw

`130:57442` draws exactly one `ModalSubject` (`130:57450`, "Existing custom
configuration" / `DETECTED`) and one note (`130:57459`). The code adds two more
between them:

| block | code | reachable when |
| --- | --- | --- |
| second `ModalSubject` "What Gate would write instead", `Gate route` green pill | `dialogs.tsx:322-330` | `gateRoute` non-null, i.e. `proxy?.relay_base_url` is bound (`NewUiApp.tsx:1955`) - the normal case |
| note "The file that changes:" + the mono path | `dialogs.tsx:332-343` | `configLocationFor(tools, slug)` names a single file (`NewUiApp.tsx:1957`) |

Both are argued in place (the second is the AG-564 requirement to name the
configuration location; the first is the transparency argument that approving
an overwrite you cannot see is not approval). Consequence for the drawing: the
shipped dialog is materially taller than the drawn 418px.

### 2.4 The Overview pane's own undrawn furniture

| state | code | reachable |
| --- | --- | --- |
| **`InstallationPicker`** in the header's `scope` slot | `Overview.tsx:102-104`, filled at `NewUiApp.tsx:2444` | **always.** The drawn header `116:26487` has exactly two children - `116:26488` "Overview" and `116:26489` "Last 24 hours". Grepping the whole 595KB metadata dump for `install`, `machine` or `picker` returns **zero** hits, so this control is drawn nowhere on the canvas. It is the largest undrawn element on the pane. |
| `ActivityGaps` in the alert slot | `NewUiApp.tsx:2503-2510` | on any partial or failed activity read |
| "Policies couldn't be read" / "No policies configured" | `Overview.tsx:178-180` | unreadable section / empty org |
| "Token savings couldn't be read" / "No savings configured" | `Overview.tsx:260-262` | same |
| `action: null` -> "Not set" in the Action column | `Overview.tsx:217-219` | a policy that names no single action, which the comment calls the common case |
| `allow` action pill (neutral) | `Overview.tsx:70` | an `allow` policy; the comment says the file draws only the three enforcing actions |

The four table states are what CLAUDE.md principle 6 predicts: the file draws
only the no-traffic case, and the failure/empty wordings are inferred from the
rule. Consistent with the contract, not a divergence.

### 2.5 Undrawn states in the drawn dialogs

| state | code | reachable |
| --- | --- | --- |
| Quit chooser primary reads **"Continue"** | `dialogs.tsx:1601-1604` | whenever `choice === "leave"`. The frame draws only the first row selected, so only "Disconnect" is drawn; `:1596-1600` says the second label is inferred. |
| `Working…` on the three quit dialogs | `dialogs.tsx:1601`, `:1690`, `:1729` | while `quitBusy` |
| Quit with **nothing** routed - no dialog at all, straight exit | `NewUiApp.tsx:1698` `if (routedForQuit.length === 0) void quitApp()` | an install with no connected or drifted tool |
| `ApplyChangesDialog` / `CloseAppsDialog` with **more than one** subject row | `dialogs.tsx:378-388`, `:417-431` map over `apps` | the master toggle: `toggleMaster` calls `offerAfterChange()` with no filter (`NewUiApp.tsx:1041`, `useRunningApps.ts:64-67`). Both drawn frames draw exactly one row. |
| **A failed `closeApps`** | `useRunningApps.ts:100-107` keeps `stage` on `confirm` and calls `onError`, which sets `actionError` (`NewUiApp.tsx:991`) | a signal that fails. See §4.3 - this one is a defect, not just undrawn. |

## 3. Sequence and transitions

### 3.1 The quit flow - **order is correct**

Verified against the code, not inferred:

| step | what runs | what it does NOT do |
| --- | --- | --- |
| **1. chooser** `QuitDialog` (`694:32272`) | `onContinue` -> `continueQuit` (`NewUiApp.tsx:933-942`). `choice === "leave"` -> `setQuit({kind:"confirm", disconnected:false})`, an explicit no-op step. `choice === "disconnect"` -> `runDisconnect()` -> `await disconnectToolsForQuit()`, **the real teardown**, then `left-behind` or `confirm`. | does not exit |
| **2. confirmation** `QuitSafeToCloseDialog` (`694:33002` / `694:33340`) | `onClose` -> `finishQuit` (`:944-947`) -> `quitApp()` -> Rust `quit_app` -> `app.exit(0)` (`lib.rs:2875-2878`) | is the only thing that exits |

**So step one carries out the choice and step two quits - the drawn order, not
the reverse.** That is what earns the past tense in `694:33021` ("Tools **are**
disconnected and their previous settings **are** restored"), and the notes are
branch-correct: `disconnected: true` -> `694:33021`'s wording,
`disconnected: false` -> `694:33348`'s. `dialogs.tsx:1548-1551` and `:1633-1640`
state the same invariant in the source. `finishQuit` is called from exactly two
places (`NewUiApp.tsx:1927` "Quit anyway", `:1933` "Close Gate Connect") and
from nowhere in step one.

**Cancel** at either step: `cancelQuit` (`:949-952`) clears the state and
`quitBusy`, returning to whatever pane was open. At step two that means the
tools are already disconnected and the app stays running -
`dialogs.tsx:1642-1646` says this is deliberate and that the note has just told
the user as much. Correct, if slightly odd.

**Two entrances, and they disagree on Linux.** This is the one real sequence
divergence in the quit flow:

| entrance | path | Linux behaviour |
| --- | --- | --- |
| window menu -> Quit Gate Connect | `Topbar.tsx:46-48` `QUIT_ITEM` -> `onMenuSelect` (`NewUiApp.tsx:1686-1707`), which derives `routedForQuit` in JS (`:1676-1684`, `connected \|\| drifted`) and opens the chooser | **shows the three-frame flow** - `onMenuSelect` never consults `platform`, though `usePlatform()` is in scope at `:366` |
| tray / popover Quit | Rust `request_quit` (`lib.rs:2832-2858`), which buffers the same `Connected \| Drifted` set, reveals the window and emits `quit-requested`; the frontend sweeps it at `:750-767` | **exits outright** - `#[cfg(not(any(target_os = "macos", target_os = "windows")))] app.exit(0)` at `:2833-2834` |

The Rust rationale is sound and documented (`lib.rs:2820-2826`: on Linux the
engine lives in a detached daemon that outlives the GUI, so the relay port
keeps serving and there is nothing to warn about). Which means it is the
**window menu** that is wrong on Linux: it asks "how do you want to quit?" and
recommends a teardown about a failure that cannot happen there. Worth raising.
The two paths agree on the *set* of tools (both filter Connected|Drifted) and
on the empty case (both exit outright), so this is only the platform gate.

Two smaller sequencing points, both correct:

- The mount-and-nudge sweep only ever opens **step one**, and only when no quit
  is already in flight (`NewUiApp.tsx:757-759` `setQuit((q) => q ?? {…})`, with
  the reason in the comment). A tray Quit landing mid-flow cannot throw the
  user back to the chooser.
- The quit state outranks every other overlay (`:1922`, first three branches of
  the dialog chain). A tray Quit arriving while a `useRouting` gate is open
  covers that gate with the chooser while its promise stays pending -
  recoverable, because cancelling the quit re-renders the gate and it resolves
  normally.

### 3.2 The running-apps tail - order matches, one loop

`offer` -> `confirm` -> `done`, and nothing is signalled until the second
confirmation:

| from | control | goes to |
| --- | --- | --- |
| `ApplyChangesDialog` | secondary "Yes, close affected apps" (`NewUiApp.tsx:2021`) | `goToConfirm` - **state change only**, no processes touched (`useRunningApps.ts:80-82`) |
| `ApplyChangesDialog` | primary "No, I will reopen later", and Escape (`dialogs.tsx:383`) | `dismiss` - sequence over |
| `CloseAppsDialog` | secondary "No, I will close later", and Escape (`dialogs.tsx:414`) | `goBack` - **back to the offer**, not out |
| `CloseAppsDialog` | primary "Yes, close apps" (`destructive: true`) | `closeApps` -> `closeRunningAgents(stage.slugs)` (`lib.rs:2281`) -> `done` |
| `ChangeReadyDialog` | "Done", and Escape | `dismiss` |

Two observations:

- **`CloseAppsDialog`'s escape hatch returns you to the offer.** Its label says
  "later", not "back", and no frame draws a back arrow, so a user who reads the
  label as "dismiss this" has to answer twice - "No, I will close later", then
  "No, I will reopen later" - to get out. Escape behaves the same way, since
  `onDismiss` is also `onGoBack`. Not a dead end, but two dismissals for one
  intention, and the copy does not prepare the user for it.
- The filter is preserved across the whole tail (`stage.slugs` from the offer is
  what `closeApps` passes at `useRunningApps.ts:94`), so the confirmation cannot
  kill a wider set than the one on screen. Correct, and the comment says why.

### 3.3 The drift gate resolves through the promise, not through `force`

`ReviewConfigDialog`'s two buttons (`NewUiApp.tsx:1957-1958`):

- `onKeep` -> `resolvePrompt(false)` -> the `ask` promise rejects with `Declined`
  (`useRouting.ts:132`) -> `setAppRouted` swallows it without marking the row
  failed (`:262-268`) -> returns `false` -> `routeApp` skips
  `offerAfterChange` (`NewUiApp.tsx:1021`). So Keep really does return to the
  pane with nothing written and no follow-up dialog. Correct.
- `onReplace` -> `resolvePrompt(true)` -> the promise resolves and execution
  continues **in place** into `ensureCaTrusted()` and `connectTool()`
  (`useRouting.ts:243-244`).

**`force` is never passed as `true` anywhere.** `rg`'d every call site:
`NewUiApp.tsx:1122`, `:1246`, `:1654` and `TrayApp.tsx:203`, `:303` all take the
two-argument form. So `useRouting.ts:215-216`'s docstring - "`force` skips the
drift gate, which is how the review dialog's 'Replace config and protect' comes
back in" - describes a mechanism that is not the one in use. The parameter is
dead and the comment is stale. The behaviour is right; only the explanation is
wrong.

### 3.4 The org switch - order matches

`openSwitchOrg` -> chooser -> `selectOrg` moves the selection in place ->
`confirmSwitchOrg` awaits `setOrg` **and** `getAccount` before setting
`org-switched` (`useSettingsActions.ts:230-245`). So the confirmation is
reached only after the switch landed, which is what lets `130:55761` say "is
**now** using Acme Engineering". Cancel at either step is `dismissPrompt`.
One extra guard the flow does not draw: `openSwitchOrg` returns without opening
anything when the account has fewer than two organizations
(`useSettingsActions.ts:209-211`).

## 4. Dead ends

### 4.1 Drawn with no way in: `Contact support`

The drawn `topnav/menu` (`116:27225`) has **four** rows; the shipped one has
three. Screenshotted the node to be sure: Visit dashboard, Contact support and
Read Gate docs each with an external-link glyph, then Quit Gate Connect in red
with none.

- `116:27231` **"Contact support"** has no code path. `Topbar.tsx:42-45`'s
  `MENU_ITEMS` contains only `dashboard` and `docs`, and `:33-39` says why -
  there is no address yet, and a menu entry that opens a broken page is worse
  than one that is missing (AG-598, to be restored together with the Settings
  row). **DRAWN-ONLY, deliberate, ticketed.** It is the only such item on the
  canvas.
- Order otherwise matches: dashboard, docs, quit, with quit last and separated.
- One trap worth recording rather than fixing: the drawn quit row *contains* an
  `Icon / SquareArrowOutUpRight` (`751:38278`, no `hidden` attribute in the
  metadata) but the frame **renders without it**. `Topbar.tsx:46-48`'s claim
  that the glyph is absent "which is how the frame renders it" is correct about
  the render and would look wrong to anyone reading the node tree alone. Same
  class of trap as the hidden `OPEN` pill and the hidden subtitles.

### 4.2 No code state with no way out

Checked every state on this canvas:

- All three quit stages have a `Cancel`/dismiss to `null`
  (`NewUiApp.tsx:1928`, `:1934`, `:1945`), and `left-behind` additionally
  offers `Try again` and `Quit anyway`.
- The three running-apps stages all reach `dismiss`, though `confirm` needs two
  presses (§3.2).
- Both routing gates resolve on dismiss (`onKeep` / "Not now" ->
  `resolvePrompt(false)`), which rejects the promise and runs
  `setAppRouted`'s `finally` -> `settle()` -> `setBusy(false)`.
- The one shape that *would* strand the window - two overlapping `ask` calls,
  where `setDecide` (`useRouting.ts:128`) overwrites the first resolver and
  leaves its promise pending forever, so `busy` never clears and every switch
  in the app goes dead - **cannot happen**: every writer checks `if (busy)` and
  returns before `setBusy(true)` (`useRouting.ts:223`, `:291`, `:340`, `:376`,
  `:407`, `:432`), and `ensureCaTrusted`'s ask only runs after the drift ask has
  already resolved. `settle`'s docstring documents having been bitten by the
  stuck-`busy` symptom by another route.

### 4.3 One real defect: a failed `closeApps` reports behind the scrim

Not a dead end, but the closest thing to one on this flow. When
`closeRunningAgents` throws, `useRunningApps.ts:100-107` deliberately stays on
`CloseAppsDialog` and hands the error to `onError`, which sets `actionError`
(`NewUiApp.tsx:991`). `actionError` renders as the shell's banner, and
`AppShell.tsx:103` puts `{notice}` **before** `{dialog}` in the tree while
`Modal` is `absolute inset-0` with a `bg-neutral-900/40` scrim
(`Modal.tsx:189`). So the explanation is dimmed behind the dialog that caused
it, and the dialog itself is unchanged - the user presses "Yes, close apps",
sees nothing move, and has no stated reason. The "stay on the confirmation so
the user can try again" intent is right; the surface it reports on is wrong.
Undrawn, so no frame settles it.

### 4.4 `ApplyChangesDialog`'s focus, and a correction to the prior audit

The bug is real and the sequence that reaches it is:

1. a switch write lands - `routeApp` (`NewUiApp.tsx:1013-1026`) or
   `toggleMaster` (`:1032-1044`);
2. `offerAfterChange` finds a running process and sets `offer`
   (`useRunningApps.ts:68-74`);
3. `ApplyChangesDialog` mounts (`NewUiApp.tsx:2018`). It passes no
   `destructive` primary and no `initialFocus`, so `Modal`'s rule
   (`Modal.tsx:182-186`, `primary?.destructive ? safeRef : initialFocus`) has
   nothing to key on and `useFocusTrap` falls back to the first focusable in
   DOM order. DOM order in `Modal` is secondary, middle, primary
   (`:249-288`), the children are non-focusable, and this dialog **inverts** the
   weighting on purpose (`dialogs.tsx:357-360`) - so the first focusable is
   `Yes, close affected apps`.

Focus does open on the wrong button, and the fix is a primary-side
`initialFocus` that `Modal` does not currently expose.

**But the consequence is milder than
`docs/review-figma-dialogs-overview.md` states.** That report says "a keyboard
user who opened this dialog with Enter closes Codex by pressing Enter again".
Enter on that button calls `onCloseApps`, which is wired to
`runningApps.goToConfirm` (`NewUiApp.tsx:2021`) - a pure stage change.
`closeRunningAgents` is reached only from `CloseAppsDialog`'s primary
(`:2028`), and *that* dialog does mark its primary `destructive: true`
(`dialogs.tsx:420-424`), so it safe-focuses `No, I will close later`. A second
Enter therefore goes **back to the offer**, not to a kill. Repeated Enter
oscillates offer -> confirm -> offer indefinitely and never closes an app.

So: a mis-focus and a keyboard trap that cannot be escaped with Enter, but
**not** a stray-Enter data loss. Worth correcting in the record, because the
severity decides whether this blocks a release.

## 5. Could not determine, and file-internal contradictions

- **`routed-1` / `routed-2` draw a state the code cannot produce.** Both frames
  label the banner "Routed" while still reading "6 of 8 Apps". `banners.tsx:91`
  derives the word from `protectedCount === totalCount`, so "Routed" can only
  ever appear beside "8 of 8". The drawn pair is a frame that was relabelled
  without its count being updated; the code is right and should not follow it.
- **`135:63018`'s subtitle contradicts its own body** ("Codex closed
  successfully" over "Open OpenCode …"). Code names the app it actually closed
  (`closedLabel`, `NewUiApp.tsx:2567`).
- **`694:33533` mislabels the quit row** as "Turn routing ON for an app that was
  OFF". Took the frames over the label.
- **The two `overview-switch org` selection frames are different widths** -
  `143:68237` is **520** and `130:55314` is **512** - with different row heights
  (54 vs 62). `Modal`'s `ModalWidth` union (`Modal.tsx:66`) has no `520` member
  and `SwitchOrganizationDialog` passes `512` (`dialogs.tsx:88`). Taking the 512
  is defensible (it is the value the union and CLAUDE.md already carry, and
  `130:55349`'s confirmation is 512 too), but the newer id is the 520 one, so
  which the designer means is **not determinable from the file**. This does *not*
  affect the flow reading - the disabled/enabled primary settles which frame is
  which regardless of width - so it belongs with the pixel audit, and it was not
  covered by `docs/review-figma-dialogs-overview.md`, whose inventory lists only
  the 512.
- **Whether the drift row's certificate gate was omitted or forgotten.** The
  code says the design has no certificate surface at all (`NewUiApp.tsx:1960`),
  and nothing on this canvas draws one, but nothing states it was a decision
  either. §2.2 reports the gap; the intent is a designer question.
- **The 6-cell main-screen matrix is drawn 4 ways.** {update, no update} x
  {2+ alerts, 1 alert, no alerts} would be six; the file draws update+2,
  update+1, update+none and none+none. The two missing combinations
  (no update + alerts) are ordinary and reachable; nothing suggests they were
  excluded on purpose.
- One `get_screenshot` call failed with an unexplained server error and
  succeeded on retry. Noted because three earlier passes on this canvas were
  lost to the same fault.

## Method and scope

Read-only on `src/` and `src-tauri/`; the only file written is this one. The
frame inventory came from a single `get_metadata` on `116:26381`, extracted to
a local XML file and queried with `grep`/`sed` rather than read into context;
per-frame subtrees were read from that dump. Five nodes were screenshotted
(`228:85602`, `116:26405`, `116:27225`, `143:68237`, `130:55314`) because
skeleton scope, banner content, menu rows and button *enabled state* are visible
in a render and not in metadata. Every code path named
above was opened at the line cited. No screen and no code path is asserted here
that was not read.
