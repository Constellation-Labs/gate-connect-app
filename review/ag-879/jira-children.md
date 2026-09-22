# AG-879 children (pulled 2026-09-17)

## AG-880 - Completion modal still asks the user to open Claude Code after it has verified the restart
Status: In review | Assignee: Gabriel Claramunt | Priority: Medium

## Observed

After changing the route, the completion modal shows "Change is ready / Claude Code closed successfully" with the body "The new Gate route is active. Open Claude Code whenever you are ready to continue." This appears after Connect has already detected that Claude Code was reopened and confirmed the config is valid, so the modal asks for a step the user has finished.

## Expected

When Connect has confirmed the reopened session and a valid config, the final state should say the route is active and in use, and the only action offered should be to dismiss. The "open Claude Code whenever you are ready" instruction should only appear while Connect is still waiting for the app to come back.

## Evidence

Gate Connect v1, macOS, 2026-09-16. Screenshot attached.

## Who it affects

Every user who completes a route change, which is the main flow of the app.

### Comment by Gabriel Claramunt (2026-09-16)
PR: https://github.com/Constellation-Labs/gate-connect-app/pull/286 (base feat/new-app-ui)

Validated from the code rather than from a running app. ChangeReadyDialog renders only behind allVerified, and bucketOf returns "verified" only for the stages routing and not_routed, both of which are readings taken after the tool came back up. So there is no reachable state where this dialog draws and the user still has an app to open. The drawn instruction was unreachable-as-true, not merely mistimed.

Root cause: docs/review-flow-overview.md:111 records the dialog firing on stage.kind === "done" straight out of closeApps, the moment right after the SIGTERM, where the copy was correct. AG-566 moved the tail behind allVerified and the copy did not follow.

Fix: subtitle now reads "<app> is back on the new route", body reads "The new route is active and in use.", and the instruction is gone. Done was already the only action. Five unit tests added; the dialog had none.

One thing for design: this deviates from the drawn copy at 134:61659, changed on this ticket's authority rather than by eye. CLAUDE.md keeps a list of decided copy exceptions and asks that a new one be raised rather than decided, so it is raised here and not added to that list. Note that the frame already contradicts itself between its subtitle and body per docs/review-flow-overview.md:124.

Verification: tsc --noEmit clean, vitest 60 files / 1154 tests pass.

## AG-881 - Remove the dollar estimate from the Tokens Saved tile
Status: In Progress | Assignee: matheus.reis | Priority: Medium

## Observed

The Overview TOKENS SAVED tile reads "4%" with "+$11.27" next to it. Connect should not be putting a dollar figure on savings, and the figure shown is wrong.

## Expected

The tile reports the savings rate only, as saved tokens over total tokens for the selected window. No currency estimate appears anywhere on Overview.

## Evidence

Gate Connect v1, macOS, 2026-09-16, Alex's Org, last-24-hours window, 1,076 messages. Screenshot attached.

## Who it affects

Every user on the Overview screen, which is the app's landing view.

## AG-882 - Tokens Saved percentage disagrees with the Gate dashboard for the same window
Status: To Do | Assignee: Gabriel Claramunt | Priority: Medium

## Observed

Connect's Overview shows TOKENS SAVED of 4% for the last 24 hours while the Gate web dashboard shows 5% for the same account and the same window. The tile also shows "+$11.27" against an account with zero spend in that period.

## Expected

Connect and the dashboard report the same savings rate for the same account and window, computed from the same source. Any money figure shown reflects actual recorded spend, which is $0.00 here.

## Evidence

Gate Connect v1, macOS, 2026-09-16, Alex's Org, last-24-hours window, 1,076 messages. Control is the Gate web dashboard read at the same time. Screenshot attached.

### Comment by matheus.reis (2026-09-16)
Investigated against staging (org `cccbb95b`, 2026-09-17). Two findings: the reported discrepancy is not a bug, but there is a real one underneath it.

h2. The 5% vs 4% is two different measurements

They are not comparable, so this pair is not evidence of a defect:

|| || Window || Source ||  
| Dashboard home tile | _7 days_ | {{economy.gateAttributedPct}} |  
| Connect Tokens saved | _24 hours_ | {{counters.tokensSaved.fraction}} |

{{apps/dashboard-web/src/pages/Dashboard.tsx}} states the window outright: "on the same \[last week, this week\] _7-day window_ as the other cards (was a 30-day aggregate, which never matched the weekly rail)". The "-62.0% vs last week" line on that tile is that weekly rail.

Connect's number is also arithmetically correct for its own window: the gateway returned {{fraction: 0.03985}}, which is 4% by both round and ceil.

The epic's key decision is "the same account _and window_" - and the dashboard has no 24-hour Tokens saved tile to match against.

h2. The real defect: the two numbers measure different things

Even on the same window they would disagree, because they count different savings:

|| || Counts ||  
| Connect ({{ActivityRepository.tokensSaved}}) | _compression only_ - {{tokens_compressed_saved}} over that plus billed input |  
| Dashboard ({{gateAttributedPct}}) | compression _+ gateway cache hits + the provider-cache credit Gate's marker injection earned_ |

So Connect will read _lower_ than the dashboard for the same org over the same window, by construction. The gateway's own comment calls compression-only "the narrowest of the available definitions".

_There is a stale claim in the code that hides this._ {{apps/gateway-proxy/src/activity/activity.types.ts}} says of the Connect figure:

{quote}The row set and both halves of the ratio are the dashboard's {{blendedReductionPct}}, so the two surfaces print the same percentage for one org over one window.{quote}

That is no longer true. Both the home tile and the Token Savings page moved to {{gateAttributedPct}} (Marcus, 2026-07-29), which is a superset. This is the same shape as AG-422 and AG-724: one number, two definitions, two surfaces that cannot reconcile.

h2. Why we did not just widen the query

The comment in {{activity.repository.ts}} says "widening the definition is a change to this one query", and that is true for the _token_ columns ({{tokens_image_saved}}, {{tokens_tool_deferred_saved}}, {{cached_input_tokens}}) - but widening to those still would not match the dashboard, because {{gateAttributedPct}} is _rate-valued_, not a token ratio:

{code}  
economyBaseline     = forwardedPrompt + compSavedAll + gateHitPrompt  
economyActual       = per-row SQL sum priced at each request's own model  
                      catalog cache rates (fallback 1.25xwrite + 0.1xread)  
gateAttributedUnits = compressionSavedUnits + gateHitUnits  
                      + providerCacheDeltaGateEarned  (needs cache_control_injected_at)  
{code}

It lives in {{dashboard-api}}'s caching repository, needs a per-model rate catalog join, and carries a decomposition identity the comments say must hold _by construction_ - with a documented case of a {{GREATEST(..., 0)}} floor silently breaking it. Reimplementing that inside {{gateway-proxy}} would create a second implementation of a number two surfaces must agree on, which is precisely the AG-422 failure mode from the other direction.

