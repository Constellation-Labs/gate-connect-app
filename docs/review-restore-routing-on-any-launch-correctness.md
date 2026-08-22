# Correctness review: fix/restore-routing-on-any-launch

Scope: branch `fix/restore-routing-on-any-launch` vs `main`, one commit
(170d4a5), one changed file (`src-tauri/src/lib.rs`, +80 -86). The change
makes the persisted routing intent survive every exit (only `proxy_disable`
clears it, lib.rs:754), extracts the crash-safety-net block from
`proxy_enable` into `fn arm_crash_safety_net` (lib.rs:682-718), and calls it
from the startup auto-enable success arm too (lib.rs:1845). Supporting state
read in full: `crates/core/src/proxy/intent.rs`,
`crates/core/src/proxy/autostart_optout.rs`. `cargo check` on `src-tauri`
passes clean (no warnings; the removed `use tauri_plugin_autostart::
ManagerExt;` in the exit block was only needed by the deleted
`is_enabled()` call).

## Scenario traces

1. Launch-at-login ON, routing on, quit, reboot, silent launch. Intent set
   at lib.rs:654, kept at exit (lib.rs:2192-2196). Silent launch: pending is
   false so the opt-out branch (lib.rs:1790) is skipped; `load_intent()`
   true (lib.rs:1816); auto-enable succeeds; `arm_crash_safety_net` sees
   `is_enabled() == Ok(true)` and no-ops (lib.rs:709); tray updated at
   lib.rs:1882. Holds.

2. Launch-at-login OFF, routing toggled on in the UI. `proxy_enable` arms
   marker then registers the item (lib.rs:661, 688-690). Clean quit:
   `disable_quiet` reverts the proxy first (lib.rs:2179), then
   `complete_pending_autostart_disable` deregisters and clears the marker
   (lib.rs:2190, 777-796); intent kept. Reboot: no login item, nothing
   stranded. Manual launch: auto-enable fires (lib.rs:1816-1845) and
   re-arms the net (register + marker). State machine closes; the only new
   effect is the per-session login-item churn assessed in finding M2.

3. Crash instead of clean quit. Boot state: item registered, marker
   pending, intent true, proxy stranded. Silent launch:
   `reconcile_on_startup` restores the snapshot and clears it
   (manager.rs:578-582), then the silent+pending branch runs
   `complete_pending_autostart_disable` and `handle.exit(0)`
   (lib.rs:1790-1807) without touching intent. Ordering is safe: the marker
   is cleared before `exit(0)`, so when `RunEvent::Exit` fires on the main
   loop the `complete_pending` call at lib.rs:2190 early-returns
   (lib.rs:780-782) and `disable_quiet` finds no engine. Keeping the intent
   does not break this branch's assumptions - the branch never reads the
   intent, and the next manual launch restores routing as designed. One
   pre-existing wrinkle noted below (P1): the exit-time `disable_quiet`
   with no snapshot force_offs all services, partially undoing what the
   reconcile just restored; that ordering is identical on main.

4. Explicit launch-at-login opt-out while routing is on.
   `set_launch_at_login(false)` defers via `record_disable`
   (lib.rs:835-841, autostart_optout.rs:59-70); status reports off/pending
   (lib.rs:818-820). Clean quit completes the opt-out and keeps the intent.
   Next manual launch: auto-enable fires and `arm_crash_safety_net`
   re-registers the login item the user removed, marker armed
   (lib.rs:1845, 687-690). The machine autostarts at the next boot only if
   the session ends uncleanly; that boot's silent launch deregisters and
   exits (lib.rs:1790-1807), so there is no unbounded autostart on
   macOS/Windows - but the register-at-launch / deregister-at-quit cycle
   now repeats every session forever, driven by launch instead of by a
   user's routing toggle. Assessment in findings M1/M2: on main the cycle
   self-terminated (exit cleared the intent, auto-enable never fired
   again); re-creating the item per routing toggle was already safety-net
   design, so the flapping is design-consistent in kind but newly
   perpetual and no longer tied to a user action.

5. Thread safety of `arm_crash_safety_net` on the startup `std::thread`.
   Precedent exists in the same thread: `complete_pending_autostart_disable
   (&handle)` at lib.rs:1804 calls `autolaunch().disable()` /
   `is_enabled()`, and setup itself calls `autolaunch().enable()` at
   lib.rs:1651. The plugin (`MacosLauncher::LaunchAgent`, lib.rs:1464-1466)
   does plist/desktop-file/registry work with no main-thread requirement,
   and `AppHandle` is Send + Sync; `proxy_enable`'s copy already ran on an
   async runtime thread, not the main thread. No issue.

6. Updater relaunch. The remaining `UPDATER_RELAUNCHING` check
   (lib.rs:2189) still guards `complete_pending_autostart_disable`, which
   is still needed (the pending marker and login item must survive the
   relaunch). No dead logic in the backend. On relaunch (not --silent) the
   pending marker plus registered item read as `is_enabled() == true`, so
   `arm_crash_safety_net` no-ops and the marker stays pending until the
   next safe point. Correct. Two frontend comments are now stale (L1).

