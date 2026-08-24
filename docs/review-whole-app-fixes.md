# Whole-app review: disposition of findings

Companion to `review-whole-app-code.md`, `review-whole-app-security.md`, and
`review-whole-app-correctness.md` (all against main @ 7fc3d16). Every finding
from the three reports is listed with what was done. Verification at the end.

## Correctness

- **H1 (side-effect engine starts never persist the routing intent)** - FIXED.
  `connect_tool` persists the intent before its auto-enable
  (`src-tauri/src/lib.rs`, `connect_tool`); the master-ON/OFF ceremonies moved
  into `gate_connect_core::routing::{enable,disable}` which persist/clear the
  intent themselves, and the CLI's `proxy enable`/`proxy disable` now use them
  (so a CLI enable also survives a restart, and a CLI disable is a real
  master-off: sweep + intent clear). The second half of the finding - Claude
  Code and Codex reading Connected against a dead relay - is fixed by a
  liveness probe (`proxy::relay_listening`, a loopback connect against the
  persisted relay port so a standalone `proxy relay` host also counts) in both
  integrations' `status()`, mirroring Hermes. Tests that seeded a dead relay
  port and asserted Connected now bind a real listener
  (`master_cycle_preserves_members.rs`, `reconcile_enabled.rs`).
- **M1 (engine crash leaves the tray green and the popover reading On)** -
  FIXED. Core exposes `proxy::set_engine_crash_observer`; the manager's crash
  fail-safe notifies it after the revert, outside the engine lock
  (`manager.rs`/`manager_windows.rs::handle_engine_crash`). The shell registers
  an observer at setup that repaints the tray and emits `proxy-state-changed`
  with the post-crash state.
- **M2 (domain-watcher retirement race)** - FIXED. The watcher now stores
  `WATCHER_ALIVE = false` under the same engine-lock acquisition that observed
  the engine gone; `enable` spawns the watcher while holding that lock, so the
  load-then-store interleaving is impossible (both managers).
- **L1 (Windows reconcile lacks the stranded-loopback sweep)** - FIXED.
  `system_proxy_windows::clear_stranded_loopback` clears a dead-loopback
  `AutoConfigURL` (and a plain `host:port` static slot), and
  `reconcile_on_startup` runs it unconditionally instead of early-returning on
  a missing snapshot, mirroring macOS.
- **L2 (`disable_inner` teardown outside the engine lock)** - FIXED. Both
  managers hold the lock for the whole teardown; a concurrent enable now waits
  instead of being falsely refused or having its fresh snapshot deleted. The
  crash callback's bounded try_lock deferral stays correct (disable IS the
  revert it wanted to run); comments updated.
- **L3 (crash window between engine-on and intent persist)** - FIXED.
  `routing::enable` persists the intent before starting the engine; a failed
  enable leaves it set, which the startup auto-enable already retries and
  degrades quietly.
- **L4 (PAC/relay accept loops spin hot)** - FIXED. Both loops sleep 100ms on
  an accept error.
- **L5 (stale-closure double-submit on proxyBusy)** - FIXED. One shared
  `useRef` busy guard checked-and-set synchronously in every mutation callback
  in `App.tsx`; the state stays for rendering.
- **Q1 (pac_script does not filter `enabled`)** and **Q2 (Linux reconcile
  re-honors from the snapshot independent of the intent)** - reviewed and left
  as-is deliberately: every `pac_script` sender passes `enabled_only` today,
  and the Linux snapshot re-honor covers the CLI-enabled detached daemon,
  which should be re-honored. Recorded here so both stay conscious choices.

## Security

- **H1 (unauthenticated loopback credential deputy on macOS/Windows)** -
  ADDRESSED per the report's stated minimum: the exposure, its exact blast
  radius, the shipped mitigations, and the cross-user token follow-up are
  documented in `docs/security-notes-loopback.md`. The browser half of the
  exposure is closed by H2 below; the same-user half is the OS trust boundary
  a token cannot fix (the caller can read the 0600 configs); the cross-user
  macOS/Windows gap is a tracked follow-up (path-token shape measured viable).
- **H2 (relay accepts cross-origin / DNS-rebound browser requests)** - FIXED.
  The relay refuses non-loopback `Host` and any non-loopback `Origin` before
  resolving or injecting anything (`relay.rs::proxy`), the PAC responder
  refuses non-loopback `Host` (`engine.rs::serve_pac`), and the acceptance
  rule is shared in `proxy/mod.rs` (`authority_is_loopback` /
  `origin_is_loopback`) with unit tests plus an end-to-end pin
  (`relay_e2e.rs::relay_refuses_rebound_host_and_cross_site_origin`).
- **M1 (test seams honored in any build)** - FIXED. All `GATE_CONNECT_TEST_*`
  reads route through `env::test_seam`, which honors them in debug builds
  only and loudly ignores them in release. CI is unaffected: `cargo test` and
  the e2e's `cargo build -p gate-connect-cli` are both debug.
- **M2 (claude-web ships the session cookie to the gateway)** - ACCEPTED as
  the product decision it already was (opt-in, CLI-only, flagged pending
  validation); recorded in `docs/security-notes-loopback.md` with the rule
  that any UI surfacing re-opens the decision.
