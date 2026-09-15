# Review: `fix/alpha4-review-findings` - code quality lens

Base `origin/feat/new-app-ui`, 29 files, +931/-135. Reviewed 2026-09-10.
`npx tsc --noEmit` is clean and `vitest run` is green (55 files, 964 tests).

The branch is well written by this repo's standards: the shared `OpenCodeEnvDialog`
extraction removes a real duplication hazard, the `TeardownReason` split is modelled
in the right place (a reason the report cannot carry), the `Update::History` variant is
correctly kept independent of `FeedState`, and the `Modal` flex/scroll rework is
carefully reasoned. The problems are of one shape: **two of the nineteen fixes do not
actually reach the user in the shipping shell**, and several of the new comments claim
more than the code delivers. The absence of new regression tests matters concretely
here, because a test at either of two existing, already-built harnesses would have
caught both. Findings below are ordered by severity, each verified against the code.

---

## High

### H1. The deliberate-sign-out copy never takes effect in the new (default) shell

`src/NewUiApp.tsx:1639` feeds `useSetup` from `prefs?.signed_out_deliberately`, but
`prefs` is loaded exactly once at mount (`src/NewUiApp.tsx:908`, via `loadPreferences`
at `src/NewUiApp.tsx:674`) and re-read afterwards only by the five preference switches
and `onRetryPreferences` (`src/NewUiApp.tsx:1949-1995`). Nothing re-reads it on a
sign-out: `settings.confirmDisconnect` (`src/lib/useSettingsActions.ts:303-330`) calls
`onSession`, which is `setAccount`/`setOAuth` only (`src/NewUiApp.tsx:1616-1628`), and
`refresh` (`src/NewUiApp.tsx:750-769`) does not touch preferences either. `NewUiApp`
does not remount on sign-out; it switches stage in place (`src/NewUiApp.tsx:2352-2394`).

So after Settings > Disconnect Gate the welcome pane still renders with
`deliberate = false` and says "Session expired" - the exact defect
`signed_out_deliberately` was added to fix. It only works after an app restart, or if
the user happens to toggle a notification preference first.

The legacy popover got this right: `src/App.tsx:1176-1187` re-reads preferences on
entering the `firstrun` screen. The two shells now disagree, and per `CLAUDE.md` the
new shell is the one that ships.

Fix is small: re-read preferences when the setup stage becomes `welcome` (mirroring the
`org-picker` effect at `src/NewUiApp.tsx:2065-2070`), or call `loadPreferences` from
`onSession`.

### H2. The new "Try again" for a failed backfill cannot re-run the backfill

Both new history affordances (`src/components/gc/SecurityPane.tsx:162` and `:217`) call
`onRetry`, which is `useSecurityFeed`'s `retry` (`src/lib/securityFeed.ts:167-172`):
`securityFeedRetry()` plus `seed()`. `securityFeedRetry` reaches
`Feed::retry_now` (`crates/core/src/security_feed/client.rs:103`), which is
`wake.notify_one()`, and `wake` is awaited in exactly one place -
`wait()` (`crates/core/src/security_feed/client.rs:319-327`), which runs *between*
connection attempts. `backfill` runs once per connection inside `connect_once`.

The state these buttons exist for is LIVE with missing history, i.e. the stream is
connected and the loop is not in `wait()`. The notify stores a permit that will be
consumed by the next backoff, and nothing re-runs the catch-up. `seed()` then re-reads
the same `history_ok == false` from `security_feed_history_ok`. Net effect: the table
flashes its skeleton rows and returns identical, and the amber banner stays. A recovery
control that cannot recover is worse than none, and it is offered twice on the same
pane.

Either give the feed a "re-run the backfill now" signal that a live connection honours,
or drop the button and say the history returns on the next reconnect.

---

## Medium

### M1. The sign-out teardown report still shows "Retry disconnect" pills

