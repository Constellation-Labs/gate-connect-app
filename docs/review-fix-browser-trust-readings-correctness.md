# Review: `fix/browser-trust-readings` (PR #243) - correctness lens

Base `origin/feat/new-app-ui`, merge-base `5e2fee30`, three commits (`28664a17`, `864747cb`, `4e39b03c`).

## Summary

The new tri-state (`NssTrust` / `NssReading`) is modelled correctly, serialises under the words the
TypeScript union spells, and reaches `groups.ts` without being flattened anywhere on the copy path:
`null` genuinely falls through to the reopen note rather than to a negative reading, and nothing new
drives a switch from observed state. `cargo check -p gate-connect-core --tests` and `tsc --noEmit`
are both clean, and the `platform.ts` signature change is accounted for at every call site. The
problems are all at the *edges* of the reading rather than in its middle. One is a hard principle-6
violation the branch newly widens: `nss_ca_trusted()`, the probe that still feeds the diagnostics
report, turns "certutil could not answer" - now including "certutil was killed at the 5s deadline
this branch added" - into `Some(false)`, which the report prints as the flat assertion
`browser store   CA MISSING (chromium)`. The rest are staleness holes opened by moving from a probe
to a recording: `browser_proxy_channel()` caches an inconclusive `gsettings` probe as `false` for
the life of the process, the recording is process-scoped in a product that ships a CLI which writes
the same store, and nothing retires the note after the user does what the note told them to do.

---

## H (must-fix)

### H1. The report's NSS line still flattens "could not read" into "not trusted", and this branch adds a new way to reach it

`crates/core/src/proxy/ca_linux.rs:472` (`nss_entry_pem`), `:479` (`nss_holds`), `:493`
(`nss_ca_trusted`), consumed at `crates/core/src/diagnostics.rs:115` and printed at
`src/lib/diagnosticsReport.ts:260`.

`nss_entry_pem` is `certutil_output(...).ok()`, so every `CertutilFailure` collapses to `None`;
`nss_holds` then returns `false`; `nss_ca_trusted` is `Some(dirs.iter().all(...))`, so one
unreadable store makes the whole machine `Some(false)`, and `diagnosticsReport.ts:260` prints
`browser store   CA MISSING (chromium)`. That is a positive claim about the user's browser store
manufactured from the absence of a reading - exactly what principle 6 forbids, and the sentence a
support engineer will act on.

What this branch changes is the set of inputs that land there. Before `28664a17`, `certutil_output`
used `Command::output()`: a locked database or one on a stalled mount hung the diagnostics
collection, which is bad but is not a false statement. After the bounded call at
`crates/core/src/proxy/ca_linux.rs:441`, the same database returns
`CertutilFailure::Failed("certutil -L ... did not finish within 5s and was killed")` and the report
asserts the CA is missing from a store it never managed to open.

Concrete: CA trusted, `ensure_trusted_nss` wrote `~/.pki/nssdb` successfully this session
(`recorded_nss_trust() == Trusted`), then the database becomes unreadable (a `sql:` lock held by
another NSS client, a permissions change, an autofs mount that stalls). User copies a report. The
report says `certificate  trusted`, then `browser store   CA MISSING (chromium)`, and prints **no**
`browser write` line at all, because `diagnosticsReport.ts:269` is deliberately silent on `trusted`.
The one line that could have contradicted the false one is the one suppressed.

The new `NssReading` type already models the three answers. The fix is to give the probe the same
shape - have `nss_holds`/`nss_ca_trusted` distinguish "asked and it does not hold it" from "could
not ask", and print `browser store   unreadable` (or nothing) for the second - rather than leaving
the honest type next to a boolean that lies.

---

## M (should-fix)

### M2. `browser_proxy_channel()` caches an inconclusive probe as a negative reading, forever

`crates/core/src/proxy/system_proxy_linux.rs:318`, feeding
`crates/core/src/proxy/manager_linux.rs:137` and `src/lib/platform.ts:114`.

```
!session_effects_suppressed() && gsettings_get("org.gnome.system.proxy", "mode").is_some()
```

`gsettings_get` (`system_proxy_linux.rs:285`) is `output_bounded(cmd, 1s).ok()??`, so `None` covers
four different things: the binary is absent (a real `false`), the schema is absent (a real `false`),
the call was killed at the 1s deadline, and the call exited non-zero. The last two are non-answers,
and the `OnceLock` freezes whichever one happened first for the process lifetime.

Concrete: GNOME session, Gate autostarted at login, first `status()` runs while `dconf-service` is
still coming up or the machine is loaded enough that `gsettings get` takes over a second. The
`OnceLock` stores `false`. For the rest of that app's life, every chat pane on a desktop where Gate
*does* write GNOME's proxy keys omits "That includes the same site in a browser that follows your
desktop proxy settings" (`src/lib/platform.ts:120`, `src/screens/GroupMembers.tsx:109`). There is no
invalidation path; quitting and relaunching the app is the only fix.

