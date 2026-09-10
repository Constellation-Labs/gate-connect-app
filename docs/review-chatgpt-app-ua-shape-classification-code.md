# Code review: chatgpt-app-ua-shape-classification

Branch summary: replaces the `ChatGPTBrowser` UA prefix roster with a shape match (`<token> ... Mozilla/...`) in `classify_client`, and adds a once-per-run stderr warning when an unrecognised non-browser client hits the `chatgpt-apps` host, so the next shell rename is announced instead of silently dropping app classification.

Overall: small, well-scoped change. The doc comments explain the failure history and the deliberate width of both nets, the new latch mirrors the existing `anthropic_unselected_logged` pattern exactly, and the classifier change is covered in both directions (new `CodexBrowser` UA classifies as App; a browser UA with a trailing `CodexBrowser/1.0` token does not). Findings below are should-fix and nits; nothing blocking.

## Findings

- **M** `crates/core/src/proxy/engine.rs:433` - the gating half of `warn_if_an_app_shell_is_unrecognised` (Unknown class + non-browser UA + `domain_claiming_host` slug check + latch) has no test, and as written it cannot get one: the predicate is fused with the `eprintln!` and the latch inside one `&self` method. The warning is half the branch's stated purpose ("say so when it drifts"), and its slug comparison and `live_rules`-vs-narrowed-rules choice are exactly the kind of condition that regresses silently. The sibling `warn_if_anthropic_is_unselected` is equally untested, so this follows an existing pattern; still, extracting the pure condition (client, ua, claimed slug) into a testable function would cover the part that can actually go wrong. Suggestion, not a demand - flagging for a decision rather than assuming the pattern is fine to extend.

- **L** `crates/core/src/proxy/mod.rs:857` - the doc on `is_non_browser_ua` says it is "used only to *report*" and is "Deliberately not a classification signal", but `is_wrapped_browser_ua` (a classification signal) calls it as its first conjunct at mod.rs:882. The intent is clearly "not a classification signal on its own"; the literal claim is false and will misdirect the next reader who greps for callers. One clause fixes it.

- **L** `crates/core/src/proxy/engine.rs:429` - doc-comment wrap glitch: "...still surfaces. That width is why it is latched to one line" runs one sentence past the wrap width used by the rest of the paragraph, which reads like a mid-edit leftover. Rewrap.

- **L** `crates/core/src/proxy/engine.rs:612-627` - the user-agent is extracted twice on every request: once inside the `classify_client` closure via the string name `"user-agent"`, and again for the warn call via `hudsucker::hyper::header::USER_AGENT`. Hoisting one `let ua = ...` above `classify_client` and passing it to both would remove the duplicate lookup and the mixed name-vs-constant style in the same function.

- **L** `crates/core/src/proxy/engine.rs:608` - pre-existing, adjacent to the touched code: the comment says "see `BROWSER_EXCLUDED_SLUGS`", a constant that no longer exists; it was renamed to `BROWSER_ROUTED` (mod.rs:826) in #145. Since this branch adds code directly under that comment, it is a cheap ride-along fix.

## Verified non-issues

- `is_wrapped_browser_ua` and `is_non_browser_ua` agree on leading whitespace: `trim_start` and `split_whitespace` skip the same bytes, so the "first token" both see is identical.
- The warn call deliberately passes `live_rules` rather than the narrowed/forced set, and the comment at engine.rs:442-443 says why; `domain_claiming_host` (mod.rs:728) includes disabled entries by design, which matches the "reports on a client, not a route" intent.
- The latch `swap` is the last short-circuit condition, so a request that fails an earlier check does not consume the one-shot. Correct ordering.
- `Option::is_none_or` needs Rust 1.82; workspace MSRV is 1.86 (Cargo.toml:13).
- A third-party UA shaped like `MyAgent ... Mozilla/...` classifying as App instead of Unknown is acknowledged in the doc (mod.rs:877-880) and is routing-equivalent today; the direction that matters (browser never reads as App) is asserted by the new `assert_ne!` loop at mod.rs:1642-1651.
