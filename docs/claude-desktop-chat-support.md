# Investigate: Claude Desktop chat support — Gate Connect

## Executive summary

Claude Desktop chat can be brought under Gate as a per-message **inspection**
surface — PII scanning, prompt-injection detection, and outgoing-prompt
compression — but **not** as a routing surface. It's a new "observe + inspect"
mode alongside the existing reseller/BYOK paths, not a config tweak.

The reason is structural: the chat completion request is not self-contained.
Its body carries only the latest turn (`prompt`, `attachments`, `files`) with no
`messages[]`, no `model`, and no `system`; the conversation history lives
server-side keyed by `{conv}`, and auth is a `sessionKey` cookie rather than an
API key. So the traffic can only be forwarded to Anthropic's own backend — it
can't be retargeted to another provider or Messages-API endpoint, Gate's
key-injection is a no-op, and completions have no stable cache key.

What makes this achievable is that the hard parts already exist. Gate-connect
already runs a MITM proxy with its own CA, system-proxy configuration, a
per-host upstream catalog, and a credential-injection rule — so TLS interception
of the Electron app is largely solved. Gate already has the inspection
primitives (Presidio PII, security-fusion injection detection, the streaming SSE
scanner, compression transforms), and they operate on **text**, so they carry
over unchanged. The work is wiring a non-Messages wire shape through a
Messages-shaped pipeline.

Effort is **medium-to-large**. Gate-connect needs a catalog entry, an additive
(cookie-preserving) injection branch, and a UI row. Gate needs a new wire-format
branch, a passthrough forward handler, a chat text extractor, ~6 gated stages,
and a model-less persistence path.

**Recommendation:** scope this as a visibility/safety surface, not a routing
one. If routing Desktop to a chosen upstream is ever the goal, that lives in
Desktop's Gateway / third-party-inference mode — a different surface, not chat.

---

## The core issue: chat completions are not self-contained

The chat call is `POST /api/organizations/{org}/chat_conversations/{conv}/completion`.
Its body carries only the latest turn — `{ prompt, parent_message_uuid,
attachments, files, sync_sources, … }` — with **no `messages[]`, no `model`, no
`system`**. Full conversation history lives server-side, keyed by `{conv}`.
Consequences:

- The only server that can service the request is Anthropic's claude.ai backend,
  so the traffic can only be *forwarded there* — never retargeted to another
  provider or a Messages-API endpoint.
- Auth is a `Cookie: sessionKey=sk-ant-sid01-…`, not `x-api-key` / `Bearer`.
  Gate's key-injection is a no-op here; the cookie must pass through untouched.
- There is no self-contained cache key (the response depends on server-side
  history), so **completions can't be cached** — only the outgoing prompt text
  can be deduped/compressed.

Everything below follows from this.

## Changes in gate-connect (the app)

The interception stack already exists (MITM proxy + own CA + system-proxy +
`ProxyDomain` catalog + credential injection). The additions:

1. **New catalog entry** (`proxy/config.rs` / `default_domains`): a
   `ProxyDomain` for `claude.ai` whose `upstream_url` routes through the Gate
   gateway with `x-gate-upstream-url: https://claude.ai`, and whose
   `passthrough_prefixes` cover statics/telemetry and the non-completion `/api/`
   paths that don't need inspection.
2. **Additive credential-injection mode** (`inject_gate_credential` in
   `proxy/mod.rs`, applied in `relay.rs`): today injection *replaces* the tool's
   auth with the Gate credential. For chat it must **preserve the `sessionKey`
   cookie and `anthropic-client-*` headers** (real auth to claude.ai) while
   *adding* the Gate key/org headers for the Gate hop only. New branch, not a
   change to the existing rule.
3. **New integration surface** (`integrations/claude_desktop.rs` + a popover row
   in `src/`): unlike `claude_code.rs`, there's no base-URL knob to write —
   Desktop chat is captured transparently via the system proxy + CA. So the
   "integration" is mostly a domain toggle, a status pill, and honest copy
   framing this as **audit/visibility, not key brokering** (the credential stays
   in Desktop's cookie jar — say so, per the "credentials are the product"
   principle).
4. **CA trust verification for Electron**: confirm Claude Desktop honors the
   system trust store (Chromium usually does) and isn't pinning. Flag as a
   validation step, not assumed.

## Changes in gate (the gateway)

The pipeline is entirely Messages/OpenAI/Gemini-shaped (`ResolveUpstreamStage`
needs a model; security/compression/turn-assembly read
`messages[]`/`system`/`contents[]`). A parallel branch for the chat shape:

1. **Surface detector**: recognize `claude.ai` + `…/completion` and stamp
   `ctx.wireFormat = "claude-web-chat"` so stages can branch.
2. **Passthrough forward mode**: fixed `claude.ai` upstream, cookie + client
   headers preserved, **no key injection**, `claude.ai` allow-listed past SSRF.
   (Fork of the BYOK handler, which is closest but assumes a client-supplied URL
   and no auth passthrough.)
3. **Chat text extractor** (a `turn-assembly` variant): map `{prompt,
   attachments, files}` → the "current message." **This is the message-by-message
   unit** the inspection stages consume.
4. **Gate off the Messages-shape stages** under this `wireFormat`: model
   resolution, model-capability, cost/token pricing, cache-control injection, and
   the `messages[]`/`contents[]` compression rewriters (feed the extractor
   instead).
5. **Model-less persistence**: `gateway_requests` assumes model + tokens + cost.
   Add a `surface`-tagged / nullable path in `PersistAndAuditStage` and the
   dashboard for cost-less chat rows.

**Reused as-is** (these operate on text, not wire shape): `packages/presidio`
(PII), `packages/security-fusion` + `injection-detector-pipeline` (injection),
the streaming SSE scanner in `ForwardRequestStage` /
`ResponseSecurityEvaluationStage` (response-side PII — claude.ai's SSE mirrors
the public streaming API closely enough to reuse the parser), and
`packages/compression` transforms (outgoing-prompt shrink).

## Caveat, restated

"Cache/compress individual messages" splits: **compressing the outgoing prompt
is viable**; **caching completions is not** (no self-contained key). Per-message
dedup/compression yes; per-completion cache hits no.

## Related

- `docs/claude-ai-chat-traffic-spec.md` — observed claude.ai wire format the
  above builds on.
