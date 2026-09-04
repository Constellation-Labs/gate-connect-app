# Flow validation: `Flows / Settings` (`116:28963`)

File `9FrccCojXy0f8QD8Wm5Lln`, canvas `116:28963`, 11 top-level frames (list verified
complete against `get_metadata`: the dump's top level holds exactly the 11 frames plus 8
`text` labels/annotations and 8 `line` dividers, nothing else).

**States-and-sequence audit, not a pixel audit.** The visual/token findings are in
`docs/review-figma-settings.md` and are not repeated here. Open designer questions live in
`docs/figma-questions-for-design.md` and are not re-raised.

Code read (read-only): `src/components/gc/SettingsPane.tsx`, `src/NewUiApp.tsx`,
`src/lib/useSettingsActions.ts`, `src/components/gc/dialogs.tsx`,
`src/components/gc/Modal.tsx`.

*(written incrementally as each measurement lands)*

---

## FIRST FINDING, recorded before anything else: the API-key exception is half wrong

`CLAUDE.md` records as a decided standing exception:

> **Replace API key** labels its field `New API key`, not the drawn `New device name`.

**The canvas draws BOTH strings, in two different frames.** From the `get_metadata` dump
of `116:28963`:

| | `177:74332` (left, x=0) | `177:74640` (right, x=1084) |
| --- | --- | --- |
| dialog frame | `177:74561` `dialog/device` 480x302 | `177:74869` `dialog/device` 480x302 |
| title | `177:74566` **Replace API key** | `177:74874` **Replace API key** |
| field 1 label | `177:74569` **Current API key** | `177:74877` **Current API key** |
| field 2 label | `177:74573` **New API key** | `177:74881` **New device name** |
| field 2 input | `177:74574` `Input`, no suffix glyph | `177:74941` `Input` + `177:74943` **`Icon / CircleX`** |

So the left frame draws exactly what the code ships. Only the right frame carries the
copy-paste slip - and that frame is *also* the one carrying a `CircleX` suffix glyph in
the field, i.e. it is a second STATE of the same dialog, not a second step.

(Layer names are Figma's auto-name from text content; both `PlaceholderText` nodes are
manually renamed, so those two need a render to read. Screenshot verification below.)

### Rendered, both frames (`get_screenshot` on `177:74561` and `177:74869`, 504x326 native)

**`177:74332` (left) - the EMPTY state.** Drawn strings, verbatim:

- Title: `Replace API key`
- Field 1 label: `Current API key`; value (read-only, grey): `sk-gw-661b17************************`
- Field 2 label: `New API key`; placeholder (grey): `Enter or paste your new key`
- Buttons: `Cancel` (outline) · `Replace key` (filled primary, drawn **washed out / disabled**)

**`177:74640` (right) - the FILLED state.** Drawn strings, verbatim:

- Title: `Replace API key`
- Field 1 label: `Current API key`; value (read-only, grey): `sk-gw-661b17************************`
- Field 2 label: `New device name`; value typed in: `sk-gw-216c63************************`,
  field drawn **focused** (blue ring) with a `CircleX` clear affordance at the right
- Buttons: `Cancel` (outline) · `Replace key` (filled primary, **fully saturated / enabled**)

### Verdict on the exception

`CLAUDE.md`'s sentence is **inaccurate as written**, though the code is right.

1. The file does not draw one label; it draws two. **The empty-state frame draws
   `New API key` - exactly what the code ships.** No override is required to match it.
2. Only the filled-state frame (`177:74640`) draws `New device name`, and it is
   unambiguously the copy-paste slip: same frame, same 480x302 `dialog/device`, same
   `Icon / KeyRound`, title still `Replace API key`, first field still `Current API key`,
   and the *value typed into the field* is `sk-gw-216c63...`, a key, not a device name.
   The section label above the pair (`191:80083` `Settings / Update device name`) is the
   same slip at the section level.
3. The "newest node wins" tiebreak points the **wrong way** here: `177:74640` > `177:74332`,
   so the newer frame is the one with the slip. That is worth knowing, because a future
   agent applying that rule mechanically would "fix" the code to `New device name`.