`src/components/gc/dialogs.tsx:2270-2274` states, as the justification for the change,
that the old wording "arrived with a Retry pill beside it inviting the user to fix
nothing". Only the section `title` and `detail` were made reason-aware
(`src/components/gc/dialogs.tsx:2276-2281`). The pill is still
`TEARDOWN_ACTION_LABEL[tool.next_action]` at `src/components/gc/dialogs.tsx:2352`, with
`tone: "amber"` hardcoded at `:2282`, and `teardown_report` stamps every Connected /
Drifted / Overridden tool with `retry_disconnect` (`src-tauri/src/lib.rs:2879`).

After a sign-out the dialog therefore reads "Still pointing at Gate / Left as they were,
on purpose" with an amber **Retry disconnect** pill on every row. The comment describes
a fix that was not made, and the surface contradicts itself. The e2e assertion added at
`e2e/new-ui-firstrun.spec.ts:226-234` checks only the absence of "could not put these
back", so it passes over this.

### M2. `DisconnectGateDialog`'s Cancel is the one that ignores `busy`

`src/components/gc/dialogs.tsx:1662-1666` documents `busy` with "both buttons refuse a
second click", copied verbatim into five dialogs. Four of them honour it
(`:157`, `:1512`, `:1569`, `:1735`); `DisconnectGateDialog`'s secondary at
`src/components/gc/dialogs.tsx:1680` is left as `{ label: "Cancel", onClick: onCancel }`.
`onDismiss` is guarded (`:1687`) but the visible Cancel is not, so a click during the
sign-out write dismisses the dialog while the request is in flight. Either add
`disabled: busy` or stop saying "both buttons" in that block. (`SwitchGatewayDialog` at
`:224` has the same gap, but pre-dates this branch and carries no such comment.)

### M3. The `actionError`-on-retry rule was applied to seven call sites and skipped the Settings dialogs

The branch adds `setActionError(null)` to the app row toggle
(`src/NewUiApp.tsx:1451`), launch-at-login (`:1930`) and the five preference switches
(`:1943`, `:1957`, `:1966`, `:1975`, `:1984`). The Settings *dialog* actions share the
same slot through `onError: (e) => setActionError(...)` (`src/NewUiApp.tsx:1659`) and
have the same retry shape: `renameDevice` deliberately keeps the dialog open on failure
so "retrying is one click" (`src/lib/useSettingsActions.ts:374-378`), and so do
`replaceKey` and `confirmSwitchOrg`. None of them clears the previous failure, so a
rename that fails and then succeeds closes the dialog and leaves the stale banner
above the pane - the identical symptom the branch set out to remove.

### M4. Comment and code disagree about the history read's failure default

