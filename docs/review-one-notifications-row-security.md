# Security review: `fix/one-notifications-row` (c446f3f)

Base: `origin/feat/new-app-ui`. Lens: security only. 16 files, +201 / -300.

The change collapses `routing_health_notifications`, `blocked_event_notifications`
and `flagged_event_notifications` into one `notifications` boolean, with one
setter, one Tauri command and one `api.ts` function. `security_notification_sound`
is untouched.

**Verdict: no high-severity finding.** The gate is fail-open in the only direction
that matters (a missing field loads as on, never as off), no removed command is
still referenced anywhere, and the notification payload is byte-for-byte the one
that was already reviewed. Two findings worth recording, one M and two L.

## Findings

### M1 - an explicit "off" from the shipped build is silently reverted to "on", and then destroyed

`crates/core/src/preferences.rs:42` derives `Deserialize` with no
`#[serde(deny_unknown_fields)]` and no `#[serde(alias = ...)]` on the new field,
and `crates/core/src/preferences.rs:51-52` declares:

```rust
#[serde(default = "default_true")]
pub notifications: bool,
```

`load()` (`crates/core/src/preferences.rs:321-329`) reads the file, parses it with
`serde_json::from_str(&raw).ok()` and falls back to `Preferences::default()` on any
error. There is no migration step anywhere in the repo: `rg -n
'routing_health_notifications|blocked_event_notifications|flagged_event_notifications'`
over the whole tree returns hits only in `plans/new-app-ui-figma.md:3015-3016`.

So for a `preferences.json` written by the shipped build:

1. The three old keys are unknown to the new struct. Serde's default is to ignore
   unknown fields, so the parse succeeds rather than falling back to defaults -
   which is the right outcome, because a fallback would also drop `device_name`,
   `session_routing_accepted` and `tool_models`.
2. `notifications` is absent, so `default_true` fires and the value is `true`.
3. On the next `save()` (`crates/core/src/preferences.rs:332-346`) the struct is
   re-serialized with `serde_json::to_vec_pretty`, which emits only the current
   struct's fields. The three stale keys are dropped from disk permanently.

Consequences, in order of how much they matter:

- A user who deliberately set `blocked_event_notifications: false` (or the flagged
  one, or the routing one) gets native alerts back on after the upgrade, with
  nothing on screen saying so. Their stored choice is then erased by the first
  write of any preference.
