# claude.ai Chat Traffic — Observed Wire Format

**Status:** informal, observation-derived. This describes an *undocumented internal frontend API*.
The overall shape (hosts, path families, SSE event sequence) is stable enough to build filters
against. Individual body field names drift without notice. **Treat a fresh capture as the source
of truth; treat this document as the map.**

Last reviewed: 2026-07-28

---

## 1. Scope

| In scope | Out of scope |
|---|---|
| claude.ai web app | `api.anthropic.com` / Messages API |
| Electron renderer process in Claude Desktop | Node side of `claude.exe` |
| | Claude Code (`ANTHROPIC_BASE_URL` path) |

The distinguishing tell: if you are seeing `POST /v1/messages`, you are on the API path, not this one.

---

## 2. Hosts

| Host | Contents |
|---|---|
| `claude.ai` | App HTML, JS bundles, and the entire `/api/` surface |
| `*.anthropic.com` CDN/static hosts | Assets, fonts, images |
| Third-party analytics / error / flag hosts (Segment, Sentry, Statsig-class) | Telemetry, feature flags |

`api.anthropic.com` does **not** appear in this traffic.

---

## 3. Request families

Every authenticated path is scoped by an organization UUID, written below as `{org}`.
Conversation UUID is `{conv}`.

### 3.1 Session bootstrap

Fires on app load, before any chat activity.

```
GET /api/bootstrap
GET /api/organizations
```

Returns account identity, organization UUID(s), entitlements, and active feature flags.
**This is where `{org}` comes from** — you need it to construct or match every subsequent path.

### 3.2 Conversation CRUD

```
GET    /api/organizations/{org}/chat_conversations
       -> sidebar list

POST   /api/organizations/{org}/chat_conversations
       -> creates conversation, returns {conv}

GET    /api/organizations/{org}/chat_conversations/{conv}?tree=True&rendering_mode=messages
       -> full message history on open

PUT    /api/organizations/{org}/chat_conversations/{conv}
       -> rename / metadata update

DELETE /api/organizations/{org}/chat_conversations/{conv}
```

### 3.3 Completion — the primary flow

```
POST /api/organizations/{org}/chat_conversations/{conv}/completion
```

Request body, approximate shape:

```jsonc
{
  "prompt": "string",
  "parent_message_uuid": "uuid",     // threading; also used for edit/regenerate
  "timezone": "Area/City",
  "attachments": [ /* pasted text blobs, inline */ ],
  "files":       [ /* references to previously uploaded files */ ],
  "sync_sources":[ /* connector-sourced context */ ],
  "personalized_styles": { /* style feature */ },
  "rendering_mode": "messages"
}
```

**Absent, and significant:**

- no `messages[]` array — conversation history lives server-side, keyed by `{conv}`
- no `model` field in Messages-API form
- no `max_tokens`, no `system`

Consequence: the request is **not self-contained**. It cannot be replayed or forwarded to an
Anthropic-compatible endpoint without first reconstructing history from
`GET .../chat_conversations/{conv}`. This is the structural reason web chat has no gateway seam.

### 3.4 Ancillary

Noisy, generally ignorable for audit purposes:

- conversation title generation
- feedback / thumbs signals
- artifact persistence
- usage and rate-limit counters

---

## 4. Response format

Completions respond with Server-Sent Events.

```
Content-Type: text/event-stream
Transfer-Encoding: chunked
```

Event sequence mirrors the public streaming API closely enough that an existing SSE parser can be
reused:

```
message_start
  content_block_start
  content_block_delta      x N     <- delta.text carries output
  content_block_stop
message_delta                      <- stop_reason, usage
message_stop

ping                               <- keepalive, discard
error                              <- mid-stream failure
```

Tool use and artifacts appear as their own `content_block` entries with distinct inner types.

### Proxy implications

- Flows appear to hang in mitmweb until the stream closes — this is normal, not a stall.
- Streamed bodies bypass normal intercept/modify ergonomics. Observation is fine; mid-stream
  mutation is impractical.
- If you need whole bodies for offline analysis, prefer a save-stream / har export over
  interactive interception.

---

## 5. Authentication

```
Cookie: sessionKey=sk-ant-sid01-...
```

Plus CSRF and anti-abuse headers (`anthropic-client-*` family, device/fingerprint values).

There is **no** `x-api-key` and **no** `Authorization: Bearer`. Any gateway logic that rewrites,
injects, or validates an API key is a no-op against this traffic.

---

## 6. mitmproxy filters

```
# completions only
~d claude.ai & ~u /completion & ~m POST

# whole app API, statics excluded
~d claude.ai & ~u ^/api/

# drop telemetry noise
~d claude.ai & !~u (sentry|segment|statsig|analytics)

# conversation history fetches (useful for reconstructing context)
~d claude.ai & ~u chat_conversations & ~m GET
```

---

## 7. Validation procedure

Before hardcoding anything above:

1. Start capture with `~d claude.ai & ~u ^/api/`.
2. Load claude.ai fresh — capture `/api/bootstrap`, record the real `{org}`.
3. Create a new conversation, send one message, let the stream complete.
4. Close the conversation, reopen it — capture the history `GET`.
5. Diff observed field names against §3.3 and §4.

Path families under `/api/organizations/` have been stable for a long period. Request body fields
are the part that moves.

---

## 8. Summary for Gate Connect

Web chat is a closed loop between Anthropic's own frontend and its own backend. Under MITM it is
fully **observable** — useful for audit, visibility, and usage accounting. It is **not routable**:
there is no base-URL override, no API-key seam, and the completion request omits the history needed
to reconstruct an equivalent Messages API call.

Routable surfaces remain:

- Claude Code — `ANTHROPIC_BASE_URL` / `ANTHROPIC_CUSTOM_HEADERS`
- Claude Desktop — third-party inference / Gateway mode
