# Correctness review: `feat/traffic-observed-refresh` vs `feat/new-app-ui`

The core scheduler (`traffic_due`) is right on every branch I could construct, including the `spaced`/`overdue` interaction, the boundaries and the note/sweeper race; no starvation, no indefinite delay, no `Instant` panic on the 1.88 toolchain.
One Medium in the window: a report the listener skips because the window is hidden is not recovered by the focus edge when the user comes back inside 30s, so the numbers stay stale in exactly the "terminal beside it" case the feature is for.
Three Lows: a `Once` poisoned by a failed `thread::spawn` would panic every later routed request, `paged` stays set after a failed or refused `loadMore`, and the reopen guard over-reads after a pane change. Everything else checked - payload type, slug vocabulary, non-request call sites, Linux placement, closure freshness - is sound.

## Findings

### M1. A report skipped while hidden is lost if the user returns within 30s

`src/NewUiApp.tsx:655` (`if (document.hidden) return;`) and `src/NewUiApp.tsx:1067` (`if (Date.now() - activityReadAt.current >= ACTIVITY_REOPEN_MIN_MS)`).

The comment at `src/NewUiApp.tsx:650` says a hidden window skips the report because "the focus edge below re-reads on the way back". The focus edge only re-reads when `activityReadAt` is 30s old, and the skip does not touch `activityReadAt`, so the two guards do not compose.

Scenario, with `document.hidden` true (window minimised, or on macOS fully occluded by another window - WKWebView follows `NSWindow.occlusionState`; a full-screen terminal running Claude Code is the everyday case):

1. t=0: window mounts, first reads land, `activityReadAt = 0`.
2. t=5..8: Claude Code sends a burst behind the terminal. Sweeper reports at ~t=13 (`TRAFFIC_QUIET` + tick). Listener sees `document.hidden` and returns; `activityReadAt` still 0.
3. t=20: user switches to the Gate window. `useWindowReopen` fires; `20s < 30s`, no read.
4. Claude Code is idle now, so the sweeper has nothing more to say. The Overview and the Claude pane show pre-burst numbers until the next burst or a later focus edge that happens to be 30s from the mount read.

The stale window is unbounded in the idle case, and the design comment claims the opposite. The `document.hidden` semantics per webview cannot be verified from code; note the failure direction: a webview that over-reports hidden silently drops reports (this bug), one that under-reports spends a read.

Fix: remember that a report was skipped, and let that override the age guard. Concretely, a `missedWhileHidden = useRef<(string|null)[] | null>(null)`; in the listener, on `document.hidden`, merge `e.payload` into it and return; in the `useWindowReopen` callback, `if (missedWhileHidden.current !== null || age >= ACTIVITY_REOPEN_MIN_MS) { const tools = missedWhileHidden.current; missedWhileHidden.current = null; refreshActivityRef.current(tools); }`. Passing the recorded tools keeps the per-tool reads scoped as the live path does. A `visibilitychange` listener would catch un-minimise without a focus change too, but the focus edge is the path the comment already names.

### L1. A failed `thread::spawn` poisons the `Once` and every later routed request panics

`crates/core/src/proxy/mod.rs:715-716`:

```rust
TRAFFIC_SWEEPER.call_once(|| {
    std::thread::spawn(|| loop {
```

`std::thread::spawn` panics when the OS refuses a thread. A panic inside `Once::call_once` poisons the `Once`, and every later `call_once` panics with "Once instance has previously been poisoned". `note_traffic` runs on every `inject_attribution`, on both the engine path (`engine.rs:884` via `apply_rewrite`) and the relay path (`relay.rs:545`), so after one failed spawn every routed request in the process panics in its handler for the rest of the run. Needs thread exhaustion to trigger, hence Low, but the blast radius is the whole data plane.

Fix: `std::thread::Builder::new().name("gate-traffic-sweeper".into()).spawn(...)` and on `Err` log under `debug_log()` and return; the `Once` then completes without a sweeper and the window falls back to the focus edge, which is the behaviour the poisoned-mutex branch at `mod.rs:722` already chooses.

### L2. `paged` stays true after a `loadMore` that fails or is refused

`src/lib/toolEvents.ts:256-260`:

