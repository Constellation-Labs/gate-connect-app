# AG-578: live security-event feed API contract

Proposal for the gateway endpoint behind Gate Connect's live blocked/flagged
event feed. Written 2026-08-31 for stakeholder review.

> **Status, 2026-08-31: built, and the proposal moved in three places while it
> was being built.** The differences are listed here rather than edited into the
> sections below, because this document is the record of what was proposed and
> the code is the record of what is served - and a reader who cannot tell which is
> which trusts the wrong one.
>
> | Proposed here | Shipped |
> |---|---|
> | Per-org Redis channel, with per-user filtering an open question (§9.3) | **Per-`(org, user)` channel.** Not a tightening for safety's sake: `@gate/auth` scopes `securityEvents` self-only for *every* role including Owner, and `ActivityController` records what conflating the two visibility families once cost. A per-org channel would have been that same mistake over a push transport. This also answers §9.3's volume question - one person's events, not an org's. |
> | Both credential types accepted, mirroring `/v1/me/activity` | **A credential with no user identity is refused**, matching what `tool-events` already does. An org-scoped `sk-gw-*` key belonging to nobody resolves no "self", so there is no correct set of events to send. Dashboard-minted keys carry their creator's `user_id` and are unaffected. |
> | "Give this route its own throttler, keyed on the credential" (§4) | **`@SkipThrottle()` plus a per-person cap of 5 concurrent streams.** The address-keyed bucket is skipped for the reason §4 gives; what replaced it counts open streams rather than requests, because a streaming route makes one request and then holds a socket for hours - concurrency is the thing worth bounding, not rate. |
>
> `hello.recovery` ships as proposed and is `false` in every deployment without
> `REDIS_URL`. The replay buffer itself (§7, phase 2) is **not built**: the bus
> fans out but does not retain, so `recovery` is currently false everywhere Redis
> is absent and would need the buffer before it can be true anywhere.

Companion to AG-578 (implementation). Cross-repo: the endpoint lands in `gate`,
the consumer is `gate-connect-app`. Deliberately modelled on
`docs/ag-572-activity-api-contract.md`, which is the record of how the last
cross-repo contract for this app was agreed, and whose §11 consumer notes apply
here almost unchanged.

## 1. Why this document exists

- AG-578's acceptance criteria name a "live-delivery service" that does not
  exist. Nothing in `gate` matches `live-delivery`, `liveDelivery` or
  `live_delivery`.
- The criteria describe latency, recovery, dedupe and connection state, but no
  endpoint, transport, schema or auth scheme. As with AG-572, the contract is
  unowned, and that being unowned is the failure mode AG-572 recorded.
- The finding that motivates this document, and it is the same one AG-572 made:
  **most of the data already exists.** This is a live transport over an event
  model the gateway already serves, not a new security subsystem.

## 2. What already exists

### In `gate`

- `GET /v1/me/tool-events` (`apps/gateway-proxy/src/activity/activity.controller.ts:467`)
  already serves per-request security verdicts, cursor-paginated, org- and
  installation-scoped, with per-row visibility gating at `:753`
  (`mine ? toAction(...) : null`).
- `deriveSecurityCategory()`
  (`apps/gateway-proxy/src/proxy/repositories/request.repository.ts:29`) already
  reduces firing criteria to `credential | phi | pii | injection | other`,
  priority-ordered. It is persisted as `securityCategory` on `gateway_requests`
  (`:657`) and its keyword sets are kept in lockstep with migration 37's SQL
  backfill. **This is AG-578's "event category".** A third implementation would
  drift from both.
- `scrubSecurityResults()` and `sanitizeForPersist()`
  (`apps/gateway-proxy/src/proxy/repositories/security-results-scrubber.ts`) are
  what already keep raw match evidence out of `security_decisions.results`.
