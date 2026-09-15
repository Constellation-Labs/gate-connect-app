# Flow validation: Tray (Figma `694:34005`) vs `TrayApp` / `Tray`

Read 2026-09-03. File `9FrccCojXy0f8QD8Wm5Lln`, canvas `      ↳ Tray ✅`
(`694:34005`). This is a **states-and-sequence** review. The visual/token audit
is `docs/review-figma-tray.md` and its findings are not repeated here.

## Frame inventory is complete

`get_metadata` on the canvas returns exactly four top-level frames, matching
the prior enumeration. Nothing else is on the page:

| Node | Name | Size |
| --- | --- | --- |
| 694:34006 | `Connect/partial` | 400 x 700 |
| 734:36992 | `Connect/full frame` | 400 x 1066.14 |
| 694:34167 | `Connect/routing` | 400 x 700 |
| 744:38073 | `Connect/menu` | 400 x 699.36 |

_(sections below appended as the read progressed)_

The four frames are three states plus one scroll-out, not four screens:

- `Connect/partial` and `Connect/full frame` are the **same mock** at two crop
  heights (amber master, "6 of 8"). The partial frame's list wrapper
  (`738:37544`) is 684 tall inside a 580 content box with no clip frame - it
  simply overflows. The full frame is that content un-cropped, adding the two
  sections the 700px window cuts off: `Other tools`, `Not installed`
  (collapsed) and the `Command-line tools` card. **Confirmed as a scroll-out,
  not a distinct state.**
- `Connect/routing` and `Connect/menu` are the same mock in the green
  all-routed state ("8 of 8"), the second with the footer menu open and the
  ellipsis button in its pressed variant. Both model the scroll region
  properly: a `scrollabe` clip frame 368x482 at y=98 (98 + 482 = 580, flush to
  the footer) around a 630-tall list, plus a 6px `scroll-indicator`.

## 1. Screen inventory

| Figma frame | Code path (file:line + trigger) | Verdict |
| --- | --- | --- |
| `694:34006` `Connect/partial` | `Tray.tsx:107-172` shell; `MasterCard` `Tray.tsx:189` amber branch, reached when `routed > 0 && routed < apps.length`. Data: `TrayApp.tsx:352` `master={proxy ? { on: proxy.running } : undefined}`, rows from `TrayApp.tsx:241-279`. | IMPLEMENTED |
| `694:34167` `Connect/routing` | `MasterCard` `Tray.tsx:186-188` green branch, reached when `all` = `apps.length > 0 && routed === apps.length` (`Tray.tsx:185`). Border swap at `:197`. | IMPLEMENTED |
| `744:38073` `Connect/menu` | `TrayMenu` `Tray.tsx:380-416`, gated `{menuOpen && <TrayMenu …>}` at `Tray.tsx:167`; state `TrayApp.tsx:70`, toggled `TrayApp.tsx:386`. | IMPLEMENTED except `Contact support` (`744:38201`), which has no code path - documented omission at `Tray.tsx:43-44`. |
| `734:36992` `Connect/full frame` | The same shell scrolled: `Tray.tsx:137` `overflow-y-auto` region. `Not installed` header -> `NotInstalledSection` `Tray.tsx:299-347`, gated `notInstalled.length > 0` at `:142`. `Command-line tools` card -> `CliCard` `Tray.tsx:352-373`, gated on `proxy.env_export_separable` at `TrayApp.tsx:358`. | IMPLEMENTED (scroll-out, not a screen) |

### Drawn sub-elements that are `hidden`/`opacity:0` - correctly absent from code

Both are deliberately-off elements, not missing ones.

- **Master card switch** `744:38099`, `opacity: 0`. Code omits it
  (`Tray.tsx:27-31`). Correct, and re-confirmed: the text block is a fixed
  162.45px inside a `justify-between` row, so removing it changes nothing.
- **`738:37368` / `738:37545`, `hidden="true"`.** A second accordion header at
  y=0, *above* the group list, whose visible label is **`Installed`**
  (`738:37369`), with count **`8 tools`** and an **`Icon / ChevronUp`**
  (`738:37373`). The design sketched a two-section accordion - a collapsible
  `Installed` list over a collapsible `Not installed` one - and shipped only
  the second. The code has no `Installed` collapse and should not grow one
  from a hidden node.
  **Correction to `docs/review-figma-tray.md`**, which read this node as "a
  hidden variant `738:37545` whose chevron points up and whose count reads
  '8 tools'", i.e. as the expanded state of the *Not installed* section. It is
  not: its label is `Installed` and it sits above the list, not below it. The
  expanded `Not installed` state is drawn **nowhere**, by either node.

## 2. States the code has that the flow does not draw

Reachability is judged from a shipped build (the new shell is the default per
CLAUDE.md), not from devtools.

