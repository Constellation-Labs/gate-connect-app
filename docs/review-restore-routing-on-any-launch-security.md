# Security review: fix/restore-routing-on-any-launch

Scope: branch fix/restore-routing-on-any-launch vs main. One commit, one file
changed (src-tauri/src/lib.rs, +80/-86). Reviewed the diff plus the unchanged
trust anchors it now leans on harder: crates/core/src/proxy/intent.rs,
crates/core/src/proxy/autostart_optout.rs, crates/core/src/env.rs
(app_support_dir), and the surrounding startup/exit paths in
src-tauri/src/lib.rs. Lens: security only.

Behavior under review: the persisted routing intent
(app-support/proxy/routing-intent.json) now survives every exit
(src-tauri/src/lib.rs:2192-2196 replaces the old conditional
set_intent(false) on exit), so any launch with intent=true re-enables the
system proxy and local MITM engine (src-tauri/src/lib.rs:1816-1845). The
crash safety net was extracted into fn arm_crash_safety_net
(src-tauri/src/lib.rs:682-718) and is now also called from the startup
auto-enable path (src-tauri/src/lib.rs:1845).

## Findings

### High

None.

### Medium

None.

### Low

1. The routing switch for system-wide interception is now a single
   unauthenticated, user-writable JSON file, honored on every launch.
   intent.rs writes plain JSON with fs::write and no integrity protection
   (crates/core/src/proxy/intent.rs:42-48) into the per-user data dir
   (crates/core/src/env.rs:159-161). Before this branch a planted
   intent=true file only took effect when launch-at-login was on or after
   a crash; the old exit handler cleared it otherwise. Now any process
   running as the user can drop {"enabled":true} and the next launch flips
   the OS proxy and starts the MITM engine unattended
   (src-tauri/src/lib.rs:1816, 1838). This does not cross a privilege
   boundary: a same-user attacker can already register login items, edit
   tool configs, or set the system proxy directly, and enabling the user's
   own gateway routing grants the attacker nothing (keys stay in the
   keychain, the engine and CA are the app's own). Rated L as a
   widened-trigger observation, not a boundary break; no fix required,
   though the report should record it as an accepted consequence of the
   restore-on-any-launch design.

## Accepted-by-design (verified, not findings)

- Consent/surprise: routing now resumes on launches the user did not
  associate with routing (manual open weeks later, updater relaunch). This
  is the branch's stated purpose, the intent is only ever set by the user's
  explicit toggle (src-tauri/src/lib.rs:654, cleared only by proxy_disable),
  and the flip is made visible: tray retint (src-tauri/src/lib.rs:1882),
  proxy-state-changed event to the popover (src-tauri/src/lib.rs:1887), and
  on a failed silent enable the popover is opened rather than prompting
  (src-tauri/src/lib.rs:1897-1903).
- The is_enabled()/enable() read-then-write race in arm_crash_safety_net is
  documented as accepted in the function docs (src-tauri/src/lib.rs:675-680);
  both racers are driven by one user in one popover and the worst case is a
  mislabeled pending marker until the next safe point.

## Checked and clear

- Corrupt or missing intent file fails safe: read_intent maps any read or
  parse failure to false, i.e. passthrough, never auto-route
  (crates/core/src/proxy/intent.rs:38-40, 50-57). A torn write from the
  non-atomic fs::write therefore fails toward "off", the safe direction.
- Symlink following on the intent write path (fs::write,
  crates/core/src/proxy/intent.rs:47) requires same-user file control and is
  pre-existing code untouched by this branch; the readable content is a
  single boolean, no secret.
- No login-item launch loop. The login item launches with --silent
  (src-tauri/src/lib.rs:1464-1466); a silent launch with the safety-net
  marker pending exits at src-tauri/src/lib.rs:1790-1806 before the intent
  check, so it never re-enables or re-arms. A manual relaunch with the
  marker still pending hits the Ok(true) no-op arm at
  src-tauri/src/lib.rs:709 and cannot double-register or clobber a real
  opt-in.
- The startup-path login-item registration is not a durable persistence
  mechanism: marker is armed before enable() so a crash between the two
  cannot leave a registration that reads as the user's choice
  (src-tauri/src/lib.rs:688-698), and complete_pending_autostart_disable
  deregisters before clearing the marker, keeping the marker on a failed
  deregistration that leaves the item registered
  (src-tauri/src/lib.rs:777-796). The exit handler still runs it on every
  non-updater quit (src-tauri/src/lib.rs:2189-2191).
- New/moved eprintln lines log OS/plugin error displays and support-dir file
  paths only; no tokens, keys, or gateway credentials. report_backend_error
  is a bounded local in-memory buffer drained to the frontend, not a remote
  sink (src-tauri/src/lib.rs:982-997).
- Privilege boundaries unchanged: no new elevation, no new IPC surface, no
  new Tauri commands; arm_crash_safety_net is a private fn reusing the exact
  code proxy_enable already ran, and the startup enable path can only reach
  it after the same manager().enable() that the user-facing toggle uses
  (src-tauri/src/lib.rs:1838-1845).
- Silent-launch opt-out path still forces routing off before exiting on
  Linux (disable_quiet at src-tauri/src/lib.rs:1797-1803), so removing the
  intent-clear there leaves no headless interception; macOS/Windows were
  reconciled to off just above (src-tauri/src/lib.rs:1768).
- proxy_disable remains the durable off: it clears the intent
  (set_intent(false) at src-tauri/src/lib.rs:754), matching the
  exit-handler comment at src-tauri/src/lib.rs:2192-2196.

## Verdict

No must-fix or should-fix security issues; the branch widens when a
pre-existing same-user trust assumption fires (one L observation) but
crosses no privilege boundary, fails safe on corrupt state, and introduces
no persistence loop or secret leakage.
