# Plan: Run Gate Connect against the local Gate stack + fake Cognito

Goal: drive the **`gate-connect` CLI/app in OAuth mode** (`X-Gate-Authorization` +
`X-Gate-Org-Id`) against the locally-running Gate stack, using the fake-Cognito
(`E2E_LOCAL_JWT`) verifier instead of a real Cognito pool. This exercises the
`feat/oauth-authenticate-proxy-calls` gateway path end-to-end on a laptop.

## Context / what we already verified

- The local stack is up in fake-Cognito mode: `infra-dashboard-api-1` (:3001) and
  `infra-gateway-proxy-1` (:3000), both with `E2E_LOCAL_JWT=1`,
  `E2E_JWT_ISSUER=local-e2e`, `COGNITO_USER_POOL_ID=us-east-1_e2elocal`,
  `COGNITO_CLIENT_ID=e2e-client-id`, and the test JWKS injected via `E2E_JWKS_JSON`.
- We can mint gateway-accepted access tokens with `gate/e2e/auth/mint.ts` for the
  seeded identity **OWNER_A** (`sub=e2e-owner-a-sub`, `email=owner-a@e2e.test`),
  who owns **ORG_A** (`id=a0000000-0000-4000-8000-000000000001`).
- The gateway serves `GET /v1/me/orgs` (X-Gate-Authorization) via
  `apps/gateway-proxy/src/cognito-auth/org-selection.controller.ts` — the exact
  endpoint Gate Connect's OAuth login calls to drive its org picker.

## Two blockers (from reading the connect app)

**B1 — Gate Connect requires an HTTPS base URL.**
`crates/core/src/account.rs:131` rejects any `gateway_base_url` not starting with
`https://`, and `crates/core/src/org.rs` calls `<base_url>/v1/me/orgs` with a
default `reqwest` client that verifies TLS against the system trust store. The
local gateway is plain HTTP on `:3000`. → We must front it with a **trusted**
HTTPS terminator.

**B2 — the fake Cognito has no login server.**
`--oauth` (`crates/core/src/oauth.rs`) opens the real Cognito **Hosted UI**
(`https://$GATE_COGNITO_HOSTED_DOMAIN/oauth2/authorize`) and exchanges the code at
`/oauth2/token`. Our fake Cognito is JWKS-only, and a token from a *real* pool
would be rejected by the local gateway (wrong signing key). So the locally-minted
token must be bridged into the app without a real Hosted-UI round-trip.

Two facts make B2 tractable without a full mock IdP:
- The CLI **prints** the authorize URL and blocks on a loopback listener
  (`http://127.0.0.1:<port>/callback`, ports tried: 8977–8979). We can complete
  the callback ourselves instead of using a browser.
- The token exchange endpoint is overridable via the test seam
  **`GATE_CONNECT_TEST_TOKEN_ENDPOINT`** (`crates/core/src/oauth.rs`). Point it at
  a tiny local HTTP endpoint that returns a minted token.
- `OAuthConfig::from_build_env` reads `GATE_COGNITO_HOSTED_DOMAIN` /
  `GATE_COGNITO_CLIENT_ID` / `GATE_COGNITO_SCOPES` with **runtime env taking
  precedence** over build-time values — so no rebuild is needed to configure it.

## Recommended approach: OAuth mode, hand-completed login + mock token endpoint

### Step 1 — Confirm the local stack (no action if already up)
```bash
docker ps --format '{{.Names}} {{.Status}}' | grep -E 'gateway-proxy|dashboard-api'
curl -sf http://localhost:3000/readyz && echo gateway-ok
```

### Step 2 — Trusted HTTPS front for the gateway (solves B1)
```bash
mkcert -install                      # installs a local CA into the system trust store
mkcert localhost 127.0.0.1           # -> localhost+1.pem / localhost+1-key.pem
# terminate TLS on 8443 -> plain gateway on 3000 (pick one):
npx local-ssl-proxy --source 8443 --target 3000 \
    --cert localhost+1.pem --key localhost+1-key.pem
# or: caddy reverse-proxy --from https://localhost:8443 --to http://localhost:3000
```
Verify the front is trusted by the same trust store reqwest uses:
```bash
curl -sf https://localhost:8443/readyz && echo https-front-ok
```
Base URL for Gate Connect = `https://localhost:8443`.

### Step 3 — Mint a gateway-accepted access token (OWNER_A)
Reuse `gate/e2e/auth/mint.ts` (must run from `gate/e2e`, shim `__dirname`; client
id defaults to `e2e-client-id`, issuer to `local-e2e` — matches the gateway).
Produces an access token whose `aud=e2e-client-id`, `iss=local-e2e`,
`token_use=access`, signed by the test key the gateway's JWKS verifies.

