# Code review: `fix/browser-trust-readings` (PR #243)

Lens: **code quality**. Base `origin/feat/new-app-ui` (merge-base `5e2fee30`), 3 commits, 22 files.
Verified locally: `cargo check --manifest-path crates/core/Cargo.toml --all-targets` exits 0 with no
warnings, `npx tsc --noEmit` is clean, and the seven touched frontend test files pass under vitest.

## Summary

This is a well-shaped branch. The central move - replacing a boolean `ca_nss_trusted` with a
three-state `NssTrust` plus a per-store `NssRefusal` list, and serving it from what the write
recorded rather than from a probe on the polled path - is the right model, and it is carried
consistently through Rust, the wire types, the copy and the report. The `degrade` split is a good
piece of design: a pure function with the precedence argument in its doc comment and three tests
that pin it. The `#[cfg(test)]` `CERTUTIL_OVERRIDE` seam is a genuinely better answer than the
`PATH` mutation it replaced, and the commit message explains why. What I found is mostly a gap
between what comments and commit messages claim and what the code does: `output_bounded`'s doc
announces a consolidation that only happened for one of the three copies, its `POLL` constant
documents a value one of those copies does not use, and a test fixture comment promises a test that
does not exist. Two behavioural nits are worth a look before merge: the 50ms poll granularity lands
on a six-call `gsettings` path on enable, and `browser_proxy_channel`'s `OnceLock` latches a
transient probe failure for the life of the process.

## Findings

### High

None. Nothing here is a correctness or shipping blocker from a code-quality standpoint.

### Medium

**M1. `browser_proxy_channel`'s `OnceLock` cannot tell "no GNOME schema" from "the probe failed
once".**
`crates/core/src/proxy/system_proxy_linux.rs:318-323` caches the result for the process lifetime.
The doc justifies the cache on the grounds that the answer "cannot change under us - a desktop
session does not gain the schema while its apps are running", which is true of the *schema* but not
of the *reading*: `gsettings_get` (line 273) returns `None` for a spawn failure, a non-zero exit, or
the new 1s timeout, and all three are folded into the same `false`. A GNOME user whose first
`status()` at boot races a cold `gsettings` (which links GLib and reads the schema cache) past one
second gets `browser_proxy_channel: false` permanently, and `browserScopeNote` silently withholds
the browser sentence for the rest of the session with no way to recover short of a restart.
Suggest caching only a positive reading, or distinguishing "probe failed" from "answered no" before
latching.

**M2. `output_bounded`'s doc claims a consolidation that did not happen, and its `cfg` prevents it.**
`crates/core/src/primitives.rs:32-34` says "Three private copies of this loop had grown before it:
`ca_windows`, `ca_linux`, and `integrations::binaries`' version probe. This is that shape once, so
the next caller is not a fourth." Only `ca_linux` was converted. `ca_windows::certutil_bounded`
(`crates/core/src/proxy/ca_windows.rs:89-107`) and `integrations::binaries::binary_version`
(`crates/core/src/integrations/binaries.rs:147-164`) still hand-roll the loop, so the count went
from three to three, not to one. Worse, `primitives.rs:44` gates the helper on
`any(target_os = "macos", target_os = "linux")`, so `ca_windows` structurally *cannot* adopt it, and
its own doc makes a deliberate case against piping and reaping on Windows. Either soften the claim
to "this is the Unix shape once, and Windows keeps its own for the reasons `certutil_bounded`
documents", or actually convert `binaries.rs` (which is the one that shares the platform gate).
Related: the helper has zero macOS call sites today - the only two are `ca_linux.rs:441` and
`system_proxy_linux.rs:285` - so the `macos` half of the `cfg` is currently speculative.

**M3. The 50ms poll granularity is paid six times on the enable path.**
`primitives.rs:50` fixes `POLL` at 50ms and the loop sleeps a full tick before re-checking
(`primitives.rs:57-68`). `gsettings_capture` (`system_proxy_linux.rs:330-343`) calls `gsettings_get`
once per key across the six `GNOME_KEYS`, sequentially, on `enable`. Where the old `.output()` returned
as soon as the child exited, each call now rounds up to the next 50ms boundary, so up to ~300ms of
added latency on a path the user is waiting on a switch for. `integrations::binaries` polls at 10ms
for exactly this reason. A shorter first tick, or a short-then-back-off schedule, costs nothing and
removes the regression.

