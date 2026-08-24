# Spec: downgrade the Responses WebSocket so work mode is capturable

Status: proposed. Owner: unassigned. Supersedes nothing; it revisits the
decision recorded in `638cbb5` ("never rewrite a protocol upgrade to the
gateway") without reversing its reasoning.

## Problem

ChatGPT desktop **work mode** is invisible to Gate. Not partially: entirely.

Measured over a work-mode capture (`flows-work`, ChatGPT desktop on Windows):

| | count |
| --- | --- |
| requests to `/backend-api/codex/responses` | 6 |
| of those, answered `101 Switching Protocols` | **6** |
| plain HTTP turns on `/backend-api/f/conversation` | **0** |
| client `response.create` frames across those sockets | 13 |
| of those carrying a non-empty `input[]` (roles `user` / `developer`) | 12 |

Every turn is sent as WebSocket text frame 0 on a socket opened with
`openai-beta: responses_websockets=2026-02-06`. The first frame is ~47 KB and
carries the whole Responses request: `model`, `instructions`, `tools`,
`prompt_cache_key`, and the user's turn in `input[]`.

An earlier note in this workstream described the coverage as "partial, the
HTTP call is captured". That was a miscount: the six `/codex/responses`
requests **are** the six upgrades, counted once as HTTP and once as WebSocket.
There is no HTTP turn in work mode.

Plain chat is unaffected and already captured: it uses
`/backend-api/f/conversation` over ordinary HTTP, in both the app and the
browser. Browser work mode is also already captured, because the website uses
the same `/f/conversation` path and opened zero WebSockets in either capture.
**The gap is exactly one client in exactly one mode.**

## Why this is worth revisiting now

Gate decided not to support WebSocket as a transport, which left work mode
permanently uncaptured under that decision. What changed is not the decision
but a fact about the client: **the app already has an HTTP fallback.**

Recovered from the native `codex` binary bundled in the official ChatGPT Linux
app (`chatgpt 26.810.52044`, `resources/codex`, extracted with `dpkg-deb -x`,
not installed and not run). The Rust source coordinates survive in the
binary's panic/log metadata:

```
core/src/client.rs:532   codex_core::client   "falling back to HTTP"
                         fields: from_wire_api, responses_websocket

"Responses WebSocket failed; HTTPS fallback may still work"
"Responses WebSocket timed out; HTTPS fallback may still work"
"websocket connection is unavailable"

transport variants:  responses_http | responses_websocket
telemetry:           model_client.stream_responses_api, transport,
                     model_client.websocket_connection
```

There is also a preflight diagnostic, `network.websocket_reachability`, whose
remediation string reads:

> Check proxy, VPN, firewall, DNS, custom CA, and WebSocket policy support.

That is the load-bearing find. OpenAI **anticipated a corporate proxy blocking
WebSocket and built the fallback for it.** Gate is that proxy. Using the
fallback is using a supported path, not exploiting a bug.

Two distinct triggers are visible: handshake **failure** and handshake
**timeout**. They matter differently (see "Refuse cleanly" below).

## Proposed change (this repo)

When an upgrade request targets a routed Responses path on an intercepted
host, **refuse the upgrade with a clean HTTP error** instead of passing it
through. The client then retries over HTTPS, which the existing route already
carries to Gate.

Today `handle_request` in `crates/core/src/proxy/engine.rs` consults
`is_upgrade_request()` and passes any upgrade through untouched. That guard
stays: it is still correct that Gate must never *rewrite* an upgrade to the
gateway, because the gateway speaks HTTP and would receive a bodyless GET.
This adds a third outcome beside "rewrite" and "pass through": **decline**.

Scope the decline as narrowly as the evidence supports:

- host is an intercepted host with an enabled row (today: `chatgpt.com`)
- path matches the Responses path the row already routes
  (`/backend-api/codex/responses`)
- the request is an upgrade (`Upgrade: websocket` + `Connection: upgrade`)

Deliberately **not** matched on the `openai-beta` value. The binary also
carries `responses_websockets_v2`, so the negotiation string is still moving;
keying on it would silently stop working on a version bump, and the failure
would look like "work mode is uncaptured again" rather than like a broken
matcher.

### Refuse cleanly, do not drop

Answer `400` with a small JSON body. Do not black-hole the connection and do
not let it hang.

The client has two fallback paths and they cost the user differently: a
refusal hits "failed" and falls back immediately, while a dropped or stalled
connection hits "timed out" and adds the full handshake timeout to **every
turn**. Same end state, one of them noticeably slower. A silent drop would
also be indistinguishable, from the user's side, from Gate being broken.

### Kill switch, not a code path

This depends on undocumented client behaviour that a vendor update can change.
It must be switchable off without a release: a config flag, defaulting **off**
until the verification below passes, and reachable by whoever is on call. If
the fallback ever stops firing, the failure mode is not "work mode is
uncaptured" (today's status quo) but "work mode does not work at all", which
is strictly worse than doing nothing.

That asymmetry is the main argument for the flag and for staged rollout.

## Prerequisite (gate repo) - do this first

**The Connect change alone yields captured-but-empty rows.** When work mode
falls back, the request arrives at `/backend-api/codex/responses` in Responses
shape, whose turns live in `input[]`. Two packages disagree about that shape:

- `packages/prompt-extraction` **does** handle it: `normalizeBody` branches on
  `parsed.input` into `normalizeResponsesInput`, so the turn is **scanned**.
- `packages/turn-assembly` **does not**: `extractTurns` reads `system`,
  `tools`, `messages[]` and the bare `prompt` fallback, and nothing else.
  `TurnPersistenceService.prepareTurns` hands it the raw body, so a Responses
  body yields **zero turns**, `session_turns` gets nothing, and
  `last_user_turn_id` stays NULL.

The visible result would be a row with a model, a cost and a security verdict,
under a conversation showing no user message: the same defect class as the
ChatGPT web-chat bug fixed in #844, arriving by a different route.

So the order is:

1. **gate**: teach `extractTurns` the Responses `input[]` shape, the same way
   `webChatMessage` taught it the web-chat shape - a local flattener, since
   turn-assembly is deliberately dependency-free and cannot import
   `normalizeResponsesInput`. Mirror that function's part-type handling
   (`input_text` / `output_text`, skip `function_call` and friends) so the two
   packages cannot drift on what counts as user-authored text.
2. **gate**: decide whether these turns deserve a surface of their own.
   `detectSurface` currently returns `api` for this path (it matches none of
   `CHATGPT_CHAT_RE`, `CODEX_MCP_RE`, `CODEX_TASKS_RE`), which is defensible -
   it *is* a Responses API call - but it means work mode will not appear under
   any chat surface filter in the dashboard. Naming it (`codex-responses`?)
   is a product call, not a technical one.
3. **connect**: this spec, behind the flag.

Step 1 is worth doing regardless of whether step 3 ever ships: it also fixes
turn storage for Codex CLI and IDE traffic, which reaches Gate over plain HTTP
today and is subject to the same empty-turns gap.

## Verification

**Stage 0 - confirm the fallback fires (before writing any Rust).**
`no_ws_fallback_probe.py` refuses the upgrade on that one path from
mitmproxy and logs both events. Run two or three work-mode turns, at least one
in a fresh conversation, in case the transport choice is cached per session.

Pass: a plain `POST /backend-api/codex/responses` with a JSON body appears.
Fail: the app errors, or loops on failed upgrades.

Stage 0 is cheap and answers the only real unknown. Do not skip it: the binary
strings prove a fallback path **exists**, not that it fires for work mode, and
the vendor's own wording is hedged ("may still work").

**Stage 1 - one machine, flag on.** Confirm turns land with content (not just
rows), that cost and model attribute correctly, and measure the added latency
of the extra round trip on the first turn of a conversation.

**Stage 2 - staged rollout**, with the flag as the rollback.

## Non-goals

- WebSocket support in Gate. This spec exists specifically to avoid it.
- Any change to plain chat, in the app or the browser. Both already work.
- Any change to browser work mode, which already uses the HTTP chat path.
- Reversing `638cbb5`. Upgrades still must never be rewritten to the gateway.

## Open questions

1. Does the fallback fire for work mode specifically, or only for the paths
   the diagnostic covers? Stage 0 answers this.
2. Is transport choice cached per session, per conversation, or per launch? It
   determines whether a refusal costs one round trip or one per turn.
3. What does `responses_websockets_v2` change? If it moves the endpoint, the
   path matcher needs revisiting.
4. Should the decline be scoped to a client class? The browser never opens
   these sockets, so `ClientClass` narrowing buys nothing today, but it would
   contain the blast radius if another client started using the transport.