Getting it wrong produces a plausible _wrong_ number on the screen people open to check exactly this, so we did not attempt the port blind.

h2. Suggested split

# _This ticket_: not a defect as reported. Either close it, or re-scope it to the definition mismatch below.

# _New gateway ticket_: {{/v1/me/activity.tokensSaved}} is compression-only while every dashboard surface plots Gate-attributed savings. Owner of the economy code to decide between sharing the computation and exposing it. The stale {{activity.types.ts}} comment goes with it.

No Connect-side change is warranted: it renders what it is given, correctly.

## AG-883 - Clicking the Tokens Saved tile adds empty space and jumps the page to the bottom
Status: In Progress | Assignee: matheus.reis | Priority: Medium

## Observed

Clicking the TOKENS SAVED tile on Overview inserts roughly a full screen of blank space below the Policies card and scrolls the view to the bottom of that empty region. Nothing appears to open, so the screen looks broken.

## Expected

Clicking the tile either opens its detail view in place or does nothing. Page height does not change and the scroll position stays where the user left it.

## Evidence

Gate Connect v1, macOS, 2026-09-16, Alex's Org. Screenshot attached, taken immediately after the click.

## AG-884 - Redacted PII events are missing from Blocked/Flagged and from Security events
Status: To Do | Assignee: Gabriel Claramunt | Priority: Medium

## Observed

Overview shows BLOCKED/FLAGGED as 0 for the last 24 hours on an account that has redacted PII events in that window, and the Security events table reads "No security events" with an OFFLINE badge in its corner. The Gate web dashboard shows those events for the same account and window. The PII / PHI policy is ON with action REDACT.

## Expected

BLOCKED/FLAGGED counts every enforcement event in the window, including redactions, and the Security events table lists those same events with time, category, tool and model. The BLOCKED/FLAGGED count in Connect must equal the equivalent count on the user's Gate dashboard for the same account and the same time period, and the Security events list must match that dashboard's events for that period. If the label is meant to exclude redactions, the tile says so and redactions get their own count.

## Evidence

Gate Connect v1, macOS, 2026-09-16, Alex's Org, last-24-hours window, 1,076 messages. Control is the Gate web dashboard read at the same time. Screenshot attached.

## AG-885 - OpenAI stays "Not started" after restart and neither Retry nor Resume clears it
Status: To Do | Assignee: matheus.reis | Priority: Medium

## Observed

The banner "Routing didn't finish coming back" reports "OpenAI is still waiting." The expanded row reads "OpenAI - Not started - Last verified: no reading yet - Check: Checked per tool, not per provider - Gate has no process to look for", with a Retry button. The ChatGPT app was opened after the restart, and pressing Retry or "Resume now" leaves the row in the same state. The banner does not clear and the app stays at "Partly routed - 2 of 3 Apps".

## Expected

Once the app for a waiting provider is running, Retry or "Resume now" finishes that provider's routing, the row moves to routed and verified, and the banner clears on its own. If the provider genuinely cannot be verified, the row says what is missing and what the user should do, in plain language.

## Evidence

Gate Connect v1, macOS, 2026-09-16, Alex's Org. ChatGPT/Codex desktop app running at the time of the screenshot. Screenshot attached.

## AG-886 - "What happened to routing" dialog is written in internal terms and gives the user nothing to act on
Status: In review | Assignee: Gabriel Claramunt | Priority: Medium

## Observed

Opening Review details shows a dialog reading "Turning routing back on, last updated 4m ago. It was trying to leave routing on for every tool it had recorded", then "0 of 1 stages completed, 1 still pending", then a card for OpenAI saying "The operation stopped before reaching this one. Nothing about it was changed" with the fields Stage "Not started", Last verified route "No reading yet", Last check "Checked per tool, not per provider", Process "Gate has no process to look for", Next action "Retry". A user reading this cannot tell what went wrong or what to do about it.

## Expected

The dialog says in plain English which app is not routed, why, and the one thing the user should do next, with the internal stage, process and check fields removed from the user-facing view. A user who has never seen the codebase can read it once and know whether to act or wait.

## Evidence

Gate Connect v1, macOS, 2026-09-16, Alex's Org, reached from the "Routing didn't finish coming back" banner. Screenshot attached.

### Comment by Gabriel Claramunt (2026-09-16)
PR: https://github.com/Constellation-Labs/gate-connect-app/pull/287 (base feat/new-app-ui)

Approach: the diagnostic readings are not the problem. AG-570 asked for them and handing this summary to someone else is still one of the things the review is for. What was wrong is that they came first. They now sit behind a per-row "Technical details" disclosure, so nothing is lost and nothing internal is the first thing a reader meets.

Before and after. Subtitle "It was trying to leave routing on for every tool it had recorded" becomes "Gate was turning routing on 4m ago and did not finish." The header "0 of 1 stages completed, 1 still pending" becomes "OpenAI is not routing through Gate yet." The row detail "The operation stopped before reaching this one" becomes "Gate did not reach this one, so nothing about it changed." The dead field "Next action: Retry" becomes the instruction "Choose Resume now on the routing notice to finish this." The pill "Not started" becomes "Unfinished".

Stage, Last verified route, Last check, Process and the failure category all move into the disclosure. The one header line kept is the engine-still-starting sentence, because it was already plain and it is what stops a reader hunting a problem that was a proxy coming up.

Note the instructions deliberately point at a surface that can act. The review is read-only per AG-570, so "press Retry" would have named a control this dialog does not have.

No Figma constraint: dialogs.tsx already recorded that the file draws no details view, AG-569 being open, so this is not a deviation from a drawn design.

Left undone on purpose: operationLine and stageCounts now have no callers. They are still exported and tested rather than deleted, since removing them is outside this ticket. Flagged in the PR for a follow-up decision.

Verification: tsc --noEmit clean, vitest 61 files / 1162 tests pass, 13 of them new.

## AG-887 - Type column is empty for every row in Recent activity
Status: In Progress | Assignee: matheus.reis | Priority: Medium

## Observed

On the Claude app page, the Recent activity table shows an em dash in the Type column for every row. The other columns are populated: Time, Security "ALLOW", Model "anthropic/claude-opus-5", and a View link. The same emptiness holds for all rows visible in the last 24 hours, across 904 messages.

## Expected

Every row shows the request type that produced it, drawn from the same record the row's model and security verdict come from. If a request genuinely has no type, the column shows why rather than a dash on every row.

## Evidence

Gate Connect v1, macOS, 2026-09-16, Alex's Org, Claude app page, rows timestamped Sep 16 13:12 to 13:13. Screenshot attached.

## AG-888 - Gate model picker allows multiple selections for apps that can only use one model
Status: In Progress | Assignee: matheus.reis | Priority: Medium

## Observed

