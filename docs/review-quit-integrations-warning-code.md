# Code review: `quit-integrations-warning` (lens: code quality)

Base: `main` (d5f44cc5). Commits: `a94f987c` (proxy doc comments), `095d7efd` (quit warning feature).

## Summary

Small, well-scoped branch. The new `QuitConfirm` takeover closely mirrors the existing `StartupRoutingNotice` pattern (same shell markup, same busy/error state shape, same raw-string error display, documented z-order above the other takeovers). The Rust side follows house idioms: `spawn_blocking` for config-file I/O, `{e:#}` error chains, block-scoped `use tauri_plugin_notification::NotificationExt` matching the existing call at lib.rs:1644, and the new commands registered in both cfg-forked `generate_handler!` lists. Doc comments are unusually good and cross-reference correctly (`request_quit` -> `quit_app`, the `RunEvent::Exit` proxy-revert claim checks out at lib.rs:1845). Tests cover copy pluralization, all three actions, and the failed-disconnect path. Findings below are one semantic-consistency issue and a handful of nits.

## Findings

### M1 - `request_quit` ignores `Drifted` and errored statuses; inconsistent with the codebase's own "managed" definition
`src-tauri/src/lib.rs:1068` filters on `matches!(integ.status(), Ok(Status::Connected))` only. The registry's `disconnect_all_managed` (`crates/core/src/registry.rs:170-180`) treats `Connected | Drifted(_)` as managed and treats `Err(_)` as managed too, with an explicit comment that a failed status probe "doesn't prove the tool is clean". A `Drifted` tool's config can still point at the loopback relay (drift is partial hand-editing), yet the tray quit exits with no warning; likewise a tool whose config is momentarily unparsable. If excluding Drifted is deliberate (user took ownership of the config), the doc comment on `request_quit` (lib.rs:1057) should say so; as written it claims to catch "config-routed CLI tools \[that\] hard-fail", which Drifted tools may also be.

### L1 - `quit_confirmed` is tracked immediately before process exit and may never flush
`src/components/QuitConfirm.tsx:31,44` call `track("quit_confirmed", ...)` and then `await quitApp()`, which is `app.exit(0)` (lib.rs:1084). posthog-js batches captures; there is no flush before the process dies, so the event will frequently be dropped - undermining the funnel that `quit_warning_shown` (App.tsx:302) opens. Precedent exists (`UpdatePanel.tsx:88-89` tracks `update_installed` then `relaunch()`), so this matches house style, but it is worth a comment acknowledging the loss or a bounded flush wait.

### L2 - `quitAnyway` can strand the panel in a permanent busy state
`src/components/QuitConfirm.tsx:42-46`: `setBusy(true)` then `await quitApp().catch(() => {})`. If the invoke rejects (webview/IPC hiccup), the error is swallowed, `busy` stays true, and all three controls stay disabled with no message. `turnOffAndQuit` resets `busy` in its catch; `quitAnyway` should too, even if the failure is near-impossible.

### L3 - one-shot `quit-requested` emit can race the frontend mount
`src-tauri/src/lib.rs:1076` emits once with no retry/ack. The popover webview persists across hide/show so the App.tsx:155 listener is normally long-mounted, but a tray Quit in the first moments after launch (before React mounts) shows the popover and drops the event - the quit silently never happens and there's no log of it. An `eprintln!`/`log` on the emit result, or noting the assumption in the comment, would make the window explicit.

### L4 - test coverage gaps in `QuitConfirm.test.tsx`
`src/components/QuitConfirm.test.tsx` covers copy, all three actions, and the failed-disconnect message, but does not assert (a) the controls are disabled while `busy` (the "Working…" state), or (b) that "Quit anyway" still functions after a failed disconnect (the comment at test line 81 promises retry-or-quit-anyway; only the retry button's presence is asserted).

### L5 - hand-rolled `joinNames` vs `Intl.ListFormat`
`src/components/QuitConfirm.tsx:8-12`: `new Intl.ListFormat("en")` does the same thing. The hand-rolled version is small, correct, and unit-tested via copy assertions, so this is purely a nit; the app is en-only.

## Non-findings (verified)

- Two `generate_handler!` lists (lib.rs:1147-1231) predate this branch (cfg fork with a comment justifying it); both were updated consistently.
- Raw error string display (`typeof e === "string" ? e : String(e)`) matches `StartupRoutingNotice.tsx:37`; the `quit_disable` title added in `errors.ts:163` is consumed by `trackError` -> `classifyError` for the `error_shown` event, same as `close_agents` - not dead code.
- z-30 above UpdatePanel (z-20) and StartupRoutingNotice (z-10) is intentional and documented in both the component doc and App.tsx render order.
- The doc-comment-only changes in `crates/core/src/proxy/manager.rs` and `system_proxy.rs` are accuracy fixes (PAC fetch fails -> silent DIRECT fallback, not ERR_PROXY_CONNECTION_FAILED) and read correctly against the surrounding code.
- New analytics event names and prop keys are properly added to the `AnalyticsEvent` union and `ALLOWED_PROP_KEYS` allowlist (analytics.ts).

## Verdict

Approve with minor changes. M1 (Drifted/Err statuses bypassing the warning) should be either fixed to match `disconnect_all_managed`'s semantics or explicitly documented as a deliberate exclusion before merge; the L items are polish.
