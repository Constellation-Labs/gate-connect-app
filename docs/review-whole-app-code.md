# Code quality review - whole application

Date: 2026-08-23. Lens: code quality (architecture/layering, file size, error
handling, dead code, frontend structure, module-level test gaps). No source
files were modified.

## What was inspected

Read in full: `src-tauri/src/lib.rs`, `src/App.tsx`, `src/lib/api.ts`,
`src/lib/errors.ts`, `crates/core/src/integrations/claude_code.rs`,
`crates/core/src/proxy/mod.rs` (lines 1-1250 in full; the remainder verified
to be the test module). Read in large part: `crates/core/src/proxy/engine.rs`
(config/API surface plus full function map), `crates/core/src/provider.rs`
(catalog and policy header), `src/screens/Home.tsx` (first ~450 lines),
`src/screens/Settings.tsx` (first ~130 lines). Function-level skims plus
targeted greps/diffs: `proxy/manager.rs`, `manager_windows.rs`,
`manager_linux.rs`, `proxy/relay.rs`, `integrations/opencode.rs`,
`openclaw.rs`, `codex.rs`, `hermes.rs`, `crates/cli/src/main.rs`. Inventoried:
`crates/core/tests/` (17 integration test files), `src/**/*.test.{ts,tsx}`,
`e2e/` Playwright specs, `ci/e2e/`.

Not inspected line-by-line: `oauth.rs`, `account.rs`, `keychain.rs`,
`audit.rs`, `ca.rs` / `ca_windows.rs` / `ca_linux.rs`, `system_proxy_*.rs`,
`helper.rs` / `helper_client.rs`, `env.rs`, `registry.rs` internals,
`GroupMembers.tsx`, `FamilyPanel.tsx`, `Diagnostics.tsx`, `Onboarding.tsx`,
`src/lib/groups.ts` internals, `ci/e2e/run.sh`. Findings below do not cover
those files' internals.

Overall impression: this is a well-cared-for codebase. Error classification
(`src/lib/errors.ts`), the catalog invariant tests in `proxy/mod.rs`
(`every_resolved_endpoint_lands_on_an_inference_prefix`,
`forwarded_paths_avoid_gate_reserved_prefixes`), and the engine/relay
integration tests are genuinely strong. No TODO/FIXME debt, no commented-out
code, no `#[allow(dead_code)]` suppressions were found. The findings below
are mostly about duplication that has already started to cost (the same fix
landing in three places) and a feature surface that is dead but still wired.

## Critical

None found.

## High

### H1. The proxy enable/disable provider ceremony is duplicated in three places across two crates

The correct enable sequence is "restore providers, enable engine, restore
again for domain-only providers", and the correct disable sequence is
"snapshot-and-disable everything, then stop the engine". That policy is
hand-copied at:

- `src-tauri/src/lib.rs:617-644` (`proxy_enable` command)
- `src-tauri/src/lib.rs:1840-1895` (startup auto-enable thread, same
  two-pass restore plus enable)
- `crates/cli/src/main.rs:572-585` (`proxy enable`, whose own comment says
  "mirrors the app's proxy_enable flow")
- disable half: `src-tauri/src/lib.rs:743-758` (`proxy_disable`) and
  `src-tauri/src/lib.rs:1429-1431` (`disconnect_tools_for_quit`)

Each site re-explains the same invariants in comments (why restore must run
before enable, why a second pass is needed, why the sweep must precede the
kill switch). The cost of leaving it: the sequence encodes correctness, not
convenience - a future fix to the ordering (or a third pass, or a new
precondition) has to land in three places, and the CLI comment is the only
thing tying it back to the app. A `provider::enable_routing(manager)` /
`provider::disable_routing(manager)` pair in `crates/core` would make the
shell and CLI one-line callers and make the sequence unit-testable where the
provider snapshot logic already lives.

### H2. ~240 lines of startup business logic live in the Tauri shell and are untestable there

`src-tauri/src/lib.rs:1700-1923` (the thread spawned in `setup`) contains
real policy, not shell glue: refresh the OAuth token, probe the gateway for
session validity, clear a stale org selection, reconcile a stranded system
proxy, complete a deferred autostart opt-out (with a Linux-only
`disable_quiet` special case), restore providers, re-enable routing, and
detect an engine/PAC port move. Nothing in `src-tauri` has tests, the CLI
cannot reuse any of it, and the function nests five levels of
platform-`cfg`'d conditionals inside a closure. The cost: this is the code
path every boot depends on, it changes often (three of the last five commits
on `main` touched it or its collaborators), and the only verification is
manual VM testing. Extracting a `startup::restore_session()` (or similar)
into `gate-connect-core` that returns a summary struct (session state, port
moved, enable outcome) and leaving only tray updates, event emission, and
`report_backend_error` in the shell would let the sequencing be tested with
the same harness `crates/core/tests` already uses.

### H3. manager.rs and manager_windows.rs are ~85% identical, and no test drives ProxyManager on any platform

