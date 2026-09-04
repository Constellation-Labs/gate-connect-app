# PR #208 review - code quality lens

Branch `fix/model-card-matches-figma` against `feat/new-app-ui`, 28 files,
+2343/-280.

The branch is in good shape mechanically: `pnpm test` is green (710 tests, 46
files), `tsc --noEmit` is clean, `cargo fmt --all --check` and
`cargo clippy -p gate-connect-core --all-targets -- -D warnings` both pass. The
new modules (`modelCompatibility`, `modelAttention`, `activityGaps.mergeNotices`,
`serve_path`) each land with real unit coverage, and the e2e picker spec grew
to cover search, sets, credits, the attention causes and AG-729's tool shapes.

What the diff does carry is a layer of merge and refactor residue that the test
suite cannot see: a global error handler registered twice, a doc comment that
migrated onto the wrong function, a stale prop comment left behind next to its
replacement, and - the one with a user-visible consequence - five new uses of a
Tailwind radius class that does not exist. Two token rules in CLAUDE.md are
broken in the file this PR is named after. Findings below are grouped by
severity and each was checked against the code as it stands on HEAD.

## H

### `src/main.tsx:46`

The `error` and `unhandledrejection` listeners, and the 8-line comment above
them, are registered a second time verbatim (lines 29-44 then 46-61).

Every uncaught error and rejection is now sent to `captureException` twice and
written to the diagnostic log twice. This defeats the stated purpose of the
block ("the local log is the one you can open") by doubling every entry in it,
and it double-counts frontend errors in PostHog. Given commit `0c5f019` is a
merge of `origin/feat/new-app-ui`, this is almost certainly a conflict
resolution that kept both sides.

### `src/components/gc/dialogs.tsx:710`, `:727`, `:734`, `:875`, `:895`

`rounded-base` is not a defined radius, so all five (plus the pre-existing
`:912`) emit no CSS and render square.

Verified by running the project's own Tailwind against these class names:
`rounded-control`, `rounded-lg`, `text-base-2xs`, `tracking-label` and
`shadow-base-xs` all emit rules; `rounded-base` emits nothing. There is no
`base` key in `borderRadius` in `tailwind.config.ts` (the scale is
`none/xs/control/sm/md/lg/xl/2xl/3xl/4xl/full` plus `DEFAULT`, `gc-lg`,
`gc-pill`). The base branch had one such use; this PR adds five more. The
comments beside them assert the opposite of what ships: line 706 says the
selected row "tightens its radius against the looser one the other rows carry"
(it is 0 vs 10px), and line 722 says the mark is "Square, as the frame draws it"
while the class was meant to soften it. On a PR titled "match the card to
Figma" this is the one class worth getting right.

## M

### `src/components/gc/AppPane.tsx:621`

The icon frame renders unconditionally, so a multi-model row draws an empty
32px bordered square where the vendor mark was deliberately dropped.

