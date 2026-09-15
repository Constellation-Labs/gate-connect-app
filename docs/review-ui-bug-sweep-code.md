# Review round 2 - PR #244 `fix/ui-bug-sweep` - code quality

Base `feat/new-app-ui`, head `8c03a4c`. 22 files, +1084/-104.

## Summary

The round-1 fixes are, with two exceptions, real and load-bearing. I re-ran
every mutation the author claimed to have checked and each one behaved as
described: dropping `unattributed` from the `StatTiles` call site fails
`AppPane.test.tsx`, reverting `hermes_config_dir` to `env_path` fails the env
seam test, `icon={tool.icon}` fails the teardown fallback test and only that
test, and narrowing `ERROR_SINK_LABELS` to `["main"]` fails
`each_shell_drains_its_own_copy_of_a_failure`. Copilot's `vi.stubEnv("DEV")`
report is genuinely wrong and the author was right to reject it - I proved the
stub is live by deleting the DEV gate from `routesShown` and watching the
"ships neither route" test fail.

Gates: `pnpm test` 986 passed, `tsc --noEmit` clean, `cargo fmt --check` clean,
`cargo clippy --locked --workspace --all-targets -D warnings` clean,
`cargo test -p gate-connect-core` clean. Working tree left clean.

Two things did not survive scrutiny. The per-label error buffer makes both
shells drain a copy of every failure, so every backend error now reaches
analytics **twice** - the destructive take used to guarantee exactly one. And
the recovery-details handoff still no-op-then-pops on the branch the author
said was covered: the window sitting on setup. Beyond those, the residue is
comment hygiene - three comments now describe code or history that is not
there, including one in a test file that restates the exact overclaim this PR
removed from the component.

On the merge resolutions: both are sound and neither leaves a stale comment.
`bannerApps` is fully gone (no orphaned `chatSlugs`), and the base's
`desiredApps` docstring at `src/NewUiApp.tsx:2142-2165` describes precisely the
code that is there. The `Modal` footer's `flex-wrap` + `shrink-0` combination
behaves: the container `shrink-0` acts on the column axis and does not fight
`flex-wrap` on the row axis, and with per-button `shrink-0` +
`whitespace-nowrap` the widest tray label ("Yes, close affected apps", ~194px)
still fits the 304px tray content box, so nothing overflows horizontally.

---

## Round 1 verification