On the Claude app page, choosing "Gate model" opens "Choose Gate models" with a checkbox per model and no limit on how many can be checked. Four are checked in the screenshot (aion-labs/aion-3-0, aion-labs/aion-3-0-mini, aion-labs/aion-rp-llama-3-1-8b, amazon/nova-2-lite) and "Apply selections" accepts them. Claude takes one model at a time, so any selection past the first cannot be honored. The ChatGPT / Codex app has the same picker with the same behavior.

## Expected

For an app that runs one model at a time, the picker offers exactly one choice: picking a model replaces the current one, and the dialog shows which single model the app will use. The multi-select picker stays only where the app can actually use more than one.

## Evidence

Gate Connect v1, macOS, 2026-09-16, Alex's Org, Claude app page, "Choose Gate models" dialog showing 345 of 345 models. Reproduced on the ChatGPT / Codex app page. Screenshot attached.

### Comment by matheus.reis (2026-09-17)
The premise here is out of date, so I have not implemented the Expected as written. Flagging rather than closing.

**A set of models is an allow-list, not an override queue.** `applyUserModelChoice` in `gateway-proxy` (`src/proxy/stages/resolve-upstream.ts`): a request for a model _in_ the set is served as the model the tool asked for and the body is left alone. Only a request for something _outside_ the set is rewritten, onto the first entry.

So the second and later selections are honoured, and they are honoured in the case that matters: three Codex sessions on three models, two of them enabled, keep their own models. That behaviour is AG-746, and it exists because the previous behaviour served the first enabled model all three times, which discarded the per-session choice silently.

Making the picker single-select would undo that, and also AG-589 plus design's 2026-09-04 call for multi-select.

**But the report is right that nothing said so.** A checkbox per model with no rule beside it makes "anything past the first cannot be honored" the reasonable reading. Two things were feeding it:

1. The picker never stated the rule.
2. Gate Connect's own header doc still said the gateway "rewrites the body's `model` to the first entry", full stop, which was true before AG-746 and is the first place anyone would check.

Both fixed on `feat/ag-879-connect-defects`. With more than one model chosen the picker now says the two facts the checkboxes cannot: the app keeps its own model whenever it asks for one of the set, and the first entry, in the user's own order, is what everything else becomes.

Worth a second opinion on one point: if a single-model app really does exist, the picker already supports `multiple={false}` and has no call site, because nothing in the backend says which tools are single-model - `model_ids` is a list for every tool. Naming those tools would be a separate ticket with a backend half.

## AG-889 - OpenAI API page tells the user its activity cannot be shown, in copy that does not explain anything
Status: To Do | Assignee: matheus.reis | Priority: Medium

## Observed

The OpenAI API entry has its own page in the sidebar, separate from the ChatGPT / Codex app. Its content is the line "This app: its requests aren't attributed to a single app yet, so its own activity can't be shown. The Overview still covers your whole organisation.", with two cards reading "Messages aren't attributed to this app" and "Recent activity isn't attributed to this app". The wording leaves the user unsure whether something is broken or whether the app is working as intended. The copy also uses the British spelling "organisation".

## Expected

OpenAI traffic is represented by one entry the user can act on, and no screen exists whose only message is that its own numbers are unavailable. Whatever page covers api.openai.com traffic says plainly that the traffic is routed and protected, and where its activity can be seen. Spelling is American throughout.

## Suggestion, not a specification

The OpenAI API entry looks like it belongs inside the ChatGPT / Codex app, with its host coverage moved into that switch's "What this switch covers". Confirm before building.

## Evidence

Gate Connect v1, macOS, 2026-09-16, Alex's Org, OpenAI API page with the switch On and Protected. Screenshot attached.

### Comment by matheus.reis (2026-09-16)
Parked pending a product decision. Investigated far enough to say what the options cost; not implemented.

h2. The suggested fix has a side effect worth knowing about

The ticket suggests folding the OpenAI API entry into the ChatGPT / Codex app. {{src/lib/groups.ts}} already argues against that, and the argument is specific:

{quote}Its own switch rather than part of ChatGPT / Codex, because nothing OpenAI ships rides it: Codex routes through the relay whatever this says, and the desktop app is on chatgpt.com. What depends on it is whatever else on the machine calls api.openai.com ... Folding it into the app switch would mean routing every script on the machine as a side effect of routing ChatGPT.{quote}

Three facts behind it:

* _Codex does not use this entry_ - it routes through the relay regardless of what this switch says.
* _The ChatGPT desktop app does not use it_ - that traffic is {{chatgpt.com}}.
* _What does use it_ is any other program on the machine calling {{api.openai.com}}.

So the fold-in makes turning on ChatGPT silently route every OpenAI-calling script on the machine. That is the same shape as the problem AG-893 just fixed, where one switch reached far past its name.

h2. The suggestion also depends on something that no longer exists

"with its host coverage moved into that switch's _What this switch covers_" - that element was removed in PR #273, because no frame draws it. There is no longer a place on the switch to put the disclosure the suggestion relies on.

h2. What the ticket is actually about

Its Expected does not ask for the entry to be moved. It asks that "no screen exists whose only message is that its own numbers are unavailable", and that whatever page covers {{api.openai.com}} "says plainly that the traffic is routed and protected, and where its activity can be seen".

That is a _page_ problem, not a composition one. The switch is meaningful; the page is the part that explains nothing.

h2. Option we did not take, and why it needs a decision

OpenAI API and OpenRouter are the same kind of thing: a host any app can be pointed at, whose traffic Gate cannot attribute per-entry because {{client_tool}} is derived from the caller's User-Agent. AG-897's Expected asks for exactly this framing - "OpenRouter is grouped with the provider endpoints rather than under Apps".

So one coherent change would close both:

# Group provider endpoints out of Apps, by the same rule that moved the shell-environment channel in AG-893.

# Give those pages honest copy - routed and protected, traffic not attributed to one app, counted in the Overview - instead of three {{n/a}} tiles and two "isn't attributed" cards.

We have not done this because it introduces a third category to the rail, and whether that is the right shape is a product call rather than an engineering one. It is also adjacent to the epic's own open question, which AG-879 assigns to the Gate Connect squad lead.

h2. What we need to unpark

One answer: _should the rail have a provider-endpoints group, distinct from Apps and Tools?_

* If _yes_ - we implement both steps above, AG-897 closes with it, and the fold-in is dropped.
* If _no_ - we still fix the page copy on its own (step 2), which is the ticket's literal Expected, and the entry stays where it is.
* If the _fold-in is still wanted_ despite the side effect above, we need a decision that routing ChatGPT may route unrelated scripts, and somewhere to disclose it now that "What this switch covers" is gone.

No code changes were made for this ticket. AG-897's band change is also held, since it is the same decision.

### Comment by matheus.reis (2026-09-16)
Parked on one product question. Investigated, not implemented.