- The block/flag handlers already **emit their own fire-and-forget events**,
  documented on `persistRequest`'s `auditEmit` parameter
  (`request.repository.ts:368`), which those handlers deliberately bypass. That
  is the publish point.
- `SseWriter` (`apps/dashboard-api/src/gatekeeper/sse-writer.ts`) is a
  worked-out server-side SSE implementation: `text/event-stream; charset=utf-8`,
  `cache-control: no-cache, no-transform`, `x-accel-buffering: no`,
  `flushHeaders()`, a 15s comment-frame keepalive as intermediary/ALB
  idle-timeout insurance, awaited `drain` backpressure and destroyed-socket
  detection.
- `ioredis ^5.4.1` is already a `gateway-proxy` dependency, and
  `packages/alerting` already implements dedupe, rate limiting and storm
  suppression for the server's own sinks.

### In `gate-connect-app`

- `src/lib/toolEvents.ts` already maps the row shape and already argues out the
  privacy semantics, including why `securityAction: null` is deliberately
  ambiguous between "no decision recorded" and "you may not see this row".
- `crates/core/src/gateway_api.rs` holds the credential rules and the
  `FailureCode` taxonomy (`Offline | SignedOut | NoOrg | Rejected | Gateway |
  Unknown`) the UI branches on.
- `src/components/gc/AppPane.tsx` already draws the `BLOCKED` / `FLAGGED` /
  `REDACTED` pills and the feed table they sit in.

## 3. Placement decision

Three options were considered. **Recommendation: option A**, for the same
reasons AG-572 chose it, which have not changed.

- **A. New route on `gateway-proxy`, `GET /v1/me/security-events/stream`.**
  Recommended.
  - Gate Connect stores exactly one URL and one auth header. No new
    configuration, no second host.
  - Sits beside `/v1/me/activity`, `/v1/me/tool-events` and `/v1/me/orgs` and
    reuses the guard shape already proven for this client.
  - `gateway-proxy` is where the decision is made, so the publish is in-process
    with the event rather than a hop away.
- **B. A dedicated streaming service.** Not recommended. It is the "live-delivery
  service" the ticket's wording implies, and it buys nothing this transport
  needs while adding a host, a deploy, a CORS surface and a second auth
  integration.
- **C. Stream from `dashboard-api`.** Not recommended, for AG-572 §3's option B
  reasons: a second base URL in `account.json`, a second auth header shape, and
  coupling the shipped desktop app to an internal API that was never a public
  contract. Note this is where `SseWriter` lives; lift the class, not the route.

**Condition on option A:** the live feed and `GET /v1/me/tool-events` must share
one visibility gate. Two implementations of "may this caller see this row" is
how a colleague's event leaks, and it is the same argument AG-572 §3 made about
two implementations of a counter.

## 4. Endpoint definition

- **Route:** `GET /v1/me/security-events/stream`
- **Service:** `apps/gateway-proxy`, registered **before** the `@All("*")` proxy
  catch-all (the same constraint `OrgSelectionModule` lives under).
- **Auth: both credential types**, exactly as `/v1/me/activity`.
  - OAuth: `X-Gate-Authorization: Bearer <cognito access token>` plus
    `X-Gate-Org-Id`. The custom slot, not standard `Authorization`.
  - API key: `sk-gw-*` through the normal `ApiKeyGuard` path, which resolves the
    org from the key.
- **Org scoping:** from the credential, never trusted from the header.
- **Installation scoping:** `?installId=` optional, mirroring `/v1/me/activity`.
  See open decision 9.3.
- **Resumption:** standard `Last-Event-ID` request header.
- **Throttling: this route needs its own throttler, keyed on the credential.**
  The default is `{ ttl: 60_000, limit: 100 }` keyed on **IP**
  (`AppModule.forRoot`), and `ProxyController` is `@SkipThrottle()`'d so
  inference is exempt but the `/v1/me/*` family is not. The activity
  controller's own doc comment already calls this out: "If the client ever
  refreshes on a timer rather than on open, this route wants its own throttler
  keyed on the credential rather than the address." A long-lived SSE connection
  is that case. Without it, one office NAT reconnecting after a deploy 429s
  itself, and a 429 raised here is indistinguishable to the client from a 429
  raised by the proxy path.

