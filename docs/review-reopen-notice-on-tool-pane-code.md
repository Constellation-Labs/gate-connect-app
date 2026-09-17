# Review: `fix/reopen-notice-on-tool-pane` - code quality

Base `feat/new-app-ui`, head `d9a56ad`. 11 files, +114 / -177.

Lens: readability, naming, dead code, over-engineering, boundary error
handling, test coverage, comment hygiene. Security and correctness are other
reports.

## Verification run

Everything below was checked against the files, not inferred from the diff.

- `npx tsc --noEmit` and `npx tsc -p e2e --noEmit`: both clean.
- `npx vitest run`: 65 files, 1254 tests, green.
- `npx playwright test e2e/new-ui-routing.spec.ts e2e/new-ui-verdict.spec.ts`:
  57 passed. `e2e/new-ui-running-apps.spec.ts -g "reopen|invitation"`:
  7 passed. So the three new selectors do resolve, and resolve uniquely.
- The repo has no prettier config and no lint script, so line-wrap drift in
  the touched comment blocks (`src/lib/reopen.ts:9`,
  `src/components/gc/dialogs.tsx:83`) is not a finding.

**The deletion itself is clean.** `rg 'ReopenBanner|reopenHidden'` over
`src/` and `e2e/` returns nothing; the only surviving hits are
`plans/new-app-ui-figma.md:2766` (deliberately, in the rewritten prose) and
`docs/review-figma-differences-report-validation.md:80` (a historical review
doc). `TrayApp.tsx`'s `reopenPending` (L902) is a different, still-live memo
feeding the tray card, not a leftover. No import, state, prop or helper was
orphaned: `StatusTile` and `Icon` are still used by five other banners
(`banners.tsx:131,197,342,457,544`), `toolName` is still used
(`NewUiApp.tsx:1342`), `offerAfterChange` has six other callers. Nothing in
`src/lib/notices.ts` or `src/components/gc/AppShell.tsx` referenced the
banner.

## H - must fix

None.

## M - should fix

**M1. The "and Overview does not" half of the renamed test asserts nothing
reliable.** `e2e/new-ui-running-apps.spec.ts:315` is a `toHaveCount(0)` run
on the first tick after `boot` resolves. `toHaveCount(0)` passes on its first
evaluation, and the reopen state only exists once the async
`routing_verdicts` sweep has landed and been applied - so the assertion can
pass simply because nothing has been drawn yet, rather than because Overview
declines to draw it. The comment above it (L313-314) says "the boot screen
carries the rail's phrase and nothing else", but no assertion reads the rail
phrase. Asserting the rail's "Reopen to finish" first would both make the
comment true and order the negative assertion after positive evidence that
the verdict arrived. (There is incidental protection: a re-added shell banner
would give two "Close tool" buttons on the pane and blow up the strict-mode
click at L321. That is a side effect, not what the test name claims.)

**M2. The two new local helpers duplicate helpers that already exist, and
then are not used consistently in their own file.**

- `openCodexPane` at `e2e/new-ui-running-apps.spec.ts:35` is byte-identical
  in body to `openCodexPane` at `e2e/new-ui-routing.spec.ts:25` (only the doc
  comment differs). `openClaudePane` at `e2e/new-ui-routing.spec.ts:30` and
  `openApp` at `e2e/new-ui-verdict.spec.ts:25` are the same one-liner again -
  four copies of "click the rail row" across three specs.
- `e2e/fixtures.ts` is where this belongs: the `App` class already owns
  `familyRow` (:60), `appSwitch` (:64), `routeApp` (:79) and `openSettings`
  (:98). One `openSection(name)` there replaces all four, and each spec's
  reason for opening the pane stays where the reason is - at the call site.
- Within the new file, `e2e/new-ui-running-apps.spec.ts:460` and `:489`
  hand-roll the helper defined 425 lines above them at `:35`, and `:462`
  hand-rolls the locator defined at `:44` with a *different* regex
  (`/Reopen .* to finish/` unanchored, against the helper's
  `/^Reopen .+ to finish$/`). Two spellings of one locator in one file is
  exactly what introducing the helper was meant to stop.
  `e2e/new-ui-running-apps.spec.ts:419` and `:436` are the same story for
  `openClaudePane`.

**M3. Three comments the deletion missed, all stating the reversed claim.**
The branch is scrupulous about this everywhere else - it rewrote
`AppShell.test.tsx:43-48`, `reopen.ts:8-10`, `useRunningApps.ts:100-102`,
`dialogs.tsx:83`, `banners.tsx:226-232` and the plan - so these read as
oversights rather than as decisions:

- `src/components/gc/Tray.tsx:874`: "One label across the three surfaces that
  raise this flow - see `banners.tsx`." Two surfaces now, and `banners.tsx`'s
  own copy of that sentence was corrected in this branch.
