# Plan: persist "route through Gate" across a restart

Status: **implemented on main** (updated 2026-07-24). The plan below is
kept as historical context; see "As implemented" for where the shipped
behavior diverged from it.

## As implemented

All three pieces landed:

- **Intent flag** - `crates/core/src/proxy/intent.rs`, stored at
  `proxy/routing-intent.json` exactly as proposed. Set true/false in the
  `proxy_enable`/`proxy_disable` commands in `src-tauri/src/lib.rs`.
- **Launch at login** - `tauri-plugin-autostart` registered with the
  `--silent` launch arg; a silent launch skips the popover show.
- **Startup re-enable** - one spawned thread in setup chains
  `reconcile_on_startup()` then, if intent is true,
  `provider::restore_all()` + `manager().enable()`.

Divergences from the plan as written:

1. **Launch-at-login is NOT coupled to the routing toggle** (supersedes
   resolved decisions 1 and 3). It defaults **on** at first run via a
   one-shot `autostart-defaulted` marker file (`lib.rs`, setup), and is
   exposed as a standalone Settings toggle. The marker distinguishes
   "never configured" from "user turned it off" so an opt-out sticks.
2. **Exit-time intent handling is hands-off** (updated 2026-08-22; this
   item previously described a conditional clear that has been removed).
   The `RunEvent::Exit` handler calls `disable_quiet()` (reverts the
   system proxy) and leaves the intent alone: it survives every quit,
   only the routing switch (`proxy_disable`) clears it, and the startup
   auto-enable honors it on any launch - login item or manual. The
   `UPDATER_RELAUNCHING` flag (`set_updater_relaunching`, set by
   `UpdatePanel.tsx` before the install) now only carries a pending
   launch-at-login opt-out through a self-update relaunch.
3. **Port persistence** (not in the plan): the engine persists its
   loopback port (`proxy/port`, `proxy/pac-port`) and rebinds it with
   `SO_REUSEADDR` on the next enable, so a restart normally comes back
   on the same port and proxy-caching apps keep working. If the port
   changes anyway, `ROUTED_CLIENTS_MAY_BE_STALE` surfaces the "restart
   your AI apps" notice.
4. **Startup routing notice** (not in the plan): when routing comes back
   on at launch, the popover shows a takeover notice with a "close
   running agents" action (#93), addressing the stale-connection risk
   for already-running tools.
5. **Deferred launch-at-login opt-out** (branch
   `feat/deferred-launch-at-login-optout`, not yet on main): turning
   launch-at-login off while routing is on is deferred via a marker
   (`crates/core/src/proxy/autostart_optout.rs`) and completed on the
   next silent launch, exempted during updater relaunches.

## Problem

After a machine restart (observed on macOS), routing is off and the user
has to re-enable it manually. There is no bug in an "auto-restart" path -
**that path does not exist.** What looks like it (`reconcile_on_startup`)
is the opposite of a restore.

Current behavior, end to end:

1. **On exit** (`src-tauri/src/lib.rs` `RunEvent::Exit`) the app calls
   `proxy::manager().disable()` directly - reverts the system proxy and
   stops the engine. Every clean quit ends with routing **off** at the OS
   level.
2. **On launch** (`lib.rs`, the `reconcile_on_startup` thread) it runs
   `proxy::manager().reconcile_on_startup()` (`crates/core/src/proxy/manager.rs:268`).
   That function only *tears down*: restore the pre-Gate system-proxy
   snapshot, or force-clear any system proxy still pointed at a dead
   loopback port (`clear_stranded_loopback`). It never calls `enable()`.
   Its job is to guarantee HTTPS isn't stranded at a dead engine port.
3. **No autostart** (`lib.rs` setup comment: "No autostart"). After a
   reboot the app + engine aren't running at all until the user opens the
   menu-bar popover.

The `restore-snapshot.json` in `crates/core/src/provider.rs` is *not*
shutdown persistence - it's the master-switch snapshot, written when the
user toggles routing **off** (`snapshot_and_disable_all`) and cleared when
they toggle it back **on** (`restore_all`).

## Why it isn't a one-line fix

The system proxy points HTTPS at a loopback port served by the engine
**inside the app process**. If the app isn't running, that port is dead and
every proxy-honoring app breaks - exactly the stranded state
`reconcile_on_startup` exists to clean up. So "re-enable on restart" is only
safe **combined with launch-at-login**; otherwise we'd either re-enable into
a dead port or have nothing running to re-enable.

## Design

Three pieces:

### 1. Persist a routing "intent" flag

A dedicated flag separate from the provider restore-snapshot. Semantics:

- Set **true** when the user enables routing (`proxy_enable` command).
- Set **false** when the user explicitly disables routing
  (`proxy_disable` command).
- **Not touched** by the exit-time `manager().disable()`. This separation
  already holds: exit calls the manager directly, not the command, so the
  flag survives a normal quit. Verify this stays true.

