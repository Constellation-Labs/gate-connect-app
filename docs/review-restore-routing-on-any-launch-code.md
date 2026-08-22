# Code quality review: fix/restore-routing-on-any-launch

Branch: fix/restore-routing-on-any-launch (commit 170d4a5)
Base: main
Files changed: src-tauri/src/lib.rs only (+80 / -86)

The change itself is clean: the exit handler no longer clears the routing
intent, the silent-launch deferred-opt-out path no longer clears it either,
and the crash-safety-net block was extracted from `proxy_enable` into
`arm_crash_safety_net` and reused by the startup auto-enable. The extraction
is a faithful move (logic, logging, and error reporting are byte-identical to
the old inline block), the new function's `#[cfg]` gate matches both call
sites, and the rewritten comments at the three touched sites accurately
describe the new invariant. The findings below are almost entirely stale
documentation elsewhere that still describes the old exit-clears-intent
behavior.

## Findings

### M1. Stale doc comment on `UPDATER_RELAUNCHING` still describes the removed exit-time intent clearing

src-tauri/src/lib.rs:931-941

The static's doc says "The exit handler clears the routing intent on a plain
quit when launch-at-login is off ... clearing the intent there would leave
routing off after every upgrade" and later that a mid-download quit "must
keep the exit-time intent clear". That behavior is exactly what this branch
deletes, and the comment now directly contradicts the new exit-handler
comment at lib.rs:2192-2196 ("survives every quit ... the only durable off is
the routing switch itself"). The flag's remaining job is only to defer
`complete_pending_autostart_disable` across an updater relaunch
(lib.rs:2189); the doc should be rewritten around that.

### M2. Frontend comments still explain the updater-relaunch mark in terms of skipping an intent clear

src/lib/api.ts:256-257, src/components/UpdatePanel.tsx:78-82 and 92-93

`setUpdaterRelaunching`'s JSDoc says the mark makes "the exit handler keep
the routing intent", and UpdatePanel's install() comment says a mark set
during the download "would make the exit handler skip its routing-intent
clear and deferred launch-at-login opt-out completion". After this change the
intent is kept on every exit regardless of the mark, so the intent half of
both rationales is dead; only the deferred opt-out completion half is still
true. A future reader of these comments would conclude the old semantics
still exist in the backend.

### M3. The "As implemented" section of the restart-persistence plan doc now documents superseded behavior as current

docs/restart-routing-persistence-plan.md:27-33

The doc opens with "Status: implemented on main" and presents its divergence
list as the shipped behavior. Divergence item 2 states "Exit-time intent
handling is conditional, not hands-off. The RunEvent::Exit handler ...
clears intent when launch-at-login is off". This branch makes exit handling
hands-off for the intent, so the item needs a one-paragraph update (or a
dated supersession note) to keep the doc trustworthy.

### L1. Stale cross-reference: `set_launch_at_login` points at a safety-net block that no longer lives in `proxy_enable`

src-tauri/src/lib.rs:829-830

The comment says "the safety-net block in `proxy_enable` share no lock; see
the accepted-race note there", but this branch moved that block and its race
note into `arm_crash_safety_net` (lib.rs:665-680); `proxy_enable` now
contains only a one-line call at lib.rs:661. The pointer should name
`arm_crash_safety_net` so the reader lands on the note.

### L2. No test accompanies the behavior change; the restore policy remains untestable shell code

src-tauri/src/lib.rs:1816, src-tauri/src/lib.rs:2192-2196

The new invariant (intent survives every exit; only `proxy_disable` clears
it) is expressed as the absence of code in the `RunEvent::Exit` handler and
in the setup thread, and src-tauri/src/lib.rs has no test module, so nothing
guards against the clearing being reintroduced. The core-level pieces that
can be tested (intent round-trip, safety-net marker transitions) already are
(crates/core/src/proxy/intent.rs:71-87,
crates/core/src/proxy/autostart_optout.rs:119-213), and extracting the shell
policy just to test it would be over-engineering for this diff, so this is
noted as an accepted gap rather than a request.

## Not findings (checked and fine)

- crates/core/src/proxy/intent.rs:1-9 module doc ("a clean quit leaves the
  intent intact and the next launch restores it") now matches reality more
  precisely than before the change.
- User-facing copy is still accurate: the Home tip (src/screens/Home.tsx:753-755)
  and the Settings sublabel (src/screens/Settings.tsx:596) tie launch-at-login
  to reboot-time restore, which remains the login item's actual role.
- The removed `use tauri_plugin_autostart::ManagerExt;` in the exit handler
  was the block's only consumer; no dead imports remain.
- `arm_crash_safety_net` handles all three `is_enabled()` outcomes with
  logging plus `report_backend_error`, unchanged from the inline original.

## Verdict

A tidy, low-risk refactor whose code is sound; it should not merge until the
stale intent-clearing documentation in lib.rs:931-941, the frontend comments,
and the plan doc are brought in line with the new invariant.
