# Plan: re-introduce Google / Gemini as a proxy-only provider

## Status

Gemini support **existed and was removed.** It was added as a proxy-only
provider in PR #6 (`5aeb8b08`, tagged `v0.0.2`) and removed in PR #73
(`ea9d56f9`, "Remove gemini support", tagged `v0.1.0-rc.1`). The removal
touched exactly two files — `crates/core/src/provider.rs` (-29) and
`crates/core/src/proxy/mod.rs` (-92) — so a re-introduction is a bounded,
mostly-additive change against a known-good prior diff. This plan restores that
surface and fixes the two things that made the original fragile: **rewrite-path
over-scoping** and the **macOS CLI-reach gap** that is almost certainly why it
was pulled.

This is the companion to
[`gemini-cli-macos.md`](./gemini-cli-macos.md), which is the load-bearing
dependency for the Gemini *CLI* actually transiting Gate on macOS. Read that
first: without it, a re-added proxy-only Gemini provider silently no-ops on
macOS exactly as before.

## Client support (as shipped on the `gemini-provider` branch)

All Gemini surfaces are **proxy-only**: none has a gateway/base-URL setting, so
there is no config-file integration (as with Cowork, they ride a proxy domain).
Reach is via the injected `HTTP(S)_PROXY` + `NODE_EXTRA_CA_CERTS` env
(`environment.d` on Linux, `~/.zshenv` on macOS, `HKCU\Environment` on Windows).