```ts
loadMore: () => {
  if (view?.nextCursor) {
    setPaged(true);
    fetchPage(view.nextCursor);
  }
},
```

`setPaged(true)` runs before `fetchPage`, which returns at `toolEvents.ts:200` when `!enabled || !tool`, and it is not reverted in the `.catch`. The field's doc says "Whether `loadMore` has extended the list past page one"; after a failed page it has not.

Scenario: user clicks Load more, the request 429s on the shared throttle bucket. `paged` is true, the list is still page one, and every traffic report from then on skips `toolEvents.reload()` at `NewUiApp.tsx:635` for this scope. Recovery is the Retry button (`NewUiApp.tsx:3714`, `onRetry={toolEvents.reload}`) or a pane change, so the user has a way out; the cost is a feed that stops following the terminal after one failed page, with nothing on screen saying so.

Fix: set `paged` when the page lands, in the `.then` branch where `cursor && prev`, or revert it in the `.catch` when `cursor !== null`.

### L3. The reopen guard's clock is not advanced by the hooks' own scope-change reads

`src/NewUiApp.tsx:618` sets `activityReadAt` at mount and `src/NewUiApp.tsx:630-631` on each `refreshActivity`. The per-tool hooks re-read on their own when `openTool` changes (`useActivity`'s `useEffect(reload, [reload])`, `useToolEvents`'s `fetchPage(null)` effect), and the org-wide hook when `installFilter`/`credential` change; none of those update `activityReadAt`.

Scenario: mount at t=0; at t=40 the user opens the Codex pane, which reads `toolActivity` and `toolEvents` fresh; at t=45 they alt-tab out and back. The guard sees 45s and re-reads all three surfaces, two of them 5s after they were read. The error is in the spend direction, not the staleness direction, so it cannot show a wrong number; it just spends the throttled budget the guard exists to protect.

Fix, if wanted: `useEffect(() => { activityReadAt.current = Date.now(); }, [openTool, installFilter, credential])` in `NewUiApp`, or leave it and accept the extra read.

## Checked and sound

- **`traffic_due` rules** (`mod.rs:744-760`). Enumerated every combination of `spaced`, `quiet`, `overdue`:
  - Burst then quiet: reported at `last_seen + 5s` (+ up to one 1s tick), then nothing until new traffic (`pending_since` cleared). Pinned by `a_burst_is_reported_once_it_goes_quiet_and_then_not_again`.
  - Continuous traffic: first report at `pending_since + 30s`; after that `pending_since` is re-armed by the next request and `spaced` holds the cadence at `last_reported + 30s`. Because a note's `now` is taken before the lock (`mod.rs:705`), a request that raced the sweep can be stamped slightly before the report instant, which makes `overdue` true a hair early; `spaced` then dominates, so the cadence is exactly 30s plus tick. No starvation: `overdue` guarantees a report within 30s of the oldest unreported request once `spaced` allows, and `spaced` allows at most 30s after the previous report. Pinned by `continuous_traffic_is_reported_on_the_interval_rather_than_never`.
  - Quiet-but-not-spaced, then traffic resumes: `pending_since` stays at the first unreported request, `last_seen` moves, and the report lands at `max(last_reported, pending_since) + 30s` or at the next quiet moment past `spaced`, whichever is first. Never indefinite.
  - Boundaries: both comparisons are `>=`, tests pin `interval - 1` empty and `interval` due.
  - `last_seen >= pending_since` always (same `now`, `pending_since` only set when `None`), so `quiet` cannot be true while `overdue` is computed off a later instant.
