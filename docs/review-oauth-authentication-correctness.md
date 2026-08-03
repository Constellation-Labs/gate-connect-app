# Correctness / logic review — `feat/oauth-authentication`

Lens: correctness / logic bugs (concurrency, edge cases, error paths, expiry
math, state-machine, chunking boundaries, test fidelity). Base `main`
(merge-base `581fb6d`). Files read: `crates/core/src/oauth.rs`,
`keychain.rs` (+ `tests/keychain_chunking.rs`), `proxy/engine.rs`,
`proxy/relay.rs` (+ `tests/relay_e2e.rs`), `proxy/manager.rs`,
`proxy/manager_linux.rs`, `account.rs`, `org.rs`, `src-tauri/src/lib.rs`, and
all `tests/oauth_*.rs`.

## Summary

The OAuth mechanics are solid: PKCE/S256, expiry-skew math, refresh-token
carry-forward, the non-2xx error path, and keychain chunk round-trip are all
correct and well-tested. The real risks are in the *credential-selection state
machine*, not the crypto. Injection decides "OAuth vs API key" purely on whether
a token string is non-empty and never consults the persisted `auth_mode`, so
the mode field and the credential actually sent can diverge. Two edge cases fall
out of that: pasting a Gate key while an OAuth session is still valid does not
actually switch to the key, and an OAuth token with no org selected is injected
half-authenticated (the gateway rejects it) while the UI reports "signed in".
The refresh path also has no single-flight guard, which is benign only as long
as Cognito refresh-token rotation stays disabled.

---

## High

### H1. Injected credential ignores `auth_mode`; switching to the API key while OAuth is still valid silently keeps using OAuth
`crates/core/src/oauth.rs:409` (`access_token_for_injection`),
`crates/core/src/proxy/engine.rs:479` (`apply_rewrite`),
`crates/core/src/proxy/relay.rs:387` (`inject_credential`),
`crates/core/src/proxy/manager.rs:99` / `manager_linux.rs:136` /
`manager_windows.rs:101` (engine seed),
`src-tauri/src/lib.rs:282-291` (`save_account`).

**Bug.** The credential choice is made solely by "is the OAuth token string
non-empty?" — `apply_rewrite`/`inject_credential` inject the bearer whenever a
token is present, and the managers seed the engine from
`access_token_for_injection()`, which returns the stored token whenever a valid
bundle exists. None of these consult `account::auth_mode`. The desktop
*startup* and *background* refresh loops (`lib.rs:1004`, `lib.rs:1094`) *do*
guard on `auth_mode == OAuth`, so the guard exists in two places and is missing
in the seed/injection path — an inconsistency that is the tell.

`save_account` (`lib.rs:288`) records `AuthMode::ApiKey` and pushes the new key
via `refresh_api_key(k)`, but it neither clears the stored OAuth tokens nor
pushes `refresh_token("")`. `refresh_api_key` only updates the key watch-channel.

**Failure scenario.** User signs in via OAuth (tokens stored, `auth_mode=OAuth`),
then pastes a Gate key to move to the legacy path.
`auth_mode` flips to `ApiKey` and the key reaches the engine's key channel, but
the still-valid OAuth token remains in the engine's *token* channel and in the
keychain. Because OAuth wins on non-empty, every request keeps going out with
`x-gate-authorization: Bearer <old cognito token>` + `x-gate-org-id`, not the
pasted `x-gate-api-key`. On the next restart the manager re-seeds
`oauth_token: access_token_for_injection()` (unguarded), so it persists across
launches. The pasted key never takes effect until the user explicitly signs out
of OAuth or the refresh token dies.

**Fix.** Make `auth_mode` the single gate. Cheapest and consistent with the
existing loop guards: have `access_token_for_injection()` return `""` when
`account::auth_mode()? != OAuth`, so every seed/inject caller agrees. (And/or
have `save_account`/`set_auth_mode(ApiKey)` call `oauth::clear()` +
`manager().refresh_token("")`.) Add a test that asserts pasting a key while an
OAuth bundle is present results in `x-gate-api-key` on the wire.

---

## Medium

### M1. OAuth token injected without an org header when no org is selected → gateway rejects all traffic while status shows "signed in"
`crates/core/src/proxy/engine.rs:479-491`,
`crates/core/src/proxy/relay.rs:387-397`,
`crates/core/src/account.rs:221` (`org_id_for_injection`),
`crates/core/src/org.rs:6-9` (contract: OAuth request without org is rejected).

**Bug.** `apply_rewrite`/`inject_credential` inject the bearer whenever the
token is non-empty and add `x-gate-org-id` only "when org is present". If a
valid OAuth token exists but `org_id` is empty, the request goes out
authenticated-but-org-less, which `org.rs` documents the gateway rejects.
Meanwhile `oauth_status`/`live_session` (`oauth.rs:400`) never consider the org,
so the UI reads `signed_in = true`. The token and org live in two independent
watch-channels seeded/updated separately (`manager.rs:99-101`, relay
`serve` loop `relay.rs:271-275`), so there is no atomicity between them.

**Failure scenario.** Signed in via OAuth, org not yet picked (or org cleared by
`switch_gateway`, which sets `org_id=None` but leaves `auth_mode=OAuth` and does
not clear tokens — see L3). Routing is enabled → engine seeds `oauth_token=<valid>`,
`org_id=""` → all proxied inference requests are rejected by the gateway, yet the
popover shows a signed-in home. Self-corrects only when the user happens to open
the org picker.