A whitespace-normalized diff of `crates/core/src/proxy/manager.rs` (600
lines) against `manager_windows.rs` (587 lines) shows only 179 changed lines
total; the function-by-function surface is one-to-one (`enable`, `disable`,
`disable_inner`, `set_domain`, `refresh_api_key/token/org`, `trust_ca`,
`untrust_ca`, `refuse_untrust_while_running`, `handle_engine_crash`,
`reconcile_on_startup`, `spawn_domain_watcher`). `manager_linux.rs` diverges
for a real reason (the detached helper daemon), but the macOS/Windows pair
duplicates orchestration whose only genuine differences are the
`system_proxy` and `ca` calls, which are already abstracted behind
platform-`#[path]` modules. Meanwhile `grep` finds zero uses of
`proxy::manager()` in `crates/core/tests/` or `crates/cli/tests/`: the
engine and relay are tested directly, but the manager's sequencing (engine
crash handling, snapshot handling, the untrust-while-running refusal, port
persistence interplay) is tested nowhere. The cost is concrete and already
visible in history: crash-recovery and port-persistence fixes (`9340dc8`,
`bedf13a`) had to be reasoned about per-platform. Merging the shared
sequencing into one generic manager parameterized over the platform modules
would halve the surface and finally give the sequencing a test seam.

## Medium

### M1. The upstream-credential feature is dead but fully wired end to end

`registry.rs:110` defaults `requires_upstream_credential` to `true`, and all
seven integrations override it to `false`. Nothing can therefore require an
upstream credential today, yet the machinery survives everywhere:

- `src-tauri/src/lib.rs:169-195` - `has_upstream_credential`,
  `save_upstream_api_key`, `clear_upstream_credential` commands (registered
  at lib.rs:1496-1498)
- `src-tauri/src/lib.rs:123-131` - `connect_tool`'s credential gate, which
  can never fire and whose error message hard-codes "Anthropic"
- `src/lib/api.ts:60-65` - `hasUpstreamCredential`, `saveUpstreamApiKey`,
  `clearUpstreamCredential`, all with zero call sites in `src/` or `e2e/`
- `Tool.requires_upstream_credential` (api.ts:15) - unused in any screen

Two costs. First, the trait default is a trap: a new integration that forgets
to override the method blocks `connect` with an error about a credential
there is no UI to enter. Second, the dead commands remain part of the
renderer-reachable attack/maintenance surface for no benefit. Either delete
the UI-side surface and flip the trait default to `false` (the CLI's
`crates/cli/src/main.rs:426,544` still uses the trait and can keep it), or
keep it deliberately and say so where the default is declared.

### M2. Unused frontend API exports mirroring commands the UI no longer calls

`src/lib/api.ts` exports `providerEnable` (line 229), `providerDisable`
(line 233), and `proxyListDomains` (line 169) with zero uses outside the
file. `src/App.tsx:412-414` even documents that the provider layer is
"analytics-only... the UI no longer shows providers", yet `provider_enable` /
`provider_disable` stay registered in both handler blocks
(`src-tauri/src/lib.rs:1521-1522, 1573-1574`). The cost: every reader has to
rediscover that these paths are CLI-only, and the api.ts wrappers imply a UI
contract that does not exist. Drop the three wrappers; if the commands must
stay for the CLI-parity story, a one-line comment on the registration saying
"CLI/core path only, no UI caller" would stop the archaeology.

### M3. JSON settings-file helpers are triplicated across integrations

`load_settings` / `write_settings` / `ensure_object` (and `settings_path`)
exist nearly verbatim in three files: `claude_code.rs:280-315`,
`opencode.rs:631-720`, `openclaw.rs:587-660` (openclaw's loader differs only
in parsing JSON5). The guard against clobbering a malformed value
(`reject_non_object_env`, claude_code.rs:322-330) exists only in
claude_code, so the protection the comment argues for is not shared by the
siblings that do the same read-modify-write dance. The mechanisms genuinely
differ per tool (TOML for Codex, env vars for Hermes), so a full framework
is not warranted - but one `integrations/json_config.rs` with the
load/atomic-write/ensure-object trio would remove two copies and make the
0o600/atomic-write/empty-file conventions a single decision instead of three.

### M4. App.tsx hand-rolls the same post-mutation resync three times, with three different fallbacks

- `src/App.tsx:789` (`toggleProxy`): `setTools(await listTools().catch(() => []))`
- `src/App.tsx:876-887` (`setToolRouted` finally): `listTools().catch(() => tools)`
  plus a try/catch `proxyStatus` re-read
- `src/App.tsx:963-981` (`setGroupRouted`): the same pair again, again with
  `catch(() => tools)`

The `() => []` vs `() => tools` divergence means a transient `listTools`
failure after a master toggle blanks the ledger, while the same failure
after a tool toggle keeps the stale list - two behaviors nobody chose. A
single `resyncLedger()` helper (and arguably a `useRoutingController` hook
owning `proxy`/`tools`/`proxyBusy` and the five mutation callbacks) would
remove the divergence and take ~300 lines out of a 1480-line component whose
size is otherwise mostly justified by orchestration comments. Related
small state smell: `Home.tsx:193-204` fetches `launchAtLoginStatus` itself
while everything else it renders arrives via props, so the screen has two
data sources with different staleness.