| # | Code state | Trigger | Reachable? |
| --- | --- | --- | --- |
| 1 | Blank window (`return null`) | `TrayApp.tsx:343-348`, `!loaded` | Probably not - see Could not determine |
| 2 | `SignedOutNote` "Sign in to get started" | `Tray.tsx:424-443`, `signedOut={account === null}` (`TrayApp.tsx:370`) | **Yes, including wrongly** - see 2a |
| 3 | No master card at all | `Tray.tsx:134` `{master && …}`, `master` undefined when `proxyStatus()` fails (`TrayApp.tsx:139`) | Yes - see 2b |
| 4 | Empty list, no copy | `trayGroups = []` (`TrayApp.tsx:243`) and `notInstalled = []` | Yes (no tools installed, or a failed scan) |
| 5 | `Config drifted` row: amber phrase, switch **on** | `STATUS_TEXT.drifted` (`Sidebar.tsx:124`) with `on: t.status.kind === "connected" \|\| "drifted"` (`TrayApp.tsx:231`) | Yes, and it is the status that raises `ReviewConfigDialog` |
| 6 | Every row `Not protected - Checking` | `verdict.ts:79`, empty verdict map | Yes - and see Dead end 1 |
| 7 | Expanded `Not installed` rows | `Tray.tsx:328-343`, `notInstalledOpen` | Yes, by clicking the drawn header |
| 8 | `SecurityCard` (3 pill states: Live / Reconnecting / Offline) | `Tray.tsx:456-485`; `TrayApp.tsx:373-383` | Yes, always, once the feed's first read lands |
| 9 | `ErrorBanner` overlaying the list | `TrayApp.tsx:394-401` | Yes, on any failed toggle |
| 10 | Five dialogs over the 400px popover | `TrayApp.tsx:403-459` | Yes - see 2c |
| 11 | `busy` switches during a write | `app.busy = routingBusy` (`TrayApp.tsx:234`) | Yes, on every toggle |

**2a. A failed credential read is drawn as "signed out".**
`getAccount().catch(() => null)` (`TrayApp.tsx:135`) collapses "no account" and
"could not read the account" into the same `null`, and `signedOut` is that
`null`. CLAUDE.md documents exactly the scenario that produces it - a developer
or user who dismisses the macOS keychain prompt - and the tray's answer is
"Gate Connect needs a Gate account or API key", which is false. Same shape as
principle 6, one level up from a figure.

**2b. A failed proxy read silently changes the information architecture.**
`proxy === null` gives `groups = []` (`TrayApp.tsx:210-219`), so `trayGroups`
falls through to `[{ id: "all", label: "", apps }]` (`:243`). Three consequences
no frame covers: the master card disappears, the vendor eyebrows and
`n of m` counts disappear, and **every chat-domain row disappears** - `apps`
is built from `tools` only (`:221-236`), and domain rows are added solely
inside the `groups` loop (`:257-266`). A user whose `proxy_status` call failed
sees a shorter list than the one they had a moment ago, with no indication why.

**2c. The dialogs are drawn for a 1024px window and raised in a 400px one.**
`Modal` is `w-[600px] max-w-full` inside `absolute inset-0 … p-6`
(`Modal.tsx:194-195`), so width clamps to 352px and does not overflow. Height
is unbounded: no `max-h`, no scroll container. `ReviewConfigDialog`
(`dialogs.tsx:277-357`) assembles a 24px-padded panel with a header, up to two
`ModalSubject` blocks, a `break-all` path note and a two-paragraph note, in
652px of usable height. Whether it actually overflows depends on the runtime
strings, so this is INFERRED, not measured - but it is the tray's most likely
broken screen, and it is the one screen where the user approves a config
overwrite.

## 3. Sequence and transitions

**Opening.** Left-click-up on the tray icon toggles (`lib.rs:3769-3793`):
visible -> `hide()` + `POPOVER_VISIBLE = false`; hidden ->
`reveal_tray_window` (`lib.rs:2672-2682`), which anchors under the icon rect
(non-Linux, `anchor_under_tray`) or at the cursor (Linux, `anchor_at_cursor`),
then `show()` + `set_focus()`. Linux additionally reaches it from the
right-click menu's **Quick status** item (`lib.rs:3747`), because SNI/
AppIndicator often never fires the left-click path.

**Startup does not open this surface.** `lib.rs:3815-3830` reveals the **main**
window on a non-silent launch. Every visit to the drawn frames is an explicit
tray-icon click or the Linux menu entry.

**Closing - there is no click-outside dismiss.** The only exits are the
tray-icon click above and `WindowEvent::CloseRequested` (`lib.rs:3158-3162`),
which an undecorated window (`decorations: false`) rarely receives. Concretely:

