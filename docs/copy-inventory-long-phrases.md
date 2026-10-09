# Copy inventory: long phrases (2026-10-09)

Every user-facing string of **12 words or more** in `src/` (tests excluded), found by walking the TypeScript AST for string literals, template literals and JSX text. Grouped by surface, longest first. 110 notices plus 3 alt texts.

Excluded as not user-facing: `console` logs in `useRouting` / `useSectionRouting`, the developer `note:` fields in `modelCompatibility.ts`'s `KNOWN_GOOD` table, and SVG paths.

**Known gaps of the scan.** A sentence assembled from several JSX branches is either partial or missing:
- `ErrorBoundary.tsx:67` "This window hit an unexpected error." + one of four routing sentences.
- `dialogs.tsx:1159` "N models are shown but cannot serve this app: <reason>" (reason from `modelCompatibility.ts:294-296`).
- `dialogs.tsx:2148` the teardown failure sentence (listed, but with placeholders).

## Figma check (2026-10-09)

Every text node of 6+ words was pulled from the flow, component and Sandbox pages of the Gate Connect file and fuzzy-matched against each row, then the borderline matches were read by hand. The **Figma** column says:

- **verbatim `node`**: the frame draws this text. 18 rows. Shortening one is a copy deviation to raise with design.
- **differs `node`**: drawn, but the code has drifted from it. 3 rows (`dialogs.tsx:449/454`, `setup.tsx:756`).
- **deviation `node`**: a rewrite CLAUDE.md already records (`dialogs.tsx:1658` Disconnect, `notices.ts:74` "didn't start"). 2 rows.
- **no**: no frame draws it. 87 of 110 notices. These are local copy and can be shortened freely.

Every `errors.ts`, `groups.ts`, `modelAttention.ts`, `activityGaps.ts`, data-sharing and model-picker string is undrawn: the frames only draw happy paths and a handful of dialogs.

## Rust notifications (already short)

| Location | Text |
|---|---|
| `src-tauri/src/lib.rs:4727` | Your session expired. Open Gate Connect to sign in again and keep routing. (13w) |
| `src-tauri/src/lib.rs:5139` | Accept the verification screen to continue with chat |
| `src-tauri/src/lib.rs:5632-5636` | Gate removed from tool configs / Failed to remove Gate from the {one} config. Edit it by hand. |
| `crates/core/src/security_feed/notify.rs:213-248` | Gate {verb} a {cat} match in {tool}. / Gate {verb} {count} more ... |

## Onboarding (first-run window) (10)

