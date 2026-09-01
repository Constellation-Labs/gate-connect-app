//! The built-in domain catalog: every provider the MITM engine may
//! intercept and the relay may route, with the per-entry rationale essays.
//! Split out of `proxy/mod.rs` so the shared routing/injection mechanics
//! there stay readable; the catalog invariants are pinned by the tests in
//! the parent module (`every_resolved_endpoint_lands_on_an_inference_prefix`,
//! `forwarded_paths_avoid_gate_reserved_prefixes`).

use super::ProxyDomain;

/// The built-in domain catalog. All entries ship `supported:true` (Anthropic
/// is also `enabled` by default; the rest are opt-in). New providers can be
/// added here and surface in the UI automatically; gate a provider behind
/// `supported:false` until Gate's upstream support for it is confirmed.
pub fn default_domains() -> Vec<ProxyDomain> {
    vec![
        ProxyDomain {
            slug: "anthropic".into(),
            // Named for what its SWITCH covers, not for the vendor: the
            // entry is the system-proxy route for the desktop apps, and its
            // switch is theirs alone. Claude Code reaches the same host, but
            // through its own configured proxy URL, whose route selector makes
            // the engine apply this entry whatever the switch says (see
            // `claude_code_route_domain`) - so a vendor name here would read as
            // a promise this switch does not make. The host line carries
            // api.anthropic.com.
            display_name: "Claude Desktop / Cowork".into(),
            // Inference for Claude Code, Claude Desktop, and Cowork all goes
            // to api.anthropic.com /v1/messages (OAuth bearer or API key),
            // confirmed against a real Cowork generation. a-api.anthropic.com
            // is Anthropic's telemetry host (Segment-style /v1/b ingestion)
            // and is deliberately left tunnelled, never intercepted. claude.ai
            // is the web/chat/login surface and is NOT part of this entry - it
            // speaks a different protocol and has its own opt-in `claude-web`
            // domain below.
            hosts: vec!["api.anthropic.com".into()],
            // Applies to every host above. Only group hosts that genuinely
            // share this upstream - never collapse distinct API hosts onto one.
            upstream_url: "https://api.anthropic.com".into(),
            // Only genuine inference endpoints are rewritten to the gateway.
            // Scoped deliberately narrow: Claude Desktop / Cowork also make
            // OAuth + account calls on this same host under /v1/ (e.g.
            // /v1/oauth/*, /v1/organizations/*), and those carry no model, so
            // the gateway can't classify them and rejects them 503 ("AI
            // unknown"). Naming the three inference paths one by one lets every
            // other /v1/ path fall through to `decide`'s default Passthrough and
            // reach the real host unchanged. Do NOT widen this back to "/v1/".
            //
            // - /v1/messages is the native Messages API, and the prefix covers
            //   its count_tokens + batches sub-paths.
            // - /v1/complete is the legacy text-completions endpoint.
            // - /v1/chat/completions is Anthropic's OpenAI-compatible endpoint.
            //   It carries a model in the body like the other two, so it is
            //   inference by the same test, and it was missing here until
            //   2026-08-31: openclaw 2026.8.1 switched its anthropic transport
            //   to it (`api=openai-completions`), and because the path fell
            //   through to Passthrough its traffic went to the real API with
            //   Gate's switch on and nothing in the gateway to show for it. A
            //   silent bypass is the one failure this entry exists to prevent,
            //   so the path is named rather than left to the default.
            rewrite_prefixes: vec![
                "/v1/messages".into(),
                "/v1/complete".into(),
                "/v1/chat/completions".into(),
            ],
            // Paths outside the inference set already pass through; this keeps
            // the Squirrel auto-updater explicit. Other /api/* paths
            // (claude_code, event_logging, bootstrap) also reach the real host
            // unrewritten.
            passthrough_prefixes: vec!["/api/desktop/".into()],
            rewrite_suffixes: Vec::new(),
            enabled: true,
            supported: true,
        },
        ProxyDomain {
            slug: "claude-web".into(),
            // Claude Desktop's CHAT surface, which is a different protocol from
            // the entry above rather than more of the same host. That one covers
            // api.anthropic.com /v1/messages; this one covers claude.ai, where
            // the desktop app sends a bare `prompt` string and Anthropic keeps
            // the conversation history server-side. Gate recognises it as the
            // `claude-web-chat` surface and treats it as inspection + audit, not
            // as key-brokered routing: there is no API key involved at all.
            display_name: "Claude Desktop chat".into(),
            hosts: vec!["claude.ai".into()],
            // The `/api` MUST ride in the upstream URL, not the forwarded path,
            // for the same reason it does on OpenRouter below: Gate's ALB routes
            // `/api/*` to the dashboard API, so a forwarded
            // `/api/organizations/...` never reaches the gateway proxy at all.
            // Every path this entry cares about lives under `/api`, so the whole
            // segment moves upstream-side and the prefixes below are written
            // POST-STRIP - `decide` and `apply_rewrite` both match and forward
            // the path Gate will actually see.
            //
            // Gate must accept the stripped spelling for the chat surface to
            // stay classified (gateway-proxy: `CLAUDE_WEB_CHAT_COMPLETION_RE` in
            // utils/proxy-helpers.ts, which anchors on `^/api/organizations/`).
            // The Codex and ChatGPT anchors beside it already tolerate both
            // splits with an optional prefix group; this one needs the same
            // treatment or the completion call is tagged `api` and loses the
            // additive-credential policy the session cookie depends on.
            upstream_url: "https://claude.ai/api".into(),
            // Prefix matching cannot isolate the chat call on its own: the
            // endpoint is `/api/organizations/{org}/chat_conversations/{conv}/completion`,
            // so the varying id sits BEFORE the distinguishing final segment.
            // Rewriting the whole organizations tree is deliberate and
            // safe - Gate classifies only the completion path as the chat
            // surface and forwards the sibling calls (skills, usage,
            // conversation reads) as ordinary passthrough, which it has explicit
            // coverage for. They add audit rows, not behaviour changes.
            rewrite_prefixes: vec!["/organizations/".into()],
            // Everything here would be pure noise or actively harmful to route:
            // the updater channel, telemetry batches, and the bootstrap/account
            // calls the app makes before any conversation exists. Written
            // post-strip, so these are the app's `/api/desktop/` etc.
            passthrough_prefixes: vec![
                "/desktop/".into(),
                "/event_logging/".into(),
                "/bootstrap/".into(),
            ],
            rewrite_suffixes: Vec::new(),
            // Opt-in. This surface carries the user's Claude SESSION cookie
            // rather than an API key, so it should never start intercepting
            // without a deliberate toggle.
            //
            // Deliberately NOT attached to the `anthropic` provider's
            // `proxy_domain_slugs` (see `provider.rs`): `provider::enable` turns
            // on every domain a provider lists, so attaching it would route the
            // session surface the moment someone enabled "Claude" - defeating
            // the opt-in above. It rides that provider's `chat_domain_slugs`
            // instead, which is how it reaches the Home ledger: `buildGroups`
            // (src/lib/groups.ts) gives it a row and a switch under Claude, and
            // `setGroupRouted` filters it out of the family cascade, so the only
            // thing that can enable it is that row's own switch (or
            // `proxy domain claude-web on` from the CLI).
            //
            // `supported: true` is the catalog's answer, not the app's:
            // `proxy::config::gated_catalog` flips it to false unless the
            // account points at the staging gateway, which is where the
            // gateway-side classification for this surface lands first. That
            // gate is the only thing standing between this entry and a row, so
            // read it before assuming the row is live in production.
            enabled: false,
            supported: true,
        },
        ProxyDomain {
            slug: "openai".into(),
            // "apps", not the vendor name: covers any system-proxy-honoring
            // client of api.openai.com, and must not read as including Codex
            // (config-routed; its embedded agent ignores the system proxy).
            display_name: "OpenAI apps".into(),
            // The OpenAI API host. Catches OpenAI-compatible clients that
            // honor the macOS system proxy and hit /v1/. Note: the Codex
            // desktop app's model calls come from its embedded Rust agent,
            // which ignores the system proxy and reaches chatgpt.com
            // directly, so the proxy can't capture them - route Codex via the
            // manual integration (config.toml base_url) instead.
            hosts: vec!["api.openai.com".into()],
            upstream_url: "https://api.openai.com".into(),
            // Inference endpoints only, same reasoning as Anthropic above: a
            // client's non-inference /v1/ calls (e.g. /v1/models preflight)
            // carry no model, so the gateway can't classify them and 503s.
            // Rewrite only the model-call paths; everything else on the host
            // passes through to real api.openai.com. Do NOT widen back to "/v1/".
            rewrite_prefixes: vec![
                "/v1/chat/completions".into(),
                "/v1/completions".into(),
                "/v1/responses".into(),
                "/v1/embeddings".into(),
            ],
            passthrough_prefixes: vec![],
            rewrite_suffixes: Vec::new(),
            enabled: false,
            supported: true,
        },
        ProxyDomain {
            slug: "chatgpt-apps".into(),
            display_name: "ChatGPT app chat + Codex tools".into(),
            // Codex Desktop's TOOL traffic, which is a separate route from the
            // `chatgpt` entry below even though both name chatgpt.com.
            //
            // That entry is RELAY-only: it exists so the relay recognises the
            // upstream hint `integrations/codex.rs` writes, and it is matched by
            // `relay::route` on `upstream_url`. This entry is the MITM half,
            // matched by `decide` on HOST - which is why the two can share a
            // host without colliding, and why the split below differs.
            //
            // What this can and cannot capture. The Electron shell honours the
            // system proxy, and the tool-plane calls observed in a capture came
            // from it: `/backend-api/wham/*` carried a Chromium user-agent.
            // `/backend-api/ps/mcp` sends no user-agent at all, so which
            // component emits it is still unverified, though the engine does see
            // it.
            //
            // The desktop app's MODEL call is visible too, and is NOT served by
            // this entry - it is served by the `chatgpt` entry below, whose
            // upstream path absorbs `/backend-api` and leaves
            // `/codex/responses` for its rewrite prefix. Confirmed 2026-08-14
            // from a captured Gate row whose body carried
            // `<app-context># Codex desktop context`, `workspace_kind:
            // "projectless"` and Windows paths under `Documents\Codex`.
            //
            // This comment previously said the opposite - that Codex's embedded
            // Rust agent ignores the system proxy, so its model calls stay
            // invisible to the engine. That holds for the standalone CLI, whose
            // agent routes via the relay, and it is why the exclusion below is
            // still correct. It does NOT hold for the desktop app, and stating it
            // unconditionally cost two debugging sessions: the traffic was
            // assumed unreachable when it was merely on the other row. Which row
            // is the whole point, because `chatgpt-apps` and `chatgpt` are
            // separate switches and this one's NAME implies it carries Codex's
            // prompts. It does not.
            hosts: vec!["chatgpt.com".into()],
            // MITM convention: `engine::apply_rewrite` preserves the request path
            // and query VERBATIM and swaps only scheme + authority, so the
            // upstream is the BARE host and the paths below are the app's real
            // ones. The relay entry uses the opposite split (`/backend-api` in
            // the upstream, short client path) because the relay sees the path
            // Codex rewrote, not the real one. Gate accepts both spellings.
            upstream_url: "https://chatgpt.com".into(),
            // Only the two path families Gate classifies as native surfaces:
            // the MCP tool plane (`codex-mcp`, scanned for indirect injection)
            // and the task/settings reads (`codex-tasks`). Deliberately NOT
            // `/backend-api/codex/responses` - that path belongs to the `chatgpt`
            // entry, which serves it on BOTH routes, and claiming it here would
            // send it upstream under this entry's bare-host split. The URL would
            // still resolve; what breaks is that one endpoint would then carry two
            // different `X-Gate-Upstream-Url` values depending on how it arrived,
            // which is the split-mismatch class that once left MITM traffic
            // classified as plain `api`. Excluding it is not a coverage gap: this
            // entry sits FIRST in catalog order, so claiming it would shadow the
            // other. Plugin-store listings are left out as pure noise.
            // `/backend-api/f/conversation` is the ChatGPT app's own chat turn
            // (Gate's `chatgpt-web-chat` surface): one message per request, reply
            // as a `delta_encoding: v1` SSE stream. It lives in THIS entry rather
            // than its own because `decide` returns on the first enabled
            // host match - a second chatgpt.com entry would be dead code.
            //
            // The `…/f/conversation/prepare` sibling is deliberately absent: it
            // only mints a short-lived `conduit_token` and carries neither prompt
            // nor reply, so routing it would add audit noise and nothing else.
            rewrite_prefixes: vec![
                "/backend-api/f/conversation".into(),
                "/backend-api/ps/mcp".into(),
                "/backend-api/wham/".into(),
            ],
            // `/backend-api/f/conversation/prepare` starts with the chat prefix
            // above, so it needs an explicit passthrough to stay unrouted -
            // passthrough prefixes are checked first in `decide`.
            passthrough_prefixes: vec!["/backend-api/f/conversation/prepare".into()],
            rewrite_suffixes: Vec::new(),
            // Opt-in, and no longer order-sensitive against the `chatgpt` entry
            // below: `decide` consults every enabled entry claiming the host, so
            // each of the two serves its own paths whether one, the other, or
            // both are switched on. It used to stop at the first host match,
            // which made this entry swallow the Responses call sitting in that
            // one - a hazard that mattered the moment either became togglable
            // without reading this file.
            //
            // Like `claude-web` above, this slug is in no provider's
            // `proxy_domain_slugs` and rides `chat_domain_slugs` instead: it
            // gets a Home ledger row and a switch under OpenAI, and the family
            // switch's cascade skips it, so the chat half of this entry
            // (`/backend-api/f/conversation`, a session-cookie surface) can only
            // be enabled from its own row or from `proxy domain chatgpt-apps
            // on`. Whatever else exposes this entry must keep that property.
            //
            // And, like `claude-web`, staging-gated: `proxy::config`'s
            // `STAGING_ONLY_SLUGS` clears `supported` on a production account,
            // so this entry has no row and cannot route there.
            enabled: false,
            supported: true,
        },
        ProxyDomain {
            slug: "chatgpt".into(),
            display_name: "ChatGPT (Codex subscription)".into(),
            // The Responses API a ChatGPT-subscription login talks to:
            // chatgpt.com/backend-api/codex/responses, bearer = the user's
            // ChatGPT OAuth token, passed through. TWO clients arrive here by
            // different routes, which is why the entry has to serve both:
            //
            // - Codex, via the relay. Its `base_url` points at the loopback
            //   relay (integrations/codex.rs) because its embedded agent ignores
            //   the system proxy; the relay matches this entry on `upstream_url`.
            // - OpenClaw, via the MITM engine. Managed proxy mode honours the
            //   proxy, so `decide` matches this entry on HOST and the engine
            //   rewrites the call - provided this domain is on, which is the
            //   user's own switch to flip (`provider::chat_domain_slugs` gives
            //   it a row under OpenAI). `integrations/openclaw.rs` prints a note
            //   naming this slug rather than enabling it.
            //
            // Both routes work off one split because `engine::apply_rewrite`
            // strips the upstream's own path from the forwarded path exactly as
            // the relay does, so a real `/backend-api/codex/responses` and
            // Codex's rewritten `/codex/responses` both arrive at the gateway as
            // `/codex/responses` under this upstream.
            hosts: vec!["chatgpt.com".into()],
            // Shape matches integrations/codex.rs exactly, because the relay
            // exact-matches the `X-Gate-Upstream-Url` header codex.rs writes:
            // the `/backend-api` segment rides in the upstream here, and the
            // client-side path is the short `/codex/responses` (Codex's
            // base_url is `<relay>/codex`, wire_api appends `/responses`). The
            // gateway concatenates path onto upstream, yielding
            // `https://chatgpt.com/backend-api/codex/responses`. That is the
            // opposite split from the `chatgpt-apps` entry above, which carries
            // the app's real paths off a bare host. The two coexist on one host
            // because `decide` consults every enabled entry rather than
            // stopping at the first - see its docs.
            upstream_url: "https://chatgpt.com/backend-api".into(),
            rewrite_prefixes: vec!["/codex/responses".into()],
            passthrough_prefixes: vec![],
            rewrite_suffixes: Vec::new(),
            enabled: false,
            supported: true,
        },
        ProxyDomain {
            slug: "openrouter".into(),
            display_name: "OpenRouter apps".into(),
            // OpenRouter's API lives at openrouter.ai/api/v1/* (OpenAI-shaped
            // chat/completions). Opt-in like OpenAI; intercepts OpenRouter
            // clients that honor the system proxy.
            hosts: vec!["openrouter.ai".into()],
            // The `/api` MUST ride in the upstream URL, not the forwarded path.
            // Gate's ALB routes `/api/*` (plus /orgs/, /admin/, /me/,
            // /agent-templates/) to the dashboard API, so a forwarded
            // `/api/v1/chat/completions` never reaches the gateway proxy at all
            // - it 404s out of a service that has no such route. Keeping `/api`
            // upstream-side sends `/v1/chat/completions`, which clears the rule,
            // and Gate reassembles the two into the URL OpenRouter serves.
            // `forwarded_paths_avoid_gate_reserved_prefixes` pins this.
            upstream_url: "https://openrouter.ai/api".into(),
            rewrite_prefixes: vec!["/v1/".into()],
            passthrough_prefixes: vec![],
            rewrite_suffixes: Vec::new(),
            enabled: false,
            supported: true,
        },
        ProxyDomain {
            slug: "opencode".into(),
            display_name: "OpenCode Zen / Go".into(),
            // Zen (`/zen/v1/*`) and Go (`/zen/go/v1/*`) are the same host and
            // the same upstream, separated only by path, so they are ONE entry:
            // `decide` returns on the first host match, so a second entry
            // sharing `opencode.ai` would never be consulted.
            hosts: vec!["opencode.ai".into()],
            upstream_url: "https://opencode.ai".into(),
            // Inference endpoints only, same reasoning as Anthropic and OpenAI
            // above: a `/zen/v1/models` preflight carries no model, so the
            // gateway can't classify it and 503s. Both Zen and Go host
            // OpenAI-shaped and Anthropic-shaped endpoints under the same
            // prefix, hence two leaves each. Do NOT widen to "/zen/".
            rewrite_prefixes: vec![
                "/zen/v1/chat/completions".into(),
                "/zen/v1/messages".into(),
                "/zen/go/v1/chat/completions".into(),
                "/zen/go/v1/messages".into(),
            ],
            passthrough_prefixes: vec![],
            rewrite_suffixes: Vec::new(),
            enabled: false,
            supported: true,
        },
    ]
}
