# Security review - `fix/reopen-notice-on-tool-pane`

Base `feat/new-app-ui`, scoped with `git diff feat/new-app-ui...HEAD`. Lens:
security. The change deletes `ReopenBanner` (the shell-wide amber notice that a
tool is still routing to the endpoint it started with) and leaves the fact on
the tool's own pane (`ReopenAlert`), the tray card, and the rail row.

## Verdict

No high findings. No secret leakage, no new logging, and no change to what is
rendered from backend-supplied strings or to how it is escaped. The awareness
loss is real but narrow: the fact still stands in persistent chrome (the rail),
and the surface that carries the *detail* (which route the traffic is on) is one
click away. Two should-fix items are about who is still told and how, not about
whether the fact survives.

## What the user is still told, verified

- A `reopen_required` verdict maps to the rail's own `reopen` status kind
  (`src/lib/verdict.ts:100`), which draws amber "Reopen to finish"
  (`src/components/gc/Sidebar.tsx:236`), with the program's name as a grey
  suffix on a multi-surface section (`src/lib/verdict.ts:194`). The rail is
  persistent 256px chrome, so this is on screen on Overview, Settings and every
  pane - the user does not have to hunt for the *fact*.
- The status cannot be masked by a sibling. `sectionStatus` takes the first
  exception in draw order (`src/lib/verdict.ts:183-190`), and the only statuses
  that qualify come from a section's config member; a proxy member can only be
  `protected` or `not-routed` (`src/lib/verdict.ts:124-128`), and every section
  in `SECTIONS` holds at most one config member (`src/lib/groups.ts:725-770`).
- The fact cannot be orphaned. `verdict_for` returns `NotInstalled` before it can
  ever return `ReopenRequired` (`crates/core/src/routing_health.rs:236-238`,
  `:274-277`), so every tool that can raise this is installed, which means
  `buildGroups` gives it a rail row (`src/lib/groups.ts:895`) and `openTool`
  resolves it on the pane (`src/NewUiApp.tsx:510-523`); a slug no section claims
  falls back to itself (`src/lib/groups.ts:1038-1040`). There is no state where
  the old banner was the only surface.
- The standing sweep is unaffected: it is armed on the verdict, not on the
  banner (`src/NewUiApp.tsx:852-879`), so the remaining surfaces do not go
  staler than the deleted one was.
- The tray card still aggregates every waiting tool with no per-pane filter
  (`src/TrayApp.tsx:902-912`, `:1053-1069`), so the "two tools at once" case has
  one place that names both.

## Findings

### M1 - The reopen notice is no longer announced to assistive tech at all

`ReopenBanner` was a live region (`role="status"`, deleted at
`src/components/gc/banners.tsx` in this diff; the e2e selected it by that role,
e.g. `e2e/new-ui-running-apps.spec.ts:313` before this change). The surviving
surfaces are not: `ReopenAlert`'s root is a plain `div`
(`src/components/gc/banners.tsx:195`), and the rail row is plain markup - the
only `role="status"` in `Sidebar.tsx` is the inventory-failure block
(`src/components/gc/Sidebar.tsx:554`), not `AppRow`
(`src/components/gc/Sidebar.tsx:601-618`). The verdict arrives from a background
sweep with no interaction behind it (`src/NewUiApp.tsx:875-879`), so for a
screen-reader user the transition from "routing" to "still on the old route" is
now silent and only discoverable by re-reading the rail. This is a
traffic-destination fact, which is the class of thing principle 1 says the user
should feel rather than have to check. Adding `role="status"` to `ReopenAlert`'s
root would restore the announcement for the pane case; the rail case would still
be silent.

### M2 - The route pair is now reachable only by navigating to the tool's pane

`ReopenAlert` is the only surface that names `routeInUse` and `requestedRoute`
(`src/components/gc/banners.tsx:205-219`), and it is gated on the open pane
(`src/NewUiApp.tsx:2761-2778`). The tray card carries the in-use route only when
exactly one tool is waiting (`src/TrayApp.tsx:1062`), and the rail carries none.
So on Overview - the landing pane - a user is told *that* a tool is on its old
route and never *where* that route points, which is the half of AG-570 that
turns the notice into something actionable. Concretely: two tools waiting, the
tray then deliberately drops the route for both, and the window shell shows two
amber phrases and no endpoint until the user opens each pane in turn. This is a
defensible product trade (the banner's cost over Overview and Settings is real),
but it should be a recorded decision rather than a side effect - the rail row is
not a substitute for the pair, and nothing on the branch says where the pair
went for the multi-tool case.

### L1 - The justification comment misquotes the surface it relies on

`src/NewUiApp.tsx:2993` and `plans/new-app-ui-figma.md:2768` both defend the
deletion with "The rail still reads 'Not protected - Reopen required' on each
affected row". The rail reads **"Reopen to finish"**, plus the program name on a
multi-member section (`src/components/gc/Sidebar.tsx:236`, `src/lib/verdict.ts:194`).
"Not protected" is the phrase this exact state was deliberately moved *off*
(`src/components/gc/Sidebar.tsx:56-64`), and the docstring being deleted in this
same diff quoted it correctly. The claim is the load-bearing "nothing is lost"
argument for removing the only shell-level surface, so leaving it wrong invites
the next reader to conclude the rail still says something it does not, or to
"restore" the old amber negative.

### L2 - The mitigation is asserted by absence, never by presence

`e2e/new-ui-running-apps.spec.ts` now asserts only that no "Close tool" button
exists on the boot screen (`:313-314` in the new text) before opening the pane;
its comment claims "The boot screen carries the rail's phrase and nothing else"
but nothing asserts the rail phrase is visible there. The same is true in
`e2e/new-ui-verdict.spec.ts:105-112`, which reads the pane card only. So the one
surface that now carries this fact on Overview has no test standing on it: a
future change to `STATUS_TEXT`, `statusSuffix` or `sectionStatus` could drop the
reopen phrase from the rail entirely and the suite would stay green, leaving the
fact on no always-visible surface at all.

## Checked and clean

- No credential, key, token, route secret or path added anywhere in the diff;
  nothing new is written to `console`, to a log, or to disk
  (`git diff ... | grep -iE 'console\.|token|secret|api_key|localStorage'`
  returns nothing on added lines).
- No change to rendering or escaping. Backend-supplied strings in the surviving
  card (`app.name`, `verdict.route_in_use`, `verdict.requested_route`) go through
  JSX text interpolation exactly as before (`src/components/gc/banners.tsx:200`,
  `:214-218`); no `dangerouslySetInnerHTML` is introduced or touched. The diff
  removes one place that rendered `toolName(slug) ?? slug` and adds none.
- No change to the action wired behind the button: it is still
  `runningApps.offerAfterChange([openTool])` for a single slug
  (`src/NewUiApp.tsx:2775`), so nothing widened the set of tools a press can
  close.
- No stale `ReopenBanner` import or reference remains in `src/` or `e2e/`; the
  two hits are in `plans/` and an older `docs/` review, both prose.
