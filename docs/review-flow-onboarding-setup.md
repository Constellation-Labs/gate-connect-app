# Flow validation: Onboarding and Setup (states and sequence)

Read 2026-09-03 against Figma file `9FrccCojXy0f8QD8Wm5Lln`.

Companion to `docs/review-figma-onboarding.md`, which is the **visual/token**
audit of the same two canvases. Nothing in that report is repeated here. This
one asks a different question: for every frame the file draws, is there a code
path that renders it; for every state the code can reach, is there a frame; and
does the order match.

Canvases read:

- **`177:79237`** - `Flows / Onboarding`, layer name `↳ Onboarding  ✅`. Two
  captioned sections: `Onboarding / Main flow` (four window frames plus one
  hidden one) and `Onboarding / Images` (three 590x220 art frames).
- **`177:79238`** - layer name `↳ Setup ✅`. **24** window frames, every one
  named `Setup`, on a grid: the caption picks the row (y), the step picks the
  column (x). Rows are `Auth / Connect with Gate ✅` (y=-56, 8 frames),
  `Auth / Connect with API key ✅` (y=939, 8), `Auth / Organizations ✅`
  (y=1934, 5), `Auth / Error states` (y=3024, 2). One further frame
  (`451:7906`) floats at y=-1125, above the first caption line, in no section.

Code read: `src/screens/Onboarding.tsx`, `src/components/gc/setup.tsx`,
`src/lib/useSetup.ts` (the state machine), `src/NewUiApp.tsx` (the container),
`src/lib/tour.ts`, `src/lib/session.ts`, `src-tauri/src/lib.rs`.

Method note: `get_metadata` on both canvases exceeded the return limit and was
written to a file; I extracted the XML and worked from it, then rendered 13
frames as PNGs to read button labels, disabled/busy states and empty states,
which metadata does not carry. `get_screenshot` failed intermittently on this
file (six "unexpected error" responses, several on retry); `209:84604`,
`209:84465`, `231:2371` and `363:12645` never rendered and are read from
metadata only. Those four are marked as such below.

---

## 1. Screen inventory

### Canvas `177:79237` - Onboarding

Progress fills measured off each frame's inner `prog-track` against the 1024
track. Eyebrow strings read off the `Introduction` / `N of M` text pair.

| Figma frame | Draws | Code path | Verdict |
|---|---|---|---|
| `232:4370` (x=0) | Welcome, no card, no eyebrow, rail **256/1024** | `Onboarding.tsx:308-341` - `index === 0` branch; `buildSteps()[0]`, `Onboarding.tsx:44-70` | IMPLEMENTED |
| `212:84747` (x=1076) | Step card, eyebrow `1 of 3`, rail **512** | `Onboarding.tsx:343-399` with `index === 1`; step data `:71-89` | IMPLEMENTED |
| `212:85100` (x=2152) | Step card, eyebrow `2 of 3`, rail **768**, plus the 242x32 locate button | same block, `index === 2`; `step.locate` gates the button at `:391-398` | IMPLEMENTED |
| `212:85391` (x=3228) | Step card, eyebrow `3 of 3`, rail **1024** | same block, `index === 3`; step data `:106-127` | IMPLEMENTED |
| `212:85283` (x=2152, y=1054, `hidden="true"`) | A **fifth** step: eyebrow `3 of 4`, title `How to connect with Gate`, two side-by-side cards `Config apps` / `Proxy apps`, note strip "Open the command center for detail...", rail **768** | none | DRAWN-ONLY (deliberately hidden - see §4) |
| `604:13757` / `604:13806` / `604:14040` | The three 590x220 illustrations, in the `Onboarding / Images` section | committed as PNGs: `assets/onboarding-what-is-gate-connect.png`, `-where-is-gate-connect.png`, `-see-what-gate-is-doing.png` | IMPLEMENTED |

