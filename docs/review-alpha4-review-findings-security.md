# Security review - `fix/alpha4-review-findings`

Base: `origin/feat/new-app-ui` · 29 files, +931/-135 · lens: **security**

## Summary

The branch is clean on the three surfaces the brief flagged as highest risk. The
`raw` error string now rendered in the send-diagnostics disclosure cannot carry a
credential: the send path is a frontend `fetch` whose only throw sites are three
literal strings and a network/timeout error, and it never echoes a header or a
response body. The email now shown as **Login ID** is display-only, read from
`oauth_status()`, and is not added to analytics or to `errorContext`'s
anonymous-only payload - the diagnostics report already carried it. The new
`security_feed_history_ok` command takes no arguments, returns one bool from an
atomic load, and exposes nothing the tray could not already learn from
`security_feed_state` / `security_feed_recent`. Credentials stay in headers on
the backfill path, so the `reqwest` error strings logged on the new failure
branches cannot leak a token, and no new log line was added at all. The two real
findings are about **state and layering, not secrets**: the backend security feed
still never resets across an account or org change - the function that would do
it has no production caller, and this branch adds the new `history_ok` to that
dead function while documenting the reset as if it happened - and the topnav
menu's z-index was raised above the modal layer, so a cross-window-triggered
confirmation dialog can render under a full-viewport scrim.

---

## H - must-fix

None.

---

## M - should-fix

### M-1. The backend security feed never resets on account/org change; the branch adds `history_ok` to a dead reset function and documents it as live

`Feed` is a process-wide `OnceLock` singleton (`src-tauri/src/lib.rs:2165-2170`)
created once at startup and never replaced. The only thing that would clear its
per-org state is `Feed::reset_for_account_change`
(`crates/core/src/security_feed/client.rs:110-119`), and **it has no production
caller**:

```
$ rg -n 'reset_for_account_change'
src/lib/securityFeed.ts:89              (a comment)
crates/core/src/security_feed/client.rs:110   (the definition)
crates/core/src/security_feed/client.rs:719   (a unit test)
```

`signal_session_changed()` (`src-tauri/src/lib.rs:1769-1773`) only emits a JS
event; it does not touch the feed.

The branch adds the new `history_ok` reset into that same unreachable function
(`crates/core/src/security_feed/client.rs:117`) and then writes, in the frontend
hook, that the backend does the clearing:

- `src/lib/securityFeed.ts:88-91` - *"The old org's failed catch-up says nothing
  about the new one's. The backend clears its own copy in
  `reset_for_account_change`; this keeps the two from disagreeing for the length
  of one read."*

The backend does not clear it, and the frontend's own clear is undone one tick
later. `seed()` lists `credential` in its dependency array
(`src/lib/securityFeed.ts:129`), so a credential change runs the clearing effect
(`:84-92`) and then immediately re-seeds from the backend:

- `src/lib/securityFeed.ts:108-112` re-reads `securityFeedHistoryOk()` -> the
  previous org's flag.
- `src/lib/securityFeed.ts:113-116` re-reads `securityFeedRecent()` ->
  `setEvents(recent)` with the previous org's buffered events.

So after an org switch the Security pane renders the **previous org's security
events under the new org's name**, which is precisely what the hook's own
comment at `src/lib/securityFeed.ts:76-83` says must not happen, and what the
test at `crates/core/src/security_feed/client.rs:711-721` asserts the reset
exists to prevent. Same for `history_ok`: a stale `false` puts an amber "Earlier
events couldn't be loaded" warning on a pane that has not asked the new org
anything (`src/components/gc/SecurityPane.tsx:144-167`), and a stale `true`
suppresses the warning after a failure.

Same OS user throughout, so this is a data-attribution defect rather than a
cross-principal leak - hence M, not H. But the Security pane is the one screen in
this product whose whole job is making a claim about the user's traffic, and
principle 6 in `CLAUDE.md` is explicit that a claim nobody's gateway answered is
worse than no claim.

**Attribution:** the buffer/dedupe half is pre-existing. The branch introduces
the `history_ok` half and introduces the comment asserting the reset is live -
which is the part that will keep the next reader from finding this.

