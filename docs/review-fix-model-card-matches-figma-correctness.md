# PR #208 review: correctness

Branch `fix/model-card-matches-figma` against `feat/new-app-ui`, 34 commits,
+2343/-280 across 28 files. Reviewed with the correctness lens: logic, edge
cases, nullability, caller contracts, the `null` vs `0` rule, and whether the
new tests pin behaviour rather than restating the implementation.

The core mechanisms hold up. `serve_path` is an exact match with both Codex
spellings covered, and the relay's own route test (`relay.rs:940`) confirms the
short spelling is the one the relay actually sees, so the mapping is real rather
than assumed. The compatibility filter is careful in the direction that matters:
an unknown verdict is never read as a refusal, a served verdict outranks the
local table in both directions, and no verdict depends on the dev pin list.
`modelAttention`'s unbroken-run rule cannot flap on its own (it is a monotone
function of the head of the feed, and one success at the head clears it by
construction), and every unread input still yields `null` rather than a
reassurance. 122 unit tests across the five changed suites pass locally.

Three things do not hold. A merge left `src/main.tsx` with two identical copies
of the global error handlers, so the PR that set out to remove one phantom ERROR
line now writes every real one twice. `modelAttention` attributes any failure run
in the window to Gate serving the model, with no evidence that Gate served those
requests. And the picker can hold a selection it does not draw: a stored model
that is in the catalogue but incompatible is filtered out of the list, is not in
the `missing` rescue list either, and is re-saved on Save.

## H

### 1. Both global error handlers are registered twice - `src/main.tsx:46`

The merge duplicated the block verbatim: handlers at `src/main.tsx:37-44` and
again at `src/main.tsx:54-61`, identical bodies, distinct function objects, so
`addEventListener` keeps both.

```
window.addEventListener("unhandledrejection", (e) => {
```

`logError` (`src/lib/log.ts:30`) is a fire-and-forget `invoke` with no dedup, and
`captureException` likewise. Base has one copy (`git show
feat/new-app-ui:src/main.tsx` has listeners at 37 and 41 only).

Failure scenario: any `void routing.setAppRouted(...)` rejects once. The
diagnostic log gets two identical `unhandled rejection: ...` ERROR lines and
PostHog gets two exception events for one fault. Every error count in this
build's telemetry is doubled, and the log this PR exists to make readable now
double-reports. Directly counter to commit `286358d`'s own goal.

## M

### 2. A refused serve path keeps `x-gate-model` on a BYOK forward - `crates/core/src/proxy/engine.rs:1230`, `crates/core/src/proxy/relay.rs:582`

When `serve_path` returns `None`, the code restores the upstream hint so the
request reaches the tool's own provider. It does not remove the model header that
`inject_model_choice` stamped a few lines earlier. This repo's own contract for
that header (`crates/core/src/proxy/mod.rs:857`) is:

```
/// Unlike the two above this is not a label on the request - it **changes what
/// the gateway serves**, so the gateway rewrites the body's `model` to the first
```

Failure scenario: Claude Code is on Gate model `openai/gpt-5`. It sends
`/v1/messages/count_tokens`, which the new exact-match test
(`crates/core/tests/model_choice_injection.rs`, `the_match_is_exact_...`) pins as
`None`. The request goes out with `x-gate-upstream-url:
https://api.anthropic.com` **and** `x-gate-model: openai/gpt-5`. By the header's
documented behaviour the gateway rewrites the body's `model` to `openai/gpt-5`
and forwards that to Anthropic, which 400s on an unknown model. The claimed
outcome ("the tool reaches its own provider instead of hanging") holds only if
the gateway ignores the model header whenever the upstream hint is present. That
is a gateway-side fact this repo does not encode, and nothing in the two new
tests asserts the header is dropped on the refused path - the passthrough test
asserts only the URI, the hint and the client credential. Either strip the header
on refusal or pin the gateway's behaviour in a test.

### 3. Failures that predate the switch are blamed on the Gate model - `src/lib/modelAttention.ts:123`, `src/NewUiApp.tsx:2275`

`recent` is `toolEvents.view?.entries.slice(0, 5)`, and that feed is scoped by
tool and install only (`activityToolEvents(tool, installId, cursor)` in
`src/lib/api.ts:160`) - not by who served the request and not by model. Nothing
correlates a failure with the moment the preference changed; `ToolModelChoice`
carries no timestamp and `ActivityEntry.time` is never read here.

Failure scenario: the user's own OpenAI key is rate-limited, so Codex logs four
consecutive errors under App default. They switch to a Gate model precisely to
work around it. `choice.source` is now `gate`, `failureStreak` is 4, and the pane
immediately says "Every recent request from this app has failed **while Gate has
been serving it**. The model may not work with this app" about a model that has
served nothing yet. The commit's own test (`fires on the transition`) pins the
mirror-image case and this one is left open, so the module is asymmetric about
the same boundary.