`icon` became optional for exactly this case (`:599`, "Omitted for a row about a
set, where no one mark could stand for it") and `:475` passes `undefined` once
`ids.length > 1`, but the wrapping `<span class="flex size-8 ... border
border-base-border">` around `{icon}` has no guard. The result is a bordered
empty box, which is not "omitted" - it is a mark that failed to load. If the
32px indent is wanted for alignment, the comment should say so and the border
should go; otherwise the span needs the same conditional as its contents.

### `src/NewUiApp.tsx:994`

`routeApp`'s doc comment was moved onto `openLink`, which now carries two
stacked JSDoc blocks, and `routeApp` (`:1018`) has none.

The diff removes the comment from above `routeApp` and re-adds it above
`openLink`, so `openLink` - a three-line `openExternal` wrapper - is documented
as being about config writes, restarts and killing running processes. The
paragraph about scoping to `slug` (which is the behaviour `routeApp:1023`
implements) now describes a function that has no slug.

### `src/components/gc/dialogs.tsx:717` and `:969`

`text-neutral-900` for body ink, against CLAUDE.md's explicit rule.

CLAUDE.md: "Ink is `base.foreground #030712`, not `neutral-900`." Both
occurrences are new on this branch (the base branch has zero in this file), and
`:794` - the sibling `ModalNote` heading twenty lines up, in the same role -
correctly uses `text-base-foreground`. So the file now renders the same element
two different colours.

### `src/components/gc/dialogs.tsx:742`

`const chosen = draft;` is a single-use alias declared 49 lines after its only
reader.

The only consumer is `renderRow` at `:693` (`chosen.includes(model.id)`), which
is defined above the declaration and works only because it is invoked later,
from the JSX. Any refactor that calls `renderRow` earlier in the body turns this
into a TDZ `ReferenceError`, and the alias buys nothing over reading `draft`
directly, which the rest of the component already does (`:758`, `:773`, `:877`).

### `src/components/gc/dialogs.tsx:517`

`GateModelOption` omits `toolShapes`, the field the compatibility logic reads.

`compatibility()` -> `toolShapeVerdict()` (`modelCompatibility.ts:227`) reads
`model.toolShapes?.[shape]` and that served verdict is the whole point of
AG-729, but the picker's declared model type carries only `id`, `vendor`,
`logo`, `tags`. It works today purely because `NewUiApp:2048` passes
`GateModel[]` through structurally. Any call site that builds a literal of the
declared type - a future unit test, a second caller - silently loses the
gateway's verdict and falls back to the dated `FALLBACK_FREEFORM_RULES` table,
with nothing failing to say so. The two types are otherwise near-duplicates;
`GateModelOption` could be `GateModel` plus `logo`.

### `crates/core/src/proxy/relay.rs:568` and `crates/core/src/proxy/engine.rs:1213`

The serve decision and the path rewrite are implemented twice, and only one copy
is tested.

Both sites compute `serves_gate_model(...) ? serve_path(path) : None` and then
rebuild the destination, in different idioms (relay splits
`routed.path_and_query` on `?` by hand; the engine goes through
`Uri::into_parts`). The comments in both acknowledge the coupling ("See the
relay's copy of this branch; the two paths must agree"), which is the argument
for a shared helper rather than for a comment. The new integration test
`the_rewrite_applies_the_serve_route` exercises the engine path only, and
`apply_rewrite_for_tests` (`proxy/mod.rs:1030`) hardcodes `BillingMode::Byok`,
so neither the relay copy nor the PAYG branch is covered.

### `src/components/gc/dialogs.tsx:554-986`

`ModelPickerDialog` is one 430-line component: 6 pieces of state, 6 memos, an
inline row renderer, an inline unavailable-model list and every branch of
loading/failure/empty/no-match/list.

The row renderer (`:692-739`) and the missing-model list (`:903-921`) are both
self-contained and would read better as siblings of `GateModelOption`, the way
`InfoRow` and `ModelOption` were split out of `AppPane`. It is the largest
component in the file by a factor of three.

### `src/components/gc/AppPane.tsx:602`

Two consecutive JSDoc blocks on the `actions` prop; the first is the orphaned
doc of the removed singular `action`.

"Omitted when there is nothing to do here, which removes the button rather than
leaving a dead one on screen" is followed immediately by "A list because the
credits row carries two...". Only the second describes the prop that exists.

### Missing test: the "no URL, no button" rule

`AppPane.tsx:510-516` cites AG-592 for withholding **Manage billing** when the
gateway names no destination, but nothing asserts it. `grep` finds "Manage
billing" only in `AppPane.tsx` - not in `AppPane.test.tsx`, not in the e2e
specs. `adaptBillingUrl` itself is thoroughly tested
(`toolModels.test.ts:246-272`), so the gap is exactly at the UI boundary where
the rule lives. One assertion each way in `AppPane.test.tsx` would close it; the
new plan-line tests right beside it are the template.

## L

### `src/components/gc/dialogs.tsx:889`

The set-aside sentence and `explain()` disagree on number in both directions:
`1 model is not shown: These models were tested and verified to reject...`
(plural predicate, singular subject) and `12 models are not shown: Gate does not
list tool support for this model...` (`modelCompatibility.ts:287`, singular
inside a plural count). One of the two combinations always reads wrong.

### `src/components/gc/dialogs.tsx:628` and `:641`

Near-verbatim duplicate comment ("Computed over the whole catalogue rather than
the filtered view..."). The copy at `:628` sits over `needs`, which is a
`needsOf(appSlug)` lookup that filters nothing, so it does not apply there; the
copy at `:641` over `usable` is the one that does.

### `src/components/gc/dialogs.tsx:644` and `:651`

`compatibility()` is recomputed over the whole catalogue in two separate memos
with the same dependencies. One memo returning `{usable, asideReason}` would
halve the work on the 344-model list the doc keeps citing.

### `src/components/gc/AppPane.tsx:374`

"Kept as one named value because three places below depend on it" - only two
remain (`:408`, `:461`). The third went with the deleted `muted` prop.

### `src/components/gc/AppPane.tsx:619`

Leftover dangling `>` on its own line, from collapsing the template-literal
className back to a string. There is no `lint` or `format` script in
`package.json`, so nothing catches this class of residue.

### `src/components/gc/AppPane.tsx:506`

`<div className="mt-2 flex flex-col gap-2">` now wraps a single `InfoRow`; the
`flex-col gap-2` is vestigial since the model row moved into its own container
at `:467`.

### `src/components/gc/dialogs.tsx:1043` duplicates `src/components/gc/AppPane.tsx:492`

The stacked-mono-id list is written twice, and the dialog copy also hand-rolls
`ModalSubject`'s frame ("The same row, holding a set") rather than extending the
component. Three places now have to agree on how a set of ids is drawn.

### `src/lib/modelCompatibility.ts:104`

Inside the `KNOWN_GOOD` doc block, the blank line before "Deliberately not a
product claim" is missing its leading `*`, so the block is visually two comments
in an editor.

### `src-tauri/src/lib.rs:2165`

The new `only` paragraph is appended to the `(async)` paragraph with no `///`
separator, so it reads as part of the reason the command is async.

### `src/lib/toolModels.ts:97` and `src/components/gc/AppPane.tsx:30`

Two exported types named `GateModel` with unrelated shapes (a catalogue row vs a
card view model whose field is `ids`). Nothing imports both today, so this is
latent, but the divergence widened here (`id` -> `ids`).

### `ci/dev-app.sh:1-30` and `CLAUDE.md:186`

Every sentence of the rationale is about macOS keychain ACLs, but the script
exports `GATE_CONNECT_TEST_SECRETS` unconditionally, so Linux and Windows
developers also get plaintext file-backed secrets to solve a prompt they never
saw. Worth either a platform guard or one sentence saying the trade is taken
everywhere for uniformity.

### `.gitignore:32`

`feature-plans/` names a directory that does not exist in the tree (the tracked
plan lives at `plans/new-app-ui-figma.md`).

### `src/components/gc/dialogs.tsx:905`

`missing.map((id) => { return (...) })` - block body and `return` for a single
expression, unlike every other `.map` in the file.

## What's good

- The Rust `serve_path` work is the strongest part of the diff: a single
  exhaustive `match`, and tests that pin identity mappings, both Codex
  spellings, the `None` guard and - the one that matters - that the match is
  exact so `/v1/messages/count_tokens` is not swept in.
- `modelCompatibility.ts`'s "only refusals are asserted" rule is applied
  consistently (`unknown` never becomes `fails`), the gateway verdict outranks
  the compiled table, the fallback carries a delete-me condition and a date, and
  the `pinnedModels` `import.meta.env.DEV` gate means a stale dev pin can never
  reach a user.
- `mergeNotices` grouping on cause **and** action kinds, rather than cause
  alone, is the right call and the test file says why.
- `modelAttention`'s failure-streak tests cover the two cases that make or break
  it: firing on the transition where older successes predate the switch, and
  self-clearing on one success at the head.
- Nothing in the diff violates the em dash rule, the `bg-blue-*`/`text-blue-*`
  rule, or the "no px font sizes" rule; `modelCompatibility.test.ts:201` even
  asserts the copy rule mechanically.
- The `AppPane` model-selection change correctly keeps intent and observation
  apart: `gateActive` is derived from `choice`, never from a reading.
