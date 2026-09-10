# Flow validation: `Flows / App` (116:30199) and `Flows / Settings` (116:28963)

File `9FrccCojXy0f8QD8Wm5Lln`. This is a **states-and-sequence** audit, not a pixel
audit: the visual/token findings for these two canvases are already in
`docs/review-figma-settings.md` and `docs/review-figma-dialogs-app.md` and are not
repeated. Open copy questions already with the designer are in
`docs/figma-questions-for-design.md` and are not re-raised.

Code under audit (read-only): `src/NewUiApp.tsx` (owns routing writes, the model
sequence and the Settings dialog stack), `src/lib/useSettingsActions.ts`,
`src/components/gc/dialogs.tsx`, `src/components/gc/Modal.tsx`,
`src/components/gc/AppPane.tsx`, `src/components/gc/SettingsPane.tsx`,
`src/lib/groups.ts`.

## Frame lists verified

`get_metadata` on both page nodes returns exactly the enumerated top-level frames and
no others: 15 on `116:30199`, 11 on `116:28963`. Both given lists are complete.

(work in progress - sections appended as measured)

## The canvas tells you the flow: section labels and annotations

Neither canvas is a flat list of screens. Both are divided by `line` dividers into
labelled sections, and the frame names (`App/<tool>/<banner>/<choice>`) are *within*
section, not across it. Read by position:

**`Flows / App` (116:30199)** - six sections:

| section label | frames, left to right | captions / annotations |
| --- | --- | --- |
| `App / Dimensions` (`154:71467`) | `154:70994` | `154:71469` "App dimensions: 1024x720px" plus the 720px/1024px callout lines |
| `App / Table guide` (`272:3260`) | `661:15923` | - |
| `App / Main screens` (`130:56395`) | `116:30663`, `116:30204`, `116:31920` | `121:35814` "App w/ Gate model selected + update available + alert", `130:56400` "...+ update available", `130:56401` "App w/ Gate model selected" |
| `App / Select a model` (`130:56398`) | `121:35174`, `127:43501`, `130:55863`, `139:66117` | `130:56394` "App w/ App default selected", `130:56976` "App w/ model switch modal open", `121:35816` "App w/ Gate model selected", `139:66782` "When the user selects a model it automatically closes the modal and applies their selection." |
| `App / Select multiple models (Opencode)` (`671:19286`) | `662:17579`, `665:18498`, `671:19431` | `671:19288` "When the user selects a model (multiple) the modal stays open until the user confirms their selections. Then the modal will close", `671:19247` "Current models will sort alphabetically, left to right using their provider. Example. Anthropic > DeepSeek > Moonshot" |
| `App / No data 1+ day state` (`228:89770`) | `408:25991`, `228:89241`, `272:1623` | - |

Two consequences for the brief:

- **`154:70994` is a dimensions artboard, not a fifth alert state.** It sits alone under
  `App / Dimensions` with the same 720px/1024px measuring lines that `191:79795` has on
  the Settings canvas. So the five `alert/gate-model` frames are five frames but only
  **two** alert causes and one mis-named result screen (below).
- **The `App / No data 1+ day state` row is three re-renders of screens already drawn
  above**, with the readings emptied. It adds no step.

**`Flows / Settings` (116:28963)** - six sections:

| section label | frames | note |
| --- | --- | --- |
| `Settings / Dimensions` (`191:80020`) | `191:79795` | annotation artboard, confirmed and set aside |
| `Settings / Main screens` (`191:80077`) | `116:28970`, `130:48905` | captions `135:63165` "Settings w/ update available", `135:63167` "Settings w/ no updates available" |
| `Settings / Update device name` (`191:80080`) | `143:67141`, `143:67481` | |
| `Settings / Update device name` (`191:80083`) | `177:74332`, `177:74640` | **the label is a copy-paste slip**: these are the two API-key frames |
| `Settings / Disconnect Gate session` (`191:80086`) | `143:70315` | |
| `Settings / Reset Gate Connect` (`191:80089`) | `177:73649`, `177:73994` | |
| `Settings / Diagnostics` (`363:9124`) | `362:8700` | |

The duplicated `Update device name` label over the API-key pair is the origin of the
field-label problem `CLAUDE.md` records as a standing exception: the designer duplicated
the device-name section and relabelled some of it. See the API-key row in the inventory
for what each of the two frames actually draws - it is not what `CLAUDE.md` says.
