# PAYG support - implementation plan

Status: **backend implemented 2026-08-20** on `feat/new-app-ui`, against
gate-connect-app `45d4ba6` and gate `main` (`59ba9bd`). The sections below are
the plan as built; §10 lists what landed and what deliberately did not.

Two things changed between the first draft and the build, and both shrank the
work:

1. **The app moved to a relay/MITM architecture.** Tool configs no longer carry
   Gate headers at all, so PAYG is no longer a per-integration header edit. It
   is a behavior change in the two shared rewrite paths (§4.2, §4.3).
2. **OAuth (Cognito) gateway auth shipped on both sides**, and PAYG works with
   it unchanged (§6).

Scope: **backend only.** How the user turns PAYG on in the desktop UI is
**deliberately undefined** (§3); the CLI (`gate-connect billing-mode`) is the
surface that drives and verifies it in the meantime.

## 1. The gateway contract

The gateway infers billing mode **from the request shape - there is no flag on
the API key** (migration 29 dropped `gateway_api_keys.billing_mode`). Source:
`apps/gateway-proxy/src/proxy/stages/resolve-upstream.ts:120-128`.

```
isByokRequest():
  1. X-Gate-Upstream-Url header present?        → BYOK
  2. API key is a passthrough (provider) token? → BYOK
  otherwise                                     → PAYG (reseller)
```

In PAYG the gateway picks an enabled provider account on the org, forwards with
*its* credentials, and debits the org's `payg_balance_cents`.

A PAYG request must carry:

| Field | PAYG | Notes |
|---|---|---|
| A Gate credential | **yes** | Either `X-Gate-Api-Key: sk-gw-…`, or `X-Gate-Authorization` + `X-Gate-Org-Id` (§6). Must resolve to an **org**. |
| `X-Gate-Upstream-Url` | **omit** | Its absence is the PAYG switch. |
| No provider token in `Authorization` / `x-api-key` | **yes** | Any non-`sk-gw-` value there is classified as a passthrough token, which forces BYOK and is then rejected 400 `missing_upstream_header` (`resolve-upstream.ts:305-315`). This is the whole of §2. |
| `model` in the JSON body | yes (already there) | The tool sends it; the gateway reads it from the body. **We never inject or rewrite it** - but see the open item in §8. |
| `X-Gate-Provider` header | optional | Pins a provider account; otherwise the gateway auto-routes (operator pref → priority → cheapest → live discovery). |

### Non-goals

- **No body rewriting / request wrapping.** Headers and tool config only.
- **We do not store an upstream provider credential locally.** In PAYG the
  *gateway* holds it. The `Integration` trait's `*_upstream_credential` methods
  stay no-ops.

### Server-side prerequisites (operator-side, not app-side)

Not headers we send, but PAYG fails without them, so the app must surface the
gateway's error envelopes (§8):

| Requirement | Failure if missing |
|---|---|
| The credential resolves to an org | 400 `no_route` |
| Org has a funded `payg_balance_cents` | 402 `insufficient_balance` (or `payg_disabled`, see below) |
| Org has an enabled provider account serving the requested model | 400 `no_route`, with `suggested_models` in the envelope |
| Model is priceable | 503 `pricing_unavailable` |

**Correction to the first draft:** `paygEnabled` is *not* an opt-in entitlement
defaulting to false. **Any positive top-up sets it**
(`apps/dashboard-api/src/billing/payg.repository.ts:545-546`), so funding *is*
activation, and the gateway's own `payg_disabled` copy was rewritten to say
"add credits" rather than "enable it in /billing"
(`payg-balance-gate.ts`, `PAYG_INACTIVE_MESSAGE`). Our error copy must not send
users looking for a toggle that does not exist. Practically, `payg_disabled` and
`insufficient_balance` are the same message to the user: top up.

There is one more refusal worth handling separately: the balance gate can return
**503 with `retryable: true`** under hold contention. Nothing was charged; retry
shortly (`payg-balance-gate.ts:334-344`).