The doc's argument for the safe direction is about the *value*, not about caching a non-answer. Cache
only conclusive outcomes: `Ok(None)` from `output_bounded` and a non-`NotFound` spawn error should
leave the cell unset so the next `status()` retries. Distinguishing those needs `gsettings_get` to
stop folding `Err` and `Ok(None)` together with `.ok()??`.

### M3. `ca_nss_trust` is process-scoped, so the CLI can flip `ca_trusted` under a running GUI and get the wrong note

`crates/core/src/proxy/mod.rs:1344-1350` claims the `None` case "is never the answer where it
matters: every path that raises the certificate note runs `ensure_trusted` first." Two reachable
paths break that, both because `RECORDED_NSS_TRUST` (`ca_linux.rs:523`) is a static in whichever
process did the write.

1. `gate-connect proxy trust-ca` (`crates/cli/src/main.rs:713` -> `manager_linux.rs:554`) runs
   `ca::ensure_trusted()` in the *CLI* process. The recording lands there and dies with it. The GUI,
   polling `status()`, sees `ca_trusted` go false -> true with `ca_nss_trust: null`
   (`manager_linux.rs:136`), and `NewUiApp.tsx:2166` raises the fall-through note: "Browsers already
   open need reopening". If the CLI's own `ensure_trusted_nss` hit `ToolsMissing`, the GUI has just
   told the user to reopen their browser on a machine where reopening cannot work - the precise loop
   `browserTrustRestartAdvice`'s doc says it exists to avoid.
2. `gate-connect proxy trust-ca --system-trust` (`manager_linux.rs:564` -> `ca::ensure_trusted_system`,
   `ca_linux.rs:224`) never calls `ensure_trusted_nss` **at all**, in any process. Same GUI symptom,
   and here no NSS store was written by anyone.

The old code probed (`ca::nss_ca_trusted()`), so it got this right. Moving to a recording is the
right call for the polled path; the doc invariant it rests on is not true, and either the invariant
or the fall-through needs to change (e.g. probe once, lazily, on the false -> true transition
specifically, rather than on every poll).

### M4. Nothing retires the failure note when the user does what it asks

`src/NewUiApp.tsx:2158-2168`.

The note is latched on `ca_trusted` false -> true and cleared by the user alone. The `tools_missing`
copy tells the user to install a package "then turn routing off and on again". Doing that does
re-run the write (`disable_inner` drops the client handle, so `enable`'s early return at
`manager_linux.rs:171` does not fire, and `ensure_trusted` calls `ensure_trusted_nss` outside its
`is_trusted` short-circuit) and `ca_nss_trust` correctly becomes `"trusted"` in the next status. But
`ca_trusted` never went false during any of it, so the effect never re-runs and the note keeps
saying "Chromium-based browsers can't see the certificate" over a machine where they now can.

The reading the UI needs is already on the wire and already changes. Watching
`ca_nss_trust` transition away from a failure variant - and clearing `browserRestart` when it does -
is a two-line change and turns a dismissible-but-wrong claim into a confirmation.

### M5. `output_bounded` does not null stdin, contrary to its own doc, and that silently changed `gsettings_get`

`crates/core/src/primitives.rs:45-56`.

The doc says "Everything else is the ordinary `std::process::Command::output` contract."
`Command::output()` sets stdin to `Stdio::null()`; `output_bounded` sets only stdout and stderr, so
the child inherits the parent's stdin. `ca_linux.rs:433` sets `.stdin(Stdio::null())` itself and has
a comment explaining why a certutil that inherits a terminal blocks on its password prompt - but
`system_proxy_linux.rs:283` does not, and it was previously calling `.output()`. So `gsettings get`
run from `gate-connect proxy status` in a terminal now inherits that terminal.

`gsettings` does not read stdin, so nothing is observably broken today. The defect is that the
helper's contract is documented as one thing and implemented as another, on the exact hazard one of
its two callers wrote a comment about. Set `.stdin(Stdio::null())` inside `output_bounded` (a caller
that wants stdin can override it after) or correct the doc.

### M6. The recording state machine has no Rust test

`crates/core/src/proxy/ca_linux.rs:565-638`. The tests added in `4e39b03c` cover `degrade`'s
precedence as a pure function (`:952`, `:974`), `certutil_output`'s three failure shapes (`:765`,
`:796`, `:813`), and `output_bounded` itself. Nothing exercises `ensure_trusted_nss`, which is where
every one of those pieces is assembled and which is the sole producer of the value the whole UI
switches on. Specifically untested:

- no NSS dirs -> `record_nss_trust(None)` (`:571`), including the "browser installed then removed
  between two enables" clearing the comment calls out;
