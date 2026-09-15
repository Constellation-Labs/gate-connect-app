# Review: `fix/alpha4-review-findings` - correctness lens

Base: `origin/feat/new-app-ui`. 29 files, +931/-135.

## Summary

Most of the branch holds up under the correctness lens. The counting change, the
activity-pane `pending` exclusion, the `Modal` layout rewrite, the `TeardownReason`
threading and the Settings value truncation all check out against the code they
touch, including the unchanged callers. Two things do not. First, the headline fix
of the branch - not calling a deliberate sign-out "Session expired" - is wired
correctly in `App.tsx` (the popover) and is inert in `NewUiApp.tsx` (the shipping
default shell), because `NewUiApp` reads `preferences` once at mount and never
re-reads them after a disconnect, so `stage.deliberate` is the pre-sign-out value.
Second, the new `historyUnavailable` state ships with a recovery action that cannot
recover: `securityFeedRetry` only shortens a backoff `wait()`, and the backfill runs
once per connection inside `connect_once`, so "Try again" on a Live feed with a
failed catch-up does nothing at all. Below that are three smaller races and copy
claims in the same feed code, one banner-semantics change worth a decision rather
than a fix, and some nits.

## Verdicts on the eight items

1. **`protectedCount` / `totalCount` from `railApps`** - superset property VERIFIED
   CLEAN; the denominator's meaning is a finding. `railApps` is a strict superset of
   `apps` and cannot be empty while `apps` is not (`src/NewUiApp.tsx:1380-1436`):
   with no groups the fallback group carries `apps` wholesale, and with groups every
   app is either claimed by a group or swept into the `unclaimed` group. `allProtected`
   already guards `totalCount === 0` (`src/components/gc/banners.tsx:102`), so an
   empty rail reads amber "0 of 0", exactly as before. What changed is that the
   denominator now includes proxy domain rows that ship `enabled: false`. See M3.
2. **`!unattributedMachine` in the two `pending` expressions** - VERIFIED CLEAN.
   No stale reading is possible and no state is left unreported. See the walk-through
   under M/L notes; one copy caveat is L4.