- **`Instant` arithmetic.** `rust-toolchain.toml` pins 1.88.0; `Instant::duration_since` saturates since 1.60 and `Option::is_none_or` is stable since 1.82. The sweeper takes `now` under the lock (`mod.rs:719`), so every mark it reads was written by a thread whose `now` preceded the lock handoff; a negative delta cannot occur and would saturate to zero if it did. Two notes racing can leave `last_seen` a sub-millisecond behind the later one; harmless.
- **Observer not yet set.** `note_traffic` returns at `mod.rs:702` before touching the map or the `Once`, so no sweeper thread exists until a shell has registered; requests before registration are not reported and cannot leak into a later batch. `OnceLock` makes the first registration win, as documented.
- **Lock discipline.** `traffic_due` runs under the mutex; the observer is called after the guard is dropped (`mod.rs:727-730`). `note_traffic` releases the lock before `call_once`. Poisoned mutex: sweeper exits, `note_traffic` skips via `if let Ok`, and the completed `Once` prevents a respawn, which matches the comment. The observer runs on the sweeper thread; `AppHandle::emit` is safe from any thread.
- **Tauri payload.** `traffic_handle.emit("traffic-observed", tools)` with `tools: &[Option<&'static str>]` satisfies tauri 2.11.2's `Emitter::emit<S: Serialize + Clone>` (`&[T]` is `Clone`, `[Option<&str>]` is `Serialize`), and serde emits `["claude-code", null]`. `cargo check -p gate-connect-desktop` passes. Matches `listen<(string | null)[]>` at `NewUiApp.tsx:654`.
- **Slug vocabulary.** `openTool` (`NewUiApp.tsx:513-524`) is drawn from `sectionMemberKeys` filtered to installed tools, so it is one of `registry.rs:20-24`: `claude-code`, `codex`, `opencode`, `openclaw`, `hermes`. `client_tool` returns `Client::slug()` for the same five (`mod.rs:1683-1689`, `taxonomy.rs:161-167`), identical strings, and that slug is what goes on `x-gate-client`, so the gateway's per-tool attribution and the pane's `tools.includes(openTool)` use one vocabulary. `chatgpt`, `claude-desktop`, `claude-web` and `chatgpt-web` can appear in the payload, none is ever `openTool`, and they correctly trigger only the org-wide read.
- **Non-request call sites.** `inject_attribution` is reached only through `inject_gate_credential` (`mod.rs:1861`), whose production callers are `apply_rewrite` at `engine.rs:884` (inside the request handler, `Decision::Rewrite` only) and `relay::inject_credential` at `relay.rs:545` (`Route::Rewrite` only, after the `/__gate/health` short-circuit at `relay.rs:518`). `probe_relay_route` (`mod.rs:1048`) GETs only the health path. The passthrough warm-up and direct (non-intercepting) relay mode never inject. Test callers (`inject_attribution_for_tests`, the `engine.rs` test module from 2391) register no observer, so `note_traffic` is a no-op there. A request that fails after `note_traffic` (header build error, upstream refusal) still produces a report; the cost is one read that finds nothing, not a wrong number.
- **Linux.** `relay::spawn` is called from the engine (`engine.rs:2329`) and `relay::serve` is the standalone daemon host (`mod.rs:1100`); both paths live in the helper daemon, which registers no observer, so "never fires on Linux" holds and the focus edge is the only refresh there.
- **Closure freshness.** `refreshActivityRef.current` is reassigned every render (`NewUiApp.tsx:642`), the listener registers once; same pattern as `useWindowReopen`. `activity.reload()`, `toolActivity.reload()` and `toolEvents.reload()` are no-ops while disabled (`activity.ts` `if (!enabled) return;`, `toolEvents.ts:200`), as the comment claims; `toolEvents.reload` still clears `paged` when disabled, harmless.
- **`attempt` generations.** A reply for an earlier scope is dropped in both hooks. A traffic event landing between a scope-change commit and its passive effect calls the new scope's `reload` once, then the effect calls it again: one duplicate read, same scope, first reply dropped. Not a wrong number.
- **`paged` on the other paths.** Cleared on scope change (`toolEvents.ts:237-241`) and on `reload` (`toolEvents.ts:262-263`); the scope-change clear runs in an effect alongside the `view` clear, so no render sees `paged` true with a foreign list. A traffic report arriving while a `loadMore` is in flight sees `paged` true and leaves the feed alone, which is what protects the in-flight page from `reload`'s `attempt++`.
- **Hidden skip and the clock.** The `document.hidden` return at `NewUiApp.tsx:655` happens before `refreshActivityRef.current`, so a skipped report does not advance `activityReadAt` and masquerade as a read (the M1 gap is the opposite: it also leaves no trace).
- **Tests.** `cargo test -p gate-connect-core --lib traffic_tests`: 4 passed. `vitest run src/lib/useToolEvents.test.tsx`: 8 passed.