### Step 4 — Tiny mock token endpoint (solves B2, token half)
A ~15-line HTTP server on `127.0.0.1:9444` that answers `POST /oauth2/token` with:
```json
{ "access_token": "<minted>", "refresh_token": "unused",
  "id_token": "<minted-id>", "token_type": "Bearer", "expires_in": 3600 }
```
(It can shell out to the mint script or read a pre-minted token from a file/env.
It ignores `code`/`code_verifier` — PKCE is not enforced locally.)

### Step 5 — Configure env + build the CLI
```bash
cd gate-connect-app
export GATE_COGNITO_HOSTED_DOMAIN=localhost:9443     # value is unused (we hand-complete authorize)
export GATE_COGNITO_CLIENT_ID=e2e-client-id          # MUST match gateway aud
export GATE_CONNECT_TEST_TOKEN_ENDPOINT=http://127.0.0.1:9444/oauth2/token
cargo build --bin gate-connect
```

### Step 6 — Run OAuth login and hand-complete the callback
```bash
./target/debug/gate-connect login --oauth --base-url https://localhost:8443
# It prints an authorize URL and waits. Parse `redirect_uri` (the 127.0.0.1:<port>/callback)
# and `state` from that URL, then:
curl "http://127.0.0.1:<port>/callback?code=anything&state=<state>"
```
The app exchanges `code` at the mock token endpoint → stores the minted
`OAuthTokens` in the OS secret store (service `ai.constellation.gate-connect`,
label `oauth-tokens`), then calls `https://localhost:8443/v1/me/orgs` to list orgs.

### Step 7 — Org selection + verify
- Login should list **ORG_A** and persist `auth_mode=oauth` + `org_id` in
  `account.json` (app-support dir).
- Verify state:
  ```bash
  ./target/debug/gate-connect status
  ```
- Verify a proxied inference call carries the OAuth headers and is accepted:
  ```bash
  curl -sS https://localhost:8443/v1/messages \
    -H 'content-type: application/json' \
    -H "x-gate-authorization: Bearer <minted-access>" \
    -H 'x-gate-org-id: a0000000-0000-4000-8000-000000000001' \
    -H 'x-gate-upstream-url: https://api.anthropic.com' \
    -H 'authorization: Bearer <your-sk-ant-...>' \
    -d '{"model":"claude-sonnet-4-20250514","max_tokens":64,"messages":[{"role":"user","content":"hi"}]}'
  ```
  (BYOK avoids needing a seeded PAYG provider/balance. For PAYG instead, ensure
  ORG_A has an enabled provider account + balance and drop the upstream/BYOK creds.)

## Alternative A — direct token injection (skip the login flow entirely)
Fewer moving parts than the mock token endpoint, but writes to the OS secret store
and bypasses the app's login code path:
- Mint OWNER_A token (Step 3).
- Write `OAuthTokens` JSON `{access_token, refresh_token, expires_at_unix}` into the
  secret store under service `ai.constellation.gate-connect` / label `oauth-tokens`
  (Linux: `secret-tool`; macOS: `security add-generic-password`).
- Write `account.json` with `gateway_base_url=https://localhost:8443`,
  `auth_mode=oauth`, `org_id=a0000000-…-01`, `org_name="E2E Org A"`.
- Still needs the HTTPS front (B1).

## Alternative B — API-key mode (no Cognito at request time)
Simplest, but does **not** exercise the OAuth/`X-Gate-Authorization` path:
- Mint a Cognito token (Step 3), POST it to `dashboard-api`
  `POST http://localhost:3001/api/v1/gateway/api-keys` → get `sk-gw-…`.
- `gate-connect login --base-url https://localhost:8443 --api-key sk-gw-…`.
- Still needs the HTTPS front (B1).

## Open questions / risks
- **mkcert / local CA install** modifies the system trust store — acceptable? If
  not, we need reqwest to accept the cert another way (e.g. `SSL_CERT_FILE`), which
  may not be wired in the connect client.
- **Redirect port registration:** ports 8977–8979 must be acceptable to the client
  config; with our hand-completed callback this is moot, but a real Hosted UI would
  require them registered on the Cognito app client.
- **Which surface to run** — the `gate-connect` CLI (this plan) vs the Tauri
  menu-bar app (`pnpm app`). The CLI is the tighter loop for verifying the OAuth
  contract; the Tauri app adds the tool-integration/proxy layer on top.
- Confirm the exact callback path and query param names from `oauth.rs` before
  running Step 6 (display-redaction obscured a few strings during research).