- No `Focused(false)` arm exists anywhere in `src-tauri/src/`. Two comments
  assert one does - `lib.rs:2406` ("the `Focused(false)` handler hides the
  popover") and `lib.rs:3169` ("unlike the `Focused(false)` dismiss below").
  There is no such handler below, or anywhere.
- `POPOVER_PINNED` (`lib.rs:1287`) is **stored at four sites and read at
  none** (`:2400`, `:2411`, `:3452`, `:3816`). Its frontend commands
  `pinPopover` / `unpinPopover` are called only from `src/App.tsx` - the
  retiring popover shell - never from `TrayApp`.
- No `Escape` or outside-click listener in `Tray.tsx` / `TrayApp.tsx`.

With `alwaysOnTop: true` (`tauri.conf.json`), the consequence is a 400x700
undecorated panel that sits over every other application until the user
returns to the tray icon. That may be the intent for a toggle-style panel, but
it is not what the code says it does, and the pin machinery that only makes
sense alongside a blur-dismiss is now dead weight.

**The menu.** Opens from the footer ellipsis (`Tray.tsx:160-166`,
`OutlineIconButton onClick={onMenuToggle}`). Closes on that same button, or on
selecting an item (`TrayApp.tsx:321` `setMenuOpen(false)`). It does **not**
close on Escape or on a click outside: there is no scrim and no listener. Since
it is `absolute bottom-12 right-4 z-10` over the list (`Tray.tsx:388`), a click
on a row still visible beside it toggles that app's routing with the menu
still open. Item order matches the drawn order (dashboard, [support], docs,
quit).

**Expand app** (`TrayApp.tsx:310-317`): `revealMainWindow()` -> `api.ts:491`
-> command `reveal_popover` -> `lib.rs:2646` `reveal_popover_window`, which
shows the 1024x720 main window without repositioning it, then
`getCurrentWindow().hide()` on the tray. One-way: nothing in the main shell
reopens the tray.

**`SecurityCard` -> `onOpen` is the same `expand`** (`TrayApp.tsx:382`). The
card's own docstring says "the pane it opens has the detail this card
deliberately does not try to fit" (`Tray.tsx:379-382`), but `expand` takes no
pane argument and `reveal_popover_window` does not navigate - it reveals the
main window on whatever pane was last active. The promised sequence is not
implemented.

**Quit handoff** (stated, not audited - the dialogs live on the Overview page).
`TrayApp.tsx:328-338` calls `requestQuit()` (`api.ts:486` -> `request_app_quit`
-> `lib.rs:2832 request_quit`) and hides the tray. macOS/Windows: if any
integration reports `Connected | Drifted`, it buffers `PENDING_QUIT_TOOLS`,
calls `reveal_popover_window` and emits `quit-requested`, so the chooser is
raised in the **main** window; otherwise `app.exit(0)`. **Linux
(`lib.rs:2833-2834`): `app.exit(0)` unconditionally** - the tray's Quit exits
with no confirmation and no warning about config-routed tools left pointing at
a dead relay.

**The master card is read-only, and that is not a trap.** No switch is
rendered, so the tray cannot start the engine directly. But any row switch
does: `setDomainRouted` calls `ensureEngineRunning` (`useRouting.ts:208-212`,
`proxyEnable()` when `proxy && !proxy.running`), and `connect_tool` starts the
engine itself for config tools. So `Off` is escapable from the tray without the
invisible switch.

**Counts cannot disagree with switches.** `Tray.tsx:183-185` and `:231-233`
derive the master total and every group caption from the same row array. That
matches `Connect/partial` + `Connect/full frame`, which are internally
consistent (protected 1+2+1+2 of 2+3+1+2 = the drawn "6 of 8").
`Connect/routing` and `Connect/menu` are **not**: the master reads "8 of 8"
with every visible switch on, while the group captions still read "1 of 2" and
"2 of 3". Where the file disagrees with itself, the code follows the
consistent pair - correct, and the newer `738:*` label nodes are in
`Connect/partial` anyway.

**Row status vocabulary diverges on the suffix.** The drawn `Not protected`
row carries a **relative timestamp** - `Not protected - 3d ago`
(`738:37562`, rendered on the Claude Desktop row in both amber frames). The
code cannot produce that: `statusDetail` (`Sidebar.tsx:146-148`) returns
`status.detail` for `not-protected`, and the only values are
`Reopen required` / `Connection problem` / `Access problem` /
`Verification failed` (`verdict.ts:43-48`), `Checking` (`:79`), or the
write-failed detail (`:78`). A relative time attaches only to
`protected` (`status.since`, `Sidebar.tsx:139`). Either the design wants a
last-checked time on a non-routing row - which is a data question, there is no
such field on the verdict - or the frame's mock text is wrong. Worth raising;
do not "fix" the code to invent a timestamp.
(Related and benign: the drawn `Codex` row puts `Off` on the third
activity line rather than as a suffix, where the code prints
`Not routed - Off`. The third line is the dropped activity line, so the code's
placement is the only one available to it.)

**Polling** (`TrayApp.tsx:148-161`): 5s interval, skipped while
`document.hidden`, and made up on the `visibilitychange` edge - so hiding the
window pauses the reads and reopening takes one immediately. Correct for a
window that is hidden rather than destroyed.

## 4. Dead ends

**1. Rows stuck at `Not protected - Checking`, indefinitely.**
`refreshVerdicts` swallows failure (`TrayApp.tsx:88` `.catch(() => null)`) and
leaves the map empty, which `verdict.ts:79` renders as
`{ kind: "not-protected", detail: "Checking" }` on every row - amber, with the
master card reading `0 of N`. The polled path only re-fetches verdicts when
the tools/proxy signature *changed* (`TrayApp.tsx:123`
`if (changed) void refreshVerdicts()`). On an idle machine nothing changes, so
a single failed sweep at first load persists until a tools/proxy change or a
`proxy-state-changed` event. **There is no refresh affordance on this
surface** - the window shell has `onRefresh` (`Sidebar`), the tray has none -
so the only user-side recovery is to toggle something or restart. The screen
is simultaneously wrong about every row and about the headline count.

**2. `POPOVER_VISIBLE` leaks `true`, disabling the on-open reconcile.**
The flag is cleared only at `lib.rs:3161` (CloseRequested) and `:3785`
(icon-click hide), and set by the `Focused(true)` arm at `:3172`. Two problems
compound:

- `TrayApp`'s own hides never clear it - `expand` (`TrayApp.tsx:313`) and quit
  (`:334`) call `getCurrentWindow().hide()` directly.
- The handler is **not window-scoped**. The `main` branch above it
  (`lib.rs:3143`) is Linux-only and does not `return`, so the main window
  gaining focus also runs `POPOVER_VISIBLE.swap(true, …)`.

So after one **Expand app**, or once the main window has been focused, the next
tray open finds the flag already `true` and skips
`provider::reconcile_enabled()` - which `lib.rs:3163-3181` documents as the
mechanism by which "a tool installed since launch, e.g. Claude Code installed
after Gate Connect, gets wired up without a relaunch". It stays skipped until
the user next dismisses the tray with the icon click. Read from the code, not
observed running.

**3. The menu survives a hide/show cycle.**
`menuOpen` (`TrayApp.tsx:70`) is React state and the window is hidden, not
destroyed. Click the ellipsis, dismiss with the tray icon, click the icon
again: the popover comes back with the menu still open over the list. Same for
`notInstalledOpen` (harmless) and for an unresolved `routing.prompt` dialog,
which returns still covering the surface. Nothing resets transient UI state on
the visibility edge - only `redetect` runs there (`TrayApp.tsx:153-155`).

No **drawn** screen is unreachable. Every frame on the canvas has a live
trigger; the only drawn thing with no code path is the menu's `Contact
support` row, which is a documented omission rather than a dead end.

## 5. Could not determine

- **Whether the designer wired these frames together.** `get_metadata` on the
  canvas returns four top-level frames and no connector nodes, and the MCP
  surface used here does not report prototype reactions. Every transition in
  section 3 is read from the code and from frame content, not from a
  designer's wiring - so "the order matches" means the code's order is
  coherent and consistent with the frames, not that it was checked against
  arrows.
- **Whether `ReviewConfigDialog` overflows the 700px window.** `Modal` sets no
  `max-h` and no scroll container (`Modal.tsx:194-195`); the assembled height
  depends on runtime config strings. Needs a running tray with a drifted tool.
- **Whether the `!loaded` blank frame is ever visible.** The `tray` webview is
  created at startup with `visible: false`, so `loaded` is normally true long
  before the first reveal. But `lib.rs:3805` records that a hidden WKWebView
  reports visibility hidden and suspends `requestAnimationFrame`, and the
  initial `Promise.all` includes a keychain-reading `getAccount()`. Whether a
  first paint after a reveal can land before `loaded` I cannot tell from the
  code.
- **Whether the absent blur-dismiss is a decision or a regression.** The
  behaviour is unambiguous; the intent is not. Two comments and a four-write
  never-read flag describe the opposite behaviour, which reads like a
  migration from the old `main`-window popover to the `tray` window that left
  the dismiss handler behind - but a deliberate "this is a toggled panel, not
  a popover" would look the same in the code minus the stale comments.
- **Whether `Not protected` should carry a last-checked timestamp.** Needs a
  designer; there is no such field on `Verdict`.