| Client | Host(s) | Routes through Gate? |
| --- | --- | --- |
| Gemini API-key clients / SDKs | `generativelanguage.googleapis.com` | Yes (`google` domain) |
| Gemini CLI (login with Google) | `cloudcode-pa.googleapis.com` | Yes (`google-codeassist`) |
| `agy` CLI (Antigravity's standalone CLI) | `cloudcode-pa` + `daily-cloudcode-pa` | Yes (prod + daily domains) |
| Gemini Code Assist VS Code extension | `cloudcode-pa.googleapis.com` | Yes (`google-codeassist`; extension is proxy-and-CA-aware, and its `:generateChat` / `:completeCode` / `:generateCode` / `:internalAtomicAgenticChat` methods are now in the rewrite set) |
| Antigravity desktop App / IDE | `cloudcode-pa` + `daily-cloudcode-pa` | **No (not routable)** |

**Why the Antigravity App / IDE can't route.** Their Go language server, in
LSP/server mode, builds its cloudcode HTTP client with an *explicit* proxy URL
taken from internal state (`getProxyServerURLWithState`, empty ->
`net/http.ProxyURL(nil)` -> direct), which overrides the environment. Confirmed
empirically: the running language server had `HTTPS_PROXY` in its environment
yet its live sockets connected straight to Google's IPs, never to the engine.
The `agy` CLI works because in CLI mode it uses the default transport
(`ProxyFromEnvironment`) and honours the env. No proxy env var or IDE setting
(`http.proxy` only re-sets the env the server ignores) reaches the App/IDE
model client; routing them would need an upstream change in Antigravity (fall
back to `ProxyFromEnvironment` when the explicit proxy URL is empty).


## Background: how a provider routes here

Gate Connect has no per-request SDK; it points a tool's traffic at the gateway
one of two ways, bundled behind one `Provider` switch (`provider.rs`):

- **Config-file route** — edit a tool's own config to set the Gate base URL +
  `X-Gate-Api-Key`. Cross-platform, no proxy/CA.
- **Proxy-domain route** — the built-in MITM engine intercepts named hosts,
  rewrites inference paths to the gateway, and injects `X-Gate-Api-Key` +
  `X-Gate-Upstream-Url` (`proxy/engine.rs:apply_rewrite`).

**Gemini is proxy-only.** The Gemini CLI can't carry a custom base URL or the
`X-Gate-*` headers on its model API (header injection works only for its MCP
servers — upstream `google-gemini/gemini-cli#1679`), so there is no viable
config-file integration. Everything rides the proxy domain, like OpenRouter.

## Prerequisites (already true — do not rebuild)

1. **The gateway already upstreams Gemini.** `google` (AI Studio) and
   `google_vertex` are supported upstream providers in the gate repo
   (`packages/upstream-providers` `providers/google-ai-studio.ts`,
   `google-vertex-handler.ts`). BYOK passthrough with `X-Gate-Upstream-Url` is
   the exact shape this proxy emits. So "can Gate forward it" is done.
2. **The engine already handles Gemini's credential shape.** Gemini
   authenticates with the key in a `?key=` query param or an `x-goog-api-key`
   header — **not** a Bearer token. `engine.rs:383` already logs `Uri::path()`
   (never `path_and_query()`) precisely so a URL-embedded key never lands in the
   debug log, and `engine.rs:442-449` forwards `x-goog-*` request headers
   (including Code Assist's `x-goog-user-project`). No new engine credential
   code is needed; `apply_rewrite` leaves the client's own key on the request
   and adds the Gate headers alongside.

## What to restore (the reverted diff, improved)

### 1. `crates/core/src/proxy/mod.rs` — two `ProxyDomain` entries in `default_domains()`

The original added both:

- **`google`** — `generativelanguage.googleapis.com` (the AI Studio /
  generative-language API; API-key clients).
- **`google-codeassist`** — `cloudcode-pa.googleapis.com` (the Gemini CLI's
  "login with Google" / Code Assist backend; OAuth-bearer, `/v1internal`
  colon-RPC methods).

Restore both, with the `upstream_url` = the same host (BYOK passthrough), but
**narrow the `rewrite_prefixes`** — this is the one substantive change from the
reverted code. The original used broad prefixes:

```
// REVERTED (too broad — will 503 on model-less calls):
google:            rewrite_prefixes: ["/v1/", "/v1beta/"]
google-codeassist: rewrite_prefixes: ["/v1internal"]
```

After the revert, the Anthropic and OpenAI domains learned the hard lesson
(see their in-code comments: *"a client's non-inference /v1/ calls carry no
model, so the gateway can't classify them and 503s … Do NOT widen back to
/v1/"*). Gemini has the same trap:

- `/v1beta/models` (the **list** endpoint) carries no model → gateway 503.
- `/v1beta/models/<model>:generateContent` and `:streamGenerateContent` are the
  real inference calls.

So scope `google` to `["/v1beta/models/", "/v1/models/"]` (trailing slash): the
bare list endpoint `/v1beta/models` no longer prefix-matches and passes through
to real Google, while the model-scoped generate calls rewrite. (The GET
model-info call `/v1beta/models/<model>` still matches — see Open decision 3.)

`google-codeassist` is harder and is the second reason this was flaky: the
Code Assist paths are colon-RPC (`/v1internal:generateContent`,
`:streamGenerateContent`, but also `:loadCodeAssist`, `:onboardUser`). Only the
first two are inference; the onboarding calls are model-less and would 503 if
rewritten. The current `rewrite_prefixes` matcher is `starts_with`, which
cannot distinguish `:generateContent` from `:loadCodeAssist` under a shared
`/v1internal` prefix. See Open decision 1 — this needs either a
method-suffix-aware matcher or deferring the Code Assist domain.

### 2. `crates/core/src/provider.rs` — one `Provider` in `providers()`

Restore the entry verbatim in intent (proxy-only, two domains), keeping the
macOS subtitle caveat the original carried:

```rust
Provider {
    slug: "google",
    display_name: "Google / Gemini",
    subtitle: if cfg!(target_os = "macos") {
        "Gemini API (CLI not supported)"
    } else {
        "Gemini API"
    },
    tool_ids: &[],                                   // proxy-only, like OpenRouter
    proxy_domain_slugs: &["google", "google-codeassist"],
}
```

The macOS subtitle caveat is only correct **if** the CLI-reach fix in
`gemini-cli-macos.md` does not land in the same release (see Open decision 2).
If that fix lands, drop the caveat and the branch.

### 3. Restore the tests removed by #73

- `provider.rs`: `gemini_provider_is_proxy_only` (asserts `find("google")`,
  empty `tool_ids`, both domain slugs).
- `proxy/mod.rs`: `gemini_is_supported`, `rewrites_gemini_paths`,
  `rewrites_gemini_code_assist_paths` — **updated** for the narrowed prefixes:
  add a case asserting the bare list endpoint `/v1beta/models` is `Tunnel`
  (passes through), not `Rewrite`, and that `:generateContent` rewrites.

## Why it was removed, and what makes the re-add stick

The revert message is terse ("remove gemini and update docs"), but the
surviving `gemini-cli-macos.md` names the real defect: on macOS the Gemini CLI
(Node/undici) ignores the system proxy and the login keychain, so a proxy-only
provider routes nothing there — the CLI connects straight to Google, bypassing
Gate. Re-adding the provider without addressing that just reintroduces the same
silent-bypass bug that got it pulled. The two durable options:

- **A (recommended): land `gemini-cli-macos.md` first**, then re-add Gemini
  with no macOS caveat. That fix (a managed `~/.zshenv` block setting
  `HTTPS_PROXY` + `NODE_EXTRA_CA_CERTS`) is general macOS CLI parity and
  benefits any Node CLI.
- **B: re-add now with the macOS caveat**, scoping the macOS story to
  proxy-honoring API-key clients and labeling the CLI unsupported there (what
  the reverted code's subtitle already did). Ships value on Linux/Windows
  immediately; macOS CLI follows with fix A.

## Files touched + new symbols

1. `crates/core/src/proxy/mod.rs` — two `ProxyDomain` entries in
   `default_domains()`; three restored/updated tests. **New symbol:** none
   (the matcher change, if chosen, is Open decision 1).
2. `crates/core/src/provider.rs` — one `Provider` entry; one restored test.
3. `README.md` — re-list Google / Gemini under Providers (the removal touched
   it too).
4. *(conditional, Open decision 1)* a method-suffix-aware match in the
   `decide`/`ProxyDomain` rewrite logic for Code Assist colon-RPC.

No engine credential changes (already handled). No config-file integration
(infeasible — Gemini is proxy-only).

## Tests

- Restore + update the three proxy tests and the provider test above.
- New: `google` domain **does not** rewrite the bare `/v1beta/models` list
  (asserts `Tunnel`) but **does** rewrite
  `/v1beta/models/gemini-pro:generateContent`.
- New (if Open decision 1 = method-aware): `google-codeassist` rewrites
  `/v1internal:generateContent` and `:streamGenerateContent` but **tunnels**
  `:loadCodeAssist` / `:onboardUser`.
- `cargo test -p gate-connect-core` green on the Linux dev box.

## Verification (manual, end-to-end)

1. Sign in; enable the Google / Gemini provider with the proxy running.
2. **API-key client** (any tool honoring the system proxy): issue a
   `generateContent` call; confirm it appears in the engine request log as
   `rewrite->gateway` and the gateway records the request under the workspace.
3. Confirm the bare model-list call is `passthrough` (not 503).
4. **Gemini CLI**: on Linux, confirm it transits Gate; on macOS, confirm the
   caveat holds (bypass) **or**, with fix A applied, that a *new* shell's
   `gemini` transits Gate.
5. Confirm the client's `x-goog-api-key` / `?key=` coexists with the injected
   `X-Gate-Api-Key` without the gateway rejecting the request, and that
   `x-goog-user-project` survives to Google for Code Assist (the open worry
   flagged in `engine.rs:443-446`).

## Open decisions (recommendations inline)

1. **Code Assist path matching.** The `/v1internal` colon-RPC surface mixes
   inference (`:generateContent`) with model-less onboarding
   (`:loadCodeAssist`, `:onboardUser`) that the gateway will 503. Recommend
   adding a small method-suffix match (rewrite only when the path ends in
   `:generateContent` / `:streamGenerateContent`) rather than the broad
   `/v1internal` prefix. Alternative: defer the `google-codeassist` domain
   entirely and ship only the API-key `google` domain first — simpler, and it
   still covers proxy-honoring API-key clients.
2. **macOS CLI story.** Recommend Option A (land `gemini-cli-macos.md` first,
   no caveat). If release timing forces it, Option B (ship with the macOS
   caveat) is acceptable and matches the reverted code.
3. **GET model-info (`/v1beta/models/<model>`).** With the trailing-slash
   scoping this still rewrites and may 503 at the gateway (no model in a GET
   body). Recommend accepting it for the first cut (SDKs rarely call it on the
   hot path) and revisiting only if it causes noise; the cleaner fix is the
   same method/verb-aware matcher as decision 1.
4. **Vertex AI (`aiplatform.googleapis.com`).** Out of scope. Its hosts are
   regional + per-project (dynamic) and it authenticates with minted OAuth
   service-account tokens, which the fixed-host `ProxyDomain` model fits
   poorly. AI Studio (`generativelanguage.googleapis.com`, one static host) is
   the right first target; Vertex is a later, separate effort.

## Out of scope

- Any config-file tool integration for Gemini (infeasible: no base-URL/header
  support on the CLI's model API).
- Vertex AI routing (decision 4).
- The macOS CLI-reach fix itself — that is `gemini-cli-macos.md`, a dependency,
  not part of this change.
