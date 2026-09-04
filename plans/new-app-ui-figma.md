# New app UI: matching the Gate Connect Figma

Source: https://www.figma.com/design/9FrccCojXy0f8QD8Wm5Lln/Gate-Connect
File key `9FrccCojXy0f8QD8Wm5Lln`, single page `Components` (113:16762).

## What the Figma actually contains

The file has five pages, only one of which the MCP `get_metadata` call
reported: `Components`, `Flows` (containing `Overview`, `Settings`, `App`),
`Sandbox`, and `Icons`.

- **Overview** - dashboard: stat tiles, Messages bar chart, Policies table,
  Token savings table. Annotated **App dimensions: 1024x720px** - superseded by
  `overview-dimensions` (`864:3466`), a 1280x800 frame; see the sync below.
- **Settings** - fully transcribed below.
- **App** - a per-app detail view, two sections (`App / Main screens` and
  `App / Select a model`). Selecting an app in the sidebar opens its own pane
  with per-app stats, a Messages chart, a model picker (App default vs Gate
  model) and a Recent activity table. This is a third screen type beyond
  Overview and Settings.
- **Sandbox** - scratch. Layers are named `Screenshot 2026-05-14 at 1:26:15 PM`,
  `overview-dimensions`, `mask`, plus duplicates. **Not a spec source; ignore.**
- **Icons** - glyph library.

### Settings screen (transcribed from `Flows / Settings`)

Sections, each a card with 8px radius; rows are label + value + trailing action.

| Section | Row | Value | Action |
| --- | --- | --- | --- |
| Device | Device | MacBook Pro | Rename |
| | Install ID | `gc_a1b2c3d4` | Copy |
| Account | Login ID | jdoe@acme.com | - |
| | Gate plan | Free | Upgrade plan |
| Connection | Gateway | Managed by Gate | - |
| | API key | `sk-gw*****************` | Replace key |
| | Active session | - | **Disconnect** (destructive) |
| Startup | Launch at login<br>"Keeps routing on after restart" | On | switch |
| | Notifications<br>"Alert me when a request is blocked or flagged" | On | switch |
| About | Tutorial | - | Replay tutorial |
| | Version | v0.1.4 | Check for updates |
| Danger zone | Reset Gate Connect<br>"Turn routing off, disconnect tools, remove this account or key, and start setup again." | - | **Review reset** (destructive) |

"Danger zone" is a red section heading; its card carries a red tint and border.

### Components page

Only component/state frames.

| Node | Name | Size |
| --- | --- | --- |
| 113:16763 | `nav/topbar` | 1024x48 |
| 113:16775 | `banner/update` | 1024x48 |
| 113:16786 | `banner/routing` | 1024x48 |
| 113:16889 | `banner/partly-routing` | 1024x48 |
| 113:16898 | `banner/alert/multiple-apps` | 726x68 |
| 113:16919 | `banner/alert/single-app` | 726x68 |
| 113:16794 | `nav/sidebar/overview` | 250x573 |
| 121:33326 | `nav/sidebar/settings` | 250x573 |
| 121:33421 | `nav/sidebar/settings` (row hover) | 250x573 |
| 116:17428 | `topnav/menu` | 224x114 |

## Re-verification of the Flows pages, 2026-08-17

`Components` is unchanged. The three Flows pages have moved.

### Settings screen: four labels and one affordance changed

| Row | Was built as | Now reads |
| --- | --- | --- |
| Device | `Rename` | `Rename device` |
| Install ID | `Copy` | `Copy ID` |
| Gate plan | `Upgrade plan` | `Upgrade plan` + external-link glyph |
| Active session | `Disconnect` | `Disconnect Gate` |

Applied. The API key value also now shows a prefix rather than a full mask -
`sk-gw-661b17***…` instead of `sk-gw***…`. That is the shell's string to build,
not the pane's, and it is a decision about how much of a key to reveal.

### Four new Settings dialogs, all marked ready

- **Rename your device** - form: `Current device name` (readonly) over
  `New device name` (focused, with a clear button). No subtitle.
- **Replace API key** - same form shape. Its second field is labelled
  **"New device name"**, copy-pasted from the rename dialog. Design bug.
- **Disconnect Gate?** - red tone, body as a plain paragraph rather than a
  tinted note, primary `Yes, disconnect Gate`.
- **Reset Gate Connect** - red tone, subtitle, a `What happens next:` list of
  three numbered steps, and a **checkbox** gating the destructive primary.

### Elsewhere

- Overview: the org-switch frames are now properly named (`overview-switch org`
  x3, where only two generically-named ones existed before).
- App: a new **choose model modal**, distinct from the model-switch
  confirmation. `Change model` currently goes nowhere.

### Built 2026-08-17

`Modal.tsx` gained the four things it lacked: a `danger` tone
(`red-100` tile, `red-600` icon), `ModalField`, `ModalSteps`, `ModalCheckbox`.
`ModalButton` gained `disabled` so the checkbox can actually refuse the primary,
and `Modal` gained `initialFocus` so form dialogs open on the field being
edited rather than the read-only one above it. Glyph added: `circleX`.

`dialogs.tsx` gained `RenameDeviceDialog`, `ReplaceApiKeyDialog`,
`DisconnectGateDialog`, `ResetGateConnectDialog`.

Deliberate deviation: `ReplaceApiKeyDialog` labels its second field
**"New API key"**, not the drawn "New device name". Shipping the design's label
would put a wrong word on the one screen where the user handles a credential.

Still unbuilt: the App page's **choose model** dialog. It is only known from a
frame caption; nothing about its contents has been read, so building it would be
invention.

## Re-verification of the Flows pages, 2026-08-19

MCP allowed exactly one read this month before the View-seat cap cut in. That
call confirmed the document still reports a single top-level page, `Components`
(113:16762): **MCP never sees the `Flows` pages**, so everything below was read
through the browser using the Layers-panel + `shift+2` method.

### Two new flow pages, both marked ready

The Pages list now reads `Components`, `Flows` (**Onboarding**, **Auth**,
`Overview`, `App`, `Settings`), `Sandbox`, `Icons`. Onboarding and Auth did not
exist on 2026-08-17, and both carry the designer's green check.

This flips **open question 6**. Setup and onboarding were built provisionally
*because* they were "not in the Figma". They now are, and what was built does
not match them.

#### Auth (177:79238) - four sections

`Auth / Connect with Gate` ✅, `Auth / Connect with API key` ✅,
`Auth / Organizations` ✅, and `Auth / Error states` with **no check**, so that
last one is still moving and should not be chased yet.

The drawn flow is auth, then **name this device**, then choose apps. The API-key
path skips the org picker entirely and goes straight to naming; the copy states
the order outright: "After it connects, you will name this device before
choosing which apps are protected."

| Step | Drawn | Built in `setup.tsx` |
| --- | --- | --- |
| Sign in | `Gate Connect` lockup, "Continue with Gate account", `or`, outline "Use an API key" | `WelcomePane`, "Sign in with Constellation" over a disclosure toggle "Use a Gate API key instead" |
| API key | its own pane: "Use an API key", field, "Connect and continue", "Go back" | inline disclosure under the welcome pane, not a destination |
| Organization | "Choose an organization", radio rows with initials tile, name, `12 members · Free plan` | `OrgPickerPane`, same shape |
| **Name this device** | own pane, field, `Continue`, `Skip naming` link | **not built** |

Copy that differs verbatim:

- Sign-in body is "Sign in once, then choose which AI apps route through Gate.
  Claude, Codex, OpenCode, and supported apps keep working normally while Gate
  handles protection underneath." The built pane says "Point your AI tools at
  Gate once, and stop thinking about credentials."
- Org-picker subtitle is "Gate Connect will use the selected organization for
  routing, activity, and PAYG credits on this device." The built pane says
  "This decides where your activity is recorded and whose Gate credits you use."
- Every pane carries a `Use a different account` link, which nothing builds.

`Auth / Organizations` draws the cardinality states explicitly: none, one, two,
three, and a scrollable many. **The empty state's escape is `Go back` plus
`Use a different account`**, inside an amber note reading "No organizations
found. You will need to setup your first organization through Gate AI before
continuing to setup Gate Connect." The built `OrgPickerPane` escapes to the key
form instead, which its own comment calls "the only way forward". The design
disagrees.

`Auth / Error states` (not ready, recorded only so it is not read as missing):
a **Setup timeout** dialog ("Go back" / "Retry") over the org picker, and a
device-name field failing validation with "Incorrect characters or symbols used".

#### Onboarding (177:79237)

A welcome frame plus a three-step tutorial, all in the window shell with a
progress bar under the topbar and a persistent `Do not show this intro again`
checkbox in the footer.

| # | Drawn | `screens/Onboarding.tsx` |
| --- | --- | --- |
| - | "Welcome to Gate Connect" / "Created by Constellation Network" | same, subtitle reads "Created by Constellation Gate AI" |
| 1 of 3 | "What is Gate Connect?" | "How to turn it on" |
| 2 of 3 | "Where is Gate Connect?" + `Show me where Gate Connect lives` | same title, and `step.locate` already builds that button |
| 3 of 3 | "See what Gate is doing" | **no counterpart** |

Step 3 is about the dashboard: "Once requests pass through Gate, the desktop app
shows recent activity, security actions, and compression savings without
exposing prompt or response content", over a note "Notifications will alert you
when a request has been blocked or flagged."

**Step 2 is worth a decision.** Its copy is "Gate Connect stays open in your menu
bar ... Click the Gate Connect icon to open the compact popover for a quick
status check, or expand it to the full desktop app for more details, alerts, and
controls." The onboarding therefore *teaches* popover and window as two states of
one product with an expand/collapse between them. **Open question 5 removed
Minimize2 on the grounds that it duplicated an OS control**, and the Auth frames
still draw both an ellipsis and a collapse button at top right. Either the
tutorial copy is stale or dropping Minimize2 was wrong; the designer should say
which.

### The action pills were sampled at last, and the inference was wrong

`Overview.tsx` still carries the comment that the `100` backgrounds "are inferred
from that pairing rather than sampled from Figma". Deep-selecting the FLAG pill
on `Overview-partly-routed-1` (116:26405) settles it:

| Property | Figma | Built |
| --- | --- | --- |
| Background | `tailwind colors/amber/200` `#FDE68A` | `bg-amber-100` |
| Text | `tailwind-colors/amber/900` `#78350F` | `text-amber-900` ✓ |
| Radius | 2px | `rounded-base`, 4px |
| Padding | 8px horizontal, 4px vertical | `px-1.5 py-0.5`, 6px / 2px |
| Type | `mono/label-12`, Geist Mono Medium 12/16, 6% | `font-mono text-base-xs font-medium tracking-label` ✓ |

Only FLAG was sampled. BLOCK, REDACT and the green `ON` pill were left alone, but
the 200 stop is consistent with the status tile's own 50 -> 200 gradient, so
expect all four to move one step and check them before shipping the change.

### Unchanged

Token layer still matches every value sampled on 2026-08-17: `base.*`,
`blue-ribbon` 700/800/900, the four Settings labels, the window frame's 8px
radius, and the Overview stat trio (`MESSAGES` / `BLOCKED/FLAGGED` /
`TOKENS SAVED`, the last with a green `+$3.10` delta).


### Built 2026-08-19

**Action pills.** `Overview.tsx` and `AppPane.tsx` move to the 200 stop at a 2px
radius with 8/4 padding, and the comments that called the backgrounds inferred
are gone. `StatusPill` picks up the pill's mono/uppercase treatment; its `Off`
fill stays neutral-100 because no frame draws an off pill. BLOCK, REDACT and the green ON fill went unsampled that
day - the renderer froze first - and were read on 2026-08-20; see below.

**The Auth flow.** `setup.tsx` gained `ApiKeyPane` and `NameDevicePane`, and
`WelcomePane` lost the inline key form: the key is a destination now, reached
through `Use an API key` under an `or` divider, with `Go back` returning. Copy
throughout is the drawn copy. `SetupHeader` takes a `ReactNode` title so the
sign-in card can show the two-tone wordmark, `PrimaryButton` gained `disabled`
so the drawn muted primaries refuse an empty field or an unselected org, and
`SetupLink` / `OrDivider` are the small pieces those panes needed.

`useSetup` gained the `api-key` and `name-device` stages and a `signOut`.
Naming derives from `device_name === null`, guarded on `sawSignedOut` for the
reason the confirmation is: null is what every never-renamed install carries, so
deriving from it alone would meet returning users with the pane on every launch.
`namingDone` covers both skipping and saving, since the preference re-read that
flips `deviceNamed` lands a beat after the save.

`signOut` drops the OAuth session **and** rewrites the account with the gateway
and no key, so the key path leaves too. It deliberately does not call
`clear_account`, which would take the chosen gateway with it and quietly repoint
a staging install at production.

`keyFormForced` and `useApiKeyInstead` are gone. **The window and the popover now
disagree about the org-picker dead end**: the window signs out, the popover still
offers the key form (`screens/OrgPicker.tsx`, pinned by
`signin.spec.ts` "the key form is reachable from the org picker's dead end").
That is the design's call, taken knowingly, and it is a real divergence until the
popover retires.

**Onboarding.** Four steps now, matching the drawn flow: the welcome, "What is
Gate Connect?", "Where is Gate Connect?" and "See what Gate is doing". `Step`
gained an optional `note` for the bordered strip each tutorial step carries, and
`hero` became optional. Step 2 keeps its platform-aware sub-heading rather than
the design's macOS-only "menu bar", and step 1 keeps the config-versus-proxy
mechanism sentence the design drops, because neither is expressible in the drawn
copy and both are true.

Both tutorial illustrations are now the Figma's own art, captured 2026-08-20:
`onboarding-what-is-gate-connect.png` (the apps-to-Gate flow diagram) and
`onboarding-see-what-gate-is-doing.png` (the Overview dashboard). Each is the
590x220 panel inside its step's card, taken through the browser at roughly 1.7x
so it holds up on a retina panel - MCP has no quota to export with and the View
seat cannot use Dev Mode. The dashboard one is cropped mid-chart in the design,
so it carries the same bottom fade the platform mockups use; the diagram is whole
and does not. `RoutingHero` and its `app-integrations.png` import are gone, and
the PNG went with them: it was the popover's Routing screen, the last caller of it
disappeared with the hero, and nothing else in the repo referenced it.

`new-ui-firstrun.spec.ts` follows, with a new test pinning that the device is
named between connecting and the confirmation, and that Continue refuses an empty
field. 428 unit tests and 156 e2e pass.

### Built 2026-08-20

**The pill fills, read off the pixels.** Rather than fight the properties panel a
second time, the Policies table was captured as a PNG and the fills counted
directly. Three matched Tailwind exactly, at distance 0.0, which is what makes
the fourth trustworthy: `red/200 #FECACA`, `amber/200 #FDE68A`, `green/200
#BBF7D0` - and REDACT at **`#DDD6FE`, which is `violet/200`, not `purple/200`
(`#E9D5FF`)**.

So the redaction hue was wrong in two places, and had been since the tokens
landed. `chart.redacted` was `#A855F7` (purple/500) against a drawn `#8B5CF6`
(violet/500), and the REDACT / `redacted` pills were `purple-200/900`. Both now
say violet. The other three chart series were confirmed correct in the same pass
(`#60A5FA`, `#F87171`, `#FBBF24`), so only the one hue moved. Nothing else in the
palette is violet, which makes this easy to "correct" back by eye - the comments
in `Overview.tsx` and `AppPane.tsx` say so explicitly.

**The onboarding window moved to the new shell.** `screens/Onboarding.tsx` was
the last thing still rendering on the popover-era `gc/` ink system. It now draws
what the design draws: a 48px topbar with the brand lockup centred, a progress
rail directly beneath it, one card per tutorial step carrying a `TUTORIAL`
eyebrow and an `N of 3` counter, and a footer holding the opt-out checkbox and
Previous / Next. Private `IntroTopbar`, `IntroProgress` and `IntroButton` are
local to the file - `gc/Topbar` owns an overflow menu the intro has no use for,
and the setup panes' buttons are private to theirs.

Two things fell out of it. The step dots are gone, replaced by the rail, which
takes over their `Step N of M` announcement through `role="progressbar"` rather
than the dots' `role="img"`. And the tutorial art is **not** wrapped in a panel:
the captures already *are* the design's 590x220 panel, border and gray field
included, so the first attempt drew two nested boxes.

The welcome frame keeps its own shape - centred mark, title, rule, copy, no card -
because that is how the design draws it.

### Sync, 2026-08-20 (second pass)

Re-read on the report that every flow is now green. Most of it is, and one
thing that was said to be green is not.

**`Auth / Error states` still carries no check.** Verified twice, the second
time after a hard reload, in both the layers panel and the canvas section label,
while the three sections beside it (`Connect with Gate`, `Connect with API key`,
`Organizations`) all show theirs. So the setup-timeout dialog and the
device-name validation rule stay unbuilt. Both frames were read on 2026-08-19
and are described above, so if the designer has signed them off verbally and the
file simply has not caught up, this is short work - but the file is the signal
this plan follows, and it says not yet.

**Settings is green throughout** - `Reset Gate Connect`, `Disconnect Gate
session`, `Update device name` (twice), `Main screens`, `Dimensions` - and every
one of those flows is built.

**The App page carries a section nothing here had ever read: `App / No data
1+ day state` ✅.** It holds two states that the panes did not have:

- **No data.** Stat tiles read `0`, `0`, **`N/A`** - not `0%`. The Messages card
  drops its axis and legend for a centred column-chart mark over "No messages
  sent in the last 24hrs", and Recent activity drops its header row for "No
  recent activity in the last 24hrs".
- **Loading.** The same cards with their labels in place and their values
  replaced by grey skeleton bars, including 24 uniform full-height bars where
  the chart goes.

The no-data half is now built, and it matters more than it looks: **both panes
already pass `EMPTY_STATS` and `buckets={[]}`**, because the 24-hour endpoint does
not exist, so this is the state the app is in every time anyone opens it. What
shipped before drew a labelled 24-column axis under 24 invisible bars and a table
header over no rows, which reads as a chart that failed rather than a quiet day.

`UsageStats.tokensSavedPercent` became `number | null` to carry it. `0%` is a
claim about traffic that was never measured; `N/A` is the design's own answer and
is the honest one. `tokensSavedAmount` follows it, since there is no amount to put
beside a figure that does not exist.

`EmptyNote` lives in `metrics.tsx` because the chart is shared by both panes, so
the Overview inherits the same treatment without a second implementation.
`metrics.test.tsx` pins all of it, including that one bucket carrying traffic is
enough to bring the chart back.

**The loading half is deliberately not built.** Nothing can be loading until
there is an endpoint to load from, and a skeleton no code path can reach is dead
code. Its appearance is recorded above for whoever lands that endpoint.

**Not re-read this pass:** the Overview page. Three page-switch attempts failed -
Figma's Pages list needs a click to select and another to navigate, and it stopped
taking the second one - so its own `overview-loading` frame was not opened. The
shared components mean the Overview picks up the empty treatment regardless; what
is unverified is whether its no-data copy differs from the App page's.

### Sync 2026-08-21: Settings, Setup, Onboarding, and the rest of the file

The remaining flows, read the same day. All Settings and Setup sections carry
the check except **`Auth / Error states`, which still has none** - the
setup-timeout dialog and the device-name validation stay unbuilt, third read
running.

**Settings caught up with the build, and the build moved twice.** The drawn
screen now includes the Change server action and a full Diagnostics section
(Share diagnostic data + Diagnostics report / View report) - both previously
built as undrawn deviations, now canon. What moved to match the drawing: the
Notifications switch is a **row under Startup** again (AG-594's separate
section is not drawn; the row keeps the honest "session expires / cannot be
put back" description because the drawn "blocked or flagged" copy still
promises events AG-578 has not built); the share-diagnostics description
drops ", responses," to read exactly as drawn; and the report dialog is
titled **"Diagnostics report"**. Kept deviations: the Help section and the
Sign-in method / certificate / What-is-collected rows (not drawn, all
previously recorded), and the drawn report subtitle's "this installed" typo,
shipped corrected. Rename, Replace key, Disconnect and Reset all match their
frames verbatim.

