# New app UI: matching the Gate Connect Figma

Source: https://www.figma.com/design/9FrccCojXy0f8QD8Wm5Lln/Gate-Connect
File key `9FrccCojXy0f8QD8Wm5Lln`, single page `Components` (113:16762).

## What the Figma actually contains

The file has five pages, only one of which the MCP `get_metadata` call
reported: `Components`, `Flows` (containing `Overview`, `Settings`, `App`),
`Sandbox`, and `Icons`.

- **Overview** - dashboard: stat tiles, Messages bar chart, Policies table,
  Token savings table. Annotated **App dimensions: 1024x720px**.
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
Families nav item; ours stays, per the standing deviation.

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
The standing deviation stands.

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
`Modal`. The Notifications description keeps the honest routing-health copy
over the drawn "blocked or flagged" for the reason recorded on 2026-08-21, and
the Help section and the Sign-in method / certificate / What-is-collected rows
remain the standing undrawn deviations. 548 unit tests and the 169-test e2e
suite pass.

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

- `tauri.conf.json`: **DONE 2026-08-17.** 1024x720, `resizable: true`,
  `decorations: true`, `alwaysOnTop` and `skipTaskbar` off. `visible: false`
  is unchanged: the tray still owns the first show.
- **The new UI is now the default** (`newUi.ts`); the popover is the fallback,
  reachable with `gcNewUi(false)` or a `VITE_NEW_UI=0` build.
- **Still popover-shaped on the Rust side.** `show_popover` calls
  `anchor_under_tray`, so the window is positioned under the tray icon rather
  than where the user left it, and `set_activation_policy(Accessory)` keeps it
  out of the dock. Both are wrong for a 1024x720 window and neither is changed
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
3. **Groups: RESOLVED 2026-08-16 - they keep a home.** The design lists apps
   flat, which cannot express routing's real shape: families (Claude, OpenAI,
   OpenRouter, plus the multi-provider "Other tools" bucket) own a master
   switch, and their members route either through a tool's own config file or
   through the local proxy - the latter being the chat domains, which the drawn
   UI does not show at all. `FamiliesPane` gives them a third sidebar
   destination. **Not in the Figma**; expect it to be redrawn.
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

- **Blocked-event, flagged-event and sound switches.** The criteria list four
  notification switches. The app fires exactly two notifications, both about
  routing (an expired session; a quit that could not put a tool back), and both
  now ride the one `Routing health` switch. Blocked and flagged notifications
  need the live security-event feed (AG-578), which does not exist. A switch for
  an event that cannot arrive tells the user they turned something off.
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
answer to a reading the user has no reason to know is stale. Detection polls
instead, on `DETECT_POLL_MS` (5s) in `NewUiApp`.

What the poll reads is deliberately narrower than `refresh`. `list_tools` walks
config files and `proxy_status` reads memory, so both are fine on a timer;
`routing_verdicts` probes the relay *and* the gateway session, so it is not. The
poll compares both readings against the last ones rendered - `detectionSignature`
over a `rendered` ref, kept current by an effect on the state itself so that a
toggle's own re-read counts as drawn - and commits nothing when they match. Every
memo below hangs off `tools` and `proxy`, so re-setting an equal-but-new object
every five seconds would rebuild the families, the settings sections and the
routing callbacks for no change. When either does move the sweep runs: a row that
just appeared has no verdict and reads "Checking", and the engine coming up
changes all of them at once.
Hidden windows skip their ticks and read once on `visibilitychange`, so nothing is
spent on a minimized window and coming back does not show a stale list. Two e2e
tests pin the split: the list is re-read with nothing asking, and an unchanged
machine does not re-run the sweep.

`scan` is still written on every tick even when nothing changed: the timestamp is
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
