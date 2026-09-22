# Handoff: address review findings on PR #290 (AG-879)

You are working on branch `feat/ag-879-connect-defects` in the Gate Connect app
(`gate-connect-app`, base branch `feat/new-app-ui`). PR:
https://github.com/Constellation-Labs/gate-connect-app/pull/290. Epic:
https://constellationnetwork.atlassian.net/browse/AG-879.

Read `CLAUDE.md` first. Two of its rules matter here: never type an em dash
anywhere, and the Figma frame wins on copy where a frame draws the thing. None of
the strings you will change is drawn by a frame except the tray card, which you
must not touch.

The full review is in `review/ag-879/review-pr-290.md`. This file is the work
list extracted from it. Do the tasks in order. Commit each task separately, in
the commit style already on the branch (a sentence for a title, a body that
says why, and the `Co-Authored-By` trailer the harness gives you). Do not
force-push, do not rebase, do not amend commits that are already on the remote.

## Verification you must run before each commit

```
npx tsc --noEmit
pnpm typecheck:e2e
pnpm test
pnpm test:e2e
```

All four must be clean. Baseline at head `8ec6e2c`: 1182 unit tests, 260 e2e.

## Task 1 (required): stop saying the shell channel is what routes OpenCode

Ground truth, from `src/lib/useRouting.ts` lines 60 to 72: OpenCode's configured
providers are routed by `crates/core/src/integrations/opencode.rs` rewriting
`provider.<id>.options.baseURL` to the loopback relay, "needing neither a
variable nor the CA". The shell channel exists to carry what that rewrite
cannot see: a provider added later, one outside the allowlist, or one skipped as
local. Read those lines before writing.

Change two strings so they agree with that:

