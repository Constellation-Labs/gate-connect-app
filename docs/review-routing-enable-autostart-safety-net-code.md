# Code Quality Review: `feat/routing-enable-autostart-safety-net` (v0.1.1...HEAD)

Scope: 44 files, ~2200 insertions. The range spans several merged features: the launch-at-login
safety net and deferred opt-out (`autostart_optout.rs` + `lib.rs` wiring), stable port
persistence across restarts (`system_proxy*` + `engine.rs` + managers), the build-fingerprint
daemon handshake (`build.rs` + `control.rs` + `helper_client.rs`), analytics coverage
(backend error buffering, PostHog dimensions), the routing takeover notice, and the update
banner. Overall quality is high: the new `autostart_optout` module is small, well-documented,
and thoroughly unit-tested; the fingerprint handshake and port-reuse behaviors both got real
integration tests (`stale_daemon_handshake.rs`, `proxy_e2e.rs`); comments consistently explain
*why*, not *what*. No must-fix (H) issues were found. The findings below are mostly
duplication that will tax future edits, a couple of misleading/fragile spots in `lib.rs`, and
frontend test gaps around the branch's flagship features.

## H (must-fix)

None found.

## M (should-fix)

- **`src-tauri/src/lib.rs:472` - analytics context mislabeled.** The failure of
  `provider::snapshot_and_disable_all()` inside `proxy_disable` is reported as
  `report_backend_error("provider_restore", ...)`, which classifies frontend-side to
  "Couldn't restore provider routing". This is the *disable/snapshot* path, not a restore;
  every dashboard reading of this event will point at the wrong operation. Either add a
  `provider_disable` context (and its `BACKEND_CONTEXTS` / title entries in
  `src/lib/errors.ts`) or at minimum use a truthful label.

- **`src-tauri/src/lib.rs:1100-1116` - stale-clients detection re-reads the port file instead
  of using the value in hand.** `prior_port` is loaded, `enable()` runs, then `new_port` is
  loaded back from disk to compare. `enable()` already returns `ProxyState` whose `port`
  field (`crates/core/src/proxy/mod.rs:136`) is the authoritative bound port, and the
  post-enable `save_port` is explicitly best-effort (`manager.rs:122`) - if that write fails,
  the re-read returns the stale prior value, `prior_port == new_port`, and the "restart your
  AI apps" notice is suppressed exactly when the persisted state is wrong. Comparing
  `prior_port != state.port` is both simpler and correct.

- **`crates/core/src/proxy/system_proxy.rs:61-113`, `system_proxy_windows.rs:67-119`,
  `system_proxy_linux.rs:86-105` - port persistence triplicated.** `load_port`/`save_port`
  (and on macOS/Windows `load_pac_port`/`save_pac_port`) are byte-for-byte identical modulo
  doc comments: same `read_to_string` + `trim().parse::<u16>().ok()` + NotFound handling,
  same `write_file(..., 0o644)`. That is seven near-identical functions across three files;
  a format or error-handling change now needs three coordinated edits. The logic has no
  platform-specific content - a small shared helper (e.g. `port_persist.rs` with
  `load(name)`/`save(name, port)`) would collapse it, and the round-trip test
  (`system_proxy.rs:507-518`, duplicated in the Linux module) with it.

- **`src/screens/Settings.test.tsx:12` - stale mock shape hides the branch's own feature.**
  `launchAtLoginStatus` is mocked as `mockResolvedValue(false)`, the pre-change boolean
  shape; the component now reads `status.enabled` / `status.pending_disable`
  (`Settings.tsx:96-99`), so every test silently runs with `undefined` state and TypeScript
  can't catch it (the mock factory is untyped). Consequence: the new pending-disable note
  (`Settings.tsx:332`) - the user-visible half of the deferred opt-out - has zero test
  coverage, and the launch-at-login toggle's re-read-after-apply path
  (`Settings.tsx:136-139`) is untested. Fix the mock to return `{ enabled, pending_disable }`
  and add a case asserting the note renders when `pending_disable` is true.

- **`src/components/StartupRoutingNotice.tsx` - new interactive component with no tests.**
  It carries real state-machine logic: the inline confirm step (`confirming`), the
  post-close copy switch on `closed === null` vs count vs zero (lines 59-72, including
  pluralization), and the error path from `closeRunningAgents`. Sibling surfaces of the same
  weight got tests this branch (`UpdatePanel.test.tsx`, `Settings.test.tsx`); this one, the
  takeover users actually act on, did not.

- **`src/App.tsx:218-232` and `src/components/UpdatePanel.tsx:60-70` - duplicated
  blur-then-focus "reopen" detector.** Both effects hand-roll the identical
  `let blurred = false; onFocusChanged(...)` edge detector (App even points at UpdatePanel's
  copy in its comment at `App.tsx:216`). Two independent listeners on the same window event
  encode the same policy; when the tray's show/focus behavior changes, both must be found
  and updated. A tiny shared hook (`useWindowReopen(onReopen)`) removes the drift risk.

## L (nit)

- **`crates/core/src/proxy/engine.rs:50-59` - cfg-gated struct field spreads cfg noise.**
  `preferred_pac_port` being `#[cfg(any(windows, macos))]` forces a matching cfg attribute at
  every `EngineConfig` construction site - currently 8 (both managers plus six sites in
  `crates/core/tests/proxy_e2e.rs:161,241,355,430,488,559`). A plain `Option<u16>` that
  Linux passes as `None` (like `owner_uid`, which stays unconditional despite being
  "a Linux concern") would cost nothing and delete the attribute at every site.

- **`src-tauri/src/lib.rs:434` - silent skip on `is_enabled()` error.** The safety-net block
  uses `if let Ok(false) = mgr.is_enabled()`, so a probe error silently skips arming the net,
  while every other failure in the same block goes through `report_backend_error`. Worth a
  one-line comment (or a report) so the asymmetry reads as chosen, not missed.

- **`src/screens/Settings.tsx:78,88,101,114,142` - `trackError(err, "generic")` where
  specific contexts now exist.** This branch introduced fine-grained contexts precisely so
  errors classify usefully, and `launch_at_login` is in the set - but the
  `launchAtLoginStatus()` load failure (line 101) and the post-toggle re-read (line 142)
  still report `generic`. Same for the key-prefix loads if `save_api_key`/`startup` fit.

- **`src/lib/errors.ts:41-43` - `backendErrorContext` untested.** The unknown-context ->
  `generic` degradation is the one behavioral guarantee protecting the analytics allowlist
  from a backend label added without a frontend counterpart; `errors.test.ts` was touched in
  this range but no case pins it.

- **`.github/workflows/release-notes-slack.yml:44-57` - regex markdown->mrkdwn converter is
  inline and untestable.** Eight chained regexes (including lookbehinds) living inside a YAML
  `github-script` block can only be verified by publishing a release or the manual dispatch
  hook. Fine for a best-effort notification, but if it grows at all, extract it to a scripted
  file under `ci/` where a unit test can feed it sample release bodies.

- **`src-tauri/src/lib.rs:708-714` / `close_running_agents` - name-only process matching is
  broad.** Matching lowercase basename `claude` will also SIGTERM Claude Desktop/Cowork
  (process name "Claude"), not just the CLI the doc comment describes. If that is intended
  (desktop clients also need a restart), say so in the comment; if not, the match needs
  tightening. Flagged here as comment hygiene; the behavioral question belongs to the
  correctness lens.
