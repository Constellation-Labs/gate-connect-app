# PR #208 review, round 2: fix verification

Base `origin/feat/new-app-ui`, head `fix/model-card-matches-figma`
(+2444/-284 across 27 files). Round 1 reviewed `…HEAD~1` (+2343/-280, 28
files); the only code no reviewer had seen is `1fe71a5 fix(models,proxy):
address review on app#208` (9 files, +156/-59). This round checks each round-1
finding against HEAD and reviews `1fe71a5` on its own merits.

Verified green at HEAD: 710 unit tests in 46 files, `tsc --noEmit` clean, and
19 injection tests including the new refused-path assertion.

## Verdict

The seven findings the commit message claims are fixed are all genuinely
fixed, and the two that carry weight are fixed correctly. The `x-gate-model`
strip is right in both call sites - I checked the relay hunk specifically
because the diff context makes it look like it lands in the served branch, and
it does not: `if mode == BillingMode::Byok && model_serve_path.is_none()`
(`relay.rs:581`) is the forward branch, and the relay dials the gateway in both
branches, which is why both `format!` calls name `gateway_base`. The `rounded-base`
sweep is complete - zero occurrences remain anywhere in `src/`, including the
one that predated the branch.

Two things to fix before merge, both introduced by the fix commit itself: the
new `useCredits` teardown drops the `.catch(() => {})` that all five siblings
carry, which re-opens the phantom-ERROR class this branch exists to close; and
neither of the two behavioural fixes (the `missing` rescue list, the attention
copy) is pinned by a test, so both can be reintroduced silently. Round-1
finding 5 was already this trap arriving through a second door.

Nothing from round 1 was made worse. Everything not listed as fixed below is
untouched, not regressed.

## New findings in `1fe71a5`

### M - the new focus teardown re-opens the unhandled-rejection class this branch closed

`src/lib/toolModels.ts:488-491`

```
return () => {
  cancelled = true;
  void pending.then((unlisten) => {
    if (cancelled) unlisten();
  });
};
```

No `.catch()`. Every other listener teardown in the app has one, added by
`286358d` for exactly this reason: `useWindowReopen.ts:32`
(`void unlisten.then((f) => f()).catch(() => {})`), `App.tsx:298`,
`NewUiApp.tsx:766`, `:783`, `:843`. Round 1's claim 7 verified the count as
five and all five as fixed. This is a sixth teardown that does not.

Failure scenario: `getCurrentWindow().onFocusChanged(...)` rejects - the IPC
seam that `286358d` found rejecting in the first place. `void pending.then(...)`
returns a derived promise nobody catches, so the rejection reaches
`window.addEventListener("unhandledrejection")` (`src/main.tsx:41`), which
writes an ERROR line to the diagnostic log and sends a PostHog exception. A
credits pane unmount then produces the phantom ERROR that this branch's own
commit removed, in the log that same commit exists to make readable.

Fix: `void pending.then((unlisten) => unlisten()).catch(() => {})`, which also
disposes of the finding below.

### L - `cancelled` is a guard over nothing

`src/lib/toolModels.ts:474`, `:488-490`

`cancelled` is written `true` on the line immediately above its only read, and
is read nowhere else, so `if (cancelled)` is always true and `unlisten()`
always runs. It reads as protection against an unmount-before-resolve race,
and there is no such race to protect against: the `.then` is attached inside
the cleanup, so it can only ever run after the unmount. `useWindowReopen`
solves the same problem in one line without the flag, and is cited in the
comment three lines above as the pattern being followed.

### M - neither behavioural fix is pinned by a test

The commit adds one test, for the Rust header strip. The two frontend fixes
that change what a user sees have none.

- **The `missing` rescue list** (`dialogs.tsx:651-658`). The e2e picker spec
  covers the catalogue-absent case (`e2e/new-ui-model-picker.spec.ts:612`,
  "lets an unavailable model be removed, which nothing else could") and the
  "Show anyway" escape (`:793`), both of which predate the fix. Nothing covers
  the case the fix exists for: a chosen model that *is* in the catalogue and is
  filtered out of `usable`, rendering as an Unavailable row. Round 1 called
  this "the same trap, reintroduced through a different door", and the
  commit's own doc comment says so too. A third door closes silently.
