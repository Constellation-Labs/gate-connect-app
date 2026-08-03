# Code review: `feat/oauth-authentication` — Code quality / maintainability

Base `main` (`581fb6d`). Reviewed the highest-risk diffs: `oauth.rs`, `proxy/relay.rs`, `keychain.rs`, `org.rs`, `account.rs`, `proxy/engine.rs`, `proxy/{mod,manager,manager_linux,helper,helper_client}.rs`, `integrations/{opencode,claude_code}.rs`, `src-tauri/src/lib.rs`, `cli/src/main.rs`, `src/{App.tsx,lib/api.ts,screens/*}`, and `tests/relay_e2e.rs`.

## Summary
The branch is unusually well-crafted for its size: doc comments explain *why*, the credential-as-injection design is consistent end-to-end, and tests are hermetic and target the real edge cases. CLAUDE.md conventions are respected. Findings are mostly **duplication that can now drift** plus one **dead-code chain** the relay migration left behind. No High-severity maintainability issues.

## Medium

**M1. Credential-injection precedence implemented twice, can drift.** `crates/core/src/proxy/engine.rs:464` (`apply_rewrite`, header writes `479-496`) and `crates/core/src/proxy/relay.rs:379` (`inject_credential`) encode the same rule. They already diverge cosmetically: `relay.rs:112-118` uses named constants, `engine.rs` hardcodes the same strings as literals. A future precedence change must land in both. Suggest hoisting the three header-name constants and one `inject_gate_credential(headers, api_key, oauth_token: Option<&str>, org_id: Option<&str>)` into `proxy/mod.rs` and calling it from both.

**M2. `refresh_gate_key` is now a no-op in every integration — the `*_everywhere` chain is dead.** All five impls return `Ok(())` (`claude_code.rs:244`, `codex.rs:504`, `hermes.rs:267`, `openclaw.rs:369`, `opencode.rs:443`), yet `registry::refresh_gate_key_everywhere` (`crates/core/src/registry.rs:164`) is still called on every key save from `crates/cli/src/main.rs:193` and `src-tauri/src/lib.rs:291` — iterating all tools to invoke no-ops on the hot save path. Suggest deleting the trait method + all impls + the free function + both call sites. Keep `manager().refresh_api_key` (still live: hot-swaps the key into the running MITM engine for system-proxy-routed GUI apps). Worth raising as a question if the hook is meant to stay for an unlanded tool.

## Low

**L1. `refresh_token`/`refresh_org`/`refresh_api_key` on Linux are three near-identical bodies** (`manager_linux.rs:224,254,284`): lock client, `load_domains`, `load` account + CA, then one 9-arg `set_intercept` where exactly one arg is the "live" value. Extract a private `resend_intercept(token: Option<&str>, org: Option<&str>)`; the three become one-liners. Reduces risk given the args already need `#[allow(clippy::too_many_arguments)]`.

**L2. Relay silently drops a malformed credential.** `relay.rs:379`: if `HeaderValue::from_str` fails for the token/key, the `if let Ok` arm is skipped and the request is forwarded with *no* Gate credential → opaque 401. The MITM path (`engine.rs:481-495`) propagates a `context()` error instead. Unlikely (ASCII `sk-gw-…`) but the asymmetry is a trap; return a 5xx from the relay on `Err`.

**L3. Duplicated 30s silent-refresh loop** in `relay.rs::serve` and `src-tauri/src/lib.rs:1088`. Sinks differ (watch channels vs `manager().refresh_token`), so full extraction is awkward; the shared `REFRESH_INTERVAL_SECS` and cross-referencing comments already mitigate. Optional.

## Nits
- **N1.** `oauth.rs:96-134` `from_build_env`: staging/prod arms duplicate the three `config_value(NAME, option_env!(NAME))` calls with a `_STAGING` suffix. Minor.
- **N2.** Empty-string sentinel for "no token/org" in `access_token_for_injection` / `org_id_for_injection` threaded through `EngineConfig` and watch channels — consistent and documented; `Option` would be type-enforced but watch channels want a concrete value. Not worth changing.
- **N3.** `src/screens/FirstRun.tsx`: the `!showKey ? (…) : (…)` branches are indented at the parent's level, reading as siblings rather than ternary arms. Prettier fixes it.
- **N4.** `src/screens/OrgPicker.tsx:14`: `onReauth,` prop is flush-left vs the 2-space-indented siblings.

## Positives (kept, not findings)
`keychain.rs` chunking has a collision-argued manifest sentinel, manifest-written-last torn-write ordering, and multibyte round-trip tests; `oauth.rs` binds IPv4+IPv6 loopback and validates `state` before code redemption; the test suite asserts on injected headers via `GATE_CONNECT_TEST_*` seams; frontend `isSignedIn`/`needsOrg` centralize the tri-state routing.