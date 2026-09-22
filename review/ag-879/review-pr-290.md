# Review: PR #290 against epic AG-879

Reviewed 2026-09-17 at head `8ec6e2c` (21 commits on `feat/ag-879-connect-defects`,
base `feat/new-app-ui` at `dc682a9`). No code was changed for this review.

Companion files in this folder:

- `jira-children.md` - all 23 children of AG-879 with their comments, as pulled today.
- `pr-290-files.md` - the 32 files the PR touches.
- `test.log`, `tsc.log`, `e2e.log` - the runs described under Verification.

## Verdict in one paragraph

The PR does what its description says for AG-881, AG-883, AG-887, AG-891, AG-900 and
AG-901, and the investigations on AG-882, AG-888, AG-889, AG-895 and AG-897 are sound
and written to the tickets. Two things should be fixed before merge, both on AG-893:
the new Settings row and the OpenCode consent dialog both say the shell channel is
what routes OpenCode, and the repo's own code comment says that is wrong; and the
same one control is now described three different ways on three surfaces, which is
the shape of the defect AG-893 reported. Three tickets the PR closes have no Jira
comment explaining what was built where the build differs from the ticket's Expected
(AG-887, AG-893, AG-891). Everything else below is minor or a process note.

## Verification

| Check | Result |
| --- | --- |
| `npx tsc --noEmit` (app) | clean |
| `pnpm typecheck:e2e` | clean |
| `pnpm test` | 61 files, 1182 tests, all pass |
| `pnpm test:e2e` | 260 passed (33.2s), all pass |

The PR description says 1167 unit tests; the branch now has 1182 with the consent
and picker tests added in the second batch. Not run: Rust tests, since the only Rust
change is a doc comment on a constant. Not verified: the running app against staging.
The PR says it was, and nothing here contradicts that, but I did not repeat it.

## Ticket by ticket

### AG-881 - drop the dollar estimate. Done, correctly.

`tokensSavedAmount` is removed from `UsageStats`, `adapt` and `Stat`'s `delta` slot.
The deviation from frame `408:25130` is stated in three places (commit, `metrics.tsx`,
`activity.ts`) and rests on the epic's Key decision, which is the right authority.
Nothing else read `delta`. No Jira comment on the ticket; the commit is enough here
since the ticket's Expected is exactly what was built.

### AG-883 - tile jump to nothing. Done, with a reading worth stating on the ticket.

The tile now navigates only when `savings.length > 0` and the read is neither pending
nor failed. The ticket's Expected says "either opens its detail view in place or does
nothing" and "scroll position stays where the user left it". The PR keeps AG-572's
navigation when there are rows, so with rows the page still scrolls. That is a
reasonable reconciliation of two tickets that conflict, but it is not the Expected as
written. It should be said on AG-883 in a sentence, so the reporter can object if they
meant "never scroll".

One thing the fix does not address: with rows present the section is still the
second-to-last card, so `block: "start"` still cannot be honoured and the pane still
pins at its maximum. It lands on content now, so the report's symptom is gone, but the
scroll is still a jump to the bottom rather than to the section's top. Worth one line
on the ticket rather than a change.

### AG-887 - Type column empty. Done, but the ticket's Expected was reinterpreted and the ticket does not say so.