- **The attention copy** (`modelAttention.ts:128`). The three failing-traffic
  tests assert `cause` only (`modelAttention.test.ts:141`, `:154`, `:175`);
  no test reads `message`. The sentence that round 1 found false against its
  own predicate can drift back with the suite green. Round-1 finding 4 asked
  for this assertion by name.

The natural place for the first is `new-ui-model-picker.spec.ts` beside `:612`;
for the second, one `expect(a?.message).toContain(...)` at `:141`.

### L - the `useCredits` docstring still describes visibility, not focus

`src/lib/toolModels.ts:428-429`

The body of the doc was updated to "Re-read when the window regains focus", but
the closing sentence still reads "Costs nothing while the window is hidden, and
one read on return." Under `onFocusChanged` the idle case is unfocused, not
hidden, which is the whole distinction the fix turns on. "while the window is
not focused" finishes the change.

### L - `missing` is now `showAll`-aware; the count beside it is not

`src/components/gc/dialogs.tsx:627` (`setAside`), `:899-913`

`setAside = models.length - usable.length` ignores `showAll`, so under "Show
anyway" the line reads "12 models are not shown: …" directly beside a button
labelled "Hide them", with all twelve on screen below it. Pre-existing on the
branch and not caused by the fix, but the fix made the neighbouring `missing`
memo `showAll`-aware and left this one behind, so the two now disagree about
the same state. Round 1 flagged only the singular/plural mismatch in this
sentence (code lens, `:889`).

Also residual rather than new: under `showAll` **plus** an active search or
vendor filter that excludes it, a chosen incompatible model again has no row
and no rescue entry, since `reachable` is the whole catalogue there while
`shown` is filtered. Identical to the pre-fix behaviour in that corner, so the
fix is strictly an improvement; worth knowing that the memo's stated rule
("narrowing the view must not make a chosen model look unavailable") holds only
while `showAll` is false.

## Round 1 findings: status at HEAD

### Correctness lens

| # | Finding | Status |
|---|---------|--------|
| 1 (H) | Both global error handlers registered twice | **Fixed.** One `error` and one `unhandledrejection` listener remain (`main.tsx:37`, `:41`) |
| 2 (M) | `x-gate-model` survives the refused serve path | **Fixed** in both call sites (`engine.rs:1235`, `relay.rs:589`) and pinned by `an_unservable_path_is_forwarded_without_claiming_to_be_served` |
| 3 (M) | Failures predating the switch blamed on the Gate model | **Open.** `recent` is still the tool+install feed with no correlation to when the preference changed, and the copy still says "while Gate has been serving it" |
| 4 (M) | "Every recent request has failed" overstates the rule | **Fixed** in copy (`modelAttention.ts:128`), still unasserted - see above |
| 5 (M) | Incompatible stored model invisible and re-saved | **Fixed**, keyed on reachability, untested - see above |
| 6 (L) | `GateModelOption` omits `toolShapes` | **Open** (`dialogs.tsx:517-529`: `id`, `vendor`, `logo`, `tags`) |
| 7 (L) | "No model matches that search" with no search | **Open** (`dialogs.tsx:948`) |
| 8 (L) | `explain("no-freeform-tools")` claims a test that may not have happened | **Open** (`modelCompatibility.ts:289`) |
| 9 (L) | Single reading-wide gap keeps its section name | **Open.** `group.length === 1` still returns before the `=== notices.length` test (`activityGaps.ts:141-144`) |
| 10 (L) | Two stranded doc comments | **Half fixed.** `routeApp`'s block is back on `routeApp` (`NewUiApp.tsx:1009`) and the orphaned `action` doc is gone (`AppPane.tsx:599`). `NewUiApp.tsx:503` still says the row "shows the first and says how many more there are" |
| 11 (L) | Failed catalogue read never retried | **Open.** `reload` is `useCallback([enabled])` and the effect's deps cannot change after a failure (`toolModels.ts:323-330`) |
| 12 (L) | Test gaps: `MIN_FAILURES` from below, `pinnedModels` outside DEV, refused-path header | **One of three.** The header is now pinned. No `MIN_FAILURES` reference in `modelAttention.test.ts`; no `pinnedModels` reference in `modelCompatibility.test.ts` |