7. Linux. The exit handler is cfg macOS/Windows (lib.rs:2177), unchanged.
   The only Linux behavior change is the silent+pending branch no longer
   clearing intent, which is the point of the branch. But the combination
   of "arm on every launch" with "no quit-time completion on Linux"
   creates the boot loop in finding M1.

Cross-checks: `ROUTED_CLIENTS_MAY_BE_STALE` / `engine_moved` are
unaffected - `prior_port` / `prior_pac_port` are snapshotted before
`enable()` (lib.rs:1835-1837) and `arm_crash_safety_net` touches no port
state. No test asserts the old exit-time intent clearing: intent.rs tests
are file round-trips (intent.rs:71-87), autostart_optout tests are
marker-only (autostart_optout.rs:119-213), and src-tauri has no test code.
The frontend reads `launch_at_login_status` only in Settings, whose
pending-note copy (Settings.tsx:610-612) already covers the safety-net
registration case.

## Findings

### M1: Linux gets a perpetual per-boot silent launch (and boot-time routing teardown) for routing-on users without a login-item opt-in

On Linux a clean quit never completes the safety-net marker: the
`RunEvent::Exit` handler is `#[cfg(any(macos, windows))]` (lib.rs:2177),
and nothing else runs at quit. Every session now re-arms the net
(lib.rs:1845 registers the item and marker on every launch, because the
intent survives), so after every clean quit the item and marker persist
into the next boot. That boot's silent launch runs `disable_quiet` -
tearing down the headless daemon routing that Linux quits deliberately
leave running (lib.rs:1797-1803) - then deregisters and exits
(lib.rs:1804-1806). The next manual launch re-enables and re-registers,
and the cycle repeats forever. On main the same cycle existed but
self-terminated at the first boot because that branch cleared the intent;
now an opted-out (or default-off) Linux user with routing left on gets an
app process at every boot indefinitely, and their between-session headless
routing is dropped at each boot. Traffic is never stranded (disable_quiet
falls to passthrough), so this is a should-fix behavior loop, not a
safety bug. Worth asking whether Linux needs the safety net at all: the
net exists for a dead-port stranding (autostart_optout.rs:1-11), and on
Linux the daemon survives a GUI crash, so the stranding it guards against
may not occur there.

### M2: opted-out macOS/Windows users now get automatic per-session login-item flapping

Trace 4 above: for a user who explicitly turned launch-at-login off while
routing was on, every manual launch re-creates the login item
(lib.rs:1845 -> 687-690) and every clean quit removes it (lib.rs:2190).
The cycle is bounded (no repeated autostart unless the session crashes)
and the comment at lib.rs:1841-1844 shows it is intended, but versus main
it is newly perpetual and detached from any user action: on main the
intent was cleared at exit for these users, so auto-enable never fired
and the item only reappeared when they themselves toggled routing on.
With `MacosLauncher::LaunchAgent` (lib.rs:1465) each registration writes a
LaunchAgent plist; on macOS 13+ background-item changes can surface
system notifications, so an opted-out user may see recurring "added as a
login item" notices every session. Recommend a conscious accept/reject of
this trade rather than letting it ship implicitly.

### L1: stale frontend comments describing the removed exit-time intent clear

src/components/UpdatePanel.tsx:78-82 says an early updater mark "would
make the exit handler skip its routing-intent clear and deferred
launch-at-login opt-out completion"; the intent clear no longer exists.
src/components/UpdatePanel.tsx:92-94 says the mark is "so the backend
keeps the routing intent and restores routing after the restart"; the
intent is now kept unconditionally and the mark's remaining purpose is
keeping the pending opt-out (and login item) armed across the relaunch.
Comments only; behavior is unchanged and still correct.

### L2: stale design doc and mildly overclaiming Settings copy

docs/restart-routing-persistence-plan.md:27-33 documents the superseded
conditional exit-time intent handling as current design.
src/screens/Settings.tsx:595 ("Keeps routing on after a restart.") is
still true for boot-time restoration but no longer the whole story, since
routing now also comes back on the next manual launch without the toggle.

### P1 (pre-existing, not introduced by this branch): exit-time disable_quiet force_offs all services when no snapshot exists

`disable_inner` with `load_snapshot() == Ok(None)` runs
`force_off(active_services)` (manager.rs:406-411, system_proxy.rs:322-331),
and `RunEvent::Exit` calls it unconditionally (lib.rs:2179). On the
scenario-3 path the reconcile has just restored and cleared the snapshot
(manager.rs:578-582), so the exit that follows `handle.exit(0)` force-offs
proxy settings (e.g. a corporate proxy) the reconcile restored - exactly
what the comment at lib.rs:1791-1793 tries to avoid for the explicit
disable. Identical on main; noted because the scenario-3 ordering question
surfaced it.

## Verdict

The state machine is sound and every traced invariant holds; the two
must-consider items are behavioral loops for launch-at-login-off users
(unbounded per-boot launch on Linux, perpetual per-session login-item
flapping on macOS/Windows), plus stale comments/copy - no traffic-safety
or data-loss bug found.