| Finding | Verdict |
| --- | --- |
| `dialogs.apply.test.tsx:63` - `vi.stubEnv("DEV")` is inert (Copilot) | **CORRECTLY REJECTED.** Removing `import.meta.env.DEV` from `src/components/gc/dialogs.tsx:106` makes "ships neither route" fail at `dialogs.apply.test.tsx:56`. The stub reaches `import.meta.env.DEV`. The author's reasoning (two mutually exclusive tests on one flag both passing) is also sound on its own. |
| `TrayApp.tsx:298` - destructive drain raced between shells | **VERIFIED FIXED**, mechanism-wise. `src-tauri/src/lib.rs:1773` is now per-label, `:1833` scopes on `window.label()`. Load-bearing: narrowing `:1788` to `["main"]` fails with "the tray must still see it". **But the fix's own docstring overclaims and it introduces a telemetry regression** - see H1 and M1. |
| `NewUiApp.tsx:2056` - banner denominator wedged amber | **ADDRESSED, but not by the commit the reply names.** Merge `8c03a4c` dropped `bannerApps` for the base's `desiredApps = railApps.filter(a => a.on)` (`src/NewUiApp.tsx:2166`). That does resolve the symptom - chat rows ship off, so they are out of an intent-filtered denominator - and it is the better mechanism. Nothing stale is left behind. Note only that `a709e96`'s commit message still describes `bannerApps` as the fix, so the reply no longer matches the tree. |
| `NewUiApp.tsx:1697` - listener no-ops, then pops later | **INCOMPLETE / OVERCLAIMED.** The empty-cache half is genuinely fixed (`src/NewUiApp.tsx:1737-1748` fetches, commits, then opens). The setup half is not - see M4. The reply says "Both empty-cache paths are reachable as you describe, including the setup one", which conflates "the cache is empty" with "there is no dialog slot"; only the first is a cache problem and only the first is fixed. |
| `TrayApp.tsx:888` - comment reasoned about the wrong fact | **VERIFIED FIXED.** `src/TrayApp.tsx:885-892` now says exactly what the reply claims. One clause overreaches: "routes to Settings when there is nothing to show" is untrue on the setup branch, where `setView` is inert (see M4). |
| `AppPane.tsx:257` - `unattributed` stopped short of `StatTiles` | **VERIFIED FIXED, claim holds.** `src/components/gc/AppPane.tsx:252` threads it; `src/components/gc/metrics.tsx:168,179,184` force the unavailable reading including the `+$0.00` delta. Reply's "dropping the prop at the `AppPane` call site fails it" is TRUE - I did it and `AppPane.test.tsx:313` fails. The fixture does carry a real `"+$0.00"` (`AppPane.test.tsx:13`), so the assertions are not vacuous. |
| `dialogs.tsx:489` - DEV gate left a dead `mt-1` in shipped builds | **VERIFIED FIXED.** `routesShown` (`src/components/gc/dialogs.tsx:104-108`) is asked before the element is built at all three sites (`:517`, `:576`, `:751`), so `details` is genuinely `undefined`. `src/components/gc/Modal.tsx:397-408` no longer claims `details` carries who reopens and now states the pass-`undefined` contract. |
| `e2e/new-ui-running-apps.spec.ts:183` - dev-only affordance asserted forever | **VERIFIED FIXED.** Both `In use:` / `Requested:` lines retired, test renamed to "names the running tool and who reopens it", and the surviving assertion (`reopen Codex yourself`) matches shipped copy at `src/components/gc/dialogs.tsx:528`. Minor nit at L6. |
| `Tray.tsx:862` - "Since Gate Connect started" was a second absolute | **VERIFIED FIXED in the component** (`src/components/gc/Tray.tsx:867-871`), and the `truncate` that ate the scope is gone. **But the test file kept the disproved claim** - see M5. |
| `crates/core/src/env.rs:560` - loop asserted the helper, not the resolvers | **VERIFIED FIXED, every claim holds.** `env.rs:576-600` goes through resolvers. "All eight" is exact: eight call sites use `tool_path_override` (`:267,308,344,347,365,396,415,429`), seven in the loop plus `CODEX_HOME` at `:530-556`. Load-bearing: reverting `:429` to `env_path("HERMES_HOME")` fails with "HERMES_HOME punched through the test home". |
| `dialogs.teardown.test.tsx:67` - assertion satisfied by the Modal's tone tile | **VERIFIED FIXED, claim holds exactly.** Scoped to the row at `dialogs.teardown.test.tsx:72`. Mutating `src/components/gc/dialogs.tsx:2404` to `icon={tool.icon}` fails 1 test and passes 2 - "fails that test and only that test" is literally true. |

---

## New findings

### H1. Every backend error now reaches analytics twice

`src-tauri/src/lib.rs:1785-1798` queues a copy of each failure for both labels
in `ERROR_SINK_LABELS`, and both shells drain: `src/NewUiApp.tsx:1033` and
`src/TrayApp.tsx:300`, each through `forwardBackendErrors`
(`src/lib/backendErrors.ts:42-53`), which calls `trackError` for **every**
entry in its buffer, not only the surfaced one. `trackError`
(`src/lib/analytics.ts:229-242`) emits an `error_shown` event *and* a
`posthog.captureException`. Both webviews are mounted from launch, so one
`report_backend_error` now produces two `error_shown` records and two entries
in the exception list.

Before `a709e96` the `std::mem::take` on a single `Vec` guaranteed exactly one.
This is a regression introduced by the fix, it silently doubles a metric the
team triages, and it is unrecoverable after the fact. Worse, one of the two
`error_shown` events is emitted by a hidden webview where nothing was shown -
the event name becomes false for half the records.

The routing-down *display* legitimately wants both shells. The analytics
forward does not. Split them: drain-for-display per label, and either forward
to analytics from one label only or tag the record with the draining label so
the double can be collapsed downstream.

### M1. `PENDING_BACKEND_ERRORS`'s docstring claims to fix a case it does not

`src-tauri/src/lib.rs:1759-1771` says "A copy per label fixes both", where the
second "way that showed" is a startup failure taken by the hidden tray
surfacing later as an unexplained banner, "because `actionError` is never
cleared on hide".

`actionError` is still never cleared on hide - `src/TrayApp.tsx:124` has no
hide-edge reset (the only clears are at `:465`, `:803`, `:834` and the banner's
own dismiss at `:933`) - and `report_backend_error` now queues startup failures
for the tray *unconditionally*. So the hidden tray still buffers a
`provider_reconcile` failure it had no part in, still drains it at
`src/TrayApp.tsx:300-301`, and still shows it as a banner the next time the
popover opens. What the fix changed is that the window is no longer blind to
it. Half of the named symptom is fixed; the docstring says both are.