- **M3 (analytics/PostHog posture)** - the review's ask was "keep the
  allowlist as the tripwire it is"; it is now pinned by unit tests
  (`src/lib/analytics.test.ts`): unlisted props are dropped and error events
  carry only the classified title.
- **L1 (`report_backend_error` ships full anyhow chains to the webview)** -
  ACCEPTED, as the report recommends: the sink is in-process, the analytics
  layer re-classifies before anything leaves the machine (now test-pinned).
- **L2 (id_token email parsed unverified)** - ACCEPTED; display-only and
  documented at the parse site.
- **L3 (install-id written with default perms)** - FIXED; written 0600 via
  `primitives::write_file` for convention's sake.

## Code quality

- **H1 (enable/disable ceremony duplicated in three places)** - FIXED. One
  policy pair in `crates/core/src/routing.rs`; the app commands, the startup
  auto-enable, and the CLI are one-line callers that only differ in how they
  surface best-effort `Warning`s.
- **H2 (startup business logic untestable in the shell)** - PARTIAL. The
  OAuth refresh + gateway session probe + stale-org clearing moved to
  `gate_connect_core::startup::refresh_session()` (the shell maps the verdict
  onto its tray flag), and the enable ceremony moved via H1. The remaining
  startup thread is genuinely shell-coupled (silent-launch `exit`, autostart
  plugin, tray, events) and stays.
- **H3 (manager.rs / manager_windows.rs ~85% duplicated, ProxyManager
  untested)** - DEFERRED. Unifying them is a cross-platform refactor that
  cannot even be compile-checked off-platform locally (cross `cargo check`
  fails on ring/aws-lc build scripts) and deserves its own change with real
  per-OS verification. This round kept the pair in lockstep (all three
  manager fixes applied to both, same wording) rather than half-merging them.
- **M1 (upstream-credential feature dead but wired)** - FIXED. Trait default
  flipped to `false` (the trap the review called out), the six redundant
  overrides deleted, the three renderer commands + registrations and the
  unreachable `connect_tool` gate removed, the DTO field and the three api.ts
  wrappers dropped along with the fixture fields; the CLI keeps the trait
  surface (`set-upstream` / `clear-upstream`), noted where the commands were.
- **M2 (unused frontend wrappers / registered-but-uncalled commands)** -
  FIXED. `providerEnable` / `providerDisable` / `proxyListDomains` wrappers
  removed; the `provider_enable` / `provider_disable` / `proxy_list_domains`
  commands and registrations removed (the e2e mock keeps its handlers, and
  routing.spec.ts's "never called" assertion still pins the UI behavior);
  a comment at the old registration site says where the provider layer lives.
- **M3 (JSON settings helpers triplicated)** - FIXED. One
  `integrations/json_config.rs` (load / JSON5 load / 0600 atomic write /
  ensure_object); Claude Code, OpenCode, OpenClaw are thin wrappers. The
  `reject_non_object_env` guard stays Claude-Code-specific by design
  (`ensure_object`'s doc now points at it).
- **M4 (three hand-rolled resyncs with divergent fallbacks)** - FIXED. One
  `resyncLedger` helper; a transient `listTools` failure now keeps the
  previous list everywhere (stale beats blank; popover reopen heals it).
- **M5 (test gaps)** - PARTIAL. The highest-value gap (analytics scrubbing, a
  privacy promise) is now unit-tested. Still open, in rough priority order:
  ProxyManager sequencing (blocked on H3's seam), `ca_windows.rs`,
  the Linux helper protocol, `FirstRun.tsx` / `OrgPicker.tsx` units.
- **L1 (stale `account` dep in toggleProxy)** - FIXED.
- **L2 (core logging is eprintln-only)** - DEFERRED as the "one decision" the
  review asked for: keep eprintln for now; the failures that matter reach the
  UI through `report_backend_error`, and a uniform prefix/macro sweep is
  mechanical whenever wanted.
- **L3 (comments narrating repository history)** - FIXED on the frontend
  (the three cited "used to" strata now state the current invariant). The
  Rust "used to" comments were reviewed and kept: each documents a fixed
  regression as the rationale for a present invariant, which is this repo's
  comment culture, not archaeology.
- **L4 (file-size seams)** - PARTIAL. `default_domains` and its rationale
  essays moved to `proxy/catalog.rs` (~290 lines out of `proxy/mod.rs`, with
  a re-export so callers are unchanged). The `src-tauri/src/macos.rs` split
  is DEFERRED: it is ~600 lines of cfg-gated objc glue sharing statics with
  the command surface, uncheckable off-platform, so a blind move risks the
  macOS build for a purely organizational win. `engine::start()` left as the
  review itself judged it (splittable but cohesive).

## Verification

- `cargo test --workspace`: 249 passed, 0 failed (Linux host).
- `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets`:
  clean.
- `pnpm test`: 282 passed (20 files); `tsc --noEmit` and `tsc -p e2e
  --noEmit`: clean.
- macOS/Windows-gated code (both managers, the Windows sweep, the crash
  observer registration, the PAC Host gate) compiles only in CI's per-OS
  jobs; cross-target `cargo check` from Linux fails on native build scripts
  (ring/aws-lc), so those paths need the usual CI pass plus a real-machine
  smoke of: engine crash repaints the tray, disable/enable flips inside one
  second, Windows boot after a crash clears a stale `AutoConfigURL`.
