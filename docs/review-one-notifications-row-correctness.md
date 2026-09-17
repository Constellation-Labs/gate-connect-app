# Correctness review: `fix/one-notifications-row` (c446f3f)

Base `origin/feat/new-app-ui`, worktree `/home/gabriel/coding/constellation/alt/gate-connect-alerts`.
Lens: correctness only (logic, edge cases, nullability, caller contracts, schema/migration safety).

**Verdict: the collapse is correct.** The `admit`/`sweep` rewrite is behaviour-preserving
for the semantics that survive the collapse, the rename is complete (no dangling symbol
anywhere in the repo), both `invoke_handler` lists were edited consistently, and the
frontend toggle keeps its optimistic-then-re-read shape. Two findings are worth acting on,
both about what is *not* in the diff: the absence of any migration for an existing
`preferences.json`, and a notification path that the new doc comment claims to gate and
does not.

Verified green in this worktree:

- `cargo test -p gate-connect-core --lib preferences` (13 passed),
  `... --lib security_feed` (49 passed)
- `cargo check --lib` in `src-tauri` (exit 0), `cargo fmt --check` clean in both crates
- `npx tsc --noEmit` and `npx tsc -p e2e --noEmit` clean
- `npx vitest run` on `SettingsPane.test.tsx`, `analytics.test.ts`,
  `analytics.privacy.test.ts` (58 passed)
- `npx playwright test` on `new-ui-settings.spec.ts` + `new-ui-security-feed.spec.ts`
  (30 passed) and `new-ui-firstrun.spec.ts` (24 passed)

---

## Findings

### M1. An existing `preferences.json` silently re-enables notifications on upgrade

`crates/core/src/preferences.rs:50-52` (the new `notifications` field, `#[serde(default =
"default_true")]`) and `crates/core/src/preferences.rs:319-328` (`load`).

The struct has no `deny_unknown_fields` and no `#[serde(alias = ...)]`, so a file written
by the current shipping build - `{"routing_health_notifications": false,
"blocked_event_notifications": false, "flagged_event_notifications": false, ...}` - parses
happily, ignores all three, and yields `notifications: true`. The next `save` (any
read-modify-write setter) drops the three keys from disk for good.

The user-visible consequence is the one the code's own comments argue against in three
places: a person who turned these off gets native notifications back, without being asked
and with the Settings row reading On. `notify.rs:95` says "A switch the user turned off has
to actually stop something, or it was never a switch"; for existing installs, the upgrade
turns it back on.

Nothing in the diff is wrong in isolation - collapsing three booleans into one necessarily
loses information - but there is no migration and no test pinning the legacy-file case.
`preferences.rs:657-666` (`a_missing_field_loads_as_on`) covers absent fields;
nothing covers *present but removed* ones.

If a migration is wanted, the cheap form is a single alias on the field that best
represents intent, e.g. `#[serde(alias = "routing_health_notifications")]`. Note the
hazard before reaching for more than one: serde rejects a struct that sees two aliases of
the same field in one object with a duplicate-field error, and `load` swallows a parse
error into `Preferences::default()` - so a three-alias version would make an old file fail
to parse entirely and would also discard `device_name`, `tool_models` and
`session_routing_accepted`. That failure mode is strictly worse than the one above. Either
one alias, or an explicit migration pass, or a deliberate "we accept the reset" note.

### M2. `set_notifications`'s doc claims to gate a notification that is not gated

`src-tauri/src/lib.rs:2345-2352` now reads "Gates everything the app can put on screen: a
request blocked or flagged by the security feed (AG-578), an expired session, a quit that
could not put a tool back on its own settings."

There are four notification call sites in the crate:

- `lib.rs:2427` (`fire_notification`, the security feed) - gated, via
  `notify_for_event` -> `Grouper::admit`/`sweep` reading `prefs.notifications`.
