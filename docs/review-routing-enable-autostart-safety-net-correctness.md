# Correctness review: v0.1.1...feat/routing-enable-autostart-safety-net

Scope: `git diff v0.1.1...HEAD` (18 commits), correctness lens only. The core mechanisms in this range - the deferred launch-at-login opt-out marker, the safety-net registration on routing-enable, port persistence for restart survival, the build-fingerprint daemon replacement, and the frontend/backend error/notice plumbing - are carefully sequenced and mostly sound: marker-before-registration ordering in `proxy_enable`, the keep-marker-on-failed-deregistration retry logic, atomic temp+rename port persistence via `primitives::write_file`, and the `enable()` idempotence guard all check out against the code. The findings below are concentrated where the state machine crosses platforms (the Linux reconcile path re-honors instead of reverting, which inverts the silent-launch opt-out branch's premise), where the stale-clients detection covers only one of the two persisted ports, and in a handful of narrower decision-input and race issues. Frontend/backend command contracts (`launch_at_login_status`, `set_updater_relaunching`, `routed_clients_stale`, `close_running_agents`, `drain_backend_errors`) were checked field-by-field and match.

## High

### H1. Linux: silent-launch opt-out completion leaves routing enabled while clearing the intent
`src-tauri/src/lib.rs:1068-1077` assumes "the reconcile above has already reverted any stranded system proxy" before it completes the deferred opt-out, clears the routing intent, and exits. That holds on macOS/Windows (`crates/core/src/proxy/manager.rs:303-328` restores the snapshot), but on Linux `reconcile_on_startup` does the opposite: `crates/core/src/proxy/manager_linux.rs:318-357` *re-honors* a present snapshot by calling `self.enable()`. Sequence on Linux: user toggles launch-at-login off while routing is on (deferred, marker armed), then reboots (the Linux exit handler never runs - the `RunEvent::Exit` block at `src-tauri/src/lib.rs:1341` is `cfg(macos, windows)` - so the snapshot and marker survive even a clean quit); the login item launches `--silent`, reconcile re-enables interception and the env drop-in, then the pending branch deregisters the login item, sets intent to false, and exits. End state: daemon intercepting headless, drop-in in place, intent=false, no app running. The branch's own goal ("a later manual launch stays passthrough") is also defeated: the next manual launch's reconcile sees the snapshot `enable()` re-wrote and re-honors routing again, so routing is sticky-on with intent permanently false. The pending branch needs to actually disable routing (or skip re-honor) on Linux before exiting.

## Medium

### M1. Stale-clients detection compares only the engine port, not the PAC port
`src-tauri/src/lib.rs:1098-1116` snapshots `system_proxy::load_port()` before/after the startup auto-enable and sets `ROUTED_CLIENTS_MAY_BE_STALE` only when that port changed. But the change persists *two* ports for exactly this reason (`crates/core/src/proxy/manager.rs:96-124`, `system_proxy.rs:56-110`): the PAC port is baked into the `AutoConfigURL`, and the code's own comments say a client that captured that URL "silently falls back to DIRECT (bypassing Gate)" when the PAC fetch fails. If the persisted PAC port is taken at startup while the engine port rebinds fine (both bind independently in `engine.rs:574-580`), PAC-caching clients silently bypass Gate and no restart notice is shown - the silent-bypass case the persistence exists to prevent goes undetected. Compare `load_pac_port()` too.

### M2. Quitting mid-update-download leaves `UPDATER_RELAUNCHING` set, skipping the exit-time intent clear and opt-out completion
`src/components/UpdatePanel.tsx:215-237` sets `setUpdaterRelaunching(true)` before `downloadAndInstall()` (necessarily, for Windows), and only resets it in the `catch`. If the user quits the app while the download is in progress (tray quit / Cmd+Q), the exit handler at `src-tauri/src/lib.rs:1354-1371` sees the flag set and skips both `complete_pending_autostart_disable` and the intent clear, even though no relaunch is coming. With launch-at-login off, the next manual launch silently re-enables routing - the exact behavior the intent-clear comment says it prevents - and a deferred opt-out's login item survives a clean quit. The static's doc comment (`lib.rs:639-647`) only accounts for the failed-relaunch case, not an in-flight download abandoned by quitting.

### M3. `record_disable` keys the defer decision off persisted intent, not live routing state
`crates/core/src/proxy/autostart_optout.rs:52-59` decides defer-vs-deregister from `intent::load_intent()`. `proxy_enable` persists that intent best-effort *after* routing is already on (`src-tauri/src/lib.rs:415-418`): if `set_intent(true)` fails (logged and reported, command still succeeds), routing runs with intent=false. A launch-at-login opt-out in that session then deregisters immediately while routing is on, and - since the safety-net block only runs inside `proxy_enable` - a subsequent crash strands the system proxy with nothing relaunching to self-heal, the precise scenario the deferral exists for. Deciding on the manager's live running state (or on `intent || running`) would close this.

### M4. `close_running_agents` matches by bare process name and will SIGTERM Claude Desktop (and any same-named binary)
`src-tauri/src/lib.rs:715-750` matches the lowercased process name (with `.exe` stripped) against `["claude", "codex", "opencode"]`. On macOS, Claude Desktop's main process is literally `Claude` (`Claude.app/Contents/MacOS/Claude`), so "Close running agents" terminates the whole desktop app, not just the CLI the doc comment ("agent CLIs") and the confirm copy describe. If closing Claude Desktop is intended (it is a routed tool), the copy and the count are at least misleading; if not, the match needs to be narrower. Any unrelated user binary named `claude`/`codex`/`opencode` is collateral either way - a caution the code applies to `hermes`/`openclaw` but not to these three.

## Low

### L1. Missed `proxy-state-changed` window at webview boot
`src/App.tsx:190-213` registers the listener after the initial-load effect starts; the backend emits exactly once, after the startup auto-enable (`src-tauri/src/lib.rs:1134`). If the enable completes after the initial `proxyStatus()` read resolves but before the async `listen()` registration lands, the flip is never observed: no routing notice, no stale-agents hint, and the popover shows routing off with nothing re-reading on reopen. The window is tens of milliseconds against a multi-second enable, so it is rare - but the new notice/stale features now depend on not missing it.

### L2. Settings: a failed post-toggle status re-read reverts a toggle that succeeded
`src/screens/Settings.tsx:129-144` wraps `setLaunchAtLogin(next)` and the follow-up `launchAtLoginStatus()` in one try/catch; if the re-read rejects after a successful toggle, the catch reverts the optimistic state and shows an error, leaving the switch displaying the opposite of the actual OS/marker state until the screen remounts.

### L3. `proxy_disable` mislabels provider-snapshot failures as `provider_restore`
`src-tauri/src/lib.rs:472` reports a `snapshot_and_disable_all()` failure under the `provider_restore` context. Analytics-only, but it makes disable-path failures indistinguishable from restore-path ones.

### L4. Safety-net arming silently skipped when `is_enabled()` errors
`src-tauri/src/lib.rs:434` uses `if let Ok(false) = mgr.is_enabled()`: an `Err` from the plugin skips the entire safety net with no log or `report_backend_error`, unlike every other failure branch in that block.

### L5. Settings pending-disable note copy assumes routing is on
`src/screens/Settings.tsx:333-343` ("While routing is on, Gate Connect stays in your login items...") also renders when the marker is pending with routing off - reachable when a safe-point deregistration failed and kept the marker (`src-tauri/src/lib.rs:518-527`), or on Linux after a clean quit (no exit handler). The mechanism is fine; the explanation is wrong for those states.

### L6. macOS `SO_REUSEADDR` can bind over another process's live wildcard listener
`crates/core/src/proxy/engine.rs:461-483`: the comment's claim that unix `SO_REUSEADDR` "never permits rebinding over a live listener" holds for identical addresses, but BSD/macOS allows a `127.0.0.1:P` bind with `SO_REUSEADDR` while another process holds a live `0.0.0.0:P` listener. Instead of falling back to an ephemeral port, the engine shadows that app's loopback traffic on the persisted port.

### L7. No synchronization between the marker/login-item mutation sites
`proxy_enable`'s safety-net tail (async runtime thread, `src-tauri/src/lib.rs:429-455`) and `set_launch_at_login` (`lib.rs:560-573`) both read-then-write the login item and marker with no shared lock. Example interleaving: the user toggles launch-at-login ON between the tail's `is_enabled()` check (false) and its arm+enable pair - the fresh opt-in ends up marked pending and reported as off. Windows are milliseconds wide; worth a note, not a redesign.

### L8. Repeated `account_reconcile` failures flood the analytics buffer
`src-tauri/src/lib.rs:204-207`: `get_account` runs on every popover reconcile and now reports each reconcile failure. A persistently failing reconcile produces an `error_shown` per popover interaction; the 32-entry cap bounds memory, not event volume.

### L9. Unreadable port file produces a spurious stale-clients notice
`src-tauri/src/lib.rs:1102` maps a `load_port()` I/O error to `None` via `.ok().flatten()`, which then compares unequal to the freshly saved port and shows the "restart your AI apps" notice even though the engine may have come back on the same port (the file was there, just unreadable at that instant).