- `src/components/gc/Tray.tsx:149`: "(AG-566 AC 3, which asks for this on
  tool detail, Overview *and* the tray)". The branch deliberately declines
  the Overview half and says so at `src/NewUiApp.tsx:3005` and
  `plans/new-app-ui-figma.md:2762-2769`. A reader landing on the Tray prop
  first gets the superseded contract.
- `e2e/new-ui-tray.spec.ts:112`: the same sentence, on the test that is now
  the *only* place still claiming Overview is in scope.

(While in `Tray.tsx:148-161`: the `reopen` prop carries two stacked doc
comments, the first of which documents a member that is no longer beneath it.
Pre-existing, not from this branch, but it is in the block M3 asks you to
touch.)

**M4. Nothing announces a pending reopen any more.** The deleted
`ReopenBanner` carried `role="status"`; `ReopenAlert` (`banners.tsx:195`) has
no role and no live region. That is *consistent* with the file - shell-width
banners announce (`NoteBanner` :267, `ErrorBanner` :454 as `alert`,
`RecoveryBanner` :540), pane cards do not (`AlertBanner` :316, `PaneNote`
:304) - so this is not a new inconsistency. But the sweep raises this card
asynchronously while the pane is already open (that is precisely the
behaviour `e2e/new-ui-running-apps.spec.ts:339-368` exercises in reverse), so
a screen-reader user on Codex's pane now gets no notification that the tool
needs reopening. Either a `role="status"` on `ReopenAlert` or a deliberate
note saying why not.

## L - nits

**L1. The card locator is copy-scoped, not element-scoped.**
`/^Reopen .+ to finish$/` (`running-apps:45`, `routing:305`, `verdict:112`)
excludes the rail's bare "Reopen to finish" only by arithmetic - `.+` cannot
match the empty string, so the 16-character rail phrase cannot satisfy a
pattern needing 17. It works (verified), but it is a fragile way to say
"the card, not the row", and rewording the heading breaks three specs at
once. `e2e/new-ui-tray.spec.ts:135` already prefers
`getByRole("heading", ...)` for the same fact; the pane card's heading is a
`<p>` (`banners.tsx:198-200`), so it has no role to take - promoting it, or a
test id, would make the locator element-scoped.

**L2. `getByRole("button", { name: "ChatGPT / Codex Protected" })`
(`running-apps:365`) is the only selector in `e2e/` that matches a rail row by
a concatenated label-plus-status name.** It depends on the accessible name
being the row label and the status label joined with a space, which holds
because both spans are flex items of `Sidebar.tsx:637` and are therefore
blockified. It passes, and the substring default means a grey suffix
("- 2m ago") does not break it. But the coupling is invisible from the test.
Scoping the existing `getByText("Protected", { exact: true })` inside a row
locator would say the same thing without depending on accname concatenation.

**L3. `openClaudePane` (`routing:30`) relies on substring matching plus
`.first()` and does not say so.** The pre-existing `openCodexPane` three
lines above carries a comment explaining the rail-section naming; the new
helper's comment explains *where the card is drawn* but not why `.first()` is
load-bearing. The pattern matches `new-ui-engine.spec.ts:154` and
`new-ui-model-picker.spec.ts:60`, so it is house style - just under-commented
by this file's own standard.

**L4. `e2e/new-ui-routing.spec.ts:274`'s title, "it is drawn beside the
reopen card, not behind it", now names a hazard the code cannot produce** -
"behind" was about ranking inside the notice chain, and the reopen is no
longer in that chain. The test still earns its place (re-adding the banner
would suppress the "Pages already open" note and fail L306), but the title
and the first comment paragraph now describe history rather than the
assertion.

**L5. `ReopenAlert` is now the sole window-side surface for this fact and has
no component test.** There is no `banners.test.tsx` in the repo at all, so
the deletion lost no unit coverage - but the remaining card's only coverage
is e2e (`running-apps` x3, `routing` x1, `verdict` x1), and its route pair
(`routeInUse` / `requestedRoute`, `banners.tsx:205-218`), including the
degraded "backend could not say" branch the prop doc at `:187-189` describes,
is asserted nowhere. Worth a small render test now that it is load-bearing.

## Things checked and found fine

- `NewUiApp.tsx:2995-3005`: the rewritten chain comment is accurate (three
  arms), names the reversal, names the precedent (#277) and names what is
  lost (AC 3's Overview half). `:3049`'s "fifth arm" is historical and reads
  as such.
- `NewUiApp.tsx:2761-2777`: `reopenAlert` was untouched and still keys off
  `openTool` rather than the pane slug, with the comment explaining why.
- `banners.tsx:226-232`: the button-label comment absorbed the deleted
  banner's fuller reasoning instead of losing it. Good salvage.
- `AppShell.test.tsx:43-48`: the elevation test's rationale was updated to
  say the banner is gone *and* why the scoped exemption stays. Exactly the
  right edit.
- `plans/new-app-ui-figma.md:2762-2789`: the plan states plainly that AC 3's
  Overview half is deliberately not met, which is the one place a future
  reader will look.