## 5. Frame schema

Response headers copy `SseWriter` verbatim, including the 15s keepalive.

### `hello`, once, first

```jsonc
{
  "recovery": true,            // whether Last-Event-ID will be honoured
  "heartbeatMs": 15000,
  "scope": { "orgId": "uuid", "installId": null }
}
```

**`recovery` is load-bearing, not decorative.** AC5 says Gate Connect retrieves
missed events "when the service supports recovery"; this field is that
conditional made mechanical, so the client reads the answer instead of guessing
it. It is `true` only when a shared store backs the replay buffer. See §7.

### `security-event`, zero or more

```
event: security-event
id: 01J8Z3QK9V0000000000000000
data: { … }
```

```jsonc
{
  "requestId": "uuid",              // the dashboard deep-link key
  "at": "2026-08-31T14:03:00Z",
  "securityAction": "block",        // "block" | "flag" only on this stream
  "securityCategory": "credential", // credential|phi|pii|injection|other|null
  "tool": "claude-code",            // nullable; see §6
  "model": "claude-opus-4",         // nullable
  "provider": "anthropic"           // nullable
}
```

- **`id:` is a ULID**, monotonic by time, so `Last-Event-ID` both resumes and
  orders. Deliberately not `requestId`: one request can produce several
  decisions (request phase and response phase both record one), so `requestId`
  is not unique per event and would silently collapse two.
- **`retry:` is sent once** with the server's preferred base delay.

### `bye`, zero or one

```jsonc
{ "reason": "shutdown" }   // "shutdown" | "superseded" | "unauthorized"
```

Lets a rolling deploy tell clients to reconnect immediately rather than
discovering a dead socket on the next keepalive.

## 6. Field derivation

- **`securityAction`** - the `security_decisions.action` column, filtered to
  `block` and `flag`. `redact` and `allow` are **not** on this stream: AG-578 is
  about blocked and flagged events, and `redact` in particular is a
  high-volume, low-signal action that would drown the feed. The Overview's
  counters and the tool-events table continue to carry all four.
- **`securityCategory`** - `deriveSecurityCategory()`, unchanged. Null when no
  criterion fired with a name the CASE chain recognises.
- **`tool`** - `gateway_requests.client_tool`, the `x-gate-client` header
  derived from the caller's User-Agent against a small allowlist. **`null` is
  ordinary, not exceptional**: `docs/ag-572-activity-api-contract.md:484-502`
  records that an agent not on the allowlist is `NULL` and never a guess, on the
  grounds that filing one tool's traffic under another's name in the view the
  user reads to find out what their machine is doing would be worse than saying
  nothing. The feed's UI must treat `null` the same way.
- **`model`, `provider`** - as `tool-events` serves them.

## 7. Recovery, and what it costs

Two mechanisms, and the client picks by reading `hello.recovery`.

- **`recovery: true`** - the server keeps a capped per-org replay buffer (Redis
  list or stream, TTL ~1h) keyed by ULID and replays from `Last-Event-ID`.
- **`recovery: false`** - no shared store, so no buffer and no cross-pod
  fan-out. The client falls back to **one** `GET /v1/me/tool-events` re-read
  filtered to block/flag since its last seen timestamp. One request on
  reconnect, not a poll.

**Why the conditional is real rather than defensive.** `gateway-proxy` treats
Redis as optional: `config/config.service.ts:920` records "Absent `REDIS_URL`
=> gateway falls back to per-pod in-memory cache", and the throttler storage
makes the same choice. A single-pod local or self-hosted deployment genuinely
has no shared store, and a stream that claimed replay it could not deliver
would silently lose events at exactly the moment the user is told it recovered.