**Setup gained two drawn things.** Every frame carries a **progress rail**
under the topbar - `SetupLayout` takes `progress` now, fed positionally per
stage. And the diagnostics step is finally drawn ("Share diagnostic data"):
rebuilt to match - share2 tile (glyph added), the drawn one-line copy, a
"Diagnostic data sharing" switch row, **Finish setup** as the primary and a
**Skip data sharing** link that records the refusal rather than leaving the
question unanswered. The itemised sent / never-sent lists moved out of the
step with the redraw; they remain in Settings under "What is collected". The
sign-in, API-key, org-picker and name-device frames all match what was built
on 2026-08-19, including the busy states.

**Onboarding matches as built** (welcome + three steps, rail, footer). No
changes.

**The rest of the file.** The Components page still carries the older
`banner/routing` copy (`Routing · 2 of 4 Apps` beside an all-protected
message); the flow frames' `Routed · N of N` wins, being both newer and
internally consistent. `topnav/menu` still draws Contact support, which stays
omitted for want of an address. Icons is a glyph library. The **Design docs
and Comments pages could not be opened** - Figma's page switching refused
across a dozen attempts in two sessions of this pass - so whatever they hold
is unread; they are documentation pages, not flow drawings, but worth a look
when the canvas cooperates.

514 unit tests and the full 163-test e2e suite pass.

### Sync 2026-08-21: the Overview flow, read at last

`Flows / Overview` (116:26381) had not been re-read since the failed page
switches of 2026-08-20. Every section carries the check: Dimensions, Main
screens, two Turn-routing-ON dialog sections, and Switching an organization.
What moved, and what was confirmed:

**The action pills did not change.** BLOCK sampled `red/200` `#FECACA` over
`red/900` at a 2px radius - exactly what shipped on 2026-08-20 - so the
Overview's action pills and the App table's 100/700 status badges really are
two different components, not one drifting spec. **The ON pill gained a
glyph**: `circleCheck` at 12px in `green/800`, a 4px gap ahead of the
`green/200`/`green/900` label. `StatusPill` moved from a bare `check` at an
8px gap. **The Manage footers are bordered buttons now**, sitting under a
full-width rule; `ManageLink` moved from a borderless text link.

**`overview-loading` (228:85602) was finally read**, and the built silhouette
was wrong in three ways: the drawn placeholder is 24 *uniform full-height*
columns, the numbered ticks render as numbers (1..24) rather than skeleton
bars, and the legend stays on screen while loading. `PendingChart` follows.
The legend is also **left-aligned** in every frame; the built one was centred.
One discrepancy left alone: the loading frame draws a rule above the legend
and the loaded frames do not, so the shared markup follows the loaded state.

**The routed banner says "Routed"**, not the "Routing" the earlier read
recorded (`Routed · 4 of 4 Apps` in every routed frame). `RoutingBanner`
updated.

**The dialogs moved, mostly in copy.** Apply changes gained its question mark
and Yes/No button prefixes ("Yes, close affected apps" / "No, I will reopen
later"); Close apps' escape is "No, I will close later" and its destructive
primary is the generic "Yes, close apps" rather than naming the app; Switch
organization's subtitle starts "Select", and its drawn primary is muted while
the selected org is the current one - `SwitchOrganizationDialog` gained
`currentId` and refuses the no-op, which the shell feeds from the account.
The Organization-switched copy, flagged on 2026-08-17 as the least certain of
the seven, is confirmed verbatim. And the three short confirmations - Change
is ready, Organization switched, Use a Gate model - are drawn at **520px**,
not the template's 600: `Modal` gained `narrow`.

Review config, Change is ready, the alert banner, the stat trio, the tooltip
and the chart's loaded anatomy all match as built; the extra transparency the
review dialog carries (the Gate route and the file path) stays. e2e:
`new-ui-running-apps.spec.ts` follows the new button labels.

### Sync 2026-08-21: the App page moved, and the build follows it

Read through the browser (MCP quota still spent). The Pages list itself has
changed: `Comments` and `Design docs` are new top-level pages, and the Flows
page formerly named `Auth` is now `Setup`. Only `Flows / App` (116:30199) was
read this pass; every section on it carries the ready check, including two new
ones: **`App / Table guide`** and the reworked **`App / Select a model`**.

**Recent activity is a different table** (`table/recent-activity`, 272:3150).
Columns are now Time / Security / Model / Message / Action. The old Status
column is gone - ERROR joined the Security pill set - and the pills were
sampled from the properties panel: **the 100 stop with 700 text**, at a 4px
radius with 8/4 padding, not the Overview pills' 200/900 at 2px. ERROR and
BLOCKED sample identically (`red/100` / `red/700`); FLAGGED is
`amber/100`/`amber/700`; REDACTED is `violet/100` with its text at the **800**;
ALLOW is `gray/100` over `base/muted-foreground` - a grey non-verdict, not a
green badge. Built in `AppPane.tsx`: one pill per row, the recorded verdict
outranking the transport error because a blocked request usually also errors
client-side, with ERROR reserved for rows where the gateway recorded no
action. The Model cell is the model name alone (the drawn vendor mark waits on
open question 2), and the Message cell carries the mono session reference -
the drawn title over it could only come from prompt text, which AG-574
excludes. **The Action column is drawn but not wired** (decision, 2026-08-21):
each row carries the design's View button with the external-link glyph, and
`AppPane` exposes `onViewEntry` for the destination, but the shell passes no
handler yet because nothing in `dashboard-web` filters by tool, machine or
time. The deep link is the remaining work, not the column.

**The model picker is a centred dialog now, not a dropdown.** The earlier
read's cut-off top edge turns out to have hidden a real header: X dismiss,
a "Search models" field, an "All providers" filter, and a count line reading
"Showing 10 of 14 models · 400+ in Gate AI". `ModelPickerDialog` rebuilt to
match: search filters on the id, the provider select is derived from the model
list, the count line reports the filter's own numbers, and rows carry trailing
radio circles with the selected one outlined. `Modal` gained an opt-in
`onClose` X for it. The count line ships the drawn "· 400+ in Gate AI" tail
verbatim (decision, 2026-08-21). One deviation, recorded at the component: the
drawn subtitle "Claude Desktop uses on Gate model" is not a sentence, shipped
as "What {app} uses on Gate model" and raised as a copy question. The list is
still empty until a gateway endpoint reports models; the empty note keeps the
search chrome off screen rather than drawing dead controls.

**The sidebar is grouped now.** Every frame on the page draws the rail's apps
under mono eyebrows - `ANTHROPIC` over the two Claude apps, `OPEN AI` over
Codex, `OPENCODE` over OpenCode - and the `PROTECTED APPS 4/4` counter is
gone, which retires open question 1. Built as `SidebarGroup`: the shell groups
rows by the routing families `buildGroups` already computes, labelled with the
drawn vendor captions via each family's `upstream_provider_name` (decision,
2026-08-21). The multi-provider tools carry a sentence fragment in that field
("your existing providers"), and the design draws each under its own name, so
the leftover family splits into one group per tool. Before the catalog loads,
one unlabelled group keeps the rows on screen. The drawn rail also shows no
Families nav item, and as of 2026-08-26 neither does ours - see below.

**Small pieces.** The Model selection card gained the drawn divider above
"Current Gate model". `UseGateModelDialog`, the alert banners, the stat trio,
the chart and the no-data states all still match their frames; none were
touched.

**`nav/sidebar/overview` (113:16794) was pulled from the Components page**, and
it confirms the grouped rail is canonical, not an App-page quirk: all three
sidebar variants there draw the vendor eyebrows and none draw the old
`PROTECTED APPS` counter. Sampled values, and what moved to match them:
container is 250px with 16px padding, a **20px** stack rhythm (was 16) and a
1px right edge at **5%** black (was 8%); nav labels and row names are
`label/12` (Geist Medium 12/16), active nav in `base/primary`, row names in
`base/foreground`; the app tile is 32x32, r4, black under the white-to-black
gradient with a 1px white border at 24%; rows are 44px, padding 6/4, gap 16,
**radius 4px** - which the radius scale maps onto `sm`, so the row moved from
`rounded-md`. The row-hover variant (121:33421) draws the treatment the rail
lacked: a `neutral-100` fill inside a 1px `neutral-200` border with the name
in `base/primary`. Built as one treatment for hover and selection, with the
border reserved at rest so rows do not shift. A second read on a fresh tab
(the first ended in the recurring renderer freeze) closed the gaps the freeze
had left: the group eyebrow is `mono/eyebrow` (Geist Mono Medium 12/16, 10%
tracking, uppercase, `base/muted-foreground`), matching what was built on
sight; a group's eyebrow sits 8px above its rows; and `apps-section`
(113:16814) spaces the groups at **12px**, so the rail moved from 16.

`new-ui-model-picker.spec.ts` follows the picker (X instead of Cancel), and
`AppPane.test.tsx` pins the merged pill column. 514 unit tests and the 114
new-UI e2e tests pass.

### Sync 2026-08-23: the Sidenav page, and the rail it redraws

Read through the browser (MCP quota still spent). Two structural changes to
the file itself: the `Comments` page is gone without ever being read, and
`Components` gained a nested page, **`Sidenav`** (408:15625), carrying the
ready check. The old `nav/sidebar/*` frames are no longer on the Components
page - Sidenav replaces them, and it redraws the rail rather than just
rehousing it. Every flow page's frames already carry the new rail.

What the redrawn rail changes, in order of consequence:

- **Chat domains are rows now.** All three sidebar variants draw nine rows:
  Claude Desktop and Claude Code under `ANTHROPIC`; Codex, OpenAI apps and
  ChatGPT under `OPEN AI`; OpenRouter under an eyebrow reading `OPENCODE`
  (a mislabel - see below); OpenCode and OpenClaw under `OTHER TOOLS`.
  Claude Desktop, OpenAI apps, ChatGPT and OpenRouter are proxy-routed chat
  members, which the drawn UI had never shown - the 2026-08-16 note on open
  question 3 ("the chat domains, which the drawn UI does not show at all")
  no longer holds. Each carries the same switch and status line as a config
  tool. The built rail filters them out (`m.kind !== "config"` in
  `sidebarGroups`), so today it shows only the config tools.
- **Every group eyebrow carries a counter** - `1 of 2`, right-aligned, mono -
  counting protected members over members (verified against all four groups'
  drawn states). `PROTECTED APPS 4/4` stays gone; this is its per-group
  descendant.
- **The multi-provider tools share one `OTHER TOOLS` eyebrow.** That reverses
  the 2026-08-21 reading, where each drew under its own name and
  `sidebarGroups` was built to split them. One group, labelled Other tools.
- **The brand marks exist in the file at last.** A `logo` component set draws
  eight marks (Claude, Claude Code, Codex, OpenAI, ChatGPT, OpenClaw,
  OpenCode, OpenRouter), and `logo-wrapper` places each in the 32px tile. A
  separate model `row` component set draws coloured vendor marks (Anthropic,
  DeepSeek, Qwen, OpenAI) beside `gate/...` ids, with the selected row
  outlined and check-marked. Open question 2 is resolvable: nothing needs
  inventing, but the View seat cannot export SVGs, so the marks need either
  a designer export or a high-DPI browser capture like the onboarding art.
- The `sidebar-menu-item` selected variant and the `status-label` trio
  (`Not protected - 2m ago` / `Protected - 25s ago` / `Not routed`) match
  what was built on 2026-08-21.

**The Families pane on this page is a pasted screenshot, not a drawing.** The
layer is literally `image 1` - `image.png`, 931x990 - and it shows the built
`FamiliesPane` (master card, family cards, the Config file / Local proxy
qualifiers). So the designer has seen the pane and keeps it as reference, but
the rail still draws no Families nav item and no frame specifies the pane.
The pane was retired on 2026-08-26; its switches moved into the rail.

**One design bug to raise:** the eyebrow above the OpenRouter row reads
`OPENCODE` in all three variants and in the flow frames' rails. The family
label here derives from `upstream_provider_name`, which reads OpenRouter, and
that stays.

**The alert banners moved.** The Components page now draws three states, and
the flow frames agree:

| Variant | Title | Body |
| --- | --- | --- |
| off (single + multiple-apps) | `Codex isn't protected` | Routing is set to **off**. Reconnect to restore protection. |
| drift (new) | Reconnect to restore protection | This app's config changed outside Gate, so its traffic isn't routed. |

The built `master-off` notice says "switched on but not routing" and the
built `drifted` notice carries the old single-app copy, so both retitle. One
question to raise before shipping the drift copy verbatim: the drawn drift
banner names no app anywhere on the card, and the card pages between apps,
so two drifted tools would be indistinguishable.

Also confirmed this pass: the Overview pane's `Last 24 hours` header caption
(already built - `period` in `Overview.tsx`); `Auth / Error states` still
without its check (fourth read, so the timeout dialog and name validation
stay unbuilt); Settings, App, Onboarding and Setup otherwise unchanged with
every recorded deviation standing; and the `Design docs` page holding exactly
the four frames `docs/new_ui_design/` already imported (`design.md` plus the
three token files) - nothing new to pull.

### Built 2026-08-23

Items 10 (minus the chat-row pane) and 12 from the queue.

**The rail follows the Sidenav page.** `sidebarGroups` in `NewUiApp.tsx` no
longer filters proxy members out: the chat domains and a family's app
surfaces are rows now, with `setDomainRouted` behind their switch (the same
dispatch the family panel's member switches use - a domain has no config
file, so no drift gate) and their status derived from the domain's own state
through `proxyMemberStatus`, extracted so the rail and the family panel
cannot phrase one domain two ways. `SidebarApp` gained `noPane`: no drawn App
pane exists for a domain, so the name renders as a label rather than a link,
with the hover fill suppressed because it is the "this opens something"
signal. Each group eyebrow carries the drawn protected-over-total counter,
derived from the rows in `Sidebar.tsx` so it can never disagree with them,
and not uppercased - the drawing reads "1 of 2". The multi-provider tools
collapsed back into one "Other tools" group. The routing banner still counts
tools only; whether domains should join its "N of M Apps" is a designer
question, not assumed here.

**The alert copy follows the redrawn banners.** `lib/notices.ts`: master-off
reads "X isn't protected" / "Routing is set to off. Reconnect to restore
protection." (plural: "N apps aren't protected"); the drift notice and
`NewUiApp`'s `driftAlert` title with the remedy, "Reconnect to restore
protection". One deviation, recorded at both sites: the drawn drift body says
"This app's" and the card names no app while paging between apps, so the
name goes where that phrase was.

**A selector trap the new rows sprang.** Playwright's `getByRole` name option
is substring matching, so the always-present "ChatGPT (Codex subscription)"
row switch now matches every `{ name: "Codex" }` selector - including one
whose job was to *wait* for a tool to appear, which stopped waiting and raced
the detection poll. The affected specs use `exact: true` or scope to the row;
new pins in `new-ui-routing.spec.ts` cover the domain-row dispatch, the
counter, and the single Other-tools eyebrow. 514 unit tests and the 166-test
e2e suite pass.

Still open from item 10: where a chat row navigates. Every drawn App frame is
a proxy member's pane, so the design says they open one; that needs an
`AppPane` feed with no config file, no drift review and no process staleness,
and it is its own PR. **Closed the same day, second pass** - domain rows open
the pane, whose activity sections say they cannot be attributed rather than
claiming a quiet day; see item 10 in "Still to do" for the mechanism and the
attribution constraint.

### Sync 2026-08-26: Settings read from the node data, not the canvas

Read through the Framelink PAT rather than the browser: `Flows / Settings`
(116:28963) at depth 4 to map it, then the Main-screens `scroll-content`
(116:28972) in full. Two calls of the PAT's own Tier-1 allowance; see
[[project-figma-mcp-view-seat-rate-limit]] for the budget. Every section on
the page still carries the check, and the section list, row order, labels and
button copy all match what was built on 2026-08-21. What the node data
settles is everything the browser reads had to eyeball.

**The value column is Geist, not Geist Mono.** Install ID, Gateway, API key
and Version are all `copy/14` (Geist Regular 14/20), and so is every other
value on the screen. The earlier read recorded them as mono and the build
followed, with a test citing CLAUDE.md's mono-for-identifiers rule as the one
the design did not overturn. It does overturn it here, and deliberately: the
same file reaches for `mono/body-14` in the diagnostics report dialog a screen
away, so the designer had the style to hand and chose the sans one. `mono` is
gone from `SettingsRow`; `SettingsPane.test.tsx` now pins the absence.

**Nine glyphs were near-misses.** The frame names its icons, and seven rows
were drawing something else: Device is `Monitor` (not `MonitorSmartphone`),
Install ID `SquareUser` (not `IdCard`), Gate plan `FileBadge2` (not
`Receipt`), Launch at login `CirclePower` (not `Power`), Share diagnostic data
`Share2` (not `ShieldCheck`), Diagnostics report `ClipboardList` (not `Info`),
Version `SquareCode` (not `CodeXml`). API key's `key` was already lucide
`KeyRound` and Reset's `refresh` already `RefreshCw`, so those two stood.
`Icon.tsx` gained the five missing glyphs, geometry taken from
`lucide-static@1.34.0` rather than drawn by hand.

**The rules sit inside the card's padding.** The card pads 16px and stacks its
rows at a 16px gap with a 1px `base/border` rule between them; the build had
per-row `px-4 py-3` with a `border-t`, which bled each rule to the card's
edges and set the rhythm at 12px. Restructured to the drawn shape.

**Spacing and type, measured:** section-to-section gap 24px (was 16),
heading-to-card gap 12px (was 8), section headings `heading/16` - Geist Medium
16/24 - rather than 14/20. Row descriptions are `copy/12` at `#6B7280`, which
is the `base/muted-foreground` token, not `neutral-600`. The On/Off word
beside a switch is `copy/14` at full foreground weight, not a 12px muted
label. Action buttons are `Size=sm`: 32px tall at 8/12 padding with a 16px
trailing glyph, where the build had `px-2 py-1` and a 12px glyph. The Danger
zone card's border is `red-600` at 40%, not `red-200` - that one had been
marked inferred at the call site since it was written.

**Two copy fixes**, both verbatim from the frame: the share-diagnostics
description reads "routing stats", not "routing state", and the diagnostics
report description ends in a full stop.

**Not changed, and why.** Foreground text stays `neutral-900` where the frame
says `#030712` (gray-950): that is the whole shell's convention, not this
screen's, and moving one pane would split it. The outline button's inset
highlight pair is likewise a Button-component treatment nothing in the new UI
reproduces yet. The destructive button's `#FEF2F2` label stays white, matching
`Modal`. The Notifications description kept the honest routing-health copy
over the drawn "blocked or flagged" for the reason recorded on 2026-08-21 - since
superseded by AG-578, which built the feed and gave the drawn sentence its own
rows - and
the Help section and the Sign-in method / certificate / What-is-collected rows
remain the standing undrawn deviations. 548 unit tests and the 169-test e2e
suite pass.

### Sync 2026-08-26: Onboarding, and a page that had moved more than it looked

Read the same way as Settings that day: `Flows / Onboarding` (177:79237) at
depth 4, then the step-2 card (212:84769) and the welcome content column
(240:4500) in full. The page carries the check and still holds four frames -
a welcome and three tutorial steps - but almost every measurement under them
had changed since the 2026-08-20 build, and the page gained a second section,
`Onboarding / Images`, holding the three illustrations as vectors.

**The eyebrow reads `Introduction`, not `Tutorial`.** The counter beside it is
unchanged (`1 of 3` .. `3 of 3`, uppercased by `mono/eyebrow`).

**The welcome frame is a different drawing.** It is no longer the app icon
over a title: it is a 96px white tile on a `blue-ribbon-300` hairline at a 16px
radius, carrying the hex mark at 56px, with the design's own inset pair - a
blue glow up from the bottom edge, a white one down from the top. Under it the
title is `heading/32` (Geist Medium 32/36 at -4%) and **two-tone**: "Welcome
to" in foreground, "Gate Connect" in `base/primary`, the wordmark's own split.
The sub is Geist Medium 14/20 muted, the rule under it is `base/input` rather
than `base/border`, and the body is `copy/16` at full foreground weight - not
14px muted - with its closing sentence, "Click Next to get started.", set in
Medium because it is the only instruction on the frame. The column is 540px.
`ConstellationHexMark` replaces `app-icon.png`, which now has no caller.