The ticket reads "Type" as the request type ("every row shows the request type that
produced it"). The PR reads the column as the guardrail category, which is what the
frame draws and what the gateway sends, and fills the no-category-but-examined case
with "Regular". Keying on `securityAction` rather than the category is the right
principle-6 line and the tests pin it.

Two gaps:

1. **No Jira comment on AG-887.** The Expected asks for something the column does not
   carry. A reporter reading "Regular" on every row may reasonably reopen this as
   "still not showing the type". The comment should say what the column is, why
   "Regular" is the honest fourth value, and that request type per row is not in the
   activity payload (if that is so; I did not check the gateway contract in
   `docs/ag-572-activity-api-contract.md`).
2. The dash cells carry a `title` tooltip ("No guardrail category recorded, or not
   your request"); the new "Regular" cell carries none. A reader who wonders what
   "Regular" means has nowhere to hover. Minor.

The em dash to hyphen change is right per CLAUDE.md and the second commit's
consolidation onto one glyph is a good catch.

### AG-891 - Gate plan "Unavailable". Done, with one dependency and one question.

The four states are real and tested (`SettingsPane.test.tsx` 322-360). `formatPlan`
maps `paid` to "Pro", `free` to "Free", passes unknown values title-cased and keeps
null. The App pane goes through the same function so the two panes agree.

- **The PR correctly says AG-891 must not close until gate#1043 lands.** The ticket
  has no comment saying so. Someone closing tickets off the merged PR will close it.
  Put the dependency on the ticket, and ideally a Jira link to whatever tracks
  gate#1043.
- **`useCredits(canRead)` now fires on every window focus for every pane.** The PR
  acknowledges the cost. It is one small GET, fine. But note the failure path: a
  transient `/v1/me/credits` failure on a focus return now flips the Settings row to
  "Unavailable + Retry" and the App pane's plan line to nothing, because `useCredits`
  drops `credits` on any failure. Before, that only happened while an app pane was
  open. Acceptable, and consistent with the hook's own comment about stale balances,
  but it is a behaviour change on Settings that the row's Retry now exists to cover.
- API-key accounts: `canRead` is `loaded && account !== null`, so API-key accounts
  also read credits. If `/v1/me/credits` refuses an API-key credential, those users
  now see "Unavailable" with a Retry that will never succeed, where before they saw
  a static "Unavailable". I did not verify the gateway's behaviour for API-key auth
  on that endpoint. Worth one check against staging with a pasted key before merge.

### AG-893 - two controls for one channel. The structure is right; the copy is not yet.

What was built: `env-proxy` is filtered out of the window's and tray's app lists
(filtering the tool list, not the ledger, and the comment explains why), a
"Command-line tools" toggle row appears in Settings under Connection, and the tray's
card becomes a status display. The OpenCode dialog gains the certificate sentence.
The reasoning that a machine-wide setting is not an app is sound and matches the
ticket's Expected ("each control states what it covers ... the two do not describe
the same coverage in different words").

**Blocking finding 1: "Required by OpenCode" and the dialog's premise are contradicted
by the repo's own comment.** `useRouting.ts` lines 60-72 say, in the doc for the
`opencode-env` gate: "This used to call the variables 'the only way OpenCode routes',
which is wrong and was wrong when it was written: `integrations/opencode.rs` rewrites
`provider.<id>.options.baseURL` to the loopback relay, needing neither a variable nor
the CA." The channel covers what the config write cannot see: a provider added later
or one outside the allowlist. So:

- Settings row: "Required by OpenCode." is false as stated. OpenCode's configured
  providers route without it. Something like "OpenCode asks for it, to cover providers
  added later" is the true sentence.
- `OpenCodeEnvDialog`: "OpenCode has no gateway setting of its own, so Gate routes it
  with your machine's proxy variables" predates this PR, but this PR edits the
  paragraph and adds the certificate sentence to it, so it is this PR's to fix.

Principle 1 and the epic's own rule about copy that describes mechanics apply
directly: the dialog is the moment of consent for a machine-wide change and it
misstates why it is being asked.

**Blocking finding 2: one control, three descriptions.** After this PR the channel is
described as:

| Surface | Copy |
| --- | --- |
| Settings row (window) | "Routes every program you start afterwards, not only AI tools, and trusts Gate's certificate in Node. Required by OpenCode." |
| Tray card (`735:37341`) | "Sets HTTPS_PROXY for your whole shell, so OpenCode and other terminal tools route too." |
| Popover Terminal blurb (`groups.ts`) | "Routes every program started after your next login, not only AI tools. Gate inspects traffic to the AI providers it knows and passes everything else through untouched." |

"Afterwards" and "after your next login" are different claims. `system_proxy.rs`
lines 370-385 say the macOS truth is "everything the session starts afterwards",
including new Terminal windows; "next login" is the Linux drop-in's truth
(`system_proxy_linux.rs` line 28). The tray names a mechanism (`HTTPS_PROXY`) the
other two avoid, and none of the three mentions the certificate except Settings. This
is the AG-893 complaint restated across three surfaces instead of two. The tray card
copy is the frame's and can stay per CLAUDE.md, but the Settings and popover strings
are ours and should agree with each other and with the platform.

**Non-blocking, worth noting:**

- Commit `f5af250` ("Say on the Terminal pane that the switch is machine-wide") made
  `describeSection` read `blurb`. Commit `1e71e5d` then removed the Terminal pane from
  the window and tray entirely. The `describeSection` change now has no consumer in
  the new shells; only the popover's `FamilyPanel` renders the blurb, and it read it
  directly already. Harmless, but the PR description's "both shells now render the
  same string" is no longer what happens. Either drop the `describeSection` change or
  keep it with a comment saying it is for a Terminal pane that may return.
- The Settings toggle can turn the channel off while OpenCode is On. OpenCode's
  status stays "Connected" because `opencode.rs` checks `baseURL`, so the rail keeps
  reading Protected, correctly, and only the providers-added-later coverage silently
  drops. Given finding 1, that is the expected behaviour, but the Settings row should
  not imply OpenCode breaks.
- The "What happened" recovery dialog still lists the environment channel as a row
  (`new-ui-routing.spec.ts` 1042-1066 still expects it). The rail no longer has a
  Terminal entry, so a reader sees a row for something they cannot find anywhere else
  in the window. Not this PR's regression, but this PR is what made it visible.
- Master toggle: `setMasterRouted` calls `proxyEnable`/`proxyDisable` and never
  touches `env_export`, so the master switch does not silently flip the channel. Good;
  this was the thing I most wanted to confirm.
- No Jira comment on AG-893. The ticket describes a card that #273 already removed
  and a Terminal row this PR removes; the ticket needs a comment saying the control
  is now in Settings, so the reporter knows where to look.

### AG-900 - desktop app left running. Done, and the harness fix is the better half.

`useSectionRouting` now passes every moved member's key; Rust's `agent_names_for`
drops the ones with no process, which `agent_names_for`'s own doc anticipated. The
e2e harness gains the two desktop rows, exact-case matching and surface-derived
`can_reopen`/`verifiable`, and the changed assertion in `new-ui-routing.spec.ts`
("Not running" count 1, "no process to look for" count 2) is right for the reason
given.

Two things to check with Gabriel, since AG-900 is assigned to them and In Progress:

- Whether they have their own AG-900 work in flight, as they did for AG-880 and
  AG-886. This PR already reverted two collisions; a third would be avoidable.
- **Interaction with AG-898 (#288).** With this change, toggling the ChatGPT / Codex
  section now offers to close both `codex` and the ChatGPT desktop app, so the reopen
  and "What happened" dialogs can now show ChatGPT and Codex as two rows, which is one
  of the three complaints in AG-898. #288 collapses that view; whichever merges second
  should be re-checked against the other. Worth a comment on both PRs.

Also: `ChatGPT` now appears in reopen offers where it never did before. Rust's
`can_reopen` for `Surface::App` needs a resolved relaunch target (`lib.rs` 3270). If
that resolution fails on a machine, the row degrades to "reopen required" rather than
"Gate will reopen", which is fine. Not verified on a real machine.

### AG-901 - consent dialog copy. Done. Two copy nits.

The new order (what the switch does, what Gate records, what it does not, the scope,
the reversibility) matches the ticket's Expected and the component test pins each
sentence. The reasoning for not saying "change it in Settings" is verified in the
commit against `accept_session_routing` and `account::clear`.

- The last paragraph says the same fact twice: "You are asked this once." and "but
  Gate will not ask this question again." One of them can go.
- "and so does claude.ai, where you are already signed in" is good. The `wide`
  fallback "including a surface where you are already signed in" is vaguer than the
  old one and only fires when a session member has no hosts; acceptable since no
  shipping section hits it.
- The ticket's Expected asks the dialog to say "how to change the decision later".
  The new text says turning Claude off stops the routing. That is the honest answer
  and the commit explains why "Settings" would be a lie. Say so on the ticket, since
  the reporter will look for a Settings path.

### AG-888 - multi-select picker. Investigated, not implemented; the right call.

The allow-list semantics are confirmed against the header doc and `applyUserModelChoice`,
the Jira comment is thorough, and the picker now states the rule. The `GATE_MODEL_HEADER`
doc fix is the kind of correction that prevents the next filing.

One copy check: "served as X, the first in the list". `draft` is built by appending
(`dialogs.tsx` 1170), so `draft[0]` is the user's first choice, and the chosen list
renders in draft order. But the catalogue list above it is in catalogue order, so
"the first in the list" is ambiguous for a reader who has two lists on screen. "the
first one you chose" would be unambiguous.

### AG-882 - 5% vs 4%. Investigated, written up. Agree with the conclusion.

The 7-day versus 24-hour finding is correct and the reason not to port
`gateAttributedPct` into `gateway-proxy` is the right one. The ticket is assigned to
Gabriel and still To Do, with your comment recommending close-or-rescope. Nobody has
acted on it. Someone should either move it or file the gateway ticket the comment
proposes; otherwise the write-up is the last thing that happens.

### AG-889 and AG-897 - parked on one product question. Agree.

The write-up in `plans/ag-879-connect-defects.md` is complete and the question
("should the rail carry a provider-endpoints group?") is the right one to send to the
squad lead. Two process notes:

- **AG-889 has the same comment posted twice** (2026-09-16, one long and one shorter).
  Delete one.
- Neither ticket names who is expected to answer or by when. The epic says the squad
  lead settles the OpenAI question; AG-889's comment should tag them.

### AG-895 - cannot reproduce. Agree, and the question to the reporter is the right move.

The trace of every `offerAfterChange` caller matches what I read in
`useSectionRouting.ts`, `useRunningApps.ts` and `lib.rs`. The ticket remains To Do
with a question to the reporter; it should probably move to a "waiting" state so it
does not read as untouched.

### AG-880 and AG-886 - reverted in favour of #286 and #287. Right call.

The revert commit's reasons are specific (`not_routed` shares the verified bucket; the
plural subject) and verifiable. Nothing of either ticket remains in the diff.

## Changes that are not on any ticket

1. **Error banner back under the scrim** (`aea4062`). This reverts #244's deliberate
   exception. The commit's argument (a modal another element sits on is not modal) is
   good, and the acknowledged cost is real: a failed rename or key replacement inside
   a dialog now reports into a dimmed banner the user cannot dismiss until the dialog
   closes. The commit names the better fix (report the error inside the dialog) and
   does not do it. This is outside AG-879's scope and rides in a PR with eleven
   tickets. It deserves its own ticket so the follow-up is tracked, and a line in the
   PR description saying #244's author was consulted, if they were.
2. **Terminal blurb correction** (`2875750`). The old sentence about local models was
   false for LAN and Tailscale hosts and the correction is right. It ends up shipping
   only in the popover, per the AG-893 note above.

## PR hygiene

- The description says "14 commits"; the PR has 21. It also has three stacked
  sections with strikethroughs. A rewrite of the body into one current statement
  would help whoever reads it after merge.
- The base is `feat/new-app-ui`, per the repo's branch topology. `mergeable_state` is
  clean.
- Every commit carries a `Co-Authored-By: Claude Opus 5` trailer. Fine, noting only
  for the record.

## Jira state to fix alongside the merge

| Ticket | Status now | Should be |
| --- | --- | --- |
| AG-881 | In Progress | In review, no comment needed |
| AG-883 | In Progress | In review, plus one comment on the AG-572 reconciliation |
| AG-887 | In Progress | In review, plus a comment on what "Type" is and why "Regular" |
| AG-891 | In Progress | In review, plus a comment: blocked on gate#1043, do not close |
| AG-893 | In Progress | In review after the copy fix, plus a comment: control moved to Settings |
| AG-900 | In Progress, assigned Gabriel | Confirm ownership with Gabriel, then In review |
| AG-901 | In Progress | In review, plus a comment on why not "Settings" |
| AG-882 | To Do, Gabriel | Close or re-scope per the comment; file the gateway ticket |
| AG-889 | To Do | Delete the duplicate comment; tag the squad lead |
| AG-895 | To Do | Waiting on reporter |
| AG-897 | To Do, Gabriel | Waiting on the same decision as AG-889 |
| AG-888 | In Progress | In review; the comment is already there |

## Summary of asks before merge

1. Fix the two AG-893 copy problems: drop or soften "Required by OpenCode" and correct
   the OpenCode dialog's premise; make the Settings and popover descriptions agree
   with each other and with `system_proxy.rs`.
2. Comment on AG-887, AG-891 and AG-893 with what was built and why it differs from
   the ticket text.
3. Check with Gabriel on AG-900 ownership and the AG-898 interaction.
4. Ticket the error-banner regression follow-up.
5. Optional: remove or justify the now-unconsumed `describeSection` blurb change;
   dedupe the AG-889 comment; tighten the consent dialog's last paragraph.