_The suggested fix has a side effect._ {{src/lib/groups.ts}} already argues against folding this entry into ChatGPT / Codex: nothing OpenAI ships rides it (Codex routes through the relay regardless, the desktop app is on {{chatgpt.com}}), so what depends on it is any _other_ program calling {{api.openai.com}}. Folding it in would make turning on ChatGPT silently route every OpenAI-calling script on the machine.

The suggestion also relies on moving coverage into "What this switch covers", which PR #273 removed because no frame draws it.

_What the Expected actually asks for is narrower_: that no screen exists whose only message is that its own numbers are unavailable. That is a page problem, not a composition one - the switch is meaningful.

_This is the same decision as AG-897._ Both entries are hosts any app can be pointed at, and Gate cannot attribute their traffic per-entry because {{client_tool}} comes from the caller's User-Agent. AG-897's Expected asks for OpenRouter to be "grouped with the provider endpoints rather than under Apps" - the same grouping would cover this one.

_The question we cannot answer ourselves:_ should the rail carry a third category for provider endpoints, distinct from Apps and Tools?

* _Yes_ - group them out of Apps and give both pages honest copy (routed and protected, not attributed to one app, counted in the Overview). Closes this and AG-897.
* _No_ - fix the page copy alone, which is this ticket's literal Expected, and the entries stay put.
* _Fold in anyway_ - needs someone to accept that routing ChatGPT routes unrelated scripts, and a place to disclose it.

Adjacent to the epic's own open question, which AG-879 assigns to the squad lead. Fuller write-up in {{plans/ag-879-connect-defects.md}} on {{feat/ag-879-connect-defects}}. AG-897's band change is held with this.

### Comment by matheus.reis (2026-09-16)
Parked pending a product decision. Investigated far enough to say what the options cost; not implemented.

h2. The suggested fix has a side effect worth knowing about

The ticket suggests folding the OpenAI API entry into the ChatGPT / Codex app. {{src/lib/groups.ts}} already argues against that, and the argument is specific:

{quote}Its own switch rather than part of ChatGPT / Codex, because nothing OpenAI ships rides it: Codex routes through the relay whatever this says, and the desktop app is on chatgpt.com. What depends on it is whatever else on the machine calls api.openai.com ... Folding it into the app switch would mean routing every script on the machine as a side effect of routing ChatGPT.{quote}

Three facts behind it:

* _Codex does not use this entry_ - it routes through the relay regardless of what this switch says.
* _The ChatGPT desktop app does not use it_ - that traffic is {{chatgpt.com}}.
* _What does use it_ is any other program on the machine calling {{api.openai.com}}.

So the fold-in makes turning on ChatGPT silently route every OpenAI-calling script on the machine. That is the same shape as the problem AG-893 just fixed, where one switch reached far past its name.

h2. The suggestion also depends on something that no longer exists

"with its host coverage moved into that switch's _What this switch covers_" - that element was removed in PR #273, because no frame draws it. There is no longer a place on the switch to put the disclosure the suggestion relies on.

h2. What the ticket is actually about

Its Expected does not ask for the entry to be moved. It asks that "no screen exists whose only message is that its own numbers are unavailable", and that whatever page covers {{api.openai.com}} "says plainly that the traffic is routed and protected, and where its activity can be seen".

That is a _page_ problem, not a composition one. The switch is meaningful; the page is the part that explains nothing.

h2. Option we did not take, and why it needs a decision

OpenAI API and OpenRouter are the same kind of thing: a host any app can be pointed at, whose traffic Gate cannot attribute per-entry because {{client_tool}} is derived from the caller's User-Agent. AG-897's Expected asks for exactly this framing - "OpenRouter is grouped with the provider endpoints rather than under Apps".

So one coherent change would close both:

# Group provider endpoints out of Apps, by the same rule that moved the shell-environment channel in AG-893.

# Give those pages honest copy - routed and protected, traffic not attributed to one app, counted in the Overview - instead of three {{n/a}} tiles and two "isn't attributed" cards.

We have not done this because it introduces a third category to the rail, and whether that is the right shape is a product call rather than an engineering one. It is also adjacent to the epic's own open question, which AG-879 assigns to the Gate Connect squad lead.

h2. What we need to unpark

One answer: _should the rail have a provider-endpoints group, distinct from Apps and Tools?_

* If _yes_ - we implement both steps above, AG-897 closes with it, and the fold-in is dropped.
* If _no_ - we still fix the page copy on its own (step 2), which is the ticket's literal Expected, and the entry stays where it is.
* If the _fold-in is still wanted_ despite the side effect above, we need a decision that routing ChatGPT may route unrelated scripts, and somewhere to disclose it now that "What this switch covers" is gone.

No code changes were made for this ticket. AG-897's band change is also held, since it is the same decision.

## AG-890 - Menu bar popover shows routing state the desktop window does not, and its Review details link opens Settings
Status: To Do | Assignee: Gabriel Claramunt | Priority: Medium

## Observed

The menu bar popover shows "Partially routed - On - 4 of 5 tools routing" and a "Routing didn't finish - OpenAI is still waiting" card with Resume now and Review details. The desktop window is open at the same moment and shows none of it: no unfinished-routing banner, and its sidebar reads APPS 2 of 3 and TOOLS 2 of 5. The two surfaces also use different words for the same state, "Partially routed" against "partly routing your apps". Clicking "Review details" in the popover opens the Settings page, while the same link in the desktop window opens the "What happened to routing" dialog.

## Expected

Both surfaces read the same routing state from the same source and update together, so an alert raised in one appears in the other, with matching counts and wording. "Review details" opens the routing details for the unfinished run from either surface.

## Evidence

Gate Connect v1, macOS, 2026-09-16, Alex's Org. Popover and desktop window captured in one screenshot. Screenshot attached.

### Comment by Alex Brandes (2026-09-16)
Second instance, 2026-09-16 14:41. The menu bar popover reads "Partially routed · On · 6 of 7 tools routing" while the desktop window header reads "Routed · 7 of 7 Apps" at the same moment. The popover also shows ChatGPT / Codex as "Not protected · Reopen required" while the desktop sidebar shows the same app as Protected, so the disagreement covers per-app protection state as well as the summary count. The two surfaces also count different nouns for the same total: "tools" in the popover, "Apps" in the header. Screenshot attached.


## AG-891 - Settings shows Gate plan as "Unavailable" for an account on Pro
Status: In Progress | Assignee: matheus.reis | Priority: Medium

## Observed

Settings then Account shows the login ID and Gate plan "Unavailable". The account is on the Pro plan, and the Claude app page in the same session shows "Paid plan - Gate credits: $56.66 available", so the plan is known elsewhere in the app.

## Expected

Gate plan shows the account's actual plan, Pro in this case, read from the same source the app already uses for plan and credits. If the plan cannot be fetched, the row says so and offers a retry rather than reading as though no plan exists.

