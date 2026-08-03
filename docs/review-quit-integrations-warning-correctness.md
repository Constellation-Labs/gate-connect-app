# Correctness review: `quit-integrations-warning`

Scope: `git diff main...HEAD` (2 commits). Lens: correctness only - logic
bugs, races, edge cases, state staleness. All findings verified against the
code on this branch; frontend suite for the new component runs green
(`npx vitest run src/components/QuitConfirm.test.tsx`, 8/8).

## Summary

The feature intercepts the tray Quit, probes the integration registry off the
main thread, and defers to a popover takeover when config-routed tools are
still Connected. The takeover's "turn off integrations" path reuses the
master-off snapshot machinery (`provider::snapshot_and_disable_all`) without
touching routing intent, expecting the startup restore to reconnect tools on
the next launch.

Two structural gaps break the feature's own contract: the warning is computed
over the full tool registry while the teardown only covers provider-mapped
tools (H1), and the promised "reapplies at next start" restore is gated on
routing intent that is cleared or absent in reachable configurations (H2).
There is also an unserialized snapshot-writer race that can wipe the restore
list (M), and a lost-event edge where Quit silently does nothing (M). The
rest is minor. Things I checked that are NOT problems: `disable_quiet` in the
`RunEvent::Exit` handler clears only the *system-proxy* snapshot
(`crates/core/src/proxy/manager.rs:188-218`), never the provider
restore-snapshot, so the snapshot survives exit; double tray-Quit clicks are
idempotent (two probes, two emits, `setQuitTools` re-set); `app.exit(0)` from
the `spawn_blocking` thread is valid in Tauri v2; the reconcile-on-focus race
(reveal fires `Focused(true)` -> `reconcile_enabled` thread) is negligible in
practice because the human read-and-click delay dwarfs the reconcile's file
I/O, and `reconcile_enabled` only touches Detected / managed-Drifted tools.

## Findings

### H1. Warning set is a superset of the teardown set: "Turn off integrations and quit" leaves named tools pointing at the dead relay

- `src-tauri/src/lib.rs:1066-1070` (probe), `crates/core/src/provider.rs:50-71`
  (catalog), `crates/core/src/provider.rs:406-419` (teardown),
  `crates/core/src/registry.rs:150-158` (registry).

`request_quit` collects Connected tools from the **full registry** - five
integrations: ClaudeCode, Codex, OpenCode, OpenClaw, Hermes. But
`disconnect_tools_for_quit` -> `snapshot_and_disable_all` -> `provider::disable`
only disconnects tools reachable through a provider's `tool_ids`, and the
provider catalog maps only ClaudeCode (anthropic) and Codex (openai).
OpenCode, OpenClaw, and Hermes are connected via the per-tool `connect_tool`
command (`src-tauri/src/lib.rs:111-145`) and belong to no provider.

Failure scenario: OpenCode is Connected. Tray Quit -> dialog says "OpenCode is
set up to route through Gate". User clicks "Turn off integrations and quit".
`snapshot_and_disable_all` returns Ok without touching OpenCode (its provider
loop never visits it), the OS notification announces "Integrations are off
while Gate Connect is closed", and the app quits with OpenCode's config still
pointing at the dead loopback relay - the exact outcome the feature exists to
prevent, now with an explicit false assurance on top. The tool is also absent
from the restore snapshot, so no later restore ever brings it back.

Fix direction: either scope the probe to provider-mapped tools (consistent
warning + teardown, but then non-provider tools quit silently broken), or make
the teardown disconnect every Connected registry tool (cf.
`registry::disconnect_all_managed`, `crates/core/src/registry.rs:169`) with a
restore story for them.

### H2. "Reapplies them at the next start" does not hold whenever routing intent is false at the next launch

- `src-tauri/src/lib.rs:1489-1500` (startup restore gated on
  `intent::load_intent()`), `src-tauri/src/lib.rs:1867-1874` (Exit clears
  intent when launch-at-login is off or an opt-out is pending),
  `src-tauri/src/lib.rs:574,584` (the only other `restore_all` callers, inside
  `proxy_enable`), `crates/core/src/proxy/mod.rs:110-112` (`relay_base_url`
  reads the persisted port, valid while the engine is down),
  `src/components/QuitConfirm.tsx:59-63` and `src-tauri/src/lib.rs:1106-1112`
  (the promises).

The startup chain returns early at `lib.rs:1489-1491` when no routing intent
is recorded - `restore_all` never runs. Intent is false at next launch in at
least two reachable configurations:

1. **Launch-at-login explicitly off** (macOS/Windows): the `RunEvent::Exit`
   handler that this quit path deliberately relies on clears the intent at
   `lib.rs:1867-1874` when `autolaunch().is_enabled()` is false or an opt-out
   is pending. So: routing on, user opted out of launch-at-login, tray Quit,
   "Turn off integrations and quit" -> snapshot saved -> Exit clears intent ->
   next manual launch skips the restore entirely.
2. **Config-only user**: `provider::enable` configures tools whenever the tool
   is detected (`enable_plan`, `provider.rs:108-114`) and `relay_base_url()`
   is persisted-port-based, so tools can be Connected while the master routing
   switch (and thus intent) was never turned on. After the quit teardown, the
   snapshot is only ever consumed by `proxy_enable`'s `restore_all` - a master
   toggle this user may never touch. Their integrations stay off
   indefinitely.

