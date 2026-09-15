# Review round 2 - PR #244 `fix/ui-bug-sweep` - CORRECTNESS lens

Base `feat/new-app-ui`, head at `8c03a4c` (hand-resolved merge of the base into
the branch). 22 files. Verified against the working tree, not against the reply
thread.

## Summary

Nine of the eleven round-1 findings are genuinely fixed, and five of those I
confirmed load-bearing by mutating the source and watching the specific
assertion fail. The Copilot dispute is decided in the author's favour with
empirical evidence.

Two findings are not closed:

- **`src/NewUiApp.tsx:1697` is INCOMPLETE.** The listener now fetches the
  summary before opening, which closes the reviewer's *first* path. Their
  *second* path - the window not being past setup - is untouched, and it is the
  one that still pops a dialog nobody asked for. The author's own docstring at
  `:1726-1731` describes this path in detail and then does nothing about it. The
  `else if` parenthetical the author calls "no longer a pop-later" still is one.
- **`src/TrayApp.tsx:298`'s second half is not fixed, and is now
  deterministic.** The window-scoped buffer is real and correct, but the mirror
  case the reviewer called "worse" - a failure surfacing as an unexplained
  banner the next time the popover opens - went from a race the tray sometimes
  lost to a guarantee, because every routing-down failure is now queued for the
  tray on purpose and nothing ever clears `actionError`.

Both merge resolutions are sound. Resolution 1's reasoning holds all the way
down, and the version that was dropped would have been actively worse. Resolution
2's layout is coherent and the unreachable-primary bug does not return. Two
residuals are recorded under New findings, one of them the tray-side twin of
round 1's own finding that nobody has closed.

Full suite green throughout: `vitest run` 986/986 in 58 files, `tsc --noEmit`
clean, and the three new Rust tests pass individually. `git status` clean.

---

## Round 1 verification

### 1. `src/components/gc/dialogs.apply.test.tsx:63` (Copilot) - **DISPUTE UPHELD: the author is right, Copilot is wrong**

`vi.stubEnv` does control `import.meta.env.DEV` here, and the two tests are
genuinely mutually exclusive on the flag.

Evidence, all measured on this tree (vitest 2.1.9, `vitest.config.ts` with no
`define`):

- Probe spec: default `import.meta.env.DEV` is boolean `true`; after
  `vi.stubEnv("DEV", false)` it is boolean `false`.
- Forcing the gate on - `src/components/gc/dialogs.tsx:106`, `import.meta.env.DEV`
  → `true` - fails `dialogs.apply.test.tsx:56` ("ships neither route"). That
  assertion can only fail if the flag was falsy under the stub.
- Forcing the gate off - same line, → `false` - fails
  `dialogs.apply.test.tsx:45` ("names both routes in a development build").

So both directions are load-bearing and neither is environment-dependent.
Copilot's premise - that `stubEnv` writes only `process.env` - does not hold for
Vitest's `import.meta.env` proxy. No change needed.

One nit worth recording next to the tests, because it is the trap that would make
Copilot right: the stub is **value-typed**. `vi.stubEnv("DEV", "false")` (the
string) comes back as `true`, so the flag survives the stub. The file passes
booleans and is correct; a future author reaching for a string would silently
re-open exactly the hole Copilot claimed. **L**

### 2. `src/TrayApp.tsx:298` (destructive drain, both shells racing) - **PARTLY FIXED / second half NOT FIXED**

The race is genuinely gone. `PENDING_BACKEND_ERRORS` is per-label
(`src-tauri/src/lib.rs:1773`), `report_backend_error` queues a copy for each
label in `ERROR_SINK_LABELS` (`:1785-1799`), and `drain_backend_errors` keys on
`window.label()` (`:1832-1841`). Verified load-bearing: narrowing the queue loop
to `["main"]` fails `each_shell_drains_its_own_copy_of_a_failure` with its own
message ("the tray must still see it"). The `Mutex<Option<HashMap>>` shape is
forced by `HashMap::new` not being `const`, and is fine.

What is not fixed is the half the reviewer called worse. `actionError` is set
from the sweep at `src/TrayApp.tsx:296-306` and cleared only by the banner's own
dismiss (`:933`) - never on hide, never on reveal, never when routing recovers.
Under one shared buffer the hidden tray *sometimes* took a failure it had no
business showing; now it is queued a copy of **every** routing-down failure by
construction, so it always does. See New findings H-2/M-2.

