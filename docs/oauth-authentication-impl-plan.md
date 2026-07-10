# Add OAuth (Cognito) auth to Gate Connect, API key becomes legacy

## Goal

Let Gate Connect authenticate to the Gate gateway with a Cognito-issued
OAuth **access token** on a custom header, instead of the static
`x-gate-api-key`. On startup, silently refresh an expired token; if that
fails, surface a "Sign in" state in the popover that opens the Cognito
Hosted UI in the browser and captures the redirect on a loopback listener.
The pasted API key stays supported as a **legacy** path.

## Decisions locked (from planning Q&A)

1. **Credential + header:** Cognito `access_token`, sent as a new custom
   header `x-gate-authorization: Bearer <access_token>` (exact name is the
   user's server-side contract; placeholder used here). Sits parallel to
   `x-gate-upstream-url`. The client tool's own `Authorization` header keeps
   passing through untouched.
2. **Redirect capture:** loopback HTTP listener
   (`http://127.0.0.1:<port>/callback`), RFC 8252 native-app pattern. No
   deep-link plugin.
3. **CLI tools:** proxy-route everything so the token is injected live and
   refreshes transparently - **stop baking credentials into tool config
   files**. See "CLI reverse proxy" below for the concrete mechanism.
4. **Startup:** try stored `refresh_token` silently; only on failure show a
   "Sign in" prompt in the popover.

## Server-side dependency (user's own workstream - NOT in this plan)

- Gateway validates the Cognito access token on `x-gate-authorization`
  (or whatever final header name) as an alternative to `x-gate-api-key`.
- The Cognito **app client** must whitelist the desktop redirect URL. For a
  loopback listener that means allowing `http://127.0.0.1` /
  `http://localhost` callback URLs (Cognito permits loopback for the
  authorization-code + PKCE flow) OR a fixed set of ports. Confirm the
  client is a **public** client (no secret) so native PKCE works.
- Needed config values to bake into the app build (mirror the web app's
  env: region, user pool id, **app client id**, **hosted domain**, scopes):
  web app uses `VITE_COGNITO_*` (see gate `apps/admin-web/src/lib/auth-config.ts`).

## Current state (evidence)

- **Injection today** - MITM engine `apply_rewrite` adds `x-gate-api-key` +
  `x-gate-upstream-url`, hot-swappable via a `watch` channel:
  `crates/core/src/proxy/engine.rs:401` (`apply_rewrite`), `:121`
  (`update_api_key`), `:187` (`api_key` receiver field).
- **Account/keychain** - Gate key in OS secret store; base URL + key prefix
  in a JSON file. `crates/core/src/account.rs:21` (`Account`), `:27`
  (`AccountFile`), `:91` (`save`), `:202` (`reconcile`);
  `crates/core/src/keychain.rs` (`get`/`set`/`delete`, `account_service`).
- **CLI tools bake the key into config files** (this is what changes):
  Claude Code merges `ANTHROPIC_CUSTOM_HEADERS` (`X-Gate-Api-Key` +
  `X-Gate-Upstream-Url`) into `~/.claude/settings.json`
  (`crates/core/src/integrations/claude_code.rs:38,42`); Codex writes the
  header into `~/.codex/config.toml`
  (`crates/core/src/integrations/codex.rs:157,391`); OpenCode similarly.
- **Rotation fan-out** - `registry::refresh_gate_key_everywhere`
  (`crates/core/src/registry.rs:158`) pushes a rotated key into every tool
  config; `proxy::manager().refresh_api_key` (`manager.rs:195`) pushes it
  into the live engine. Both called from `save_account`
  (`src-tauri/src/lib.rs:261`).
- **Startup hook** - `src-tauri/src/lib.rs` `.setup()` closure (~`:765`) +
  the off-thread reconcile at `:797`.
- **Frontend** - `src/lib/api.ts` (`getAccount`, `saveAccount`,
  `getAccountKeyPrefix`), `src/components/AccountForm.tsx` (create/edit
  account form), `src/App.tsx` (top-level account/proxy state),
  `src/screens/Settings.tsx`.
- **No OAuth infra exists** - no `oauth2`/`jsonwebtoken`, no deep-link, no
  outbound HTTP client in the shipped binary (`reqwest` is dev-only).
- **CSP** locks `connect-src` (`src-tauri/tauri.conf.json:30`) - token
  endpoint calls happen in Rust, so the webview CSP need not change, but the
  `opener` capability must allow the Cognito domain
  (`src-tauri/capabilities/default.json:17`).

## Architecture

Two injection sites, one shared token source.

```
                         ┌───────────────────────────┐
   Cognito Hosted UI ──▶ │  oauth module (crates/core)│
   (browser, PKCE)       │  code→token, refresh,      │
                         │  store in keychain,        │
                         │  watch::channel<Token>     │
                         └──────────┬────────────────┘
                                    │ current access token (hot-swap)
              ┌─────────────────────┼─────────────────────┐
              ▼                                            ▼
   MITM engine apply_rewrite                    CLI reverse proxy (NEW)
   (Cowork / Claude Desktop,                    (Claude Code / Codex /
    system-proxy apps)                           OpenCode point baseURL
   injects x-gate-authorization                  at http://127.0.0.1:PORT;
   live per request                              injects token live, no CA,
                                                  no secret in config file)
```

### Token source (new `crates/core/src/oauth.rs`)

- `struct OAuthTokens { access_token, refresh_token, id_token?, expires_at: OffsetDateTime }`.
- Stored as one JSON blob in keychain under a new
  `keychain::account_service("oauth-tokens")` label (secret). Non-secret
  bits (`expires_at`, subject/email for UI) also recorded in `account.json`
  for cheap status reads without touching the secret store.
- Functions: `begin_login() -> AuthorizationRequest` (builds PKCE
  verifier/challenge, state, the Hosted UI `/oauth2/authorize` URL, and the
  loopback redirect URL), `complete_login(code, verifier) -> OAuthTokens`
  (POST `/oauth2/token` `grant_type=authorization_code`),
  `refresh(refresh_token) -> OAuthTokens` (`grant_type=refresh_token`),
  `current() -> Option<OAuthTokens>`, `clear()`.
- Uses `expires_in` from the token response (no client-side JWT verify;
  server verifies signature). A small skew margin (~60s) triggers refresh.
- New deps (promote/add, rustls, no OpenSSL): `reqwest` (prod, `rustls` +
  `json`), plus PKCE bits via already-present `ring`/`rustls` or small
  `sha2`+`base64`+`rand`. `time` already present.

### Loopback redirect listener

- Reuse the ephemeral-loopback bind pattern from `engine.rs:435`. Bind
  `127.0.0.1:0`, serve a single GET `/callback`, read `code`+`state`,
  validate `state`, return a tiny "you can close this tab" HTML, hand the
  code back to `complete_login`. Timeout + cancel if the user abandons.

### CLI reverse proxy (NEW, `crates/core/src/proxy/local_relay.rs` or similar)

- A plain-HTTP loopback server the CLI tools point their base URL at
  (`ANTHROPIC_BASE_URL=http://127.0.0.1:PORT`, Codex `base_url`, OpenCode
  `baseURL`). It injects `x-gate-authorization: Bearer <fresh token>` +
  `x-gate-upstream-url` and forwards over TLS to the real gateway. Shares
  the `watch::channel` token so refresh is invisible; **no CA trust and no
  credential in any config file** (directly serves design principle #1:
  "no config files").
- Tool integrations change: `claude_code.rs`/`codex.rs`/`opencode.rs` stop
  writing `X-Gate-Api-Key`/headers and instead write the loopback base URL.
  `has_upstream_credential`/drift checks updated accordingly.
- **Tradeoff** this reverse proxy must be running whenever a
  CLI tool makes a request. Today CLI config is static and survives the app
  being closed. New design requires Gate Connect running (menubar app,
  launch-at-login already exists) and disconnect/restore the config on close.

### MITM engine (GUI/system-proxy apps)

- `EngineConfig`/`GateHandler`: add a `token` `watch::Receiver<Arc<str>>`
  alongside `api_key`. `apply_rewrite` injects `x-gate-authorization` when
  an OAuth token is present, else falls back to `x-gate-api-key` (legacy).
  Add `update_token` mirroring `update_api_key`.
- `proxy::manager().refresh_token(...)` mirrors `refresh_api_key` across
  `manager.rs` / `manager_windows.rs` / `manager_linux.rs`.

### Account model: auth mode

- `AccountFile` gains `auth_mode: AuthMode` (`OAuth` | `ApiKey`, default
  `ApiKey` so existing installs load unchanged - mirrors the PAC
  back-compat pattern in `system_proxy.rs`). New `enum AuthMode` (the
  codebase has no credential-kind enum yet; this introduces it).
- `reconcile()` extended: an OAuth account with no tokens + no refresh →
  signed-out; zero-residue on `clear()` must also delete the oauth-tokens
  keychain entry (honors the disconnect-zero-residue contract tested in
  `crates/core/tests/disconnect_zero_residue.rs`).

## Tauri layer (`src-tauri/src/lib.rs`)

- New commands: `oauth_begin_login()` (opens browser via `opener`, starts
  loopback listener, returns once tokens stored), `oauth_status() ->
  { signed_in, expires_at, email }`, `oauth_sign_out()`,
  `set_auth_mode(mode)`. Register in the `generate_handler!` blocks
  (`:642`, `:676`).
- `.setup()` (~`:765`): after existing init, spawn the silent-refresh
  check - if `auth_mode == OAuth` and a `refresh_token` exists and the
  access token is expired/near-expiry, refresh and push into engine +
  relay; on failure, set a "needs sign-in" state the UI reads. Never
  auto-open the browser (per decision #4).
- A background refresh timer (or refresh-on-demand just before injection)
  keeps the token fresh while running; simplest: refresh in the relay/engine
  when `expires_at` is within the skew margin, single-flight guarded.
- `opener` capability (`capabilities/default.json`): add an allow entry for
  the Cognito hosted domain so `oauth_begin_login` can open it.

## Frontend (React)

- `src/lib/api.ts`: add `oauthBeginLogin()`, `oauthStatus()`,
  `oauthSignOut()`, `setAuthMode()`; extend `Account`/status types with
  `authMode` + oauth status.
- New `src/screens/SignIn.tsx` (or extend `AccountForm.tsx`): primary CTA
  "Sign in with Constellation" (opens Hosted UI); a secondary/expandable
  "Use an API key instead (legacy)" that keeps the current paste-key form.
  Follow cg tokens - ink primary button, mono for identifiers, shadow-as-
  border, one-room popover panel (no nested modal).
- `src/App.tsx`: top-level state gains oauth status; route to SignIn when
  signed out / refresh failed; show connected state otherwise.
- `src/screens/Settings.tsx`: show signed-in identity (email) + "Sign out",
  and the legacy key prefix only when in API-key mode.

## Build-time config

- Add `src/lib/config.ts` (or a Rust-side const) values for Cognito:
  hosted domain, app client id, region, user pool id, scopes - baked via
  `VITE_COGNITO_*` per environment, exactly like the gateway base URL is
  baked via `VITE_GATE_DEFAULT_BASE_URL`. The Rust oauth module needs the
  client id + hosted domain + scopes; pass them from the frontend on the
  login command, or bake into the binary via env at build (prefer baking so
  the flow works before any UI config).

## Testing

- **oauth unit** - PKCE challenge derivation, authorize-URL construction,
  token-response parsing, expiry/skew logic (pure, no network).
- **token exchange/refresh** - against a mock `/oauth2/token` loopback
  server, mirroring the `proxy_e2e.rs` mock-gateway pattern
  (`crates/core/tests/proxy_e2e.rs`).
- **engine e2e** - extend `proxy_e2e.rs` to assert `x-gate-authorization`
  is injected (and client `Authorization` still passes through untouched).
- **CLI reverse proxy e2e** - tool → loopback relay → mock gateway asserts
  the token header + upstream-url, and that refresh swaps the header live.
- **integration config** - Claude Code/Codex/OpenCode now write the
  loopback base URL and **no** credential header; drift + zero-residue
  disconnect updated (`account_reconcile.rs`, `disconnect_zero_residue.rs`,
  `cli_flows.rs`). Keep the `GATE_CONNECT_TEST_HOME` /
  `GATE_CONNECT_TEST_SECRETS` seams.
- **CLI** - a `gate-connect login --oauth` path (device-less: prints the URL
  / opens browser + loopback) parallel to the existing `Login` command
  (`crates/cli/src/main.rs:137`), tested hermetically.

## Sequencing / milestones

1. `oauth.rs` (PKCE, token exchange/refresh, keychain storage) + unit/mock
   tests. No UI yet.
2. Loopback redirect listener + `oauth_begin_login` command; verify a full
   browser round-trip end-to-end manually.
3. MITM engine: `x-gate-authorization` injection + token watch/hot-swap +
   manager `refresh_token`; extend `proxy_e2e.rs`.
4. `AuthMode` in account model + reconcile/zero-residue + startup silent
   refresh in `.setup()`.
5. CLI reverse proxy + rewrite Claude Code/Codex/OpenCode integrations off
   config-embedded creds; update drift/disconnect tests.
6. Frontend SignIn screen + Settings identity + api.ts wiring.
7. CLI `login --oauth`.

## Risks / open items

- **Cognito loopback callback allow-list**: exact allowed redirect URLs /
  port strategy is server-side config the user must set.
- **Exact header name** (`x-gate-authorization` here) is a placeholder for
  the user's server-side contract.
- **Refresh-token lifetime / rotation**: if Cognito rotates refresh tokens,
  persist the new one on every refresh; if the refresh token itself expires,
  fall back to interactive sign-in (already the plan).
- **Windows/Linux parity**: loopback listener, opener, and manager
  `refresh_token` must be implemented across all three `manager_*` files.
