# Security review: feat/traffic-observed-refresh (vs feat/new-app-ui)

Nothing on this branch touches credential handling, and nothing user- or network-controlled reaches the event payload: the only value a request can influence is which of ten fixed `Option<&'static str>` keys it is counted under. The amplification bound the design leans on holds (worst case 24 throttled reads a minute against a 100/min bucket, and only with every client kind the classifier knows active at once). The one real fragility is that the sweeper thread is spawned inside `Once::call_once` on the proxy's request path, so a single failed `thread::spawn` would poison the `Once` and turn every later gateway-bound request into a panic.

Reviewed: `git diff feat/new-app-ui...HEAD` (6 files, +352/-2). Tests run: `cargo test -p gate-connect-core --lib traffic_tests` (4 pass), `vitest run src/lib/useToolEvents.test.tsx` (8 pass).

## Findings

### High

None.

### Medium

None.

### Low

#### L1. Sweeper spawn inside `Once::call_once` on the request hot path: one failed spawn poisons every later request

- `crates/core/src/proxy/mod.rs:715-716`

```rust
TRAFFIC_SWEEPER.call_once(|| {
    std::thread::spawn(|| loop {
```

**What.** `std::thread::spawn` panics if the OS refuses the thread. A panic inside the `call_once` closure poisons the `Once`, and every subsequent `call_once` on a poisoned `Once` panics. `note_traffic` is called from `inject_attribution` (`mod.rs:1464`) on every gateway-bound request on both the MITM and relay paths, and the workspace builds with the default unwind strategy (no `panic = "abort"` in `Cargo.toml`), so the panic surfaces in the hyper task handling that request.