Step count and rail: four frames, fills 25/50/75/100. Code renders
`IntroProgress step={index + 1} total={steps.length}` with `steps.length === 4`
and `width: (step/total)*100%` (`Onboarding.tsx:172-186`, `:305`) - exact. The
eyebrow numbers itself against the three tutorial steps only
(`tutorialTotal = steps.length - 1`, `:290`), which is what the frames draw.

Footer, both affordances: `Previous` is drawn on the welcome frame at zero
opacity and code uses `invisible` + `disabled` (`:437-446`); `Next` becomes
`Get started` on the last step and calls `finish()` (`:447-466`). `finish()`
closes the window, records the seen-flag, tracks `tour_completed` and emits
`gc:tour-seen`. The Rust handler at `src-tauri/src/lib.rs:3124-3129` intercepts
`CloseRequested` on the `onboarding` label and calls `reveal_popover_window`,
which shows window label **`main`** (`lib.rs:2646-2659`) - i.e. the 1024x720
shell, despite the legacy name. So the last step lands the user in the app, and
so does an early close. Correct.

The "don't show again" footer checkbox is drawn on all four frames (206px
instance) and implemented at `Onboarding.tsx:403-433`. See §2 for the state it
gets wrong.

### Canvas `177:79238` - Setup

All 24 frames render the same shell (48px topnav, hidden `banner/update`, the
8px rail) around one 496px `card/organization`. Grouped by what the card draws.

