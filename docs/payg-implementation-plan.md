# PAYG support — implementation plan

Status: ready to implement — gateway contract confirmed against `swarm-deck`
(§5). Gate Connect is hard-wired to **BYOK** today; this plan adds
**PAYG (pay-as-you-go / reseller)** as a selectable mode.

## 1. The gateway contract (confirmed from `swarm-deck`)

The gateway infers billing mode **from the request shape — there is no flag on
the API key.** Source: `apps/gateway-proxy/src/proxy/stages/resolve-upstream.ts`.

```
isByokRequest():
  1. X-Gate-Upstream-Url header present?        → BYOK
  2. API key is a passthrough (provider) token? → BYOK
  otherwise                                     → PAYG (reseller)
```

So Gate Connect is "set to BYOK" for exactly one reason: **every path it writes
emits `X-Gate-Upstream-Url`.** Removing that header (and only that header) flips
a request to PAYG. In PAYG the gateway picks an enabled provider account on the
org, forwards with *its* credentials, and debits the org's `payg_balance_cents`.

A PAYG request must therefore carry:

| Field | PAYG | Notes |
|---|---|---|
| `X-Gate-Api-Key: sk-gw-…` | **yes** | Must be a real gateway-issued, **org-scoped** key. A passthrough provider token forces BYOK and is rejected without an upstream header. |
| `X-Gate-Upstream-Url` | **omit** | Its absence is the PAYG switch. |
| `model` in the JSON body | yes (already there) | The agent already sends it; the gateway extracts it via `extractRequestModel(rawBody)`. **We never inject it.** |
| `X-Gate-Provider` header | optional | Pins a provider account; otherwise the gateway auto-routes (operator pref → partner → cheapest → live discovery). |

### Non-goals

- **No body rewriting / request wrapping.** The model travels in the agent's own
  body; the gateway reads it from there. Gate Connect only manipulates headers
  and tool config.
- We do not store an upstream provider credential locally. In PAYG the *gateway*
  holds it. The `Integration` trait's `*_upstream_credential` methods stay no-ops.

### Server-side prerequisites (operator-side, not app-side)

These are not headers we send, but PAYG fails without them — the app must
surface the gateway's errors clearly (see §5, "Still open"):

| Requirement | Failure if missing |
|---|---|
| Key is org-scoped (`orgId` present) | 400 `no_route` |
| Org has `paygEnabled: true` entitlement (opt-in; **default false** even on paid) | 402 `payg_disabled` |
| Org has an enabled provider account serving the model | 400 `no_route` |
| Model is priceable (`canBill`) | 503 `pricing_unavailable` |
| Sufficient `payg_balance_cents` | 402 `insufficient_balance` |

## 2. Design decision: mode placement

**Account-level `BillingMode` (one mode for all tools), default `Byok`.** This
matches the product's "enter once, reused for every tool" model and keeps the
snapshot/restore and master-switch logic untouched. Per-provider mode is a
possible future extension (would move the field onto `Provider`/`ProviderState`
and into the snapshot); out of scope here.

## 3. File-by-file changes

### 3.0 The unifying rule (confirmed — see §5)

In PAYG, whatever the tool sends in its **native auth slot** (`Authorization`
bearer / `x-api-key`) **must be the gate key (`sk-gw-…`), never a real provider
key.** This is forced by the gateway's auth guard
(`apps/gateway-proxy/src/auth/guards/api-key.guard.ts`):

- Any token in `Authorization`/`x-api-key` that does **not** start with `sk-gw-`
  is classified as a **passthrough token** → which **forces BYOK** → and is then
  rejected (no `X-Gate-Upstream-Url`) with a 400. So a leftover `sk-ant-…` /
  `sk-…` provider key in the tool's config breaks PAYG.
- A token that **does** start with `sk-gw-` is read as the gateway key itself,
  de-duplicated against `X-Gate-Api-Key`, and **stripped before forwarding
  upstream** (`proxy-helpers.ts` `STRIPPED_HEADERS`) — no leak, no upstream auth
  failure.