Also: the test reimplements the drain locally rather than calling
`drain_backend_errors`, so the command's own keying is untested - a hardcoded
`"main"` inside the command would leave the test green. The docstring is honest
about why (a unit test cannot build a `tauri::Window`), so this is a coverage
note, not a defect. **L**

### 3. `src/NewUiApp.tsx:2056` (green unreachable from the family switch) - **VERIFIED FIXED** (by the merge, not by `a709e96`)

The `bannerApps` memo the author added was dropped in the merge in favour of the
base's `desiredApps` (`src/NewUiApp.tsx:2166`). The finding is still satisfied,
and the base's version is the better of the two. Full evidence in
**Merge resolution review 1**. No dangling `bannerApps` reference survives
(`rg bannerApps` is empty; `tsc --noEmit` clean).

### 4. `src/NewUiApp.tsx:1697` (no-op now, pop later) - **INCOMPLETE, and the parenthetical is OVERCLAIMED**

The listener at `src/NewUiApp.tsx:1737-1747` does now `await recoverySummary()`,
`setSummary`, and only then `setDetailsOpen(true)`. That closes the reviewer's
path 1 (a failed fetch at mount leaving the cache empty).

**Path 2 is untouched and fully live.** Chain, all verified:

- The `useEffect` sits at `:1737`, well above the early return at `:2443`
  (`setup.stage.kind !== "ready"` → `SetupLayout`, which has no dialog slot).
  `NewUiApp` begins at `:221` and has no return before `:2438`, so the listener
  is registered and fires in every setup stage.
- `recovery_summary` (`src-tauri/src/lib.rs:2689`) reads the journal and
  `pending_restore()` only. No account, no session. It returns `Some` exactly
  when the tray's card is showing.
- The tray's card is reachable while signed out for the same reason: it is driven
  by `pendingRestore()` (`src/TrayApp.tsx:208`, `:786`), and `onReview` is
  unconditional (`:891-895`).
- The only reset of `detailsOpen` is the dialog's own `onClose`
  (`src/NewUiApp.tsx:2889`), which cannot run if the dialog never rendered.

So: after a sign-out teardown, the user clicks "Review details" in the popover,
the tray hides, the window comes forward on the sign-in card, and nothing opens -
the exact symptom `request_recovery_details` was written to remove. `detailsOpen`
and `summary` are both now set, and `RestoreDetailsDialog` pops unprompted the
moment the user finishes signing in. The `else` arm is inert here too:
`setView({ kind: "settings" })` changes nothing while `SetupLayout` is rendering.

The author's docstring at `:1726-1731` names this path exactly and then does not
guard it, which is the part that makes this an overclaim rather than an oversight.

**The parenthetical still stands as a pop-later, not merely a no-op.**
`detailsOpen && summary` at `:2886` is the seventh arm of the chain that starts
at `:2695`: quit, three `runningApps.stage` arms (`:2799`, `:2805`, `:2811`), two
`modelOverlay` arms (`:2828`, `:2854`). With any of those open, the tray click
sets both pieces of state and the dialog appears when that overlay closes. The
window's own banner button (`:2668`) is immune because `noticeAboveDialog` is
false for the recovery banner (`:2642`), so the z-20 scrim covers it - but the
tray is a different window and no scrim reaches it. **H** for the setup path,
**M** for the chain.

### 5. `src/TrayApp.tsx:888` (reasoning about the backend's answer) - **PARTLY FIXED**

The comment no longer claims the backend's `Some` decides whether anything opens,
which was the finding. But its replacement asserts a guarantee the window does
not give: "the window re-reads the summary on this event and routes to Settings
when there is nothing to show" (`src/TrayApp.tsx:885-887`). In the setup stage
neither half is true - nothing opens and nothing routes. This rides entirely on
finding 4. **M**

### 6. `src/components/gc/AppPane.tsx:257` (`unattributed` stops short of `StatTiles`) - **VERIFIED FIXED**

Threaded at `src/components/gc/AppPane.tsx:252`; forced in `metrics.tsx:165`
(both counters), `:176-180` (percent) and `:180-184` (the `tokensSavedAmount`
delta suppressed). The test asserts no `0`, no `0%`, no `+$0.00` and exactly
three `n/a` against the same all-zero fixture. Verified load-bearing: dropping
the prop at `AppPane.tsx:252` fails
"prints no figure at all when nothing on the pane is attributable".

