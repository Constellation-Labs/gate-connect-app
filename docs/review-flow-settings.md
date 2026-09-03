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