### 4. "Every recent request has failed" is a claim the rule no longer checks - `src/lib/modelAttention.ts:128`

The predicate is now the head run, not the window; the sentence was left as it
was.

Failure scenario: `recent = [error, error, error, success, success]` - the exact
input of the new `fires on the transition` test. Output message: "Every recent
request from this app has failed". Two of the five recent requests succeeded.
This is a statement about the user's own traffic on the screen they would check
it on, which is the class of claim principle 6 asks to be true; "the last three
requests" would be. No test asserts the sentence against the data it describes.

### 5. An incompatible stored model is invisible in the picker and re-saved anyway - `src/components/gc/dialogs.tsx:620`, `:644`, `:658`

`missing` is `draft.filter((id) => !servable.has(id))`, i.e. only ids the
catalogue does not list. `shown` is drawn from `usable`, which excludes anything
`compatibility` refuses. A model that is *in* the catalogue but refused for this
tool therefore appears in neither list, while still counting in `draft`.

Failure scenario: Codex has `openai/gpt-4o` stored (saved before the filter
existed, or via "Show anyway"). Reopen the picker: no row anywhere in the dialog
carries that id, "Unselect all (1)" says something is selected, "Save models" is
enabled, and pressing it writes `["openai/gpt-4o"]` back unchanged. The user can
neither see nor clear the member without first pressing "Show anyway" or wiping
the whole set. This is the same defect commit `8f110ed` fixed for
catalogue-absent models, with a different reason for the row's absence.

## L

### 6. The picker's own model type omits the field the filter reads - `src/components/gc/dialogs.tsx:517`

`GateModelOption` declares `id`, `vendor`, `logo`, `tags` but not `toolShapes`.
It works today only because `NewUiApp.tsx:2048` passes the real `GateModel[]`
through by reference, so the extra property survives at runtime.

Failure scenario: a future call site maps the catalogue to attach `logo`
(`models={models.map(m => ({...m, logo: mark(m.vendor)}))}` is the obvious next
step, since `logo` exists for exactly that). `toolShapes` is not in the target
type, TypeScript reports nothing, every served verdict silently disappears and
the hardcoded fallback table becomes the sole authority again - the state AG-729
was written to end.

### 7. "No model matches that search" is drawn when nothing was searched - `src/components/gc/dialogs.tsx:934`

The empty-list branch has one sentence for two causes.

Failure scenario: `appSlug = "codex"`, catalogue holds only `openai/gpt-4o`,
search box empty. Count line reads "Showing 0 of 0 models・1 in Gate AI", the
set-aside notice above explains the real cause, and the list says the models were
filtered out by a search the user never typed.

### 8. `explain("no-freeform-tools")` claims a test that may not have happened - `src/lib/modelCompatibility.ts:289`

The copy is "These models were tested and verified to reject...", but
`FALLBACK_FREEFORM_RULES` refuses by *family prefix* (`anthropic/`, `google/`,
`deepseek/`) on evidence gathered from a few members on 2026-08-28.

