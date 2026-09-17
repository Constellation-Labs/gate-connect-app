# Review: `fix/reopen-notice-on-tool-pane` - correctness lens

Base `feat/new-app-ui`, head `d9a56ad`. Scope: `git diff feat/new-app-ui...HEAD`
(11 files, +114/-177).

Verification run: `pnpm exec tsc --noEmit` clean; `vitest run` over
`AppShell.test.tsx`, `useRunningApps.test.tsx`, `verdict.test.ts`,
`groups.test.ts` - 112 passed. e2e not run.

**No H findings.** Three M, four L.

---

## 1. Was the banner load-bearing for anything else? No.

Answered in full, no finding.

- The standing sweep is armed off the verdicts, not off the banner:
  `src/NewUiApp.tsx:852` builds `reopenWaiting` from
  `[...verdicts.values()].some(v => v.reason === "reopen_required")`, and
  `src/NewUiApp.tsx:875-879` arms/disarms the `REOPEN_IDLE_WATCH_MS` interval on
  that boolean alone. Neither line ever referenced `reopenPending`, the deleted
  memo, or the banner's mount state.
- The focus-edge re-check is the same: `src/NewUiApp.tsx:1140` gates
  `refreshVerdicts()` on `reopenWaiting` inside `useWindowReopen`.
- `onNothingRunning` is wired at `src/NewUiApp.tsx:1341` to `refreshVerdicts`
  and is passed into `useRunningApps` unconditionally; `src/lib/useRunningApps.ts:151`
  stores it in a ref at hook level. It has no dependency on a rendered banner.
- The banner's `onReopen` was `runningApps.offerAfterChange([slug])`, which
  `ReopenAlert`'s own `onReopen` still calls (`src/NewUiApp.tsx:2775`). The
  deleted `onDismiss`/`reopenHidden` had exactly one consumer, the banner.
- `ReopenBanner` has no remaining importer anywhere in `src/` or `e2e/`
  (only two prose mentions, in `plans/` and `docs/`), and no unit test
  referenced it. Removing it left no unused imports - `Icon` and `StatusTile`
  are still used elsewhere in `banners.tsx`, and `toolName` is still consumed at
  `src/NewUiApp.tsx:1342`.

## 2. Can a tool reach `reopen_required` with no surface?

**Not today.** I traced the whole chain and it closes, but it closes by
coincidence rather than by construction - see M-1.

Which slugs can carry the verdict at all:

- `routing_verdicts_now` sweeps `registry()` minus `hidden_in_ui`
  (`src-tauri/src/lib.rs:2681-2683`). The registry is six integrations
  (`crates/core/src/registry.rs:346-353`) and **none of them is hidden** -
  `hidden_in_ui` defaults false (`crates/core/src/registry.rs:333`) and
  `env_proxy`'s override returns false too
  (`crates/core/src/integrations/env_proxy.rs:194-196`). So the swept set is
  `claude-code, codex, opencode, openclaw, hermes, env-proxy`.
- `reopen_required` additionally needs `reopen_pending_for(slug)` true, which
  returns early false when `agent_process_names(slug)` is empty
  (`src-tauri/src/lib.rs:2586-2593`). `AGENT_PROCESSES`
  (`src-tauri/src/lib.rs:2017-2059`) names `claude-code, codex, opencode,
  anthropic, chatgpt`. Intersecting the two: **only `claude-code`, `codex` and
  `opencode` can ever carry `reopen_required`.** `anthropic` and `chatgpt` are
  proxy-domain keys and get no verdict; `openclaw`, `hermes` and `env-proxy`
  have no process names.
- A not-installed tool is closed off one level up: `verdict_for` returns
  `NotInstalled` before it reaches either reopen branch
  (`crates/core/src/routing_health.rs:236-238`). So `openTool`'s
  `status.kind !== "not_installed"` filter cannot strand a reopen verdict.
- Domain-only sections (`openrouter`, `openai`, and the chat domains inside
  `claude`/`chatgpt`) are never swept, so `openDomain === true` panes cannot
  carry a hidden reopen.

Which panes resolve them:

- `openTool` (`src/NewUiApp.tsx:510-523`) takes the **first** installed config
  member of the open section. `claude-code` is member 0 of `claude`
  (`src/lib/groups.ts:730`), `codex` is member 0 of `chatgpt`
  (`src/lib/groups.ts:741`), `opencode` is the sole member of `opencode`
  (`src/lib/groups.ts:773`). All three resolve.