3. **`Modal` flex/absolute rewrite** - VERIFIED CLEAN. `useFocusTrap` is unaffected
   (it queries DOM order, and the close button's DOM position did not move), and all
   62 `Modal` call sites were checked for block-flow dependence. Nits in L5.
4. **`historyUnavailable`** - NOT CLEAN. `Ordering::SeqCst` is correct and
   `set_history_ok`'s swap-based emit-on-change is sound for a single connection task.
   The problems are the retry (M1), the never-called reset path (M2), and two narrow
   races (L1, L2).
5. **`deliberate`** - `App.tsx` VERIFIED CLEAN; `useSetup` has the equivalent hole,
   and it is the one that ships. See H1.
6. **`onTeardown(reason)`** - VERIFIED CLEAN. Four call sites, all correct.
7. **`setActionError(null)` on the toggles** - mostly clean, one real loss. See L3.
8. **`min-w-0 max-w-xs shrink`** - VERIFIED CLEAN. No row that must stay fully
   visible is affected.

## Findings

### H1 - The new shell still calls a deliberate sign-out "Session expired"

`src/NewUiApp.tsx:1639` passes `signedOutDeliberately: prefs?.signed_out_deliberately`
into `useSetup`. `prefs` is filled by `loadPreferences` (`src/NewUiApp.tsx:674-678`),
which is called from the mount effect (`src/NewUiApp.tsx:908`), from the six
preference toggles' `.finally()` (`:1949, :1961, :1970, :1979, :1993`), from
`onRetryPreferences` (`:1995`) and from the onboarding diagnostics step (`:2457, :2467`).
It is called from no sign-out path. `settings.confirmDisconnect`
(`src/lib/useSettingsActions.ts:303-330`) calls `oauthSignOut`, re-reads the account
and OAuth status, and hands them to `onSession`; nothing re-reads preferences, and
`NewUiApp` is not remounted.

Failure scenario: user is signed in (so `signed_out_deliberately` is `false` on disk
and in `prefs`). They open Settings, click Disconnect Gate, confirm. `oauth_sign_out`
writes `signed_out_deliberately = true` (`src-tauri/src/lib.rs:602`). The derived
stage becomes `{ kind: "welcome", reauth: true, deliberate: prefs.signed_out_deliberately }`
= `false` (`src/lib/useSetup.ts:182-193`), so `WelcomePane`
(`src/components/gc/setup.tsx:436-455`) renders the heading "Session expired" and
"Sign in again to keep routing your apps through Gate" over the user's own click.
Quitting and relaunching fixes it, which is what makes it easy to miss in manual
testing. `App.tsx:1172-1186` does not have this hole: it re-reads `getPreferences()`
on every entry to `screen === "firstrun"`, and `signOut` awaits `oauthSignOut()`
before `setScreen("firstrun")` (`src/App.tsx:672-676`), so the write is ordered before
the read.

Fix shape: add `void loadPreferences()` alongside the account re-read in the
`onSession` handler, or key an effect on the account/oauth pair the way `keyPrefix`
is keyed (`src/NewUiApp.tsx:925-928`).

### M1 - The "Try again" offered for a failed catch-up cannot re-run the catch-up

`SecurityPane` draws "Try again" in both new places (`src/components/gc/SecurityPane.tsx:147-167`
and `:205-222`), wired to `securityFeed.retry`. `retry` calls `securityFeedRetry()`
then `seed()` (`src/lib/securityFeed.ts:173-178`). `securityFeedRetry` reaches
`Feed::retry_now`, which is `self.wake.notify_one()`
(`crates/core/src/security_feed/client.rs:103-105`). `wake` is only awaited inside
`wait()` (`client.rs:317-327`), which the run loop enters between connection attempts.
While the stream is Live the task is parked in `connect_once`'s read loop, so
`notify_one` merely stores a permit. The backfill runs exactly once per connection,
on `hello`, behind `if !hello.recovery && !backfilled` (`client.rs:585-588`).

Failure scenario: the gateway answers `/history` with 400 (the case the feature was
written for). `set_history_ok(false)` emits, the pane shows LIVE plus the amber
"Showing events from this session only" strip. The user clicks Try again. `seed()`
re-invokes `security_feed_history_ok`, which returns the same `false`;
`securityFeedRetry` parks a permit that will be consumed by the next backoff sleep.
Nothing on screen changes, no request is made, and the strip stays. The user can
click indefinitely. The catch-up only retries if the stream happens to drop.

Either give the retry a real path to a backfill (a command that re-runs it, or a
forced reconnect), or drop the action and state the condition without offering one.

### M2 - `reset_for_account_change` is never called, and the comments say it is

`crates/core/src/security_feed/client.rs:110-116` gained
`self.history_ok.store(true, ...)`, and `src/lib/securityFeed.ts:88-91` justifies its
own reset with "The backend clears its own copy in `reset_for_account_change`; this
keeps the two from disagreeing for the length of one read." A repo-wide grep for
`reset_for_account_change` finds the definition (`client.rs:110`) and one call, inside
the test module (`client.rs:719`). The `Feed` is a process-lifetime
`OnceLock<Arc<Feed>>` (`src-tauri/src/lib.rs:2165-2169`) and nothing restarts it, so
in a shipped build the backend's `history_ok` never resets on an org switch.

Failure scenario: org A's catch-up fails, so backend `history_ok` is `false` and the
pane shows the strip. The user switches to org B. The frontend `[credential]` effect
sets `historyOk` back to `true` (`securityFeed.ts:84-92`), then `seed()` immediately
re-reads `security_feed_history_ok()` and gets org A's `false` back, painting the
warning on a pane that has asked org B nothing. If org B's backfill then also fails,
`set_history_ok(false)` sees no change and emits nothing, so the two stay silently
coupled. The state converges to the right answer only because the failure repeats;
a success emits `true` and clears it.

Not a crash, but the code and its comments disagree about what protects the invariant.
Either call `reset_for_account_change` from the account-change path, or delete the
`history_ok` line from it and correct both comments.

### M3 - Green "Gate Connect is protecting you" is now unreachable on a default install

`totalCount` is `railApps.length` (`src/NewUiApp.tsx:2510`) and `railApps` includes
proxy domain rows, whose status comes from `proxyMemberStatus`
(`src/lib/verdict.ts:116-120`): `protected` only when `domain.enabled && proxyOn && caTrusted`
(`src/lib/groups.ts:463-467`). In `crates/core/src/proxy/catalog.rs` exactly one entry
ships `enabled: true` (line 79); the rest are `enabled: false` (lines 150, 207, 305,
347, 375, 407 …), including `openai` under Experimental and the chat surfaces the
provider catalog attaches through `chat_domain_slugs`
(`crates/core/src/provider.rs:93, :134`).

Failure scenario: a user routes every config tool they have. `apps` is 4 of 4, the
family panel says Protected, and the banner previously read green "Routed - 4 of 4
Apps". After this change the rail also lists, say, `openai`, `openrouter` and
`claude-web`, all off by design, so `allProtected` is false and the strip reads amber
"Gate Connect is partly routing your apps - Partly routed - 4 of 7 Apps" on a machine
where nothing is wrong. The old asymmetry it fixes was real, but the fix makes the
green state depend on the user enabling opt-in interception surfaces they were never
asked to enable. Worth a decision (count only rows whose `desired` is true? count
config tools plus enabled domains?) rather than shipping the amber-forever banner.

### L1 - The seeded `history_ok` read can clobber a fresher event

`seed()` guards the reply with `mine === attempt.current`
(`src/lib/securityFeed.ts:108-112`). That is an epoch guard, not an ordering guard: it
drops replies from a *previous* credential, not a stale value from the same epoch.
The `security-feed-history` listener (`:163-165`) has no guard at all.

Failure scenario: the user clicks Try again while the feed is healthy.
`securityFeedHistoryOk()` is invoked and reads `true`. Before its reply reaches the
webview the connection drops, reconnects, and the backfill fails, emitting
`{ok:false}`; the listener sets `historyOk = false`. The invoke reply then lands, still
in the same epoch, and sets `historyOk = true`. The strip disappears with the history
genuinely missing, and no further emit is coming (emit-on-change already fired). Narrow,
because the invoke normally answers in a millisecond and the backfill needs the network,
but the code has nothing that prevents it. Stamping the read with a monotonic counter
that the listener also bumps would close it.

Related: `seed()` returns before `++attempt.current` when `enabled` is false
(`securityFeed.ts:95-96`), so a credential change that also disables the hook leaves
the previous account's in-flight reads unguarded. Pre-existing shape, now with one
more read riding on it.

### L2 - A history flip between the seed read and listener registration is lost forever

The seed effect (`securityFeed.ts:131-133`) runs before the listener effect
(`:135-171`), and `listen()` is itself asynchronous, so there is a window at mount in
which the read has already been served and no listener exists. Because
`set_history_ok` emits only on change (`client.rs:84-88`), a `true -> false` flip
inside that window is never re-announced: the frontend holds `true` and the backend
holds `false` until something flips it back. The window is short (the backfill needs a
round trip) but the consequence is permanent for that connection, which is the
combination worth knowing about.

### L3 - A toggle now clears an unrelated failure whose only trace was the banner

`actionError` is a single slot, fed by every settings action through
`onError: (e) => setActionError(classifyError(e, "generic"))` (`src/NewUiApp.tsx:1659`)
as well as by routing. The seven new `setActionError(null)` calls
(`:1450, :1930, :1943, :1956, :1965, :1974, :1983`) clear it unconditionally.

Failure scenario: the user clicks Rename device; the write fails; the banner says so.
Without reading it they flip "Alert me when a request is blocked". The banner vanishes
on the click, the notification write succeeds, and nothing is left saying the rename
failed. The device row still shows the old name, which is the only remaining hint.
The same shape applies within the toggle group: switch A fails and sets the error,
switch B clears it and succeeds, and A's `loadPreferences()` rollback then moves A
back with no explanation on screen. The comment's reasoning ("the click is the moment
the last failure stops being the current answer") is right for a retry of the same
subject and wrong for a different one; keying the clear on the subject, or clearing
only errors this control raised, would keep the fix without the loss.

### L4 - "the gateway has no traffic attributed to it yet" is also said when the list read failed

`useInstallations` sets `resolved` in `.finally()`, on the failure path too
(`src/lib/activity.ts:574-582`), so `unattributedMachine` (`src/NewUiApp.tsx:516`) is
true both when the gateway answered "I do not know this machine" and when the
installations request never got an answer. The alert copy (`src/NewUiApp.tsx:3156-3162`)
states the first as fact. Pre-existing in `unavailable` and in the copy; this branch
extends the same conflation to `pending`, so a failed installations read now renders
as a confident "no traffic attributed to it yet" instead of a skeleton. Both are wrong;
the new one is wrong more assertively. Worth distinguishing "asked and told no" from
"could not ask" in `useInstallations` if this copy is going to make a claim.

The rest of item 2 is clean, and worth recording so it is not re-derived: when
`unattributedMachine` is true, `installsResolved` is true, so the old expression was
permanently `true`; `toolActivity` and `toolEvents` are gated on `machineKnown`
(`:518, :574`), so both stay `null` forever; `unavailable.chart` and `unavailable.events`
already OR in `unattributedMachine` (`:3105-3111`), so nothing goes unreported;
`stats` falls back to `EMPTY_STATS`, all-null, which `metrics.tsx:119` renders as `n/a`.
`pending` and `unavailable` cannot both be false with the data absent: `pending` can
only be false via `openDomain`, `unattributedMachine`, or `installsResolved && (view || failure)`,
and each of the first two forces `unavailable`, while a non-null `failure` with a null
`view` also forces it. Stale numbers are impossible too: `useActivity` clears `view`
on any `installId` change (`activity.ts:483-486`), and `useInstallations` batches
`setCurrent(null)` with `setResolved(false)` (`activity.ts:562-564`), so there is no
render in which `unattributedMachine` is true while a previous scope's `view` is still
held.

### L5 - `Modal` nits

Item 3 is clean: `useFocusTrap` walks `panelRef` in DOM order, the close button is
still the panel's first child (`float-right` never changed DOM order either), and
`absolute right-5 top-5` lands where the old `-mr-1 -mt-1` did, with the header's new
`pr-8` (`Modal.tsx:243-249`) more than covering the 20px the button reaches into the
padding box. `max-h-full` resolves against the scrim's content box, which already
excludes its `p-6`. Three things to know rather than fix:

- `overflow-y: auto` forces the computed `overflow-x` to `auto` as well, so a wide
  child in a narrow dialog gets a horizontal scrollbar where it used to overflow
  silently.
- Controls inside the scrolling body draw `focus-visible:outline-offset-2`, which now
  clips against the container edge at the very top and bottom of the scroll range.
- Three children already scroll themselves - the diagnostics `<pre>` (`dialogs.tsx:800`,
  `max-h-72`), the model list (`dialogs.tsx:1332`, `max-h-[22rem]`) and
  `CollectedDataScroller` (`dialogs.tsx:1830`, `max-h-96`). At the 800px floor these
  can produce two nested scrollbars. Correct, just busy; capping them in `vh` rather
  than `rem` would let the outer one do the work.

### L6 - Latent double count if a domain slug ever equals a tool slug

`sidebarGroups` pushes a row per group member (`src/NewUiApp.tsx:1387-1409`), and
`buildGroups` warns that "tool and domain slugs share no namespace guarantee -
`opencode` is both" (`src/lib/groups.ts:55-61`). Today nothing lists the `opencode`
domain in `LEFTOVER_GROUPS.domainSlugs` and no provider's `domain_slugs` names a tool
slug, so `railApps` holds no duplicate keys and the new count is exact. If a backend
catalog ever adds one, `totalCount` counts that slug twice and `appFor` resolves to
whichever row came first. Cheap insurance would be deduping `railApps` by slug, but
it is not a bug today.

## Verified clean, in detail

- **Item 6.** `onTeardown` has four call sites and all four are right:
  `confirmDisconnect` passes `"sign-out"` (`useSettingsActions.ts:323`, and sign-out
  does keep the configs), `confirmReset` passes `"teardown"` on both the success and
  the failure path (`:454, :462`, and reset does attempt a restore), and the master
  routing switch passes `"teardown"` when turning routing off (`NewUiApp.tsx:1234`).
  `TeardownReportDialog` defaults to `"teardown"` (`dialogs.tsx:2250`), so the tray's
  and any other caller's behaviour is unchanged.
- **Item 8.** Only four row shapes carry both `description` and `value`, and so take
  the new `min-w-0 max-w-xs shrink` branch: Device (`SettingsPane.tsx:230-247`),
  Sign-in method ("Gate account" / "API key", `:311-333`), Gate certificate
  ("Trusted" / "Not trusted") and Version (`:549-561`). Every long value in the pane -
  Install ID (`:249-255`), Login ID and Gate plan (`:261-273`), Gateway (`:277-286`)
  and the masked API key (`:288-299`) - carries no description and keeps the
  `min-w-0 flex-1` branch untouched. Of the four, only the device name can exceed
  `max-w-xs` (320px), which is the case the change exists for, and `title={row.value}`
  keeps the full string reachable.