### M5. Module-level test gaps that matter

Rust: beyond H3 (ProxyManager), `ca_windows.rs` (615 lines, includes the
`certutil_bounded` hang fix from `bedf13a`), `helper.rs` / `helper_client.rs`
(the Linux daemon protocol), and `config.rs` have no `mod tests` and are not
driven by `crates/core/tests/` directly - they are exercised only through
whichever platform CI happens to run.

Frontend: `src/lib/analytics.ts` (182 lines - event gating, error-title
scrubbing before anything goes over the wire) has no unit test even though
`errors.test.ts` next to it tests the classification it depends on;
`FirstRun.tsx` (275 lines) and `OrgPicker.tsx` (193 lines) have no unit
tests (partially covered by `e2e/signin.spec.ts`); `App.tsx` itself is only
covered end-to-end. The screens that do have tests are covered thoroughly
(`Home.test.tsx` is 822 lines), which makes the untested sign-in path the
odd one out. The analytics scrubbing is the highest-value gap: it is a
privacy promise ("only the classified title is sent", api.ts:334-337) with
no test pinning it.

## Low

### L1. Stale-closure dependency in toggleProxy

`src/App.tsx:799` reads `account?.auth_mode` inside `toggleProxy` to pick
the 401 remedy wording, but the `useCallback` deps at line 810 are
`[proxy, proxyBusy, ensureCaTrusted]` - `account` is missing. After an
auth-mode change (key user upgrades to OAuth in the same session), a failed
toggle can classify with the old mode and tell an OAuth user to replace an
API key Settings no longer shows. `setGroupRouted` (line 983) includes
`account` correctly, which is what makes the omission look accidental rather
than chosen.

### L2. Logging in core is eprintln-only and inconsistent with the shell's error seam

`crates/core` diagnostics go to stderr (`engine.rs` 11 sites, `manager.rs`
10, `manager_windows.rs` 9, plus scattered others), with inconsistent
prefixes (`[gate-proxy]`, `gate proxy:`, none). `engine::init_tracing`
configures `tracing` for hudsucker's output but the crate's own messages
bypass it. For a Finder-launched .app stderr is lost, so only failures the
shell explicitly routes through `report_backend_error`
(`src-tauri/src/lib.rs:998`) ever become visible; the exit-time proxy-revert
failure at lib.rs:2196-2198, for example, is eprintln-only and vanishes.
Not urgent (the important paths are covered), but worth one decision: either
a `log_warn!` macro with a uniform prefix in core, or routing core warnings
through the same buffered seam.

### L3. Comment style narrates repository history

Many comments describe previous states of the code rather than the current
one: `src/App.tsx:349-364` ("this comment used to claim a status poll that
had never existed"), `App.tsx:445-451` ("This used to also test
hasSeenTour()..."), `Home.tsx:158-166` ("This comment used to say..."),
and similar in `proxy/mod.rs` catalog entries. The rationale-comment culture
here is a strength overall, but the "used to" strata are write-only: they
answer questions only the original author had, they cannot be verified
against the code, and they push files toward the 1000-line marks flagged in
this review (`proxy/mod.rs` is 1783 lines of which roughly 470 are comments
in the first 1000). Git history already stores the archaeology; comments
that survive an edit should describe the invariant, not the diff.

### L4. File sizes - mostly justified, two real seams

For the record on the four flagged files: `proxy/mod.rs` (1783) is ~530
lines of tests plus a comment-heavy catalog; moving `default_domains()` and
its essays into a `catalog.rs` is the one cheap split. `engine.rs` (1282) is
fine except `start()` at lines 838-1063, which builds CA, three listeners,
the PAC server, and the runtime in one 225-line function - splittable but
cohesive. `Home.tsx` (972) and `App.tsx` (1480) are addressed by M4; beyond
that their size is comments and JSX, not hidden structure. `lib.rs` (2821)
is addressed by H1/H2; the remaining ~900 lines of macOS objc window/tray
glue (lines 2220-2821) are legitimately shell code, though a
`src-tauri/src/macos.rs` would make the command surface readable on its own.

## Positive notes (things worth keeping as-is)

- `inject_gate_credential` (`proxy/mod.rs:418`) and `resolve_endpoint`
  (`proxy/mod.rs:985`) are exactly the right "decide once, share everywhere"
  helpers; the catalog invariant tests around them are the best tests in the
  repo.
- Frontend error handling is unusually consistent: everything user-visible
  funnels through `classifyError`, backend failures reach the UI via the
  buffered `drain_backend_errors` seam, and `TrustDeclined` as a
  silent-abort sentinel is a clean pattern.
- The 17-file `crates/core/tests/` suite covers the engine, relay, OAuth,
  keychain chunking, and disconnect-residue behavior end to end; the gaps
  named in M5/H3 are the exceptions, not the rule.