- `lib.rs:4346` (teardown on quit) - gated.
- `lib.rs:5300` (the 30s dead-session tick) - gated.
- `lib.rs:3526-3535` (`signal_session_dead`) - **not gated**. It fires the identical
  "Your session expired..." body with no preference read at all.

This is pre-existing: the base has the same ungated block at its `lib.rs:3543-3552`, and
the diff does not touch it. It matters now only because the new comment asserts a contract
the code does not keep, and because `signal_session_dead` and the gated tick are two
routes to the same message - `SESSION_NEEDS_SIGNIN.swap` at `lib.rs:3517` means whichever
fires first suppresses the other, so a user with the switch off can still get the
notification whenever the 401 observer / startup recheck wins the race. Either gate
`signal_session_dead` the same way or soften the comment; do not leave both.

### L3. Stale module doc in `preferences.rs`

`crates/core/src/preferences.rs:17-22` still reads "The per-category security-event
switches (blocked / flagged) and the sound toggle arrived with the live event feed they
gate (AG-578)". The per-category switches no longer exist. The paragraph's actual point
(a switch that gates nothing is worse than a missing switch) survives the collapse and is
worth keeping; only the naming is stale.

### L4. Stale sentence in the `sweep` doc

`crates/core/src/security_feed/notify.rs:148-151`: "a user who turns blocked notifications
off mid-storm". There is no blocked switch any more. The test named for this behaviour
(`a_switch_turned_off_mid_storm_silences_the_summary`, line 490) was updated; the doc was
not.

### L5. One untested edge the collapse makes reachable

With the switch off, `admit` returns at `notify.rs:94-98` *before* the bucket is created,
so an event that arrives while notifications are off does not consume the "first event in
a bucket speaks" slot: turning the switch back on lets the next event speak immediately.
That is the right behaviour and it is what the code does, but nothing asserts it. A
three-line addition to `a_switch_that_is_off_stops_something` (admit while off, then admit
the same event while on, expect a fire) would pin it. Nit, not a bug.

---

## Checked and correct

**`notify.rs` `admit` (lines 86-132).** Compared against the base line by line. The old
`match event.action { Block => blocked_pref, Flag => flagged_pref }` sat in exactly the
position `if !prefs.notifications` now occupies: after `let mut out = self.sweep(prefs,
now)` and before the key is built, the bucket is looked up or inserted, and the `Fire` is
pushed. Nothing moved relative to a side effect. `out` (which may already carry a sweep
summary) is still returned on the early exit, so a summary produced by the sweep is not
swallowed by an off switch - and cannot be, because that same sweep now filters on the
same flag.

**`notify.rs` `sweep` (lines 152-187).** Order preserved exactly: collect expired keys,
`self.buckets.remove(&key)` (so the bucket is retired whether or not the user wants to
hear about it), `if bucket.suppressed == 0 { continue }`, then the preference check, then
push. The removal-before-check ordering is what makes
`a_switch_turned_off_mid_storm_silences_the_summary` (line 490) and
`a_summary_is_delivered_once` (line 476) both hold; it is unchanged. The per-action
`match key.0` for the *title* and for `summary_body` is untouched, so a flagged bucket
still summarises as "More requests flagged".

**The rewritten test (`notify.rs:380-395`).** It still proves its name: with `prefs(false,
..)` both a Block and a Flag event produce an empty result, and with a fresh `Grouper` and
`prefs(true, ..)` both fire. The case the rewrite dropped - blocked off / flagged on and
its mirror - tested per-action independence, which is precisely the property the change
removes; there is no way to keep it. Nothing else was lost: the grouping, window, summary,
sound and mid-storm tests all still run with the same shapes (49 tests pass).

**`preferences.rs` defaults and round trip.** `Default` sets `notifications: true`
(line ~215); `defaults_are_everything_on` (line 613), `a_missing_field_loads_as_on`
(line 657) and `an_explicit_false_survives_a_round_trip` (line 667) were updated to the new
field and pass. `set_notifications` (line ~358) keeps the read-modify-write shape, so it
cannot clobber a field it has not heard of. For a file containing only the three removed
fields, see M1 - that is the one gap.