## 2. The one rule: nothing but the Gate credential may authenticate

Confirmed in `apps/gateway-proxy/src/auth/guards/api-key.guard.ts`:

- A **passthrough token** is the first non-`sk-gw-` value found in
  `Authorization` / `x-api-key` (`pickPassthrough`). Its presence **forces
  BYOK**, which without an upstream URL is a 400. A leftover `sk-ant-…` /
  `sk-…` / `sk-ant-oat01-…` therefore *breaks* PAYG - it is not merely ignored.
- The gateway never forwards the client's auth in reseller mode anyway:
  `buildForwardHeaders` (`utils/proxy-helpers.ts`) drops inbound
  `authorization`, `x-api-key`, and **every** `x-gate-*` header, then the
  provider handler attaches the provider account's own credential (direct /
  OpenAI-compatible per `authScheme`, Bedrock SigV4, Vertex server-side OAuth).

Two ways to satisfy the rule. The first draft chose (a) for every integration;
the current architecture makes (b) available and much cheaper:

- **(a) Put the Gate key in the tool's own auth slot.** Works (an `sk-gw-`
  value is read as the gateway key, de-duped against `X-Gate-Api-Key`, and
  stripped upstream), but it means writing our credential into each tool's
  config, snapshotting what was there, and restoring it on disconnect.
- **(b) Strip the tool's auth header in flight.** The relay and MITM engine
  already own every intercepted request, and the Gate credential travels in its
  own header, so removing `authorization` / `x-api-key` on the rewrite path is
  sufficient and touches no tool config. **This plan chooses (b).**

## 3. Mode placement, and why the UI is out of scope

**Account-level `BillingMode` (one mode for all tools), default `Byok`.** This
matches the product's "enter once, reused for every tool" model and keeps the
snapshot/restore and master-switch logic untouched. Per-provider mode is a
possible future extension (would move the field onto `Provider`/`ProviderState`
and into the snapshot); out of scope.

**The desktop UI affordance is undefined and not part of this plan.** What ships
here is the mechanism plus a headless way to drive it:

- `account.json` carries `mode`, so it is settable and inspectable.
- The CLI carries `gate-connect billing-mode [byok|payg]` (§4.6), so the whole
  path is drivable and verifiable end to end without any UI work.
- `src-tauri` exposes the mode on the existing account-status command so the UI
  can *read* it when a design exists. No new UI is written; no segmented control,
  no onboarding copy changes. When the UI is designed it consumes the same
  command and the same setter.

Consequence for §4.5: a mode change must reconfigure everything already
connected, and that logic belongs in the Tauri/CLI command layer, not in a
future UI.

## 4. File-by-file changes

### 4.1 `crates/core/src/account.rs`

- Add, next to the existing `AuthMode`:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
  #[serde(rename_all = "lowercase")]
  pub enum BillingMode { #[default] Byok, Payg }
  ```
- Add `billing_mode: BillingMode` to `Account` and to `AccountFile`
  (`#[serde(default)]`, exactly like `auth_mode`, so existing `account.json`
  files load as `Byok` - no migration).
- `load()` populates it.
- Add `billing_mode() -> Result<BillingMode>` (no keychain read, mirrors
  `auth_mode()`) for `status()` checks and the status command, plus
  `billing_mode_for_injection()` for the proxy managers - infallible, falling
  back to `Byok` so an unreadable account degrades to "the provider bills the
  user", never to spending an org's balance.
- **`save`'s signature is unchanged.** As built, the mode is set by its own
  `set_billing_mode()`, exactly like `set_auth_mode()`, and `save` /
  `switch_gateway` carry the persisted value forward. That is the safer shape:
  `save` runs on every key rotation and URL edit, and a mode it had to be told
  each time is a mode one forgetful caller silently resets.

### 4.2 `crates/core/src/proxy/relay.rs` - the main change

This is where the four config integrations (Claude Code, Codex, OpenCode,
OpenClaw) now get their Gate headers: their configs hold **one base URL and no
headers** (see the module doc, and the OpenCode test asserting no
`X-Gate-Api-Key` / `X-Gate-Upstream-Url` in the written config).