1. `src/components/gc/SettingsPane.tsx`, the `shell-proxy` row's `description`
   (currently: "Routes every program you start afterwards, not only AI tools,
   and trusts Gate's certificate in Node. Required by OpenCode."). Remove
   "Required by OpenCode." and replace it with a clause that is true, e.g.
   "OpenCode asks for it, to cover providers you add later." Keep the row to one
   line's worth of text; the commit `73b8361` trimmed it for that reason.
2. `src/components/gc/dialogs.tsx`, `OpenCodeEnvDialog`'s body paragraph
   (currently begins "OpenCode has no gateway setting of its own, so Gate routes
   it with your machine's proxy variables."). Rewrite the first sentence so it
   says the variables cover providers OpenCode's config does not name, rather
   than that they are how OpenCode routes. Keep the git/curl/npm sentence and
   the Node certificate sentence that follow.

Update the tests that pin these strings (`SettingsPane.test.tsx`, and whatever
test covers `OpenCodeEnvDialog`; grep for the old phrases). Update the doc
comment above `OpenCodeEnvDialog` if it repeats the false premise.

## Task 2 (required): make the channel's descriptions agree across surfaces

After this PR the same setting is described three ways:

| Surface | File | Copy |
| --- | --- | --- |
| Settings row | `src/components/gc/SettingsPane.tsx` | "Routes every program you start afterwards ..." |
| Popover Terminal blurb | `src/lib/groups.ts`, the `terminal` section's `blurb` | "Routes every program started after your next login ..." |
| Tray card | `src/components/gc/Tray.tsx`, `CliCard` | "Sets HTTPS_PROXY for your whole shell ..." |

Do NOT change the tray card; it is the frame's copy (`735:37341`).

The platform truth, from `crates/core/src/proxy/system_proxy.rs` lines 370 to
385 and `crates/core/src/proxy/system_proxy_linux.rs` line 28: on macOS the
variables reach everything the login session starts afterwards, including new
Terminal windows; on Linux the drop-in applies at the next login. Pick one
wording that is true on both (e.g. "every program you start after this, or
after your next login on Linux" is too long; "programs started from now on" is
false on Linux). A wording that is true everywhere: "every program started
after this takes effect" is vague. Prefer stating the macOS behaviour and
qualifying Linux in the popover blurb only if the blurb is shown there. Check
where `blurb` renders (`src/screens/FamilyPanel.tsx`) and whether the platform
is knowable at that call site (`proxy.env_export_separable` is false on Linux,
which is one signal). Make the Settings description and the popover blurb use
the same sentence for the same claim. Both should mention the Node certificate
effect, since the review found only Settings did.

Also decide about `describeSection` in `src/lib/groups.ts` (around line 965):
commit `f5af250` made it read `blurb`, and commit `1e71e5d` then removed the
Terminal pane from the window and tray, so the change has no consumer in the
new shells. Either remove that branch and its test in `groups.test.ts`, or keep
it and shorten its comment to say it is for a pane that no longer renders.
Removing is preferred. Say which you did in the commit body.

## Task 3 (minor copy): three small tightenings

All in `src/components/gc/dialogs.tsx`:

1. `SessionConsentDialog`, last paragraph: "You are asked this once. Turn {name}
   off whenever you like and the routing stops, but Gate will not ask this
   question again." says the once-only fact twice. Keep one statement of it.
   Update `dialogs.consent.test.tsx`.
2. `ModelPickerDialog`, the AG-888 sentence: "Anything else it asks for is
   served as {draft[0]}, the first in the list." Two lists are on screen (the
   catalogue and the chosen set). Change "the first in the list" to "the first
   one you chose". Update `dialogs.picker.test.tsx`.

And in `src/components/gc/AppPane.tsx`, `RecentActivity`, the Type cell: the
dash cells carry a `title` tooltip and the new "Regular" cell does not. Add a
`title` to the "Regular" span only, e.g. "Examined by Gate; no guardrail
matched". The cleanest place is `src/lib/toolEvents.ts` next to `REGULAR`, then
render it in the cell when `entry.category === REGULAR` (export the constant or
add a `categoryTitle` field; prefer the field so the component does not compare
strings). Add a unit test in `toolEvents.test.ts`.

## Task 4 (docs): the PR body

Do not push anything for this task. Write a replacement PR body to
`review/ag-879/pr-290-body.md` that is one current statement of what the PR
does, with no strikethroughs and no stacked "batch" sections. It must:

- list the tickets by outcome: closed (AG-881, 883, 887, 891, 893, 900, 901),
  investigated with no code change (AG-882, 888, 895), parked (AG-889, 897),
  removed in favour of other PRs (AG-880 -> #286, AG-886 -> #287; AG-898 -> #288
  was never here);
- keep the warning that AG-891 must not close until
  Constellation-Labs/gate#1043 lands;
- keep the "Also in here" items: the error banner back under the scrim, the
  local-model sentence correction, the em dash removal;
- name the AG-898 interaction: this PR makes the ChatGPT / Codex section offer
  to close the ChatGPT desktop app too, which can show ChatGPT and Codex as two
  reopen rows, which #288 is collapsing; whichever merges second re-checks;
- give the real counts: unit and e2e test totals after your changes;
- end with the attribution line the harness requires.

## Task 5 (Jira): draft comments, do not post

Write the following to `review/ag-879/jira-comments-draft.md`, one section per
ticket, ready to paste. Do not post them; the user will review and post. Use
plain sentences, American spelling, no em dashes. Each comment should be under
200 words.

- **AG-887**: the Type column is the guardrail category the gateway records,
  which the frame draws; per-request "type" in the reporter's sense is not in
  the activity payload (verify this against `docs/ag-572-activity-api-contract.md`
  and say what the payload does carry). A request the gateway examined with no
  match now reads "Regular"; a row with no security action at all keeps the
  dash. Ask whether that answers the report.
- **AG-891**: the row now reads the plan from `/v1/me/credits` and says "Pro"
  for `paid`, matching the dashboard. Do not close: the endpoint reads a cached
  column, so a lapsed org still reports `paid`; Constellation-Labs/gate#1043
  fixes that, and this closes when it lands.
- **AG-893**: the card was removed by #273 and the Terminal row is gone; the
  one control is now a "Command-line tools" toggle in Settings under
  Connection, stating what it reaches and the certificate effect. The tray
  shows the state and offers no second switch. Say where to look.
- **AG-883**: the tile navigates only when the Token savings section has rows,
  and is an ordinary tile otherwise. This keeps AG-572's navigation; if the
  reporter meant "never scroll", say so and it becomes a plain tile.
- **AG-901**: the dialog does not say "change it in Settings" because nothing
  in Settings changes it by design (`accept_session_routing` never un-records;
  reset leaves `preferences.json`). Turning the app off is the way to stop the
  routing, and the dialog now says that.
- **AG-889**: note that the same comment is posted twice on 2026-09-16 and one
  should be deleted (the user does this in Jira; just flag it in the draft).

## Task 6: questions only the user can answer

Put these at the top of `review/ag-879/jira-comments-draft.md` under "For you
to decide", do not act on them:

1. AG-900 is assigned to Gabriel Claramunt and In Progress. This PR fixes it.
   Confirm with Gabriel that they have no AG-900 branch, as happened with
   AG-880 and AG-886.
2. The error-banner-under-scrim change (`aea4062`) reverts #244 and is outside
   AG-879. It needs its own ticket for the follow-up the commit names (report
   a dialog's own failure inside the dialog). Suggest filing it under AG-550 or
   AG-879, user's call.
3. AG-882 is To Do and assigned to Gabriel with a close-or-rescope
   recommendation nobody has acted on. Someone should file the gateway ticket
   the comment proposes.
4. Optional check before merge: on staging with a pasted API key (not OAuth),
   confirm what `/v1/me/credits` returns. If it refuses API-key auth, Settings
   now shows "Unavailable" with a Retry that cannot succeed for those users.

## What not to do

- Do not touch AG-880, AG-886, or AG-898 code; those are in #286, #287, #288.
- Do not change `Tray.tsx`'s `CliCard` copy.
- Do not implement AG-889, AG-895 or AG-897; they are parked on a product
  decision.
- Do not post to Jira or GitHub. Everything outward-facing is a draft file.
- Do not add em dashes anywhere, including in commit messages.
