# Code review: one Notifications row (`fix/one-notifications-row`, c446f3f)

Lens: code quality only. Base `origin/feat/new-app-ui`, 16 files.
Correctness and security are reviewed separately.

The collapse itself is clean and goes all the way down, as the commit message
claims: three preferences, three setters, three commands, three `api.ts`
functions and three pane rows became one of each, and `notify.rs` stopped
branching on `event.action` to pick a switch. `cargo check -p gate-connect-core
--tests` is clean and `SettingsPane.test.tsx` passes (35 tests). No added line
in the diff carries an em dash, and the commit message carries no attribution
footer, so the house rules in `CLAUDE.md` hold.

Three of the asked-about leftovers came back clean and are recorded here so
nobody re-checks them:

- **`shieldBan` and `triangleAlert` are not dead.** Both are still live in
  `src/lib/routingState.ts:54,68,74,82`, `src/components/gc/SecurityEvents.tsx:268,299,375`,
  `src/components/gc/dialogs.tsx` (12 uses), `banners.tsx:414,529`,
  `setup.tsx:620`, `AppPane.tsx:497` and `NewUiApp.tsx:3239`. Keeping them in
  `Icon.tsx:38,39,235,241` is right.
- **No symbol leftovers.** `rg` over the repo finds no
  `routing_health_notifications` / `blocked_event_notifications` /
  `flagged_event_notifications` / `setRoutingHealthNotifications` /
  `set_blocked_event_notifications` / `set_flagged_event_notifications` /
  `routing-health` / `blocked-events` / `flagged-events` outside
  `plans/new-app-ui-figma.md`, where they are deliberate history. The unrelated
  `routing_health` *module* hits are a different thing entirely.
- **The e2e fake matches the real surface.** `e2e/install.ts:504` registers
  `set_notifications` and nothing stale, `e2e/backend.ts:197,624` carry the one
  field, and `e2e/new-ui-security-feed.spec.ts:276-277` actively asserts the two
  old rows are gone. That negative assertion is the right thing to have added.

---

## M1. `preferences.rs`'s module doc still describes the split it just deleted

`crates/core/src/preferences.rs:17-22`. The scope note reads "The per-category
security-event switches (blocked / flagged) and the sound toggle arrived with
the live event feed they gate (AG-578) and not before". The per-category
switches no longer exist; only the sound toggle does. This is the highest-value
stale comment in the diff because it is the file's own header, it is the first
thing a reader of `Preferences` sees, and it names fields that are gone. The
field docstring 25 lines below (`:43-50`) tells the new truth, so the file
contradicts itself top to bottom.

The reasoning in the note is still worth keeping (a switch that gates nothing is
worse than a missing switch) - it just needs to be about `notifications` and
`security_notification_sound`.

## M2. The sound row is not covered by `preferencesUnavailable`, and its test now says it is

`src/components/gc/SettingsPane.tsx:509-523`. The Notifications row (`:495`) and
the share-diagnostics row (`:559`) both swap to `unavailable: { onRetry }` when
the preferences read failed. The sound row does not: it renders
`toggle.on = securityNotificationSound ?? true` unconditionally. So a failed
`get_preferences` now draws a Retry affordance on one row and, immediately below
it, a switch asserting On for a value nobody read - which is exactly what the
sibling test's own docstring forbids (`SettingsPane.test.tsx:223-227`: "A failed
read must not draw a switch. `false` and 'could not be read' look identical on a
toggle").

It is pre-existing, from the AG-578 build, but this commit is what put the two
rows side by side and rewrote the block, and it is a three-line fix.

The test at `src/components/gc/SettingsPane.test.tsx:237` makes it worse by
asserting the opposite: "marks both preference switches unavailable together,
since they share one read" checks `notifications` and `share-diagnostics` only.
There are three switches behind that one read, and the third is the one that
does not follow the rule the test name states. Either gate the sound row and add
it to the assertion, or the test name is a claim the code does not honour.

## M3. No test locks in the documented migration

`crates/core/src/preferences.rs:650-665`. The plan (`plans/new-app-ui-figma.md`,
"One migration consequence, stated rather than guarded") and the commit message
both state the load behaviour for an existing `preferences.json`: the three old
keys are now unknown fields, serde drops them, and `notifications` loads at its
default `true`. Nothing tests it. `a_missing_field_loads_as_on` covers absent
fields, not *unexpected* ones, and the difference matters: a later
`#[serde(deny_unknown_fields)]`, or a `load()` that starts treating a parse error
as fatal, would turn every upgrading install's preferences file into a hard
failure and no test in the crate would notice.

One test - parse `{"routing_health_notifications":false,"blocked_event_notifications":false,"flagged_event_notifications":false}`
and assert it yields `notifications == true` - both guards the seam and makes
the documented "gets it back on once" behaviour executable rather than only
written down. This is the one real coverage gap the collapse left.

## M4. "Notification sound" now sits under a global switch but is still security-only

`src/components/gc/SettingsPane.tsx:511-521` and `src-tauri/src/lib.rs:2427-2440`.
`sound` is only ever applied on the security-feed path (`fire_notification`); the
three routing notifications (`lib.rs:3528`, `:4391`, `:5304`) build their
notification without it. That was coherent when the sound row sat under Blocked
requests and Flagged requests. Now it sits directly under a row labelled
"Notifications" that gates routing notifications too, so the label reads as
global while the preference, the description and the code are security-scoped.

The description ("Play a sound with security alerts") does carry the truth, and
the block comment above the row is accurate, so this is a naming/scope decision
rather than a lie in the code. But it is the one thing the collapse changed the
meaning of without touching it, and it is worth a deliberate answer rather than
an accident: either the label narrows, or the sound preference widens to the
routing notifications.