- unreadable cert -> `record_nss_trust(None)` (`:589`), which must not read as a store verdict;
- one store refusing -> `WriteFailed` with exactly one refusal carrying that store's path;
- `ToolsMissing` reached in situ (not just through `degrade`), with refusals staying empty;
- the steady state where every store already holds the CA -> `Trusted` with no certutil write.

The `CERTUTIL_OVERRIDE` seam at `:392` plus a `TmpDir` HOME makes all five reachable; the existing
fake-certutil harness at `:724` is already most of the way there.

### M7. `groups.test.ts:783` is vacuous for half of what it loops over

```
if (body.includes("routing")) { expect(body).toContain("turn routing off and on again"); }
```

The `write_failed` body (`src/lib/groups.ts:250`) contains no "routing", so for that half of the
loop the only surviving assertion is `not.toContain("switch routing on again")`, which any string
without those words passes. The test's name promises coverage it does not give.

That guard is papering over a real gap rather than a stylistic one: `write_failed` prescribes no
retry at all. It tells the user the report names the store and why, and then - once they have
unlocked it or fixed the permission - gives them nothing to do. The same "turn routing off and on
again" that `tools_missing` gets is the action that re-runs the write, and it belongs in both.

---

## L (nits)

- **L1.** `crates/core/src/primitives.rs:29-33`: the doc says three private copies "had grown before
  it. This is that shape once, so the next caller is not a fourth." Only the `ca_linux` copy was
  actually replaced. `crates/core/src/proxy/ca_windows.rs:89` (`certutil_bounded`) and
  `crates/core/src/integrations/binaries.rs:150` still hold theirs, so `output_bounded` is currently
  a fourth copy rather than a consolidation. The doc's follow-on claim that "the two in `ca_*` did
  not [reap]" is also wrong about `ca_windows`, whose comment at `:85-88` explains that it
  deliberately does not wait because reaping is a Unix concern - correct on Windows.
- **L2.** `crates/core/src/proxy/mod.rs:1310` documents `refusals` as "Empty for every outcome but
  `WriteFailed`". `ensure_trusted_nss` can violate it: a store that fails with `Failed` pushes a
  refusal (`ca_linux.rs:616`), and a *later* store failing with `Missing` promotes the outcome to
  `ToolsMissing` (`:550`) without clearing the vec. `src/lib/diagnosticsReport.ts:279` prints
  refusals for any non-`trusted` outcome, so the report would say `FAILED - certutil not installed`
  and then list a store that refused. Contrived (certutil would have to disappear mid-loop) but the
  contract is stated and neither side enforces it; `diagnosticsReport.test.ts:296`'s
  `not.toContain("refused")` does not cover it.
- **L3.** `src/screens/GroupMembers.test.tsx:97`: "the false case has its own test below" - there is
  no such test. All four render sites (`:99`, `:284`, `:370`, `:416`) pass `browserChannel={true}`.
  The behaviour is covered in `platform.test.ts:141`, but nothing pins that `GroupMembers` actually
  threads the prop through `explain` (`GroupMembers.tsx:64`, `:109`, `:655`), which is the wiring
  this commit added.
- **L4.** `crates/core/src/primitives.rs:49`: the 50ms poll gap is checked after the first
  `try_wait`, which for a freshly spawned child is essentially always `None`, so every bounded call
  costs ~50ms of pure sleep. On the enable path that is 2-3 certutil calls per existing NSS dir plus
  the `gsettings` probe. With one store it is negligible; the nine-candidate list at `:318` makes the
  worst case around a second of added latency on a user-visible switch.
- **L5.** `crates/core/src/proxy/system_proxy_linux.rs:320` probes whether
  `org.gnome.system.proxy mode` is *readable*, but the claim the copy makes depends on
  `gsettings_apply` (`:360`) being able to *write* it, and that is `run_best_effort` - it logs and
  continues on failure. A session where the schema reads but the writes are refused gets
  `browser_proxy_channel: true` and a chat row claiming browser coverage that is not happening,
  which is the one error the field's doc says it exists to prevent.
- **L6.** `crates/core/src/primitives.rs:429-438`: the hang test uses a 300ms deadline and asserts
  the wait was `< 10s`. A regression that made the deadline 30x too long would pass. The child's
  `sleep 30` bounds it, so a tighter bound (say 5s) still leaves ample slack on a loaded runner.
- **L7.** `output_bounded` kills the direct child only, not its process group. `sh -c 'echo $$ > f;
  sleep 30'` in `a_killed_command_is_reaped` (`primitives.rs:451`) leaves an orphaned `sleep` behind
  for 30s after the shell is reaped. Harmless for `certutil` and `gsettings`, which do not fork, but
  the helper is presented as general.