Residual: `RecentActivity` honours the flag only inside the
`activity.length === 0` arm (`AppPane.tsx:703-717`), so rows plus
`unattributed: true` still prints the rows. `MessagesChart` gets this right - its
`unattributed` arm (`metrics.tsx:298`) precedes the bars, so it wins over a
non-empty series. Same "invariant lives in the caller" shape the finding
objected to, one card over. **L**

### 7. `src/components/gc/dialogs.tsx:489` (dead 4px from a truthy element) - **VERIFIED FIXED**

`routesShown` at `:104-108`; all three call sites decide before building the
element (`:518`, `:577`, `:751`). `details` is genuinely `undefined` in every
shipped build and in dev for a row with no routes. `Modal.tsx:397-408` now
documents the pass-`undefined` contract and no longer claims `details` carries
who reopens the tool. Nit: the predicate folds the DEV gate in, so
`RoutePair`'s own `if (!routesShown(tool)) return null` at `:133` is now
unreachable-but-harmless belt and braces. **L**

### 8. `e2e/new-ui-running-apps.spec.ts:183` (dev-only affordance asserted as AC 1) - **VERIFIED FIXED**

Both `toContainText` lines retired, the test renamed to what it pins, and it now
asserts `"Codex"` and `"reopen Codex yourself"`. That second string does render
for a single `canReopen: false` tool: `mine.length === 0` →
`dialogs.tsx:528` with `toolLabel` returning the single tool's name. The build
split stays covered in `dialogs.apply.test.tsx` (see finding 1). Copy nit,
pre-existing and now load-bearing on an e2e assertion: that same sentence opens
"Gate Connect can close these apps" for a set of one. **L**

### 9. `src/components/gc/Tray.tsx:862` (an absolute replaced by another absolute) - **VERIFIED FIXED**

`src/components/gc/Tray.tsx:867-871` now reads "No recent security events" /
"N recent security event(s)", `truncate` is gone in favour of `min-w-0` so the
truncation point is moot, and three unit tests cover zero/plural/singular. The
deviation from both suggested forms is the better call: "recent" is true under
the cap, under the credential clear, and on a quiet run, which neither "This
session" nor a second scope line manages.

Residual: with `state: "offline"` and `count: 0` the card still asserts "No
recent security events" beside an OFFLINE pill - a reading printed while the feed
is not reading. Principle 6, one step past what this finding asked for. **L**

### 10. `crates/core/src/env.rs:560` (loop cannot detect its own regression) - **VERIFIED FIXED**

