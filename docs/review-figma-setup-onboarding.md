# Figma audit: Setup and Onboarding

Read 2026-08-30 over the official Figma MCP, file `9FrccCojXy0f8QD8Wm5Lln`.
5 MCP calls: 2 `get_metadata` (both page nodes, full trees), 3 `get_screenshot`
(`209:84046`, `710:36505`, `710:36133`). Nothing failed; both page trees were
read whole.

## Summary

**`Flows / Setup` (177:79238) has barely moved and the build still matches it.**
Four sections, same four as the 2026-08-21 read, same frames. The only recent
node ids on the page are `686:23557-23562` (the org picker's header row
re-instanced, no measurable change against its `229:*` siblings) and the `451:*`
batch already read on 2026-08-26. Copy on sign-in, API key, name-device,
organizations and diagnostics is verbatim what `setup.tsx` ships. What is wrong
is smaller: the drawn progress rail runs on quarters (25 / 50 / 50 / 75 / 100)
and the build invents six fractions of its own; the org picker's many-orgs state
scrolls its list inside a fixed card and the build scrolls the whole pane.
**`Auth / Error states` still carries no ready check - fifth read running** - so
the setup-timeout dialog and the device-name validation rule stay correctly
unbuilt.

**`Flows / Onboarding` (177:79237) moved, and it moved in the art.** The four
frames, their copy, their eyebrows, their notes, their footers and the progress
rail are all unchanged from the 2026-08-26 build and match it. But steps 2 and 3
had their illustrations **redrawn as live component mockups in the `710:*`
batch** - the same batch as the 2026-08-28 Overview redraw - and both built
assets are now stale. Step 2's is worse than stale: the shipped PNG teaches the
retired popover (Proxy / Direct Gateway rows) inside a macOS desktop, where the
frame now draws the new popover with the app list. The page also **lost its
ready mark**: the section is now labelled `Onboarding / Main flow`
(`665:19161`, a new node) with no check, and `Onboarding / Images`
(`604:13754`) has none either.

## Ready checks

Read off the top-level section labels in each page's metadata. I cannot see the
Pages-list check over MCP - only these canvas labels.

`Flows / Setup` (177:79238):

| Section label | Node | Check |
| --- | --- | --- |
| `Auth / Connect with Gate ✅` | `209:84348` | yes |
| `Auth / Connect with API key ✅` | `209:84351` | yes |
| `Auth / Organizations ✅` | `230:91098` | yes |
| `Auth / Error states` | `231:2366` | **no** |

`Flows / Onboarding` (177:79237):

| Section label | Node | Check |
| --- | --- | --- |
| `Onboarding / Main flow` | `665:19161` | **no** |
| `Onboarding / Images` | `604:13754` | **no** |

The Onboarding labels are the change. On 2026-08-26 the plan recorded the page as
carrying the check; `665:19161` is a newer node than the `604:*` Images section,
so the main-flow label was added or renamed after that read and arrived without
one. Given the `710:*` art redraw underneath it, treat the whole page as moving
again.

The Setup labels still read `Auth / …` although the page itself was renamed
`Setup` on 2026-08-21. Cosmetic, designer's to fix.

## Findings