Failure scenario: a gateway that predates `tool_shapes` lists
`anthropic/claude-opus-6`, released after that sweep. It is refused by the
`anthropic/` prefix and the notice tells the user it was tested and verified to
reject Codex's tool shape. Nobody tried it. The module's own doc ("Only refusals
are asserted... an entry nobody can trace is not evidence") argues against this
sentence.

### 9. A single reading-wide gap keeps its section name - `src/lib/activityGaps.ts:141`

The doc says "A group that covers every gap in the reading is attributed to the
reading as a whole", but the `group.length === 1` early return returns `first`
untouched before the `group.length === notices.length` test can apply.

Failure scenario: one section fails with `attribution` and the other four
answered. `notices.length === 1`, `group.length === 1`, subject stays e.g.
"Messages" instead of "Activity". Harmless here (the narrower subject is arguably
better), but it means the "covers every gap" rule is not what the code
implements, and `leaves a single notice exactly as it was` pins the exception
without naming the contradiction.

### 10. Two doc comments now describe the wrong declarations

- `src/NewUiApp.tsx:994-1002`: the merge moved `routeApp`'s doc block above
  `openLink`'s, so `openLink` carries two stacked doc comments and `routeApp`
  (line 1017) has none. Ironic given `8f110ed` fixed exactly this class of drift.
- `src/NewUiApp.tsx:503`: "The pane's 'Current Gate model' row shows the first
  and says how many more there are" - superseded by this PR, which lists the whole
  set (`AppPane.tsx` `ids`) and deleted `alsoEnabled`.

### 11. A failed catalogue read is never retried while the tool is on a Gate model - `src/lib/toolModels.ts:327`

Pre-existing, but this PR adds the comment that blesses it ("Once per session,
not once per opening").

Failure scenario: `openPref.source === "gate"`, so `enabled` is permanently true.
The catalogue fetch fails once (transient offline). `models` stays `null`,
`failure` is set, and the effect's deps (`enabled`, `models`, `reload`) never
change again, so no further read is attempted. Opening the picker shows "Gate
could not list its models... Close this and try again", and closing and
reopening cannot work for the rest of the session. Under App default the same
failure recovers, because `enabled` toggles.

### 12. Test coverage gaps against the claimed behaviour

- Nothing pins the `MIN_FAILURES` boundary from below. `stays quiet on a single
  failure` covers 1; a run of 2 (the value that decides whether the constant is 2
  or 3) is untested, so lowering the threshold to 2 would not fail a test.
- Nothing pins `pinnedModels` returning `[]` outside DEV
  (`modelCompatibility.ts:177`), which is the whole safety argument for keeping
  the table in the tree. The suite tests `knownGoodFor`/`isPinned` only.
- No test asserts the refused-serve-path request drops (or keeps)
  `x-gate-model` - see finding 2.

## Claims verified

1. **`serve_path` maps a request to a route the gateway can serve, or refuses;
   `/codex/responses` maps to `/v1/responses`.** Confirmed.
   `crates/core/src/proxy/mod.rs:970` is an exact match on four paths plus the
   two Codex spellings, and `relay.rs:940`'s pre-existing route test shows the
   relay really does see `/codex/responses` after slug stripping. The engine
   applies it after the upstream-path strip, so the engine sees the same
   gateway-native shape. Both call sites compute the decision identically.
2. **A refused path keeps its upstream hint so the tool reaches its own provider
   instead of hanging.** Confirmed for the hint and the credential
   (`engine.rs:1230`, `relay.rs:582`, and the
   `the_same_request_without_a_gate_model_keeps_its_passthrough_route` test).
   **Not** confirmed for the model header, which stays on the request - finding 2.
3. **The compatibility filter uses catalogue tags plus a hardcoded empirical
   table, and nothing is hidden outright.** Confirmed with one correction to the
   description: the table is no longer "GPT-5 family only for Codex". Only six
   prefixes assert a refusal; everything unmatched is `unknown` and therefore
   offered. The "Show anyway" escape and the set-aside sentence are both present
   (`dialogs.tsx:895`), and `compatibility` never gates saving. Finding 5 is the
   exception to "nothing is hidden": an already-selected incompatible model is
   hidden with no row to clear.
4. **`modelAttention` counts an unbroken run of failures.** Confirmed, and it
   cannot flap: `failureStreak` reads only the head of a newest-first list, so the
   warning appears at three consecutive failures and disappears on the first
   success at the head, with no hysteresis to oscillate. The tests pin the
   transition case, the self-clear, the interleaved case and both unread inputs.
   Two caveats: a tool that fails most requests but not consecutively
   (`[E,S,E,S,E]`) never reaches the threshold, so the warning is silent in a
   genuinely broken case - no regression from the old rule, but not what the
   message implies; and findings 3 and 4 above.
5. **`activityGaps.ts` / `toolModels.ts`: "Current Gate model" hidden under App
   default and pluralized for a set.** Confirmed in `AppPane.tsx` (the section is
   inside `gateActive &&`, the heading pluralizes on `ids.length > 1`, and the
   radio description names the remembered model instead). Note the claim
   misattributes the file: this is `AppPane.tsx`, while the `activityGaps.ts`
   change is the unrelated `mergeNotices` addition.
6. **Credits re-read on window focus.** Partially confirmed.
   `src/lib/toolModels.ts:463` listens on `visibilitychange`, not focus, matching
   the established pattern in `NewUiApp.tsx:743` and `TrayApp.tsx:156`. That fires
   when the window is hidden or minimized, not when it merely loses focus, so the
   motivating scenario in the doc comment (the user runs the tool in a terminal
   and comes back to the still-visible window) is not covered. The `null` vs `0`
   handling around it is right: `formatCredits` returns `null` for an unread
   balance and `$0.00 available` for a real zero, and `plan` now nulls instead of
   defaulting to "free".
7. **A phantom ERROR came from five listener teardowns dropping the unlisten
   promise; the fix awaits/catches them.** Confirmed, and the count is exactly
   five: `App.tsx:469`, `:521`, `:554`, `NewUiApp.tsx:775`, `Onboarding.tsx:266`.
   All five now `.catch(() => {})`. Undercut by finding 1, which doubles every
   remaining log line.