**Multi-pod fan-out is the same problem.** A per-pod in-memory subscription only
sees decisions its own pod made, so with N pods a client sees roughly 1/N of its
org's events and nothing says so. Redis pub/sub on a per-org channel is what
makes the feed correct at N>1, and `ioredis` is already a dependency. This is
not an optimisation; without it the feed is wrong in production and right in
development, which is the worst way round.

**Dedupe stays client-side regardless**, because a replay boundary and a
backfill window both overlap by design. See the app-side plan.

## 8. Privacy constraints

AG-578 AC3: events must not include prompt text, response text, matched
credentials or secrets, raw evidence, or sensitive values in full.

- Enforce by **omitting these fields from the DTO entirely**, not by filtering
  client-side. AG-572 §7's reasoning applies unchanged: a field that never
  crosses the wire cannot leak through a log, a crash report, or a future UI
  change.
- `security_decisions.results` is `jsonb` and holds match evidence. It must
  **never** be selected into this payload, not even scrubbed.
  `scrubSecurityResults()` is the reference for what "raw evidence" means, not a
  licence to send a scrubbed version of it.
- `conversationTitle` is **omitted from this stream**, unlike `tool-events`
  where Figma 272:3286 restored it and product accepted what the label is. A
  notification that fires unprompted on a desktop, possibly on a shared screen,
  is a different surface from a table the user opened, and AC3 names prompt text
  explicitly.
- `sessionRef` is omitted: it identifies a conversation and earns nothing on a
  feed keyed by request.
- The per-row visibility gate from `tool-events:753` applies before an event is
  published to a subscriber, not after.

## 9. Open decisions for stakeholders

1. **AC1's "95% within 3s" has no measurement anywhere.** Neither repo measures
   delivery latency, and "receipt by the live-delivery service" names a
   component that does not exist, so the clock has no start. Decide whether this
   ticket adds instrumentation (a publish timestamp in the frame, compared at
   the client) or the figure is aspirational. An acceptance criterion nobody can
   evaluate is not one.
2. **Is `redact` really out?** §6 excludes it on volume grounds. If the product
   intent is "everything the guardrails did", the feed needs a filter control
   and the volume question in 9.3 gets sharper.