**Recommended edit to `CLAUDE.md`** (a wording correction, not a reversal - do not change
the code): say that the empty-state frame `177:74332` draws `New API key` and the code
matches it; the `New device name` in `177:74640` and in the section label `191:80083` is a
duplicated-section slip. That turns a "we deliberately disobey the file" exception into a
"the file agrees with us in the frame that matters" note, which is a much cheaper thing to
defend.

### Bonus state the pair reveals

The two frames are **one dialog in two states, not two steps**: empty + disabled primary,
then filled + focused field + clear glyph + enabled primary. That is a drawn
enable/disable rule for the Replace-key primary button. Checked against the code below.

---

## The three pairs are ALL one dialog in two states, not sequential steps

Settled by structural diff of the metadata subtrees plus a render of each. Every pair is
`empty / invalid -> filled / valid`, and the only thing that changes is the primary
button's enabled-ness (plus, on the two input dialogs, a clear glyph in the field):

| pair | frame A (left) | frame B (right) | structural diff |
| --- | --- | --- | --- |
| Rename device `143:67141` / `143:67481` | field empty, `Rename device` primary washed out | field filled, focused, `Icon / CircleX` clear glyph, primary saturated | B adds exactly one node: `Icon / CircleX` in the second `Input`. Nothing else differs. |
| Replace API key `177:74332` / `177:74640` | placeholder only, primary washed out | key typed, focused, `CircleX` (`177:74943`), primary saturated | same one-node diff, **plus** the `New API key` -> `New device name` label slip |
| Reset `177:73649` / `177:73994` | checkbox unchecked, `Reset Gate Connect` washed out | checkbox checked, primary saturated | **byte-identical node trees** (diff of both subtrees with ids/coords stripped is empty). The difference is purely the checkbox fill and the button fill. |

So the canvas draws **one flow rule three times**: the destructive/committing primary is
disabled until the dialog's precondition is met. There is no second step, no success
screen and no in-flight screen anywhere on this canvas.

Drawn strings for the two flows that are not the API key:

**Rename device (`143:67451`)**: title `Rename your device`, field 1 `Current device name`,
field 2 `New device name`, buttons `Cancel` · `Rename device`.

**Reset (`177:73944`)**: title `Reset Gate Connect`, subtitle
`This removes Gate Connect setup from this device.`, section heading `What happens next:`,
steps `1 Routing turns off` / `Managed tools return to their saved pre_gate configurations.`,
`2 Tools disconnect` / `No app on this device remains protected by Gate.`,
`3 Account and keys are removed` /
`Your local sign-in, organization, and keychain credentials are cleared.`, checkbox
`I understand that setup will restart on this device`, buttons `Cancel` · `Reset Gate Connect`.

---

## The disconnect exception, answered with quoted strings

`CLAUDE.md`:

> **Disconnect Gate?** says the session ends and configs are kept, not the drawn sentence
> about the keychain, which describes Reset.

**Accurate.** `143:70315` -> `143:70617` `card/organization`, 480x198, renders:

- Title `143:70623`: **`Disconnect Gate?`**
- Body `164:73502`: **`Protection turns off, your apps stop routing through Gate, and your
  API key is removed from the keychain.`**
- Buttons: `143:70627` **`Cancel`** (outline) · `143:70628` **`Yes, disconnect Gate`**
  (filled destructive)

The drawn body does claim keychain removal, and step 3 of the *Reset* dialog
(`177:73975`) makes the same claim in almost the same words
(`Your local sign-in, organization, and keychain credentials are cleared.`), which is the
evidence that the sentence was pasted from Reset. So the exception is correctly stated and
should stay exactly as written.

**One thing nobody has recorded: there is a hidden alternate subtitle in this dialog.**
`143:70624` is `Your next requests will use Constellation Gate PAYG credits`, sitting
directly under the title, marked `hidden="true"` in the metadata. It is a deliberately-off
layer, not a missing one - but it is the designer's other idea for what this dialog should
say, and it is about billing, not the keychain. Worth knowing before anyone rewrites the
disconnect copy; not a defect and not a third exception.