- Each is present in `apps` (`src/NewUiApp.tsx:1553`), so the extra
  `appFor(apps, openTool)` guard at `src/NewUiApp.tsx:2768-2769` does not fire.
- The tray is a second, always-reachable surface and is unchanged:
  `src/TrayApp.tsx:902` lists **every** `reopen_required` verdict regardless of
  which pane is open, with the same "Close tool" action. That materially lowers
  the risk of the shell banner's removal.

### M-1. `openTool` and `apps` are derived by two different filters, and both now fail silently

`openTool` (`src/NewUiApp.tsx:510-523`) filters `tools` by
`status.kind !== "not_installed"` only. `apps` (`src/NewUiApp.tsx:1553`) filters
by that **and** `!isSettingsManaged(t.slug)`. `reopenAlert` needs both to agree:
it resolves the slug from the first and the display name from the second
(`src/NewUiApp.tsx:2765-2769`), and returns `undefined` on either miss.

Today `env-proxy` is the only settings-managed member
(`src/lib/groups.ts:853`), it cannot carry `reopen_required` (no
`AGENT_PROCESSES` row), and its `terminal` section is dropped from the rail
entirely because `buildGroups` is fed a filtered tool list
(`src/NewUiApp.tsx:1435`), so no pane can open on it. Three independent reasons,
none of them stated at the call site.

Related latent shape: `openTool` uses `.find()`, so it is correct only while no
section holds two config tools. `AGENT_PROCESSES` already names two slugs
(`anthropic`, `chatgpt`) that sit *after* the config member in their sections
(`src/lib/groups.ts:730,741`); the day either of those starts producing a
verdict, or a second config tool joins a section, `openTool` resolves to the
wrong slug and the card silently does not draw.

That is not a present-day bug. It is worth flagging because this change removed
the surface that was masking it: before, a slug the pane could not resolve still
appeared in the shell banner (which keyed off `verdicts` directly and fell back
to `toolName(slug) ?? slug`). Now the only remaining fallbacks are the rail
suffix and the tray. A comment at `src/NewUiApp.tsx:2765` naming the "one config
tool per section" invariant as the thing `reopenAlert` depends on would be
enough.

## 3. Does the rail row still carry it? Yes - but not under the name the branch claims.

### M-2. The comment and the plan both state the rail phrase wrongly, and that claim is the whole justification for the change

`src/NewUiApp.tsx:3003` (new in this branch):

> The rail still reads "Not protected - Reopen required" on each affected row

`plans/new-app-ui-figma.md` repeats it verbatim in the paragraph ending "So AC
3's Overview half is deliberately not met."

The rail draws neither string:

- `src/components/gc/Sidebar.tsx:236` - `reopen: { label: "Reopen to finish",
  className: "text-amber-600" }`. It is its own phrase, amber, and explicitly
  **not** `not-protected`; `src/lib/verdict.ts:100` returns `{ kind: "reopen" }`
  precisely so the row does not say "Not protected"
  (`src/lib/verdict.ts:97-99` argues the point).
- "Reopen required" is a *dialog stage* label (`src/lib/reopen.ts:68`), drawn in
  the apply/progress dialogs, never on a rail row.
- The suffix is the tool name, and only on a multi-member section:
  `src/lib/verdict.ts:194` drops the detail when `group.members.length === 1`.
  So `ChatGPT / Codex` reads "Reopen to finish - Codex" while `OpenCode` reads a
  bare "Reopen to finish". The claim "each affected row names the tool" is true
  for two of the three reachable cases, not three.

The deleted `ReopenBanner` doc had this right ("the rail still carries 'Reopen to
finish' on every affected row", and it went on to note that the claim was false
until the phrase existed). The replacement text regressed a fact that the
previous author had gone to the trouble of getting right, and it is the fact the
whole removal rests on.

Fix is one sentence in each of the two places:
`The rail still reads "Reopen to finish" on each affected row, with the tool
named after the dash on a multi-surface section.`

## 4. Do the rewritten e2e tests still assert what they claim?

Mostly yes. Each rewritten test keeps a positive assertion that fails if the card
is not drawn, so none is wholly vacuous. One exception:

### M-3. The one assertion guarding *this change* is unsynchronized and can pass before the sweep lands

`e2e/new-ui-running-apps.spec.ts:315`:

```
await expect(app.page.getByRole("button", { name: "Close tool" })).toHaveCount(0);
```