## Evidence

Gate Connect v1, macOS, 2026-09-16, Alex's Org, gateway https://gateway-staging.constellationgate.ai, signed in with a Gate account. Screenshot attached.

## AG-892 - Hermes desktop app traffic is not routed or counted while Connect reports Hermes as Protected
Status: To Do | Assignee: matheus.reis | Priority: Medium

## Observed

The Hermes entry in Connect is on and labeled Protected, described as "Hermes in your terminal." Its page shows MESSAGES 0, BLOCKED/FLAGGED 0, TOKENS SAVED n/a, "No messages sent in the last 24hrs", "No recent messages", and "Tokens saved: Nothing is set up for this yet, so there is nothing to report." At the same moment the Hermes desktop app is running with an active session, sending and receiving messages, and its own status bar reads "Gateway ready". None of that traffic appears in Connect.

## Expected

The Hermes switch covers the Hermes desktop app as well as the terminal, its traffic is routed and counted, and Messages and Recent activity show those sessions. If the switch genuinely covers the terminal only, the entry says so and does not read as Protected for the desktop app.

## Evidence

Gate Connect v1, macOS, 2026-09-16 13:40, Alex's Org. Hermes desktop app in an active session at the time of capture. Control: Claude in the same session shows over 900 messages, so counting works for other apps. Screenshot attached.

### Comment by matheus.reis (2026-09-16)
Traced what Gate actually supports for Hermes before proposing a fix, because the question underneath this ticket is whether we support Hermes Desktop at all. We do not, and the entry does not say so.

h2. What the Hermes switch covers

_The CLI only, by construction._ The integration's entire payload is four variables in {{\~/.hermes/.env}} ({{crates/core/src/integrations/hermes.rs}}):

{code}  
HTTPS_PROXY=http://127.0.0.1:<engine-port>  
HTTP_PROXY=http://127.0.0.1:<engine-port>  
NO_PROXY=localhost,127.0.0.1,::1  
HERMES_CA_BUNDLE=<app-support>/proxy/ca-bundle.pem  
{code}

That file is read by {{hermes_cli/env_loader.py}} _at CLI startup_, before any client is constructed. The desktop app is a separate GUI process that never loads it, so nothing the switch writes reaches it.

There is also _no Hermes proxy domain_ - no host-level interception - so Gate has no second route to that traffic either. The section's own description already says "Hermes in your terminal"; the entry is named "Hermes" and reads _Protected_, which is what makes it read as product-wide coverage.

h2. Why no traffic was counted

Two independent gates, and the traffic failed the first:

# _Routing_: the desktop app's requests never reach Gate, per the above.

# _Attribution_: {{classify_client}} matches a User-Agent containing {{hermes}} to {{Client::Hermes}} ({{proxy/mod.rs}}). So if its traffic did reach Gate and its UA carries that string, it _would_ be counted under Hermes.

So this is a routing gap, not a counting gap. Claude counting over 900 messages in the same session is consistent with that.

h2. Worth testing before we build anything

Hermes Desktop may already be routable _today_, with no new capability. {{classify_client}} treats every third-party client that honours the system proxy as {{Unknown}} and routes it in full - "OpenClaw, Hermes, an in-house script ... and all of them are routed". If the desktop app honours {{HTTPS_PROXY}} or the OS proxy setting, then turning on the machine-wide shell-proxy channel should route it, and the UA matcher should then attribute it.

That channel is the control AG-893 just moved into Settings ("Command-line tools"). _Could the reporter turn it on and re-check?_ If Hermes Desktop appears, this ticket becomes a coverage and naming problem rather than a missing integration, and the fix is much smaller.

h2. The limitation to decide on

Regardless of that test, the entry currently claims more than the switch delivers. Options, in increasing cost:

# _Name the surface._ The rail already does this for multi-surface products - Claude lists "CLI" and the desktop app as separate members under one section. Hermes has one member and a product name. Renaming the entry so its scope is visible is a small change and stops the Protected claim overreaching.

# _Hide Hermes_ until the desktop app is supported. Honest, and removes a switch most users cannot benefit from - but it also removes working CLI coverage from the people using it.

# _Support Hermes Desktop properly._ New capability, so AG-550 rather than this epic.

We have deliberately not implemented any of these. Option 1 is decidable from the principle AG-893 established (a control must match what it controls), but it changes a visible status from Protected to something narrower, and whether that ships before desktop support exists is a product call rather than ours.

Recommend: run the machine-wide-channel test first. It may collapse this to option 1 plus a documentation line.

## AG-893 - Terminal switch and the shell environment variables card describe overlapping coverage the user cannot tell apart
Status: In Progress | Assignee: matheus.reis | Priority: Medium

## Observed

The Terminal entry reads "Command line tools that follow your proxy settings" and "What this switch covers: Covers every program started after your next login, not only AI tools." The separate card below the sidebar reads "Also set shell environment variables - Routes command-line tools too. Machine-wide: it reaches git and curl, not only your AI tools." Both are on. Nothing on the page tells the user how the two differ, or why they would want git and curl traffic going through Gate.

## Expected

Each control states what it covers in terms the user can act on, the two do not describe the same coverage in different words, and the page gives a reason a user would turn on machine-wide routing. Traffic from non-AI tools like git and curl is covered only where the user has chosen that, with the benefit stated.

## Evidence

Gate Connect v1, macOS, 2026-09-16, Alex's Org, Terminal page with both controls on. Screenshot attached.

## AG-894 - Overview Messages chart stops hours before the Claude app chart for the same window
Status: To Do | Assignee: Gabriel Claramunt | Priority: Medium

## Observed

Both charts are labeled last 24 hours. The Overview chart's x-axis ends at hour 11 and its header reads "Last 24 hours - updated 11:55 AM". The Claude app chart, open at the same time, runs through hour 13 and includes work done at 1pm. Overview reports 1,076 messages against Claude's 929, so Overview holds more traffic but shows a shorter window.

## Expected

Both charts cover the same window, ending at the current hour, and Overview refreshes to the present rather than holding the time it was first loaded. A user comparing the two sees the same last hour on each.

## Evidence

Gate Connect v1, macOS, 2026-09-16, Alex's Org, captured at 13:25 with Overview last updated 11:55 AM. Screenshot attached.

## AG-895 - Toggling OpenCode asks the user to close Codex, an unrelated app
Status: To Do | Assignee: matheus.reis | Priority: Medium

## Observed

Turning the OpenCode switch on the OpenCode page opens "Apply changes to running apps?" listing Codex as "Running, and still using the route it started with", with the actions "Yes, close affected apps" and "No, I will reopen later". The banner behind it reads "Reopen to finish - Codex is still using the route it started with." Codex was not changed. OpenCode and Codex are separate apps, so changing one should not interrupt the other.

## Expected