The loop now goes through the resolver per variable and asserts
`starts_with(&scratch)`, matching what the `CODEX_HOME` case already did.
Coverage is genuinely all eight: `CODEX_HOME` separately plus the seven-entry
table (`CLAUDE_CONFIG_DIR`, `OPENCODE_CONFIG_DIR`, `OPENCODE_CONFIG`,
`XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `OPENCLAW_CONFIG_PATH`, `HERMES_HOME`). I
swept the file independently: every remaining `env_path`/`var_os` for a per-user
tool override routes through `tool_path_override` (`:267`, `:308`, `:344`,
`:347`, `:365`, `:396`, `:415`, `:429`); the only bare read left is
`PROGRAMDATA` at `:378`, which is a machine-wide path and correctly outside the
seam. Test passes. The `:245-247` docstring's "all eight" is now true.

### 11. `src/components/gc/dialogs.teardown.test.tsx:67` (assertion carried by the Modal's tone tile) - **VERIFIED FIXED**

Scoped to the row off the "Mystery Tool" label. Verified load-bearing: changing
`dialogs.tsx:2404` to `icon={tool.icon}` fails
"falls back to a glyph, not a word, when no mark is supplied" and only that
test - the exact mutation the reviewer named, which used to leave all three
green. The traversal (`closest("div")?.parentElement`) is coupled to
`ModalSubject`'s DOM shape, but it fails rather than passes if that shape moves,
which is the safe direction. **L**

---

## Merge resolution review

### 1. `src/NewUiApp.tsx` topbar banner denominator - **your reasoning is sound, and the dropped version would have been worse**

Both halves of your argument check out end to end.

*Green is reachable on a default install.* Chat rows are pushed with
`chat: true` from `provider.chat_domain_slugs` (`src/lib/groups.ts:554-556`).
Every chat surface ships `enabled: false` in the catalog -
`claude-web` (`crates/core/src/proxy/catalog.rs:149`), `chatgpt-apps` (`:305`),
`chatgpt` (`:347`) - and `enabled: false` → `desired: false`
(`groups.ts:492`) → `on: false` (`src/NewUiApp.tsx:1411`) → excluded from
`desiredApps` (`:2166`). `cascadeTargets` skips them (`groups.ts:771`), so
flipping every family switch leaves them off and out of the denominator. The
banner reaches `allProtected`.

*An opted-in chat row is still counted, and green is still reachable for it.*
`routed: domain.enabled && proxyOn && caTrusted` (`groups.ts:491`) →
`proxyMemberStatus` returns `{kind:"protected"}` (`src/lib/verdict.ts:117`) →
counted in `protectedCount` (`NewUiApp.tsx:2167`). Confirmed.

And the version you dropped was not merely redundant, it was wrong in a way the
base's is not: `bannerApps` removed chat rows from **both** halves, so a chat row
the user switched on and which was **not** routed would have been invisible to
the banner, and the topbar would have gone green while a row the user asked for
sat unrouted on the screen below it. That is a false all-clear on the one piece
of chrome whose job is to say otherwise - strictly worse than the amber
overclaim it was fixing. Taking the base's intent filter is right.

Two residuals, neither undoing the above:

- `totalCount === 0` renders amber (`src/components/gc/banners.tsx:102` guards
  `totalCount > 0 &&`), so a user who deliberately switches everything off reads
  "Gate Connect is partly routing your apps · Partly routed · 0 of 0 Apps". The
  guard is pre-existing, but the intent filter is what makes 0-of-0 reachable by
  a user action rather than only on an empty machine. See New findings M-3.
- The `anthropic` domain ships `enabled: true` (`catalog.rs:79`), so it is in the
  denominator without the user having asked. I chased this because it is the same
  shape as round 1's finding, and it is **not** a defect: its two unrouted states
  are master-off and needs-trust, both actionable and both correctly amber, so
  green stays reachable once routing is on and the CA is trusted. Recorded so the
  next reader does not have to re-derive it.

What the merge did **not** settle is the author's own open question from round 1,
and it is now a live inconsistency rather than a hypothetical. See New findings
M-1.

### 2. `src/components/gc/Modal.tsx` panel overflow - **coherent, and the unreachable-primary bug does not return**

All three of your checks hold:

- **The panel still caps.** `src/components/gc/Modal.tsx:230`:
  `relative flex max-h-full max-w-full flex-col … p-6`. No whole-panel
  `overflow-y-auto` survived the merge.
- **Only the body scrolls.** `:296`: `mt-6 flex min-h-0 flex-col gap-4
  overflow-y-auto`. The absence of `flex-1` is fine and arguably better: the
  default `flex-shrink: 1` plus `min-h-0` is exactly what lets it shrink below
  its content when the panel hits the cap, and leaves it at intrinsic height when
  it does not. Header (`:250`) and footer (`:311`) are both `shrink-0`.
- **`flex-wrap` on a `shrink-0` footer is safe.** `shrink-0` gives the footer
  `flex-basis: auto` with no shrink, so a wrapped row's larger intrinsic height
  is honoured by the column and the body absorbs the loss. The primary cannot be
  pushed out. The only failure mode would be header + wrapped footer alone
  exceeding the cap; at 400x700 the scrim's `p-6` leaves 652px against roughly
  48 (header) + 108 (two wrapped 36px rows, `gap-3`, `mt-6`) + 48 (panel
  padding) ≈ 204. Not close.

`whitespace-nowrap` + `shrink-0` on the three buttons is the right pairing with
`flex-wrap`: it is what turns "squeeze the label until it wraps inside its own
`h-9` box" into "move the button to the next line". `justify-end` keeps each
wrapped line right-aligned, and DOM order puts the primary last, so it lands
bottom-right rather than above the secondary.

One incidental dependency worth a line, since `max-h-full` is now the thing
holding the layout together: in the tray the scrim's `absolute inset-0`
(`:216`) has **no positioned ancestor** - `Tray`'s root (`Tray.tsx:179`) is
`flex h-screen w-full flex-col` with no `relative`, and `main.tsx` wraps it in
nothing - so it resolves against the initial containing block. That happens to be
the popover viewport, so `max-h-full` is correct today. `AppShell` states the
same requirement explicitly and carries `relative` for it
(`AppShell.tsx:105`). See New findings L-4.

---

## New findings

### H

**H-1. `src/NewUiApp.tsx:1737-1747` - the reveal listener still arms a dialog the shell cannot render, and never disarms it.**
Round-1 finding 4's second path, unfixed. `detailsOpen` and `summary` are both
set while `setup.stage.kind !== "ready"` (`:2443`) is rendering `SetupLayout`,
which has no dialog slot; the only reset is the dialog's own `onClose` (`:2889`).
After a sign-out teardown the tray's "Review details" is reachable
(`TrayApp.tsx:891`, driven by an account-free `pendingRestore` at `:208`) and
`recovery_summary` answers without a session (`src-tauri/src/lib.rs:2689`), so
the click produces nothing and then pops `RestoreDetailsDialog` unprompted the
moment sign-in completes. Gating the open on `setup.stage.kind === "ready"` (and
either deferring or dropping the intent otherwise) is what the docstring at
`:1726-1731` already promises.

**H-2. `src/TrayApp.tsx:296-306` - the tray now holds a routing-down banner it can never clear, for failures it did not cause.**
Round-1 finding 2's mirror case, converted from a race into a certainty.
`report_backend_error` queues a copy for `"tray"` unconditionally
(`src-tauri/src/lib.rs:1788`), the hidden tray sweeps on every
`backend-error-pending`, and `setActionError` is cleared only by the user's own
dismiss (`:933`) - not on hide, not on reveal, not when a later
`provider_reconcile` succeeds. So the startup auto-enable failing at launch, or a
"Resume now" the user pressed *in the main window*, both land as an undated
`ErrorBanner` over the popover hours later, alongside a `MasterCard` that may by
then be reporting everything routed. The three contexts this reaches are
`restore_routing`, `provider_restore` and `provider_reconcile`
(`src/lib/backendErrors.ts:25-29`), all of which describe a *state* rather than
an event, so the fix is to clear or re-derive on the next successful reconcile
rather than to stop delivering it.

### M

**M-1. `src/components/gc/Tray.tsx:295-298` - `MasterCard` is now the surface that can never go green, and the two shells disagree.**
The author raised this in round 1 and asked for a read; the merge answered the
window and left the tray. `apps = groups.flatMap(g => g.apps)` counts every row
including the chat rows that ship `enabled: false`, so `all` requires routing a
session-cookie surface nobody switched on - round 1's finding 3, verbatim, on the
popover. Worse, the two counts now disagree by exactly the switched-off chat
rows: the topbar can read green "3 of 3 Apps" while the card under the same
tray icon reads amber "0 of 5 tools routing". The docstring's "it counts every
row - chat domains included" reads like a drawn decision, which is why it
survived; it predates the chat rows existing as per-row opt-ins, and it is the
same argument `Group.cascadeDesired` (`src/lib/groups.ts:413-421`) already
accepted for the family switch.

**M-2. `src/NewUiApp.tsx:2886` - the tray's request can still be starved by an earlier dialog arm and fire later.**
Six arms precede `detailsOpen && summary` (`:2695`, `:2799`, `:2805`, `:2811`,
`:2828`, `:2854`). Since the request arrives from a different window, the scrim
that protects the in-window banner button does not apply. The author's reply
claims this is "no longer a pop-later"; it is - `detailsOpen` is set, nothing
resets it, and the dialog surfaces when the running-apps or model overlay
closes. Cheapest correct behaviour is to refuse the open while another overlay
holds the slot (and say so), rather than to reorder a precedence that looks
deliberate.

**M-3. `src/components/gc/banners.tsx:102` - "0 of 0" reads as a fault.**
`allProtected = totalCount > 0 && …` means an empty denominator renders amber
"Gate Connect is partly routing your apps · Partly routed · 0 of 0 Apps". With
the denominator now being intent (`src/NewUiApp.tsx:2166`), a user who switches
everything off reaches this by their own deliberate action, where before it
needed a machine with no tools. Nothing is partly routing and nothing was asked
for; the banner should say that, not report a gap. The guard itself predates the
PR, so this is the merge making a latent state reachable rather than a
regression.

### L

**L-1. `src/components/gc/AppPane.tsx:703-717` - `unattributed` is advisory in the feed and mandatory everywhere else.**
`StatTiles` (forced, `metrics.tsx:165`) and `MessagesChart` (forced ahead of the
bars, `metrics.tsx:298`) both override their input; `RecentActivity` honours the
flag only when `activity.length === 0`, so rows plus `unattributed: true` prints
the rows. Same shape as round-1 finding 6, one card over, and the tests do not
cover it.

**L-2. `crates/core/tests/disconnect_zero_residue.rs:75/78, 87/90, 111/114 - `GATE_CONNECT_TEST_HOME` is captured, set and restored twice.**
`prev_seam` and `prev_test_home` read the same variable at the same moment, so
today the duplicate restore is idempotent and harmless. It is dead duplication a
future edit can desync - move either read past either `set_var` and the restore
starts writing the scratch path back into the developer's environment.

**L-3. `src-tauri/src/lib.rs:5510+` - `each_shell_drains_its_own_copy_of_a_failure` does not exercise `drain_backend_errors`.**
It reimplements the drain against the static, so the command's `window.label()`
keying (`:1833`) is uncovered: hardcoding `"main"` there leaves the test green.
The docstring is honest about why. Also `ERROR_SINK_LABELS` is a fixed pair, so a
label that never drains (a platform where the tray webview is not created)
accumulates to the 32 cap and stays there - bounded, but it is 32 stale failures
waiting for the first shell that ever calls with that label.

**L-4. `src/components/gc/Tray.tsx:179` - the tray's modal scrim has no positioned ancestor.**
`Modal`'s scrim is `absolute inset-0` (`Modal.tsx:216`) and the panel's cap is
`max-h-full` against it. In the tray that resolves against the initial containing
block, which is the popover viewport, so it is correct by coincidence rather than
by construction. `AppShell.tsx:105` carries `relative` and a comment saying why;
the tray root does not.

**L-5. `src/components/gc/Tray.tsx:867` - an offline feed still prints a count.**
`state: "offline"` with `count: 0` renders "No recent security events" next to an
OFFLINE pill. Principle 6: the buffer is not being read, so there is no reading
behind the zero. One step past what round-1 finding 9 asked for, and the new
tests only cover `state: "live"`.

**L-6. `src/components/gc/dialogs.apply.test.tsx:42/53 - the stub is value-typed.**
Booleans work (measured); `vi.stubEnv("DEV", "false")` comes back truthy. Worth a
line in the file's docstring, because a string here is what would make Copilot's
objection true.

**L-7. `src/components/gc/dialogs.tsx:133 - `RoutePair`'s own guard is now unreachable.**
`routesShown` folds the DEV gate in and every caller asks it first, so the
early return can only fire if a future caller forgets - which is the point, but
worth saying rather than leaving as an apparent double-check.

**L-8. `src/NewUiApp.tsx:1740 - a tray click silently dismisses an unrelated window error.**
`setActionError(null)` runs before the fetch, so a "Review details" press in the
popover clears a failed-rename report the user has not read. Mirrors what the
org-switch listener at `:1680` already does, so this is consistency rather than
a new mistake.

---

## Verification notes

Mutations run and reverted (`git status` clean at exit; the untracked
`docs/review-ui-bug-sweep-security.md` belongs to a sibling review, not to this
one):

| Mutation | Result |
| --- | --- |
| `dialogs.tsx:106` DEV gate → `true` | `dialogs.apply.test.tsx:56` fails |
| `dialogs.tsx:106` DEV gate → `false` | `dialogs.apply.test.tsx:45` fails |
| `dialogs.tsx:2404` → `icon={tool.icon}` | teardown fallback test fails, and only it |
| `AppPane.tsx:252` drop `unattributed` | "prints no figure at all" fails |
| `lib.rs:1788` → `["main"]` | `each_shell_drains_its_own_copy_of_a_failure` fails |
| `provider.rs:969` → `if false && …` | `a_snapshot_entry_for_an_unknown_provider_is_dropped_not_retried` fails |

Suites: `vitest run` 986/986 (58 files); `tsc --noEmit` clean;
`cargo test --lib` for `each_shell_drains`, `the_test_home_seam_outranks` and
`a_snapshot_entry_for_an_unknown_provider` all pass.

Not covered by this lens: `pnpm app` on any OS (nothing here touches
`keychain.rs`), Playwright, and `cargo clippy -D warnings`.
