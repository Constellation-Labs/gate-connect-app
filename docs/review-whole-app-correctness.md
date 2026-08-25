# Correctness review - whole app (Gate Connect, main @ 7fc3d16)

Lens: real bugs, races, state-machine holes, platform divergence. Method:
full read + interleaving traces of the lifecycle-critical code; every finding
below carries a concrete failure sequence verified against the source.
Anything I could not drive to a concrete sequence is listed as a question.

## What I inspected

Read in full: `src-tauri/src/lib.rs`; `crates/core/src/proxy/`
`manager.rs`, `manager_windows.rs`, `manager_linux.rs`, `engine.rs`,
`relay.rs`, `intent.rs`, `autostart_optout.rs`, `port_persist.rs`,
`config.rs`, `system_proxy.rs` (macOS), `mod.rs` (through
`resolve_endpoint`); `crates/core/src/provider.rs`;
`crates/core/src/integrations/codex.rs`, `claude_code.rs`;
`src/App.tsx`, `src/components/UpdatePanel.tsx`,
`src/components/QuitConfirm.tsx`, `src/lib/useWindowReopen.ts`.

Scanned (connect/disconnect/write patterns, not line-by-line):
`opencode.rs`, `hermes.rs`, `openclaw.rs`, `dotenv.rs`, `env_proxy.rs`,
`primitives.rs` (`write_file` temp+rename confirmed), `registry.rs` (grep
only).

Not inspected: `ca*.rs` / `cert_authority.rs` trust-store code, the Linux
helper daemon internals (`helper.rs`, `helper_client.rs`, `control.rs`,
`flock.rs`), `system_proxy_windows.rs` / `system_proxy_linux.rs` bodies,
`oauth.rs` / `account.rs` / `keychain.rs`, `Home.tsx` / `Settings.tsx`
bodies beyond targeted greps, the CLI crate, tests/e2e.