**`src-tauri/src/lib.rs`, both read sites.** `lib.rs:4346` (teardown/disconnect) and
`lib.rs:5300` (dead-session tick) both read `preferences::load().notifications` where they
read `.routing_health_notifications`; the surrounding conditions, the `#[cfg]` on the tick
site and the "the list is still returned either way" behaviour are unchanged.

**Both registration lists.** They are the two arms of one `#[cfg]` fork inside a single
`invoke_handler` (`lib.rs:4463-4611`): lines ~4470-4553 are the
macOS/Windows/Linux arm, lines ~4557-4609 the `#[cfg(not(any(...)))]` fallback. Neither is
test-only nor a second window. Both carried `set_routing_health_notifications` and both now
carry `set_notifications` (lines 4527 and 4602). `set_blocked_event_notifications` /
`set_flagged_event_notifications` only ever existed in the desktop arm, alongside the rest
of the security-feed commands, so removing them from that arm alone is right. The fallback
arm still lacks `set_security_notification_sound` and the `security_feed_*` commands, which
is pre-existing and consistent with that arm having no proxy subsystem. `cargo check`
passes, which is the real proof: `generate_handler!` would not compile against a missing
function.

**`src/NewUiApp.tsx:2340-2351`.** Same five steps as before, in the same order:
`setActionError(null)`, optimistic `setPrefs((p) => (p ? { ...p, notifications: next } :
p))` (correct state key, and the `p ? ... : p` guard means a click before the first load
is a no-op rather than a synthesised object), `setNotifications(next)`, `.catch` into
`classifyError(e, "generic")`, `.finally(() => void loadPreferences())`. The rollback is
that re-read, exactly as for the sound and diagnostics switches beside it. `next` is
computed from `prefs?.notifications ?? true`, matching the backend default, so the first
click on an unread preference turns it off rather than "on".

**`src/components/gc/SettingsPane.tsx:488-522`.** The row is gated on
`onToggleNotifications` and keeps the `preferencesUnavailable && onRetryPreferences`
branch that the old routing-health row had; `notifications ?? true` matches the backend
default. The sound row's lack of that branch is **pre-existing**: `git show
origin/feat/new-app-ui:src/components/gc/SettingsPane.tsx` shows the same row at its
line 551-561 with a bare `toggle` and no `unavailable`. Not introduced here. (It is
arguably wrong - when the preferences read fails the sound row shows a working switch over
a value nobody could read - but that is a separate, older bug.)

**The e2e stub.** `e2e/backend.ts:192-199` and `:621-624` define and default the single
`notifications` field; `e2e/install.ts:501-505` registers `set_notifications` and drops the
three old handlers. `rg` over `e2e/` finds no reference to `set_routing_health_notifications`,
`set_blocked_event_notifications` or `set_flagged_event_notifications`, and the specs that
used them were rewritten (`new-ui-security-feed.spec.ts:255-279`,
`new-ui-settings.spec.ts:230-241`). The switch locators are unambiguous under Playwright's
default substring name matching: the accessible name is `row.label`
(`SettingsPane.tsx:826-831`), so "Notifications" does not also match "Notification sound".
All three touched specs pass.

**Repo-wide rename.** `rg` for every removed Rust field, Rust function, Tauri command and
TS export returns hits only in `plans/new-app-ui-figma.md`, where they are the narrative of
the change. The old popover shell (`src/screens/Settings.tsx`) never exposed these
switches, so nothing there needed updating.

**Docs.** `docs/figma-questions-for-design.md:244-266` marks question 8 RESOLVED and
records both live consequences (the drawn sentence under-describes the routing half; the
sound row stays undrawn). `SettingsPane.tsx:485-487` points at it. Consistent.