Note also the button labels: the disconnect primary is drawn `Yes, disconnect Gate`, not
`Disconnect`. Checked against the code below.

---

## 1. The disabled-until-valid rule, checked against the code

The canvas draws the rule three times. **The code implements it in all three, plus a
fourth place the canvas does not draw.**

`Modal`'s contract (`src/components/gc/Modal.tsx`) supports it directly:

- `ModalButton.disabled?: boolean` (line 32), with a comment that names the reset
  dialog as the reason it exists: *"Refused, not hidden ... a button that vanishes
  tells the user less than one that stays put and explains itself by staying dim."*
- The primary renders `onClick={primary.disabled ? undefined : primary.onClick}`,
  `aria-disabled={primary.disabled || undefined}` and
  `primary.disabled ? "cursor-not-allowed opacity-45" : ""` (lines 278-291).

Two notes on that rendering. It is `aria-disabled` on a live `<button>`, not the native
`disabled` attribute, so the button stays focusable and stays in the tab order - which is
the accessible-name-still-announced behaviour you want for a *refused* control, and it is
inert because `onClick` is `undefined`, so a keyboard Enter/Space does nothing. And
`opacity-45` is what produces the "washed out" primary the two empty-state frames draw. No
`type="submit"` and no Enter-key handler exists anywhere in `Modal`, so there is no path
that submits around the refusal.

Per dialog:

| dialog | code | precondition in code | matches drawn? |
| --- | --- | --- | --- |
| `RenameDeviceDialog` (`dialogs.tsx:1109`) | `primary={{ label: "Rename device", disabled: !newName.trim() }}` | field non-empty after trim | **yes** |
| `ReplaceApiKeyDialog` (`dialogs.tsx:1161`) | `primary={{ label: "Replace key", disabled: !newKey.trim() }}` | field non-empty after trim | **yes** |
| `ResetGateConnectDialog` (`dialogs.tsx:1264`) | `primary={{ label: "Reset Gate Connect", destructive: true, disabled: !acknowledged }}` | checkbox ticked | **yes** |
| `DisconnectGateDialog` (`dialogs.tsx:1219`) | `primary={{ label: "Yes, disconnect Gate", destructive: true }}` - no `disabled` | none | **n/a**, and correct: `143:70617` draws no field and no checkbox, so there is no precondition to gate on. The dialog is a single state, not a pair. |

`.trim()` is *stricter* than the drawn rule: a field holding only spaces reads as filled on
the canvas (a filled field is drawn filled) and reads as empty to the code. That is the
right direction to err and is not a defect.

The hook re-checks the same precondition server-side of the button
(`useSettingsActions.ts`): `replaceKey` bails on `if (!base || !key || busy) return;`,
`renameDevice` on `if (!name) return;`, `confirmReset` on
`if (prompt?.kind !== "reset" || !prompt.acknowledged || busy) return;`. So the rule is
enforced twice, and the second enforcement also covers the double-submit case the button
does not (see section 4).

### The one divergence: Rename never reaches its drawn empty state

`openRenameDevice` (`useSettingsActions.ts:305`) does
`setPrompt({ kind: "rename-device", currentName }); setNewDeviceName(currentName);` - it
**prefills the New device name field with the current name**, with the documented reason
that "the commonest rename is a small change to what is already there."

So `143:67141`, the drawn empty state with the washed-out `Rename device` primary, is
**not reachable by opening the dialog**. The dialog opens already in the `143:67481`
state: field filled, `CircleX` clear glyph present (`ModalField` renders it on
`onChange && value`), primary saturated. The empty state is only reachable by clearing the
field, which is what the drawn `CircleX` is for.

Two consequences worth flagging, neither of them a bug in the disable rule itself:

- The drawn pair reads as "type a name, then the button lights up". The shipped dialog
  reads as "the button is already lit, holding the name you already have."