Prior reviews read: `review-restore-routing-on-any-launch-correctness.md`
and `review-main-correctness.md`. Accepted trades from the branch review are
NOT re-reported here: the Linux per-boot silent-launch teardown loop for
opted-out routing-on users (its M1), the macOS/Windows per-session
login-item flapping (its M2), and the pre-existing exit-time
`force_off`-with-no-snapshot behavior (its P1). Two findings from the older
main review are verified FIXED on current main: the frontend enable-all loop
that clobbered per-provider off-choices is gone (`App.tsx` `toggleProxy` now
only reflects the backend's restore), and the enable/disable persisted-flag
asymmetry is gone (`provider.rs:328-341` persists the on-flag even with the
proxy off). The stale `UpdatePanel` comments flagged as L1 are also fixed.

---

## High

### H1. Side-effect engine starts never persist the routing intent, so a restart strands config-routed tools at a dead relay - while the app explicitly promises they reconnect

The routing intent is written in exactly two places: `proxy_enable` sets it
true (`src-tauri/src/lib.rs:654`) and `proxy_disable` sets it false
(`lib.rs:770`). But the engine also starts as a side effect of connecting a
tool: `connect_tool` calls `manager().enable()` + `mark_routing_enabled()`
and never touches the intent (`lib.rs:135-141`). The frontend reaches that
path from the tool row and the family switch (`src/App.tsx:845-890`
`setToolRouted`, `App.tsx:901-984` `setGroupRouted` - both call
`connectTool`, never `proxyEnable`). `mark_routing_enabled` is only the
in-memory staleness timestamp (`lib.rs:1041-1047`); it persists nothing.

Failure sequence:

1. User signs in and routes Claude Code from the family panel (or a tool
   row), never touching the master "Route through Gate" switch - or, the
   even easier variant: user turned the master off at some point (intent now
   durably false, `lib.rs:770`), then later toggles a family on.
2. `connect_tool` auto-enables the engine; `~/.claude/settings.json` now
   carries `ANTHROPIC_BASE_URL = http://127.0.0.1:<relay-port>/anthropic`
   (`crates/core/src/integrations/claude_code.rs:159-216`). Traffic routes.
   The UI even announces the side-effect start ("started" notice,
   `App.tsx:186-195`). Intent on disk: still false/absent.
3. Clean quit. The exit handler reverts the system proxy only
   (`lib.rs:2194-2214`); the tool config stays pointed at the relay (that is
   the documented design - the startup restore is supposed to bring the
   relay back).
4. Next launch (manual, or login-item at reboot): the startup thread hits
   `if !load_intent() { return; }` (`lib.rs:1832`) and never starts the
   engine or relay.

Result: Claude Code (and any relay-routed tool) dials a dead loopback port
on every request and hard-fails until the user opens the popover and flips
the master switch. Three aggravations:

- The ledger reassures. `claude_code.rs:120-145` and `codex.rs:289-311`
  compute "Connected" from `relay_base_url()`, which reads the *persisted*
  relay-port file (`relay.rs:64-71`) - it exists whether or not anything is
  listening. Hermes, by contrast, checks engine liveness and reports drift
  ("dead address", `hermes.rs` `compute_status`). So the two tools that
  matter most read Connected while unreachable.
- The quit-time promise is false for this user. Both the QuitConfirm copy
  ("reconnects when Gate Connect starts again",
  `src/components/QuitConfirm.tsx`) and the `disconnect_tools_for_quit`
  notification ("everything reconnects when Gate Connect starts again",
  `lib.rs:1441-1448`) depend on `restore_all` running at the next launch -
  and `restore_all` only runs from `proxy_enable` and the intent-gated
  startup path (`lib.rs:624/635/1840/1890`, `provider.rs:749-771`). With
  intent false, the swept-tools snapshot sits unrestored indefinitely.
- `reconcile_enabled` (startup/focus) cannot heal it either: it re-applies
  configs but never starts the engine (`provider.rs:452-496`).

Fix direction: persist the intent wherever the engine is deliberately
started on the user's behalf (`connect_tool`, and arguably the CLI enable),
or stop gating the startup restore on the intent alone when managed tool
configs point at the relay. Either way, Claude Code/Codex `status()` should
factor engine/relay liveness the way Hermes does.

---

## Medium

### M1. An engine crash reverts the system proxy but leaves the tray green and an open popover reading "routing on" indefinitely

`handle_engine_crash` (`crates/core/src/proxy/manager.rs:527-556`, same in
`manager_windows.rs:523-552`) drops the dead handle, reverts the system
proxy, and clears the snapshot - and stops there. It lives in core, has no
`AppHandle`, emits no event, and updates no tray. The only tray repaint
sites are the toggle commands, `ThemeChanged`, the startup auto-enable, and
the sign-in-attention edge (`src-tauri/src/lib.rs:648, 764, 1586-1590,
1899, 1955, 1986`); the macOS appearance watcher repaints only when the
menu bar flips light/dark (`lib.rs:2713-2740`). The frontend has no status
poll by design - it re-reads only on `proxy-state-changed` (emitted solely
by the startup auto-enable, `lib.rs:1904`) and on popover reopen
(`App.tsx:493-496`, `useWindowReopen.ts`).

Failure sequence:

1. Routing on; tray dot green; popover open showing On.
2. The engine thread dies (bind loss, panic inside hudsucker, runtime
   failure). The fail-safe fires, `on_unexpected_exit` reverts the system
   proxy. Traffic now flows direct - nothing is stranded, which is correct.
3. Observed bad state: the tray keeps the green "routing on" dot and the
   "routing on" tooltip for hours; an open popover keeps rendering On. The
   user believes their AI traffic is audited/brokered by Gate while it goes
   straight to the provider. It heals only when the user happens to reopen
   the popover (status then reads engine None + snapshot cleared -> off) or
   when the macOS menu bar changes appearance.

This inverts the product's core reassurance: the one pixel PRODUCT.md calls
most important (the status dot) shows a state that is no longer true, on
exactly the failure path the fail-safe was written for. Fix direction: give
core a crash callback into the shell (the `APP_HANDLE` OnceLock already
exists for exactly this pattern, `lib.rs:993`) so the tray repaints and a
`proxy-state-changed` is emitted.

### M2. Domain-watcher retirement race can leave a running engine with no cross-process watcher

`spawn_domain_watcher` (`manager.rs:60-107`; identical in
`manager_windows.rs:70-117`) retires with a load-then-store that is not
atomic with the enable path's `swap`:

1. `proxy_disable`: engine handle taken; engine is `None`.
2. Watcher tick: locks the engine mutex, observes `None`, releases the
   lock - and is descheduled *before* executing
   `WATCHER_ALIVE.store(false)` (`manager.rs:80`).
3. `proxy_enable` completes in full: installs the new engine and calls
   `spawn_domain_watcher()`, whose `WATCHER_ALIVE.swap(true)`
   (`manager.rs:63`) returns `true` (the old watcher has not stored false
   yet), so no new watcher spawns.
4. The old watcher resumes, stores `false`, and exits.

Observed bad state: engine running, `WATCHER_ALIVE == false`, no watcher
thread. From then on, a domain toggle written by *another process*
(`gate-connect proxy domain <slug> on` beside the routing menubar app)
updates `domains.json` and is never pushed to the engine - precisely the
config/engine disagreement the watcher exists to prevent (its own doc
comment cites the measured symptom). In-process toggles still work
(`set_domain` pushes directly). The trigger is an off-then-on flip landing
inside one 1s watch tick, so it is rare but user-reachable (a fast double
toggle, or disable+enable from a script). Fix shape: re-check the engine
and store `false` under the same mutex acquisition, or CAS the flag and
re-verify the engine after a failed spawn.

---

## Low

### L1. Windows startup reconcile still lacks the stranded-loopback sweep (macOS parity gap, still open)

macOS `reconcile_on_startup` runs `clear_stranded_loopback()`
unconditionally after the snapshot step (`manager.rs:591-597`,
`system_proxy.rs:531-579`). Windows returns early when no snapshot exists
(`manager_windows.rs:572-573`) and has no equivalent sweep. Any path that
loses the snapshot without unwinding WinINET (a crash between the registry
write and `save_snapshot` durability, the L2 interleaving below, manual
file deletion) leaves `AutoConfigURL` pointing at a dead loopback PAC: the
PAC fetch fails, WinINET silently falls back to DIRECT, and no startup ever
repairs the registry value. Traffic flows (fail-open) but the stale setting
persists indefinitely. First flagged in `review-main-correctness.md` (#3);
verified still unfixed on current main.

### L2. `disable_inner` does its teardown outside the engine lock, so a concurrent enable is either falsely refused or has its fresh snapshot deleted

`disable_inner` takes the engine handle and releases the mutex in one
statement (`manager.rs:384-389`), then runs `disable_env` -> restore ->
`running.stop()` -> `clear_snapshot()` unlocked (`manager.rs:396-418`).
`enable` holds the lock throughout, but checks cross-process state with
`engine_hosted_elsewhere()` (`manager.rs:188`, `mod.rs:140-149`), which
probes the persisted snapshot + port:

- Enable landing between the take and the `stop()`: the old engine still
  accepts and the snapshot still exists, so the same-process enable bails
  with the misleading "hosted by another process ... Quit that process"
  error. Window: the whole duration of the restore subprocesses (hundreds
  of ms of `networksetup` calls).
- Enable landing between `stop()` and `clear_snapshot()`: it proceeds,
  snapshots the (already-restored) system state, saves it, brings the
  engine up - and the in-flight disable then deletes that fresh snapshot.
  End state: routing on with no snapshot, so `engine_likely_running()` is
  false (cross-process status reads "stopped", `record_disable` judges
  routing off and would deregister the safety net, and the exit-time
  disable falls to `force_off`, dropping a user's corporate proxy instead
  of restoring it). On Windows this also feeds L1.

The frontend serializes its own toggles via `proxyBusy`, so realistic
triggers are the exit handler's `disable_quiet` racing an in-flight
`connect_tool` auto-enable or the startup auto-enable thread (quit within
seconds of launch), and CLI-beside-GUI use. The stop-to-clear window is
narrow; the false-refusal window is not. Self-heals at the next launch in
the exit-race case, so Low.

### L3. Crash window between engine-on and intent-persist / safety-net arm

In `proxy_enable` the order is: engine up (`lib.rs:628-630`) -> tray ->
`set_intent(true)` (`lib.rs:654`) -> `arm_crash_safety_net`
(`lib.rs:662-663`). A crash after the engine is routing but before those
two best-effort steps leaves: system proxy pointed at a dead port, intent
false, no login item (for a launch-at-login decliner). Nothing relaunches
at boot, and the next *manual* launch reconciles the proxy but does not
restore routing (intent false). macOS's `clear_stranded_loopback` and the
PAC's DIRECT fallback mean traffic is not stranded, so the cost is
"routing silently off after a crash" rather than an outage. The window is
milliseconds and the steps are documented best-effort; noting it because
persisting the intent *before* `enable()` (it is the user's intent, known
at click time) would close it for free.

### L4. PAC and relay accept loops spin hot on a persistently failing listener

`serve_pac` retries `accept()` forever with bare `continue`
(`engine.rs:806-809`), as does the relay's `accept_loop`
(`relay.rs:281-284`). Transient errors (EMFILE, ECONNABORTED) make this
correct; a listener that fails permanently (fd invalidated, exhaustion that
does not clear) turns either loop into a 100% CPU spin for the engine's
lifetime, with no log line. A small sleep-on-error or an error-class check
would bound it.

### L5. Stale-closure double-submit guard on the busy flag (pre-existing)

`toggleProxy` / `setDomain` / `setToolRouted` / `setGroupRouted` all guard
with `if (proxyBusy) return;` read from the render closure
(`App.tsx:755-760, 815-820, 845-850, 901-909`). Two activations delivered
before React re-renders both see `false` and both proceed (two
`proxyEnable` calls are idempotent backend-side thanks to the manager lock,
but two `connectTool`s interleave config writes). Buttons render disabled
while busy, so exposure needs same-tick double events (Enter+click).
Previously reported in `review-main-correctness.md` (#4); still present,
still Low.

---

## Questions (not findings)

- Q1: `pac_script` writes a PAC line for every domain in the watch channel
  without filtering `enabled` (`engine.rs:770-791`). Today every sender
  passes `enabled_only` (`engine.rs:863, 153-155`), so this is correct, but
  the invariant is implicit and one future sender passing the full catalog
  would route disabled hosts' traffic through the engine (they would then
  be blind-tunnelled by `should_intercept`, so the consequence is latency,
  not interception). Worth a debug assertion or filtering at the point of
  use.
- Q2: On Linux, `reconcile_on_startup` re-honors routing from the snapshot
  alone (`manager_linux.rs:533-573`), independent of the intent file. The
  cases I could construct all agree with the intent (disable clears the
  snapshot before clearing the intent, and a failed disable propagates
  before `set_intent(false)` runs, `lib.rs:743-770`), and a CLI-enabled
  detached daemon arguably *should* be re-honored. Flagging so the
  two-sources-of-truth structure is a conscious choice.

## Cross-checks that came back clean

- Updater relaunch: `UPDATER_RELAUNCHING` is set after download / before
  install (`UpdatePanel.tsx:74-109`), reset on install failure, and the
  exit handler only exempts the opt-out completion (`lib.rs:2206-2208`);
  the intent survives every exit, so routing restores after the relaunch.
  A relaunch failure after a successful install leaves the flag set, which
  at worst defers the opt-out - documented and accepted.
- Single instance: `tauri_plugin_single_instance` is the first plugin
  registered; a second launch reveals the popover in the running instance
  (`lib.rs:1457-1466`). A `--silent` second launch would show the window,
  but login items do not fire while the app runs, so no real path hits it.
- Crash safety-net ordering: marker before registration
  (`lib.rs:697-734`), completion only at proxy-safe points, silent-launch
  completion clears the marker before `exit(0)` so the exit handler's
  retry early-returns. Interleavings traced in the branch review still
  hold.
- Enable-time crash of the engine mid-sequence: the deferred fail-safe plus
  `is_finished()` re-check inside the lock (`manager.rs:299-319`) covers
  it; `handle_engine_crash`'s 1s try_lock bound is enough because no lock
  holder other than `enable` exceeds it, and `enable` runs the same revert
  itself.
- Integrations: Codex, Claude Code, OpenCode, Hermes, OpenClaw all
  snapshot prior values once (first connect only), write atomically via
  `primitives::write_file` (temp+rename, symlink-escape guard), restore
  conservatively when their marker/sidecar is missing, no-op when the
  config file is gone, and drop sidecars only after the restored config is
  on disk. Codex's passthrough stub for thread resumability and the
  inline-table upgrades are handled. Hand-edits between connect and
  disconnect degrade to conservative behavior (only our keys touched).
- Port persistence: the 47100-47199 band, randomized rotation, deferral of
  this install's own ports, and the TIME_WAIT-vs-live-listener probe in
  `bind_preferred` are coherent; a moved port raises
  `ROUTED_CLIENTS_MAY_BE_STALE` and the frontend surfaces it.
- Relay routing: slug resolution, legacy header fallback (select-only,
  never widens the target), hop-by-hop stripping, Gate-header stripping on
  passthrough, and credential precedence shared with the MITM engine all
  check out against the catalog invariant tests.