3. **Volume for a high-traffic org.** A per-org channel with no scoping could
   push thousands of events an hour at a desktop app that renders every one.
   Decide whether the stream is **installation-scoped by default** (matching the
   Overview's picker, and arguably the honest reading of "events on this
   machine") and whether the server caps events per connection per minute.
4. **Who may see which events.** `docs/ag-572-activity-api-contract.md:436`
   records that the `securityEvents` policy "already answers no for everyone"
   for cross-installation visibility. Confirm the live feed inherits that
   answer rather than quietly widening it, since a stream is a much easier
   surface to over-share on than a table.
5. **Whether `packages/alerting`'s grouping policy is AC5's "notification
   grouping rules"**, or whether a separate client-side spec is intended. If the
   former, the app should lift its constants so client and server agree on what
   a storm is; if the latter, the rules need writing down.
6. **AC7's "The dashboard uses the selected organization."** The app deliberately
   puts no org id in the dashboard URL: `src/lib/config.ts` records that the
   dashboard resolves the active org from its own session and a client-supplied
   one would be ignored or disagree. If the dashboard session and the app's
   selected org can diverge, that is a real gap, but it is a dashboard-side fix
   and not a link format. Confirm which is meant.
7. **Retention of the replay buffer.** ~1h is proposed. An app closed overnight
   backfills from `tool-events` instead, which is bounded by that endpoint's own
   retention. Confirm both.

## 10. Suggested phasing

- **Phase 1, the smallest correct slice.** The endpoint with `recovery: false`,
  the credential-keyed throttler, the shared visibility gate, and the Rust
  client + pane in the app. Backfill goes through `tool-events`. This satisfies
  every acceptance criterion except the replay half of AC5, and it is the slice
  that proves the transport.
- **Phase 2, correctness at scale.** Redis pub/sub fan-out and the replay
  buffer, flipping `hello.recovery` to true. **Phase 1 is only honest on a
  single pod**, so this is not optional before production; it is sequenced
  second because it is testable against a working phase 1.
- **Phase 3, notifications.** The per-category switches and grouping rules,
  which are app-side and depend on nothing new from the gateway.

## 11. Consumer notes for `gate-connect-app`

AG-572 §11 applies almost unchanged, and is worth repeating because each item
is a bug someone already hit:

- **The connection must live in Rust, not the webview.** `src-tauri/tauri.conf.json`'s
  CSP `connect-src` does not include gateway origins, and widening it would be
  the wrong fix since the webview also holds no token.
- **`.no_proxy()` is mandatory**, or the app's own machine-wide `HTTPS_PROXY`
  export captures the call and the injected `X-Gate-Api-Key` produces a spurious
  401.
- **Take the token from `oauth::live_session()`**, which refreshes, not
  `oauth::current()`, which is a raw keychain read.
- **Only a definite 401 is a rejection.** A feed failure must never sign the
  user out. This is the `SessionProbe` discipline from `org.rs`.
- **The async client, not the blocking one.** `gateway_api::call_json` is
  `reqwest::blocking` with a 15s timeout; a long-lived stream cannot use it. The
  `stream` feature is already enabled for `proxy::relay`.
- **Mirror the `GATE_CONNECT_TEST_*_ENDPOINT` seam** via `env::test_seam`, which
  refuses to honour seams in release builds.

## 12. What was built, and where

**`gate`, branch `feat/ag-578-security-event-stream`:**

- `apps/gateway-proxy/src/security-events/security-event-bus.ts` - the fan-out.
  Redis pub/sub when `REDIS_URL` is set, an in-process emitter when it is not,
  and `supportsRecovery` is the difference the client is told about.
- `.../publish-decision.ts` - reduces a committed decision to a feed event.
  `feedAction` keeps `redact` and `allow` off the stream and lets `block` outrank
  `flag`.
- `.../security-events.controller.ts` - the SSE route, the self-only check, the
  concurrent-stream cap, and a local `SseStream` modelled on `dashboard-api`'s
  `SseWriter`.
- `.../security-events.module.ts`, registered in `app.module.ts` **before**
  `ProxyModule`.
- The publish is called from `RequestRepository.persistRequest` **after** its
  transaction commits, synchronously and non-throwing.
- 37 tests across three spec files; the whole `gateway-proxy` suite (5266) passes.

**`gate-connect-app`, branch `feat/new-app-ui`:**

- `crates/core/src/security_feed/` - `sse.rs` (the frame decoder), `mod.rs`
  (types, dedupe, backoff, credentials), `client.rs` (the connection and its
  state machine), `notify.rs` (notification grouping).
- `src/lib/securityFeed.ts`, `src/components/gc/SecurityPane.tsx`, a third
  sidebar entry, a tray card, three Settings switches.
- `Pill` and `BADGE_STYLES` lifted from `AppPane` into `base.tsx`, because two
  surfaces draw them now.

## 13. What is still open

Beyond §9's decisions, two things this document proposed and the build did not
do:

1. **The replay buffer.** `hello.recovery` is wired end to end and is the honest
   answer today, which is `false` without Redis. Making it `true` needs the
   capped per-org list and the `Last-Event-ID` read in §7. Until then every
   reconnect backfills through `/v1/me/tool-events`, which is correct but heavier
   than a resume.
2. **A latency measurement for AC1.** Still none, and still §9.1's point: the
   frame carries no publish timestamp, so nothing can evaluate "95% within three
   seconds". Adding one to the payload is cheap and would make the criterion
   checkable rather than aspirational.