Toggling an app's switch affects only that app. The restart prompt lists the app whose route changed, OpenCode in this case, and no others. If a change genuinely affects a second app, the dialog says which change reached it and why.

## Evidence

Gate Connect v1, macOS, 2026-09-16 14:19, Alex's Org, OpenCode page with the switch On and Protected, header reading "Partly routed - 5 of 6 Apps". Screenshot attached.

### Comment by matheus.reis (2026-09-17)
I cannot reproduce this from the code, and I think the report is two things at once. Not fixing it blind.

**Toggling OpenCode cannot put Codex in that dialog on any current path.** Every caller of `offerAfterChange` is scoped to the slugs that actually wrote:

* `routeSection` passes the members that wrote, and `cascadeTargets` never leaves the section clicked. The OpenCode section has one member, `opencode`.
* `routeApp` passes `[slug]`, with a comment recording the older bug where it did not - flipping Codex used to offer to close a running `claude`.
* Both reopen banners pass a single slug.
* Only the master toggle passes nothing, and it means every tool.

Rust narrows it again: `agent_names_for(Some(["opencode"]))` yields the one process name `opencode`.

**The Evidence here points at a different sequence.** It records the banner behind the dialog already reading "Reopen to finish - Codex is still using the route it started with". That banner is verdict-driven, it predates the click, and its own "Close tool" button raises exactly this dialog for Codex. So the likeliest story is a Codex reopen that was already pending, with the dialog attributed to the OpenCode click because that is what had just happened.

**Could you confirm whether the Codex banner was up before you touched the OpenCode switch?** That settles it either way.

**The suspicion behind the report is sound, though, and this is the part worth a decision.** Turning OpenCode on _does_ reach Codex, just not through this dialog. `setAppRouted` couples OpenCode to `env_export`, which sets the proxy variables for every process started afterwards, so anything already running - Codex included - keeps the environment it launched with. Gate offers to close none of them, because `env-proxy` contributes no process name.

That is arguably the real version of what this ticket asks for: "if a change genuinely affects a second app, the dialog says which change reached it and why". Making the OpenCode toggle offer to close every running agent is a product call about how loud a machine-wide switch should be, and it lands on the same disclosure surface AG-893 just settled, so I have left it for that decision rather than taking it myself.

Full write-up in `plans/ag-879-connect-defects.md` on `feat/ag-879-connect-defects`.

## AG-896 - OpenCode page shows no messages while the dashboard records the same traffic, and explains its counts in filler copy
Status: To Do | Assignee: Gabriel Claramunt | Priority: Medium

## Observed

The OpenCode page shows MESSAGES 0, BLOCKED/FLAGGED 0, TOKENS SAVED n/a, "No messages sent in the last 24hrs" and "No recent messages", several minutes after an OpenCode session at 2:21 PM sent a request and got a reply. The Gate dashboard shows those messages. OpenCode is On and Protected, and the header reads "Routed - 6 of 6 Apps". The page also carries two lines above its tiles: "Tokens saved: Nothing is set up for this yet, so there is nothing to report." and "These counts cover OpenCode. Gate routes more than that for this app, and the rest is not attributed to an app, so it is not counted here." Both describe internal attribution rather than anything the user can use or act on, and the same copy appears on the Claude and Hermes pages.

## Expected

Messages sent through a routed app appear on that app's page, with Messages and Recent activity matching the Gate dashboard for the same account and window, within the page's stated refresh interval. Each line of copy on the page either tells the user something they can act on or is removed, and where counts exclude some traffic the page says what is excluded in concrete terms and where to see it.

## Evidence

Gate Connect v1, macOS, 2026-09-16 14:23, Alex's Org. OpenCode session timestamped 2:21 PM visible in the same screenshot. Control: the Gate dashboard shows the messages. Screenshot attached.

## AG-897 - OpenRouter traffic is routed but shows no activity, and OpenRouter is listed under Apps
Status: To Do | Assignee: Gabriel Claramunt | Priority: Medium

## Observed

OpenRouter is On and Protected and the header reads "Routed - 7 of 7 Apps", but its page shows MESSAGES, BLOCKED/FLAGGED and TOKENS SAVED all as "n/a", with "Messages aren't attributed to this app" and "Recent activity isn't attributed to this app". Traffic was successfully routed to Gate through OpenRouter and none of it appears. OpenRouter is also listed in the sidebar under APPS, alongside Claude and ChatGPT / Codex, though it is a provider endpoint rather than an app a user runs. Its own description says so: "Any app you have pointed at OpenRouter."

## Expected

Traffic routed through OpenRouter is counted and listed on its page, with Messages and Recent activity matching the Gate dashboard for the same account and window. OpenRouter is grouped with the provider endpoints rather than under Apps, so the APPS list holds only applications the user launches.

## Evidence

Gate Connect v1, macOS, 2026-09-16 14:37, Alex's Org, OpenRouter page with the switch On and Protected, sidebar showing APPS 3 of 3 and TOOLS 4 of 5. Screenshot attached.

### Comment by matheus.reis (2026-09-16)
Parked with AG-889 - they are the same decision. See the full write-up in AG-889's comment and in {{plans/ag-879-connect-defects.md}} on {{feat/ag-879-connect-defects}}.

Short version:

_The composition half is right and cheap._ OpenRouter is {{band: "apps"}} today and its own description is "Any app you have pointed at OpenRouter", which is the definition of not-an-app. Moving it is one word, and it follows the same rule that moved the shell-environment channel out of the rail in AG-893.

_The activity half cannot be fixed as asked._ "Traffic routed through OpenRouter is counted and listed on its page" is not reachable: {{client_tool}} is derived from the caller's User-Agent, so traffic through OpenRouter is attributed to whichever app sent it, or to nothing. OpenRouter's page can never have its own attributed activity - which is why it shows {{n/a}} everywhere. The honest fix is for the page to say it is routed and protected and that its traffic is counted in the Overview, rather than showing three {{n/a}} tiles and two "isn't attributed" cards.

_Why it is held rather than done:_ this ticket's Expected asks for OpenRouter to be "grouped with the provider endpoints rather than under Apps", and {{api.openai.com}} (AG-889) is the same kind of entry. Doing one without the other leaves the rail inconsistent, and doing both introduces a third category - which is a shape decision for the squad lead, adjacent to the epic's own open question.

One answer unparks both: should the rail carry a provider-endpoints group, distinct from Apps and Tools? If not, we still fix the page copy and leave the entries where they are.

No code changes made for this ticket.

## AG-898 - "What happened" dialog splits tools across three outcome groups and lists ChatGPT and Codex separately
Status: In review | Assignee: Gabriel Claramunt | Priority: Medium

## Observed