**The tutorial card is 640px at a 16px radius under `shadow/lg`**, where the
build had 680px, 8px and `shadow/sm`. Inside it: eyebrow row, 12px, title
(Geist SemiBold 20/**24**), 8px, subtitle - and the subtitle is full
foreground, not muted, as is every paragraph under it. Then 24px to the media
block, which stacks the 220px illustration and the note at 16px. The note is
**transparent** with a 1px border at 12/16 padding, and its text is full
foreground too. The build had a white fill, 8/12 padding and muted text
throughout.

**The locate button is not inside the card.** The frame puts "Show me where
Gate Connect lives" between the card and the footer, centred, as a `Size=sm`
outline button with a **left `Focus` glyph**. Moved out, and the glyph added.

**The progress rail is 8px, not 4**, on a `base/background` track with a
bottom hairline. Its fill is `#7195FF` under a left-to-right black-to-white
64% wash, which composites from a deep navy to a pale blue; the build
approximated it with two blue-ribbon stops. The literal is spelled out at the
call site because `#7195FF` sits between ramp stops and is not a token.

**The footer** pads 12/24 rather than standing at a fixed 56px, and its
buttons are a fixed 220px pair at a 12px gap, each filling half - so Next does
not shift when its label becomes Get started. Next carries the Button
component's right arrow; Get started does not. On the welcome frame the design
draws Previous at **zero opacity** rather than dropping it, which is what
holds that alignment, so it is `invisible` now instead of dimmed.

`Icon.tsx` gained `arrowRight` and `focus`; `tailwind.config.ts` gained
`base-3xl` (32px) and `tracking-heading-32` for `heading/32`, which sits
between Tailwind's own 3xl and 4xl and had no stop.

### Sync 2026-08-26, second pass: the last two intro steps, and both quotas

Both quotas ran out inside this pass, which is worth recording as a working
constraint rather than a footnote. The step-3 card (212:85210) came back; the
next call, for step 4 (212:85407), hit Framelink's **429 with a 398,582-second
retry** - about 4.6 days - and the official MCP answered the same request with
the View-seat tool-call cap. So the PAT bucket and the OAuth bucket are both
empty, and the browser method in [[reading-figma-via-browser]] is the only
route left this month.

**Step 3 confirmed the card anatomy a second time** - 640px, r16, shadow/lg,
eyebrow `Introduction` / `2 of 3`, title Geist SemiBold 20/24, subtitle
`copy/14` at full foreground - and turned up one miss: **the note's glyph is
`MonitorSmartphone`, not `Monitor`**. Fixed. Its note text and the second
sentence of its subtitle match the build verbatim.

Two things left alone. The drawn subtitle opens "Gate Connect stays open in
your menu bar, so it is always easy to access while you work", which is the
macOS-only phrasing the 2026-08-19 deviation already covers; the build keeps
`whereItLives(platform)`. And **the two notes disagree with each other**: step
2's icon-to-text gap is 8px, step 3's is 12px, on what is otherwise the same
component. The build stays at 8px for both rather than reproducing the
difference - worth a designer question, not a build.

**Step 4 was read through the browser the same day**, both quotas being gone,
and the guess above was right: **the glyph is `BellDot`**, the bell with an
unread mark, not the plain `bell`. Confirmed by ctrl-click into the vector and
reading `Parent component: Icon / BellDot` off the properties panel. Added and
applied.

Walking up from there settled the note gap too: step 4's note is `Fill (590px)`
x `Hug (42px)`, r8, 1px `base/border`, padding 12/16, **gap 12px** - the same
as step 3. So two of the three frames say 12 and step 2 says 8; the build moves
to 12 and step 2 is the outlier, not the rule.

Step 4's title, subtitle and note copy match the build verbatim. **One thing
the design does not draw**: the build closes the step with "That's all there is
to it. Sign in and your first app is one toggle away." The frame has the
subtitle and then the illustration, nothing between. Left in place and raised
rather than deleted - it is a sign-off before Get started, and removing user
copy is the designer's call, not this pass's.

**Not changed.** Step 1 keeps the config-versus-proxy mechanism paragraph the
design drops and step 2 keeps its platform-aware sub-heading, both for the
reasons recorded on 2026-08-19. The design runs its step-1 subtitle and first
body paragraph together as one text node; ours keeps them as two paragraphs of
the same words at the card's own 8px rhythm. The welcome frame's ground is a
white-to-gray-50 gradient where the other three are flat gray-50, left flat.
The primary button's `blue-ribbon-500` hairline and the Button component's
inset highlights are shell-wide treatments nothing in the new UI reproduces,
same call as on the Settings pane. 548 unit tests and the 169-test e2e suite
pass.

### Sync 2026-08-26, third pass: every dialog in the file, measured

The Settings main screen was already right - `116:28972` matches row for row
after the morning's pass - so this one went after the dialogs, which the morning
deliberately did not read. `Flows / Settings` (116:28963), `Flows / Overview`
(116:26381) and `Flows / App` (116:30199) were mapped, then eight dialog frames
read in full. All three pages carry the check.

**One glyph was still wrong on the main screen.** Login ID draws
`Icon / UserRound`; the build had lucide `User`. `Icon.tsx` gained `userRound`,
geometry consistent with the `usersRound` already there. `user` stays in the
palette unused, the same way `receipt` and `power` did after the morning's five.

**Dialogs are 16px, everywhere.** Every dialog frame on all three pages -
`modal/organization`, `card/organization`, `dialog/device`, `card/choose-model` -
is 24px padding, 24px gap, white, 1px `base/border`, `shadow/lg`, radius **16**.
The build was on `rounded-xl`, which this repo's scale makes 14px. CLAUDE.md's
"12px modal radius LOCKED" line is rewritten in the same pass; it was the last
of the four conflicts in §3 still standing.

**Width is per dialog, and the file means it.** Four values, so `narrow` (a
boolean for one invented 520) is gone and `Modal` takes `width`:

| Width | Dialogs |
| --- | --- |
| 480 | Rename device `143:67735`, Replace API key `177:74869`, Disconnect Gate `143:70617` |
| 512 | Switch organization `130:55314`, Organization switched, Change is ready `134:61659`, Use a Gate model `130:48278` |
| 544 | Reset Gate Connect `177:74223`, alone |
| 600 | Review config `130:57442`, Apply changes, Close apps, Diagnostics report `363:9027`, Choose model `665:18400` |

Undrawn dialogs - Switch gateway, the OAuth offer, Collected data, Restore
details, both quit dialogs - keep the 600 default.

**The tone tile is a bordered gradient, not a flat 100.** 44x44 at radius 8
with a 24px glyph on a toned dialog, 40x40 with a 20px glyph on a neutral one,
`shadow/2xs` under both: warning `#FFFBEB`→`#FDE68A` over `#FCD34D`
(`130:57444`), success `#F0FDF4`→`#BBF7D0` over `#86EFAC` (`134:61661`), danger
`#FEF2F2`→`#FECACA` over `#FCA5A5` (`177:79233`), neutral white over
`base/border` (`451:8038`). The build had one 48px flat-100 tile for all four.
The glyph's own fill is not in the export, so the existing ink stayed.

**Header and buttons.** Title is `heading/18` (18/24), not 20; it sits 12px
from the tile, not 16; the subtitle is `base/muted-foreground` and hangs
directly under the title with no gap. Header to body is 24, matching body to
buttons. Buttons are radius **8** - the Button component set itself
(`685:20928`, `685:20942`) says 8 at both sizes, and the Overview and App
instances agree; only the older Settings dialog instances still carry 4. Side
padding is 12, not 16, and the secondary button's label is `base/primary`, not
neutral-900.

**Not changed, and why.** The Settings pane's own row buttons keep radius 6 and
a `base/border` line: every drawn instance on that screen says 4 (which this
repo's scale rounds to 6), and the component-versus-instance conflict is worth
settling for all buttons at once rather than for one pane. The destructive
button's label stays white over the drawn `#FEF2F2`, as `Modal` already had it.
The disconnect dialog's card carries a `red-600/40` border where reset's carries
grey; one of the two is a slip, so both stay grey. The page title's 20/24 is
left at the token export's 20/28 for the reason `tailwind.config.ts` records at
`letterSpacing.heading`. Action-button labels still take no `-0.01em`.

554 unit tests and the 171-test e2e suite pass.

### The Families pane is retired, 2026-08-26

The pane existed because the rail could not express routing: it listed config
tools flat, with no master switch, no family switches and no rows for the chat
domains. Three of those four have since been fixed in place - proxy members
became rail rows on 2026-08-23, and the eyebrows already carry a
protected-over-total counter - which left the pane holding two controls and a
duplicate of every row beside them. Two surfaces for the same switch is the
condition `lib/groups.ts` warns about from the other direction: it is one more
place for intent and observation to disagree.

So `FamiliesPane.tsx` is deleted, `SidebarView` loses its `families` kind, and
the rail takes both controls:

- **The engine's switch** (`MasterRouting`, moved onto `Sidebar`) sits between
  the nav divider and the app groups, which is exactly what it governs -
  "everything below stays off until this is on" is now literally true of what
  follows it. Laid out label-over-description rather than the pane's
  label-beside-switch: the rail is 256px and the untrusted-certificate line is a
  sentence. Its `envExport` sub-switch follows it under a rule, minus the pane's
  `codeXml` glyph, which there is no width for.
- **The family switch** joins each eyebrow, right of the counter, driven by
  `SidebarGroup.routing` and gated on `onToggleGroup` the way the pane's was
  gated on `onToggleFamily`. It carries the family's own name ("Claude"), not
  the eyebrow's vendor caption ("Anthropic") - a switch named for the vendor
  would claim to govern more than it does - and its intent is still
  `cascadeDesired > 0`, so it never renders on over a set it cannot flip.

Nothing about the dispatch changed: `routeFamily`, `toggleMaster` and
`setEnvExport` are the same calls with the same guards, and the member switches
were already the rail's. What went with the pane is `memberToFamilyMember`, the
`families` memo, and the `Config file` / `Local proxy` qualifier - the rail's
rows have never carried the mechanism, and adding it to fifteen rows to preserve
it on four was not worth the noise.

One e2e assertion moved with it: `getByText("Not trusted")` in the certificate
spec is now `{ exact: true }`, because the master card says "the certificate is
not trusted" in a sentence on every screen. 554 unit tests and the 171-test e2e
suite pass.

### Sync 2026-08-26, fourth pass: the file wins, including where we argued

Standing instruction, given this day: **whatever is in Figma takes precedence
over any local decision.** That is a rule change, not a measurement, and it
reverses most of the "not changed, and why" notes above. What it moved:

**Four variables settled four arguments.** `get_variable_defs` on `116:28972`
and `143:67735` returns the tokens by name, which is stronger evidence than a
hex on a frame:

| Variable | Value | What it overturns |
| --- | --- | --- |
| `base/foreground` | `#030712` | The shell-wide `neutral-900` convention. 79 call sites across 13 files. |
| `base/primary-foreground` | `#f9fafb` | `text-white` on a filled primary. |
| `base/destructive-foreground` | `#fef2f2` | `text-white` on a filled destructive, kept twice on the argument that `Modal` already had it. |
| `custom/destructive\40` | `#dc262666` | "One of the two is a slip, so both stay grey." It is a *named* variable, so the Disconnect dialog's red edge is intent. `Modal` gained `edge`. |

**The Button component set is now reproduced, moulding and all.** `685:20855`
defines two sizes and nothing else: `default` at h36/10-12 and `sm` at h32/8-12,
both `rounded-md`, bordered `base/input`, gap 8, and each carrying a drop shadow
plus an inset pair - a dark bottom lip and a light top edge. That pair is what
the plan had been calling "a Button-component treatment nothing in the new UI
reproduces yet", twice. Four `boxShadow` tokens now carry it, the filled primary
takes its white/black 8% gradient, and the ad-hoc pills that were `px-2 py-1`
with a 12px glyph became `sm` with a 16px one - Overview's Manage link, three in
`AppPane`, the rail's inventory retry. Two `letterSpacing` tokens carry the
label tracking (-2% at 14px, -1% at 12px) that had been dropped.

**Three copy corrections were reversed with the rest, and two were reversed
back the same day** - decided, not re-argued, and now recorded in CLAUDE.md so
the next pass does not undo them again:

- **Replace API key** keeps **"New API key"**. `177:74869` labels the field
  "New device name", copy-pasted from the rename dialog; the drawn label would
  put a wrong word on the one screen where the user handles a credential.
- **Disconnect Gate?** keeps its own body. `164:73502` reads "Protection turns
  off, your apps stop routing through Gate, and your API key is removed from the
  keychain", which describes Reset - the row below it on the same screen.
  Disconnecting ends the session and touches no keychain item.

Both are the same failure mode: the frame's words describe a *different action*
on the same screen. Everything else the file says about those two dialogs - the
480px width, the danger edge, the `base/foreground` ink on that paragraph -
stands; only the sentences are ours.

**The third stood as drawn until AG-578 landed**: the Notifications row said
"Alert me when a request is blocked or flagged" while the events behind it needed
a live security feed that did not exist, so the row promised more than it fired.
Left as the file had it at the time, unlike the two above. Resolved 2026-08-31,
and not by editing the copy: the feed was built, and the drawn sentence moved to
the Blocked/Flagged rows it was describing.

**Smaller, all measured.** The Settings page title takes the frame's 20/**24**
rather than the token export's 20/28. The rail's right edge is `base/border`,
not the 5% black an early frame read gave it - the `sidebar` component (437:161)
names the variable. The app tile's overlay pair is 24%, not 32% (`408:14180`).
`ModalField`'s label sits 8px above its input, the input carries `shadow/xs`,
and the read-only one is transparent at 60% with no shadow, as `143:67746`
draws it.

**Where the file disagrees with itself**, recorded so the next pass does not
re-litigate: the Settings dialogs draw radius-4 buttons and 32px neutral tone
tiles where Overview and App draw radius-8 and 40px, and the Button component
says 8. Component over instance, newer node id over older - so 8 and 40 - and
that rule is now written into CLAUDE.md.

**Not done, and not a judgement call.** The rows the build adds and no frame
draws - Sign-in method, Gate certificate, What is collected, and the whole Help
section - are still there. Matching the file exactly would mean deleting the
only route to removing the certificate and to switching an API-key account onto
a Gate account, which is a functional regression rather than a design one. Left
standing for a decision. The onboarding window is likewise still on the `gc/`
ink system; retheming it is its own piece of work, as it was before.

554 unit tests and the 171-test e2e suite pass.

### Sync 2026-08-27: rendered against the frame, not read off it

"It doesn't really look like the design." The four passes of 2026-08-26 all
compared *values*
in the node data; this one rendered `Settings / Dimensions` (191:79795) to PNG,
screenshotted the built pane at the same 1024x720, and sampled pixels. Four
things the node reads had never surfaced, in descending order of how wrong they
looked:

1. **The ground was `gray-100`.** The window frames fill `#F9FAFB`
   (`base/background`); `#F3F4F6` is a full step darker, it sat under every card
   on four panes, and it was the single most obvious difference on screen.
   CLAUDE.md had said `gray-100` since before the Figma existed.
2. **Row icons were `neutral-500`.** Sampled at `#030712` in the render -
   `base/foreground`, the same ink as the label beside them. A 20px grey glyph
   next to full-strength text reads as disabled.
3. **The rail was 256px.** The dimensions frame draws 250 beside a 774px content
   area, and 250 + 774 is the window. The frames reporting 256 hang a 1030px row
   off a 1024px window. At 250 the content column lands on the drawn 726 after
   its 24px pads; at 256 it was 720.
4. **Pane buttons had taken the component's radius and edge.** The fourth pass
   resolved component-over-instance and gave them `rounded-md` on `base/input`.
   The render disagrees: 4px corners on an `#E5E7EB` line, sampled. A `control`
   stop (4px) exists on the radius scale now - the token export names none,
   which is why this had been rounded to `sm` (6px) when the scale was adopted.

**The tiebreak is narrowed** in CLAUDE.md as a result. "Component over instance"
was the wrong rule: what the frame renders is what "looks like the design"
means. The component set now settles only what no frame draws. Buttons are the
worked example - panes draw 4 on `base/border`, dialogs draw 8 on `base/input`,
and both now do.

The method is the point. Node data gives values that are easy to match
individually and still add up to a screen that reads wrong; a render and a
pixel sample catch the ones nobody thought to look up.

554 unit tests and the 171-test e2e suite pass.

### The undrawn rows stop looking undrawn, 2026-08-27

Sign-in method and Gate certificate are ours, not the file's, and they were
wearing a paragraph each - a sentence about the OS secret store, a sentence
about local inspection. `Row` gives any row with a value a fixed 184px label
column, so both sentences wrapped inside a gutter: six lines on one row, four on
the other, on a screen where no drawn row is taller than two.

The proposal on the table was a hover. It was declined for the reason the pane
exists: hiding where the credential lives behind a tooltip is the opposite of
"reassurance comes from transparency". Both descriptions are **removed**
instead, which is what the file does - every drawn row that carries a value
carries no second line, and every drawn row with a second line carries no value.
The Connection card is now four uniform value rows with their values in one
column. What removing the certificate costs is said by the confirmation dialog,
which is where a consequence belongs anyway.

`signInNote` survives as the row's `description`, the way `updateNote` does on
Version: a transient line about what is happening right now, not a paragraph the
row wears at rest.

`Row` keeps the two-shape rule this turned up, because those two notes still
need it: a row with a description takes the full width and puts its value beside
the control, a row with only a value keeps the 184px column. The drawn rows are
unaffected either way - none of them has both - but the next person to add a row
lands in the right shape instead of in the gutter.

554 unit tests and the 171-test e2e suite pass.

### The family switches come off the rail, 2026-08-27

They lasted a day. Retiring the Families pane moved two controls into the rail -
the engine's master switch and a per-family switch on each eyebrow - and the
second one was a third control over traffic that two others already govern: the
row switch under it and the master switch above it. The drawn rail has neither
the switch nor anywhere to put it; the eyebrow holds a label and a counter.

Removed: `SidebarGroupRouting`, `SidebarGroup.routing`, `Sidebar`'s
`onToggleGroup` and `AppShell`'s pass-through, the `routing` field the shell was
building for each group, and `routeFamily`. The eyebrow is back to
`items-baseline` with the counter as its only right-hand element.

**The cascade itself is untouched.** `useRouting.setFamilyRouted` and
`Group.cascadeDesired` stay: the popover's `FamilyPanel` still drives them, and
they keep their coverage in `useRouting.test.tsx` (seven tests on the guards -
certificate first, abort on refusal, name the members that failed) and in
`e2e/routing.spec.ts`'s family-panel block, which is where the
"a family switch never touches a chat or subscription surface" guarantee is
tested end to end. `e2e/new-ui-family-master.spec.ts` is deleted, because it
drove a control that no longer exists and asserted nothing those two do not.

`NewUiApp`'s `FamilyCascadeError` branch is now unreachable from this shell and
says so at the call site. It is cheap to keep and would be needed again the day
a family control is drawn.

554 unit tests pass; the e2e suite is 166, down the five that spec held.

### "What is collected" becomes a link, 2026-08-27

The Diagnostics section had three rows where the file draws two: share the data,
and view the report. The third was AG-603's read-only field list, and it was the
kind of undrawn row that is hard to argue for - a whole row, icon and all, for a
disclosure *about* the row above it.

Removing it outright would have taken the criterion's only surface with it
("What is collected opens the field list without changing the setting"), so it
moved rather than went: `SettingsRow` gained `descriptionLink`, and the
share-diagnostics row now ends its own description with **See what is
collected**. The section is two rows again, the disclosure reads as part of the
sentence it qualifies, and `CollectedDataDialog` opens unchanged - still
read-only, still writing nothing, which is the half of the criterion that
matters.

`descriptionLink` is deliberately narrow: a label and a handler, rendered inline
inside the description paragraph, and it needs a `description` to attach to. It
is the affordance for "and here is exactly what that means", not a general slot.

The two e2e tests move with it - same assertions, `See what is collected`
instead of `View list`.

554 unit tests and the 166-test e2e suite pass.

### Both ways in, 2026-08-20

The popover lets an account be either a pasted key or a Gate sign-in, and lets a
key account move to the second whenever it likes: `screens/Settings.tsx` carries
a permanent **"Switch to Constellation sign-in"** row under the key.

The window shell had the choice at **first run only**. `WelcomePane` offers both
routes and both are wired, but after setup the sole path from a key account to a
Gate account was the one-time `OAuthOfferDialog` - and `markOAuthOfferSeen()`
means dismissing it once removed the route permanently. An install that said "not
now" could never say yes.

`SettingsPane` now carries a **Sign-in method** row directly under the API key,
valued `API key`, whose action is **Use a Gate account**. It is gated exactly as
Replace key is, on `auth_mode === "api_key"`, so an OAuth account is not offered a
switch to what it already is - matching the popover, which offers no reverse
either. While the browser flow is open the row's description says where the user
should be looking, reusing the `updateNote` mechanism rather than inventing a
busy state for `SettingsAction`.

It calls the existing `useSettingsActions.upgradeToOAuth`, **not**
`useSetup.signIn`. That distinction was already documented in the hook and is the
whole reason the action exists: `signIn` saves the account first, with the default
gateway and no key, which is right for a machine with no account and wrong here -
it would repoint a staging install at production and drop the key the user still
has. The e2e test pins it by asserting `oauth_begin_login` fired and
`save_account` did not.

**Not in the Figma.** The drawn Settings screen has no such row; this is parity
with the shipping app, not a design instruction. Expect it to be redrawn, and
treat it the way the diagnostics row is treated.

**A trap this row fell into first.** `upgradeToOAuth` throws on a browser flow
that fails, times out or is abandoned, and the row was calling it through `void`.
The rejection became an unhandled promise, `finally` cleared the busy flag, and
the pane went silent - which is indistinguishable from a button that does
nothing, and is exactly how it was reported. Any call site for one of these
actions needs a `catch` onto `setActionError`, the way `acceptOffer` beside it
always had. The e2e that pins it fails without the fix on *uncaught page errors*,
because the fixture treats an unhandled rejection as a crash.

`settings.busy` also has to be in the `settingsSections` memo's dependency list.
That array names individual callbacks on purpose - the hook returns a fresh
object each render - so a value read inside the memo and missing from the array
simply never updates, which is what kept the waiting note from ever appearing.

### Left undone, deliberately

- **`Auth / Error states` carries no ready mark**, so the setup-timeout dialog and
  the device-name validation rule are unbuilt. The field accepts anything the
  backend accepts.
- **Minimize2 is still a question for the designer.** Onboarding step 2 teaches
  expand/collapse between popover and window; open question 5 removed the control.
  Nothing was changed either way pending an answer.
- **The onboarding window is still on the old `gc/` ink system.** The design draws
  these four steps inside the new window shell - topbar, progress bar, card - and
  what ships is the popover-era layout with new words in it. Retheming it is its
  own piece of work and was not attempted here.
- **"Go back" and "Use a different account" do the same thing** on the org dead
  end, because by then the session is already spent and dropping it is the only
  move. Both are drawn; worth asking whether one was meant to do something else.

### Audit 2026-08-30, and what it corrected 2026-08-31

Four parallel reads of the whole file against the build - Settings, App, Setup +
Onboarding, and the shared component set - written up in
`docs/review-figma-{settings,app,setup-onboarding,components}.md`. Overview and
Tray had been read that same week and are not repeated there. What the audit
turned up and this pass fixed:

**The switch was 11% undersized, everywhere.** The `Switch` set (`408:14253`)
draws a 36x20 track with a 16px knob at a 2px inset; `BaseSwitch` rendered
32x18 with a 14px knob. The 2026-08-26 measurement had been taken off an
instance **scaled 1.125x**, and all four of its numbers divide back exactly
(36/1.125 = 32, 20/1.125 = 17.78, 16/1.125 = 14.22, 2/1.125 = 1.78).
`base.tsx`'s own docstring had said "36x20 track, 16px thumb" the whole time;
only the code below it disagreed. The knob's travel moves with it, to 18px.

**`mono/eyebrow` is 8% tracking, not 10%.** `437:161`'s variables give
`letterSpacing: 8`. `Sidebar.tsx` had been carrying a hardcoded
`tracking-[0.96px]` to get the drawn value, so one Figma variable rendered two
ways - 0.96px in the rail, 1.2px at every site that named the token. The token
moved to 0.96px (and `eyebrow-14` to 1.12px) and the hardcode is gone.

**Tone-tile size follows the dialog's width, not its tone.** The three 480px
Settings dialogs draw a 32px tile with a 16px glyph - `danger` Disconnect
(`143:70620`) included, where the build drew a 44px red one - and the 600px
Diagnostics report is *neutral* yet draws 44px with a 24px glyph
(`363:9029`). The glyph does not track the box either, so `TILE_SIZES` names
both and each drawn dialog declares which it takes. The old tone-derived pair
stays as the default, so every undrawn dialog is untouched.

**Smaller, all measured:** the diagnostics report body is `mono/body-14`, not
the 12/16 it rendered (`363:9120`) - the one screen a user reads a wall of text
on; the rename dialog draws `Monitor` and the diagnostics dialog
`ClipboardList`, both of which the 2026-08-26 glyph sweep fixed on the Settings
rows and never followed into `dialogs.tsx`; dialog body blocks stack at 16px,
not 12; the reset dialog's numbered tiles are 36px; the Settings value column
starts at 233, so its label gutter is 189 rather than 184; and the unavailable
counter renders lowercase **`n/a`** (`272:1728`), which the plan had
transcribed as `N/A` from the same frame.

**The setup rail runs on quarters.** Measured off each pane's `prog-track` fill
against the 1024 track: sign-in 256, API key 512, org picker 512, name device
768, diagnostics 1024. The two second-step panes share a stop because they are
alternatives rather than a sequence. Six invented fractions until now.

**The onboarding art was stale, and step 2's was wrong.** Steps 2 and 3 were
redrawn as live mockups in the `710:*` batch, and the shipped step-2 capture
showed the **retired** popover (Proxy / Direct Gateway rows) inside a desktop
mockup - first run was teaching a UI the product no longer has. Both are
re-exported from `710:36505` and `710:36133` at 3x through
`download_assets`, which the earlier captures could not use. Step 2 loses its
per-platform mockups with the redraw: the frame draws the popover panel itself
rather than its place on a desktop, so the three `where-is-gate-connect-*.png`
files are gone and one asset replaces them. The platform-specific *sentence*
(`whereItLives`) still carries where the icon lives, which is the recorded
deviation it always was.

**Checked and deliberately not changed.** The org picker's header tile is 44px
(`686:23559`) while the name-device pane's is **48** (`209:84721`) - the panes
disagree with each other, so the build's uniform 48 stays and the file gets the
question. `brand=moonshot` exists in the `logo` set with no Moonshot provider
anywhere in the product, so nothing is added for it. Four docstrings citing the
now-deleted Components-page nodes were repointed at the live flow frames rather
than at ids that no longer resolve.

**Left for their own PRs:** the App page, which moved more than any other and
whose findings land squarely on the files PR #208 is reworking - the OpenCode
model card, the enabled-set grid, the picker's copy and checkbox shape - plus
the redrawn Recent activity table, which is blocked on data: its new `Type`
column needs a guardrail category no gateway field carries
(`lib/toolEventRow.ts`). And the org picker's many-orgs scroll, which is a
`SetupLayout` change rather than a value.

### Sync 2026-08-28: the Tray page, and Built the same day

The file gained a nested page under Flows, **`↳ Tray` (694:34005)**, carrying
the ready check: four 400x700 frames redrawing the compact popover in the new
language. Read over MCP (metadata, design context for `Connect/partial`,
variable defs on the routing card and the menu) plus screenshots of all four.

**What the frames draw.** `Connect/partial` and `Connect/routing` are the two
master states; `Connect/full frame` is the unclipped scroll content;
`Connect/menu` opens the footer menu. Anatomy: a 64px header with the **Gate AI
lockup** (the `gate-ai-logo-mark` vector, "Gate" `#002554` / "Connect"
`#3646e7` - the mark's own inks, not tokens) and a `sm` outline **Expand app**
button; a master card (white, r8, p12) with the banner tile recipe at 36px -
amber-50→200 on an amber-300 border with `ShieldBan` for "Partially routed",
the green set with `ShieldCheck` for "Gate is protecting you" - over a muted
"On · 6 of 8 tools routing" line; the rail's groups as bordered cards
(`shadow/xs`, rows divided by rules) under `mono/eyebrow` at **14px** (the
rail's is 12) with the same protected-over-total counter; a collapsed **"Not
installed" section** ("NOT INSTALLED · 8 ˅"); a **Command-line tools** card
carrying the env-export switch with its own copy ("Sets HTTPS_PROXY for your
whole shell, so OpenCode and other terminal tools route too."); and a 56px
footer naming the org beside a 32px ellipsis button whose menu adds **Quit
Gate Connect** (red, `LogOut`, no external glyph in the render) to the
topbar's dashboard/docs pair. Rows are drawn twice: 64px three-liners
(name, status with qualifier, "345 messages · 23 alerts" / "No recent
messages" / "Off") and 40px two-liners on the routing frame's Other tools.
The master card's switch is drawn at **opacity 0** in every frame. The frame
window itself is r16 under `shadow/md` on a dark backdrop.

**Built as a third window.** `tauri.conf.json` declares `tray` (400x700,
undecorated, always-on-top, skip-taskbar, hidden); `main.tsx` routes on the
label, with `?window=tray` as the plain-browser fallback the e2e suite and dev
use. `components/gc/Tray.tsx` is the surface (header, master card, groups,
not-installed, CLI card, footer + menu, dialog slot), `TrayApp.tsx` the shell:
its own slim load + 5s detection poll, and the same `useRouting` /
`useRunningApps` dispatch as the window, with the drift review, trust prompt
and close-apps dialogs rendered in-window (`Modal` is `max-w-full`, so the
480-600 widths shrink to fit). `proxyMemberStatus` moved to `lib/verdict.ts`
so the rail and the tray derive a domain's line from one function.
`GateAiLogoMark` holds the exported vector; `Icon` gained `users`, `expand`,
`chevronDown`. Rust: tray left-click now toggles the tray window (plain
`hide()` everywhere - the Linux minimize dance protected decorations this
window does not have), the menu gained **Quick status** (Linux trays never
fire the click path, so without it the flow would be unreachable there),
`request_app_quit` exposes the tray-menu quit to the popover's own Quit entry,
and the c63e1880 anchoring helpers (`anchor_under_tray`, `monitor_at`,
`anchor_at_cursor`) are resurrected verbatim, scoped to this window - it is a
popover again, so placement is correct here. The Linux decoration repair in
`on_window_event` is now gated to the main window so the tray cannot consume
its pending flags and get resized to the main window's bounds.

**Deviations, each recorded at the component:**

- **The master card renders no switch** - matching what the frames render
  (opacity 0). If that was reserved space, the designer should say so.
- **No activity line on rows.** No per-tool reading exists (the endpoint is
  org-scoped and rate-limited per address), and "No recent messages" is itself
  a measurement claim. Rows take the drawn two-line shape until a reading
  exists; the drawn counts are recorded above for whoever lands it.
- **Master off / none-routing state inferred** ("Not protected" + the drawn
  sub-line format) - the page draws only partial and full.
- **Signed-out state inferred**: a hand-over card to the full window, where
  setup lives.
- **Not-installed rows when expanded are inferred** - only the collapsed
  header is drawn - and carry no switch: a connect would materialise a config
  for a tool the user does not have.
- **Contact support stays omitted** from the menu (same reason as `Topbar`),
  and the Quit entry drops the external-link glyph its metadata carries
  because the render does.
- **Switch size is the component set's 32x18** (`BaseSwitch`); the tray frames
  disagree with themselves (36x20 on config rows, 32x17.78 on Other tools).
- **The window ships square-cornered and opaque.** The drawn r16-on-shadow
  needs per-platform transparency work (the old popover's CALayer path);
  deferred, recorded here.
- **Blur-to-dismiss is not built.** Click-away is the popover convention, but
  it needs the pin coordination around OS dialogs (cert trust) and a
  tray-click race guard, none of it verifiable from this machine. The tray
  icon toggles; Expand and Quit hide.
- The master sub-line uses single spaces around its dot; the frame draws
  doubles, which HTML collapses anyway.

Covered by `Tray.test.tsx` (16 tests: the three master states, counter
derivation, intent-driven switches, collapsed/expanded not-installed, CLI
dispatch, menu contents, hand-overs) and `e2e/new-ui-tray.spec.ts` (the
fake window label is `BackendState.windowLabel`; pins that a row switch
reaches `connect_tool`, Expand reaches `reveal_popover`, Quit reaches
`request_app_quit`, and signed-out hands over). 628 unit tests and the
190-test e2e suite pass; `cargo check` and `fmt` are clean, with the
macOS/Windows anchor branches reasoned about, not compiled, same as c63e1880
noted when it deleted them.

### Sync 2026-08-28: the Overview page, and the quit flow drawn at last

Re-read `Flows / Overview` (116:26381) over MCP, the first look since
2026-08-21. The page still carries the check and its sections are unchanged,
but three things moved, all in the same batch of node ids as the Tray page
(694:*) or later (706:*, 751:*).

**The quit flow is drawn**, in three new frames under a section whose label is
a copy-paste of the routing-ON section's ("Overview / Turn routing ON for an
app that was OFF", `694:33533` - worth a designer fix, since it is how the flow
is catalogued). It replaces the three-button dialog AG-596 shipped:

- **Step one, the chooser** (`694:32272`, 600px, warning tile): "Quit Gate
  Connect?" over "**N** protected apps are still routed through Gate", the
  count in Medium inside a regular sentence. Then "Select how you want to quit
  the app" and two selectable rows - "Disconnect tools and quit" / "Restore
  saved configurations, turn routing off, then quit." carrying a green
  **SAFEST** pill, and "Quit without disconnecting" / "Leave configurations
  pointed at Gate. Requests that depend on the local proxy may pause." Under
  them a `blue-ribbon/50` note: "**Closing the main window is a different
  action.** You can safely **minimize** this app to the menu bar to keep
  protection running quietly." Buttons Cancel / Disconnect.
- **Step two, the confirmation** (`694:33002` and `694:33340`, 536px, success):
  "Safe to close Gate Connect" over a `base/background` note that reports the
  branch - "Tools are disconnected and their previous settings are restored.
  Setup will be waiting the next time you open the app." or "Routing settings
  were left in place. Some tools may need Gate Connect running to complete
  requests." Buttons Cancel / Close Gate Connect.

So the exit moved: step one *carries out* the choice and step two is what
actually quits, which is what lets the confirmation speak in the past tense.
Built that way. **AG-596 survives the redraw and is the reason the flow is not
just two dialogs**: a teardown that leaves a tool behind goes to
`QuitLeftBehindDialog` instead of the confirmation, whose "their previous
settings are restored" would be exactly the claim the ticket forbids. The
left-behind dialog is still undrawn.

`Modal` gained `ModalChoice` (the selectable row: title, consequence, optional
pill - distinct from `ModalOption`, which leads with an org avatar), two more
`ModalNote` tones (`info` and `neutral`, both the drawn bordered 12px box),
a `tile="sm"` for the confirmation's 32px/radius-6 tile - a *toned* dialog at
the small size, so tile size does not follow from tone after all - and
`subtitle` now takes a `ReactNode` for that Medium count. `ModalWidth` gained
**536**, which is the one value worth questioning: both frames sit with their
left edge exactly where a *512* dialog centred in 1024 would start and their
right edge 24px past centre, which reads as a stretched edge rather than a
chosen number. Shipped as drawn, raised as a question.

**The topnav menu gained Quit** (`116:27225`, now 4 items at 146px): red ink,
`LogOut`, and no external-link glyph, since it is the one entry that does not
leave the app. Wired to the same chooser, with the routed tools derived from
`tools` rather than swept from the backend buffer - that buffer only fills when
the *tray* deferred a quit. With nothing routed it exits outright, which is the
rule Rust's own `request_quit` already follows. Contact support stays omitted
for want of an address, as everywhere else.

**The Messages chart was redrawn** (`706:*`, ~1,950 nodes) and three things
changed:

- **Ticks read `HH:00`**, not a bare hour. `hourTick` now formats them, and the
  tooltip heading and the accessible table go through it too, so one bucket
  cannot be phrased three ways. The `chart/tooltip` card itself is an older
  node (`191:*`) still headed with a bare hour; the redrawn axis around it
  wins, and the test that pins the heading is better for it - "12:00" cannot be
  mistaken for the 8+2+2+0 sum the way "12" could.
- **The legend sits under a full-width rule** in the loaded state now, which
  retires the discrepancy 2026-08-21 left alone (only the loading frame drew
  one). Measured with it: legend items at 24px, swatch-to-label at 8px, and the
  label ink is `base/foreground`, not the muted grey the build had.
- **The drawn chart has 12 hourly columns** (00:00-11:00) under a header that
  still says "Last 24 hours", and the endpoint's contract is 24 dense hourly
  buckets. Not followed: the column count is data, and reshaping the client to
  12 would misreport the reading. Raised rather than built. The placeholder
  keeps its 24 positional ticks and only widens its tick box to the loaded
  axis's, so the axis cannot jump sideways when a reading lands.

`dialogs.quit.test.tsx` pins the two dialogs' copy and the primary's label
following the selection; `new-ui-quit.spec.ts` follows the new flow and gains
the menu entry's two cases. 620 unit tests and the 188-test e2e suite pass.

**The popover keeps the old three-button quit** (`components/QuitConfirm.tsx`).
The two shells disagree until the popover retires, the same standing divergence
as the org-picker dead end - this is the drawn one.


## Branching model

`feat/new-app-ui` is the **integration base** for the design work, not a branch
heading for `main`. PR #151 stays open as work in progress; every PR that
implements a piece of the design targets this branch rather than `main`.

    git fetch origin
    git switch -c feat/wire-routing-actions origin/feat/new-app-ui

Consequences worth keeping in mind:

- **Keep it green.** Downstream branches inherit whatever is broken here, so a
  red base costs everyone. CI covers macOS, Windows and Linux Rust plus the
  frontend; all eight checks were green as of 2026-08-17.
- **Merge `main` in periodically.** The longer this diverges, the worse the
  eventual reconciliation, and every downstream branch inherits the drift.
- **The popover stays until routing works.** It is the only surface that can
  change what is routed, so it is load-bearing, not dead weight. The e2e suite
  is pinned to it (`VITE_NEW_UI=0` in `playwright.config.ts`).
- **This plan is the shared roadmap.** "Still to do" below is the queue that
  downstream PRs draw from; keep it current rather than tracking work elsewhere.
- **Green CI here means less than it looks**, though less so than it did.
  Routing, Settings and the routing verdict are wired and carry their own e2e
  specs (`new-ui-routing.spec.ts`, `new-ui-settings.spec.ts`,
  `new-ui-verdict.spec.ts`), which opt into the new shell per-test through
  localStorage. Everything listed under "Still to do" is
  still covered by nothing.

## How the designer marks readiness (from Chad, 2026-08-14)

- **A white check mark in a section title above a flow means that flow is
  ready.** Sections without one are still moving.
- **Every main frame is named for the step it represents**, deliberately so MCP
  reads and this work can tell which state a frame is. Trust the names.
- **Hidden layers were stripped and 2,000+ layers renamed** with Figma's own
  agent, because MCP struggles with hidden layers. Reads after 2026-08-14 are
  against the cleaned file.
- **The designs are frozen as of 2026-08-14**, with one exception: dialog
  states are still being worked on. Do not chase dialog changes until the
  designer says they are done - `Modal.tsx` and `dialogs.tsx` are the parts most
  likely to move.

## Reading the file without MCP quota

The Figma MCP server allows only **6 read calls per month** on a View seat.
When exhausted, read the file through the browser instead. Two things that do
NOT work: mouse-wheel `scroll` never reaches the WebGL canvas, and clicking the
canvas selects a nested child so `shift+2` zooms to the wrong thing.

What works: **click the frame in the Layers panel, then press `shift+2`**
(zoom to selection). Switch pages by clicking them in the Pages list, then
`shift+1` to fit. Allow ~10s after a page switch; the first
`Page.captureScreenshot` after navigation often times out at 30s, so wait and
retry once.


## The three forks that gated implementation (all RESOLVED)

Decided 2026-08-14: **the Figma wins on all three.** The app becomes a
1024x720 window, adopts the shadcn-flavoured token names, and CLAUDE.md's
Aesthetic Direction gets rewritten rather than obeyed. Detail kept below for
the record.

### 1. Window model: popover → window

`src-tauri/tauri.conf.json` today is a 380×620 `resizable: false`,
`decorations: false`, `alwaysOnTop: true`, `skipTaskbar: true` menubar
popover. The design is 1024 wide with **macOS traffic lights** at
top-left (113:16764, three 12px ellipses) and a **Minimize2** button at
top-right (113:16774).

That is a regular application window, not a popover. Deciding this
rewrites:

- `tauri.conf.json` - width, `resizable`, `alwaysOnTop`, `skipTaskbar`
- `src-tauri/src/lib.rs` - transparent window + CALayer `cornerRadius`
- `src/components/LinuxTitleBar.tsx` - already draws its own chrome
- `pinPopover` / `unpinPopover` in `src/lib/api.ts` - popover-only concepts

Open sub-question: what does Minimize2 do? Plausibly "collapse back to
the menubar popover", which would mean keeping *both* shells.

### 2. Design system: bespoke `cg/` → shadcn/ui

The returned tokens are shadcn CSS variable names on the default Tailwind
palette:

```
--base/card        white
--base/background  #f9fafb
--base/border      #e5e7eb
--base/input       #d1d5db
--base/primary     #203de2   ← blue-ribbon/700
--base/muted-foreground #6b7280
tailwind-colors/neutral/{50,600,900}
tailwind-colors/amber/600  #d97706   "Not protected"
green                      #16a34a   "Protected"
```

The Button frames link to `ui.shadcn.com/docs/components/button`. The repo
has **no** shadcn, radix, cva, tailwind-merge, clsx, or lucide dependency,
and runs a bespoke OKLCH ink system in `tailwind.config.ts` + `.impeccable/design.json`.

### 3. CLAUDE.md conflicts

The project's own `CLAUDE.md` Aesthetic Direction contradicts the design
on four locked points:

| CLAUDE.md says | Figma does |
| --- | --- |
| "Primary is ink-900, **never blue**" | `--base/primary #203de2` on active nav, switches, links |
| "Brand indigo is reserved for the logo glyph only" | Wordmark "Gate" in `#1d37b6`, blue switches |
| "**Shadow-as-border.** No 1px solid borders on cards" | 1px `--base/border` on sidebar, org switcher, topbar, nav item |
| "12px modal radius **LOCKED**" | 4px and 8px radii throughout |
| Status pill is "the single most important pixel" | Per-app toggle **Switch**, pill gone |
| Light surface, ink-on-paper | `banner/update` is dot-matrix navy |

If the Figma wins, CLAUDE.md's Aesthetic Direction section needs rewriting
in the same PR, otherwise every future session fights the design.

## Copy / model changes

- "Protected" / "Not protected" replaces the connected/routed vocabulary.
- Relative timestamps per app: "Protected - 2m ago", "- 25s ago".
- Sidebar counter `PROTECTED APPS  4/4` (mono eyebrow, 12px, tracking 1.2px).
- Banner right-rail: `Routing · 2 of 4 Apps` / `Partly routed · 0 of 4 Apps`.
- Alert copy: "Codex isn't protected / It's config changed outside Gate, so
  its traffic isn't routed. Reconnect to restore protection."
- `topnav/menu`: Visit dashboard, Contact support, Read Gate docs.

## Reuse inventory

Already in `src/components/gc/`, reusable:

- `ui.tsx` - `Button`, `IconButton`, `Switch`, `CardButton`, `SectionLabel`
- `Icon.tsx` - 16 glyphs: shieldCheck, layers, settings, chevronRight,
  caretRight, chevronLeft, cube, key, copy, eye, eyeOff, check, refresh,
  trash, info, x, plus, search, logOut, book
- `PopHeader.tsx`, `ConstellationHexMark.tsx`

New glyphs the design needs (lucide names): `UsersRound`, `ChevronsUpDown`,
`LayoutDashboard`, `Settings2`, `ShieldBan`, `TriangleAlert`, `Headset`,
`BookOpenText`, `SquareArrowOutUpRight`, `Ellipsis`, `Minimize2`.

## Navigation rewrite

`src/App.tsx` uses a depth-ranked push/pop stack (`Screen` union +
per-screen `depth`) because "the popover is one room". The design replaces
that with a **persistent sidebar + swapped content pane**. The depth
metaphor and the slide animations become dead code.

Screens affected: `Home.tsx` (972 lines), `Settings.tsx` (825),
`GroupMembers.tsx` (660) - plus their tests (`Home.test.tsx` 822 lines,
`Settings.test.tsx` 310, `GroupMembers.test.tsx` 406) will break on any
DOM restructure.

## Verified token values

Sampled from Figma's properties panel (readable on a View seat, no MCP quota).
The design uses **Tailwind's default palette** throughout, so most of these need
no new tokens - the config uses `extend`, leaving `neutral`/`amber`/`green`/`gray`
intact.

Primary ramp (needs config entries, `blue-ribbon`):

| Stop | Hex | Used by |
| --- | --- | --- |
| 700 | `#203DE2` | switches, active nav, links |
| 800 | `#1D37B6` | "Gate" wordmark, update banner gradient start |
| 900 | `#172563` | update banner gradient end |

Status tile - one shape, two palettes. 28x28, `rounded-base` (4px), 4px padding,
1px border, vertical gradient from the 50 stop to the 200 stop, icon stroked at
the 700 stop:

| Variant | Gradient | Border | Icon |
| --- | --- | --- | --- |
| protecting | `green/50 #F0FDF4` -> `green/200 #BBF7D0` | `green/300 #86EFAC` | `green/700 #15803D` |
| partly routing | `amber/50 #FFFBEB` -> `amber/200 #FDE68A` | `amber/300 #FCD34D` | `amber/700` |

Banner geometry:

| Banner | Size | Padding | Fill | Bottom border |
| --- | --- | --- | --- | --- |
| `banner/update` | 1024 x hug 48 | 12 / 16 | gradient 800 -> 900 + dot matrix | 1px |
| `banner/routing` | 1024 x 48 | 8 / 16 | `base/card` #FFFFFF | 1px `base/border` #E5E7EB |
| `banner/partly-routing` | 1024 x 48 | 8 / 16 | `base/card` | 1px `base/border` |

All are `flex-row`, `justify-between`.

## Implementation plan

### Phase 1 - Token layer (DONE)

`tailwind.config.ts`: `base.*` semantic colors, `blue-ribbon` ramp,
`shadow-base-{2xs,xs,lg}`, `rounded-base`, `tracking-eyebrow`,
`text-base-{2xs,xs}` in rem. Add `blue-ribbon.900` when the update banner lands.

### Phase 2 - Sidebar (DONE)

`src/components/gc/Sidebar.tsx` - `Sidebar`, `SidebarView`, `SidebarApp`,
plus private `OrgSwitcher`, `NavItem`, `AppRow`, `AppSwitch`.
`Icon.tsx` gained `usersRound`, `chevronsUpDown`, `layoutDashboard`, `settings2`.

### Phase 3 - Banners (DONE)

New `src/components/gc/banners/`:

- `StatusTile` - the shared 28x28 gradient tile, variant `protecting | partly`.
- `RoutingBanner` - white, status tile + message, right rail
  `Routing · 2 of 4 Apps` (green) or `Partly routed · 0 of 4 Apps` (amber).
  Replaces nothing; this is new.
- `UpdateBanner` - the navy gradient + dot matrix, "Update available - v0.5.0",
  outline `Update` button, dismiss X. Retargets the slim-banner branch already
  inside `UpdatePanel.tsx` (which today also owns a startup takeover).
- `AlertBanner` - amber card, 36x36 tile, `TriangleAlert`, title + body, a
  switch and a dismiss X. Two variants: `single-app`, and `multiple-apps` which
  adds prev/next chevrons to page between apps.

New glyphs needed: `shieldBan`, `triangleAlert`, `refreshCw`. `shieldCheck` and
`x` already exist.

### Phase 4 - Topbar and window chrome (DONE)

`src/components/gc/Topbar.tsx` - `Topbar`, `TopnavMenu`, `TopnavAction`, plus
private `WindowControls`, `TrafficLight`, `OutlineIconButton`. Glyphs added:
`ellipsis`, `minimize2`, `squareArrowOutUpRight`, `headset`, `bookOpenText`.
Reuses the existing `ConstellationHexMark` at 24px - its aspect ratio (0.8628)
matches the Figma lockup's 20.708x24 exactly.

**Window controls are the system's, not ours.** The design draws its own
traffic lights (and draws them green / yellow / red, the reverse of macOS);
decided 2026-08-14 to drop them and let the OS render real ones. The topbar
keeps a 60px `aria-hidden` spacer where they sat, so the brand lockup holds its
drawn position (504px of 1024) and never slides underneath them.

This makes the Phase 9 window config load-bearing, because today
`tauri.conf.json` sets `decorations: false` and no controls exist at all:

- **macOS** - `decorations: true` with `titleBarStyle: "Overlay"` and
  `hiddenTitle: true`, so the OS paints the traffic lights over our topbar.
- **Windows** - system controls sit top **right**, not left, where our Ellipsis
  and Minimize2 buttons already are. The 60px left spacer is wasted there and
  the right cluster will collide. Needs a platform-conditional layout.
- **Linux** - `LinuxTitleBar.tsx` already draws its own chrome; reconcile.

### Phase 5 - Overview pane (DONE)

`src/components/gc/Overview.tsx` - `Overview` plus the types the 24-hour
backend will need to satisfy (`OverviewStats`, `MessagesBucket`, `Policy`,
`PolicyAction`, `Saving`); private `StatTiles`, `Stat`, `MessagesChart`,
`PolicyTable`, `SavingsTable`, `StatusPill`, `ManageLink`. `Card` went into
`base.tsx` since the Settings pane uses the same surface.

Chart is hand-rolled from stacked divs, no chart library: the mark is four
rectangles, and hand-rolling keeps the series on design tokens. Bars are the
design's 20px and distribute across the card, so 24 buckets or 6 both work.

Hover shows `chart/tooltip` (`191:79768`): a 200px-wide white card, 1px
`base/border`, r8, `shadow/md`, p8, heading in `mono/eyebrow` at 14px (10%
tracking, hence the `tracking-eyebrow-14` token alongside the 12px one), then
the four legend rows with right-aligned values. Anchored by percentage across
the plot area rather than by measuring a bar, and flipped to the left of the
cursor over the last third so it stays inside the card. Hover-only and inside
the `aria-hidden` subtree on purpose: the sr-only table already carries the same
figures, so exposing both would announce every hour twice.

Verified against Figma: pane `gray/100`, card Fill 726 / r8 / `#E5E7EB` /
`shadow/sm`, bars 20px, legend swatches 12x12 r2.

Measured values worth keeping:

- pane padding 24, gap between cards 16
- stat tiles are one card of three columns divided by `border-l`
- action pill label is `mono/label-12` (Geist Mono Medium 12/16, 6% tracking)
- pane title and section captions are `heading/20` (Geist Medium 20/24, -1%)
- window frame radius is 8px, not the popover's 12px

Two values were **inferred, not sampled**: the action pill backgrounds
(`red/amber/purple-100`) and the On pill (`green-100`). Text sits at the 900
level in Figma and 100/900 is the standard Tailwind badge pairing, matching the
50 -> 200 tile gradients, but they are worth a spot check.

### Phase 6 - Settings pane (DONE)

`src/components/gc/SettingsPane.tsx` - `SettingsPane` plus `SettingsSection`,
`SettingsRow`, `SettingsAction`; private `Row`, `ActionButton`. Named
`SettingsPane`, not `Settings`, so it does not read as a drop-in for
`screens/Settings.tsx`, which stays until the shell swap.

Rows are uniform enough across all six sections that the pane takes a
declarative section/row model rather than a prop per field. Reuses `Card` and
`BaseSwitch`.

Button vocabulary, read off the design:

- **outline + blue label** for Rename, Copy, Upgrade plan, Replace key,
  Replay tutorial, Check for updates
- **filled red** for the two destructive ones, Disconnect and Review reset
- Startup rows pair an "On"/"Off" label with the switch
- values that are identifiers render mono: install ID, API key, version

Nine glyphs added: `monitorSmartphone`, `idCard`, `user`, `receipt`, `globe`,
`link`, `power`, `bell`, `codeXml`.

The danger card's `red-50` fill and `red-200` border are **inferred** from the
red section heading, not sampled.

Still outstanding: `screens/Settings.tsx` (825 lines) and its 310-line test are
untouched - they get retired in Phase 9, not here.

### Phase 7 - App detail pane (DONE)

`src/components/gc/AppPane.tsx` - `AppPane` plus `ModelChoice`, `GateModel`,
`ActivityEntry`, `ActivityStatus`, `ActivitySecurity`; private `ModelSelection`,
`ModelOption`, `InfoRow`, `RecentActivity`, `Pill`.

The stat card and Messages chart are identical to the Overview's, so they moved
to `src/components/gc/metrics.tsx` (`StatTiles`, `MessagesChart`, `UsageStats`,
`MessagesBucket`) and both panes import them. `OverviewStats` was renamed
`UsageStats` in the move - nothing consumed it yet.

Structure: header (app tile, name, protected status, On + switch), stat tiles,
Messages chart, Model selection, Recent activity.

Model selection is a two-option radio group (App default / Gate model), the
selected one carrying a `base-primary` border and a `circleCheck`. Below it,
"Current Gate model" with vendor + mono model id and a `Change model` button,
then Gate credits with `Add credits`.

Recent activity is Time / Status / Security / Conversation / Action, with mono
uppercase pills - status `success | error`, security
`allow | flagged | redacted | blocked` - and a `View` button per row.

Glyphs added: `circleCheck`, `creditCard`.

The **Gate model option's icon is a placeholder** (`layers`). The design draws a
three-node cluster that reads as brand art rather than a lucide glyph;
`ModelOption` takes a `ReactNode` so the real mark can be passed without
inventing one.

### Phase 8 - Modals (DONE)

`src/components/gc/Modal.tsx` - `Modal`, `ModalSubject`, `ModalNote`,
`ModalOption`, plus `ModalTone`, `ModalButton`, `PillTone`.

All seven dialogs in the file are drawn from one template, so this is one
component with slots rather than seven near-copies:

    tone tile + title + subtitle
    optional subject card (icon, name, description, status pill)
    optional note block
    right-aligned secondary / primary buttons

600px wide, 12px radius, centred over a scrim. Reuses the repo's existing
`useFocusTrap`, pointing initial focus at the *secondary* button whenever the
primary is destructive - the hazard that hook's own comment calls out.

The seven, and how they map onto the template:

| Dialog | Tone | Subject pill | Buttons |
| --- | --- | --- | --- |
| Switch organization | neutral | (radio list instead) | Cancel / Switch organization |
| Organization switched | success | - | Done |
| Review `<app>` configuration | warning | `DETECTED` amber | Keep existing config / Replace config and protect |
| Apply changes to running apps | warning | `OPEN` green | Close affected apps / I will reopen later |
| Close affected apps now? | warning | `OPEN` green | Go back / **Close `<app>`** (red) |
| Change is ready | success | - | Done |
| Use a Gate model for `<app>`? | neutral | `PAYG` | Keep App default / Use Gate credits |

Note that "Apply changes to running apps" makes the *less* destructive option
primary: `I will reopen later` is the filled button, `Close affected apps` the
outline one.

`src/components/gc/dialogs.tsx` holds the seven concrete dialogs -
`SwitchOrganizationDialog`, `OrganizationSwitchedDialog`, `ReviewConfigDialog`,
`ApplyChangesDialog`, `CloseAppsDialog`, `ChangeReadyDialog`,
`UseGateModelDialog`. Copy lives with them rather than in the shell, so it stays
next to the design it came from and the shell supplies only names and handlers.

`ModalSubject` grew a `variant`: `subject` names a thing and describes it (bold
name over grey detail), `identity` inverts that for the model row where the
vendor is a quiet label above a mono model id. `Modal` grew `subtitleTone`,
because the Gate-model dialog is the one place the design states its cost
consequence in `base-primary` rather than grey.

`OrganizationSwitchedDialog`'s copy was read at low resolution and is the least
certain of the seven.

### Phase 9 - Shell swap (PARTIALLY DONE)

`src/components/gc/AppShell.tsx` composes chrome, banners, sidebar and a dialog
slot. The open pane arrives as `children`: routing is a one-line switch on
`view.kind` at the call site, and folding it in would force the shell to accept
every pane's data. The dialog is a slot for the same reason - the shell owns
*that* a dialog covers the window, the caller owns which one.

Nothing below is done yet, and all of it changes what ships:

- `tauri.conf.json`: **DONE 2026-08-17**, resized 2026-09-04. 1280x800 on a
  1024x800 floor, `resizable: true`,
  `decorations: true`, `alwaysOnTop` and `skipTaskbar` off. `visible: false`
  is unchanged: the tray still owns the first show.
- **The new UI is now the default** (`newUi.ts`); the popover is the fallback,
  reachable with `gcNewUi(false)` or a `VITE_NEW_UI=0` build.
- **Still popover-shaped on the Rust side.** `show_popover` calls
  `anchor_under_tray`, so the window is positioned under the tray icon rather
  than where the user left it, and `set_activation_policy(Accessory)` keeps it
  out of the dock. Both are wrong for a 1280x800 window and neither is changed
  yet - unverified Rust edits were not worth guessing at.
- `src-tauri/src/lib.rs`: the transparent-window + CALayer `cornerRadius` work
  is popover-specific.
- `App.tsx`: replace the depth-ranked push/pop `Screen` stack with
  sidebar + content pane. The `depth` map and slide animations become dead code.
- Retire `pinPopover` / `unpinPopover` from `lib/api.ts`.

### Phase 10 - Tests and docs

`Home.test.tsx` (822), `Settings.test.tsx` (310), `GroupMembers.test.tsx` (406)
will all break on DOM restructure. Rewrite CLAUDE.md's Aesthetic Direction in
the same PR.

## Open questions

1. **`PROTECTED APPS 4/4`** reads 4/4 in every frame while switch states vary,
   sitting above two off switches with "2 of 4 Apps" beside it. Either it means
   something other than protected-count, or it was never bound. The sidebar
   currently computes protected-over-total.
2. **App brand logos: RESOLVED 2026-08-23 - drawn, pending export.** The
   `Components / Sidenav` page carries a `logo` component set with all eight
   marks, plus coloured vendor marks in the model `row` set. Still no assets
   in the repo (`AppRow` falls back to an initial, the Model cells render the
   name alone); see item 11 under "Still to do" for the export route.
3. **Groups: RESOLVED 2026-08-16, re-resolved 2026-08-26 - they live in the
   rail.** The design lists apps flat, which cannot express routing's real
   shape: families (Claude, OpenAI, OpenRouter, plus the multi-provider "Other
   tools" bucket) own a master switch, and their members route either through a
   tool's own config file or through the local proxy - the latter being the chat
   domains, which the drawn UI does not show at all. `FamiliesPane` gave them a
   third sidebar destination until the rail could carry them; it now does, and
   the pane is gone. **Still not in the Figma**: the master card and the family
   switches on the eyebrows are additions to the drawn rail.
4. **Diagnostics: RESOLVED 2026-08-16.** A row under About opens
   `DiagnosticsDialog`, which shows the report before offering to copy it.
   `buildSettingsSections` owns the row, and a test pins it so it cannot be
   dropped silently.
5. **Minimize2: RESOLVED 2026-08-16 - removed.** With window controls coming
   from the OS, a second minimise affordance was a duplicate. The topbar now
   carries only the overflow menu, and the brand lockup is genuinely centred
   rather than sitting at the design's 504px, an offset that existed only
   because a second button balanced it.
6. **Onboarding / FirstRun / OrgPicker / Success: RESOLVED 2026-08-19 - drawn,
   and rebuilt against the drawing.** Built provisionally on 2026-08-16 because
   the design had nothing to say about first run; `Flows / Onboarding` and
   `Flows / Auth` landed on 2026-08-19 and both are marked ready. `setup.tsx`
   now follows them - see "Built 2026-08-19". What remains open is the
   `Auth / Error states` section, which carries no ready mark, and the fact that
   the onboarding tutorial still renders in the popover-era `gc/` ink system
   rather than the window shell the design draws it in.
7. **Does the chart's `total` series double-count? RESOLVED 2026-08-17 - no,
   the four are additive.** Settled by the Figma's own `chart/tooltip`
   (`191:79768`), added to the file after this question was written. It lists
   `Total messages 8 / Blocked 2 / Flagged 2 / Redacted 0` under a heading
   reading `12`, and that heading carries `mono/eyebrow` - the style the axis
   ticks use, i.e. an identifier naming the column, not a fifth figure. So
   "Total messages" is the remainder series, `MessagesBucket` was already right,
   and the endpoint keeps sending `requests` plus the three security counts with
   the client subtracting. The sr-only table now names the sum "All messages" so
   two different figures no longer share one column name.
8. **Sidebar app states: RESOLVED 2026-08-16.** `SidebarApp` now carries an
   `AppStatus` union (`protected` / `not-protected` / `drifted` /
   `not-routed`), and separately an `on` boolean. The split is deliberate and
   `lib/groups.ts` explains why: observed routing drives the status line, user
   intent drives the switch. Conflating them made the switch destructive, since
   an enabled-but-unrouted member rendered off and clicking it turned the
   setting off rather than on.
9. **Tailwind version skew.** The design names shadows on Tailwind v4's scale
   (v4 `shadow-xs` == v3 `shadow-sm`); the repo is on v3.4.17. The `base-*`
   shadow tokens absorb the mapping, but any new value read off Figma needs
   shifting one step before use.

## Still to do

The queue downstream PRs draw from, in the order that unblocks the most.

1. **First run, disconnect and reset: DONE.** `lib/useSetup.ts` derives the
   stage from account and OAuth state rather than storing it, which is what makes
   reset work - clearing the account is the whole handoff, with no separate "show
   first run" flag to disagree with what is on disk. Two things cannot be derived
   and are flagged in the hook: whether the confirmation pane has been seen (a
   returning user must not be greeted by it) and the org-picker dead end's escape
   to the key form. `isSignedIn` / `needsOrg` moved to `lib/session.ts` so both
   shells cannot drift on what counts as signed in.
2. **Updates: DONE.** The mechanism moved to `lib/useUpdate.ts` and both shells
   use it - the opposite call from `useRouting` and `useSettingsActions`, because
   nothing here was tangled with the popover's shape and what it holds is a
   sequence whose ordering is load-bearing: the relaunch mark lands after the
   download (quitting mid-download is a genuine exit that must keep its cleanup)
   and before `install()` (on Windows the installer exits from inside that call).
   A second copy could drift into a botched update. The window uses the banner
   `AppShell` already had a slot for; the popover keeps its takeover-then-banner
   escalation. Settings' "Check for updates" reports its result in the version
   row, since silence on a pressed button reads as broken.
3. **The running-apps sequence: DONE.** `lib/useRunningApps.ts` runs it after a
   config write that actually happened - `setAppRouted` now reports that, so a
   declined review or a failed write never reaches it. Deliberately not part of
   `useRouting`: that hook's prompt is a *gate* that blocks a write until
   answered, and this is the opposite, a sequence that follows a write and can be
   walked away from without changing what was saved. Nothing is signalled without
   two answers, and a failed scan stays silent rather than defaulting to showing
   (the popover defaults the other way, but it is choosing whether to show
   advice; this offers to kill processes). The e2e fixture gained
   `runningAgentNames`, since it stubbed only the count probes.
4. **The family master cascade: DONE.** The rules moved to `cascadeTargets` in
   `lib/groups.ts` and both shells use them: chat members never ride a family
   switch, a drifted config is never adopted by one, and members already in the
   target state are left alone. The action is `setFamilyRouted` in `useRouting`,
   which trusts the certificate ahead of the loop (a member's connect
   auto-enables the engine, so the system dialog must not be sprung from member
   three) and names the members that failed rather than reporting "couldn't
   connect this tool".

   Two things this turned up. The new UI was driving the family switch from
   `Group.desired` instead of `cascadeDesired` - the exact bug `groups.ts`
   documents, where a chat member switched on alone leaves the master stuck on.
   And a config member's `desired` is derived from `connected`, so a **drifted
   member is outside the family switch in both directions**; the row and the
   alert card are the ways back. That is pre-existing shared behaviour, so it is
   pinned by a test rather than changed here.
5. **The per-app model picker: UI DONE, no backend exists.** `App / Select a
   model` was finally read (browser, "App w/ choose model modal open"): the
   design draws a **dropdown anchored to Change model**, one row per model, the
   current one first and outlined, listing eleven `gate/...` ids across
   Anthropic, DeepSeek, Qwen, Kimi and OpenAI. Built as `ModelPickerDialog`; the
   choice is confirmed through `UseGateModelDialog` because switching to a Gate
   model spends PAYG credits.

   **There is no model backend at all** - no Tauri command, no Rust, nothing to
   persist to. The choice is session state and says so at the call site, and the
   picker ships with an empty list rather than the drawn ids: shipping those
   would put a fabricated model catalogue in front of the user, which is the same
   argument the zeroed metrics make. What it needs is an endpoint reporting the
   gateway's models and somewhere to record the per-app selection.

   Two details of the frame could not be read: the top edge was cut off, so
   whether the panel carries a search field is unknown (omitted), and the drawn
   ids render in the UI face rather than mono. Mono was used, since CLAUDE.md
   names model ids explicitly and every other identifier in the design is mono.
6. **Truthful routing status (AG-562): DONE.** Status lines no longer come from
   `Tool.status`. `crates/core/src/routing_health.rs` computes a verdict from
   three inputs - the integration's config state, a new loopback health check
   that the relay answers itself (`/__gate/health`, 204, never forwarded), and
   whether the tool's process predates the last routing change - and
   `routing_verdicts` reports it per tool. `lib/verdict.ts` maps that onto the
   four phrases the design draws, carrying the ticket's reason in the grey suffix
   the design already has a slot for.

   `Status` was deliberately **not** rewritten. It describes config on disk and is
   threaded through every integration, `provider.rs` and their tests; the verdict
   is a layer on top, which is also what keeps intent and observation separate one
   level below where `groups.ts` documents the same rule.

   Three things this turned up. The probes run **once per sweep**, not per tool -
   they ask about shared infrastructure (the relay port, the account's session), so
   per-tool calls would let two rows in one refresh disagree. `SessionProbe::Unavailable`
   must map to `Verification failed` rather than `Access problem`, or an offline
   machine accuses a perfectly good credential. And a tool with no known process
   name (OpenClaw, Hermes) has *unobservable* staleness rather than none, so it can
   still read Protected but will never be told to reopen.

   **Two conflicts raised rather than resolved in code.** AG-562 specifies the
   words On / Off / Needs attention; the Figma draws Protected / Not protected /
   Config drifted / Not routed. The design won the coloured phrase and the ticket
   won the reason, because the Figma is the source of truth for copy - the
   remaining question is for the designer on AG-561. Separately, AG-564/566/570/596
   all assert Gate "does not restore user-authored values", but
   `integrations/claude_code.rs` saves `previousEnv` and restores it on disconnect,
   and `groups.ts:312` documents that behaviour too. The code is right; the copy
   was not written.

   What this does **not** do: prove a tool sent traffic. Nothing attributes
   requests to a tool. That needs a per-tool segment in the relay path
   (`/<tool>/<domain-slug>` instead of `/<domain-slug>`), which would also give
   AG-574 its per-tool counts.
7. **Controls the popover had and the window did not: DONE.** A sweep of which
   `lib/api.ts` commands each shell can reach found eleven gaps, all of them
   wiring rather than missing backends.

   The one that changed behaviour rather than reach: **there was no way to turn
   routing on or off after first run.** `proxyEnable` was called only by
   `useSetup.turnOnRouting` and `proxyDisable` only by reset, and
   `proxy_set_domain` does not start the engine the way `connect_tool` does - so a
   chat domain toggled with the engine off wrote intent, routed nothing, and had
   no control anywhere to recover. `useRouting` now owns three engine-level
   actions (`setMasterRouted`, `setEnvExport`, `untrustCa`), `setDomainRouted`
   starts the engine after the certificate gate, and `FamiliesPane` grew a master
   card carrying the switch and the shell-environment sub-switch. Not in the
   Figma, like the rest of that pane.

   The **app pane's own switch was `noop`**, and its `isProtected` came from the
   observed verdict - the exact conflation `lib/groups.ts` documents, where a
   drifted app renders off and clicking the switch turns off the setting the user
   was trying to turn on. It now reads intent and calls `routeApp`. Its label is
   `Route <app>` rather than `<app>`, because the sidebar row for the same app is
   on screen with a switch of its own.

   Also wired: the topnav's dead **Read Gate docs** entry (and Contact support
   removed, since there is no address - `SettingsPane` omits its row for the same
   reason); the **diagnostics report**, which was passing `backend`, `oauth` and
   `agents` as null and `clientsStale` as `false` - a claim, not an unknown - and
   now runs the probes `screens/Diagnostics.tsx` runs; the **API-key row**, which
   drew `sk-gw` plus twenty asterisks and now shows the recorded prefix or says
   the key is in the keychain; the **first-launch tutorial**, which only the
   popover opened; the **gateway picker** on the sign-in card and a Change server
   row in Settings, so the window is no longer stuck on the build's default; the
   **certificate**, which could be trusted but never removed; the dead **Add
   credits** and **Manage** buttons, pointed at the dashboard; and the one-time
   **OAuth offer**, which no key-based install on the default shell was seeing.

   Two rows needed a backend rather than wiring, and got a small one. **Install
   ID** showed the PostHog distinct id, which is absent in a build with no project
   key and absent again the moment somebody opts out of diagnostics - a row
   reading Unavailable for reasons that had nothing to do with the install.
   `primitives::install_id` already existed, cached at
   `<app_support_dir>/install-id`, but was macOS-only with zero callers because it
   read `/dev/urandom`; it now uses `rand` (already a dependency, for PKCE) and is
   exposed as `install_id`. The analytics id stays in the diagnostics report under
   its own name - they are two different facts. **Device** was hardcoded `"-"`
   while `RenameDeviceDialog` sat built and unwired: `preferences.json` gained
   `device_name: Option<String>`, `device_name` resolves the override or the
   machine's hostname (`sysinfo`, already a `src-tauri` dependency), and
   `set_device_name` stores it. `Preferences` is no longer `Copy`.

   The name is local - nothing sends it anywhere yet - and clearing it back to the
   hostname is a backend behaviour with no UI path, since the dialog refuses an
   empty field. Both are worth revisiting when the dashboard grows a device list.

   Covered by `e2e/new-ui-engine.spec.ts`: the hook tests cannot see whether a
   control is connected to an action, which is what all of these were.

8. **Metrics.** Overview and `AppPane` render zeros against `EMPTY_STATS`
   pending the 24-hour endpoint. Open question 7 (whether `total` double-counts)
   should be settled in that response shape, not in the chart.
9. **Retire the popover.** `App.tsx`, `screens/`, `gc/ui.tsx`, the `gc.*`
   palette, `pinPopover` / `unpinPopover`, and `VITE_NEW_UI=0` all go together.
   Item 1 was the blocker and is done, so what remains is repointing the e2e
   suite at the new shell and deleting; the popover's own specs go with it.
10. **The redrawn rail (Sidenav page, read 2026-08-23): DONE.** Built
    2026-08-23 - chat and app-surface members are rail rows (switch on
    `setDomainRouted`, status from the domain's own state), each eyebrow
    carries its protected-over-total counter, and the multi-provider tools
    share one `Other tools` group. See "Built 2026-08-23".

    Domain rows open the App pane too now (same day, second pass): header,
    status, switch and model card all work, and the activity sections say
    *why* they cannot be shown instead of reporting a quiet day. The gateway
    attributes requests to config tools only - `client_tool` is derived from
    each tool's own user agent, and traffic from these surfaces arrives
    unattributed on purpose (a guessed slug would file one app's traffic
    under another's name) - so the per-tool reads never fire for a domain
    (`openDomain` in `NewUiApp.tsx`): filtering by a domain slug would return
    an empty reading and the pane would claim a quiet day over traffic it
    cannot see. The tiles read `N/A`, the chart and feed carry their
    unavailable notes, and the pane's gap slot names the cause, mirroring the
    unattributed-machine treatment. Per-surface activity needs real
    attribution on the gateway; AG-572's contract doc records why the
    user-agent heuristic cannot provide it.
11. **Brand marks: STOPGAP SHIPPED 2026-08-23, designer export still wanted.**
    `src/components/gc/BrandMark.tsx` vendors the seven rail marks (Claude,
    Claude Code, Codex, OpenAI, OpenRouter, OpenCode, OpenClaw) from
    lobehub/lobe-icons (MIT), all `currentColor` so the tile's white text
    colours them, and `brandMarkFor(slug)` maps both tool and domain slugs
    onto them - Hermes has no mark and keeps the initial fallback. Wired into
    the rail rows and the App pane header tile through the `logo` props that
    already existed. These are the same official logos the Figma draws, so
    when the designer's export lands only `BrandMark.tsx` changes. Still
    pending with the model backend: the coloured vendor marks for the model
    picker rows and activity Model cells (`anthropic`, `deepseek-color`,
    `qwen-color`, `kimi-color` in the same set).
12. **Alert banner copy: DONE 2026-08-23.** `master-off` and `drifted` in
    `lib/notices.ts` (and `NewUiApp`'s `driftAlert`) carry the drawn copy.
    The drift card names the app in its body where the drawing says "This
    app's" - the card pages between apps and the drawn card never names one -
    raised with the designer rather than shipped ambiguous.

Rows the design draws that have no backend command at all: device rename and
notifications, plus plan upgrade, which has no billing URL to open. They render
their value and omit their control.

Two rows are gated on the account's auth mode rather than hidden outright:
**Replace key** appears only for an API-key account (on an OAuth account
`saveAccount` with a key would flip `auth_mode`, quietly converting the account
behind a button that says "replace"), and **Disconnect Gate** only for an OAuth
account, which is the only one with a session to end.

## Copy corrections to raise with the designer

Two dialogs say something the action does not do. Both are implemented correctly
and the reason is recorded at the component:

- **Replace API key** labels its second field "New device name", copy-pasted from
  the rename dialog. Shipped as "New API key": the drawn label would put a wrong
  word on the one screen where the user handles a credential.
- **Disconnect Gate** says protection turns off, apps stop routing and the key is
  removed from the keychain. That describes Reset, a separate row on the same
  screen. Implemented as ending the session, and the body copy corrected, rather
  than shipping two destructive actions that claim the same consequences.

## Drift repair (AG-568)

The review dialog and the drift *gate* were already built (`ReviewConfigDialog`,
`useRouting.ts` refusing to adopt a drifted config silently). Two things were
missing, and one of them was a correctness bug.

**A failed write left the row lying.** `connect_tool` failing sent the error to a
transient banner, and the status line - the thing next to the switch the user
just clicked - carried on describing the state from before the click. It was
*true*, since nothing was written, and useless. `useRouting` now remembers which
slugs failed (`writeFailures`), and the row reads "Configuration update failed".

That state is deliberately **not** a sixth `routing_health::Reason`. The Rust
reasons are derived from evidence - a config on disk, a relay that answers, a
process older than the last change - and a failed write leaves none of that to
probe. It is session state, cleared the moment a write for that slug succeeds, and
it arrives at `verdictStatus` as a separate argument for exactly that reason.

Worth noting: **AG-562's list of five reasons is incomplete.** AG-564 and AG-568
both name "Configuration update failed" as a status. Raised on those tickets
rather than smuggled into the enum.

**The dialog showed what Gate found but not what it would write.** Approving an
overwrite without seeing the replacement is approving a value you cannot see, on
the one screen where the user hands their tool's routing to us. `ProxyState`
gained `relay_base_url` (non-secret - it is already written verbatim into every
config-routed tool's own file), and the dialog shows it. With no relay port bound
the row is omitted rather than guessed at.

Still open on the ticket: the per-failure action set ("Retry, Use tool defaults,
Documentation, Diagnostics, or Contact support based on the failure"). The sidebar
row has a switch and no room for a second control, and the switch *is* the retry -
but a documentation or diagnostics link per failure needs the per-app pane, and
Contact support needs a URL that does not exist. Also open: "last completed check
or routed request" in the summary, which needs the activity endpoint.


## Settings sections and preferences (AG-594)

There was **no preferences store anywhere**. `account.rs` was the only config, and
it holds a credential and an identity, so the notification and diagnostics
choices had nowhere to live - which is why `SettingsPane` had a `notifications`
prop that the shell never passed. New `crates/core/src/preferences.rs`: a small
JSON file next to `account.json`, deliberately separate so a preference change
does not rewrite the file holding the key prefix, and so clearing the account on
reset does not take the preferences with it.

**Every preference defaults to on, and a missing field loads as on.** That is
what lets a switch read On before anything has been written, rather than showing
Off and inviting the user to "fix" a setting that was never off.

Sections went from 6 to the 8 the criteria name that can be built: Device,
Account, Connection, Startup, Notifications, Diagnostics, About, Help, plus the
Danger zone. Notifications and Diagnostics moved out of Startup and About
respectively.

### Rows deliberately not built, and why

- ~~**Blocked-event, flagged-event and sound switches.**~~ **Built 2026-08-31
  with AG-578.** The entry is kept rather than deleted because the reason it gave
  is the reason they waited: a switch for an event that cannot arrive tells the
  user they turned something off, so they landed with the feed that fires them
  and not before. All four AG-594 switches now exist. The `Notifications` row
  keeps its narrower routing wording and the drawn "Alert me when a request is
  blocked or flagged" moved to the two rows that actually gate that - see the
  2026-08-31 sync below.
- **The permission row.** `tauri-plugin-notification` hardcodes
  `PermissionState::Granted` on desktop - `desktop.rs` returns it unconditionally,
  the state is only real on mobile. A permission row built on that would report
  "granted" on a Mac with notifications denied. Real detection needs per-platform
  native work (UNUserNotificationCenter on macOS).
- **Contact support.** There is no support URL anywhere in the app. `GATE_DOCS_URL`
  exists, so Documentation is wired; support is omitted rather than pointed at an
  invented address. The topnav's Contact support entry is dead for the same reason
  (AG-598).
- **Send diagnostics now, and the diagnostic reference.** No upload path exists;
  that is AG-603.

### Unavailable rows

`SettingsRow` gained `unavailable: { onRetry }`. The shell now tracks *whether a
read failed* separately from the value it failed to produce:
`launch?.enabled ?? false` collapsed "off" and "could not be read" into one Off
switch, which is a claim about the user's setting they cannot distinguish from one
they made. Wired for launch-at-login and for the preferences pair, which share one
read and so fail together - a failed preferences read leaves launch-at-login's
switch alone, and a test pins that.

## Refreshing the inventory (AG-558)

Detection ran on backend events only, so a tool installed while the window was
open stayed invisible until something unrelated repainted the sidebar. The
"Protected apps" eyebrow first gained a small refresh control for this. It has
since been removed: the Figma draws no such control, and a control is the wrong
answer to a reading the user has no reason to know is stale.

**It has an event of its own now, and the poll is gone (2026-09-03).** The middle
answer was a 5s `DETECT_POLL_MS` timer in both shells - twelve config-file walks a
minute, forever, for something that happens a handful of times in a machine's
life. `core/src/tool_watch.rs` watches the paths each integration declares
(`Integration::watch_paths`) and emits `tools-changed`; both shells re-read on it,
the same shape as `proxy-state-changed`. The watch arms the deepest directory that
exists at or above each target and re-arms as the rest appear, because the
interesting paths are the ones that do not exist yet - `~/.codex/config.toml` is
what shows up when someone installs Codex.

What a re-read covers is deliberately narrower than `refresh`. `list_tools` walks
config files and `proxy_status` reads memory; `routing_verdicts` probes the relay
*and* the gateway session, so a filesystem event must not be able to trigger it -
a package manager writing in a watched directory would otherwise aim a burst of
probes at the gateway. `redetect` compares both readings against the last ones
rendered - `detectionSignature` over a `rendered` ref, kept current by an effect on
the state itself so a toggle's own re-read counts as drawn - and commits nothing
when they match. Every memo below hangs off `tools` and `proxy`, so re-setting an
equal-but-new object would rebuild the families, the settings sections and the
routing callbacks for no change. When either does move the sweep runs: a row that
just appeared has no verdict and reads "Checking", and the engine coming up changes
all of them at once.

The `visibilitychange` read stays, and is not a poll. It covers what a watch cannot
see - Hermes' launcher can sit anywhere on `$PATH`, and a `$PATH` entry has no
directory to arm - and what a watch that failed to start would miss entirely. Three
e2e tests pin the arrangement per shell: the list is re-read on the event and not on
a timer, an unchanged machine does not re-run the sweep, and a tool that appears
stays invisible until the event lands.

`scan` is still written on every read even when nothing changed: the timestamp is
the empty card's evidence that something is still looking.

The inventory card keeps its own Refresh / Try again. A *failed* scan is a state a
user may reasonably want to retry against rather than wait out, which is why
`refreshNow` and the `refreshing` flag survive. `refreshing` is deliberately
separate from `routingBusy`: that one guards a *write*, and refusing to re-read
during a toggle would be the wrong coupling.

The rest of AG-558 is not buildable and is documented on the ticket: every
integration returns `requires_upstream_credential() == false`, so "installed but
unavailable" cannot occur; "installed but unsupported" needs detection of tools
Gate has no integration for; "incomplete installation" needs per-integration
probes; and the per-entry request counts need per-*tool* attribution, which the
in-flight `feat/activity-overview-client` does not provide (it is per-installation
- `activity.ts` has no tool or slug in it).


## An empty inventory is not a failed one (AG-560)

`listTools().catch(() => [])` turned a failed read into an empty array, so a device
Gate could not scan rendered exactly like a device with no AI apps on it - blank
list, "0/0" count, no explanation. `InventoryState` in `Sidebar.tsx` now tells the
two apart:

- **`none`** - the scan completed and found nothing. Carries the scan time, which
  is what makes it an answer rather than a shrug, plus Refresh.
- **`failed`** - the scan could not complete. Amber, says Gate does not know what
  is installed and that nothing was changed, and offers Try again.
- **`ok`** - there are rows, and the rows speak for themselves.

Before the first scan lands the state is `ok`, deliberately: "no apps detected" is
a claim, and nothing has checked yet.

The card's Refresh is now the only one in the rail: the eyebrow control it used to
hide behind was removed when detection started polling (see AG-558 above).

Not built, and recorded on the ticket: "a detected but unsupported tool remains
visible" needs detection of tools Gate has no integration for; "a known but absent
tool may provide an installation action" is optional in the criteria and would put
uninstalled tools in a rail the Figma draws as installed apps only; and the
model-control gating would remove the picker shipped in #159.

## The diagnostics switch now gates something (AG-603)

The "Share diagnostic data" switch added for AG-594 recorded a preference that
**nothing read**, while PostHog collected regardless. `lib/analytics.ts` is a real
channel - initialised at boot in `main.tsx`, capturing a closed set of event names,
a filtered prop allowlist, classified error titles and two coarse super-properties
- and it had no user opt-out at all. A switch that implies control it does not have
is worse than no switch.

Now:

- **`initAnalytics` reads consent before constructing the client.** An install that
  opted out never creates it. Consent is checked *before* `posthog.init`, not
  after, because opting out afterwards would still have put the device on the wire
  first.
- **A failed consent read means do not collect.** `preferences::load()` is
  infallible in Rust, so the only path here is the IPC failing - and consent that
  cannot be confirmed is not consent.
- **`setAnalyticsConsent` applies a change immediately.** Off opts the live client
  out; on starts it if this session never did (the opted-out install) and opts back
  in otherwise. Called from the Settings switch *before* the write, so a failed
  write cannot leave the client sending after the user said no.
- **`initAnalytics` is now async and not awaited** in `main.tsx`: blocking first
  paint on an IPC round trip would trade a visible delay for a few milliseconds of
  telemetry.

A read-only "What is collected" list opens from Settings without touching the
setting, as the criteria require. Its contents are written from what
`analytics.ts` actually sends, **not** from the ticket's field list - that list
describes an upload that does not exist (installation name, verification state,
event-delivery state, notification permission), and describing it would be
describing something Gate does not do, on the one screen whose job is telling the
truth about what leaves the machine.

Still open on the ticket: the onboarding Diagnostic data step (shared with AG-554),
"Send diagnostics now" and the diagnostic reference (no upload path exists), and
scoping the choice to the selected organization.


## The diagnostic-data onboarding step (AG-554 / AG-603)

Consent before collection: `lib/analytics.ts` starts PostHog at launch, so what
this step buys is a person who has actually been asked. It sits between the
sign-in confirmation and Overview.

**Derived, not remembered.** `preferences.json` gained
`share_diagnostics_recorded`, and `useSetup` returns the `diagnostics` stage while
it is false. That keeps the guarantee the hook already had - stage comes from what
is on disk - so a reload mid-setup cannot skip the step, and answering it is what
dismisses it. `undefined` (the read still in flight) is deliberately *not*
unanswered, or the step would flash at someone who answered months ago.

`share_diagnostics_recorded` defaults to **false**, including on installs written
before the field existed. Those see the step once, which is the point: sharing
defaults to on, and a default nobody was asked about is not consent. It is also why
this is a separate field rather than inferring an answer from `share_diagnostics` -
the default and a deliberate "yes" are the same value and must not be the same
fact.

Continue records the **displayed** value whether or not it changed, because leaving
the default in place is still an answer; treating it as unanswered would ask again
next launch. The same command backs the Settings switch, so changing it there also
counts as answering.

The sent / never-sent lists are shared with the Settings disclosure
(`CollectedDataLists`, one copy, framed by whatever `Wrapper` each caller passes).
Two copies would drift, and these are the claims the product's reassurance rests
on - the moment the onboarding promise and the Settings disclosure disagree,
neither can be trusted.

Layout is provisional; the Figma draws no diagnostics step.

### A harness bug this turned up

`e2e/install.ts`'s `get_preferences` returned the *live* `state.preferences`
object, which the write stubs mutate in place. So `setPrefs` received an identical
reference, React skipped the re-render, and a preference change was invisible to
anything derived from it. Real IPC serialises every response; the stub now returns
a copy. Worth remembering for the other handlers that still return `state.x`
directly.

Still open on AG-554: the device-derived installation-name suggestion (device
rename has no backend command), and distinguishing cancellation from expiration
from failure in the browser sign-in.
## Interrupted routing, explained and resumable (AG-570)

Three changes, in three PRs.

**1. The window shell had no backend-error drain.** `drainBackendErrors` existed only
in `App.tsx`, so a failure buffered Rust-side went to telemetry and nowhere else -
including `report_backend_error("provider_restore", ...)`, which fires on both
restore passes in `proxy_enable`. Routing could fail to come back and the window
said nothing. `forwardBackendErrors` moved to `lib/backendErrors.ts` and both shells
import it; the popover's behaviour is unchanged.

**2. The snapshots were already a record of unfinished work.** `restore_all`
re-attempts each recorded entry, keeps failures in the file, and clears it only once
everything is back. Nothing read them for display. `provider::pending_restore()`
reports what they owe, `resume_restore` calls `restore_all` and returns **what is
still outstanding** rather than unit, and `RecoveryBanner` names it with Resume now
and Finish later. Amber, not red: nothing is lost, and resuming retries exactly what
failed.

**3. A per-entry journal** (`crates/core/src/recovery.rs`) records what the restore
*did*, as opposed to what is left. Written as it goes and seeded before the first
attempt, because an interrupted operation is the one worth describing and a journal
written on completion would be missing for every case it serves. `RestoreDetailsDialog`
shows it read-only - AG-570 requires that reviewing changes no state.

Details worth keeping:

- **One journal for the whole restore.** Both passes seed it up front; two writers
  would clobber each other's file, which they briefly did.
- **Outcomes come from the control flow, never from an error message.** The restore
  already branches on not-installed, unknown-slug and signed-out, so classifying
  those costs nothing and cannot drift.
- **Dropped entries owe nothing.** `NotInstalled` and `Unknown` are settled, not
  outstanding; reporting them would ask for action nobody can take.
- **A journal is an explanation, never a blocker.** A missing or corrupt one reads
  as absent and the restore still runs. Its fields are `#[serde(default)]`-backed
  for the same reason - a unit test caught that they were not, which would have
  silently dropped the explanation on any older file.
- **Known shortcoming:** only the tool pass can report `DeferredSignedOut`.
  `enable_skipping` has no signed-out branch, so a provider that cannot re-enable
  for want of an account lands on `WriteFailed`. Pinned by a test so it stays
  visible.

Still open: the recovery action lives in the shell banner, not the tray; per-tool
"last verified route" and "check result" in the summary would need the verdict sweep
to persist its results; and item 8's default-writing result list exists for the quit
path only (#162), not yet for sign-out and reset.
## Quit and teardown in the window shell (AG-596)

The popover has carried the three-way quit since it shipped
(`components/QuitConfirm.tsx`: disconnect-and-quit / quit-without-disconnecting /
cancel, with focus on cancel). The window shell had **nothing**: no
`quit-requested` listener and no dialog, so a tray Quit raised in the new UI went
unanswered. Fixed by porting the flow, not reinventing it - `QuitDialog` in
`dialogs.tsx` shares the popover's copy so a user who has seen one is not asked
to work out whether the other means something different.

`Modal` gained an optional **`middle` action**. Three buttons because the
outcomes genuinely differ: collapsing "quit without disconnecting" into the
primary would hide the consequence that makes it a separate choice, and
collapsing cancel would leave no way out. It is the only dialog that needs it.

The larger change is behavioural, and it applies to **both** shells.
`provider::snapshot_and_disable_everything` swallowed per-tool disconnect
failures into `eprintln!` and returned `Ok(())`, so a quit that left a tool
pointing at a relay about to die reported success and fired a notification
saying "your tools are back on their own settings". It now returns the display
names of what it could not put back:

- `disconnect_tools_for_quit` returns `Vec<String>` rather than `()`.
- The quit notification only claims a clean teardown when there was one.
- Both shells stop, name the tools, and offer retry / quit-anyway rather than
  exiting quietly. AG-596 is explicit: Gate Connect "does not claim cleanup
  completed".

Not covered here, and still open on the ticket: the reset result's equivalent
listing ("if reset leaves a credential, configuration, or routed tool, Gate
Connect lists it"). Reset's checkbox gate and confirmation are built; enumerating
*residue* after the fact needs the same treatment `snapshot_and_disable_everything`
just got, applied to the reset path.

The restore-vs-AC conflict recorded against AG-564/566/570/596 is unchanged by
this work: `claude_code.rs` does restore prior values, so no copy here claims
otherwise.
## Naming the file Gate rewrites (AG-564)

`Integration::config_location()` is new: the file each integration edits, as a
display string, implemented for Claude Code, Codex, OpenCode, OpenClaw and Hermes
and defaulting to `None` for the environment channel, which writes machine-wide
settings rather than a file of its own. It reaches the UI on `ToolDto` and the
drift review now names the file before asking the user to approve a rewrite.

Two things this is deliberately *not*:

- **Not a probe.** Reading the location creates nothing. A test pins that,
  because a call that materialised a config directory would make `status()`
  report a tool as present because the UI had looked at it.
- **Not a secret.** It is a path in the user's own home directory, and the point
  of showing it is that they can go and read it - which is the reassurance
  CLAUDE.md's first design principle asks for, and stronger than any sentence
  about what Gate does or does not touch.

### The confirmation this ticket asks for, and why it is not here

AG-564 asks for a confirmation before *every* turn-on: "Before routing is turned
on, Gate Connect states that it will replace the tool's routing configuration
with Gate values", with **Use Gate routing** and **Cancel**. It was built, and
then removed before shipping, for reasons worth recording:

1. **The design draws a bare switch.** The Figma has no confirmation on this flow,
   and AG-563 - this ticket's design counterpart - is still In review, so the
   designer has an opinion in flight that this work cannot see. CLAUDE.md makes the
   Figma the source of truth for exactly this kind of question.
2. **It stacks two modals on a first turn-on.** The gate lands before
   `ensureCaTrusted`, so the first tool a user routes would raise "Route X through
   Gate?" and then "Trust the Gate certificate?" back to back.
3. **The criteria are written per tool and say nothing about the cascade.**
   `setFamilyRouted` calls `connectTool` directly rather than through
   `setAppRouted`, so a family switch would have escaped the gate anyway - which is
   just as well, since a modal per member would make that switch unusable. But
   that means the ticket's model and the product's shape disagree.

The warning does exist for the case where something of the user's is actually at
risk of being overwritten: a drifted config, which has had the review dialog all
along. What a general confirmation would add is coverage of the *clean* case,
where `integrations/codex.rs` still replaces the user's own `model_provider` (and
stashes it for disconnect). That is a real gap, and a real design decision -
raised on AG-564 and AG-563 rather than settled here.

## Unavailable and held readings (AG-576)

The ticket's written ACs still describe a Stale badge and a Refresh control; the
PO's later call dropped both. What shipped instead:

- **Skeletons, never a zero.** `Skeleton` and `EmptyNote` in `gc/base.tsx`, drawn
  by the stat tiles, the chart (24 fixed-height columns) and both Overview tables
  while the first read is in flight. A figure on screen is a figure something
  measured: `0` says the user sent nothing, a dash says we asked and were
  refused, and neither is true while we are still asking.
- **"No messages sent in the last 24hrs"** is the exact empty copy, in the chart
  and in `AppPane`'s recent activity. It appears only when the reading really is
  zero - a dense series of 24 zero buckets - and never over a section the gateway
  declined.
- **`ActivityView.missing`** is new and is not `gaps`. `gaps` says *why*, in copy,
  for the notice under the header; `missing` says *whether*, as a flag, for the
  card. A declined section and an empty one both arrive as an empty array, so
  without the flag the pane reports "no policies configured" over a list nobody
  would hand over.
- **The last reading is served off disk.** `crates/core/src/activity_cache.rs`
  writes the response body to `activity-cache.json` at 0600, keyed on
  gateway + auth mode + org + install filter, and `useActivity` fires the cached
  read and the network read together. The cached body is applied only if it wins
  the race, in either direction: a slow disk must not overwrite a fresh reading,
  and a failed fetch must not be papered over with older numbers arriving late.
  A scope change blanks the view; the retry button does not. Sign-out clears the
  file with the account.

**The empty state is sampled, from `228:89241`** (`App/Claude-desktop/alert/
gate-model`, supplied 2026-08-19). Two corrections came out of it:

- **A counter with no reading behind it reads `N/A`; a measured zero reads `0`.**
  The frame draws Messages `0`, Blocked/Flagged `0` and Tokens saved `N/A` side
  by side, and that is one case rather than a per-counter rule: an org with no
  traffic, where the two counts genuinely are zero and savings genuinely has no
  figure. The rule reproduces it exactly and also covers the case the Figma does
  not draw - nothing read at all, which reads `N/A` three times. No delta is
  drawn next to `N/A`.

  Settled 2026-08-19 after trying the alternative. Falling the two counts back to
  `0` on a failed read matched the mock on every screen, at the price of printing
  "0 messages" over traffic the pane could not see, and of contradicting the
  chart four inches below it that was already saying "Messages couldn't be read".
  A number looks more authoritative than a sentence, so the tile would have won
  that disagreement. `N/A` keeps the whole pane telling one story.
- The empty note is a **glyph above the sentence**, not a bare line of text: a
  36px tile with a 1px `base/border` and a 20px muted icon, centred, with the
  sentence beneath. `messageCircleX` for a message feed (added to `Icon.tsx`),
  `shieldCheck` for policies, `layers` for savings.

  Each card keeps its own glyph in *both* sentences. A first pass gave every
  "couldn't be read" note a warning triangle, which stacked three of them down a
  pane whose single cause was already stated once in the notice above; one
  refused credential read as three problems. The sentence carries the
  difference, the notice carries the alarm.

The frame also confirms two things already built: the empty chart sits inside the
normal Messages card with its heading intact, and Recent activity still lists
rows dated well outside the 24-hour window while the chart above it says no
messages were sent. The feed deliberately outlives the chart's window.

Still open: the skeleton silhouettes. No frame draws a loading state, so the
placeholder columns and rows are inferred from the shapes they stand in for.

### Sync 2026-08-31: the live security-event feed (AG-578)

**A third rail entry, and it is undrawn.** `Security events` now sits between
Overview and Settings in the nav block. The Sidenav page (408:15625) draws two
entries, not three, and no frame draws this pane at all - so it is built from the
component set, which is what this file's own rule asks for where no frame draws
the thing. Recorded here as a deviation rather than slipped in. If the designer
draws it later, the frame wins and this entry gets rebuilt to match.

What it is made of, all existing pieces:

- The pane layout is `Overview`'s (`flex flex-1 flex-col gap-4 overflow-auto
  bg-base-background p-6`, header with a right-hand cluster).
- The table is `AppPane`'s recent-activity table, columns Time / Security /
  Category / Tool / Model / Action.
- The badges are the same `BADGE_STYLES` pair - BLOCKED `red/100`-`700`, FLAGGED
  `amber/100`-`700`. **`Pill` and `BADGE_STYLES` moved from `AppPane` into
  `base.tsx`** as part of this: two surfaces draw them now, and a second copy is
  how the two drift into disagreeing about what BLOCKED looks like.
- The empty and unavailable states are `EmptyNote`, the loading state `Skeleton`,
  per the three-way split every other card implements.
- The event detail is a 512px `Modal`.

**One new vocabulary, deliberately not reusing the status one.** The feed's
connection reads `Live` / `Reconnecting` / `Offline` as a `Pill` in the header,
green / amber / neutral. It does **not** reuse `STATUS_TEXT`: that vocabulary is
about one app's traffic, and driving a feed indicator from it would conflate "is
this app routed" with "is the feed connected" - the exact conflation this file and
`lib/groups.ts` keep warning about, in a new place. Offline is neutral rather than
red for the same reason: a feed that is not running is not an error, and painting
it red beside a healthy routing switch invites the reading that routing broke too.

**Settings gained three rows** under the existing Notifications row: Blocked
requests, Flagged requests, Notification sound. That is AG-594's four switches
complete. The Notifications row's description changed from the drawn "Alert me
when a request is blocked or flagged" to "Alert me about routing problems",
because the drawn sentence now belongs to the two rows that gate exactly that and
one switch cannot honestly claim both.

### The tray's activity line, 2026-09-03

**The frames draw one and the build drew half of it first.** Every tray row frame
carries "345 messages · 23 alerts" under the status line, and `Tray`'s docstring
had recorded the whole line as unbuilt since 2026-08-28 on the grounds that no
per-tool reading existed. The alerts went first, from `alertCounts` in `TrayApp`:
the live feed (AG-578) attributes every blocked or flagged request to a tool slug,
and the tray already listens to it for the security card. The message half landed
a day later, below.

**The message half followed on 2026-09-04**, and the note above is why it took a
second pass: `GET /v1/me/activity` answers for one tool at a time, inside a
100-per-minute throttle bucket keyed on the source address rather than the
credential. A read per row per open is the one fan-out that budget cannot take.

What made it affordable is a **held** figure plus a TTL, in `lib/toolMessages.ts`:

- `activity_cache.rs` went from one slot to a per-tool map *under one scope*, so
  the tray gets every row's last reading in a single disk read and one tool's
  fetch no longer evicts another's. The scope still holds one org and one machine
  at a time, which is the property the single slot was protecting - nothing
  accumulates across sign-ins.
- The popover paints from that disk read, then refreshes only what is older than
  `STALE_MS` (45s), **one tool at a time**. So a look refreshes and a second look
  ten seconds later does not: the TTL never limits how often somebody may look,
  it collapses repeated looks inside a window where the answer would be the same.
  That matters because one of the three moments people open this app is
  "debugging why a tool isn't connecting" - the click-close-click case.
- The window's own app-pane reads write the same cache, so a tool whose pane was
  opened is already warm in the tray.

Two things it dragged in. `GET /v1/me/installations` now runs in the tray too:
a null `installId` means *org-wide*, so without the `machineKnown` gate the
popover would put the whole org's traffic on this machine's rows. And the tray's
credential string gained the api-key prefix, for the reason `activity_cache.rs`
records - a replaced key can mean a different org while every other field stays
identical.

**A held figure says how old it is.** It can be a minute old, or the last thing
that landed before the network went, and a number that says nothing about its age
reads as a live one. The row has no width for "measured 14:03", so `measuredAt`
becomes the line's tooltip. A gateway that declined the counter yields no figure
at all rather than a zero - "unavailable" and "never read" want the same thing
from a row with one line to say it in, and the app pane is the surface with room
to tell them apart.

The clean fix is still a per-tool breakdown inside the one reading the Overview
already takes; that would retire the cache map, the TTL and the tray's
installations read together, and fix the window's rail at the same time.

**The rail draws neither, deliberately.** The counters were built on the window's
256px rail first and moved here on 2026-09-03: the tray frames are the ones that
draw an activity line, and the window reports the same traffic on the app pane,
where there is room for the events themselves. `SidebarApp.alerts` lives on the
shared row type because both surfaces build rows from one shape; only the tray
renders it.

Three states, per principle 6: a reading, including a measured `0`, which draws
"No alerts" (the frames draw digits, but their own empty-state copy is a phrase,
and a bare `0` under a status line reads as a figure that failed to arrive);
in flight, which draws a `Skeleton`, and only while a feed that is actually
running has not answered, because with no account `loading` never clears; and no
reading at all, which draws **nothing**. That last covers an unreadable feed and,
permanently, the chat-domain rows - the feed keys events on the tool slug and a
domain's traffic arrives unattributed on purpose, so a `0` there would report a
quiet day over traffic Gate cannot see.

Grey, not amber: a blocked request is Gate doing its job, not the app failing,
and `STATUS_TEXT` is the only vocabulary on that row entitled to report a fault.

Covered by eight cases in `Tray.test.tsx`, nine in `useToolMessages.test.tsx`
(the TTL above all: a refactor that dropped it would look right on screen and
quietly turn every open into N gateway calls), and one in
`e2e/new-ui-tray.spec.ts` for the seam a component test cannot see - the popover
reading the feed buffer on open and moving when an event is pushed.

**The message half has no e2e coverage, deliberately.** The fake backend answers
no activity command, so a browser run cannot reach a figure at all; teaching it
`activity_overview` would also start answering the *window's* Overview, which
several specs currently assert the unavailable states of. Not worth reopening
those to cover what thirteen unit tests already pin.

## Row labels are surface kinds now (2026-09-04)

**What the rows are called.** Every ledger row is named for the surface it
covers rather than for the product behind it: **App** (the desktop apps), **Web**
(the browser tab), **CLI** (the terminal), **Proxy** where a family has one
mechanism and nothing to split. The vendor is said once, by the heading above
the rows, which is why the labels can be one word.

- **Anthropic** - App (`anthropic`), Web (`claude-web`), CLI (`claude-code`)
- **OpenAI** - App (`chatgpt`), Web (`chatgpt-apps`), CLI (`codex`). The
  `openai` domain made a fourth row against a three-row spec; it left the family
  the same day, see the entry below.
- **OpenRouter** - Proxy (`openrouter`)
- **OpenClaw** - CLI, **Hermes** - CLI, **Experimental** - OpenCode,
  **Terminal tools** (`env-proxy`, unhidden for this) and **OpenAI API**
  (`openai`)

**Where the names live.** In the backend, as before: `proxy/catalog.rs` for
domains, each integration's `row_label` for tools, `provider.rs` for the
headings (Anthropic, not Claude). The rename is global across the UI, so the
drift dialog now reads "CLI's config changed outside Gate" - chosen deliberately
over a label-beside-name split, because one name per row is the thing a user can
point at.

**`row_label` is not `display_name` (2026-09-04).** The row label started on
`Integration::display_name`, which is also what the CLI prints, what error
contexts and log lines name, and what the quit takeover lists. Those have no
heading to lean on, so four integrations answering "CLI" collided there: the
takeover read "CLI and CLI still route" - two tools the user cannot tell apart
at the moment they decide whether to close them - and `gate-connect list`
printed three such rows. `display_name` is the product again ("Claude Code");
`row_label` is the ledger's one-word label and defaults to `display_name`, so a
new integration is right everywhere until it opts in. `list_tools` is the only
reader of `row_label`, which is why every UI surface - the rail, the pane, the
dialogs above - is unchanged. `registry.rs`'s
`display_names_are_distinct_across_the_registry` pins the collision shut.

**Where the descriptions live.** `MEMBER_DESCRIPTIONS` in `src/lib/groups.ts`,
keyed by member slug, carried on `GroupMember.description` and rendered by the
app pane's header between the h1 and the status line. Copy, not catalog data:
the backend names the surface it routes, this says what that surface is to the
reader. It is what makes a one-word label legible on a pane that has no vendor
eyebrow. A slug with no entry gets no line.

**Where the headings come from.** `LEFTOVER_GROUPS`, also in `groups.ts` - a
frontend split of the tools the provider catalog claims for nobody, rather than
new `provider.rs` entries. "Other tools" survives as the catch-all for a tool no
heading claims, which in a shipped build should be none.

**Open.** The Figma has no frame for any of this: the labels, the descriptions,
the three new headings and the OpenCode dialog all arrived as copy, and the pane
header grew a third line to hold a sentence the drawn header does not have. It
wants a design read.

**`openai` moved to Experimental (2026-09-04).** The four-rows-against-three
problem above resolved by taking the row out of the family rather than naming it.
api.openai.com rides no OpenAI tool - Codex resolves relay routes against the
whole catalog rather than the enabled set, and the ChatGPT desktop app is on
chatgpt.com - so the switch is generic host interception, and its real dependants
are OpenClaw and Hermes, which blind-tunnel anything outside the enabled catalog.
`provider.openai.proxy_domain_slugs` is empty now; that family switch governs
Codex alone. `LEFTOVER_GROUPS` gained a `domainSlugs` field to carry it, named
per heading rather than swept, because `opencode` is both a tool slug and a
domain slug and a sweep would draw two members under one key.

It is labelled **OpenAI API**, not "OpenAI apps" and not `api.openai.com`. The
row's subject is a host, so the label names the host's role and the host itself
goes in the description ("Anything on this machine that calls api.openai.com
directly. Gate intercepts that host, so apps with no gateway setting of their own
still route."). Two reasons the identifier is not the label: the popover row
already prints `api.openai.com` in its own mono slot, so a sans label repeating
it says it twice and sets an identifier in body type; and the window UI prints
the host nowhere else, so the sentence is the only place it is missing.

## Design sync 2026-09-04: the twelve open questions, answered

All twelve questions raised at the last Figma pass came back from design in one
sitting. Six changed the code, three were already right, three closed with no
work. What each resolved to, and where it landed:

| # | Question | Answer | Code |
| --- | --- | --- | --- |
| 1 | Mono or sans for identifiers? | **Sans.** Mono is reserved for eyebrows and pill labels, nothing else. Plus a standing rule Figma cannot express: *"always use tabular nums on numbers"*. | Changed |
| - | *(this is the question `#208` left open as "contested and unresolved", with instructions to keep mono until it came back. It came back.)* | | |
| 2 | OpenCode's model card | Multi-select is real, and the picker now has a **single-select mode** too. | Changed |
| 3 | Picker auto-close or confirm? | Both, split by mode - see below. | Changed |
| 4 | `size=default` radius | Answered for **cards**: 8px outer, 4px inner. The `Button` component's own value was not answered. | Changed (cards) |
| 5 | 536px quit dialogs | *"dialogs should all be centered"* - so 536 was a dragged edge, not a width. | Changed |
| 6 | Notifications: one row or three? | Ignore the question; the three-row split stays. | No change |
| 7 | Picker copy | New title, subtitle and `Apply selections`; no unselect-all; **Apply refuses until a selection actually changes**. | Changed |
| 8 | Two undrawn onboarding paragraphs | Closed without an answer; both stay. | No change |
| 9 | Tray tracking slips | `label/copy-12` and `-14` are **-1%**, `label/copy-16` is **-2%**. `heading/16` is a different style and stays -1%. | Changed |
| 10 | "Routing" or "Routed"? | Banner "partly routing", pill "Partly routed", nav rows "Not routed". | Already correct |
| 11 | Tooltip heading | `HH:00`. | Already correct |
| 12 | Redacted swatch | violet-500, not purple-500. Confirmed three ways in the token export. | Already correct |

### 1 - identifiers are sans, and numbers are tabular

This reverses the rule CLAUDE.md had carried since the first pass. The four
identifier classes the question named are drawn in the UI face on every frame,
and design confirmed that is deliberate: mono marks an eyebrow or a pill label
and nothing else. `font-mono` came off the config path, the picker's model rows,
the model list in `UseGateModelDialog`, the `identity` row's id, the App pane's
model id and activity reference, the sidebar's scan time, the update banner's
version, the chart's axis ticks, and the two key/URL input fields (whose `mono`
prop is gone rather than left unused).

Two things kept it, and neither is an identifier: the diagnostics report's
`<pre>`, for which the file names its own `mono/body-14` style, and the raw
backend error string under a Details disclosure. Both are machine output.

The sidebar and tray **eyebrow counters** also kept it. The comment on
`Sidebar.tsx` records them as sampled off the frame at Geist Mono Regular on
2026-08-23, they sit on the eyebrow's own line, and "the file wins" applies -
so flipping them on an inference about a rule aimed at identifier *values* was
not this pass's call. Raise it if design disagrees; they get tabular figures
either way, which is what the alignment argument actually wanted.

`tabular-nums` is set once per new-UI root - `AppShell`, `Tray`, `Onboarding` -
rather than on each figure. "Always" is what was asked for, and a root class is
the only version of it a later figure cannot miss.

**Landed on top of `#208`.** Matheus's model-selection work (`c630e9c`) went in
while these answers were being applied, and it reworks the same picker. Its
implementation is the base - compatibility filtering, the dev pin list, AG-590
enforced on the primary rather than on the row - and design's answers are
layered onto it. Where the two disagreed, the answer won and the reason is in
the section below.

### 2, 3 and 7 - the picker has two modes now

`ModelPickerDialog` takes `multiple`, defaulting to `true` because that is what
every tool does today. It is not a glyph switch; the two modes are different
interactions, which is also what settles question 3's contradiction:

- **Multiple** draws the frame's square checkbox per row and keeps `Cancel` /
  `Apply selections`. The primary is now refused until the draft is a different
  *set* from the one already applied - design, unprompted: *"if they open the
  modal after selections are applied, then the apply button is disabled"*.
  Compared as a set, not a sequence: reordering the same models is not a change
  to write. That sits alongside `#208`'s empty-draft refusal, which is where
  AG-590's "the final model cannot be removed" now lives; both guard the
  primary, so the row itself stays freely clearable.
- **Single** draws a circle-check on the highlighted model and nothing on the
  others, has no button row at all, and applies on the click - the new
  selection highlights, the old one un-highlights, the dialog closes. Neither
  guard applies: single-select cannot reach an empty set, and every click writes
  exactly one model.

Copy is design's: title `Choose Gate models` / `Choose a Gate model` by mode,
subtitle `<App> will be able to use these models` in both, primary `Apply
selections`.

**`Unselect all` is gone**, and that one is design removing its own control:
*"there isn't a select all option ... they can always cancel and start over, as
the selects are only applied with the button"*. `#208` had added it from Figma
682:20043 and hung the draft count on it, so three of its e2e cases and the
count row went with it - the checked rows state the set now, and the footer
states the consequence. Note the reason `#208` gave for moving AG-590 off the
row ("that cannot coexist with Unselect all") is moot as a result; the
placement stands anyway, because the primary is where the write happens.

**`multiple={false}` has no call site yet.** Nothing in the backend model says
which tools are single-model: `model_ids` is a list for every tool. Wiring it up
waits on the multi-model question with Matheus.

### 4 - a card inside a card drops to 4px

Design counted every card in the file: Overview 67, Settings 65, App 41, Sandbox
51, all 8px. The only 4px nodes are three inner frames in the App page's model
stack, which is exactly `ModelChoiceRow` x2 and `CurrentModelRow` - all three
were drawing `rounded-lg` (10px), a radius the design scale does not contain.

The picker's list is the worked example design actually cited, so it is an 8px
card now (it drew 10px) and its rows are 4px. `#208` had read the frame as
drawing the *chosen* row at 4px and the others looser, and commented it that
way; the rule overrides that, since these rows are inner cards. The selection
still reads - it carries the muted ground and a real border - it just no longer
changes shape.

One `rounded-lg` survives, on the App pane's amber drift note. It is a note
rather than a card and design's answer did not reach it; flagged, not changed.

The question as asked was about the `Button` component set (685:20928) and did
not get an answer, so the pane-4 / dialog-8 button rule stands on the frames as
before.

### 5 - there is no 536

Both quit-confirmation frames sit with their left edge at 255.64, which is
exactly where a 512 centred in 1024 starts, with the right edge 24px past
centre. *"Dialogs should all be centered"* settles it: they are off-centre 512s,
and `ModalWidth` is back to the four widths CLAUDE.md always claimed it had.
Every dialog was already centred in markup, so nothing moved but the width.

`#208` had gone the other way, documenting 536 as measured and warning that "a
scan that filters for centred frames will report no 536 anywhere and be wrong".
The measurement was right; it was the inference about intent that design
overturned. Worth keeping both halves in CLAUDE.md so the next reader does not
re-derive the frame and re-add the width.

### 9 - tracking belongs to the size

The reported slip was `label/14` drawn at both 0% and -1%, which is what happens
when a text style is split across a size class and a separate `tracking-*` that
a call site can forget. So the three values live in the `fontSize` tuples now -
`text-base-xs`, `text-sm` and `text-base` carry -0.12px, -0.14px and -0.32px -
and cannot be forgotten. Tailwind emits `letterSpacing` utilities after
`fontSize` ones, so `tracking-eyebrow` and `tracking-label` still win on the
eyebrows and pills that draw at those sizes; verified against built CSS rather
than assumed.

Safe to redefine Tailwind's own `sm` and `base` because neither reaches the
popover: `App.tsx`, `screens/Home.tsx` and `gc/ui.tsx` size themselves entirely
off the `gc-*` ramp. Both keep Tailwind's default line-height, so no leading
moved.

`#208` had answered the same report the other way, adding named
`tracking-label-12` / `-14` tokens and using them at 27 call sites. Both stay:
the named tokens mirror the Figma variables one-to-one, which is this file's
convention, and the tuples are the floor under them rather than a replacement.
They agree on 12 and 14 to the value.

**`heading/16` is a separate style at -1%, and that is the correction to make
here.** `#208` verified it off the frames (`card/policies` 116:26707 and
siblings draw their titles 24px tall) and put `tracking-heading-16` on the
Settings and Overview card headings. Design's -2% answer named `label/copy-16`,
not `heading/16`, so both hold and the headings keep their override - the -2%
default is for `copy/16` body text. The one call site design's answer governs
directly is the onboarding block already commented `copy/16`, which lost its
`tracking-heading` (that was `heading/20`'s -1% on 16px text, a mismatch
predating the question).

This was checked against Figma before deciding, and the check was
**inconclusive**: MCP exposes only the `Design docs` page now, whose token
export carries no `letterSpacing` at all - `label/N`, `copy/N`, `heading/N` and
`mono/eyebrow` are text styles rather than variables in those collections, and
the Flows pages remain invisible to MCP. The resolution above rests on `#208`'s
frame reads plus the fact that design named only `label/copy`. Worth one line of
confirmation.

The three wordmarks keep their measured `tracking-[-0.16px]` literal. The lockup
is 16px semibold at -1% and is neither `label/copy-16` nor a heading, so letting
the new `text-base` default take it to -2% would have been a change on no
evidence. The tray's two hardcoded `tracking-[1.12px]` runs became
`tracking-eyebrow-14`, the same value under its own name, which is the other
half of what question 9 reported.

### What the Figma check did settle

The `Design docs` page independently confirms **question 12**: `--color-chart-4`
is `--tw-violet-500`, `tokens.json` gives `chart.4` as `#8b5cf6`, and `design.md`
spells out "4 - Redacted: violet-500 (#8b5cf6)". Three agreeing sources, so
`#208`'s note parking this one as unresolved can be closed.

It also flags something for later: the same export gives **chart 2 (Blocked) as
red-500 `#ef4444`**, while `#208` moved it to red-400 `#f87171` on two frame
nodes. The export was generated 2026-08-20 and those nodes are newer, so
"match what the frame renders" points at red-400 and nothing was changed here -
but the token export still disagrees, which is worth knowing before anyone
samples it again.

## Window size, 2026-09-04: 1280x800 on a 1024x800 floor

The main window was 1024x720 on a 1024x720 floor, from the `App dimensions:
1024x720px` annotation the first pass read. `overview-dimensions` (`864:3466`)
supersedes it: the frame is **1280x800**, and unlike the annotation it can be
checked, because it decomposes to the pixel.

| Layer | Width | From |
| --- | --- | --- |
| window | 1280 | frame `864:3466` |
| sidebar | 256 | `864:3472`, `shrink-0` |
| content column | 1024 | `864:3473` at x=256 |
| pane | 976 | `864:3474`, 24px padding each side |
| chart | 944 | `864:3509`, 16px card padding each side |

944 is the number that matters, and it is why the width moved. The drawn bucket
is a 32px bar on an 8px gap (`864:3511` is 31.667 wide, the next starts at
39.667), so 24 of them need `24x32 + 23x8 = 944` exactly. The old window left
the pane 768px, which is 24 axis labels at `w-8` touching edge to edge with
nothing between them - the crowding that prompted this.

**The floor is 1024x800, not 1280x800.** Height cannot go below the drawn 800;
width can, and a window between 1024 and 1280 crowds the axis by that same
arithmetic. Raising `minWidth` to 1280 is what would make the fit unconditional,
and it is deliberately not done here - it would forbid the 1024 width the design
used until today, and it was not asked for. Worth deciding rather than drifting
into.

Two duplicated values, both intentional and both moved together:
`tauri.conf.json`'s `minWidth`/`minHeight`, which macOS, Windows and X11
enforce, and `MAIN_MIN_SIZE` in `src-tauri/src/lib.rs`, which is the only thing
a Wayland session enforces. The comment on the constant says so; it is now
`(1024.0, 800.0)`.

The `onboarding` window is untouched at 1080x720 on a 760x560 floor. It is built
in Rust rather than configured, it is a different surface, and nothing about the
24-bucket chart applies to it.

### Not changed: the bars are still 20px

The same frame says the bar is **32px** wide on an **8px** gap.
`metrics.tsx` draws `w-5` (20px) on `gap-1` (4px). The axis labels are already
`w-8`, so with `justify-between` in a 944px chart the *labels* now land within a
few tenths of a pixel of the drawn 39.667px step - which is why the width alone
fixes the reported crowding. The bars themselves stay narrow until someone asks:
it is a visual change to the chart rather than a window-size one, and this pass
was about the window.