### Code lens

**Fixed:** duplicated error handlers; all six `rounded-base` uses (now
`rounded-md` on rows and buttons, `rounded-sm` on controls, per CLAUDE.md, and
zero remaining in `src/`); `routeApp`'s doc comment; the orphaned `action` doc
on `AppPane`.

**Open, M:** the unguarded 32px icon frame still draws an empty bordered square
for a multi-model row (`AppPane.tsx:619-624` - the `<span>` around `{icon}`
has no conditional); `text-neutral-900` for body ink at `dialogs.tsx:731` and
`:983`, twenty lines from a sibling using `text-base-foreground`;
`const chosen = draft;` at `:756`; `GateModelOption` missing `toolShapes`; the
serve decision implemented twice with only the engine copy tested
(`apply_rewrite_for_tests` still hardcodes `BillingMode::Byok`, so the relay
copy and the PAYG branch remain uncovered - and the relay is where the branch
condition is subtler); `ModelPickerDialog` still one component, now ~450 lines;
no test for the "no URL, no Manage billing" rule (`grep` still finds the string
only in `AppPane.tsx:510`, `:515`).

**Open, L:** all of them, unchanged - the plural/singular mismatch in the
set-aside sentence; the near-verbatim duplicate comment at `dialogs.tsx:609`
over `needs`, which filters nothing, and `:623` over `usable`, which does; two
memos recomputing `compatibility` over the catalogue with the same deps (`:626`,
`:663`); "three places below depend on it" where two remain
(`AppPane.tsx:374`, readers at `:408` and `:461`); the dangling `>` at
`AppPane.tsx:617-618`; the vestigial `flex-col gap-2` at `:506`; the twice-written
stacked-mono-id list; the missing `*` in the `KNOWN_GOOD` block
(`modelCompatibility.ts:104`); the `only` paragraph still run into the `(async)`
one (`src-tauri/src/lib.rs:2165`); two `GateModel` types
(`toolModels.ts:98`, `AppPane.tsx:30`); `dev-app.sh` exporting
`GATE_CONNECT_TEST_SECRETS` unconditionally under an all-macOS rationale
(`:54`); `feature-plans/` in `.gitignore` naming an absent directory;
the block-bodied `missing.map` (`dialogs.tsx:918`).

### Security lens

| Finding | Status |
|---------|--------|
| M - `x-gate-model` survives the fallback to BYOK | **Fixed.** Removed in the forward branch of both paths, with the reasoning inline and a test |
| M - served requests keep the client's `chatgpt.com` cookies; `cf_clearance` injected after the rewrite | **Open.** `CLIENT_AUTH_HEADERS` is still `["authorization", "x-api-key"]` (`mod.rs:1158`); the injection still runs post-`apply_rewrite` |
| L - serve rewrite widens the UA-spoofable spend surface | Open by design; accepted loopback posture |
| L - `adaptBillingUrl` accepts `http:` | **Open** (`toolModels.ts`: `parsed.protocol === "https:" \|\| parsed.protocol === "http:"`) |
| L - dev secret files are 0644 inside the 0700 directory | **Open.** `keychain.rs:86` is still a bare `std::fs::write` |
| L - dev script accepts any URL | Open by design; dev-only script |

The one security finding with real weight is fixed. The cookie/`cf_clearance`
one is the only M left, and it is the same call it was in round 1: exposure is
to the first-party gateway, and the ordering predates the PR while this PR is
what makes the route complete.

## Commands run

```
git rev-list --left-right --count origin/feat/new-app-ui...HEAD   # 0 35, merge base = their tip
git diff origin/feat/new-app-ui...HEAD --stat                    # 27 files, +2444/-284
git diff origin/feat/new-app-ui...HEAD~1 --stat                  # 28 files, +2343/-280 (round 1's scope)
git show 1fe71a5
pnpm test                                                        # 46 files, 710 tests, all pass
npx tsc --noEmit                                                 # clean
cargo test -p gate-connect-core --test model_choice_injection    # 19 pass
grep -rn 'rounded-base' src/                                     # none
```