| Words | Location | Figma | Text |
|---:|---|---|---|
| 47 | `src/screens/Onboarding.tsx:89` | no | For Claude Code and Codex, Gate Connect points the app’s own config at your gateway and restores it when you disconnect. For apps like Claude Desktop or ChatGPT, it routes the provider’s domain through a local proxy. Your Gate key stays in {secretStoreName(platform)}, not a plain file. |
| 31 | `src/screens/Onboarding.tsx:67` | verbatim `240:4508` (1st half) | Gate Connect routes the AI apps you already use through Gate. Every message is checked on the way: prompt-injection attempts are stopped, sensitive values are redacted, and compression trims token spend. |
| 29 | `src/screens/Onboarding.tsx:113` | verbatim `232:3544` (frame's menu-bar sentence is per-platform in code) | Click the Gate Connect icon to open the compact popover for a quick status check, or expand it to the full desktop app for more details, alerts, and controls. |
| 28 | `src/screens/Onboarding.tsx:68` | verbatim `240:4508` (2nd half) | You sign in once, choose the apps to cover, and keep working as usual. Setup is only a few short steps to complete. Click Next to get started. |
| 23 | `src/screens/Onboarding.tsx:88` | verbatim `212:84775` (rest) | You sign in once, choose the apps you want covered, and keep working in Claude, Codex, OpenCode, and other supported apps as usual. |
| 22 | `src/screens/Onboarding.tsx:138` | verbatim `212:85415` | Once requests pass through Gate, the desktop app shows recent activity, security actions, and compression savings without exposing prompt or response content. |
| 17 | `src/screens/Onboarding.tsx:86` | verbatim `212:84775` (1st sentence) | Gate Connect is your desktop app to route the AI apps you already use through Gate AI. |
| 16 | `src/screens/Onboarding.tsx:139` | no | That’s all there is to it. Sign in and your first app is one toggle away. |
| 15 | `src/screens/Onboarding.tsx:115` | verbatim `212:85267` | Open the desktop app for detail. Collapse to a popover for a fast status check. |
| 12 | `src/screens/Onboarding.tsx:140` | verbatim `231:3502` | Notifications will alert you when a request has been blocked or flagged. |

## Setup flow (7)

| Words | Location | Figma | Text |
|---:|---|---|---|
| 50 | `src/components/gc/setup.tsx:756` | differs `373:13518`: code adds setup steps, org/account attribution and the final-note sentence | Opt-in to send Gate errors, routing stats and setup steps to help fix problems, tied to your organization and, if you signed in with Constellation, your account. Saying no or skipping still sends one final note saying so, tied the same way. Prompts, credentials, or private information are never shared. |
| 25 | `src/components/gc/setup.tsx:456` | verbatim `206:80566` | Sign in once, then choose which AI apps route through Gate. Claude, Codex, OpenCode, and supported apps keep working normally while Gate handles protection underneath. |
| 22 | `src/components/gc/setup.tsx:501` | verbatim `209:84602` | Paste a Gate API key from your dashboard. After it connects, you will name this device before choosing which apps are protected. |
| 17 | `src/components/gc/setup.tsx:627` | verbatim `231:2188` | You will need to setup your first organization through Gate AI before continuing to setup Gate Connect. |
| 16 | `src/components/gc/setup.tsx:550` | verbatim `209:84077` | Naming will help you tell this device apart from the others connected to your Gate account. |
| 16 | `src/components/gc/setup.tsx:615` | verbatim `686:23562` | Gate Connect will use the selected organization for routing, activity, and PAYG credits on this device. |
| 13 | `src/components/gc/setup.tsx:454` | no | Sign in again whenever you want to start routing your apps through Gate. |

## Dialogs (32)

| Words | Location | Figma | Text |
|---:|---|---|---|
| 65 | `src/components/gc/dialogs.tsx:1573` | no | OpenCode's own settings cover the providers you had set up when you turned it on. Anything you add later reaches Gate through your machine's proxy variables instead, and those apply to every command line tool that reads them, git, curl and npm included. One of them also tells Node to trust Gate's certificate, so every Node program you start afterwards accepts the traffic Gate inspects. |
| 45 | `src/components/gc/dialogs.tsx:1831` | no | A device id generated on this machine. Once you sign in, and while sharing is on, your organization id too, and your account id if you signed in with Constellation, so setup can be measured from download to first request. Never your name or email. |
| 32 | `src/components/gc/dialogs.tsx:1865` | no | When something fails, the state Gate was in: your operating system version, which tools are installed, whether routing was on, and whether the event stream was connected. Not what you were doing. |
| 31 | `src/components/gc/dialogs.tsx:1842` | no | The first time you say no, or turn sharing off later, one final note says so, with your organization id, and tied to your account if you signed in with Constellation. |
| 31 | `src/components/gc/dialogs.tsx:2148` | no | Couldn’t put {joinNames(tools)} back on {plural ? "their own settings" }. {plural ? "They still point" : } at Gate, which has no session behind {plural ? "them" : "it"} now. |
| 29 | `src/components/gc/dialogs.tsx:1894` | no | Which app made the request, when Gate can tell from the request itself - Claude Code, Codex, and so on. Unrecognised apps are sent unlabelled rather than guessed at. |
| 28 | `src/components/gc/dialogs.tsx:1848` | no | Which action happened, from a fixed list - routing turned on or off, an app connected, a setup step completed or failed, an update installed. Never free text. |
| 23 | `src/components/gc/dialogs.tsx:316` | no | Constellation sign-in keeps your session in {secretStore} and refreshes it on its own, so there is nothing to rotate when a key expires. |
| 23 | `src/components/gc/dialogs.tsx:449` | differs `1336:13329`: frame opens "If Gate takes over:" | Gate Connect saves a private snapshot of these settings, replaces only the routing fields, and keeps the credential in your operating system keychain. |
| 23 | `src/components/gc/dialogs.tsx:1889` | no | The same device id, stored with each request your account sends, so your activity view can group requests by machine. It authorizes nothing. |
| 22 | `src/components/gc/dialogs.tsx:1269` | no | Gate needs at least one model to serve this app. Choose one, or cancel and switch the app back to App default. |
| 22 | `src/components/gc/dialogs.tsx:1383` | no | Gate sets {app.name}'s model to these in its own config. Return to App default at any time to restore your previous model. |
| 22 | `src/components/gc/dialogs.tsx:1658` | **deviation** `164:73502` (documented exception in CLAUDE.md) | This device signs out of Gate and stops sending activity. Your apps keep their current configuration, and signing back in restores routing. |
| 21 | `src/components/gc/dialogs.tsx:1599` | no | Terminals and tools that are already open keep the environment they started with. Reopen them if you want their traffic covered. |
| 20 | `src/components/gc/dialogs.tsx:344` | no | Your gateway and your routing stay exactly as they are. You can switch either way later, under Connection in Settings. |
| 20 | `src/components/gc/dialogs.tsx:1772` | no | Automatic collection never includes your name, email or keys. A report you send yourself carries more, and is listed last. |
| 19 | `src/components/gc/dialogs.tsx:1853` | no | A short label for each action: which app or provider it concerned, and whether it was on or off. |
| 18 | `src/components/gc/dialogs.tsx:1047` | no | This gateway offers no models of its own, so apps keep using the model they are configured with. |
| 18 | `src/components/gc/dialogs.tsx:1275` | no | Requests for the models enabled here consume Gate credits. Gate never uses a model you have not enabled. |
| 18 | `src/components/gc/dialogs.tsx:1923` | no | Your email, organization name and organization id, so a support thread can find the account it is about. |
| 17 | `src/components/gc/dialogs.tsx:454` | differs `1336:13329` (2nd half): fixes the frame's "disconnected Gate Connect, or a complete reset" | Your configuration is restored when you turn protection off, disconnect Gate Connect, or do a complete reset. |
| 16 | `src/components/gc/dialogs.tsx:269` | no | Your stored key is forgotten, managed tools disconnect, and Gate Connect relaunches against the new server. |
| 16 | `src/components/gc/dialogs.tsx:1037` | no | Nothing has changed: this app keeps the model it is using. Close this and try again. |
| 16 | `src/components/gc/dialogs.tsx:1857` | no | A classified title when something fails, e.g. "keychain denied". The underlying message stays on this machine. |
| 16 | `src/components/gc/dialogs.tsx:1927` | no | Where Gate keeps its files, and the gateway, proxy and relay addresses this machine is using. |
| 16 | `src/components/gc/dialogs.tsx:1931` | no | Which tools are installed, their routing status, and which agents were running when you sent it. |
| 16 | `src/components/gc/dialogs.tsx:2045` | no | The report is on its way to Constellation Gate. Routing and event delivery were not interrupted. |
| 16 | `src/components/gc/dialogs.tsx:2057` | no | The report is on its way to Constellation Gate. Routing and event delivery were not interrupted. |
| 15 | `src/components/gc/dialogs.tsx:413` | verbatim `1336:13317` | Gate found settings that it didn’t create. They will not be replaced without your approval |
| 13 | `src/components/gc/dialogs.tsx:624` | verbatim `363:9033` (fixes the frame's "this installed") | The state of this install, as text you can hand to someone else |
| 13 | `src/components/gc/dialogs.tsx:2017` | no | Paste this reference into your support request so we can find the report. |
| 13 | `src/components/gc/dialogs.tsx:2018` | no | One report, sent once. This does not change your Share diagnostic data setting. |

## Certificate / routing notes (groups.ts) (11)

| Words | Location | Figma | Text |
|---:|---|---|---|
| 57 | `src/lib/groups.ts:226` | no | Gate added its certificate to your {store}, but at least one of the separate stores {family} keep would not take it. The diagnostics report names which one and why; a Firefox profile with a Primary Password needs the certificate imported in Firefox’s own settings. Once it is fixed, turn routing off and on again to retry. {BROWSER_RESTART} |
| 38 | `src/lib/groups.ts:217` | no | Gate added its certificate to your {store}, but {family} each keep a separate one, and Gate needs certutil to write it. Install it (Debian/Ubuntu: libnss3-tools, Fedora/RHEL: nss-tools), then turn routing off and on again. Command-line tools are unaffected. |
| 34 | `src/lib/groups.ts:236` | no | Gate added its certificate to your {store}, but {family} each keep a separate one and Gate has not written those, so turn routing off and on again to add it. Command-line tools are unaffected. |
| 34 | `src/lib/groups.ts:286` | no | Gate removed its certificate from your certificate store, but needs certutil to remove it from the browsers’ own stores and it is not installed. Remove the Gate Connect certificate in each browser’s certificate settings. |
| 31 | `src/lib/groups.ts:292` | no | Gate removed its certificate from your certificate store, but at least one browser’s own store would not let go of it. Remove the Gate Connect certificate in that browser’s certificate settings. |
| 20 | `src/lib/groups.ts:245` | no | Gate has added its certificate to your {store} and your browsers. A browser reads its certificates when it starts. {BROWSER_RESTART} |
| 19 | `src/lib/groups.ts:623` | no | Routes every program you start from now on, not only AI tools, and tells Node to trust Gate's certificate. |
| 17 | `src/lib/groups.ts:246` | no | Gate has added its certificate to your {store}. A browser reads its certificates when it starts. {BROWSER_RESTART} |
| 12 | `src/lib/groups.ts:91` | no | Chats in the Claude desktop app, and on claude.ai in a browser |
| 12 | `src/lib/groups.ts:97` | no | Chats in the ChatGPT desktop app, and on chatgpt.com in a browser |
| 12 | `src/lib/groups.ts:261` | no | Quit and reopen any open browser so it stops trusting the certificate. |

## Model attention and compatibility (7)

| Words | Location | Figma | Text |
|---:|---|---|---|
| 42 | `src/lib/modelAttention.ts:128` | no | The last few requests from this app have failed while it has been on Gate models. The model may not work with this app. Switch back to App default to use the app's own model again, or choose a different Gate model. |
| 34 | `src/lib/modelAttention.ts:146` | no | None of the {gone.length} models chosen here are available from Gate any more. Requests will fail until you choose another or return to App default - Gate will not pick a replacement for you. |
| 29 | `src/lib/modelAttention.ts:165` | no | There are no Gate credits left, so requests using a Gate model will fail. Add credits, or return this app to App default to use its own model again. |
| 28 | `src/lib/modelAttention.ts:157` | no | Pay-as-you-go is off for this organization, so Gate cannot serve a model for it. Requests will fail until it is enabled or this app returns to App default. |
| 17 | `src/lib/toolModels.ts:196` | no | You switched {appName} to {toModel ?? "another model"} in {appName}, so it is back on App default. |
| 16 | `src/lib/modelCompatibility.ts:294` | no | Gate does not list tool support for this model, and {appName} sends tools with every request. |
| 15 | `src/lib/modelCompatibility.ts:296` | no | These models were tested and verified to reject the form {appName} sends its tools in. |

## Error hints (errors.ts) (14)

| Words | Location | Figma | Text |
|---:|---|---|---|
| 22 | `src/lib/errors.ts:290` | no | This tool sends all of its traffic through Gate’s local proxy, so routing has to be on before it can be connected. |
| 21 | `src/lib/errors.ts:305` | no | Click Trust again and choose Yes in the Windows security warning. Apps with no gateway setting can’t route until it’s trusted. |
| 21 | `src/lib/errors.ts:546` | no | Hermes already has its own proxy settings in ~/.hermes/.env, and Gate left them alone. Remove them to route Hermes through Gate. |
| 19 | `src/lib/errors.ts:419` | no | Another copy of Gate Connect, or a gate-connect proxy relay, is using it. Quit that one, then try again. |
| 19 | `src/lib/errors.ts:533` | no | OpenCode isn’t signed in to a provider Gate can route. Run opencode auth login, then turn OpenCode on again. |
| 18 | `src/lib/errors.ts:404` | no | Gate stopped waiting after five minutes. Try again, and complete the sign-in in the browser window that opens. |
| 16 | `src/lib/errors.ts:434` | no | Check that you’re online and that the gateway URL in Settings is right, then try again. |
| 15 | `src/lib/errors.ts:276` | no | This tool or action isn’t supported here yet. The details below help when reporting it. |
| 14 | `src/lib/errors.ts:386` | no | Allow Gate Connect to use {store} in your OS privacy settings, then try again. |
| 13 | `src/lib/errors.ts:454` | no | Open Settings and reconnect: sign in again, or replace your Gate API key. |
| 13 | `src/lib/errors.ts:511` | no | Try again. If it keeps failing, the details below help when reporting it. |
| 13 | `src/lib/errors.ts:541` | no | Codex isn’t signed in yet. Run codex login, then turn it on again. |
| 12 | `src/lib/errors.ts:324` | no | Try again and approve the sign-in in the browser window that opens. |
| 12 | `src/lib/errors.ts:452` | no | Check that the key is for this gateway, then paste it again. |

## Panes, rail, tray, banners (10)

| Words | Location | Figma | Text |
|---:|---|---|---|
| 22 | `src/components/gc/Tray.tsx:510` | no | Gate Connect could not reach your stored credential, so it cannot tell what is routed. Open the app window to try again. |
| 21 | `src/components/gc/Tray.tsx:531` | no | Gate Connect needs a Gate account or API key before it can route your tools. Sign in from the app window. |
| 19 | `src/components/gc/Sidebar.tsx:514` | no | Gate looked for supported AI apps and found none installed. Install one and refresh, and it will appear here. |
| 18 | `src/components/gc/AppPane.tsx:532` | no | Gate could not read this app's model setting, so it is not shown. The setting itself is unchanged. |
| 18 | `src/components/gc/Sidebar.tsx:513` | no | Gate couldn’t read this device’s app list, so it doesn’t know what is installed. Nothing has been changed. |
| 18 | `src/components/gc/banners.tsx:236` | verbatim `1408:22471` | It was already running when its configuration changed, so it is still using the route it started with. |
| 14 | `src/components/gc/SettingsPane.tsx:514` | verbatim `361:8388` | Send Gate errors and routing stats to help fix problems. Never prompts or credentials. |
| 14 | `src/components/gc/SettingsPane.tsx:553` | no | Send one report to Constellation Gate and get a reference for your support request. |
| 14 | `src/components/gc/SettingsPane.tsx:634` | verbatim `130:48898` | Turn routing off, disconnect tools, remove this account or key, and start setup again. |
| 13 | `src/components/gc/AppPane.tsx:641` | no | No Gate model chosen yet. Choose one to write it into {appName}'s config. |

## Window-level dialogs (NewUiApp / TrayApp) (7)

| Words | Location | Figma | Text |
|---:|---|---|---|
| 35 | `src/NewUiApp.tsx:3221` | no | When you trust a new certificate, quit and reopen any AI tools that are running. They read the certificate when they start, so one that is already open will fail to connect until you do. |
| 24 | `src/NewUiApp.tsx:3203` | no | Routing turns off while the certificate is gone, and your tools keep their configuration. Your operating system may ask for permission to remove it. |
| 16 | `src/NewUiApp.tsx:3162` | no | The certificate stays on this machine, and you can remove it from Settings at any time. |
| 16 | `src/NewUiApp.tsx:3183` | no | Sites and apps routed through the local proxy stop being inspected until it is trusted again. |
| 16 | `src/TrayApp.tsx:1032` | no | The certificate stays on this machine, and you can remove it from Settings at any time. |
| 13 | `src/NewUiApp.tsx:3151` | no | Gate inspects your AI traffic locally, which needs a certificate your system trusts. |
| 13 | `src/TrayApp.tsx:1027` | no | Gate inspects your AI traffic locally, which needs a certificate your system trusts. |

## Other lib notices (12)

| Words | Location | Figma | Text |
|---:|---|---|---|
| 22 | `src/lib/dashboard.ts:40` | no | Gate Connect is pointed at a gateway with no dashboard to open. Switch to a Gate server in Settings to reach it. |
| 22 | `src/lib/reopen.ts:80` | no | Open again, on the new route. Gate routes this one through the system proxy, so there is no per-tool check to run. |
| 16 | `src/lib/activityGaps.ts:194` | no | The credential in use has no user attached, so Gate cannot tell whose activity this is. |
| 15 | `src/lib/platform.ts:190` | no | macOS will ask for your login password. The prompt is named “security”, not Gate Connect. |
| 15 | `src/lib/reopen.ts:84` | no (paraphrases banner `1408:22471`) | Gate could not close it, so it is still using the settings it started with. |
| 15 | `src/lib/reopen.ts:88` | no | Gate could not confirm where its traffic goes, so it is not claiming either answer. |
| 14 | `src/lib/notices.ts:82` | no | Gate cannot read this app's traffic until its certificate is trusted on this machine. |
| 14 | `src/lib/platform.ts:155` | no | That includes the same site in a browser that follows your desktop proxy settings. |
| 13 | `src/lib/activityGaps.ts:184` | no | Your role in this organization cannot see this. An owner or admin can. |
| 13 | `src/lib/notices.ts:74` | **deviation** `154:71081` (documented "didn't start" rewrite) | Routing didn’t start when Gate Connect opened, so this app’s traffic isn’t protected. |
| 13 | `src/lib/reopen.ts:74` | no | Asking this tool to close so it can pick up its new configuration. |
| 12 | `src/lib/reopen.ts:86` | no | Gate could not write this tool's configuration, so nothing changed for it. |

## Image alt text (3)

Not notices, but long. Listed for completeness.

| Words | Location | Figma | Text |
|---:|---|---|---|
| 21 | `src/screens/Onboarding.tsx:79` | no | Claude, OpenAI and Gemini connected by lines that meet at the Gate mark, which passes a response on to the app |
| 17 | `src/screens/Onboarding.tsx:98` | no | The Gate Connect popover: a routing summary over the list of apps, with an Expand app button |
| 17 | `src/screens/Onboarding.tsx:125` | no | The Overview dashboard: messages, blocked and flagged counts, tokens saved, and a bar chart of message volume |

## Shortened (2026-10-09)

89 edits covering the 87 undrawn rows (two strings are duplicated in `TrayApp.tsx`; `SHELL_CHANNEL_COVERAGE`'s tail is a separate edit). 1846 words down to 1141. The CLI's copy of `BROWSER_REMOVED_RESTART` (`crates/cli/src/main.rs`) moved with it.

Where a unit test protected a fact rather than a wording, the fact was kept in short form rather than the test loosened: the Windows *security warning* name and what stays broken, "Command-line tools are unaffected", "at least one" store, "turn routing off and on again", "certificate settings", "Gate will not pick a replacement" (a ticket's hard rule), "verified to reject", "desktop proxy settings", and the data-sharing disclosure's "while sharing is on" / "the first time you say no" / "stored with each request your account sends" (which `analytics.inventory.test.ts` pins to `docs/analytics-events.md`).

| Location | Words | Before | After |
|---|---|---|---|
| `src/screens/Onboarding.tsx:89` | 47 → 23 | For Claude Code and Codex, Gate Connect points the app’s own config at your gateway and restores it when you disconnect. For apps like Claude Desktop or ChatGPT, it routes the provider’s domain through a local proxy. Your Gate key stays in ${secretStoreName(platform)}, not a plain file. | Gate edits each app’s config, or routes it through a local proxy, and undoes it when you disconnect. Your key stays in ${secretStoreName(platform)}. |
| `src/screens/Onboarding.tsx:139` | 16 → 10 | That’s all there is to it. Sign in and your first app is one toggle away. | That’s it. Sign in and turn on your first app. |
| `src/components/gc/setup.tsx:454` | 13 → 8 | Sign in again whenever you want to start routing your apps through Gate. | Sign in again to start routing through Gate. |
| `src/components/gc/dialogs.tsx:1574` | 65 → 30 | OpenCode's own settings cover the providers you had set up when you turned it on. Anything you add later reaches Gate through your machine's proxy variables instead, and those apply to every command line tool that reads them, git, curl and npm included. One of them also tells Node to trust Gate's certificate, so every Node program you start afterwards accepts the traffic Gate inspects. | Providers you add to OpenCode later route through your proxy variables. These apply to every command-line tool, git, curl and npm included, and make every Node program trust Gate's certificate. |
| `src/components/gc/dialogs.tsx:1597` | 21 → 11 | Terminals and tools that are already open keep the environment they started with. Reopen them if you want their traffic covered. | Terminals and tools already open aren't covered until you reopen them. |
| `src/components/gc/dialogs.tsx:1828` | 45 → 32 | A device id generated on this machine. Once you sign in, and while sharing is on, your organization id too, and your account id if you signed in with Constellation, so setup can be measured from download to first request. Never your name or email. | A device id made on this machine. Once you sign in, and while sharing is on, your organization id too, and your account id with Constellation sign-in. Never your name or email. |
| `src/components/gc/dialogs.tsx:1838` | 31 → 22 | The first time you say no, or turn sharing off later, one final note says so, with your organization id, and tied to your account if you signed in with Constellation. | The first time you say no, or turn sharing off later, one final note says so, tied to your organization and account. |
| `src/components/gc/dialogs.tsx:1843` | 28 → 22 | Which action happened, from a fixed list - routing turned on or off, an app connected, a setup step completed or failed, an update installed. Never free text. | Which action happened, from a fixed list: routing on or off, an app connected, a setup step, an update. Never free text. |
| `src/components/gc/dialogs.tsx:1847` | 19 → 13 | A short label for each action: which app or provider it concerned, and whether it was on or off. | A short label per action: which app or provider, and on or off. |
| `src/components/gc/dialogs.tsx:1850` | 16 → 12 | A classified title when something fails, e.g. "keychain denied". The underlying message stays on this machine. | A short error title, like "keychain denied". The full message stays here. |
| `src/components/gc/dialogs.tsx:1858` | 32 → 22 | When something fails, the state Gate was in: your operating system version, which tools are installed, whether routing was on, and whether the event stream was connected. Not what you were doing. | On a failure: your OS version, installed tools, and whether routing and the event stream were on. Not what you were doing. |
| `src/components/gc/dialogs.tsx:1881` | 23 → 21 | The same device id, stored with each request your account sends, so your activity view can group requests by machine. It authorizes nothing. | The same device id, stored with each request your account sends, to group your activity by machine. It grants no access. |
| `src/components/gc/dialogs.tsx:1885` | 29 → 19 | Which app made the request, when Gate can tell from the request itself - Claude Code, Codex, and so on. Unrecognised apps are sent unlabelled rather than guessed at. | Which app sent the request, when Gate can tell: Claude Code, Codex and so on. Unknown apps go unlabelled. |
| `src/components/gc/dialogs.tsx:1913` | 18 → 13 | Your email, organization name and organization id, so a support thread can find the account it is about. | Your email and organization name and id, so support can find your account. |
| `src/components/gc/dialogs.tsx:1917` | 16 → 14 | Where Gate keeps its files, and the gateway, proxy and relay addresses this machine is using. | Where Gate keeps its files, and the gateway, proxy and relay addresses in use. |
| `src/components/gc/dialogs.tsx:1921` | 16 → 12 | Which tools are installed, their routing status, and which agents were running when you sent it. | Installed tools, their routing status, and agents running when you sent it. |
| `src/components/gc/dialogs.tsx:1768` | 20 → 14 | Automatic collection never includes your name, email or keys. A report you send yourself carries more, and is listed last. | Automatic collection never includes your name, email or keys. Reports you send carry more. |
| `src/components/gc/dialogs.tsx:2140` | 13 → 7 | at Gate, which has no session behind {plural ? "them" : "it"} now. | at Gate, which is now signed out. |
| `src/components/gc/dialogs.tsx:316` | 23 → 12 | Constellation sign-in keeps your session in ${secretStore} and refreshes it on its own, so there is nothing to rotate when a key expires. | Your session lives in ${secretStore} and renews itself. No key to rotate. |
| `src/components/gc/dialogs.tsx:1270` | 22 → 14 | Gate needs at least one model to serve this app. Choose one, or cancel and switch the app back to App default. | Choose at least one model, or cancel and return the app to App default. |
| `src/components/gc/dialogs.tsx:1384` | 22 → 13 | Gate sets {app.name}'s model to these in its own config. Return to App default at any time to restore your previous model. | Gate writes these into {app.name}'s config. Return to App default anytime to undo. |
| `src/components/gc/dialogs.tsx:345` | 20 → 9 | Your gateway and your routing stay exactly as they are. You can switch either way later, under Connection in Settings. | Nothing changes now. Switch anytime under Connection in Settings. |
| `src/components/gc/dialogs.tsx:1047` | 18 → 10 | This gateway offers no models of its own, so apps keep using the model they are configured with. | This gateway has no models, so apps keep their own. |
| `src/components/gc/dialogs.tsx:1274` | 18 → 10 | Requests for the models enabled here consume Gate credits. Gate never uses a model you have not enabled. | Enabled models use Gate credits. Gate never uses any other. |
| `src/components/gc/dialogs.tsx:270` | 16 → 10 | Your stored key is forgotten, managed tools disconnect, and Gate Connect relaunches against the new server. | This forgets your key, disconnects tools and restarts Gate Connect. |
| `src/components/gc/dialogs.tsx:1036` | 16 → 7 | Nothing has changed: this app keeps the model it is using. Close this and try again. | Nothing changed. Close this and try again. |
| `src/components/gc/dialogs.tsx:2042` | 16 → 10 | The report is on its way to Constellation Gate. Routing and event delivery were not interrupted. | The report is on its way. Routing was not interrupted. |
| `src/components/gc/dialogs.tsx:2001` | 13 → 7 | Paste this reference into your support request so we can find the report. | Paste this reference into your support request. |
| `src/components/gc/dialogs.tsx:2002` | 13 → 8 | One report, sent once. This does not change your Share diagnostic data setting. | Sends one report. Your sharing setting doesn’t change. |
| `src/lib/groups.ts:226` | 57 → 33 | Gate added its certificate to your ${store}, but at least one of the separate stores ${family} keep would not take it. The diagnostics report names which one and why; a Firefox profile with a Primary Password needs the certificate imported in Firefox’s own settings. Once it is fixed, turn routing off and on again to retry. ${BROWSER_RESTART} | The certificate was refused by at least one browser store; the diagnostics report says which. Firefox with a Primary Password needs it imported by hand. Then turn routing off and on again. ${BROWSER_RESTART} |
| `src/lib/groups.ts:217` | 38 → 24 | Gate added its certificate to your ${store}, but ${family} each keep a separate one, and Gate needs certutil to write it. Install it (Debian/Ubuntu: libnss3-tools, Fedora/RHEL: nss-tools), then turn routing off and on again. Command-line tools are unaffected. | ${family} need certutil to get the certificate. Install libnss3-tools (Debian/Ubuntu) or nss-tools (Fedora/RHEL), then turn routing off and on again. Command-line tools are unaffected. |
| `src/lib/groups.ts:236` | 34 → 20 | Gate added its certificate to your ${store}, but ${family} each keep a separate one and Gate has not written those, so turn routing off and on again to add it. Command-line tools are unaffected. | ${family} don’t have the certificate yet, so turn routing off and on again to add it. Command-line tools are unaffected. |
| `src/lib/groups.ts:245` | 20 → 9 | Gate has added its certificate to your ${store} and your browsers. A browser reads its certificates when it starts. ${BROWSER_RESTART} | Certificate added to your ${store} and your browsers. ${BROWSER_RESTART} |
| `src/lib/groups.ts:246` | 17 → 6 | Gate has added its certificate to your ${store}. A browser reads its certificates when it starts. ${BROWSER_RESTART} | Certificate added to your ${store}. ${BROWSER_RESTART} |
| `src/lib/groups.ts:286` | 34 → 16 | Gate removed its certificate from your certificate store, but needs certutil to remove it from the browsers’ own stores and it is not installed. Remove the Gate Connect certificate in each browser’s certificate settings. | Removing it from browsers needs certutil. Remove the Gate Connect certificate in each browser’s certificate settings. |
| `src/lib/groups.ts:292` | 31 → 15 | Gate removed its certificate from your certificate store, but at least one browser’s own store would not let go of it. Remove the Gate Connect certificate in that browser’s certificate settings. | A browser kept its copy. Remove the Gate Connect certificate in that browser’s certificate settings. |
| `src/lib/groups.ts:261` | 12 → 8 | Quit and reopen any open browser so it stops trusting the certificate. | Quit and reopen any open browser to finish. |
| `src/lib/groups.ts:623` | 19 → 12 | Routes every program you start from now on, not only AI tools, and tells Node to trust Gate's certificate. | Routes all programs you start next and makes Node trust Gate's certificate. |
| `src/lib/groups.ts:726` | 15 → 6 | Gate inspects traffic to the AI providers it knows and passes everything else through untouched. | Only AI provider traffic is inspected. |
| `src/lib/groups.ts:91` | 12 → 9 | Chats in the Claude desktop app, and on claude.ai in a browser | Chats in the Claude desktop app and on claude.ai |
| `src/lib/groups.ts:97` | 12 → 9 | Chats in the ChatGPT desktop app, and on chatgpt.com in a browser | Chats in the ChatGPT desktop app and on chatgpt.com |
| `src/lib/modelAttention.ts:128` | 42 → 16 | The last few requests from this app have failed while it has been on Gate models. The model may not work with this app. Switch back to App default to use the app's own model again, or choose a different Gate model. | Recent requests on Gate models failed. Try another Gate model, or switch back to App default. |
| `src/lib/modelAttention.ts:145` | 29 → 21 | ${gone[0]} is no longer available from Gate. Requests will fail until you choose another model or return to App default - Gate will not pick a replacement for you. | ${gone[0]} left Gate, so requests will fail. Choose another model or return to App default. Gate will not pick a replacement. |
| `src/lib/modelAttention.ts:146` | 34 → 23 | None of the ${gone.length} models chosen here are available from Gate any more. Requests will fail until you choose another or return to App default - Gate will not pick a replacement for you. | All ${gone.length} chosen models left Gate, so requests will fail. Choose another or return to App default. Gate will not pick a replacement. |
| `src/lib/modelAttention.ts:157` | 28 → 15 | Pay-as-you-go is off for this organization, so Gate cannot serve a model for it. Requests will fail until it is enabled or this app returns to App default. | Pay-as-you-go is off, so Gate models will fail. Enable it or return to App default. |
| `src/lib/modelAttention.ts:165` | 29 → 16 | There are no Gate credits left, so requests using a Gate model will fail. Add credits, or return this app to App default to use its own model again. | No Gate credits left, so Gate models will fail. Add credits or return to App default. |
| `src/lib/toolModels.ts:196` | 17 → 14 | You switched ${appName} to ${toModel ?? "another model"} in ${appName}, so it is back on App default. | You picked ${toModel ?? "another model"} in ${appName}, so it’s back on App default. |
| `src/lib/modelCompatibility.ts:294` | 16 → 13 | Gate does not list tool support for this model, and ${appName} sends tools with every request. | Gate lists no tool support for this model, and ${appName} always sends tools. |
| `src/lib/modelCompatibility.ts:296` | 15 → 11 | These models were tested and verified to reject the form ${appName} sends its tools in. | These models were tested and verified to reject ${appName}’s tool format. |
| `src/lib/errors.ts:290` | 22 → 12 | This tool sends all of its traffic through Gate’s local proxy, so routing has to be on before it can be connected. | This tool routes through Gate’s local proxy, so turn routing on first. |
| `src/lib/errors.ts:305` | 21 → 17 | Click Trust again and choose Yes in the Windows security warning. Apps with no gateway setting can’t route until it’s trusted. | Click Trust again and choose Yes in the Windows security warning. Until then, some apps can’t route. |
| `src/lib/errors.ts:546` | 21 → 15 | Hermes already has its own proxy settings in ~/.hermes/.env, and Gate left them alone. Remove them to route Hermes through Gate. | Hermes has its own proxy settings in ~/.hermes/.env. Remove them to route it through Gate. |
| `src/lib/errors.ts:419` | 19 → 14 | Another copy of Gate Connect, or a gate-connect proxy relay, is using it. Quit that one, then try again. | Another Gate Connect or gate-connect relay is using it. Quit it and try again. |
| `src/lib/errors.ts:533` | 19 → 14 | OpenCode isn’t signed in to a provider Gate can route. Run opencode auth login, then turn OpenCode on again. | OpenCode has no provider Gate can route. Run opencode auth login, then try again. |
| `src/lib/errors.ts:404` | 18 → 13 | Gate stopped waiting after five minutes. Try again, and complete the sign-in in the browser window that opens. | Gate stopped waiting after five minutes. Try again and finish in the browser. |
| `src/lib/errors.ts:434` | 16 → 9 | Check that you’re online and that the gateway URL in Settings is right, then try again. | Check your connection and the gateway URL in Settings. |
| `src/lib/errors.ts:276` | 15 → 11 | This tool or action isn’t supported here yet. The details below help when reporting it. | Not supported here yet. The details below help when reporting it. |
| `src/lib/errors.ts:386` | 14 → 10 | Allow Gate Connect to use ${store} in your OS privacy settings, then try again. | Allow Gate Connect to use ${store} in your privacy settings. |
| `src/lib/errors.ts:454` | 13 → 11 | Open Settings and reconnect: sign in again, or replace your Gate API key. | Reconnect in Settings: sign in again or replace your API key. |
| `src/lib/errors.ts:452` | 12 → 8 | Check that the key is for this gateway, then paste it again. | Make sure the key is for this gateway. |
| `src/lib/errors.ts:511` | 13 → 9 | Try again. If it keeps failing, the details below help when reporting it. | Try again, or report it with the details below. |
| `src/lib/errors.ts:541` | 13 → 10 | Codex isn’t signed in yet. Run codex login, then turn it on again. | Codex isn’t signed in. Run codex login, then try again. |
| `src/lib/errors.ts:324` | 12 → 8 | Try again and approve the sign-in in the browser window that opens. | Try again and approve it in the browser. |
| `src/lib/errors.ts:550` | 13 → 8 | ${m[1]} isn’t installed on this machine. Install it, then turn it on again. | ${m[1]} isn’t installed. Install it, then try again. |
| `src/components/gc/Tray.tsx:511` | 22 → 12 | Gate Connect could not reach your stored credential, so it cannot tell what is routed. Open the app window to try again. | Couldn't reach your stored credential. Open the app window to try again. |
| `src/components/gc/Tray.tsx:532` | 21 → 11 | Gate Connect needs a Gate account or API key before it can route your tools. Sign in from the app window. | Sign in from the app window to start routing your tools. |
| `src/components/gc/Sidebar.tsx:513` | 18 → 9 | Gate couldn’t read this device’s app list, so it doesn’t know what is installed. Nothing has been changed. | Couldn’t read this device’s app list. Nothing was changed. |
| `src/components/gc/Sidebar.tsx:514` | 19 → 9 | Gate looked for supported AI apps and found none installed. Install one and refresh, and it will appear here. | No supported AI apps found. Install one and refresh. |
| `src/components/gc/AppPane.tsx:533` | 18 → 9 | Gate could not read this app's model setting, so it is not shown. The setting itself is unchanged. | Couldn't read this app's model setting. Nothing was changed. |
| `src/components/gc/AppPane.tsx:641` | 13 → 12 | No Gate model chosen yet. Choose one to write it into {appName}'s config. | No Gate model chosen. Choose one to add it to {appName}'s config. |
| `src/components/gc/SettingsPane.tsx:553` | 14 → 9 | Send one report to Constellation Gate and get a reference for your support request. | Send one report and get a reference for support. |
| `src/NewUiApp.tsx:3222` | 35 → 14 | When you trust a new certificate, quit and reopen any AI tools that are running. They read the certificate when they start, so one that is already open will fail to connect until you do. | After trusting a new certificate, quit and reopen any running AI tools, or they won't connect. |
| `src/NewUiApp.tsx:3204` | 24 → 18 | Routing turns off while the certificate is gone, and your tools keep their configuration. Your operating system may ask for permission to remove it. | Routing turns off while the certificate is gone. Tool configs are kept. Your system may ask to confirm. |
| `src/NewUiApp.tsx:3163` | 16 → 10 | The certificate stays on this machine, and you can remove it from Settings at any time. | It stays on this machine. Remove it in Settings anytime. |
| `src/TrayApp.tsx:1033` | 16 → 10 | The certificate stays on this machine, and you can remove it from Settings at any time. | It stays on this machine. Remove it in Settings anytime. |
| `src/NewUiApp.tsx:3182` | 16 → 10 | Sites and apps routed through the local proxy stop being inspected until it is trusted again. | Proxied sites and apps aren’t inspected until it’s trusted again. |
| `src/NewUiApp.tsx:3151` | 13 → 12 | Gate inspects your AI traffic locally, which needs a certificate your system trusts. | Gate needs a trusted certificate to inspect AI traffic on this machine. |
| `src/TrayApp.tsx:1027` | 13 → 12 | Gate inspects your AI traffic locally, which needs a certificate your system trusts. | Gate needs a trusted certificate to inspect AI traffic on this machine. |
| `src/lib/dashboard.ts:40` | 22 → 10 | Gate Connect is pointed at a gateway with no dashboard to open. Switch to a Gate server in Settings to reach it. | Switch to a Gate server in Settings to use it. |
| `src/lib/reopen.ts:80` | 22 → 13 | Open again, on the new route. Gate routes this one through the system proxy, so there is no per-tool check to run. | Open again. It routes through the system proxy, so there’s nothing to check. |
| `src/lib/reopen.ts:84` | 15 → 10 | Gate could not close it, so it is still using the settings it started with. | Gate couldn’t close it, so it keeps its old settings. |
| `src/lib/reopen.ts:86` | 12 → 6 | Gate could not write this tool's configuration, so nothing changed for it. | Couldn’t write its config. Nothing changed. |
| `src/lib/reopen.ts:88` | 15 → 6 | Gate could not confirm where its traffic goes, so it is not claiming either answer. | Couldn’t confirm where its traffic goes. |
| `src/lib/reopen.ts:74` | 13 → 9 | Asking this tool to close so it can pick up its new configuration. | Closing it so it picks up the new config. |
| `src/lib/activityGaps.ts:194` | 16 → 10 | The credential in use has no user attached, so Gate cannot tell whose activity this is. | This credential has no user, so activity can’t be attributed. |
| `src/lib/activityGaps.ts:184` | 13 → 8 | Your role in this organization cannot see this. An owner or admin can. | Only an owner or admin can see this. |
| `src/lib/platform.ts:190` | 15 → 11 | macOS will ask for your login password. The prompt is named “security”, not Gate Connect. | macOS will ask for your password, in a prompt named “security”. |
| `src/lib/platform.ts:155` | 14 → 11 | That includes the same site in a browser that follows your desktop proxy settings. | That includes the site in browsers using your desktop proxy settings. |
| `src/lib/notices.ts:82` | 14 → 10 | Gate cannot read this app's traffic until its certificate is trusted on this machine. | Gate can’t read its traffic until the certificate is trusted. |
