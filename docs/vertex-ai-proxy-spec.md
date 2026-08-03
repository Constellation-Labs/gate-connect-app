# Spec: route Google Vertex AI through Gate Connect

## Status & relationship to the Gemini plan

This is the deferred item from
[`gemini-provider-reintroduction-plan.md`](./gemini-provider-reintroduction-plan.md)
(Open decision 4). The Gemini plan covers **AI Studio**
(`generativelanguage.googleapis.com`, one static host, API-key). This spec
covers **Vertex AI** (`aiplatform.googleapis.com` + regional hosts,
OAuth-bearer, project/region in the path), which the current `ProxyDomain`
model cannot express as-is. It is a separate spec because it requires two new
engine capabilities, not just catalog rows.

Gate-side context: the gateway already has a first-class `google_vertex`
upstream (`packages/upstream-providers/src/google-vertex-handler.ts`), but that
is **reseller mode** — the gateway holds a service-account JSON, mints tokens,
and clamps serving to US regions. Gate Connect does **not** use that path. The
proxy routes as **BYOK passthrough** (`X-Gate-Upstream-Url` present →
`billingMode = "byok"` in the gateway's `resolve-upstream.ts`): the client's
own bytes and its own OAuth bearer are forwarded, SSRF + DNS-pin only. See
[Auth](#auth) and [Open questions](#open-questions) for what that implies.

## Goal

Let a user with Vertex-configured tooling (the Vertex/GenAI SDKs, `gcloud`,
LangChain, or the Gemini CLI in Vertex mode via `GOOGLE_GENAI_USE_VERTEXAI=1`)
flip one switch and have their Vertex inference traffic transit Gate — so the
redaction policy and audit-trail anchoring apply — without putting any GCP
credential into Gate Connect.

## Non-goals

- **No credential storage or token minting in Gate Connect.** The client's
  tooling already mints short-lived OAuth bearers (`gcloud auth
  print-access-token`, ADC, SA impersonation). The proxy forwards them; it
  stores nothing. (This is the opposite of the gateway's reseller
  `google_vertex` handler, which *does* mint — not in scope here.)
- **No config-file integration.** Like AI Studio Gemini, this is proxy-only.
- **No residency enforcement.** In BYOK the region is whatever host the client
  targets; the gateway's reseller-mode US-only clamp does **not** apply (see
  Open questions).

## Background: why the current model doesn't fit

Two facts about Vertex break the existing `ProxyDomain` contract:

1. **The host is regional, not singular.** Vertex serves from
   `{REGION}-aiplatform.googleapis.com` (e.g.
   `us-central1-aiplatform.googleapis.com`, `europe-west4-aiplatform.googleapis.com`)
   plus `aiplatform.googleapis.com` for the `global` endpoint. Today
   `ProxyDomain.hosts` is a `Vec<String>` matched by case-insensitive
   **equality** (`matches_host`, `mod.rs:211-214`), gating both the CONNECT
   (`should_intercept_host`, `mod.rs:249`) and `decide` (`mod.rs:256`).
   Enumerating every region is brittle; this needs **suffix/wildcard host
   matching**.

2. **The upstream can't be one static string.** `decide` returns
   `Rewrite { upstream_url: d.upstream_url.clone() }` from a fixed field
   (`mod.rs:271-273`). For a wildcard host set, the correct upstream is the
   *host the request actually came in on* — region-preserving. This needs a
   **"same-host" upstream mode**.

A third issue is shared with the Gemini plan (Code Assist): Vertex inference is
colon-RPC on a path that also carries non-inference calls, so prefix matching
alone over-captures (see [Path scoping](#path-scoping)).

## Design

### 1. Wildcard host matching

Extend the host match to support a leading-`*` suffix pattern alongside exact
hosts. Concretely, give `ProxyDomain` a way to say "match
`*-aiplatform.googleapis.com` and `aiplatform.googleapis.com`". Recommended
shape (keeps `hosts` for the exact case, adds an explicit pattern list so the
wildcard is never an accidental substring match):

```rust
pub struct ProxyDomain {
    // ...
    pub hosts: Vec<String>,          // exact, existing behavior
    pub host_suffixes: Vec<String>,  // NEW: match if host == suffix's bare form
                                     // OR ends with "-" + suffix's bare form
    // ...
}
```

`matches_host` becomes: existing exact check, OR for each suffix `s`, `host`
equals the bare domain (`aiplatform.googleapis.com`) or ends with
`-aiplatform.googleapis.com`. Anchoring on the leading `-` (region separator)
avoids matching an unrelated `evil-aiplatform.googleapis.com.attacker.com` —
combine with the existing DNS-pin/SSRF guard. This is a **general capability**;
any future regional provider reuses it.

Both `should_intercept_host` (the CONNECT gate) and `decide` call
`matches_host`, so both pick up wildcard support with one change.

### 2. Same-host upstream

Add an upstream mode so `decide` can echo the intercepted host instead of a
static URL:

```rust
pub enum Upstream {
    Static(String),  // existing: e.g. "https://api.anthropic.com"
    SameHost,        // NEW: upstream = "https://" + <intercepted host>
}
```

For Vertex, `Upstream::SameHost`. `decide` already receives `host`, so it
builds `Rewrite { upstream_url: format!("https://{host}") }`. This preserves
the region the client chose and means one domain entry covers every Vertex
region. (Anthropic/OpenAI/etc. stay `Static` — no behavior change.)

### 3. Path scoping

Vertex inference paths (model + method in the path):

- Gemini: `/v1/projects/{p}/locations/{r}/publishers/google/models/{m}:generateContent`
  and `:streamGenerateContent` (also `/v1beta1/...`).
- Claude on Vertex: `...:rawPredict` / `:streamRawPredict`.
- Model-Garden MaaS (OpenAI-shaped): `.../endpoints/openapi/chat/completions`.
- Embeddings/media: `...:predict`.

Non-inference calls on the same host (`.../locations`, `.../models` discovery,
operations polling) carry no model and the gateway will 503 them — the same
trap the Anthropic/OpenAI domains warn about ("Do NOT widen back to /v1/").
A broad `/v1/projects/` prefix over-captures. **This requires the method-suffix
matcher already proposed as Gemini plan Open decision 1**: rewrite only when the
path ends in one of `:generateContent`, `:streamGenerateContent`,
`:rawPredict`, `:streamRawPredict`, `:predict`, or matches the
`/endpoints/openapi/chat/completions` suffix. Build this matcher once; both
Vertex and Gemini Code Assist consume it.

### 4. Provider surface

Add Vertex as a **proxy-only** provider. Two options:

- **A (recommended):** a distinct `Provider { slug: "google-vertex",
  display_name: "Google Vertex AI", tool_ids: &[], proxy_domain_slugs:
  &["google-vertex"] }`. Keeps AI Studio and Vertex as independent switches —
  they are different accounts, auth, and billing.
- **B:** fold a `google-vertex` domain slug into the existing `google` provider
  from the Gemini plan. Simpler UI, but conflates two distinct GCP surfaces
  under one toggle; not recommended.

Carry the same macOS CLI caveat as the Gemini plan: proxy-honoring SDK/`gcloud`
clients work everywhere, but the Gemini CLI (Node) needs the
[`gemini-cli-macos.md`](./gemini-cli-macos.md) `~/.zshenv` reach-fix on macOS.

## Auth

Vertex uses `Authorization: Bearer <OAuth2 access token>` (short-lived, ~1h),
minted client-side. The proxy's `apply_rewrite` (`engine.rs:488`) rebuilds the
URI to the gateway and injects `X-Gate-Api-Key` + `X-Gate-Upstream-Url`; the
client's `Authorization` bearer rides through untouched — which is exactly what
BYOK passthrough needs. **No Gate Connect credential storage, no minting, no
rotation.** This is strictly simpler than the AI Studio `x-goog-api-key`/`?key=`
case the engine already handles.

## Redaction & audit

Vertex generateContent bodies are the native Gemini `contents[]` shape. The
gateway's redaction path is already content-addressed and explicitly covers
`contents[]`, `systemInstruction`, and tool arguments (per the gate-side Gemini
spike, `apply-redaction-pairs.ts`), and audit/anchoring ride the standard
outbox. So policy application is inherited once the request reaches the gateway
— the connect-side work is purely getting the bytes there with the right
upstream. The MaaS OpenAI-compat path (`chat/completions`) is likewise a shape
the gateway already redacts.

## Open questions (need a live check before shipping `supported: true`)

1. **Does BYOK classify a path-encoded model?** Vertex carries the model in the
   path (`.../models/{m}:generateContent`), not the body. Confirm the gateway,
   in `billingMode = "byok"`, either forwards without needing to classify or
   extracts the model from the Vertex path — otherwise inference calls 503 the
   way model-less calls do. This is the single biggest go/no-go risk.
2. **Residency.** BYOK bypasses the reseller handler's US-only region clamp, so
   a client hitting `europe-west4-aiplatform.googleapis.com` sends EU-region
   traffic through Gate. Confirm that is acceptable for the audit/anchoring
   story, or restrict `host_suffixes` to US regions if policy requires it
   (contradicts "region the client chose", so this is a product call).
3. **`x-goog-user-project` / quota project.** Vertex bills the request's quota
   project; confirm the header (and any `x-goog-*`) survives the gateway hop
   (the same worry flagged for Code Assist in `engine.rs:443-446`).
4. **DNS-pin coverage.** Confirm the SSRF/DNS-pin layer accepts the regional
   `*-aiplatform.googleapis.com` hosts once wildcard matching lets the CONNECT
   through.

## Verification (manual, end-to-end)

1. Enable the Vertex provider with the proxy running; issue a
   `generateContent` against a regional host from a proxy-honoring client
   (e.g. `curl` with a `gcloud` bearer, or the Vertex SDK).
2. Confirm the engine logs `rewrite->gateway` with
   `X-Gate-Upstream-Url: https://<the regional host>` (same-host echo working),
   and the gateway records the request.
3. Confirm a discovery/`operations` call on the same host is `passthrough`
   (not 503) — the method-suffix scoping working.
4. Confirm a second region (e.g. `europe-west4-`) also routes, proving the
   wildcard + same-host design (subject to Open question 2).
5. Confirm the client's OAuth bearer reaches Google unbroken (a real
   generation returns 200).

## Phasing

- **Phase 1 — engine capability:** wildcard host matching (§1) + `SameHost`
  upstream (§2) + the shared method-suffix matcher (§3). Land with unit tests;
  no provider exposed yet.
- **Phase 2 — resolve Open question 1** (path-model classification in BYOK)
  with a live gateway. If it fails, that is a gateway change, tracked
  separately, and this spec blocks on it.
- **Phase 3 — expose the provider** (§4) with `supported: true`, the Gemini
  Code Assist domain adopting the same matcher, and the macOS caveat.

## Out of scope

- Reseller-mode Vertex (gateway-held SA JSON + minting) — that is the gateway's
  existing `google_vertex` handler, a different product path.
- AI Studio Gemini — covered by the companion plan.
- Config-file integration — infeasible (proxy-only).
