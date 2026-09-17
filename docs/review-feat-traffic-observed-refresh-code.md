# Review: feat/traffic-observed-refresh vs feat/new-app-ui - code quality

A small, well-documented change: a coalescing traffic observer in the core, one emit in the shell, one listener plus a focus-edge re-read in the window, and a `paged` flag so a self-initiated feed refresh does not pull pages out from under the reader. No must-fix findings. Two should-fix: a doc comment orphaned by the insertion point of `ACTIVITY_REOPEN_MIN_MS`, and Rust tests that re-implement `note_traffic`'s recording step instead of calling it. The rest is nits about redundancy and comment duplication.

Verified: `cargo test -p gate-connect-core --lib traffic_tests` (4 pass), `cargo clippy -p gate-connect-core --all-targets -- -D warnings` (clean), `pnpm vitest run src/lib/useToolEvents.test.tsx` (8 pass), `tsc --noEmit` (clean). No em dash in the diff.

## H (must-fix)

None.

## M (should-fix)

### M1. `ALL_MISSING`'s doc comment is now orphaned
- `src/NewUiApp.tsx:3823-3829`
- What: the new constant and its docblock were inserted between the existing `/** Before a reading lands, no section has one. Kept out of the render ... */` comment and `const ALL_MISSING`. The result is two consecutive `/** */` blocks over `ACTIVITY_REOPEN_MIN_MS`, and `ALL_MISSING` has no comment at all. Editors and TypeDoc attach the first block to the wrong symbol.
- Why: the orphaned comment describes stable object identity for a render-time object, which has nothing to do with a 30s threshold; a reader will trust the wrong explanation.
- Fix: move `ACTIVITY_REOPEN_MIN_MS` and its doc above the `ALL_MISSING` docblock (or below `EMPTY_STATS`), so each comment sits on its own symbol.

### M2. The Rust tests re-implement `note_traffic`'s recording rather than calling it
- `crates/core/src/proxy/mod.rs:703-713` (the recording in `note_traffic`), `:770-785` (`seen()` in `traffic_tests` duplicates it), `:823-826` (the third test duplicates it again inline)
- What: the `entry(tool).or_insert(TrafficMark{..})`, `last_seen = now`, `pending_since.get_or_insert(now)` sequence exists three times. The tests exercise `traffic_due` against marks they built themselves, so a future change to how `note_traffic` records (say, resetting `pending_since` on a new burst) would leave the tests green while testing a shape the production code no longer produces.
- Why: `traffic_due` is documented as "pure, so the timing is testable without a thread"; the recording half deserves the same treatment, and it is the half that decides what `pending_since` means.
- Fix: extract `fn mark_traffic(seen: &mut TrafficSeen, tool: Option<&'static str>, now: Instant)` holding the recording, called from `note_traffic` under the lock and from the tests in place of `seen()` and the inline copy. This also folds the redundant `last_seen: now` followed by `mark.last_seen = now` (L2 below) into one place.

## L (nit)

### L1. `paged` is set before the page lands, so a failed "load more" pins it true
- `src/lib/toolEvents.ts:262-265`, doc at `:191-195`
- What: `loadMore` calls `setPaged(true)` and then `fetchPage`. If that page fails, the list is still page one but `paged` is `true`, so every traffic-observed refresh skips the feed for that scope until a scope change or the user's own reload. The field's doc says "has extended the list past page one", which is then untrue.
- Fix: either set `paged` in `fetchPage`'s `.then` when `cursor` is non-null, or reword the doc to "has asked for a page past the first". The first matches the stated intent (protect pages the user is reading); the second is one line.

### L2. Redundant double assignment on insert
- `crates/core/src/proxy/mod.rs:707-713`
- What: `or_insert(TrafficMark { last_seen: now, .. })` followed unconditionally by `mark.last_seen = now`. Correct, and the shape is the same one the tests copy. Goes away with M2's extraction; `entry().and_modify(..).or_insert(..)` is the idiom if it stays inline.

### L3. Unused derives on `TrafficMark`
- `crates/core/src/proxy/mod.rs:672`
- What: `Clone, Copy, PartialEq, Eq, Debug` are all derived; none is used. `due` is a `Vec<Option<&str>>`, so the debug log at `:726` never formats a mark, and the tests compare `due` vectors, not marks. Harmless on a private type; `Debug` alone is the honest set.

### L4. The sweeper re-fetches the observer on every tick
- `crates/core/src/proxy/mod.rs:728-730`
- What: the thread is only ever spawned from `note_traffic` after `TRAFFIC_OBSERVER.get().is_none()` returned false, and a `OnceLock` never unsets, so the `if let Some(observer)` inside the loop cannot be `None`. Neighbours (`:354`, `:578`) use `let Some(observer) = X.get() else { return };` once at the top.
- Fix: take `let Some(observer) = TRAFFIC_OBSERVER.get() else { return };` at the top of `note_traffic` (the neighbouring let-else style) and move the `&'static` reference into the spawned closure.

### L5. Test module placement differs from the rest of the file
- `crates/core/src/proxy/mod.rs:764` vs `:2825`
- What: `mod traffic_tests` is the only inline `#[cfg(test)]` module in a 4700-line file whose other tests all live in `mod tests` at the bottom. Reasonable to keep it beside the code it tests, but say so in one line or move it; a reader looking for tests will look at the bottom first.