**Fix.** Treat "OAuth token but no org" as not-ready: either don't inject the
bearer until an org is present (return `""` from `access_token_for_injection`
when `org_id_for_injection()` is empty in OAuth mode), or surface an explicit
"pick an org" state that `oauth_status` reports so the UI can't show a working
home. At minimum, guarantee token and org are pushed together.

### M2. No single-flight around `ensure_fresh`; concurrent refreshers are safe only while Cognito rotation is off
`crates/core/src/oauth.rs:422-437` (`ensure_fresh`, refresh + `store`),
`oauth.rs:400` (`live_session`), `src-tauri/src/lib.rs:1090-1108` (30s loop),
`oauth_status_now` (`lib.rs:376`), `proxy/relay.rs:264-276` (relay serve loop).

**Bug.** On desktop the 30s background loop, `oauth_status` polling, and startup
all reach `ensure_fresh`, which on expiry POSTs a refresh and `store`s the
result — with no lock and no re-check. The standalone `proxy serve` relay runs
its own refresh loop against the *same* keychain bundle in a separate process.
`parse_token_response` (`oauth.rs`) already supports rotation (it carries a new
`refresh_token` when the response includes one), so this is a live concern, not
hypothetical.

**Failure scenario.** Two callers see the token expired at the same tick and
both call `refresh`. With rotation *off* (current Cognito default) both reuse the
same refresh token and succeed — merely wasted work, last write wins. If rotation
is ever enabled: caller A rotates and stores a new refresh token; caller B, using
the now-invalidated old token, gets `invalid_grant` → its `ensure_fresh` errors →
`live_session` returns `None` for that call → a spurious "signed out" / empty
injection for one tick before the next read reuses A's fresh bundle. Two processes
(app + relay) make this more likely.

**Fix.** Wrap `ensure_fresh` in a process mutex and re-check `is_expired` after
acquiring it (skip the refresh if another caller already renewed). Document that
correctness currently depends on rotation being disabled, or make the failed-refresh
path re-read the store and retry once before reporting `None`.

---

## Low

### L1. Chunk-size budget counts chars, not UTF-16 code units — the Windows blob cap it works around can still be exceeded
`crates/core/src/keychain.rs:29` (`MAX_CHUNK_CHARS = 1024`),
`keychain.rs:154` (`split_chunks`).

The constant's own comment reasons in UTF-16 bytes ("2 bytes/char for ASCII"),
but `split_chunks` caps by `char` count. A chunk of 1024 non-BMP chars (e.g.
emoji) is 2048 UTF-16 code units = 4096 bytes, over the 2560-byte Windows cap —
re-tripping the exact limit chunking exists to dodge. Not reachable by the
actual payloads (JWTs, `sk-gw-*` keys, PEM are all ASCII/BMP), so latent.

**Fix.** Budget by `ch.len_utf16()` accumulation rather than char count, or
assert/document the ASCII/BMP-only assumption for stored secrets.

### L2. `switch_gateway` leaves stale, cross-pool OAuth tokens in the keychain
`crates/core/src/account.rs:176-200`.

`switch_gateway` deletes the API key and clears the org, but does not call
`oauth::clear()`, and leaves `auth_mode` as-is. After switching from staging to
prod (or vice-versa) the old pool's tokens linger. `from_build_env` now resolves
the *new* gateway's pool, so a refresh of the old refresh token against the new
pool fails — self-correcting to signed-out, but the stale bundle sits in the
secret store until a full `clear`/`reconcile` (which only wipes OAuth when the
URL is gone, `account.rs:317`). Consider clearing OAuth tokens on gateway switch,
matching the "org is environment-specific" reasoning already applied to the org.

---

## Test-fidelity notes

- **No test covers the H1/M1 couplings — because the couplings don't exist.**
  The suite proves token-presence → bearer (`relay_e2e.rs`,
  `engine.rs` `rewrite_with_oauth_token_*`) but never asserts that `auth_mode`
  or org-selection gates injection. So the branch's tests pass while the
  "pasting a key switches to the key" and "no org ⇒ don't inject" behaviors are
  simply unimplemented. Add wire-level tests for both.
- **`oauth_session_e2e.rs` step 4 proves "unreachable ⇒ None, bundle kept" but
  not "revoked (400) ⇒ None, bundle kept."** It points at a *closed port*
  (connection refused), not a 400. The 400 path is exercised separately in
  `oauth_refresh_error_e2e.rs`, but that test only asserts `refresh()` errors —
  it never drives `live_session` to confirm a 400 leaves the stored bundle
  intact. The more important revoked-token case (should keep the bundle so a
  transient 5xx doesn't sign the user out) is therefore not end-to-end proven.
- **`keychain.rs` unit test `split_chunks_reassembles_exactly` asserts
  `chars().count() <= MAX_CHUNK_CHARS`, not byte size** (`keychain.rs:265`), so
  it cannot catch L1. The integration test (`keychain_chunking.rs`) uses ASCII
  only. Neither exercises the UTF-16-byte boundary the cap is about.
- Positive: `oauth_env_selection.rs`, `oauth_token_e2e.rs`,
  `oauth_login_e2e.rs`, and the `relay_e2e.rs` hot-swap/unknown-upstream cases
  are faithful and assert real behavior (request bodies, headers on the wire,
  persisted state), not trivia.