**Impact.** Availability only, and only after a precondition the attacker does not control well: thread creation has to fail exactly once, at the first gateway-bound request after the observer is registered (that is, at the app process's first routed request each launch, under thread or memory exhaustion). If it does, the proxy's data plane is dead for the life of the process: every routed request panics before the credential is stamped. Nothing user-controlled triggers it, so this is fragility, not an exploit. The auth observer beside it does not have this shape because it spawns from the shell, not the core.

**Fix.** Use `std::thread::Builder::new().name("gate-traffic-sweeper").spawn(...)` and handle `Err` (log under `debug_log`, give up on sweeping, leave the `Once` completed). Or move the spawn off the request path entirely: spawn the sweeper from `set_traffic_observer`, which the shell calls once at setup, and drop `TRAFFIC_SWEEPER`.

#### L2. A local process can multiply the report cadence by choosing its classifier key

- `crates/core/src/proxy/mod.rs:701-714` (`note_traffic`), `crates/core/src/proxy/mod.rs:744-763` (`traffic_due`), `src/NewUiApp.tsx:630-636`

**What.** The per-tool map key comes from `client_tool` (`mod.rs:1664`), which reads `anthropic-client-platform`, `anthropic-client-app`, `User-Agent`, `originator` and the `oai-*` headers. All of those are set by the tool, so a process behind the proxy can rotate its headers and land under each of the nine slugs plus `None` in turn. Each key is reported independently at most once per `TRAFFIC_REPORT_INTERVAL` (30 s), and keys going quiet at different ticks produce separate `traffic-observed` events. The frontend runs `activity.reload()` on every event regardless of sender (`NewUiApp.tsx:632`).

**Impact.** The attacker gets the Overview re-read up to 10 times per 30 s instead of once, plus one per-tool overview and one feed read when the open pane's tool is among the senders. That is at most 12 requests per 30 s, or 24 a minute, against the gateway's 100-requests-per-minute per-source-IP bucket (`src/lib/activity.ts:413-417`). It is bounded, and it needs code already running on the user's machine and routing through their proxy, which can hit the gateway directly with the user's own tools anyway. Recorded because the design's stated bound is "two reports a minute per active tool" (`mod.rs:672-676`) and the multiplier is 10, not 1.

**Fix.** Optional. If the bound should not depend on the classifier's key count, coalesce in the frontend: skip `activity.reload()` when the last self-initiated read (`activityReadAt`) is younger than some floor, or in the core report all due tools in one batch by aligning `last_reported` across keys. Neither is required for safety.

#### L3. Event payload is trusted without a shape check in the listener

- `src/NewUiApp.tsx:654-656`, `src/NewUiApp.tsx:633`

**What.** `listen<(string | null)[]>("traffic-observed", (e) => refreshActivityRef.current(e.payload))` passes the payload straight to `tools.includes(openTool)`. A payload that is not an array throws inside the Tauri event callback.

**Impact.** Practically none. The only emitter is `src-tauri/src/lib.rs:4706`, whose payload is `&[Option<&'static str>]` serialised from a fixed slug set. `core:default` (granted to `main` and `onboarding`, `src-tauri/capabilities/default.json:7`) does include `core:event:allow-emit`, so a script in one of those webviews could emit a spoofed event, but those load only `WebviewUrl::App` content; the one remote webview (the Cloudflare challenge window, `lib.rs:3641`, `WebviewUrl::External`) is listed in no capability and cannot emit or listen. The tray capability grants listen/unlisten only. Worst case from a spoof is extra throttled reads or a thrown callback that React never sees.

**Fix.** Optional: `if (!Array.isArray(e.payload)) return;` before the call, matching how the payload is typed.

## Checked and sound

**Credential handling is untouched.** The only change to the injection path is three lines in `inject_attribution` (`mod.rs:1461-1464`) calling `note_traffic(tool)` with the same `tool` value the function already computed. `inject_gate_credential` (`mod.rs:1853`), `strip_client_auth`, the OAuth/API-key selection, `keychain.rs`, `account.rs` and `oauth.rs` are not in the diff. `note_traffic` receives only the `Option<&'static str>` slug, never the headers.

**Nothing attacker-controlled reaches the payload or the frontend.** The map key type is `Option<&'static str>` (`mod.rs:652`) and every `Some` value is a `Client::slug()` or one of the two web-client constants returned by `anthropic_client`, the UA allowlist, `chatgpt_app` and `openai_web` (`mod.rs:1664-1800`). Header bytes are matched against needles and discarded; nothing is copied. The event payload (`lib.rs:4706`) and the debug line (`mod.rs:728`, gated on `GATE_PROXY_DEBUG`) carry only those slugs: no install id, device name, gateway URL, path or credential.

**Only gateway-bound traffic reaches `note_traffic`.** `inject_attribution` has three callers: `inject_gate_credential` (reached from the engine's `apply_rewrite`, `engine.rs:1637`, which runs only on a `MatchedRoute` being rewritten onto the gateway, and from the relay's `inject_credential`, `relay.rs:703`, which forwards only to the gateway) and the `#[doc(hidden)]` test seam (`mod.rs:1565`). Pass-through traffic to non-intercepted hosts never gets here.

**The BTreeMap cannot grow without bound.** Keys are drawn from a closed set: 7 `Client` slugs the classifier can return, `CLAUDE_WEB_CLIENT`, `CHATGPT_WEB_CLIENT`, and `None`, so at most 10 entries for the life of the process (`mod.rs:706-713`). Entries are never removed and never need to be; `TrafficMark` is three `Instant`s.

**The hot-path lock is short and never awaited across.** `note_traffic` holds `TRAFFIC_SEEN` for one `entry().or_insert()` and two field writes (`mod.rs:706-713`), with no I/O and no `.await`; the `std::sync::Mutex` is fine here. The `OnceLock::get().is_none()` early return (`mod.rs:702-704`) keeps the CLI relay and the Linux daemon from ever taking the lock. The sweeper takes the lock for one walk of at most 10 entries per second (`mod.rs:719-723`) and releases it before calling the observer (`mod.rs:730-732`), so a slow `emit` cannot stall the request path.

**Lock poisoning is handled and practically unreachable.** The sweeper returns on `Err(_)` (`mod.rs:722`) and `note_traffic` ignores `Err` (`mod.rs:706`), so a poisoned mutex degrades to "no more reports" rather than a spin or a panic on the request path. Poisoning requires a panic while holding the lock; the only code that holds it is the entry update and `traffic_due`, whose `Instant::duration_since` calls saturate rather than panic. Once the sweeper has exited it is not restarted (the `Once` is spent), which is the documented behaviour and the focus edge covers it.

**The amplification bound holds, including across tools, `None` senders and the frontend.** Per key: `traffic_due` requires `last_reported` to be at least `TRAFFIC_REPORT_INTERVAL` old before reporting again (`mod.rs:750-752`), so no key fires more than once per 30 s regardless of request rate; the four unit tests pin this. Per event: `activity.reload()` is one `GET /v1/me/activity` (`crates/core/src/activity.rs:91-103`), `toolActivity.reload()` one more, `toolEvents.reload()` one `GET /v1/me/tool-events`, and the last two run only when the open pane's tool is in the batch (`NewUiApp.tsx:633`), which for a `null` sender is never. Focus edge: guarded by `ACTIVITY_REOPEN_MIN_MS` = 30 s (`NewUiApp.tsx:1067`, `:3830`), the same spacing. Hidden window: skipped (`NewUiApp.tsx:655`). No hook retries on failure or on a 429: `useActivity` (`activity.ts:444-492`) and `useToolEvents` (`toolEvents.ts:198-227`) set a failure and stop, so a throttled reply cannot start a retry storm. All hooks are no-ops while `enabled` is false (`activity.ts:445`, `toolEvents.ts:200`), so a signed-out or unverified window makes no reads at all. Worst case is the 24/min in L2.

**The sweeper thread is spawned only where an observer exists.** `note_traffic` returns before `call_once` when no observer is registered (`mod.rs:702-704`), so the CLI relay (`serve_relay`, `mod.rs:1099`) and the Linux helper daemon never start it. On Linux the daemon hosts both the engine and the relay (`manager_linux.rs:200-224` receives `relay_port` from `set_intercept`), so the app process's observer is registered (`lib.rs:4705`) but never fires, exactly as the comments say; the focus edge is the fallback there.

**Tauri event surface.** `app.emit` broadcasts to every webview, but the only remote-origin webview (the Cloudflare challenge window, `WebviewUrl::External`, `lib.rs:3641`) appears in no capability file and no capability declares `remote`, so it can neither receive nor emit. The listener is registered once with a latest-callback ref and unlistened on unmount (`NewUiApp.tsx:653-661`).

**Frontend `paged` flag.** `toolEvents.paged` (`toolEvents.ts:181-186`, `:263-270`) only decides whether the feed is re-read; it does not change what is fetched or how, and `fetchPage` still enforces the tool slug via the `ToolId` type on the Rust side (`activity.rs:146-160`).