| Figma frames | Draws | Code path | Verdict |
|---|---|---|---|
| `202:80095` (r1c1, rail **256**) and `209:84133` (r2c1, rail **512**) | Sign-in: brand tile, `Gate Connect` wordmark, the sign-in sentence, `Continue with Gate account` / `or` / `Use an API key` | `setup.tsx:408-463` `WelcomePane`; `NewUiApp.tsx:1755-1766`, trigger `stage.kind === "welcome"` (`useSetup.ts:166-171`) | IMPLEMENTED; the two frames disagree on the rail (see §3) |
| `209:84185` (empty, primary muted), `209:84465` (metadata only), `231:331` (value `sk-gw-216c63****…`, primary spinning) | `Use an API key`: field, `Connect and continue`, `Go back` link | `setup.tsx:469-514` `ApiKeyPane`; `NewUiApp.tsx:1767-1776`, trigger `stage.kind === "api-key"` (`useSetup.ts:168`, set by `openApiKey`) | IMPLEMENTED, two state divergences (§3) |
| `208:81532` (3 orgs, first selected, `Continue` enabled), `231:193` (same, `Continue` spinning), `209:84604` (3 orgs, **no** `CircleCheck` on any row - metadata only) | `Choose an organization` + `Use a different account` | `setup.tsx:575-646` `OrgPickerPane`; `NewUiApp.tsx:1777-1790`, trigger `stage.kind === "org-picker"` = `needsOrg(account, oauth)` (`useSetup.ts:165`, `session.ts:20-22`) | `208:81532` IMPLEMENTED; `231:193` DIVERGES (no spinner in code); `209:84604` DRAWN-ONLY (§4) |
| `229:90709` (1 org, selected) | single-org picker | `OrgPickerPane` renders it, but `loadOrgs` auto-confirms when `list.length === 1` (`useSetup.ts:255-263`) | DRAWN-ONLY, deliberately (§4) |
| `229:90782` (2 orgs), `229:90855` (3 orgs) | multi-org picker | as `208:81532` | IMPLEMENTED |
| `229:90928` (3 orgs, list **clipped with a visible scrollbar**, third row cut) | long-list picker | no equivalent: `setup.tsx:624` is a plain `flex flex-col gap-3` with no max-height; the whole card grows and `SetupLayout`'s `overflow-auto` (`:74`) scrolls the page instead | DIVERGES |
| `231:2102` (r3c1) | Zero-org: amber note `No organizations found.` + the Gate-AI sentence, `Go back` primary, `Use a different account` link | `setup.tsx:607-621`, the `organizations.length === 0` branch | IMPLEMENTED (copy verbatim; the frame's glyph is `Icon / CircleAlert`, code uses `triangleAlert`) |
| `209:84046` (empty field, no clear button, primary muted), `209:84353` (filled, clear button, primary enabled), `231:272` (filled, primary spinning), plus r2 duplicates `209:84411` / `209:84294` / `231:388` | `Name this device` + `Skip naming` | `setup.tsx:521-562` `NameDevicePane`; `NewUiApp.tsx:1791-1799`, trigger `stage.kind === "name-device"` (`useSetup.ts:176-178`) | empty/filled IMPLEMENTED (`clearable` gates the clear button, `setup.tsx:302`); busy DIVERGES (no spinner) |
| `363:12645` (r1c8, metadata only) and `376:13771` (r2c8), rail **1024** | `Share diagnostic data`, the sharing row with its switch reading `On`, `Finish setup`, `Skip data sharing` | `setup.tsx:709-760` `DiagnosticsPane`; `NewUiApp.tsx:1800-1829`, trigger `stage.kind === "diagnostics"` = `diagnosticsAnswered === false` (`useSetup.ts:186`) | IMPLEMENTED |
| `231:2271` (r4c1) | **`Setup timeout` modal** over a dimmed org picker whose `Continue` is spinning: `TimerReset` glyph, "Gate Connect timed out will trying to process your request. Would you like to try your request again, or go back to the setup start?", `Go back` / `Retry` | nothing modal-shaped; `SetupError` (`setup.tsx:321-330`) is an inline red note inside the card, with no Retry and no Go back | DIVERGES |
| `231:2371` (r4c2, metadata only) | `Name this device` with the inline validation string `Incorrect characters or symbols used` under the field | none. `NameDevicePane` has only `maxLength={DEVICE_NAME_MAX_LENGTH}` (128, `api.ts:660`); `rg` finds no charset validation anywhere in `src/` | MISSING |
| `451:7906` (y=-1125, no section) | `Name this device` redrawn: 32px tile **beside** a left-aligned title, and a right-aligned action footer (`Skip naming` link + a 36px `Continue`) instead of the full-width stacked primary | `SetupHeader`'s `row` branch (`setup.tsx:162-174`) is exactly this layout - and **no pane passes `row`**. The `SetupFooter` its docstring names (`setup.tsx:130`) does not exist | DRAWN-ONLY (§4) |

Rail stops, measured: sign-in 256, API key 512, org picker 512, name device 768,
diagnostics 1024. `NewUiApp.tsx:1743-1753` encodes 0.25 / 0.5 / 0.5 / 0.75 / 1.
Matches, with the two exceptions in §3.

**`plans/new-app-ui-figma.md:133-175` is confirmed stale.** Its Auth gap table
lists as missing the strings `Continue with Gate account`, `Use an API key`,
`Name this device` and the sign-in subtitle; all four are present verbatim in
`setup.tsx` (`:454`, `:457`, `:540`, `:446`). It also names
`src/components/InstallationPicker.tsx`, which does not exist - the file is
`src/components/gc/InstallationPicker.tsx` and it is the Overview pane's
installation `<select>`, with no frame on either canvas.

---

## 2. States the code can produce that the flow does not draw

| Code state | Trigger | User-reachable? | Notes |
|---|---|---|---|
| `stage: "connected"` -> `ConnectedPane` (`setup.tsx:653-689`, `NewUiApp.tsx:1830-1837`) | `signedIn && sawSignedOut && !confirmationSeen` (`useSetup.ts:180`) | **Yes, on every fresh sign-in** - both routes pass through it | No frame anywhere on the canvas draws "You're connected", a green `circleCheck` tile, `Turn on routing` or `Not now`. Grepping the canvas XML for those strings returns nothing. This is a whole step in the shipping flow with no drawn counterpart |
| `stage: "welcome"` with `reauth: true` -> `Session expired` / `Sign in again` (`setup.tsx:434-445`) | `account.auth_mode === "oauth" && !signedIn` (`useSetup.ts:171`) | Yes, whenever an OAuth session dies | Undrawn |
| `stage: "loading"` -> renders `null`, a blank window (`NewUiApp.tsx:1716-1720`) | first read of account + OAuth in flight | Yes, briefly on every launch | Undrawn and deliberately blank; the comment argues the case |
| `GatewayPicker` under the sign-in and API-key cards (`setup.tsx:345-401`, passed unconditionally at `NewUiApp.tsx:1723-1731`) | always rendered | **Yes, in shipped builds.** `GATEWAY_SERVERS` (`config.ts:31-45`) is Production + Staging in any build; only the `localhost` entry is `import.meta.env.DEV`-gated | Nothing on the canvas draws a gateway line or a `change` control under the buttons. `setup.tsx:421` claims "only dev builds render" this - the container does not honour that. A shipped user can repoint the app at staging from the sign-in card |
| `SetupError` on the sign-in pane and the API-key pane (`setup.tsx:450`, `:494`) | any `signIn` / `connectWithApiKey` rejection | Yes - a wrong key is the ordinary case | `Auth / Error states` draws a timeout and a device-name validation error, and nothing for a failed sign-in or a rejected key |
| `ConnectedPane` and `DiagnosticsPane` with a failed action | `turnOnRouting` / `setShareDiagnostics` rejection | Yes | Neither pane takes an `error` prop, so nothing is drawn *or* rendered. See §4 |
| "Do not show this intro again" checkbox on a `#settings` replay | `openOnboardingWindow("settings")` (`NewUiApp.tsx:1574`) | Yes | `dontShow` is local state initialised to `false` (`Onboarding.tsx:238`) and never read from storage, so on a replay the box renders **unchecked** over a stored flag that is `1`. Checking then unchecking it calls `setTourSeen(false)`, which `tour.ts:22-32` implements as `removeItem` - the intro comes back on next launch. `finish()` writes `true` again, but an early window close leaves it off. Also asymmetric: only `setTourSeen(true)` is broadcast over `TOUR_SEEN_EVENT` (`Onboarding.tsx:274`), so an un-check never reaches the main webview's storage |

---

## 3. Sequence and transitions

**The two entries to the intro window are identical, and the flow draws one.**
`open_onboarding_window` normalises `source` to `firstrun` or `settings` and
splices it into the webview URL as a hash (`src-tauri/src/lib.rs:2418-2436`).
`Onboarding.tsx:242` reads it back, and the value is used **only** in the two
`track()` calls (`:271`, `:288`). No copy, step, affordance or step count
differs. `firstrun` fires from `NewUiApp.tsx:833-836` when `hasSeenTour()` is
false; `settings` from `NewUiApp.tsx:1574` (Settings -> Replay tutorial). The
canvas draws no second entry, and it does not need to - with the one exception
in §2's last row, which is a state the replay entry gets wrong.

**Setup order matches per row, except that the code inserts an undrawn step.**

- Figma `Auth / Connect with Gate`: sign-in (25%) -> org picker (50%) -> name
  device (75%) -> diagnostics (100%).
- Figma `Auth / Connect with API key`: sign-in -> API key (50%) -> name device
  (75%) -> diagnostics (100%). **No org picker in this row.**
- Code (`useSetup.ts:163-188`, evaluated top-down): `loading` -> `org-picker`
  *(if `needsOrg`)* -> `api-key` / `welcome` -> `name-device` -> **`connected`**
  -> `diagnostics` -> `ready`.

The API-key row's omission of the org picker is correct, not an oversight:
`needsOrg` is `auth_mode === "oauth" && signed_in && !org_id`
(`session.ts:20-22`), so an API-key account never enters that stage. The two
rows and the two code branches agree.

The `connected` step is the one real order divergence: the code puts a pane
between `name-device` and `diagnostics` that the file does not draw anywhere.

**The rail reads 100% one step early.** `NewUiApp.tsx:1743-1753` is a ternary
chain whose fallthrough (`: 1`) covers both `connected` *and* `diagnostics`. So
the rail goes 75% -> 100% (connected) -> 100% (diagnostics) -> app, and the user
sees a full bar with a step still to answer. The frames only ever put 1024 on
the diagnostics card.

**The file disagrees with itself on the sign-in rail.** `202:80095` draws 256;
`209:84133`, the same card at the head of the API-key row, draws 512. Code
renders 0.25. The API-key row's copy of the sign-in card looks like a duplicate
whose rail was not reset - it is step 1 of 4 in both rows, and its two siblings
in column 1 of the other rows are not sign-in cards at all. Taking 25%, which
is what the code does. Note the newer-id rule points the other way here
(`209:84133` > `202:80095`), so this is a judgement against node age; flagging
it rather than treating it as settled.

**Busy states are drawn and not implemented.** Three frames draw a spinner
inside the primary with the label beside it and the arrow gone: `231:193`
(confirming an org), `231:272` (saving a device name), `231:331` (connecting a
key). `PrimaryButton` (`setup.tsx:204-217`) renders `opacity-70` and
`aria-busy`, and nothing else - `children` plus the optional arrow. The three
busy frames therefore have no code path, and every long call in this flow (an
OAuth round trip, a `setOrg`, a key validation) shows only a dimmed button.

**The API-key field.** `231:331` draws the value as `sk-gw-216c63` followed by
asterisks, with a `CircleX` clear button. Code uses `type="password"` (fully
masked, no visible prefix) and does not pass `clearable`
(`setup.tsx:497-504`), so there is no clear button on that field at all. The
device-name field does pass `clearable` and matches its frames.

**Where an error sends you.** In the code, nowhere: `SetupError` is an inline
note and the pane stays put, with the user's own affordances unchanged. The
drawn timeout (`231:2271`) is a modal with two destinations - `Retry` (re-run
the request) and `Go back` ("go back to the setup start", per its own copy).
Neither exists. Note also that `Go back` in the org picker's zero-org branch is
wired to `signOut()`, as is `Use a different account`
(`NewUiApp.tsx:1786-1787`) - two labels, one destination, which the code
comment defends and which the frames do not contradict since the drawn dead end
offers both.

---

## 4. Dead ends

**Drawn screens with no way in (four).**

1. `209:84604` - the org picker with **no row selected**. `loadOrgs` sets
   `setSelectedOrgId(list[0]?.orgId)` (`useSetup.ts:254`) the moment the list
   lands, so a non-empty list is always rendered with a selection, and an empty
   list renders the zero-org branch instead. The frame's state is unreachable
   in code. Its purpose is presumably to draw the disabled primary, which the
   code does have (`disabled={!selectedId}`) but can never show.
2. `229:90709` - the **single-org** picker. `loadOrgs` auto-confirms at
   `list.length === 1` (`useSetup.ts:255-263`), so the pane never paints. This
   one is a deliberate, documented divergence: `setup.tsx:572-574` says "the
   container auto-advances when the user belongs to exactly one, so this only
   renders for a real choice". Worth a designer decision rather than a code
   change - the frame exists, so the designer may have intended the
   confirmation.
3. `451:7906` - the parked `Name this device` redesign at y=-1125, in no
   captioned section, above the `Connect with Gate` caption line. Its layout
   (32px tile beside a left-aligned title; a right-aligned footer with the link
   and a 36px button) is precisely `SetupHeader`'s unused `row` branch plus the
   `SetupFooter` its docstring names and that does not exist. So the code
   carries half the scaffolding for a frame nothing points at. Its node id
   (451) is newer than the row it sits above (209/231), which under the
   newest-wins rule would make it the current intent - but it is outside every
   ✅ section and duplicated by three in-row frames that are drawn the old way.
   I did not take it. Needs the designer to say whether it is next or discarded.
4. `212:85283` - the hidden fifth onboarding step, `How to connect with Gate`,
   eyebrow **`3 of 4`**, with `Config apps` / `Proxy apps` side by side. This is
   worth naming precisely because it explains something the visual audit
   flagged and could not source: the code's extra third paragraph on step 1
   ("For Claude Code and Codex, Gate Connect points the app's own config at
   your gateway... For apps like Claude Desktop or ChatGPT, it routes the
   provider's domain through a local proxy") is this retired step's content,
   folded into the surviving step. `hidden="true"` is an explicit layer toggle,
   and the `3 of 4` eyebrow means the flow was cut from five frames to four.
   The prose is not orphaned copy - it is a step the designer removed.

**Code states with no way out (one hard, one silent).**

1. **Hard: the diagnostics step after a failed write.** `onContinue` and
   `onSkip` both route a `setShareDiagnostics` rejection to `setActionError`
   (`NewUiApp.tsx:1817`, `:1826`). `actionError` is rendered **only** inside
   `AppShell` (`NewUiApp.tsx:1897-1904`), which is unreachable while
   `stage !== "ready"`. `DiagnosticsPane` takes no `error` prop. And the stage
   is derived from the stored flag, so `diagnosticsAnswered` stays `false` and
   the pane stays. Both buttons therefore appear to do nothing, forever, with
   no message and no third affordance. Restarting does not help - the stage is
   derived, not stored. This is the last step of setup, so the user cannot
   reach the app.
2. **Silent: `Turn on routing` after a failed `proxyEnable`.** `turnOnRouting`
   sets `setError` (`useSetup.ts:349`) but `ConnectedPane` takes no `error`
   prop and `setupError` is not passed to it (`NewUiApp.tsx:1830-1837`), so the
   spinner clears and nothing changes on screen. Not a trap - `Not now` still
   calls `onDone` and exits - but the failure is invisible.

---

## 5. Could not determine

- **Four frames never rendered.** `get_screenshot` returned "unexpected error"
  on `209:84604`, `209:84465`, `231:2371` and `363:12645`, across retries.
  Their rows in §1 are read from metadata (text layer names, instance names,
  hidden flags, geometry) only. For `231:2371` that means I have the error
  string `Incorrect characters or symbols used` and its 208x16 box under the
  field, but not its colour or icon; for `363:12645` I have the same child set
  as `376:13771`, which I did render, and cannot say what distinguishes the two
  copies of the diagnostics step beyond the row they close.
- **What `209:84465` is for.** The API-key row's third column has the same
  child set as its neighbours and I could not render it. `209:84185` is empty
  and `231:331` is busy, so by analogy with the name-device triple it is the
  filled/enabled state, but I did not confirm it.
- **`Auth / Error states` completeness.** The section carries no ✅ and draws
  exactly two frames: the `Setup timeout` modal over the org picker, and the
  device-name validation error. Per the brief I am not treating the missing
  checkmark as a code bug. What I can say is that neither drawn error has a
  code path, and that the code's own error surface (`SetupError`, inline,
  reachable on four panes) has no frame. Whether the section is unfinished or
  the code is behind it is a designer question.
- **Whether `connected` is meant to exist.** The pane is a real step in the
  shipping flow and nothing on either canvas draws it. I cannot tell from the
  file whether it was never drawn or was removed the way `212:85283` was -
  there is no hidden frame for it, which weakly suggests never drawn.
- **The disabled-vs-enabled arrow.** Across the name-device triple the
  *disabled* primary draws `Continue →` and the *enabled* one draws `Continue`
  with no glyph. That is the same self-disagreement the visual audit raised as
  its mismatch 5 and I have nothing new to settle it with; noting only that the
  inconsistency tracks button *state*, which may be why it happened.
- **`src/components/OAuthOffer.tsx`** is imported only by `App.tsx` (the legacy
  popover). The new shell's equivalent is `OAuthOfferDialog` in
  `src/components/gc/dialogs.tsx`. Neither has a frame on these two canvases,
  so neither can be validated against this Figma - as the previous audit also
  found.