- The reverse - a user who left alerts on ending up off - **cannot happen**. There
  is no path that yields `notifications: false` from an old file. The existing test
  at `crates/core/src/preferences.rs:653-666` pins exactly this ("an absent field
  must not read as off"), and `serde_json::from_str::<Preferences>("{}")` is
  asserted equal to `Default`.

Rated M rather than H because the failure direction is louder, not quieter: nobody
is silenced into missing a security alert by the migration. It is still a silent
reversal of an explicit consent-shaped choice, and it is unrecoverable once the
file is rewritten. The cheap fix is a `#[serde(alias)]` or a one-shot read of the
old keys: if any of the three was `false`, seed `notifications` as `false`. A less
cheap but honest alternative is to leave it and accept that the collapse resets the
preference for everyone, which is at least a defensible product decision - but it
should be a decision, not an artefact of `#[serde(default)]`.

### L1 - the row's copy under-describes what the switch now silences

`src/components/gc/SettingsPane.tsx:491-494` draws one row labelled
`Notifications` with the description `Alert me when a request is blocked or
flagged`. That switch now also gates the two routing notifications: the expired
session and the quit that could not put a tool back
(`src-tauri/src/lib.rs:4346` and `src-tauri/src/lib.rs:5300`).

Read as a security question: the direction that would matter is a user turning the
switch off for a reason unrelated to security and losing security alerts as a side
effect. The copy makes the security half the *only* thing named, so someone turning
it off has been told plainly that block and flag alerts stop. The under-described
half is routing, which is the benign direction. The comment at
`src/components/gc/SettingsPane.tsx:485-487` already records this and routes it to
question 8 in `docs/figma-questions-for-design.md`, so it is tracked rather than
missed. No action required for security.

### L2 - the sound row stays live when the switch above it is off

`src/components/gc/SettingsPane.tsx:508-520` renders `Notification sound`
unconditionally, with no dependence on `notifications`. With notifications off the
sound switch gates nothing. Cosmetic, and arguably correct (it preserves the value
for when notifications come back on), but it is the same "a switch that gates
nothing" argument the module doc in `crates/core/src/preferences.rs:17-22` makes
against shipping dead switches. Nit.

## Clean, verified

**The Tauri command surface is consistent.** `set_notifications` is defined once
(`src-tauri/src/lib.rs:2349-2352`) and appears in both `generate_handler!` arms:
the desktop one at `src-tauri/src/lib.rs:4527` and the non-desktop one at
`src-tauri/src/lib.rs:4602`. The three removed commands appear nowhere in the tree.
`set_security_notification_sound` remains desktop-only
(`src-tauri/src/lib.rs:4551`), which matches the feed itself being desktop-only and
is unchanged by this commit.

**No capability or permission file references any of these commands.**
`src-tauri/capabilities/` holds `default.json`, `desktop.json` and `tray.json`; the
only notification entry is the plugin permission `notification:default`
(`src-tauri/capabilities/default.json:18`). App commands are not capability-gated
here, so removing three commands needed no capability edit and none was missed.

**No dangling frontend reference.** `src/lib/api.ts` exports `setNotifications`
only; `setRoutingHealthNotifications`, `setBlockedEventNotifications` and
`setFlaggedEventNotifications` are gone from the file and from every caller.
`src/NewUiApp.tsx` wires a single `onToggleNotifications`. The e2e fake backend
(`e2e/install.ts`) registers `set_notifications` and no longer answers the removed
names, so an accidental leftover call would fail loudly in e2e rather than silently.

**IPC exposure is reduced, not widened.** Three commands became one, each taking
the same `enabled: bool` and doing a read-modify-write of a single field
(`crates/core/src/preferences.rs:360-364`). Nothing new is reachable from the
webview.

**No secret leakage in the notification path.** `notify.rs`'s diff touches only the
gate and the tests; `body_for` (`crates/core/src/security_feed/notify.rs:207-217`)
and `summary_body` (`crates/core/src/security_feed/notify.rs:229-249`) are
unchanged and still interpolate only `category` and `tool`, both of which are
gateway-derived enum-ish strings (`credential | phi | pii | injection | other` and a
tool slug, per `crates/core/src/security_feed/mod.rs:90-98`). No prompt text, no
response body, no matched evidence, no request id, no key material. The tests that
enforce this are still present and still run
(`crates/core/src/security_feed/notify.rs:274-291` and the forbidden-substring
sweep at `:552-558`). `fire_notification` (`src-tauri/src/lib.rs:2417-2440`) passes
only title, body and sound to the plugin, and logs nothing.

**The gate does not touch the in-app record.** `notify_for_event`
(`src-tauri/src/lib.rs:2392-2410`) is the only consumer of the preference on the
event path, and its doc comment is accurate: the event reaches the window
regardless. Turning the switch off suppresses the interruption, not the feed, so a
user with notifications off can still see every blocked and flagged request in the
Security pane. That is the property that keeps M1 from being an H.

**Mid-storm changes still honour the switch.** `Grouper::sweep`
(`crates/core/src/security_feed/notify.rs:150-183`) re-reads `prefs.notifications`
before emitting a trailing summary, so a user who turns the switch off during a
burst does not get the summary afterwards. The collapse preserved this; the test at
`crates/core/src/security_feed/notify.rs:492-505` still covers it.

**Analytics gained nothing and lost nothing.** `src/lib/analytics.ts` reads
preferences in exactly one place, `getPreferences().then((p) =>
p.share_diagnostics)` (`src/lib/analytics.ts:122-123`). It has never referenced any
notification preference and still does not. The diff to
`src/lib/analytics.privacy.test.ts:49` and `src/lib/analytics.test.ts` is a rename
inside the mocked `Preferences` object and nothing more; no event property, no
allowlist entry and no diagnostics payload changed. The privacy review those tests
encode is intact.

**File permissions unchanged.** `save()` still writes 0644
(`crates/core/src/preferences.rs:335`), which is correct: nothing in this file
is a credential.