After routing is turned off, the "What happened" dialog sorts tools into three groups: "Applied and verified" (Claude Code, OpenCode), "Reopened, not checked" (ChatGPT), and "Verification failed" (Codex). Each group carries its own explanation of what Gate did and did not check, so a user has to read three different outcome states to learn what happened to five tools. ChatGPT and Codex appear as two separate entries with different statuses, though they are the same app in Connect, listed in the sidebar as one entry, "ChatGPT / Codex". The Codex entry offers four actions, Retry verification, Use tool defaults, View diagnostics and Contact support, with nothing explaining why a routing check would need support.

## Expected

The dialog reports one outcome per app, with ChatGPT and Codex as the single app they are elsewhere in Connect, in one list a user can read at a glance. Actions offered are ones the user can act on themselves, and support is offered only where Connect has determined the user cannot resolve it.

## Evidence

Gate Connect v1, macOS, 2026-09-16 15:09, Alex's Org, dialog shown after turning "Route traffic through Gate" off. Screenshot attached.

### Comment by Gabriel Claramunt (2026-09-16)
PR: https://github.com/Constellation-Labs/gate-connect-app/pull/288 (base feat/new-app-ui)

All three complaints addressed.

Three outcome groups: the settled view sorted tools into up to six headed buckets, each with its own paragraph about what Gate did and did not check. Every row already states its own stage and what that stage means, so the buckets were a second copy of the same account one level up. It is now one list, in both the in-flight and settled states.

ChatGPT and Codex: SECTIONS in lib/groups.ts has grouped codex, chatgpt-apps and chatgpt into one app named "ChatGPT / Codex" since it was written, and the rail draws it that way. The dialog is handed running processes, one per surface, which is why it listed them apart. A new appForMember lookup and a new reopenAppRows grouping fix that through the same static table, so the two surfaces cannot drift.

An app reports its worst member. Reporting the better half is how a dialog tells somebody everything is fine while their editor is not routed, and the pairing in this ticket, ChatGPT reopened beside Codex failed, is exactly that shape. Where members disagree the row names each underneath. Actions stay per slug, because AG-566 AC 10 requires that retrying one tool never repeats the change for another, so merging changes what is shown and never what a button does.

Contact support: dropped from verify_failed, and from close_failed as well. The ticket reports only the Codex case, but the rule its Expected states covers both, and a process Gate could not signal is one the person can close from their own window. View diagnostics stays on both, which hands over a reading rather than opening a ticket about a routing check. Say if you would rather keep close_failed's; it is a one-line revert.

Verification: tsc --noEmit clean, vitest 61 files / 1164 tests pass, 15 of them new, including the exact four-tool set from this ticket.

## AG-899 - Turning "Route traffic through Gate" off breaks connectivity for every app, including apps Gate does not support and a local LLM server
Status: To Do | Assignee: Gabriel Claramunt | Priority: High

## Observed

With "Route traffic through Gate" switched off and every app and tool reading "Not routed - Off", apps cannot reach their providers. This holds for apps that were routed and for apps that were never turned on, and for sessions started after the switch was flipped off.

* Claude Code: "API Error: Connection refused - a firewall or proxy may be blocking it (ConnectionRefused)", retrying at attempt 9/10, and "Auto-update failed".
* Codex: MCP clients fail to start, then "stream disconnected before completion: Connection refused (os error 61)" and "Reconnecting... waiting for network".
* OpenCode: "Cannot connect to API: Unable to connect. Is the computer able to access the url? retrying in 13s attempt #5".
* OMP, which Gate does not list as a supported app, pointed at a local LLM server on mac-studio: "Error: Unable to connect. Is the computer able to access the url?" on every request.
* Earlier the same session, OMP pointed at OpenRouter failed the same way with routing off, and turning the OpenRouter switch back on restored it.

Connect shows "No apps are set to route through Gate Connect" and the banner "3 apps aren't protected. Routing is set to off. Reconnect to restore protection."

## Expected

Turning routing off returns every app to the path it would take with Gate not installed. Apps that were routed, apps that were never routed, apps Gate does not support, and traffic to a local server all connect normally, including sessions started after the switch is flipped. Turning Gate off never leaves the machine unable to reach a provider.

## Evidence

Gate Connect v1, macOS, 2026-09-16, Alex's Org. Two screenshots attached: 15:11 with OMP on OpenRouter, and 15:37 with Claude Code, Codex, OpenCode and OMP on a local server all failing at once. Control: connectivity returned when the relevant switch was turned back on.

## Who it affects

Anyone who turns Gate routing off, which is the documented way to stop using Gate. It also reaches software Gate does not claim to touch.

