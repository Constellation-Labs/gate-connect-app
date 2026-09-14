//! The built-in domain catalog: every provider the MITM engine may
//! intercept and the relay may route, with the per-entry rationale essays.
//! Split out of `proxy/mod.rs` so the shared routing/injection mechanics
//! there stay readable; the catalog invariants are pinned by the tests in
//! the parent module (`every_resolved_endpoint_lands_on_an_inference_prefix`,
//! `forwarded_paths_avoid_gate_reserved_prefixes`).

use super::ProxyDomain;
use crate::taxonomy::{Client, Credential, Scope};

/// The built-in domain catalog. All entries ship `supported:true` (Anthropic
/// is also `enabled` by default; the rest are opt-in). New providers can be
/// added here and surface in the UI automatically; gate a provider behind
/// `supported:false` until Gate's upstream support for it is confirmed.
pub fn default_domains() -> Vec<ProxyDomain> {
    vec![
        ProxyDomain {
            slug: "anthropic".into(),
            // Named for the SURFACE, under a heading that names the CLIENT.
            // The heading is "Claude Desktop" now rather than "Anthropic", so
            // the label separates that one program's two surfaces - "API" here,
            // "Chat" on the claude.ai entry below - instead of separating the
            // desktop apps from the browser tab, which is a distinction this
            // entry never made. The sentence spelling each one out lives with
            // the UI copy (`src/lib/groups.ts`), because it describes the
            // surface rather than stating a fact the catalog knows.
            //
            // A vendor-shaped name was wrong for a second reason worth keeping:
            // this entry is the system-proxy route for the desktop apps, and
            // its switch is theirs alone. Claude Code reaches the same host,
            // but through its own configured proxy URL, whose route selector
            // makes the engine apply this entry whatever the switch says (see
            // `claude_code_route_domain`) - so a vendor name would read as a
            // promise this switch does not make. The host line carries
            // api.anthropic.com.
            display_name: "API".into(),
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
            // Aimed at the desktop app. Claude Code reaches this same
            // entry through its own proxy URL's route selector
            // (`claude_code_route_domain`), which is a client-scoped route
            // rather than anything this row's switch governs. Brokered: the
            // rewrite injects Gate's credential and drops the caller's.
            client: Client::ClaudeDesktop,
            credential: Credential::Brokered,
            scope: Scope::Host,
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
            display_name: "Chat".into(),
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
            // The `credential` below is what keeps it out of every cascade:
            // `provider::cascade_domains` filters on it, so `provider::enable`
            // cannot route the session surface as a side effect of someone
            // enabling "Claude". The family still LISTS the slug, which is how
            // it reaches the ledger - `buildGroups` (src/lib/groups.ts) gives
            // it a row and a switch under Claude Desktop, and `cascadeTargets`
            // filters it out by the same field - so the only thing that can
            // enable it is that row's own switch (or `proxy domain claude-web
            // on` from the CLI).
            //
            // This used to be enforced by keeping the slug out of a second
            // array. Two arrays meant "not cascaded" and "not on the ledger"
            // were one edit apart, and the rows were invisible for a while
            // because of exactly that.
            //
            // `supported: true` in both environments. This entry was gated to
            // the staging gateway while the gateway-side classification for the
            // surface was staging-only; that classification is deployed to
            // production, so the row is offered everywhere and `enabled: false`
            // above is the only thing keeping it off.
            enabled: false,
            supported: true,
            // The same client as the entry above, which is the whole reason
            // to name a client rather than a surface: these are two surfaces of
            // one program, and a user who asks for "my Claude app routed" means
            // both. Additive because the request carries their own claude.ai
            // session cookie and Gate supplies no key for it - which is also
            // what keeps this row off the family switch, now derived from this
            // field instead of from a second slug array.
            client: Client::ClaudeDesktop,
            credential: Credential::Additive,
            scope: Scope::Host,
        },
        ProxyDomain {
            slug: "openai".into(),
            // "OpenAI API", not "OpenAI apps" and not the vendor name: this is
            // the API HOST, and what the row is about is which host Gate
            // intercepts. It must not read as including Codex (config-routed;
            // its embedded agent ignores the system proxy) or the ChatGPT
            // desktop app (on chatgpt.com).
            //
            // The host itself is deliberately NOT in the label. The popover row
            // already prints `api.openai.com` in its own mono identifier slot -
            // duplicating it in a sans label would say it twice there and break
            // the rule that identifiers are mono. It goes in the row's
            // description instead (`MEMBER_DESCRIPTIONS` in
            // `src/lib/groups.ts`), which is a full line in the window UI, where
            // the host appears nowhere else.
            //
            // Not in the OpenAI family at all any more, and not on its
            // surface-kind naming. This is the generic API host: nothing OpenAI
            // ships rides its switch. Codex is config-routed through the relay,
            // which resolves against the whole catalog rather than the enabled
            // set, so it routes whatever this says; the ChatGPT desktop app is
            // on chatgpt.com and is the `chatgpt` entry below. What flipping
            // this does is intercept api.openai.com for any system-proxy-
            // honouring client - which in practice means the multi-provider
            // harnesses, since OpenClaw and Hermes blind-tunnel anything outside
            // the enabled catalog. So the row is `Client::AnyApp`, which puts
            // it under "Any app on this machine" with the other entries that
            // cover whatever reaches them, and `provider.rs` does not list it.
            //
            // Not on the family's surface-kind scheme ("App" there means the
            // ChatGPT desktop app) because it is not in that family and not a
            // product surface at all - it is a host.
            display_name: "OpenAI API".into(),
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
            // Nothing OpenAI ships rides this switch - Codex is config-routed
            // through the relay, the desktop app is on chatgpt.com - so what
            // actually depends on it is whatever else on the machine talks to
            // api.openai.com, which is what `AnyApp` says out loud. It is also
            // why this row no longer needs an "Experimental" heading to live
            // under.
            client: Client::AnyApp,
            credential: Credential::Brokered,
            scope: Scope::Host,
        },
        ProxyDomain {
            slug: "chatgpt-apps".into(),
            display_name: "Chat".into(),
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
            // Like `claude-web` above, this entry is `Credential::Additive`:
            // it gets a ledger row and a switch under ChatGPT, and every
            // cascade skips it, so the chat half of this entry
            // (`/backend-api/f/conversation`, a session-cookie surface) can
            // only be enabled from its own row or from `proxy domain
            // chatgpt-apps on`. Whatever else exposes this entry must keep
            // that property.
            //
            // And, like `claude-web`, offered in both environments: the staging
            // gate both entries carried is gone now that the gateway classifies
            // these surfaces in production, so `enabled: false` is what keeps
            // this one off until its own switch says otherwise.
            enabled: false,
            supported: true,
            // The app's own chat turn, plus the browser tab sharing the host;
            // `rules_for_client` narrows only the browser. Additive for the
            // same reason as claude-web: a session cookie, not a brokered key.
            client: Client::ChatGpt,
            credential: Credential::Additive,
            scope: Scope::Host,
        },
        ProxyDomain {
            slug: "chatgpt".into(),
            display_name: "Subscription".into(),
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
            //   user's own switch to flip (it is `Credential::Additive`, so it
            //   gets a row under ChatGPT and no cascade reaches it).
            //   `integrations/openclaw.rs` prints a note naming this slug
            //   rather than enabling it.
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
            // Host, not Client, even though Codex arrives through the relay:
            // the OpenClaw route documented above is plain interception, so
            // flipping this decrypts chatgpt.com for every proxy-honouring
            // client on the machine. Additive - the bearer is the user's own
            // ChatGPT subscription token, passed through untouched.
            //
            // Which client this is *aimed* at is entangled with the unsettled
            // Cowork question (`src-tauri/src/lib.rs`'s `AGENT_PROCESSES` notes
            // two contradictory captures and says at least one reading is
            // wrong). `ChatGpt` keeps the row where the ledger already drew it,
            // under the app whose host it shares, rather than deciding that
            // question here.
            client: Client::ChatGpt,
            credential: Credential::Additive,
            scope: Scope::Host,
        },
        ProxyDomain {
            slug: "openrouter".into(),
            display_name: "OpenRouter".into(),
            // The vendor name, because the heading no longer carries it: this
            // row sits under "Any app on this machine" with the other entries
            // that cover whatever happens to reach their hosts, so "Proxy" -
            // which said by what mechanism it routes, under a heading that
            // already read "OpenRouter" - would now name nothing at all.
            // OpenRouter has no Gate Connect
            // integration and no chat surface, so unlike the families above
            // there is no App/Web/CLI split to make.
            //
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
            // Any app the user has pointed at OpenRouter, which is what this
            // family's blurb already said in prose and no field could hold.
            client: Client::AnyApp,
            credential: Credential::Brokered,
            scope: Scope::Host,
        },
        ProxyDomain {
            slug: "opencode".into(),
            display_name: "Zen / Go".into(),
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
            // prefix, so each surface needs a leaf per shape. Zen has three:
            // it splits the OpenAI shape across `/chat/completions` (DeepSeek
            // and friends) and `/responses` (the GPT family), and a missing
            // leaf is silent - `classify` calls it passthrough, so the request
            // goes straight to opencode.ai under the user's own Zen key and
            // never reaches Gate. Go documents only the two.
            // Do NOT widen to "/zen/".
            rewrite_prefixes: vec![
                "/zen/v1/chat/completions".into(),
                "/zen/v1/responses".into(),
                "/zen/v1/messages".into(),
                "/zen/go/v1/chat/completions".into(),
                "/zen/go/v1/messages".into(),
            ],
            passthrough_prefixes: vec![],
            rewrite_suffixes: Vec::new(),
            enabled: false,
            supported: true,
            // Zen and Go are OpenCode's own hosted models, so the row belongs
            // with the editor rather than under a vendor heading of its own.
            client: Client::OpenCode,
            credential: Credential::Brokered,
            scope: Scope::Host,
        },
    ]
}
