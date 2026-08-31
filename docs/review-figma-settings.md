# Figma audit: `Flows / Settings` (116:28963)

Read 2026-08-30 against branch `review/figma-audit`.

## Summary

**The Settings page has not been redrawn since the 2026-08-26 pass.** The
highest node-id generation anywhere on the page is `440:*` (the sidebar
instances, now 256px wide, already absorbed by commit 77589b28); there is
nothing in the `69x:*` / `7xx:*` / `8xx:*` range that marks the Tray and
Overview redraws. Section list, row order, labels, values, icons, button copy,
switch positions, section gap (24), heading gap (12), card padding (16) and
row rhythm (16 + 1px rule inside the padding) on `116:28972` all still match
`SettingsPane.tsx` exactly. The 2026-08-26 pass got the main screen right.

What it **missed** is the dialogs' chrome. That pass recorded a tone-tile rule
of "44px/24px glyph on a toned dialog, 40px/20px on a neutral one"; the
Settings frames do not draw that. The three 480px dialogs (Rename, Replace API
key, **Disconnect**) draw a **32px tile with a 16px glyph**, and the 600px
Diagnostics report - a *neutral* dialog - draws **44px with a 24px glyph**. Tile
size tracks dialog width here, not tone, and the build gets three of the five
drawn Settings dialogs wrong. Two dialog glyphs are also still the pre-2026-08-26
near-misses that were fixed on the pane rows but never followed into
`dialogs.tsx`, and the diagnostics report body renders two type steps below the
drawn `mono/body-14`. Three fresh design bugs surfaced from the node data
(a stray `Last 24 hours` caption, and two copy-pasted values on rows that have
none); the build is already right on all three.

Nothing was unreadable. 3 Figma calls used (1 metadata + 2 screenshots).

## Findings

| What | Figma (node id + drawn value) | Built | Verdict |
| --- | --- | --- | --- |
| Diagnostics report body type | `363:9120` - `mono/body-14`, 15 lines in a 300px block = **14/20** | `dialogs.tsx:494` `font-mono text-base-xs leading-4` = **12/16** | **DRIFT** |
| Rename-device dialog glyph | `143:70310` / `177:79221` `Icon / Monitor` | `dialogs.tsx:976` `icon="monitorSmartphone"` | **DRIFT** |
| Diagnostics dialog glyph | `363:9030` `Icon / ClipboardList` | `dialogs.tsx:486` `icon="info"` - and `SettingsPane.tsx:447` already uses `clipboardList` for the row that opens it | **DRIFT** |
| Disconnect dialog tone tile | `143:70620` wrapper **32x32**, `143:70621` glyph **16x16** (screenshot confirms the small tile) | `tone="danger"` → `Modal.tsx:193,196` **44px / 24px** | **DRIFT** |
| Rename + Replace-key tone tiles | `143:70309`, `177:74563`, `177:74871` wrapper **32x32**, glyph **16x16** | neutral default → `Modal.tsx:192,196` **40px / 20px** | **DRIFT** |
| Diagnostics report tone tile | `363:9029` wrapper **44x44**, `363:9030` glyph **24x24**, neutral fill | neutral default → **40px / 20px** | **DRIFT** |
| Dialog body block gap | 16px: rename fields `143:67460` (y0,h60) → `143:67465` (y76); reset `177:73952` (h234) → checkbox `177:73976` (y250) | `Modal.tsx:223` `mt-6 flex flex-col gap-3` = **12px** | **DRIFT** |
| Reset dialog step number tile | `177:73957` **36x36**, text-group at x=48 (36 + 12) | `Modal.tsx:599` `size-8` = **32px** | **DRIFT** |
| Settings row value column | value text at **x=233** on every value row, both frames (`116:28991`, `116:28999`, `127:44726`, `127:44733`, `127:44762`, `143:68381`, `116:30093`, and the duplicate `191:79874`) | `SettingsPane.tsx:609` `w-[184px]`: 20 icon + 12 + 184 + 12 = **228** | **DRIFT** (5px) |
| Device row second line | `116:28986` draws label + value + button, **no description** | `SettingsPane.tsx:221-226` adds "Sent with this device's traffic…" / "Not sent. Name this device…" | **DRIFT** - deliberate (commit 58364f28, `x-gate-device-name`) but **not recorded** in `plans/new-app-ui-figma.md`; record it or drop it |
| Settings page header caption | `116:28976` / `191:79863` `Last 24 hours`, right-aligned beside the title | Header is the `Settings` h1 alone (`SettingsPane.tsx:548`) | **DESIGN BUG** - a time range on a page with no time-ranged figure; carried over from the Overview header (plan L618). Build is right. |
| About / Tutorial row value | `116:30083` value `v0.1.4` on the **Tutorial** row | `SettingsPane.tsx:459-463` - no value | **DESIGN BUG** - copy-pasted from the Version row directly below (`116:30093`, same string). The 2026-08-21 transcription recorded this row's value as "-"; the text node is present and not hidden. Build is right. |
| Connection / Active session value | `127:44778` value `sk-gw************************` | `SettingsPane.tsx:346-353` - no value, button only | **DESIGN BUG** - duplicates the API key row's mask two rows up. Same story as Tutorial: transcribed as "-" in 2026-08-21, actually drawn. Build is right. |
| Replace API key, second field label | `177:74881` `New device name` (the sibling frame `177:74573` says `New API key`) | `dialogs.tsx:1044` `New API key` | **RECORDED** - locked copy exception #1, CLAUDE.md. File disagrees with itself; the correct half wins. |
| Disconnect Gate? body | `164:73502` "Protection turns off, your apps stop routing through Gate, and your API key is removed from the keychain." | `dialogs.tsx:1095-1098` "This device signs out of Gate and stops sending activity. Your apps keep their current configuration…" | **RECORDED** - locked copy exception #2, CLAUDE.md. |
| Diagnostics dialog subtitle | `363:9033` "The state of this installed, as text you can hand to someone else" | `dialogs.tsx:489` "…this install…" | **RECORDED** - typo, correction noted at the call site. |
| Notifications description | `116:29086` "Alert me when a request is blocked or flagged" | `SettingsPane.tsx:391` verbatim | **OK** - the honest routing-health wording was reverted to the frame on 2026-08-26. |
| Sign-in method, Gate certificate, Help section, `See what is collected` link | Not drawn anywhere on `116:28963` | `SettingsPane.tsx:287-342, 476-512, 428-431` | **RECORDED** - standing undrawn deviations. |
| Gateway `Change server` button | `298:4624`, 106px, drawn unconditionally | gated on `onChangeGateway` (dev builds) | **RECORDED** - plan L384. |
| `SwitchGatewayDialog`, `OAuthOfferDialog`, `RestoreDetailsDialog` | No such frames on this page | Built | **RECORDED** - each docstring says the file draws none. |
| Section/row geometry: 24px section gap, 12px heading→card, 16px card padding, 16px row gap with the rule inside the padding, 24px pane padding, 24px header→list | `116:28977` and children | `SettingsPane.tsx:544,553,565-581` | **OK** |
| Dialog geometry: 480/544/600 widths, 24px padding, 24px header→body, 24px body→buttons, 36px buttons at a 12px gap | `143:67451`, `177:73944`, `363:9027` | `Modal.tsx:66-74,166,223,226` | **OK** |
| Every row icon, label, value and button label on the main screen | `116:28978` … `130:48891` | `buildSettingsSections` | **OK** |