### Comment by Alex Brandes (2026-09-16)
Root cause, traced in the code on \`feat/new-app-ui\` (91925c1), which is the branch this build comes from. Note the screens in the screenshot are not on \`main\`.

\*\*Why every app breaks, including ones switched off and ones Gate does not manage\*\*

Routing on exports seven variables into the login session, pointing every process on the machine at the engine: \`http_proxy\`, \`https_proxy\`, \`HTTP_PROXY\`, \`HTTPS_PROXY\`, \`no_proxy\`, \`NO_PROXY\`, \`NODE_EXTRA_CA_CERTS\`, built in \`crates/core/src/proxy/proxy_env.rs:46-99\` and written with \`launchctl setenv\` in \`crates/core/src/proxy/system_proxy.rs:436-445\`, from \`manager_core.rs:329\`.

A per-app switch decides what the engine \*rewrites\*, not what it \*carries\*. \`decide()\` in \`crates/core/src/proxy/mod.rs:2709-2745\` returns \`Passthrough\` for anything not matching an enabled domain, and the engine blind-tunnels it. So an app switched off was still dialing 127.0.0.1 and depending on the engine being alive. An app Gate does not manage, such as OMP, breaks for the same reason: it honors \`HTTPS_PROXY\` like any HTTP client. \`NO_PROXY\` is only \`localhost,127.0.0.1,::1\`, so traffic to a local LLM server on a Tailscale name rides the engine too.

Turning routing off calls \`disable_env()\` then stops the engine (\`manager_core.rs:410-458\`). \`disable_env\` runs \`launchctl unsetenv\`, which only changes what processes started \*after\* it inherit. Every terminal, editor and CLI already running keeps the old value and dials a port that no longer accepts. A new session started from a terminal window that was already open inherits that window's stale environment, which is why new sessions failed too.

Turning the OpenRouter switch back on fixed it because the engine's port is persisted and reused (\`crates/core/src/proxy/port_persist.rs\`), so the listener came back on the same address the stale variables already pointed at. The traffic being routed was incidental.

\*\*Same defect through a second door\*\*

\`src-tauri/src/lib.rs:5353-5357\` runs the same teardown on \`RunEvent::Exit\`, so quitting Gate Connect breaks already-running terminals identically. A fix that only changes the toggle leaves this one.

Fix tracked in AG-911.


### Comment by Alex Brandes (2026-09-16)
Measured on the affected Mac, 2026-09-16 16:45, to settle which channel does what.

Three channels carry Gate's proxy, and they fail differently:

1\. \*\*PAC\*\* at \`127.0.0.1:50367/proxy.pac\`, enabled on Wi-Fi and Tailscale. Its contents route only \`api.anthropic.com\`, \`claude.ai\` and \`chatgpt.com\`; everything else returns \`DIRECT\`. Read only by clients that consult the OS proxy setting. A dead PAC makes those clients fall back to direct, so it cannot be what refused the connections.  
2\. \*\*launchctl environment\*\*, exported machine-wide when the shell-environment choice is on. \`no_proxy\` is only \`localhost,127.0.0.1,::1\`, so LAN and Tailscale hosts ride the engine. \`launchctl unsetenv\` cannot reach processes already running, so a stale value refuses.  
3\. \*\*Per-tool config rewrites.\*\* \`\~/.claude/settings.json\` still holds \`HTTPS_PROXY=http://gate-claude-code:route@127.0.0.1:50365\` and \`NODE_EXTRA_CA_CERTS\`. Claude Code reads that at every launch, independently of the PAC and of the environment, so it breaks whenever the engine is down for any reason, including a clean quit.

Timing: \`proxy/env-routing.json\` was last written 15:29:18, so the environment channel was on through the 15:09 to 15:19 screenshots and was turned off at 15:29. The 15:37 failures are processes still holding the value from before that write.

For the record, two readings that do not hold. \`env-routing.json\` is the persisted opt-in for the machine-wide environment export (\`ExportChoice\` in \`crates/core/src/proxy/proxy_env.rs\`), not engine state, so its \`false\` does not contradict \`routing-intent.json\`. And the engine is not in a failed state: PID 82337 listens on 50365, 50366 and 50367 and the PAC returns 200. The "proxy engine is not running" lines are WARNs from the pre-enable restore pass (\`crates/core/src/provider.rs:794\`), which runs before the engine starts by design (\`crates/core/src/routing.rs:44-56\`).


## AG-900 - Closing Claude leaves the Claude desktop app running while Connect reports it closed
Status: In Progress | Assignee: Gabriel Claramunt | Priority: Medium

## Observed

The "What happened" dialog lists Claude Code under "Waiting for you to reopen" with the status "Reopen required - Closed. Open it again and Gate will check its route." The Claude desktop app was still open and usable at that moment, in an active chat. The Claude entry in Connect describes itself as covering both surfaces: "Claude Code in your terminal, and the Claude desktop app - its model calls and its chats." So the close action reached Claude Code only, and the dialog reports a closed state that is not true of the app the switch claims to cover.

## Expected

An entry that covers both Claude Code and the Claude desktop app closes both when Connect closes it, and reports each surface's real state. Where Connect cannot close a surface, it says which one is still running and what the user should do, rather than reporting it closed.

## Evidence

Gate Connect v1, macOS, 2026-09-16 15:17, Alex's Org. Claude desktop app open in an active chat in the same screenshot. Screenshot attached.

## AG-901 - "Route Claude through Gate?" consent dialog is written in terms the user cannot follow
Status: In Progress | Assignee: matheus.reis | Priority: Medium

## Observed

Turning on Claude opens a dialog headed "Route Claude through Gate?" with the subtitle "This also routes claude.ai, which Gate sees on the account you are already signed in with", then three paragraphs: "Gate records and inspects that traffic. It does not supply a key for it, and it cannot read anything you are not sending anyway.", "It is matched on host, so it covers everything on this machine that sends to claude.ai - not only Claude.", and "Asked once per app. Turning Claude off later does not bring this question back." The buttons are "Not now" and "Route Claude". A user is being asked to consent to traffic inspection, and the text does not make clear what Gate will see, what it will not see, or what they are agreeing to.

## Expected

The dialog states in plain English what turning Claude on does, what Gate records, and what it does not, in the order a person needs it, so a user can decide without rereading. It says plainly that this consent is asked once and how to change the decision later.

## Evidence

Gate Connect v1, macOS, 2026-09-16 15:19, Alex's Org, dialog shown when turning the Claude switch on. Screenshot attached.

## AG-911 - Keep a local passthrough listener bound when routing is off
Status: To Do | Assignee: Gabriel Claramunt | Priority: High

Keep a local listener bound to the engine's persisted port whenever routing is off, forwarding every connection verbatim to the real host, so an app that already holds Gate's proxy environment variables can still connect.

Routing on exports http_proxy, https_proxy, HTTP_PROXY, HTTPS_PROXY, no_proxy, NO_PROXY and NODE_EXTRA_CA_CERTS into the login session, pointing every process on the machine at 127.0.0.1 on the engine's port. Turning routing off unsets them with launchctl and stops the engine. launchctl only changes what processes started after it inherit, so every terminal, editor and CLI already running keeps the old value and dials a port that no longer accepts. That is why turning routing off leaves apps unable to connect even when their own switch was off, and even for apps Gate does not manage. The user's intent when they flip that switch is to stop Gate seeing their traffic, not to take a proxy out from under running processes, and those are different actions that should not share one control.

## Scope

* Off stops interception, certificate minting, gateway rewriting and audit rows, and forwards every connection straight to the real host.
* The listener survives quitting Gate Connect. The app-exit path currently runs the same teardown as the toggle, so a listener that dies with the GUI fixes the toggle and leaves the quit path breaking the same sessions.
* Disconnect keeps today's destructive behavior: variables unset, listener stopped, and the user told which apps need restarting.
* no_proxy covers private, link-local and Tailscale addresses in both states, so local-network traffic never traverses the engine.
* The UI says plainly that a local passthrough stays running while routing is off, and that it inspects nothing.

## Acceptance criteria

* With routing turned on and then off, a new session started in a terminal window that was already open connects successfully.
* In the off state Gate writes no audit rows and performs no TLS interception for any app.
* Quitting Gate Connect leaves already-running terminals and apps able to connect.
* Disconnect removes Gate from the path and names the apps that need restarting.
* Traffic to private, link-local and Tailscale addresses does not traverse the engine in either state.

## Dependency

Fixes the behavior reported in AG-899.

### Comment by Alex Brandes (2026-09-16)
Three channels point tools at the engine, not two, and the criteria above should cover all three.

The third is the per-tool config rewrite. \`\~/.claude/settings.json\` holds \`HTTPS_PROXY=http://gate-claude-code:route@127.0.0.1:50365\` and \`NODE_EXTRA_CA_CERTS\`, written by Gate. Claude Code reads it at every launch, independently of the PAC and of the exported environment, so it breaks whenever the engine is down for any reason, including a clean quit. Codex, OpenCode and Hermes have the same shape.

A listener that stays bound covers this channel too, so the criteria hold. Adding one:

\- Gate never leaves a tool's own config pointing at a port nothing is serving.

Evidence in AG-899's comments.