In both cases the dialog copy ("reapplies them at the next start") and the
system notification ("everything reconnects when Gate Connect starts again")
are false; traffic silently bypasses Gate until the user manually re-enables
things. The tools themselves work (they are back on their original settings),
which makes the bypass harder to notice. Fix direction: run a
quit-snapshot-specific restore at startup independent of routing intent (the
snapshot's existence already encodes "these were on"), or distinguish the
quit-time snapshot from the master-off snapshot, or at minimum make the copy
and notification conditional/accurate.

Note: on Linux the Exit handler is cfg'd out, so case 1 does not apply there;
case 2 applies on all platforms.

### M1. Unserialized snapshot writers: quit teardown can clobber the restore snapshot with an empty list

- `src-tauri/src/lib.rs:677` (`proxy_disable` caller), `src-tauri/src/lib.rs:1098`
  (`disconnect_tools_for_quit` caller), `crates/core/src/provider.rs:406-419`
  (`snapshot_and_disable_all` saves unconditionally, even `[]`),
  `crates/core/src/provider.rs:375-391` (non-atomic `fs::write`, and
  `load_snapshot` maps parse failures to `[]` via `unwrap_or_default`).

Nothing serializes the two `snapshot_and_disable_all` call sites (both run on
blocking-pool threads). Concrete sequence: routing on with providers P1, P2;
user flips the master routing toggle off (`proxy_disable` starts: saves
snapshot `[P1, P2]`, begins disconnecting) and then clicks tray Quit while the
toggle is in flight. The probe at `lib.rs:1066` can still see a Connected tool
mid-disconnect, so the dialog appears. Seconds later the user clicks "Turn off
integrations and quit" - by now `proxy_disable` has finished, every provider
reads disabled, so `snapshot_and_disable_all` computes `enabled = []` and
`save_snapshot(&[])` **overwrites** the `[P1, P2]` snapshot. The next
master-on restores nothing and `restore_all` clears the (empty) snapshot.

Secondary hazard on true interleaving: `save_snapshot` is a plain `fs::write`
(no temp-file rename), and `load_snapshot` silently converts a torn/corrupt
file into `[]`, so even a partial overlap degrades to lost restore state
rather than an error. A cheap fix: skip the save when a snapshot already
exists and the computed enabled set is empty, or guard both call sites with a
shared mutex.

### M2. Lost `quit-requested` event: tray Quit can silently do nothing

- `src-tauri/src/lib.rs:1075-1076` (reveal + emit), `src/App.tsx:151-157`
  (listener registered in a mount effect), `src-tauri/src/lib.rs:1037-1039`
  (`reveal_popover_window` returns early with no "main" window).

Tauri events are fire-and-forget: if the webview hasn't registered the JS
listener yet when `app.emit("quit-requested", ...)` fires, the event is
dropped. `listen()` is itself async and attached in a `useEffect` on mount, so
a tray Quit in the first moments after launch (or during a webview reload)
races it. Result: the popover pops up showing the normal home screen, no
takeover appears, and the app does not exit - the user's Quit click is
swallowed with no retry, no timeout fallback, and no second chance short of
clicking Quit again. If the "main" window is missing entirely,
`reveal_popover_window` returns early but the emit still fires into the void
and the quit is likewise abandoned. Consider having the frontend pull pending
quit state on mount (command + event), or having `request_quit` fall back to
`app.exit(0)` after an unanswered timeout.

### L1. Drifted tools are excluded from the warning but included in the teardown

- `src-tauri/src/lib.rs:1068` (`matches!(.., Ok(Status::Connected))`),
  `crates/core/src/provider.rs:255` (`disable` treats `Connected | Drifted` as
  connected), `crates/core/src/provider.rs:332-339` (managed drift is
  considered re-connectable).

A tool in `Status::Drifted` with our management marker still has a config
pointing at the relay (e.g. an older scheme or port) - it degrades on quit the
same way a Connected tool does, but the probe skips it, so the app exits
silently. The asymmetry is at least inconsistent with `disable`'s own
definition of "connected". Low because a managed-drifted config is typically
already degraded while the app runs.

### L2. `quitAnyway` failure wedges the dialog

- `src/components/QuitConfirm.tsx:41-44`.

`quitAnyway` sets `busy = true`, then `await quitApp().catch(() => {})`. If
the invoke rejects (webview/IPC hiccup), the error is swallowed, `busy` is
never reset, and all three controls (including Cancel) stay disabled forever.
Mirror `turnOffAndQuit`'s error handling or reset `busy` in the catch.

### L3. Stale takeover survives a blur-dismiss

- `src-tauri/src/lib.rs:1286+` (`Focused(false)` hides the popover),
  `src/App.tsx:149-157` (`quitTools` cleared only by Cancel).

Clicking outside the popover while the quit takeover is up hides the window
(the quit is implicitly abandoned - arguably fine), but `quitTools` is never
cleared. The next time the user opens the popover, possibly much later and for
an unrelated reason, the quit dialog is still front and center with the tool
list captured at the original probe. Consider clearing `quitTools` on window
hide (e.g. listen for a hide/blur signal) or re-probing on show.

### L4. Duplicate `quit_warning_shown` analytics on repeated tray clicks

- `src/App.tsx:301-303`, `src-tauri/src/lib.rs:1076`.

Each tray Quit click emits a fresh payload array; `setQuitTools` stores a new
reference, so the exposure effect refires and `quit_warning_shown` is tracked
again while the same dialog is already showing. Harmless metric inflation.

## Verdict

Not ready to merge as-is. H1 (the warning names tools the teardown does not
disconnect, then asserts they are off) directly contradicts the feature's
purpose and must be fixed. H2 means the headline promise - integrations come
back at the next start - fails for launch-at-login opt-outs and config-only
users; fix the restore path or the copy. M1 and M2 are real but narrower
races worth closing while the code is fresh. The L items are polish. The
overall shape (probe off-thread, snapshot-without-intent-change, finish via
`quit_app`) is sound once the warning/teardown sets agree and the restore
trigger matches the promise.