Either clear `actionError` on hide, or narrow the claim to what the change
actually did.

### M2. `each_shell_drains_its_own_copy_of_a_failure` re-implements the code under test

`src-tauri/src/lib.rs:5528-5538` defines a local `drain(label)` that is a
verbatim copy of `drain_backend_errors`'s body (`:1833-1843`) minus the
`window.label()` read. The docstring is honest about why ("a `tauri::Window` a
unit test cannot build"), but the consequence is that the test can stay green
while the real command is broken - a "simplification" of the command that
reintroduced `std::mem::take` over the whole map would not be caught.

Extract `fn drain_for_label(label: &str) -> Vec<BackendError>`, have
`drain_backend_errors` call it with `window.label()`, and have the test call the
same function. One line of restructuring buys the coverage the test is
advertised as providing.

### M3. `disconnect_zero_residue.rs` duplicates the seam it says it is adding

`crates/core/tests/disconnect_zero_residue.rs` adds `prev_seam` (`:52`), a
`set_var("GATE_CONNECT_TEST_HOME", &dir)` and a `restore(...)` in `Drop`
(`:108`). All three already existed: the base has `prev_test_home`, the same
`set_var`, and the same `restore` - and they are still there, visible as
unchanged context in the diff. The file now reads the variable twice, sets it
twice, and restores it twice.

The comment is worse than the duplication. It says "Without it this suite read
and OVERWROTE the developer's real Codex `auth.json` and `config.toml`" - but
the seam was already set on the base, so that never happened in this file. The
comment describes an incident that belongs to `codex_billing_mode.rs`
(correctly documented at `crates/core/src/env.rs:236-243`) and attributes it
here.

Drop `prev_seam` and its two statements, and either delete the comment or
rewrite it to what is actually true now: the XDG pins below are redundant
because `tool_path_override` kills them under the seam.

### M4. The recovery-details handoff still no-op-then-pops when the window is on setup

`src/NewUiApp.tsx:1737-1748` now fetches the summary before opening, which
fixes the empty-cache path. It does not fix the other path round 1 named.

`recovery_summary` (`src-tauri/src/lib.rs:2689-2696`) needs no account - it
reads the journal and `pending_restore`, and `pending_restore`
(`crates/core/src/provider.rs:884-905`) is pure snapshot reads. So in exactly
the situation the tray shows its recovery card, `recoverySummary()` returns
`Some`, the listener runs `setSummary(fresh); setDetailsOpen(true)` - and
`src/NewUiApp.tsx:2443` has already returned `SetupLayout`, so there is no
dialog slot. `detailsOpen` is reset in exactly one place, the dialog's own
`onClose` at `:2890`, which cannot run. The moment the user finishes signing
in, `RestoreDetailsDialog` pops unprompted. That is the round-1 bug, on the
round-1 branch.

The docstring compounds it. `:1726-1731` files this case under "Two reachable
ways the cache is empty" - it is not a cache case at all; the cache gets
populated fine and the slot is what is missing - and then `:1733-1735`
concludes "So: fetch, commit, and only then open", implying the fetch covers
it.

Gate the open on `setup.stage.kind === "ready"` (or reset `detailsOpen` when
the shell unmounts), and correct the docstring's diagnosis.

Also here, `:1741-1742` tests `fresh` twice in a row; one block reads better.

### M5. `Tray.test.tsx`'s describe docstring restates the overclaim the PR removed

`src/components/gc/Tray.test.tsx:128-134` opens with "The count is what the
feed has buffered since launch, and the card has to say so." That is precisely
the claim `src/components/gc/Tray.tsx:850-862` now spends a paragraph refuting
- the array is capped at `FEED_CAPACITY` with the oldest evicted and emptied on
any credential change - and "the card has to say so" mandates the copy that was
deleted. `a709e96` updated the component's comment and the assertions but left
this block untouched, so the test file's own header is contradicted by the
inline comment three lines below it (`:139-141`).

### M6. No test for the second half of the unknown-provider fix

`crates/core/src/provider.rs:1158-1173` adds the guard to
`restore_one_provider`, the per-row Retry path, and its comment names a
concrete symptom: "the per-row Retry failed identically every time and left the
entry in the snapshot for the next one". The new test
(`:1481-1533`) only exercises `restore_all`. Given that the bug class this PR
is fixing is "a card no action can clear", and Retry is the user's other
button, the Retry path deserves the same assertion - the setup is already
written and only the call under test changes.

---

### L1. `restore_one_provider`'s new branch duplicates the snapshot tail

`crates/core/src/provider.rs:1166-1172` re-implements the clear/save logic that
already sits at `:1206-1215`. Its own comment says it is "the same one
`restore_one_tool` has always had" - but `restore_one_tool` factors it into a
`drop_from_snapshot` closure (`:1230-1238`) precisely to avoid this. Two copies
of a snapshot rewrite is how the two come to disagree.

Also `find(slug)` is called twice, at `:1154` for the display name and `:1158`
for the guard; `:1154` already handles `None`.

### L2. The new provider test leaves its temp home behind

`crates/core/src/provider.rs:1487-1493` removes the directory before creating
it but never after, so every run leaves a `gate-provider-unknown-*` tree in
`temp_dir`. The env restore is careful; the directory is not.

### L3. `RoutePair`'s own early return is now unreachable

`src/components/gc/dialogs.tsx:110` guards on `routesShown(tool)`, and all
three call sites (`:517`, `:576`, `:751`) already asked the same question. The
component's guard is defensible as an invariant, but the `routesShown` docstring
says "Callers ask BEFORE building the element" without noting that the
component asks again, so a reader cannot tell which of the two is the contract.

### L4. `metrics.tsx:168` formatting is inconsistent with its own sibling

`const count = ...` packs an unparenthesised nested ternary onto one 96-char
line, while the structurally identical expression eight lines below
(`:176-182`) is expanded across five. There is no prettier gate in CI, so this
is readability only - but the two should match.

### L5. `metrics.tsx:292` adds a condition that cannot be false

`const empty = !pending && !unavailable && !unattributed && highest === 0` - the
`unattributed ?` arm at `:299` precedes `empty ?` at `:310` in the same chain,
so `empty` is only ever read when `unattributed` is falsy. (`!unavailable` was
already redundant for the same reason; the new term follows the existing
pattern rather than fixing it.)

### L6. Two e2e assertions where one does the work

`e2e/new-ui-running-apps.spec.ts:177` asserts `toContainText("Codex")`, which
is strictly subsumed by `:182`'s `toContainText("reopen Codex yourself")`. The
first line proves nothing the second does not.

### L7. `AppPane.test.tsx:313` fails for the right reason by accident

`screen.queryByText("0")` *throws* on multiple matches rather than returning
null, so when I dropped `unattributed` from the call site the failure was
"found multiple elements" rather than a clean assertion. It does fail, and it
fails on the right change - but `queryAllByText("0")` with a length assertion
says what is meant and reports it legibly.

### L8. `reopenSubjects` has moved into a leaf presentational module

`src/components/gc/BrandMark.tsx:114-130` now hosts a model mapper and, for it,
imports a type from `./dialogs` (`:3`). Both new imports are `import type` so
there is no runtime cycle, and the docstring justifies "not in either shell" -
but it does not address `lib/reopen.ts`, which owns the model, has no
dependency on either component module, and would not point a leaf component at
a dialog module. Note that its sibling `teardownSubjects` stayed in
`src/NewUiApp.tsx:3427-3438`, so the two now live in different layers.

### L9. `Modal.tsx`'s footer comment omits the per-button `shrink-0`

`src/components/gc/Modal.tsx:302-310` explains `flex-wrap`, the buttons'
`whitespace-nowrap`, and the row's `shrink-0`. The buttons also gained
`shrink-0` (`:324`, `:338`, `:357`), and it is load-bearing - `whitespace-nowrap`
alone would let a flex item shrink and overflow its own text. A reader who maps
the comment's single `shrink-0` mention onto the row will read the button-level
one as noise.

### L10. `Modal.tsx`'s panel comment names two different window sizes for one case

`:219-220` describes the long dialog "at the 1024x800 floor"; the paragraph
added at `:227-229` calls the same case "at 1280x800". Both are real sizes per
CLAUDE.md, but the two sentences describe the same overflow at different ones.

### L11. `drainBackendErrors`'s doc no longer says what it does

`src/lib/api.ts:998-1000` still reads "Hand over (and clear) the backend's
buffered analytics errors." The load-bearing new fact is that it drains only
the *calling window's* copy - a second caller in the same window gets nothing.
The PR documented `requestRecoveryDetails` four lines up and left this one.