This is the only thing in the suite asserting that a pending reopen does **not**
appear in shell chrome, i.e. the only regression guard against the deleted
banner coming back. It is taken immediately after `boot()`, and:

- `boot` awaits only the first `h1` (`e2e/fixtures.ts:137-139`);
- `setLoaded(true)` runs in the same tick as a fire-and-forget
  `void refreshVerdicts()` (`src/NewUiApp.tsx:1074-1081`), so the sweep has not
  resolved when the `h1` appears;
- `toHaveCount(0)` passes on its first evaluation against a page that has not
  populated yet.

So a re-added `ReopenBanner` - which only mounts once the verdicts arrive -
would very likely not be caught. Suggested fix: assert the positive state first,
so the sweep is known to have landed on Overview, then take the count. E.g. wait
for the rail row (`getByRole("button", { name: "ChatGPT / Codex Reopen to
finish" })`) before asserting `Close tool` has count 0.

### L-1. `Close tool` clicks are no longer scoped to a surface

`e2e/new-ui-running-apps.spec.ts:321` and `:395` now click a page-wide
`getByRole("button", { name: "Close tool" })` where the old code scoped it to the
banner locator. Unique today (the pane card is the only holder in the window,
and the tray is a separate window), so it works - but the test no longer asserts
*which* surface it pressed, which is exactly the property the change is about.
Scoping the click to `reopenCard(app)`'s nearest container would restore it.

### L-2. No unit-level coverage replaces what was removed

The deleted banner had no unit test and the new pane card has none either; the
entire "a pending reopen has a surface" guarantee now rests on three e2e specs.
Given M-1 (the card returns `undefined` on three separate silent paths), a small
`NewUiApp`-level or component-level test would be cheap insurance. Not a defect
in this diff, just noting where the floor now is.

## Stale prose the branch's own rename missed (L)

The branch deliberately rewrote "shell banner" to "reopen card" in
`src/lib/reopen.ts:8-9`, `src/lib/useRunningApps.ts:99-100`,
`src/lib/useRunningApps.test.tsx:148` and `src/components/gc/dialogs.tsx:83`.
Three sites with the same wording were not updated:

### L-3. `src/NewUiApp.tsx:1336-1339`

The `onNothingRunning` comment still reads "a tool the banner named" / "the
reading the banner was built from". This is the twin of the comment that *was*
updated at `src/lib/useRunningApps.ts:99-100`, two lines apart in meaning.

### L-4. `e2e/new-ui-tray.spec.ts:111-114`

> AG-566 AC 3: "Reopen to finish" belongs on tool detail, Overview *and* the tray.

The branch's own plan now says AC 3's Overview half is deliberately not met. The
test itself is correct and still passes (it only exercises the tray); the doc
comment above it now asserts the opposite of the branch's decision.

### L-5. `src/components/gc/Tray.tsx:148-150`

The `reopen` prop doc carries the same "(AG-566 AC 3, which asks for this on tool
detail, Overview *and* the tray)" parenthetical.

---

## Things checked and found correct

- `AppShell`'s notice-slot elevation is unchanged and the exemption was already
  scoped per banner, not per slot; the test change at
  `src/components/gc/AppShell.test.tsx:43-48` is comment-only and the assertion
  at `:61` still holds (`src/components/gc/AppShell.tsx:110-125`).
- `proxyAdvice`'s suppression-beside-a-measurement
  (`src/NewUiApp.tsx:2745-2748`) reads the same `openTool` + verdict condition
  as `reopenAlert`, so the two cannot both draw. Unchanged by this diff and
  still consistent.
- `sectionStatus`'s exception scan (`src/lib/verdict.ts:184-197`) takes the
  first exceptional member in draw order. For both multi-member sections the
  config tool is member 0, so a `reopen` can never be shadowed by a sibling
  domain's status. (A domain's `not-routed` is not in the exception set anyway.)
- The `reopenCard` regex `/^Reopen .+ to finish$/` binds to
  `src/components/gc/banners.tsx:200-202` and cannot collide with the rail's
  bare phrase or a pane header's `statusDetail`, both of which fail the anchors.
- `getByRole("button", { name: "ChatGPT / Codex Protected" })`
  (`e2e/new-ui-running-apps.spec.ts:356`) relies on Playwright's substring
  accessible-name match, so it survives a later `since` suffix on `protected`.
- The e2e fake's verdict derivation (`e2e/install.ts:632-677`) still produces
  `reopen_required` for a `connected`/`detected` tool under `staleAgents > 0`,
  which is what all three rewritten tests depend on.