## Ranked DRIFT, most severe first

1. **Diagnostics report body is 12/16 where the file draws 14/20** (`dialogs.tsx:494`).
   This is the one screen a user reads a wall of text on and hands to someone
   else; two steps down from the drawn size on a mono block is the most visible
   miss on the page.
2. **Rename dialog draws `MonitorSmartphone`** (`dialogs.tsx:976`) where the frame
   and its own Settings row both say `Monitor`. Same glyph, two different marks,
   one click apart.
3. **Diagnostics dialog draws `Info`** (`dialogs.tsx:486`) where the frame and its
   own Settings row say `ClipboardList`. Same problem, opposite direction.
4. **Tone-tile size follows width, not tone.** The rule in `Modal.tsx:121-129` is
   wrong for this page: Rename / Replace / **Disconnect** all draw 32px with a
   16px glyph (`tile="sm"` exists and is close, but its glyph is 20px, not 16),
   and the neutral Diagnostics report draws 44px with a 24px glyph. The
   Disconnect case is the loudest - the build shows a 44px red tile where the
   file shows a 32px one.
5. **Dialog body blocks stack at 12px against a drawn 16px** (`Modal.tsx:223`).
   Affects every Settings dialog with two children.
6. **Reset dialog's numbered tiles are 32px against a drawn 36px** (`Modal.tsx:599`).
7. **Value column starts 5px early** (`SettingsPane.tsx:609`, 228 vs 233).
8. **The Device row's second line is undrawn and unlogged.** Almost certainly a
   deliberate consequence of `x-gate-device-name`; it just needs an entry in
   `plans/new-app-ui-figma.md` next to the other standing deviations.

## Notes

- `191:79795` `Setting/dimensions` is a plain duplicate of the main frame, not an
  annotated spec; its sidebar instance is still 250px while every other Settings
  frame's is 256px. Cosmetic staleness in the file, no build consequence.
- CLAUDE.md still says "a persistent 250px sidebar". Every frame on this page now
  draws 256. Out of scope for this page, but the sentence is stale.