Storage: a small JSON/flag file under `env::app_support_dir()`, alongside
`provider/restore-snapshot.json` and the proxy domain config. Suggest
`proxy/routing-intent.json` (`{ "enabled": true }`). New helpers in
`crates/core/src/proxy/` (e.g. `intent.rs`): `set_intent(bool)`,
`load_intent() -> bool`.

Wire into the two commands in `lib.rs`:
- `proxy_enable` (currently `lib.rs:343`): on success, `set_intent(true)`.
- `proxy_disable` (currently `lib.rs:378`): on success, `set_intent(false)`.

### 2. Launch at login

Add `tauri-plugin-autostart` (v2) to `src-tauri/Cargo.toml` and register it.
Per-OS it uses login items (macOS), registry Run key (Windows), and a
`.desktop` autostart entry (Linux).

- Add the macOS login-item permission to `src-tauri/capabilities/default.json`
  (`autostart:default` or the specific allow-* perms the plugin needs).
- **Couple autostart to routing intent**, don't expose it as an independent
  toggle the user has to discover. When the user turns routing on, enable
  autostart; when they turn it off, disable autostart. (Confirm this product
  decision - see open questions.)

### 3. Re-enable on startup, after reconcile, without flashing the popover

In `lib.rs` setup, **after** `reconcile_on_startup` has run (ordering
matters - reconcile must clean any stranded state first), if
`load_intent()` is true:

- `provider::restore_all()` then `proxy::manager().enable()` - mirroring the
  `proxy_enable` command body. Run it off the main thread (enable can block
  on the CA-trust prompt and waits up to ~10s for engine readiness), the same
  way `reconcile_on_startup` is already spawned.
- **Sequence with reconcile:** reconcile currently runs in its own
  `std::thread::spawn`. The auto-enable must run *after* it completes, not
  concurrently, or they race on the system-proxy state. Chain them in one
  spawned thread (reconcile, then conditional enable) rather than two.

**Suppress the popover on an autostart launch.** We just changed setup to
show the popover on launch ("default to open"). A login-triggered launch
should start **silent** - no window stealing focus at login - and just bring
routing up in the background. `tauri-plugin-autostart` can pass a launch arg
(e.g. `--minimized`/a custom flag); detect it in setup and skip the
`window.show()` + `set_focus()` in that case. The tray icon still appears.

## Files to touch

- `crates/core/src/proxy/intent.rs` (new) - intent flag persistence.
- `crates/core/src/proxy/mod.rs` - export the new module.
- `src-tauri/Cargo.toml` - add `tauri-plugin-autostart`.
- `src-tauri/src/lib.rs` - register the plugin; set intent in
  `proxy_enable`/`proxy_disable`; chain auto-enable after reconcile in setup;
  skip popover show on autostart launch; couple autostart to intent.
- `src-tauri/capabilities/default.json` - autostart permission.
- Possibly `src/` - if we surface any of this in the UI (a "keep routing on
  after restart" note/affordance on the routing screen).

## Risks / things to verify on macOS

- **Keychain at login.** Auto-enable reads the Gate API key from the
  keychain. The login keychain is unlocked at login, so a read should
  succeed without a prompt *if the ACL allows our binary* - but the first
  autostart after granting could prompt. A prompt at login with no popover
  visible is bad UX. Confirm the engine's key read is promptless in the
  autostart context, or defer enable until the key is confirmed readable.
- **CA-trust prompt.** `enable()` can block on the CA-trust admin prompt. If
  the CA is already trusted (normal steady state) this is a no-op; confirm a
  login-time enable never surfaces an admin prompt. If it can, don't
  auto-enable - show the popover and let the user confirm instead.
- **Ordering race** between reconcile and auto-enable (addressed above by
  chaining in one thread).
- **Cross-platform parity.** `reconcile_on_startup`/`enable` exist in
  `manager.rs` (macOS), `manager_linux.rs`, and `manager_windows.rs`. The
  intent + startup-enable logic lives in `lib.rs` and is platform-agnostic,
  but autostart and the silent-launch arg differ per OS - verify on each.
- **Stale config-based tools.** Config-based providers (Codex) keep their
  Gate pointers across exit (exit only reverts the system proxy, not
  per-tool config). Auto-enable should leave them as the user left them;
  confirm `restore_all()` + `enable()` reproduces the prior steady state and
  doesn't double-apply.

## Decisions (resolved)

1. **Couple autostart to routing.** Turning routing on registers a login
   item and arms re-enable-after-reboot; turning it off removes it. One
   toggle, with a visible note on the routing screen so the login item isn't
   a surprise. (Not a separate user setting.)
2. **Skip silent prompts at login.** If auto-enable would require *any*
   prompt (keychain unlock, CA-trust, admin), do **not** prompt - open the
   popover and let the user confirm the enable instead. Never surface a
   stray dialog at login.
3. **Register the login item on the first routing enable.** The first time
   the user turns routing on, also register autostart.