### L6. The throttle rationale is written in four places
- `crates/core/src/proxy/mod.rs:630-636`, `src-tauri/src/lib.rs:4690-4700`, `src/NewUiApp.tsx:643-651`, `src/lib/activity.ts:410-420`
- What: "the endpoint is throttled per source address, so a poll would spend a shared budget" appears in full in all four. `useActivity`'s doc already owns it and the core doc points there. The `lib.rs` comment is the one that could shrink to "see `proxy::TRAFFIC_OBSERVER` and `useActivity`" plus the two facts that are local to it (payload shape, not window-gated, never fires on Linux). Comment hygiene only.

### L7. `ACTIVITY_REOPEN_MIN_MS` is a cross-language duplicate of `TRAFFIC_REPORT_INTERVAL` with no name link
- `src/NewUiApp.tsx:3827` vs `crates/core/src/proxy/mod.rs:665`
- What: the doc says "the same spacing the relay's traffic reports keep" without naming the constant. Someone changing `TRAFFIC_REPORT_INTERVAL` will not find this with `rg`. Name it in the comment.

### L8. `activityReadAt` is only advanced by `refreshActivity`, not by the hooks' own re-reads
- `src/NewUiApp.tsx:613-616`, `:1066-1068`
- What: opening an app pane makes `toolActivity` and `toolEvents` read fresh (their scope changed), but the ref is not touched, so the doc's "starts at mount, which is when the hooks above do their first read" is only true for the first read. Effect: a focus edge shortly after opening a pane re-reads numbers that are seconds old. One wasted read at most; noting it because the doc overstates what the guard tracks.

### L9. No test on the window side for the routing decisions
- `src/NewUiApp.tsx:625-631`
- What: the `tools === null || tools.includes(openTool)` filter, the `paged` guard and the `document.hidden` skip are untested. `NewUiApp` has no test file and none of the neighbouring listeners are tested either, so this is consistent with the file, not a regression. If a test is wanted, the filter is a two-line pure function that could live in `lib/activity.ts` beside `useActivity` and be tested there. The Rust side also has no test with a `None` tool key (an unnamed sender), which is a real branch of the payload.

## What is fine

- **Consistency with the neighbouring observers.** `set_traffic_observer` mirrors `set_cf_challenge_observer` / `set_gate_auth_observer` exactly (`OnceLock`, first-registration-wins, same doc sentence). The "not cfg-gated, no-op on Linux" reasoning is stated once in the core and referenced from the shell, matching the auth observer's comment at `lib.rs:4664-4675`.
- **Coalescing lives in the right layer.** The core owns the burst/quiet/interval logic; the shell emits as-is; the window does not debounce. Each layer says which of the others owns what, and the claims agree (30s, one report per tool, after quiet).
- **The single hook point is real.** `inject_attribution` is reached from both the loopback relay (`relay.rs:703`) and the MITM engine (`engine.rs:1637`) through `inject_gate_credential`, once per request each. The app's own activity reads use a `.no_proxy()` client (`crates/core/src/activity.rs:12`, `gateway_api.rs:134`), so a refresh cannot re-trigger itself through the relay.
- **Error handling at the boundaries.** `TRAFFIC_SEEN.lock()` failures are handled on both sides: `note_traffic` drops the sample, the sweeper exits rather than spinning, and the comment says what covers the gap (the focus edge). `emit` failure is ignored, as every neighbouring emit is. `unlisten` rejection is swallowed with the same idiom and for the reason the `proxy-state-changed` listener records at `NewUiApp.tsx:975-980`.
- **The latest-callback ref pattern** for `refreshActivityRef` matches `openSwitchOrgRef` (`:1810-1811`) and `useWindowReopen`'s own `callback` ref, and the comment gives the specific race (`listen()`/`off()` both async) that the `switch-org-requested` listener's comment documents at length. Subscribing once with `[]` is the file's established answer.
- **`document.hidden` skip** matches the `tools-changed` listener's guard at `:937`.
- **`paged` is reset on scope change** (`toolEvents.ts:240`) as well as on `reload`, so a pane switch cannot inherit a stale `true`.
- **The new vitest test** drives the hook through the real harness, checks both transitions, and asserts the list is back to page one after `reload`, which is the property the flag exists for.
- **`traffic_due` is genuinely pure**, and the four tests cover the four documented rules: quiet-then-once, cadence under continuous traffic, never closer than the interval, and per-tool independence.
- **`is_none_or`** (`mod.rs:752`) is within the pinned toolchain (`rust-toolchain.toml` 1.88.0, `rust-version = "1.88"`); it is already used in `lib.rs:2002`.
- **Sweeper thread lifecycle** (started lazily by `Once`, permanent thereafter) is a departure from the per-fire threads the auth observer spawns, but it is the right shape for a periodic sweep and the doc says why it starts late. A panic inside the observer would end the thread for good, but the only thing it calls is `emit`, which returns a `Result`.
- **Naming** is clear throughout: `note_traffic` / `traffic_due` / `TrafficMark` / `pending_since` / `last_reported` read as the doc describes them; `paged` is the right word for the flag.
- **No em dashes** anywhere in the diff.