Related and smaller: with `notifications` off, the sound row still renders live
and gates nothing - the precise failure mode `preferences.rs`'s own module doc
argues against. Disabling it under an off master would be consistent with that
rule; it is a product call, not a defect.

## M5. `plans/new-app-ui-figma.md` marks one superseded paragraph and leaves two

The diff correctly stamps "**Superseded 2026-09-17**" on the 2026-08-31 entry
(`:2999`) and rewrites the AG-594 bullet (`:2220-2228`). Two other paragraphs in
the same file still assert the old world as current, with no marker:

- `:994-998` - "Resolved 2026-08-31, and not by editing the copy: the feed was
  built, and the drawn sentence moved to the Blocked/Flagged rows it was
  describing." That resolution is exactly what this commit reversed, and the
  paragraph is the file's record of which drawn sentences are ours and which are
  the frame's - one of the things `CLAUDE.md` sends a reader here for.
- `:731` - "The Notifications description kept the honest routing-health copy
  over the drawn 'blocked or flagged' ... since superseded by AG-578, which
  built the feed and gave the drawn sentence its own rows." Doubly superseded now.

The plan is explicitly a file people read before starting UI work, and the
commit's own standard for it is the marker it applied elsewhere. Same treatment
for these two.

## L6. The two Startup rows draw the same glyph

`src/components/gc/SettingsPane.tsx:492` and `:513` both use `icon: "bell"`.
Adjacent rows with an identical 20px glyph read as a rendering bug before they
read as a family. Neither is drawn: `116:29086` gives the Notifications row its
bell, and the sound row has no frame at all (question 8 records that). So the
sound row's icon is a free choice, and something like a speaker/volume glyph
would distinguish them - worth raising with the same question that already asks
design for the row, rather than deciding by eye. Flagging it as worth a line, not
as a defect.

## L7. `notify.rs`'s `sweep` doc still speaks in the old vocabulary

`crates/core/src/security_feed/notify.rs:148-151`: "Re-reads the switches rather
than trusting the ones in force when the bucket opened: a user who turns blocked
notifications off mid-storm ...". There is no "blocked notifications" switch any
more. The paragraph's point survives intact - it is about re-reading rather than
about which switch - so this is a one-word fix ("turns notifications off"). The
gate it documents (`:166-168`) is correct and tested
(`a_switch_turned_off_mid_storm_silences_the_summary`, `:491`).

## L8. The relocated comment in `NewUiApp.tsx` now says the same thing twice

`src/NewUiApp.tsx:2337-2339` ("Optimistic then re-read: the switch has to move on
click, and the re-read is what makes a failed write show up ...") and
`:2352-2354` ("Same optimistic-then-re-read shape as the switch above: the switch
has to move on click, and the re-read is what surfaces a failed write ..."). The
second was a cross-reference between two *different* handlers before the
collapse; with the middle handlers gone it is a paraphrase of the comment two
lines of code above it. Delete the second, or shorten it to the cross-reference
it used to be. The inner "Same as the switch above: the retry clears its own last
failure." at `:2357` is a third statement of the same rule inside the same
function.

Also `:2342` says "Every one of these switches rolled back correctly ..." where
the count is now three preference switches; the sentence reads fine without a
number, which is what `:2330` already did ("Same rule as the preference switches
below"). No change needed, just do not reintroduce a count.

## L9. Test coverage on the Rust side lost nothing real

`crates/core/src/security_feed/notify.rs:381-396`. The old
`a_switch_that_is_off_stops_something` proved blocked-off/flagged-on and its
reverse - assertions about a distinction the type no longer expresses, so keeping
them was impossible. The replacement covers both actions in both directions
through the same switch, which is the strongest statement available now. The
sweep-path gate keeps its own test (`:491`), and the sound flag keeps its
independence test (`:398`). The helper's signature change `prefs(blocked,
flagged, sound)` -> `prefs(on, sound)` is applied consistently at all 14 call
sites. Nothing to do; recorded because the question was asked.

## L10. Two test names now overcount

- `e2e/new-ui-settings.spec.ts:215` - "both preference switches read On before
  anything has been written" asserts Notifications and Share diagnostic data,
  while three switches come from that read. It was inaccurate before this commit
  too (there were five), so this is only worth fixing if the file is open.
- `e2e/new-ui-security-feed.spec.ts:257` - "the notification switches reach the
  backend" is fine as written; the comment under it was correctly updated to
  "Two switches, not the four AG-594 names".

## L11. `docs/figma-questions-for-design.md` invents a third closing marker

`docs/figma-questions-for-design.md:244` heads the entry "## 8. RESOLVED - ...".
Every other closed question in the file uses ANSWERED (`:121`, `:214`, `:334`,
`:392`, `:563`) or WITHDRAWN (`:312`). Design skim this file by its headings, and
a third verb invites the reading that RESOLVED means something ANSWERED does not.
The body is excellent - it keeps the open half (the row under-describes the
routing notifications) visible under a closed heading, which is the right shape.
Just use ANSWERED.

## L12. The non-desktop `generate_handler!` still omits the sound command

`src-tauri/src/lib.rs:4556-4610`. The desktop list (`:4470`) registers
`set_notifications` and `set_security_notification_sound`; the
`cfg(not(any(macos, windows, linux)))` list registers only the first. This is
pre-existing - the sound command arrived desktop-only with AG-578 and the diff
merely renamed the routing entry in both lists - and the branch is not a shipped
target. Noting it so the asymmetry is a decision on record rather than a
surprise the next time that list is edited.