| What | Figma (node + drawn value) | Built | Verdict |
| --- | --- | --- | --- |
| Onboarding step 2 illustration | `710:36505` `introduction_img_2`, 590x220: the **new** Gate Connect popover panel - "Gate Connect" lockup with an `Expand app` button, a green "Gate is protecting you / On · 7 of 7 routing" row, and the sidebar app list (ANTHROPIC / OPEN AI / OPENROUTER / OTHER TOOLS), footer "Acme Engineering" + `Quit` | `src/screens/Onboarding.tsx:100-133` uses `where-is-gate-connect-{macos,linux,windows}.png` - a macOS/Windows/Linux desktop mockup whose popover is the **retired** one (`Proxy · On · intercepting 3 domains`, `Direct Gateway`) | **DRIFT** |
| Onboarding step 3 illustration | `710:36133` `introduction_img_3`, 590x220: Overview with org selector "Chad's organization", groups **ANTHROPIC** / OPEN AI, banner "Gate Connect is protecting" / "Routed · **8 of 8** Apps", Claude Desktop switch **off** | `src/screens/Onboarding.tsx:135-149`, `src/assets/onboarding-see-what-gate-is-doing.png` (captured 2026-08-20): "Acme Engineering team", group **CLAUDE**, "Gate Connect is protecting **you**", "Routed · **4 of 4** Apps", both switches on | **DRIFT** |
| Setup progress rail stops | `prog-track` fills, 1024 track: sign-in `202:80095` **256 (25%)**; API key `209:84185` **512 (50%)**; org picker `208:81532` **512 (50%)**; name device `209:84046` **768 (75%)**; diagnostics `363:12645` **1024 (100%)** | `src/NewUiApp.tsx:1685-1697`: welcome `0.1`, api-key `0.25`, org-picker `0.4`, name-device `0.6`, diagnostics `0.8` | **DRIFT** |
| Org picker, many organizations | `229:90953` card fixed at 496x**496**; `229:91001` "row" 446 wide holds `229:91072` "stack" at **432** - a 14px gutter - so the list scrolls inside the card and the header and buttons stay put | `setup.tsx:624-635` renders every org in a plain column; `SetupLayout`'s content area (`setup.tsx:74`) scrolls the whole card instead | **DRIFT** |
| Org picker header tile | `686:23559` icon-wrapper **44x44** with a 24px `Icon / UsersRound` (all Organizations frames agree: `229:90737`, `229:90810`, `229:90883`, `231:2130`, `209:84632`) | `HeaderTile size={48}` via `SetupHeader`'s centred branch (`setup.tsx:98-122`, `:178`) - 48x48 for every pane | **DRIFT** |
| Setup timeout dialog | `231:2344` (496x198 over the org picker): title "Setup timeout", body "Gate Connect timed out will trying to process your request. Would you like to try your request again, or go back to the setup start?", 32px `Icon / TimerReset` tile, right-aligned `Button` 104x36 + 83x36 | nothing | **RECORDED** - `Auth / Error states` has no check (5th read) |
| Device-name validation | `231:2371`: the name field failing with "Incorrect characters or symbols used" | `NameDevicePane` (`setup.tsx:521-562`) only disables the primary on an empty field; no character rule | **RECORDED** - same section, no check |
| Timeout dialog's subtitle | `231:2344` carries "Your next requests will use Constellation Gate PAYG credits" under "Setup timeout" - the org picker's PAYG line, pasted onto a timeout. Plus "timed **out will** trying" | n/a | **DESIGN BUG** (in an unready section - raise, do not build) |
| Sign-in frame's rail stop | `202:80095` (Connect with Gate) draws **256**; the identical sign-in frame `209:84133` in the Connect with API key section draws **512**. Same pane, two stops | n/a | **DESIGN BUG** |
| Floating "Name this device" variant | `451:7906` at y=-1125, **above every section label and inside none**: card 496x**283**, header as a 32px tile beside the title, footer `button-group` 494x69 with right-aligned `Button` 104x36 + 83x36 | `setup.tsx:129-130` documents exactly this archetype (`row` + "`SetupFooter`"), but `row` has no call site and `SetupFooter` does not exist | **RECORDED** - unlabeled draft, not ready |
| Retired 4-step onboarding frame | `212:85283`, `hidden="true"`: "How to connect with Gate", eyebrow "Introduction / **3 of 4**", Config apps / Proxy apps split | nothing | **OK** - hidden in the file, correctly not built |
| Step 2 alternate subtitle | `212:85218` "Use the full Gate Connect window to monitor activity, inspect each AI tool, choose Gate models, and resolve issues…" is `hidden="true"`; the visible node is still `232:3544` "Gate Connect stays open in your menu bar…" | `Onboarding.tsx:124-126` follows `232:3544` (with the recorded platform-aware first sentence) | **OK** |
| Secondary actions in Setup panes | Every pane's second `Button` instance is 446x44, but the render (`209:84046` screenshot) shows it as a **centred underlined blue text link** ("Skip naming") | `SetupLink` (`setup.tsx:222-232`) - same treatment | **OK** |
| Onboarding progress rail | welcome `402:13843` 256, step 1 `212:85064` 512, step 2 `221:85579` 768, step 3 `221:85589` 1024 - quarters | `Onboarding.tsx:183-208`, `:320`: `step/total` over 4 steps = 25/50/75/100 | **OK** |
| Onboarding footer buttons | welcome `232:4448`/`232:4449` and every step: two `Button` 104x36 at x=0 and x=116 - a 220px pair, 12px gap | `Onboarding.tsx:457` `w-[220px] … gap-3`, `IntroButton` h-9 | **OK** |
| Locate button | `267:5083`, 242 wide, outside the card between it and the footer, `Focus` glyph | `Onboarding.tsx:408-415` | **OK** |
| Note glyphs | step 1 `212:85071` ShieldCheck, step 2 `212:85266` MonitorSmartphone, step 3 `231:3501` BellDot | `Onboarding.tsx:97,131,156` | **OK** |
| Setup copy, all panes | `206:80566`, `686:23562`, `209:84185`/`209:84239`, `209:84071`, `363:12645` | verbatim in `setup.tsx:446,491,541,602,730-735` | **OK** |
| Org row states | `451:8008` selected `Icon / CircleCheck`, `451:8016`/`451:8024` unselected `Radio` | `ModalOption` (`Modal.tsx:448-495`) | **OK** |
| Empty-org state | `231:2102`: amber note "No organizations found." / "You will need to setup your first organization through Gate AI before continuing to setup Gate Connect.", `Icon / CircleAlert`, then two full-width buttons | `setup.tsx:607-621` - same copy, Go back + Use a different account | **OK** (built glyph is `triangleAlert`, drawn is `CircleAlert` - not worth a line of its own) |

### Panes the file does not draw

Unchanged from the record, restated so they are not read as missing: there is no
frame for the **"You're connected" confirmation** (`ConnectedPane`,
`setup.tsx:653-689`), none for the **"Session expired" reauth** variant
(`setup.tsx:433-447`), and none for the **gateway picker**
(`setup.tsx:345-401`). All three are deliberate build additions.

## Ranked

1. **Onboarding step 2 art** - a first-run user is shown a popover the product no
   longer has. Highest impact of anything here, and it is on the screen whose
   whole job is "this is what the thing looks like".
2. **Onboarding step 3 art** - stale in four visible details, including a
   sidebar group label (`CLAUDE`) the app stopped using on 2026-08-23.
3. **Setup rail stops** - wrong on all five setup screens; no test pins the
   current fractions, so this is a five-line change.
4. **Org picker many-orgs scroll** - with enough organizations the header and
   the Continue button scroll away, where the design keeps them fixed.
5. **Org picker header tile** - 4px, one pane.