**Fix shape:** call `security_feed().reset_for_account_change()` from wherever
`signal_session_changed()` fires (`src-tauri/src/lib.rs:426, 445, 490, 622, 643`
and the org-switch command), or drop the claim from
`src/lib/securityFeed.ts:88-91`.

---

## L - nits

### L-1. The topnav menu now paints above the modal layer, and dialogs are not only opened by menu clicks

`src/components/gc/Topbar.tsx:230` raises the menu's own scrim to `z-40` and
`:248` raises the panel to `z-50`. `Modal` is `z-20`
(`src/components/gc/Modal.tsx:209`) and `AppShell`'s notice wrapper is `z-30`
(`src/components/gc/AppShell.tsx:111`). The comment at `Topbar.tsx:238-247`
justifies going above the dialog with *"a menu and a dialog never coexist -
`onMenuSelect` closes this before opening one"*.

That holds for dialogs the menu itself opens. It does not hold for dialogs raised
by cross-window events, which the shell listens for:

- `src/NewUiApp.tsx:874` - `listen("quit-requested", sweep)`, the quit chooser.
  This is the dialog that decides whether tool configs are torn down and
  restored, and it is fired by the tray's Quit or the OS quit accelerator, which
  a user can hit with the menu open.
- `src/NewUiApp.tsx:1673` - `listen("switch-org-requested", ...)`, opened from
  the tray.

In that window the dialog renders at `z-20`, underneath a `fixed inset-0 z-40`
scrim, so its buttons are not clickable and the panel is partly occluded by the
`z-50` menu. Recoverable - the first click lands on the scrim and dismisses the
menu, and `Modal`'s `useFocusTrap` (`src/components/gc/Modal.tsx:199-206`) still
moves focus into the dialog so the keyboard path works - which is why this is L
rather than M. Branch-introduced; before this change the menu was `z-20`/`z-10`
and tree order put the dialog in front, which was correct.

### L-2. `signed_out_deliberately` has one clearing site, and it is best-effort

`crates/core/src/preferences.rs:156` adds the persisted bool. It is set `true` at
`src-tauri/src/lib.rs:602` (`oauth_sign_out`) and cleared at
`src-tauri/src/lib.rs:571` (`oauth_begin_login`). Both writes are `let _ = ...`,
so a failed preferences write is silent. Nothing clears it on
`account::clear()` (`crates/core/src/account.rs:478-512`, the disconnect path),
on reset (`:527-540`), or on an API-key sign-in.

The failure mode that matters is the clear at `:571` failing: a later genuine
session expiry or a server-side revocation then renders **"You are signed out"**
(`src/components/gc/setup.tsx:436-440`,
`src/screens/FirstRun.tsx:124-146`) - the app telling the user they ended the
session when in fact the gateway did. That is a security-relevant
misattribution on a credential surface.

Blast radius is small, which is why this is L and not M:

- The copy only renders under `reauth`, which requires
  `account.auth_mode === "oauth"` (`src/App.tsx:1245`,
  `src/lib/useSetup.ts:187`), so the API-key path cannot reach it.
- After a disconnect or reset the account is gone, so `reauth` is false and the
  stale flag is unreachable.
- `oauth::store` is only reached through `oauth::login` and the refresh loop
  inside `crates/core`, so `oauth_begin_login` really is the only new-session
  entry point - there is no second sign-in path that would bypass the clear.

The flag itself is a bool in an already `0o644` `preferences.json`
(`crates/core/src/preferences.rs:310`), so it adds no PII to disk and cannot be
used to escalate anything - it only chooses one of two sentences.

### L-3. A truncated backfill still reports `history_ok = true`

`crates/core/src/security_feed/client.rs:435` sets `history_ok` true as soon as
the page parses, and the `page.truncated` branch at `:445-449` only writes a log
line. So a catch-up that fetched one page of a longer window renders as a
complete history, with no warning on the pane. This is the same class of claim
the branch set out to fix (a partial reading presented as a whole one, principle
6), one step further down. The `HistoryPage` doc comment at
`crates/core/src/security_feed/mod.rs:286-289` says `truncated` is read
*"because a partial catch-up that presents as a complete one is the thing this
whole route exists to stop"* - the new signal is the first place it could have
been surfaced and was not.