- **L8.** `src/App.tsx:1285-1288`: the comment says `?? false` avoids "a claim that Gate is
  intercepting a browser" on an unresolved proxy state. `browserScopeNote` ignores the flag entirely
  on `macos`/`windows` (`platform.ts:116-118`), so on those platforms the sentence is made whether or
  not `proxy` has resolved. The guard is only load-bearing on Linux; the comment reads as if it were
  general.

---

## Traced and correct

- **`ProxyState` construction is exhaustive.** Only two sites exist
  (`manager_core.rs:173`, `manager_linux.rs:121`), both set the two new fields, and the macOS/Windows
  `browser_proxy_channel: true` is right: the PAC goes into the OS setting a browser reads.
- **No status/enable race on the pair `(ca_trusted, ca_nss_trust)`.** `status()` takes
  `self.client.lock()` at `manager_linux.rs:70` and `enable()` holds the same mutex across
  `ca::ensure_trusted()` (`:165-188`), so a concurrent poll cannot observe the anchor installed with
  the NSS write still in flight. The two fields in one snapshot are always from the same completed
  write. (`status` reading them after `drop(guard)` looked like a hole and is not.)
- **Wire words.** `NssTrust` serialises `trusted` / `tools_missing` / `write_failed`
  (`mod.rs:1978-1983`), matching both hand-written TS unions (`api.ts:315`, `api.ts:998`), and
  `NssRefusal`'s field names are pinned (`mod.rs:1990`). `#[serde(default)]` on both new
  `ProxyState` fields gives `None` / `false`, the safe directions.
- **`degrade` precedence.** `ca_linux.rs:550` is correct in all four combinations: `Missing` always
  wins, `ToolsMissing` is never demoted to `WriteFailed`, and `Trusted + Failed` -> `WriteFailed`.
  Tests at `:952` and `:974` cover the ordering both ways.
- **`ensure_trusted_nss` reads the cert before touching any store** (`:583`), so an unreadable cert
  cannot leave a store with the old entry deleted and nothing added, and it records `None` rather
  than a verdict (`:589`). The `nss_holds` skip at `:600` keeps the destructive delete/add window out
  of the steady state.
- **`untrust_nss` clears the recording to `None`** (`:656`) rather than leaving a `WriteFailed`
  standing, so a deliberate removal is not reported as a fault.
- **Observed state vs intent (principle 2) is untouched.** `browserChannel` and `ca_nss_trust` reach
  only copy: `explain()`'s `<p>` (`GroupMembers.tsx:655`), `chatScopeNote`/`browserScopeNote`, and
  the two `PaneNote`s in `NewUiApp.tsx:3070-3078`. No switch's checked state, disabled state, or
  handler reads either.
- **`null` is not flattened on the copy path.** `browserTrustRestartAdvice` (`groups.ts:233`) falls
  through to the reopen note for both `"trusted"` and `null`, and `groups.test.ts:796` pins that they
  produce the same title. `browserScopeNote` returns `""` for `unknown` and for
  `linux + !browserChannel` (`platform.ts:114`), so the absent sentence is absent rather than guessed.
- **The `write_failed` promise is answerable.** `diagnostics::collect()` is invoked from
  `src-tauri/src/lib.rs:1066`, i.e. the same process that ran `ensure_trusted`, so
  `recorded_nss_trust()` is populated when the note that points at the report is on screen.
  `diagnosticsReport.ts:270-283` prints the store and certutil's own words, one line per refusal.
- **"Off and on again" really does reach the write.** `enable`'s early return
  (`manager_linux.rs:171`) fires only while `self.client` is `Some`; `disable_inner` (`:303`) clears
  it. `ensure_trusted` (`ca_linux.rs:196`) calls `ensure_trusted_nss` outside the `is_trusted`
  short-circuit, with a comment saying why. The instruction in the `tools_missing` copy is reachable.
- **`platform.ts` rename is complete.** Both `browserScopeNote` call sites (`groups.ts:292`,
  `GroupMembers.tsx:109`), both `chatScopeNote` call sites, the `browserChannel` prop chain
  (`App.tsx:1288` -> `FamilyPanel.tsx:46,62,152` -> `GroupMembers.tsx:230,248`), and every test
  fixture were updated; `npx tsc --noEmit` and `cargo check -p gate-connect-core --tests` are both
  clean.
- **`pem_body`** (`ca_linux.rs:506`) strips armour and all whitespace, so an NSS export and the
  on-disk PEM compare equal across line wrapping and line endings; `:931` and `:941` pin both
  directions.
- **NSS candidate paths** (`:318`) are enumerated rather than globbed, all under the given `HOME`
  (`:919`), lead with the unconfined database (`:884`), and `nss_db_dirs` filters to those that
  exist - so "no Chromium ever ran" is an empty list and a `None` reading, not a `false`.