- Add a `mode` watch-channel to `RelayState`, alongside `token` / `api_key` /
  `org` / `intercept`.
- In the `Route::Rewrite` arm of the request handler (~`relay.rs:472-495`), when
  the mode is `Payg`:
  - **skip `set_upstream_header`**, and
  - **remove `authorization` and `x-api-key`** from the cloned header map (§2).
- `Route::Passthrough` is untouched. It serves account/metadata paths (e.g.
  Claude Code's `/api/oauth/usage`) and the non-intercepting Linux daemon, both
  of which must keep the tool's own credential and never see a Gate header.
- **Free win, worth an assertion in tests:** the relay already strips the
  catalog's upstream path prefix from the request line (the `/api` in
  OpenRouter's entry rides in the upstream URL). With no upstream header, what
  reaches the gateway is the plain `/v1/chat/completions` - which is exactly the
  path the reseller path wants. No extra path handling needed.

### 4.3 `crates/core/src/proxy/engine.rs` + `mod.rs`

Same change on the MITM side (Cowork/Claude Desktop, Gemini, OpenRouter, and any
other proxy-routed domain).

- Add `mode: BillingMode` to `EngineConfig` (next to `oauth_token` / `org_id`),
  plus `mode_tx`/`mode_rx` and `RunningEngine::update_mode`, mirroring
  `update_token` / `update_org` exactly (`engine.rs:111-118`, `:790-804`).
- **`apply_rewrite` has drifted from the first draft's signature.** It is now
  `apply_rewrite(req, gateway, upstream_url, api_key, oauth_token, org_id)`
  (`engine.rs:563-570`), and credential injection is factored into
  `super::inject_gate_credential` shared with the relay. Add the mode and, in
  `Payg`: skip the `UPSTREAM_URL_HEADER` insert (`engine.rs:597-600`) and strip
  the client's `authorization` / `x-api-key`.
- Decide *where* the strip lives: putting it in `inject_gate_credential`
  (`proxy/mod.rs:530-573`) gets both callers at once and keeps the two paths
  from drifting, which is the stated reason that helper exists. Preferred.
- **Why the strip matters most here:** Cowork and Gemini CLI send their own
  provider credential (an `sk-ant-oat01-…` OAuth bearer) that we cannot change
  through any config file - only by rewriting the in-flight request.
- **UX note:** with the strip, Cowork no longer needs a working Anthropic
  sign-in for traffic to flow in PAYG. It may still prompt on its own; that is
  outside our control and worth a release note.
- **Consumer chats stay out of PAYG (decided, and expected).** The
  consumer-chat entries (`claude-web`, `chatgpt-apps`) are session-cookie
  surfaces whose paths are not the inference-API shapes the reseller path
  serves, and their traffic is covered by the user's subscription rather than
  billed by Gate (the dashboard already treats their cost as an estimate). So
  PAYG applies to the API domains only: `anthropic`, `openai`, `openrouter`,
  `google`/`google-codeassist`. The consumer entries keep today's BYOK/
  passthrough behavior in *both* modes, i.e. the mode check is per-domain, not
  global: rewriting them without an upstream header would break them for no
  gain.

### 4.4 Managers and the Linux helper (IPC change)

- macOS: `manager.rs:225` - set `mode` on the in-process `EngineConfig`.
- Windows: `manager_windows.rs:231` - same.
- **Linux does not build `EngineConfig` in the manager.** The engine runs in the
  detached privileged helper; the construction site is `helper.rs:304`, fed over
  IPC by `control::Request::SetIntercept` (`control.rs:124-136`). So threading
  the mode is an **IPC-protocol change**: add `#[serde(default)] mode` to
  `SetIntercept` (default keeps an older client parseable), thread it through
  `helper_client::set_intercept`, and pass it to `EngineConfig` in
  `handle_request`.
- Manager `refresh_mode(mode)` → `running.update_mode(...)`, parallel to
  `refresh_api_key`. On Linux there is no separate update message:
  `refresh_api_key` re-sends `SetIntercept` and the helper applies it live to the
  running engine, so `refresh_mode` folds into the same re-send. No new `Request`
  variant.

### 4.5 `src-tauri/src/lib.rs`

- The account setter takes the mode and passes it to `account::save`.
- **On a mode change**, call `manager().refresh_mode(mode)`. Tool configs
  themselves do **not** need rewriting under the (b) design - that is the payoff
  of §2 - so `provider::enable` re-runs are only needed if §4.7 turns out to be
  required for Codex.
- Extend the account-status command to return the current mode (read-only, for a
  future UI).

### 4.6 `crates/cli`

The only user-facing surface in this plan. Add the mode to `login` (or a small
`mode` subcommand) and print it in `whoami` / `status`, so PAYG is drivable and
verifiable headlessly. Mirror whatever shape `--oauth` already uses in
`cli_flows.rs` so the tests stay uniform.

### 4.7 Config integrations - mostly nothing, one open question

Under design (b) there is **no** per-integration change: no managed
`ANTHROPIC_API_KEY`, no Codex `[auth] command` swap, no OpenCode
`options.apiKey` override, no OpenClaw `apiKey` override, and **no
`previous_api_key` addition to the OpenClaw sidecar snapshot**. All of that
(§§3.4-3.6.1 of the first draft) is obsolete: those integrations write a base URL
and no headers, and the relay now owns the credential.

Two items survive, one of them settled below:

- **Codex in PAYG: an unauthenticated provider block (resolved against the
  Codex docs).** Today `connect()` calls `read_auth_mode()`, which hard-fails
  when `~/.codex/auth.json` is absent, because `requires_openai_auth = true`
  needs a `codex login` session and the relay base URL suffix differs by mode
  (`relay_base_url_for`, `codex.rs:143-147`). In PAYG the user may never have
  run `codex login`, and we do **not** want their OpenAI credential anyway.

  The Codex docs document three mutually exclusive provider auth options, the
  third of which is exactly what PAYG wants: "If you don't set
  `requires_openai_auth` (or set it to `false`) and you don't set `env_key`",
  then "Codex assumes the provider doesn't require authentication" - offered for
  local models, and our `base_url` **is** a loopback endpoint. So in PAYG:
  - **omit `requires_openai_auth` and `env_key`.** Codex then sends no
    `Authorization` at all, which is precisely what §2 wants: no passthrough
    token to force BYOK, nothing for the relay to strip, and no credential of
    ours on disk.
  - **skip `read_auth_mode()`** and force the apikey path shape (`/v1`, the
    `api.openai.com` catalog entry), since the ChatGPT `/backend-api/codex`
    shape is subscription-only and therefore never PAYG.
  - **`status()` must become mode-aware**: it currently reports drift when
    `requires_openai_auth` is missing (`codex.rs:278-289`), which is the correct
    PAYG shape.

  **Explicitly rejected: writing a fake `auth.json`.** It is Codex's own
  credential store with an undocumented, version-drifting schema; it would make
  Codex claim a login it does not have and collide with a later real
  `codex login`; deleting it safely needs ownership tracking we do not have; and
  a dummy credential 401s on the `Route::Passthrough` account/metadata paths
  instead of failing in one obvious place. None of that is needed now.

  **Fallback if Codex turns out to demand a bearer anyway:** the docs define
  `[model_providers.<id>.auth]` with `command` (must print the token to stdout),
  `args`, `cwd`, `refresh_interval_ms` (default 300000) and `timeout_ms`
  (default 5000), and state "Do not combine with `env_key`,
  `experimental_bearer_token`, or `requires_openai_auth`" - which confirms the
  mutual-exclusivity note at `codex.rs:408-409`. We still have
  `helper_script_path()` / `HELPER_FILENAME` and the disconnect-time cleanup, so
  reviving the helper is a small change; point it at a script that prints the
  gate key from the keychain (fail-safe: an `sk-gw-` bearer is read as the
  gateway key rather than forcing BYOK). `experimental_bearer_token` is
  documented but discouraged, and would put a secret in `config.toml` - do not.

  **Separate constraint found while validating this:** PAYG Codex needs a
  provider account that natively serves the **Responses API**. Reseller mode
  delegates to the provider handler with the request path preserved
  (`forward-request.ts:552-575`) and there is no Responses → Chat Completions
  translation anywhere in the gateway, so `/v1/responses` only routes to an
  OpenAI / OpenAI-compatible upstream that speaks it. A PAYG org with only
  Bedrock or Anthropic accounts cannot serve Codex, and will fail
  `no_route`. Worth saying in the error copy (§8).
- **Drift checks.** Confirm no other integration `status()` asserts anything that
  changes under PAYG. Since none of them write Gate headers any more, this is
  expected to be a no-op - confirm rather than assume.

Docs consulted: <https://learn.chatgpt.com/docs/auth> (provider auth options)
and <https://learn.chatgpt.com/docs/config-file/config-reference>
(`model_providers` field reference). Append `.md` to either URL to read the
source markdown.

## 5. Tests

- `account`: legacy `account.json` (no `billing_mode`) → `Byok`; round-trip with
  `Payg`.
- `relay`: in `Payg`, the rewritten request has **no** `x-gate-upstream-url`,
  **no** `authorization`, **no** `x-api-key`, and still carries the Gate
  credential; in `Byok` all of today's assertions hold unchanged. Extend the
  existing `relay_e2e.rs` cases.
- `relay`: `Route::Passthrough` in `Payg` still forwards the tool's own
  `Authorization` and injects nothing.
- `engine`: extend `rewrite_swaps_authority_keeps_path_and_injects_headers` -
  `x-gate-upstream-url` absent in `Payg`, present in `Byok`, client auth stripped
  in `Payg`.
- `control`/`helper`: `SetIntercept` round-trips the mode; a payload without the
  field parses as `Byok`.
- `cli_flows`: the new flag persists the mode and `whoami`/`status` reports it.
- Manual: against a funded org with an enabled provider account, in **both**
  api-key and OAuth auth modes (§6).

## 6. Does PAYG support OAuth on the gate side? Yes.

Verified in gate `main`. The gateway accepts a Cognito access token as an
alternative to an `sk-gw-` key, and PAYG needs nothing extra for it.

- **Headers:** `X-Gate-Authorization: Bearer <cognito access token>` plus
  **required** `X-Gate-Org-Id`. A 4th, independent slot that the BYOK
  3-slot extraction never reads, and stripped upstream like every `x-gate-*`
  header (`api-key.guard.ts:83-101`).
- **Permanently enabled**, not behind a rollout flag: "OAuth-token auth is
  permanently on" (`config/cognito.config.ts`).
- **Why PAYG just works:** the guard builds a synthetic `ApiKeyMeta` carrying
  `orgId` (from the header, after verifying an active membership) and
  `allowedModels: []` (`cognito-auth/cognito-request-auth.service.ts:80-100`).
  Everything downstream - the balance gate, reseller routing, the debit - keys on
  `ctx.orgId`, not on a key row. So the §1 contract is satisfied identically.
- **Mutually exclusive with a key.** A request carrying both
  `X-Gate-Authorization` and an `sk-gw-` value is rejected
  `ambiguous_gateway_auth`. Our `inject_gate_credential` already enforces
  one-or-the-other, so nothing to change.
- **§2 still applies in OAuth mode.** A passthrough token is still extracted and
  attached to the OAuth-authed request (`api-key.guard.ts:98-99`), so it still
  forces BYOK. The strip is required in both auth modes - it is not an api-key-
  mode workaround.
- **Known limitation, accepted:** `recordUsage` is keyed on a real
  `gateway_api_keys.id`, so a synthetic `cognito:<userId>:<orgId>` id records
  nothing. The dashboard's per-key activity view will not show OAuth-authed
  traffic; **org-level PAYG billing is unaffected** (it keys on `orgId`).
- **Corollary worth noting:** OAuth mode suits PAYG *better* than a static key.
  Access tokens expire, and only the relay/engine can refresh them per request -
  which is exactly the design the app already has, and another reason design (b)
  beats writing credentials into tool configs.

`GET /v1/me/orgs` (OAuth-only, `cognito-auth/org-selection.controller.ts`) is
already what the app calls to pick the org, and `account.rs` already persists the
selection. No new work for PAYG.

## 7. What changed from the first draft

Recorded so the diff is reviewable rather than mysterious:

1. **§§3.4-3.6.1 deleted.** Per-tool header shaping and credential overrides are
   obsolete: config integrations write a base URL and no headers, and the relay
   injects everything. PAYG is now two shared code paths, not five.
2. **The OpenClaw `previous_api_key` snapshot change is dropped** with them - it
   only existed to restore a key we no longer overwrite.
3. **`paygEnabled` corrected** (§1): funding activates PAYG; the "opt-in,
   default false even on paid" claim was wrong, and the error copy follows.
4. **The legacy `x-gateway-api-key` claim removed.** The first draft listed it as
   an accepted credential slot. It is no longer accepted as one
   (`api-key.guard.ts:31-32`), and we do not need it - we send the canonical
   `X-Gate-Api-Key`.
5. **`apply_rewrite`'s signature and the `EngineConfig` construction sites
   re-verified** against current line numbers; credential injection is now
   shared via `inject_gate_credential`.
6. **UI descoped** (§3), CLI added as the drivable surface.
7. **OAuth answered** (§6), and shown to need no PAYG-specific work.
8. **Two would-be open items closed**: consumer-chat domains stay BYOK by design
   (§4.3), and the model-id worry was overstated - the gateway already maps the
   bare / dated / dotted vendor ids our tools send (§8).

## 8. Open items

- **Model ids: no app-side work, and the gateway already maps the forms our
  tools send.** "We never inject the model" holds, and the earlier worry that a
  Bedrock/Vertex-backed org would 400 on Claude Code's native id was overstated.
  `applyAliasRewrite` (`resolve-upstream.ts:130-220`) rewrites a bare vendor
  short name via, in order: a live Bedrock candidate's native id, the static
  `GLOBAL_ALIASES` map, then a **canonical-identity fallback** whose own comment
  names this exact case ("Claude Code sends `claude-fable-5` (or the dated
  `claude-fable-5-20260115`)"). Verified against
  `packages/upstream-providers/src/canonical-id.ts`:

  ```
  claude-sonnet-4-5           -> anthropic/claude-sonnet-4-5
  claude-sonnet-4-5-20250929  -> anthropic/claude-sonnet-4-5
  claude-haiku-4.5            -> anthropic/claude-haiku-4-5
  gpt-5.2                     -> openai/gpt-5-2
  ```

  Dated and dotted forms collapse onto the canonical the discovery row is stored
  under, and the canonical path then applies the full provider precedence and
  translates to each account's native id. Two residual gaps, both gateway-side
  and both failing loudly rather than silently:
  - a model the live discovery index does not know yet (cold start before the
    first discovery tick, or a genuinely unlisted model) 400s `no_route` with
    `suggested_models`;
  - dotted / Bedrock-shaped ids (`anthropic.claude-…-v1:0`) are deliberately not
    canonicalized by the rewrite, because the `-vN:M` build suffix is
    version-significant, so they must already be a discovered native id.

  So: **we ship no model override.** If the mapping needs to widen, that is a
  gateway change (extend the alias/canonical path), not a Gate Connect one. The
  app's part is only to surface the 400 well, and it can attribute a rewrite in
  diagnostics via the `alias-from` / `alias-to` response headers the gateway
  already sets.
- **Error surfacing.** Map `insufficient_balance` / `payg_disabled` (one message:
  top up), `no_route` (with `suggested_models`), `pricing_unavailable`, and the
  retryable 503 to `ErrorBlock` copy. The gateway also sets
  `x-gate-error-source: gateway` on all of these, which the relay can key on
  without parsing bodies.
- **Codex needs an OpenAI-compatible provider account** that serves the
  Responses API (§4.7); a Bedrock/Anthropic-only PAYG org cannot serve it.

## 9. Sequencing

1. `account.rs` mode + `load_billing_mode` + CLI flag (no behavior change: every
   caller passes `Byok`).
2. `inject_gate_credential` / relay / engine behavior behind the mode, with the
   tests in §5.
3. Manager wiring + the Linux `SetIntercept` field + `refresh_mode`.
4. Error copy, then the open items in §8. UI whenever it is designed.

## 10. What landed (2026-08-20)

Backend complete; no UI. Names as built, since a few differ from the draft:
`Account::billing_mode` (not `mode`), `account::billing_mode()` /
`billing_mode_for_injection()` / `set_billing_mode()`, and
`ProxyManager::refresh_mode()` taking no argument - it reads the persisted mode
rather than being told one, so a live engine can never route under a mode the
account does not hold.

| Area | Change |
|---|---|
| `account.rs` | `BillingMode { Byok, Payg }`, `#[serde(default)]` on the file field, setter + two readers. `save`, `switch_gateway` carry it over. |
| `proxy/mod.rs` | `inject_gate_credential` takes the mode and strips `authorization` / `x-api-key` in PAYG; `PAYG_ELIGIBLE_SLUGS` + `effective_billing_mode`; `Decision::Rewrite` carries the slug. |
| `proxy/relay.rs` | `mode` watch-channel; the `Route::Rewrite` arm skips (and removes) the upstream hint in PAYG; `serve()` seeds and refreshes it. |
| `proxy/engine.rs` | `EngineConfig::billing_mode`, `mode_tx`/`mode_rx`, `RunningEngine::update_mode`, `apply_rewrite(.., mode)`. |
| `proxy/control.rs`, `helper_client.rs`, `helper.rs` | `SetIntercept` carries `#[serde(default)] billing_mode`; the daemon applies it live via `update_mode`. |
| `manager*.rs` | Seed `EngineConfig`; `refresh_mode()` on all three (Linux folds into the `SetIntercept` re-send). |
| `registry.rs`, `provider.rs` | `ConnectInput::billing_mode`, populated from the account at all four sites. |
| `integrations/codex.rs` | PAYG writes an unauthenticated provider block and pins the `/v1` shape; `status()` is mode-aware in both directions. |
| `src-tauri/src/lib.rs` | `set_billing_mode` command, `billing_mode` on `AccountDto`, `reapply_codex_for_mode`. |
| `crates/cli` | `gate-connect billing-mode [byok|payg]`; `whoami` reports it. |
| `src/lib/api.ts` | `BillingMode` type, `Account.billing_mode`, `setBillingMode` - type mirror only, no UI. |

Tests: `tests/billing_mode.rs` (6, persistence incl. the no-field upgrade case),
`tests/codex_billing_mode.rs` (4, both provider shapes + the no-`auth.json`
case + drift both ways), 3 new relay e2e cases (PAYG strip, ineligible domain
stays BYOK, passthrough untouched), 2 new `apply_rewrite` cases, 3 new
`proxy/mod.rs` unit tests (eligibility, catalog cross-check, strip behind a
caller-supplied key), 2 new `control.rs` IPC cases.

**Not done, deliberately:** the UI (§3); error-copy mapping for the gateway's
402/503 envelopes (§8) - the codes are documented there but nothing consumes
them yet; and no audit event for a mode switch, unlike `set_auth_mode`, which
emits one. That last one is a real gap worth closing: who pays is at least as
audit-worthy as which credential authenticates.

**Not compiler-verified:** `manager.rs` (macOS) and `manager_windows.rs` are
`cfg`-gated, so a Linux `cargo check` never builds them. Both edits mirror the
Linux/shared paths exactly (one `EngineConfig` field, one `refresh_mode`), but
CI on those platforms is what actually proves it.
