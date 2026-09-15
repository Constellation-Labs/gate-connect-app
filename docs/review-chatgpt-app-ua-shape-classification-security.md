# Security review: chatgpt-app-ua-shape-classification

Branch summary: replaces the `ChatGPTBrowser` user-agent prefix roster with a shape rule (`is_wrapped_browser_ua`) for classifying the ChatGPT/Codex app shell, and adds a once-per-run stderr warning when an unrecognised non-browser client hits the `chatgpt-apps` host.

## Scope reviewed

`git diff main...HEAD` (one commit, aad8a79) touching `crates/core/src/proxy/engine.rs` and `crates/core/src/proxy/mod.rs`, plus the surrounding classification, routing, and logging code those changes feed into (`classify_client`, `rules_for_client`, `decide` call sites, `handle_request`, `should_intercept`, the `chatgpt-apps` catalog entry).

## Findings

No must-fix (H) or should-fix (M) issues found. Two low-severity observations.

- **L** - `crates/core/src/proxy/engine.rs:619` / `crates/core/src/proxy/engine.rs:453`: the new warning is the only unconditional (non-`debug_log`) log line reachable by a non-owner local peer. `peer_allowed` gates MITM at CONNECT (engine.rs:543) and gates key injection at rewrite (engine.rs:687-688), but plain-HTTP absolute-form requests reach `handle_request` without either gate, so on Linux with `owner_uid` set, another local user's request to `http://chatgpt.com/...` can emit that user's user-agent and host into the owner's stderr log. Exposure is one UA string, latched to once per engine run, loopback only; the pre-existing per-request line at engine.rs:733 has the same property but only under opt-in `debug_log`. If you want parity with the CONNECT-stage warning (engine.rs:543 runs before engine.rs:559), gate the call on `self.peer_allowed(ctx)`.

- **L** (accepted-risk confirmation) - `crates/core/src/proxy/mod.rs:881` and `crates/core/src/proxy/mod.rs:952`: the shape rule widens which clients classify as `App`, and because the app UA check runs before the web-marker check (mod.rs:959-968), a client that both prepends a token to a `Mozilla/` UA and sends the web markers (`oai-device-id` etc.) now escapes the browser narrowing in `rules_for_client` (mod.rs:988-1008), meaning its full chatgpt.com plumbing, session cookie included, is rewritten to the gateway with Gate credentials attached. This requires a spoofed UA: every real browser's UA leads with `Mozilla/`, so `is_non_browser_ua` (mod.rs:862) excludes it, and the new negative tests (mod.rs:1636-1648) pin that direction. Spoofing is self-affecting only, per the documented threat model (mod.rs:916-918: the classifier decides what Gate may see, not what a client may do, and the proxy is restricted to the local owner). Noting it so the widened surface is a recorded decision rather than an accident.

## Checked and found sound

- **Routing privilege**: `ClientClass` is consumed only by `rules_for_client` (mod.rs:988), which narrows `Web` and treats `App` and `Unknown` identically, and by the debug log (engine.rs:734). Misclassifying a third-party client as `App` therefore grants nothing today; the doc claim "both route identically" at mod.rs:879 matches the code.
- **The load-bearing direction holds**: a browser's own UA starts with `Mozilla/` (after `trim_start`), so `is_wrapped_browser_ua` can never classify the real website as `App`; claude.ai's Web classification additionally rides the decisive `anthropic-client-platform` header checked first (mod.rs:927-933).
- **Log injection**: the UA reaches the warning via `HeaderValue::to_str` (engine.rs:625), which rejects non-visible-ASCII bytes, and is printed with `{ua:?}` so anything odd is escaped; `host` comes from hyper's parsed `Uri` authority (engine.rs:613), which cannot carry CTLs or whitespace. No newline or ANSI injection path into stderr.
- **Pathological input / ReDoS**: no regex anywhere in the change; `is_wrapped_browser_ua` is a single `split_whitespace` scan, linear in a header whose size hyper already bounds. The warning path adds one `domain_claiming_host` linear scan over the small catalog, only on the Unknown+non-browser branch, and latches after the first hit.
- **Secret leakage**: the warning logs only host and UA, neither a credential channel here; the existing discipline of excluding query strings (engine.rs:628-631) and redacting `x-goog-api-key` (engine.rs:747-748) is untouched.
- **Latch soundness**: `AtomicBool::swap(true, Ordering::Relaxed)` (engine.rs:448) is race-safe for its purpose; the worst outcome of reordering is a duplicate log line.