`src/lib/securityFeed.ts:106-107` says "Failing quietly to `true` is right too: not
knowing whether history is missing is not evidence that it is." The code
(`src/lib/securityFeed.ts:108-112`) is `.catch(() => {})`, which leaves whatever the
state already held. It defaults to `true` only because the initial state and the
credential-change reset happen to be `true`; once `historyOk` is `false`, a rejected
re-read on the next `seed()` (a retry, or a backend without the command) keeps the
warning up. `e2e/install.ts:469-471` repeats the same claim ("the same rule the hook
applies when this command is unavailable"). Small in effect, but this repo's comment
style only earns its keep if the comments are true.

### M5. Test coverage: two harnesses were extended for the new states and then not used

The author's decision to add no new tests is defensible for the copy and layout fixes.
It is not defensible for the two new state machines, and in both cases the harness work
was already done:

- `e2e/backend.ts:153`, `:501` and `e2e/install.ts:472` add a `historyOk` fixture knob,
  documented as "a spec that wants the LIVE-with-no-history case sets this false and
  leaves `state` at 'live'". No spec sets it. The knob is currently dead test code, and
  the pane's new third state (banner, empty-cell variant, retry) has zero coverage.
- `crates/core/tests/security_feed_e2e.rs` already has `mock_stream_with_history`
  (`:117-168`) serving both routes on one port. A history route answering 400 and an
  assertion that `Update::History { ok: false }` is emitted once, and not repeated on
  the next reconnect, is about ten lines. `set_history_ok`'s change-guard
  (`crates/core/src/security_feed/client.rs:83-88`) and the reset at `:117` are
  untested.
- The fake backend's `oauth_sign_out` (`e2e/install.ts:337-340`) does not set
  `signed_out_deliberately`, unlike the real command (`src-tauri/src/lib.rs:602`). So
  the harness cannot reproduce the welcome-pane copy at all, which is why H1 went
  unnoticed. `src/lib/useSetup.test.tsx` only updates existing assertions to
  `deliberate: false`; no test asserts the `true` branch.

---

## Low

### L1. "Sending…" comment overstates the file's convention

`src/components/gc/dialogs.tsx:2040-2042` justifies the ellipsis as matching "every
other in-flight label in this file ("Working…")". Two in-flight labels use ASCII dots:
`"Switching..."` (`:226`) and `"Waiting for browser..."` (`:301`). Normalize those two
or soften the comment.

### L2. `plan: "Unavailable"` collides with the vocabulary for a failed read

`src/NewUiApp.tsx:1869` hardcodes the string. In the same object `deviceName` and
`installId` use "Unavailable" to mean "the read returned nothing"
(`src/NewUiApp.tsx:1853`, `:1857`), and `SettingsPane` renders a literal "Unavailable"
plus a Retry button for `row.unavailable` (`src/components/gc/SettingsPane.tsx:724-731`).
A field the product simply does not have now reads, permanently and unretryably, as a
field that failed. Either omit the row until the gateway names a plan, or use a word
that cannot be mistaken for a failure.

### L3. `SecurityPane`'s new prop and duplicated retry markup

`historyUnavailable?: boolean` (`src/components/gc/SecurityPane.tsx:124`) is optional
where `loading` and `unavailable` beside it are required, though `useSecurityFeed`
always supplies it - the optionality only lets a future caller silently lose the state.
The "Try again" button is also written twice, once in the banner
(`:157-167`) and once in the empty cell (`:210-220`), with different class strings for
the same control.

### L4. The topbar menu scrim now covers the notice banner

`src/components/gc/Topbar.tsx:230` raises the scrim to `z-40`, above `AppShell`'s notice
wrapper at `z-30` (`src/components/gc/AppShell.tsx:111`). That wrapper exists precisely
so "a failed rename stays clickable" over the modal scrim
(`src/components/gc/AppShell.tsx:107-110`); with the menu open, the notice's own action
now needs two clicks. The comment at `Topbar.tsx:238-247` reasons about the menu-vs-notice
paint order but not about this consequence, and the tradeoff is defensible (first click
dismisses a menu) - it should just be the sentence that is written down.

### L5. `set_signed_out_deliberately`'s doc promises more call sites than exist

`crates/core/src/preferences.rs:340-343` says it is "Called with `true` by
`oauth_sign_out` and with `false` wherever a sign-in completes". There is exactly one
clearing call, in `oauth_begin_login` (`src-tauri/src/lib.rs:571`); an API-key sign-in
leaves the flag set. Harmless today because the copy is gated on
`auth_mode === "oauth"` (`src/lib/useSetup.ts:190`), but the comment states an invariant
the code does not maintain, and the next person to widen the welcome pane will rely on it.

### L6. `title={row.value}` is unconditional

`src/components/gc/SettingsPane.tsx:755` sets a native tooltip on every value row, not
only truncated ones, so fully visible values ("Trusted", "Unavailable", a short device
name) now get a tooltip repeating themselves on hover. The comment above it
(`:752-754`) describes the truncated case only.

---

## Checked and found sound

Worth recording so a later reader does not re-litigate: the `Modal` flex rework is
correct (the overlay at `src/components/gc/Modal.tsx:209` gives `max-h-full` a bounded
parent, `min-h-0` on the body at `:290` is required and present, the absolute close
button's `right-5 top-5` matches the stated 24px-minus-4px geometry, and no `Modal`
child in the tree relies on overflowing the panel); the `ModalSubject icon="cube"` fix
is real (`icon: ReactNode` at `src/components/gc/Modal.tsx:380`); the widened
`unminimize` cfg matches this file's 69 other uses of the same triple; the
`Update::History` variant does not break the two other `Update` match sites; and
`railApps`/`unattributedMachine` are both defined before their new uses.