**M4. The `POLL` doc comment is factually wrong.**
`primitives.rs:49` reads "The value all three copies used." `ca_windows.rs:73` does use 50ms, but
`binaries.rs:159` sleeps 10ms. Since M2 leaves both of those copies in the tree, the claim is
checkable and false, and it is the sentence that would justify not revisiting the value in M3.

**M5. A test fixture comment promises a test that does not exist, and the `false` path has no
component coverage.**
`src/screens/GroupMembers.test.tsx:97-99` says "The GNOME case, so the chat rows' browser sentence is
exercised; the false case has its own test below." There is no such test: all four occurrences of the
prop in that file are `browserChannel={true}` (lines 99, 284, 370, 416), and `FamilyPanel.test.tsx:132`
is `true` as well. So `explain`'s new `browserChannel` parameter
(`src/screens/GroupMembers.tsx:64,109`) is never rendered in its `false` state. The unit-level
behaviour is covered in `platform.test.ts` and `groups.test.ts`, so this is a coverage gap rather than
a hole, but the comment should either be removed or the test written.

**M6. The report cannot distinguish "no write recorded" from "the write was clean".**
`src/lib/diagnosticsReport.ts:269-284` emits the `browser write` row only when
`ca_nss_write` is non-null *and* the outcome is not `trusted`. Null and `trusted` both print nothing,
so a report taken from a process that never wrote reads exactly like a report from a clean write.
That is the shape principle 6 in CLAUDE.md warns about ("a section that was never read says so in
words"), and it bites in the specific case the feature exists for: on Linux the engine outlives the
GUI, so a user who restarts the window with routing already on has `recorded_nss_trust() == None`
and a report that quietly implies everything is fine. One line - `browser write   not attempted this
session` - closes it. Note the probed `ca_nss_trusted` line above still fires, which limits the
damage but does not remove the ambiguity.

**M7. `gsettings set` on the enable path is still unbounded.**
`system_proxy_linux.rs:175-187` (`run_best_effort`) uses a plain `.output()`, and it is called with
`gsettings set` at lines 376, 395 and 404, i.e. once per key on enable and again on disable. The
argument the branch wrote for bounding `gsettings_get` (`system_proxy_linux.rs:276-284`: a dbus peer
that does not answer blocks in the call, and this runs where a user is waiting on a switch) applies
verbatim to these writes against the same peer. The commit title scopes itself to "the last
unbounded *probe*", which is accurate, but leaving the writes unbounded means the hang class is only
half closed.

### Low

**L1. `output_bounded` leaks the child when `try_wait` errors.**
`primitives.rs:58` propagates the error with `?`, so the child is neither killed nor reaped - which
is precisely the zombie the doc comment at lines 25-30 makes a point of avoiding on the timeout
path. Rare, but the asymmetry is worth a `kill`/`wait` before returning.

**L2. Magic duration at the `gsettings` call site.**
`system_proxy_linux.rs:285` inlines `std::time::Duration::from_secs(1)` where `ca_linux.rs:289`
gives its budget a named const with a doc comment (`CERTUTIL_TIMEOUT`). The rationale for the 1s is
in a comment three lines above, so nothing is lost in understanding, but the two call sites read
inconsistently and the value cannot be referenced from a message or a test the way
`CERTUTIL_TIMEOUT` is at `ca_linux.rs:447,771`.

**L3. `diagnostics.rs` interleaves two `cfg` pairs.**
The new `ca_nss_write` functions (`crates/core/src/diagnostics.rs:118-128`) are inserted between the
Linux and non-Linux halves of `ca_nss_trusted` (lines 113-116 and 130-135), so the explanatory
comment on the non-Linux `ca_nss_trusted` now sits three functions away from its twin. Commit
864747cb complains about exactly this pattern ("Two new consts had landed between `NSS_TOOLS_HINT`'s
doc comment and `NSS_TOOLS_HINT`"), so the standard is the branch's own. Also, the non-Linux
`ca_nss_write` (line 125) has no doc comment where its Linux counterpart does.

**L4. A multi-line `reason` will break the report's column layout.**
`ca_linux.rs:454-459` builds `Failed` from `String::from_utf8_lossy(&out.stderr).trim()`, and
certutil's stderr is frequently two lines. That string reaches `NssRefusal.reason` unchanged
(`ca_linux.rs:618`) and is interpolated into a single `row(...)` at
`src/lib/diagnosticsReport.ts:282`, where `row` is `label.padEnd(LABEL_WIDTH) + value`
(`diagnosticsReport.ts:93-95`). Any embedded newline emits an unaligned continuation line into a
fixed-column report. Collapsing whitespace in `reason` at the Rust boundary is a one-liner.
(Printing the store path itself is fine and consistent: the report already prints `data dir` at
`diagnosticsReport.ts:220`.)

**L5. The `write_failed` title and body disagree on the count.**
`src/lib/groups.ts:250-251`: the title is "One browser certificate store refused the certificate"
while the body correctly says "at least one of the separate stores". The refusals list is a `Vec`
and the report prints one line per entry, so the title is the odd one out.

**L6. The wire-word test pins renames but not additions.**
`crates/core/src/proxy/mod.rs:1971-1983` checks the three serialised strings, and its doc explains
that a renamed variant would compile on both sides and stop matching. A *new* variant has the same
failure mode - `browserTrustRestartAdvice` (`src/lib/groups.ts:242-257`) falls through to the reopen
note for anything it does not recognise - and the test would still pass. An exhaustive `match` over
`NssTrust` inside the test (so adding a variant fails to compile) would cover both.

**L7. `explain` is up to four positional parameters.**
`src/screens/GroupMembers.tsx:60-65` now takes `(member, platform, group, browserChannel)`. Two
booleans/strings away from being easy to transpose at the single call site
(`GroupMembers.tsx:658`). An options object is the usual answer once a helper reaches this width.

**L8. Two timeout assertions are loose enough to pass a broken deadline.**
`primitives.rs:429-437` sets a 300ms deadline against a 30s child and asserts only `waited < 10s`;
`ca_linux.rs:770-773` asserts `elapsed < CERTUTIL_TIMEOUT * 3`, i.e. 15s for a 5s budget. Both would
catch a fully unbounded regression and neither would catch a deadline off by an order of magnitude.
The comments acknowledge the looseness deliberately ("not a claim about scheduler precision on a
loaded runner"), so this is a judgement call rather than a defect - but 2x rather than 33x would
still be safe on a loaded runner.

## What's good

- **The model change is the right one, and it is complete.** `NssTrust` / `NssRefusal` / `NssReading`
  are threaded end to end - Rust struct, serde words, hand-written TS union, copy, report - with no
  call site left on the old boolean, and `tsc` and `cargo check` both confirm it. The
  `ProxyState`-takes-the-outcome / `Diagnostics`-takes-both split
  (`manager_linux.rs:136`, `diagnostics.rs:104`) is a real distinction with a real reason behind it,
  and both doc comments say which is which.
- **`degrade` is the model finding well handled**: pure, split out so the precedence is testable
  without a database to fail, with three tests including both orderings
  (`ca_linux.rs:550-555`, tests at 948-985) and a fourth that pins the same precedence through the
  real caller via the shim (`ca_linux.rs:784-790`).
- **The `CERTUTIL_OVERRIDE` seam is the right call and the commit says why.** `#[cfg(test)]` rather
  than an env var, `Mutex`-guarded, with a `Drop` that restores and a `SHIM_LOCK` that serialises the
  three tests. I checked: no other test in the crate reaches `certutil_output`, so the shim's own
  lock is sufficient.
- **The reap test earns its complexity.** Having the child write its own pid rather than counting
  process-wide zombies is the correct fix for the parallel-suite failure the commit describes, and
  the "state is the field after the last `)`" detail in `/proc/pid/stat` is right.
- **The `PaneNote` reordering at `src/NewUiApp.tsx:3061-3070`** is a genuine fix with a correct
  premise: I confirmed a chat member is built as `{...memberFromDomain(...), chat: true}` with
  `kind: "proxy"` (`src/lib/groups.ts:484,556`), so on Linux both notes do draw and the old order did
  read as a contradiction.
- **`groups.ts:218-224` is accurate about the mechanism it describes.** `manager_linux::enable` does
  return early at lines 171-174, before `ca::ensure_trusted`, so "turn routing off and on again" is
  the reachable instruction and "switch routing on again" was not. The test at
  `groups.test.ts` that pins this is a good way to keep the copy honest.
- The `record_nss_trust(None)` calls on the no-stores, unreadable-cert and untrust paths
  (`ca_linux.rs:571,587,657`) each carry a one-line reason for why absence is the right answer there
  rather than a stale verdict. That is the part of principle 6 easiest to get wrong and it is right
  here.