So for every integration: in PAYG, set the tool's own credential field to the
**gate key** (or remove any real provider key) in addition to dropping the
upstream-URL header.

### 3.1 `crates/core/src/account.rs`

- Add:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
  #[serde(rename_all = "lowercase")]
  pub enum BillingMode { #[default] Byok, Payg }
  ```
- Add `mode: BillingMode` to `Account` and to `AccountFile`
  (`#[serde(default)]` so existing `account.json` files load as `Byok` — no
  migration needed).
- `load()` populates `Account.mode`; `save()` persists it.
- Add `load_mode() -> Result<BillingMode>` (cheap, **no keychain read**, mirrors
  `load_base_url`) for the UI and for `status()` checks.
- **Signature change:** `save(gateway_base_url, api_key, mode)` — update the one
  caller in `src-tauri/src/lib.rs` (§3.8). Run `rg -n 'account::save'` to confirm
  the full caller set before landing.

### 3.2 `crates/core/src/registry.rs`

- Add `mode: BillingMode` to `ConnectInput`. Integrations read it to choose the
  header shape. (`upstream_url` stays — it's still used in BYOK.)

### 3.3 `crates/core/src/provider.rs`

- In `enable()`, build `ConnectInput { gateway_base_url, upstream_url, mode: account.mode }`.
- The proxy-domain branch needs the mode at the engine level, not per-call — see
  §3.7. No change to the snapshot/restore logic.

### 3.4 `crates/core/src/integrations/claude_code.rs`

- `build_headers(gate_api_key, upstream_url, mode)`:
  - `Byok` → `X-Gate-Api-Key: …\nX-Gate-Upstream-Url: …` (unchanged).
  - `Payg` → `X-Gate-Api-Key: …` only.
- **`ANTHROPIC_API_KEY` (PAYG only) — confirmed required.** Set
  `ANTHROPIC_API_KEY = <gate key>` in the managed `env` block (added to the
  `managed` list + `previousEnv` so disconnect restores cleanly). Two reasons,
  both confirmed in §5:
  1. It gives Claude Code a credential to start with (the user has no Anthropic
     key in PAYG).
  2. **Critically**, it overrides any ambient `sk-ant-…` key. If the user's real
     Anthropic key reached the gateway as `x-api-key`, the guard would classify
     it as a passthrough token and force BYOK → 400. The gate key (`sk-gw-…`) is
     instead read as the gateway key, de-duped against `X-Gate-Api-Key`, and
     stripped upstream — so PAYG proceeds cleanly.
- `status()`: when `account::load_mode()? == Payg`, accept the header block if it
  contains the Gate key header and **do not require** `X-Gate-Upstream-Url`;
  also validate the managed `ANTHROPIC_API_KEY` if we wrote it. Otherwise it
  reports false `Drifted`.
- `refresh_gate_key`: unchanged — it rewrites the key line and leaves the rest;
  works whether or not the upstream line is present.

### 3.5 `crates/core/src/integrations/codex.rs`

Most involved, because BYOK here derives everything from the user's `codex login`.

- `http_headers` (PAYG): write only `X-Gate-Api-Key` (drop `X-Gate-Upstream-Url`).
- **Auth-mode / base_url suffix:** BYOK reads `~/.codex/auth.json` to pick
  `/codex` (chatgpt) vs `/v1` (apikey). In PAYG there may be no login and the
  gateway routes by model, so **force the apikey shape** (`base_url = gateway/v1`,
  upstream irrelevant) and **skip `read_auth_mode()`** — it currently hard-fails
  when `auth.json` is absent.
- **Bearer / `[auth] command` — confirmed approach.** Codex always sends an
  `Authorization` bearer, produced today by the helper reading the user's token.
  In PAYG, replace the helper with one that prints the **gate key** (so
  `Authorization: Bearer sk-gw-…`). Confirmed safe in §5: the guard reads an
  `sk-gw-…` bearer as the gateway key (not a passthrough), and `STRIPPED_HEADERS`
  removes it before forwarding upstream — the reseller path then re-keys with the
  provider account's own credential. Conversely, leaving the user's real OpenAI
  key as the bearer would be classified as passthrough → forced BYOK → 400.
- `status()`: when mode is `Payg`, drop the `X-Gate-Upstream-Url` and auth-mode
  expectations from the drift check.

### 3.6 `crates/core/src/integrations/opencode.rs`

- `apply_override` (PAYG): write the `headers` block with only `X-Gate-Api-Key`
  (drop `X-Gate-Upstream-Url`).
- **Provider `options.apiKey` (PAYG):** today the per-provider `apiKey` (the
  user's real provider key, e.g. `{env:ANTHROPIC_API_KEY}`) survives the merge
  and OpenCode sends it as the upstream auth. In PAYG that's a non-`sk-gw-` token
  → passthrough → forced BYOK → 400. So in PAYG override each gated provider's
  `options.apiKey` with the **gate key** (snapshot the original in the sidecar
  `ProviderSnapshot` so disconnect restores it). Per the §3.0 rule.
- `status()`: when mode is `Payg`, drop the `has_upstream` requirement from the
  per-provider drift check; optionally assert `options.apiKey == gate key`.

### 3.6.1 `crates/core/src/integrations/openclaw.rs`

> Added after this plan was first written. OpenClaw is a fourth config
> integration (`ToolId::OpenClaw`, wired live in `registry.rs`) that the
> original draft predates. It is multi-provider like OpenCode, so the §3.6
> shape applies — with one consequential difference in the credential path.

- `apply_override` (PAYG): write each gated provider's `headers` block with only
  `X-Gate-Api-Key` (drop `X-Gate-Upstream-Url`), exactly as §3.6.
- **Provider `apiKey` (PAYG) — the OpenClaw-specific divergence:** unlike
  OpenCode, OpenClaw today is a *pure passthrough* injector. `apply_override`
  only rewrites `baseUrl` + `headers` and **deliberately preserves the
  provider's `apiKey`** (the module doc says `apiKey`/`api`/options are "preserved
  verbatim"; a test asserts `apiKey` stays `${env:ANTHROPIC_API_KEY}`). So the
  user's real provider key still reaches the gateway as the upstream auth.
  Dropping the upstream-url header is therefore **not sufficient**: that
  non-`sk-gw-` key is classified as a passthrough token → forced BYOK → 400.
  PAYG must *also* override each gated provider's `apiKey` with the **gate key**
  (so the guard reads `sk-gw-…`, de-dupes it against `X-Gate-Api-Key`, and strips
  it upstream). Per the §3.0 rule, same end-state as OpenCode's `options.apiKey`
  override and Claude Code's managed env.
- **Snapshot change:** the sidecar `ProviderSnapshot`
  (`<app_support_dir>/openclaw-state.json`) currently captures `previous_base_url`
  + `previous_headers` only. Extend it with `previous_api_key` so PAYG can restore
  the user's original key on disconnect / mode change. This is the one structural
  edit OpenClaw needs beyond the §3.6 pattern.
- `status()`: when mode is `Payg`, drop the `X-Gate-Upstream-Url` expectation from
  the per-provider drift check; optionally assert `apiKey == gate key`.
- No new trait surface: OpenClaw already implements `default_upstream_url`, and its
  `refresh_gate_key` participates in `refresh_gate_key_everywhere`.

### 3.7 `crates/core/src/proxy/engine.rs` (+ managers)

Covers the proxy-routed providers (Cowork, OpenRouter, Gemini).

- Add `mode: BillingMode` to `EngineConfig`.
- Add a live-update channel mirroring the existing `api_key` one:
  `RunningEngine::update_mode` + `mode_tx`/`mode_rx` `watch`, held on
  `GateHandler`.
- `apply_rewrite(req, gateway, upstream_url, api_key, mode)`: in `Payg`,
  - skip inserting `x-gate-upstream-url`, **and**
  - **strip the client's `authorization` / `x-api-key`** from the intercepted
    request. This is required by the §3.0 rule and is specific to the proxy path:
    Cowork/Gemini-CLI send their *own* provider credential (e.g. an
    `sk-ant-oat01-…` OAuth bearer) that we can't change via config — only by
    rewriting the in-flight request. Left intact, that non-`sk-gw-` token is
    classified as a passthrough → forced BYOK → 400. `X-Gate-Api-Key` alone
    authenticates the request (confirmed §5), so removing the client auth is
    safe. Everything else (authority swap, `x-gate-api-key`) is unchanged.
- **Construction sites have drifted, and Linux now differs architecturally.** The
  original list (`manager.rs:92`, `manager_linux.rs:90`, `manager_windows.rs:94`)
  is stale:
  - macOS: `manager.rs:94` — set `mode: account.mode` on the in-process
    `EngineConfig`.
  - Windows: `manager_windows.rs:96` — same, in-process.
  - **Linux: `manager_linux.rs` no longer builds `EngineConfig` at all.** The
    engine runs in a detached privileged helper daemon; the real construction
    site is **`helper.rs` (~219)**, fed over IPC by `control::Request::SetIntercept`
    (`control.rs:96`, which today carries `gateway_base_url` + `api_key` + CA +
    domains). So threading `mode` is an **IPC-protocol change**: add a `mode` field
    to `Request::SetIntercept`, thread it through `helper_client::set_intercept`,
    and in `helper.rs::handle_request` pass it to `EngineConfig`. This crosses a
    process boundary into a separately-installed binary, not a one-line edit.
- Manager `refresh_mode(mode)` → `running.update_mode(...)`, parallel to
  `refresh_api_key`. On macOS/Windows it is an in-process call. **On Linux there is
  no separate update message:** `manager_linux::refresh_api_key` re-sends
  `SetIntercept`, and the helper detects the running engine and applies it live via
  `update_api_key` (`helper.rs:211`). So once `mode` rides on `SetIntercept`,
  `refresh_mode` on Linux folds into the same re-send: send the new `mode` and have
  `handle_request` call `update_mode` on the live branch. No new `Request` variant
  needed.
- **UX note:** with the strip above, Cowork no longer needs a working Anthropic
  sign-in for traffic to flow in PAYG (its credential is removed and the gateway
  re-keys). Adjust onboarding copy (§3.9) so PAYG doesn't tell users to sign in to
  Anthropic inside Cowork. (Cowork may still *prompt* for sign-in on its own; that
  UX is outside our control and worth a release note.)

### 3.8 `src-tauri/src/lib.rs`

- `save_account(base_url, api_key, mode: String)` — parse `mode` → `BillingMode`,
  pass to `account::save`.
- On a **mode change**, configs must be rewritten (header shape differs). Today
  `save_account` only calls `refresh_gate_key_everywhere` (key only). Add: for
  each currently-connected provider, re-run `provider::enable(slug)` so each tool
  config is rewritten for the new mode; and call `manager().refresh_mode(mode)`.
- Extend the account-status getter command to return the current `mode` for the UI
  (check the existing `account_status`/get-account command).

### 3.9 UI (`src/`)

- `lib/api.ts`: `saveAccount(baseUrl, apiKey, mode)`.
- `components/AccountForm.tsx`: add a BYOK | PAYG segmented control. Copy:
  - BYOK keeps "Sign in to Anthropic from inside Cowork …".
  - PAYG: "Usage is billed to your Gate workspace balance — no provider key
    needed."
- Surface the active mode on `Home`/`Settings` (a small pill near the account row).
- `lib/config.ts`: optional `DEFAULT_BILLING_MODE` (default `byok`).

## 4. Tests

- `account`: legacy `account.json` (no `mode`) → `Byok`; round-trip with `Payg`.
- `claude_code`: `build_headers(Payg)` emits a single header line; PAYG `status()`
  accepts the no-upstream shape; `ANTHROPIC_API_KEY` managed/restored.
- `codex`: PAYG `http_headers` omit `X-Gate-Upstream-Url`; `connect()` in PAYG
  does not require `auth.json`.
- `opencode`: PAYG `apply_override` omits `X-Gate-Upstream-Url`; PAYG `status()`
  accepts it.
- `engine`: extend `rewrite_swaps_authority_keeps_path_and_injects_headers` —
  assert `x-gate-upstream-url` is **absent** in `Payg` and present in `Byok`.
- `provider`: `enable()` threads `account.mode` into `ConnectInput`.

## 5. Resolved questions (gateway evidence)

All three blocking questions are now answered from `swarm-deck`. The §3.0 rule is
the distilled conclusion.

**Q1 — Authorization slot in PAYG: stripped and re-keyed. ✅**
In reseller mode the gateway never forwards the client's auth.
`apps/gateway-proxy/src/utils/proxy-helpers.ts` (`STRIPPED_HEADERS`, ~L4–34;
`buildForwardHeaders`, ~L68–106) drops inbound `authorization` and `x-api-key`,
then each provider handler attaches the provider-account credential:
Direct/OpenAI-compatible sets `Authorization: Bearer`/`x-api-key`/`api-key` per
`authScheme` (`packages/upstream-providers/src/direct-handler.ts` ~L115–130,
client auth filtered ~L219–227); Bedrock signs SigV4 from `account.credentials`
(`bedrock-handler.ts` ~L408–414); Vertex mints a server-side OAuth token
(`google-vertex-handler.ts` ~L505–510). **All `x-gate-*` headers are also
stripped before upstream** (`proxy-helpers.ts` ~L76). So sending the gate key as
the bearer is safe — discarded, never leaked.

**Q2 — Auth required on a PAYG request: `X-Gate-Api-Key` alone is sufficient. ✅**
`apps/gateway-proxy/src/auth/guards/api-key.guard.ts` accepts the gateway key via
`X-Gate-Api-Key` (canonical), `x-gateway-api-key` (legacy), or an `sk-gw-…` value
in `Authorization: Bearer` / `x-api-key`. No `Authorization` header is required;
it may be entirely absent. PAYG additionally requires the key be **org-scoped**
(`payg-balance-gate.ts` ~L91–96: missing `orgId` → 400 `no_route`). Whether
Claude Code itself refuses to *boot* without a local key is a client-side detail,
but §3.4 sets `ANTHROPIC_API_KEY=<gate key>` regardless — see Q3.

**Q3 — Key type / passthrough classification: confirmed. ✅**
Gateway keys are `sk-gw-` + 64 hex (`KEY_PREFIX = "sk-gw-"`,
`apps/{dashboard-api,gateway-proxy}/src/api-keys/api-keys.service.ts`). A
**passthrough token** is the first non-`sk-gw-` value found in
`Authorization`/`x-api-key` (`api-key.guard.ts` `pickPassthrough`, ~L132–139),
and its presence forces BYOK (`resolve-upstream.ts` `isByokRequest` ~L113–123).
**Consequence (the §3.0 rule):** any real provider key (`sk-ant-…`, `sk-…`)
sitting in a tool's auth slot will be read as a passthrough → forced BYOK → 400
without an upstream URL. So PAYG must put the **gate key** in that slot (Claude
Code: `ANTHROPIC_API_KEY`; Codex: the credential-helper output; OpenCode:
`options.apiKey`; OpenClaw: provider `apiKey` (§3.6.1); proxy/Cowork: strip it
— §3.7).

**Still open (non-blocking, app-side):**
- **Error surfacing.** Map the gateway's `payg_disabled` / `insufficient_balance`
  / `pricing_unavailable` / `no_route` envelopes to friendly `ErrorBlock` copy.
  Not a blocker for the core wiring.

## 6. Sequencing

1. `account.rs` mode + `ConnectInput` + thread through `provider.rs` (no behavior
   change yet — all callers pass `Byok`).
2. Per-integration auth shaping per §3.0 (claude_code, codex, opencode, openclaw)
   + engine
   header strip.
3. IPC + UI + mode-change reconfigure.
4. Tests throughout; manual verify against a PAYG-enabled, funded org with an
   enabled provider account.