- Nothing refuses a **no-op rename**: with the field left at `currentName`, the primary is
  enabled and `setDeviceName(currentName)` is sent. Compare `SwitchOrganizationDialog`
  (`dialogs.tsx:66`), which explicitly refuses the no-op with
  `disabled: currentId !== undefined && selectedId === currentId` and a comment saying
  "the drawn dialog mutes it ... confirming a no-op switch would fire the whole switch
  sequence to change nothing." The same argument applies to rename and is not applied.
  I could not determine whether any frame draws the prefilled-but-unchanged state, so I am
  raising this as a code-consistency question, not as a design mismatch.

---

## 2. Every drawn button label against the code

All four dialogs' button labels match verbatim. Nothing to fix here.

| dialog | drawn secondary | code | drawn primary | code |
| --- | --- | --- | --- | --- |
| Rename device `143:67451` | `Cancel` | `Cancel` | `Rename device` | `Rename device` |
| Replace API key `177:74561` | `Cancel` | `Cancel` | `Replace key` | `Replace key` |
| Disconnect Gate? `143:70617` | `Cancel` | `Cancel` | **`Yes, disconnect Gate`** | **`Yes, disconnect Gate`** |
| Reset Gate Connect `177:73944` | `Cancel` | `Cancel` | `Reset Gate Connect` | `Reset Gate Connect` |

The disconnect primary specifically: drawn `Yes, disconnect Gate`, shipped
`Yes, disconnect Gate`. It is **not** shortened to `Disconnect` anywhere in the code.

Titles, subtitles and the reset body all match verbatim too, which is worth recording
since the two named copy exceptions make it easy to assume the copy drifts generally:

- `Rename your device` / `Current device name` / `New device name` - all match.
- `Replace API key` / `Current API key` - match. `New API key` matches the empty-state
  frame `177:74332` (see the first finding).
- `Reset Gate Connect` / `This removes Gate Connect setup from this device.` /
  `What happens next:` / all three step titles and descriptions / the checkbox label
  `I understand that setup will restart on this device` - all match verbatim.

### One label mismatch nobody has recorded: the API-key placeholder