---

## Verified clean

Stated explicitly so the next reviewer does not re-derive them.

- **The new `raw` disclosure in `SendDiagnosticsDialog`
  (`src/components/gc/dialogs.tsx:2079-2086`, fed from
  `src/NewUiApp.tsx:1833-1835`) cannot carry a secret.** The send path is a
  frontend `fetch` (`src/lib/diagnosticsUpload.ts:100-130`) whose only throw
  sites are two literal strings (`:101`, `:124`) and a `fetch` /
  `AbortSignal.timeout` rejection. No header, no response body, and no
  credential is interpolated into any of them. `collectReport`
  (`src/NewUiApp.tsx:1770-1803`) catches every `invoke` rejection individually,
  so no backend error string reaches this dialog either.
  `ErrorDetails` (`src/components/gc/banners.tsx:450-483`) is the pre-existing
  component; this is its one new call site in the branch.
- **The email in Settings is display-only.** `src/NewUiApp.tsx:1864` reads
  `oauth?.email` from `oauth_status()` (a keychain read,
  `src/lib/api.ts:116`). It is not added to any `track()` call, and
  `lib/errorContext.ts`'s anonymous-only posture is untouched - its docstring's
  "no name, email, or account identifier" claim is still true. The diagnostics
  report already emitted it (`src/lib/diagnosticsReport.ts:232`), so no new
  egress path was opened. Falls back to `"-"` for an API-key account, which has
  no email.
- **The new `title={row.value}` tooltip (`src/components/gc/SettingsPane.tsx:750`)
  reveals nothing new.** It only restores text already rendered but ellipsed, it
  only applies to rows carrying a `description`, and the API-key row has none
  (`src/components/gc/SettingsPane.tsx:288-298`) - its value is masked upstream
  anyway (`src/NewUiApp.tsx:3434-3437`).
- **`security_feed_history_ok` is correctly scoped.** `src-tauri/src/lib.rs:2259`
  - no arguments, no deserialization, returns a bool from
  `AtomicBool::load` (`crates/core/src/security_feed/client.rs:76-78`). Sync
  command doing no blocking work, matching `security_feed_state` beside it.
  App-defined commands are not capability-gated here (see the note in
  `src-tauri/capabilities/tray.json`), so the tray can call it - which discloses
  nothing it cannot already get from `security_feed_state` /
  `security_feed_recent`. The paired `security-feed-history` event
  (`src-tauri/src/lib.rs:4621-4623`) broadcasts a bare bool.
- **No credential can reach the new failure paths' logs.** `credential_headers()`
  puts the bearer and org id in headers only
  (`crates/core/src/security_feed/mod.rs:308-341`), and `history_endpoint()`
  (`:276-282`) derives from the account base URL with no query secret - so the
  `reqwest` `Display` strings already logged around the new `set_history_ok`
  calls cannot contain a token. The branch adds **no** new log line: every
  `set_history_ok(false, sink)` at `client.rs:358, 372, 386, 407, 417, 427` sits
  inside a pre-existing log block.
- **No unsafe deserialization.** `Update::History { ok: bool }`
  (`crates/core/src/security_feed/mod.rs:131-144`) is an internal enum with only
  `Debug, Clone`; it is never deserialized. `HistoryPage`'s derives are
  unchanged.
- **The `unminimize()` cfg widening grants nothing new.** `src-tauri/src/lib.rs:3569`
  and `:3948` both sit after code that already called `show()` + `set_focus()`
  on every platform, so a second launch of the executable could already
  foreground the window. `unminimize` is a no-op on a non-minimized window.
- **The e2e changes are test-only.** `e2e/backend.ts:146-171, 498-508` and
  `e2e/install.ts:466-470` extend the fake Tauri harness; nothing there is
  reachable from a shipped build.
