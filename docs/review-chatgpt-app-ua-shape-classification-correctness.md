# Correctness review: chatgpt-app-ua-shape-classification

Branch summary: replaces the `ChatGPTBrowser` UA prefix roster in `classify_client` with a shape match (non-`Mozilla/` first token followed by a `Mozilla/`-prefixed token), and adds a once-per-run stderr warning when an unrecognised non-browser client sends a request on the `chatgpt-apps` host.

## Verification performed

- Read the full diff plus surrounding code: `classify_client`, `is_non_browser_ua`, `is_wrapped_browser_ua`, `rules_for_client`, `domain_claiming_host`, `route_rules`, `GateHandler` (clone/latch semantics), and both call sites in `handle_request` / `should_intercept`.
- Confirmed the critical direction holds: a real browser UA always opens with `Mozilla/`, so `is_non_browser_ua` rejects it and `is_wrapped_browser_ua` can never classify the browser as `App` (crates/core/src/proxy/mod.rs:862-887). Leading whitespace is handled consistently (`trim_start` in the guard, `split_whitespace` in the shape check).
- Confirmed the latch is race-safe: `swap(true, Relaxed)` on an `Arc<AtomicBool>` shared across handler clones, and it sits last in the short-circuit chain so it is only consumed by a request that would actually print (crates/core/src/proxy/engine.rs:440-450).
- Confirmed `domain_claiming_host` over the unnarrowed `live_rules` resolves `chatgpt.com` to `chatgpt-apps` (catalog order puts it before the relay-only `chatgpt` entry, crates/core/src/proxy/catalog.rs:160 vs :258; asserted by the existing test at crates/core/src/proxy/mod.rs:1485-1488).
- Confirmed `Option::is_none_or` is available (workspace `rust-version = "1.86"`, stabilised in 1.82).
- `cargo test -p gate-connect-core --lib proxy`: 150 tests pass, including the new positive (CodexBrowser, legacy ChatGPTBrowser) and negative (Mozilla-first) classification cases.
- `rg` for `ChatGPTBrowser`/`CodexBrowser` across the repo: no other code references the old prefix, so no caller was left matching the roster.

## Findings

1. **L** crates/core/src/proxy/mod.rs:881-887 (used at :952): narrow behavior drift for previously-matched UAs. The old rule was `ua.starts_with("ChatGPTBrowser")`, which also matched a UA that carries the token without a wrapped browser UA after it (a bare `ChatGPTBrowser/2.0`, or `ChatGPTBrowser` glued to the rest without a space). The shape rule requires a whitespace-separated later token starting with `Mozilla/`, so such a UA now classifies `Unknown` instead of `App`. All captured shells are wrapped, and `App`/`Unknown` route identically today, so the only observable differences are debug-log labels, the new warn line becoming eligible for it, and the documented future guard (the app check outranking web markers) no longer covering that hypothetical shape. Worth knowing, not worth changing.

2. **L** crates/core/src/proxy/engine.rs:442-446: the comment says the check uses "the MITM entry for chatgpt.com, whether or not it is switched on", but when no enabled entry claims `chatgpt.com` the CONNECT is blind-tunnelled by `should_intercept` and the inner requests never reach `handle_request`, so the warn is unreachable with the host fully off. It genuinely fires with `chatgpt-apps` off only when the sibling `chatgpt` entry (same host) is on. Behavior is fine; the comment slightly overstates reach.

3. **L** crates/core/src/proxy/engine.rs:447-450: the single per-run latch can be consumed by an ordinary third-party client (the state the doc itself calls "ordinary" and "harmless"), in which case a renamed app shell arriving later in the same run is never named in the log. The printed message hedges for this ("Expected for third-party clients"), and any wider net would be noise, so this is a documented tradeoff rather than a bug; noted so the limitation is on record.

No high or medium severity issues found. The classification change is additive for every UA shape seen in captures, the browser-narrowing direction is protected by both the code structure and new tests, and the warning path cannot alter routing (it only reads and logs).