`177:74332` draws the empty New-API-key field with placeholder text
**`Enter or paste your new key`**. The code ships `placeholder="sk-gw..."`
(`dialogs.tsx`, `ReplaceApiKeyDialog`'s second `ModalField`).

This is a genuine, unrecorded divergence from the drawn copy, and it is not covered by the
standing `New API key` exception - that exception is about the *label*, and this is the
*placeholder*. It is also a substantive difference: the drawn string is an instruction
("enter or paste"), the shipped one is a format hint. The design contract says the file
wins on copy. Either restore `Enter or paste your new key`, or add this to the exception
list with a reason. I am not proposing which; it is a decision, not a bug.

For symmetry: the Rename dialog's New-device-name field has **no** placeholder in the code
and needs none, since it opens prefilled.

---

## 3. Row inventory: `116:28970` / `130:48905` vs `SettingsPane.tsx`

**First: the two frames draw the same settings list.** I diffed both metadata subtrees.
`116:28970` -> `121:32874` -> ... -> `116:28977` `settings-list` and `130:48905` ->
`362:8424` -> ... -> `362:8497` `settings-list` are the same seven sections, the same
fifteen rows, the same labels, the same values, the same button/switch placement, row for
row. The **only** difference between the two frames is the topsection: `116:28970` is
144px tall and carries `121:32846` `banner/update` ("Update available - v0.5.0") above the
status banner; `130:48905` is 96px and has no update banner. So this pair is a
*chrome* variant, not a settings-content variant, and neither frame draws a row the other
does not. Nothing below needs to distinguish them.

### What the frames draw

| section | row | value drawn | control drawn |
| --- | --- | --- | --- |
| Device | `Device` | `MacBook Pro` | Button (110w) |
| Device | `Install ID` | `gc_a1b2c3d4` | Button (68w) |
| Account | `Login ID` | `jdoe@acme.com` | **none** |
| Account | `Gate plan` | `Free` | Button (122w) |
| Connection | `Gateway` | `Managed by Gate` | Button (106w) |
| Connection | `API key` | `sk-gw-661b17************************` | Button (93w) |
| Connection | `Active session` | `sk-gw************************` | Button (117w) |
| Startup | `Launch at login` / `Keeps routing on after restart` | - | `On` + Switch |
| Startup | `Notifications` / `Alert me when a request is blocked or flagged` | - | `On` + Switch |
| Diagnostics | `Share diagnostic data` / `Send Gate errors and routing stats to help fix problems. Never prompts or credentials.` | - | `On` + Switch |
| Diagnostics | `Diagnostics report` / `Everything Gate knows about this install, as shareable text.` | - | Button (89w) |
| About | `Tutorial` | `v0.1.4` | Button (107w) |
| About | `Version` | `v0.1.4` | Button (128w) |
| Danger zone | `Reset Gate Connect` / `Turn routing off, disconnect tools, remove this account or key, and start setup again.` | - | Button (97w) |

### Drawn rows with no code path

Only one, and it is a *control*, not a whole row:

- **`Gate plan` -> the Button (`143:68350` / `362:8535`, 122px, the width of "Upgrade
  plan") is never reachable.** `buildSettingsSections` gates it on `onUpgradePlan`
  (`SettingsPane.tsx:185`), and `NewUiApp.tsx` never passes it - the comment at
  `NewUiApp.tsx:1569` says so explicitly: *"Deliberately absent, so the control is absent
  too: plan upgrade has no billing URL to open."* The row itself renders; its drawn button
  does not. This is a known, reasoned omission, not a miss.

Every other drawn row has a code path. Note the Account section's value copy diverges,
though, in two places that are worth a designer's eye:

- **`Login ID` draws an email (`jdoe@acme.com`); the code passes `account?.org_name`**
  (`NewUiApp.tsx`), i.e. the *organization* name, falling back to `-`. A row labelled
  "Login ID" showing an org name is not what the frame promises. I did not trace whether
  an email is available on `Account` at all, so I am flagging the divergence, not
  proposing the fix.
- **`Gate plan` draws `Free`; the code passes the literal `plan: "-"`**, with the comment
  "Device name and plan have no backend yet, so they read as unknown rather than as
  invented values." That is the right call under the "a figure is a measurement" principle
  and needs no change - but it does mean the Account section ships with two of its two
  values reading as placeholders on a fresh install.
- **`Active session` draws a value (`sk-gw************************`); the code's `session`
  row has no `value` at all** - label plus a destructive `Disconnect Gate` action, nothing
  else. The drawn value is also a masked *key*, on the row that ends an *OAuth session*,
  which is the same category of slip as the `New device name` label in the Replace-key
  dialog. The code's omission looks deliberate and correct; recording it so nobody
  "restores" the drawn string.
- **`Tutorial` draws a value `v0.1.4` (`116:30083` / `362:8625`)** - the same string as the
  Version row directly below it. The code's `tutorial` row carries no value. This is
  plainly a copy-paste from the Version row and not a real requirement; the code is right.
  Recording it as a third instance of the file duplicating a row and forgetting to change
  the pasted content (the others being `New device name` in the Replace-key dialog and the
  `Settings / Update device name` section label above it). That is now a *pattern* in this
  file, which is a more useful thing to tell the designer than three separate bug reports.

### Code rows nothing draws

| row | section | condition | drawn? |
| --- | --- | --- | --- |
| `sign-in-method` (`Sign-in method` / `Gate account` or `API key`, with a `Use a Gate account` action) | Connection | always, in one of two shapes | **no** - neither frame has this row |
| `certificate` (`Gate certificate` / `Trusted` \| `Not trusted`, with a destructive `Remove certificate`) | Connection | `certificate !== undefined`, i.e. wherever there is a proxy subsystem | **no** |
| `docs` (`Documentation` / `Setup, routing, and troubleshooting`, `Read docs`) plus its whole `Help` **section** | Help | `onOpenDocs` is always passed (`NewUiApp.tsx:1566`), so this section always renders | **no** - the frames have no Help section at all |

All three are user-visible on a normal install. The Connection section as shipped can be
five rows deep (Gateway, API key **or** Sign-in method, Sign-in method, Gate certificate,
Active session) against the three the file draws, and there is an eighth section (Help)
between About and Danger zone that the file does not have.

`Gate certificate` in particular carries the screen's **third destructive action**, which
`CLAUDE.md` says to question when a third appears. The code already questions and keeps it
in a comment on the row. Undrawn plus destructive is the combination most worth a
designer's confirmation, so it belongs in the questions doc if it is not there already.

The `Startup` section's one-Notifications-row-vs-four question and the `Diagnostics`
section are excluded here as instructed: the first is already with the designer, the
second is drawn and matches.

---

## 4. In-flight and success: what the code does where the canvas draws nothing

The canvas draws two states per flow (empty/invalid, filled/valid) and stops. Both of the
states *after* the primary is pressed are undrawn and both are user-reachable, so both are
in scope here.

### In flight: the hook has the state, and three of the four dialogs never receive it

`useSettingsActions` holds `busy: boolean`, and its own interface comment says what it is
for:

> `/** An action is in flight. Dialog primaries read this to avoid a double submit. */`

**They do not.** Grepping every render site in `NewUiApp.tsx`, `settings.busy` is passed
to exactly one dialog:

- `SwitchGatewayDialog` gets `busy={settings.busy}` (`NewUiApp.tsx:2140`) and uses it
  properly - `label: busy ? "Switching..." : "Switch and relaunch"` and
  `disabled: busy || selectedUrl === currentUrl` (`dialogs.tsx:157-161`).

Every one of the four Settings dialogs in scope is rendered without it:

| dialog | render site | receives `busy`? | in-flight affordance |
| --- | --- | --- | --- |
| `ReplaceApiKeyDialog` | `NewUiApp.tsx:2108` | no | **none** |
| `RenameDeviceDialog` | `NewUiApp.tsx:2128` | no | **none** |
| `DisconnectGateDialog` | `NewUiApp.tsx:2151` | no | **none** |
| `ResetGateConnectDialog` | `NewUiApp.tsx:2156` | no | **none** |

None of the four component signatures even accepts a `busy` prop, so this is not a missed
prop at one call site - the affordance was never built for them.

What the user sees, from the click until the dialog closes: the primary stays fully
saturated, keeps its original label, stays clickable, and nothing moves. Clicking it again
re-enters the handler, which returns immediately on `if (busy) return` - so there is no
double submit at the backend, but there is also no acknowledgement that the first click
landed. The double-submit protection is real; the *feedback* the comment implies is not.

This matters most for **Reset**, which is the slowest of the four by construction.
`confirmReset` (`useSettingsActions.ts`) runs `proxyDisable()` (stopping the engine and
unwinding system HTTPS), *then* `clearAccount()` (which "disconnects managed tools before
wiping anything") - two backend round-trips with a tool-disconnection sweep inside the
second. It is the longest-running action on the screen and the one with the least on-screen
evidence that it started. The precedent for fixing it is already in the file: `QuitDialog`,
`QuitSafeToCloseDialog` and `QuitLeftBehindDialog` all take `busy`, as does
`SwitchGatewayDialog`. The four Settings dialogs are the odd ones out.

### A real in-flight bug: the wrong sentence appears in the pane behind

`NewUiApp.tsx:1507`:

```
signInNote: settings.busy
  ? "Finish signing in on the page that opened in your browser."
  : undefined,
```

`signInNote` lands on the Connection section's `Sign-in method` row as its description
(`SettingsPane.tsx:311` and `:321`). But `busy` is a **single flag shared by every action
in the hook** - `replaceKey`, `renameDevice`, `confirmDisconnect`, `confirmReset`,
`confirmSwitchOrg`, `openSwitchOrg`, `confirmSwitchGateway` and `upgradeToOAuth` all set
it. Only the last of those opens a browser.

So while a rename, a key replacement, a disconnect or a reset is running, the Settings pane
asserts that the user should go finish signing in in their browser. No browser was opened.

Most of the time this sits under the dialog's scrim, which softens it but does not fix it -
`Modal` renders `absolute inset-0 z-20 ... bg-neutral-900/40` inside `AppShell`'s
`relative` root, which is documented as covering "the window including its chrome", and the
480px dialog leaves the pane's rows visible around it at 60% opacity. But **`openSwitchOrg`
sets `busy` with no dialog open at all** - it is the `oauthListOrgs()` fetch that runs
*before* `SwitchOrganizationDialog` appears, and it returns without opening anything when
`orgs.length < 2`. On a single-org account, clicking Switch organization in the sidebar
therefore flashes "Finish signing in on the page that opened in your browser." onto the
Sign-in method row, undimmed, and then nothing else happens.

The fix is a separate flag for the OAuth-upgrade flow, or a discriminated
`busy: { action: ... } | null`. I have not proposed a patch; this is a review.

### A second, smaller layering consequence

On failure all four flows keep the dialog open and route the error to
`setActionError(classifyError(e, "generic"))` (`NewUiApp.tsx:1330`), which renders an
`ErrorBanner` in `AppShell`'s `notice` slot - i.e. **above** the pane and **under** the
modal scrim. The banner is legible through the 40% scrim, but it is outside the dialog's
focus trap and its `onDismiss` button sits beneath a full-window overlay div, so it cannot
be dismissed until the dialog is closed. Also worth noting: the banner is the *only* place
a failed rename or key replacement is reported. Nothing appears inside the dialog itself,
which is where the user is looking and where the field they need to correct lives. The
hook's comments say the dialog is deliberately left open "so the user can correct it" - but
what needs correcting is announced somewhere else on screen.

### Success: no success screen, and that is mostly right

There is no success state for any of the four. On success each one calls `setPrompt(null)`
and the dialog simply disappears. The confirmation is the pane changing underneath:

| flow | what confirms it | good enough? |
| --- | --- | --- |
| Rename device | `onDeviceName(await fetchDeviceName())` re-reads the resolved name, so the Device row's value changes | yes - the changed value is the receipt |
| Replace API key | `onAccount(await getAccount())`, and an effect at `NewUiApp.tsx:817` re-reads `keyPrefix` on every account change, so the API key row's mask changes | yes, with a caveat below |
| Disconnect Gate | `onSession({account, oauth})`; `onDisconnect` is withheld once `auth_mode !== "oauth"`, so the Active session row disappears entirely | yes - a row vanishing is unambiguous |
| Reset Gate Connect | `onSession({ account: null, oauth: null })`; the derived setup stage drops the whole window to sign-in | yes, and emphatically |

The caveat on Replace API key: the mask update is **two** awaits deep. `getAccount()`
resolves and sets `account`, which fires the `keyPrefix` effect, which awaits
`getAccountKeyPrefix()`. Between the dialog closing and that second read landing, the API
key row still shows the **old** masked key. It is brief and self-correcting, but it is the
one moment where the screen shows a stale credential immediately after the user replaced
it, on the screen whose whole job is telling the truth about credentials. Worth a look; not
worth blocking on.

The one flow on this screen that *does* get a success screen is the org switch:
`confirmSwitchOrg` ends on `setPrompt({ kind: "org-switched", name })`, rendering
`OrganizationSwitchedDialog`, with the reasoning that "the org decides what gets billed and
what the gateway rejects requests without, so 'did that work?' is a question worth
answering." That reasoning applies least to rename (the value is right there) and most to
**disconnect** and **reset**, both of which change the app's whole posture and both of
which currently just close. I am not recommending adding two more dialogs - the pane
changes visibly in both cases - but it is the asymmetry a designer should be asked about,
since the canvas draws no opinion either way.

---

## 5. Dead ends

Things that exist in the code but no user can reach, or that a comment claims wrongly.

1. **The `Upgrade plan` button is drawn and unreachable.** Covered in section 3.
   `onUpgradePlan` is declared on `buildSettingsSections` (`SettingsPane.tsx:108,185`) and
   passed by nobody. Deliberate and commented.
2. **The `Support` row is unreachable.** `onContactSupport` is declared
   (`SettingsPane.tsx:126,214`) and never passed, so the `support` row never renders. The
   comment at `NewUiApp.tsx:1567` says why: `GATE_SUPPORT_URL` 404s (AG-598). Deliberate.
   Note that this is why the `Help` section currently renders with exactly one row.
3. **`CollectedDataDialog`'s docstring is stale, and says the opposite of the truth.**
   `dialogs.tsx:1345` opens with, in bold, *"**Nothing renders this today.** Its Settings
   row was removed on 2026-08-27 for being undrawn, which took AG-603's only surface with
   it."* That is no longer accurate. The dialog **is** reachable: `SettingsPane.tsx:490`
   attaches a `descriptionLink` labelled `See what is collected` to the
   Share-diagnostic-data row's description whenever `onViewCollectedData` is supplied;
   `NewUiApp.tsx:1578` supplies it (`() => setCollectedDataOpen(true)`);
   `NewUiApp.tsx:2098` renders `<CollectedDataDialog>` on that state; and
   `SettingsPane.tsx:682-690` renders the link. The docstring's *own third paragraph*
   describes exactly this door ("Opened from a link inside the share-diagnostics row's own
   description"), so the comment contradicts itself - the first paragraph was simply never
   deleted when the link landed. **Only a comment needs fixing here; the code is right.**
   Recording it because a stale "nothing renders this" is the kind of note that gets a
   working feature deleted in a later cleanup pass.
4. **Not a dead end, checked and cleared:** `settings.newKey` / `settings.newDeviceName`
   are both cleared by `dismissPrompt`, so a cancelled draft cannot leak into the next
   dialog. The hook's comment claims this and the code does it.

### One consistency note that belongs to the visual review, flagged here for the pointer

`ResetGateConnectDialog` passes `tone="danger"` but **not** `edge="danger"`, while
`DisconnectGateDialog` passes both. `edge` is what swaps the panel border from
`base-border` to `border-base-destructive/40` (`Modal.tsx:194-197`). Reset also omits the
`tile="sm"` that all three of its 480px siblings carry. So the most destructive dialog in
the app draws the softer edge and the larger tone tile. Whether the file draws it that way
is a token question for `docs/review-figma-settings.md`, not a flow question, so I am not
adjudicating it here - only noting that the two danger dialogs are configured differently
and that difference is not explained by a comment the way every other divergence in this
file is.

---

## Could not determine

Honest gaps, so nobody reads a silence as a clearance.

1. **Whether any frame draws the Rename dialog in its shipped opening state.** The code
   opens it prefilled with the current name (section 1). `143:67141` draws it empty and
   `143:67481` draws it with a *different* name typed in. Neither is "prefilled and
   unchanged". If a third frame exists elsewhere in the file I did not find it - I only
   enumerated canvas `116:28963`.
2. **Whether `Sign-in method`, `Gate certificate` or a `Help` section are drawn anywhere
   else in the file.** Section 3's "nothing draws these" claim is scoped exactly to
   `116:28970` and `130:48905`, the two frames the task named, where I read the full
   metadata subtree. I did not sweep the other canvases, so the correct statement is "the
   Settings frames on this canvas do not draw them", not "the design does not have them".
3. **Whether an email address is available to fill the `Login ID` row.** The code passes
   `account?.org_name`. I did not open `lib/api.ts`'s `Account` type to see whether an
   email field exists, so I cannot say whether the drawn `jdoe@acme.com` is unimplemented
   or unimplementable.
4. **Whether `OrganizationSwitchedDialog` has a drawn frame.** `Modal.tsx`'s width comment
   cites `130:55314` / `134:61659` / `130:48278` for the 512px confirmations, which are not
   on this canvas. I did not verify that any of those is the org-switched success screen,
   so section 4's remark that the org switch is "the one flow with a success screen" is a
   statement about the **code**, not a claim that the file draws one.
5. **Rendering.** I read `116:28970` and `130:48905` from `get_metadata` only and did not
   take screenshots of them - the row inventory is a structural comparison of layer names
   and text content, which is what the question asked for. Every value string quoted in
   section 3 is a Figma layer name, and Figma auto-names text layers from their content, so
   those are reliable; but I have not visually confirmed, for example, which of the drawn
   buttons are outline and which are filled. If that matters for a fix, it needs a render.
