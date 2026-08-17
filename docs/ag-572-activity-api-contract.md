# AG-572: activity overview API contract

Proposal for the gateway endpoint behind the Gate Connect 24-hour activity
overview. Written 2026-08-11 for stakeholder review. Not yet approved and not
yet implemented.

Companion to AG-572 (implementation) and AG-571 (design). Cross-repo: the
endpoint lands in `gate`, the consumer is `gate-connect-app`.

## 1. Why this document exists

- AG-572's acceptance criteria describe counters, an hourly chart, policy rows
  and token savings, but name no endpoint, schema, or auth scheme.
- AG-571 is a design task and does not produce an API contract, so the contract
  is unowned. This proposal fills that gap.
- The finding that motivates it: **most of the data already exists.** This is a
  composition endpoint, not a new metrics subsystem.

## 2. What already exists in `gate`

- `apps/gateway-proxy` is the service Gate Connect already talks to. It serves
  `GET /v1/me/orgs` from `src/cognito-auth/org-selection.controller.ts`, whose
  doc comment describes the connect app's login flow by name.
- `apps/dashboard-api` holds the aggregation logic today:
  - `RequestRepository.getStats({ orgId, since, until, bucketGranularity })` at
    `src/requests/request.repository.ts:605` already returns org-scoped totals
    plus an hourly `buckets[]` array.
  - Exposed at `GET /api/v1/gateway/requests/stats`
    (`src/gateway-proxy/gateway-proxy.controller.ts:163`).
  - Security aggregates at `GET /api/v1/gateway/security/metrics`.
  - Policy and savings config at `GET /api/v1/orgs/:orgId/security-policy`,
    `.../compression/config`, `.../caching/config`, `.../caching/stats`.
- `apps/dashboard-api/src/admin/admin.repository.ts:1735` is the reference
  implementation for zero-filled hourly series (`generate_series` left-joined to
  requests). Reuse this shape; a bare `GROUP BY` omits empty hours, which would
  silently break the "zero" state AG-572 requires.

## 3. Placement decision

Three options were considered. **Recommendation: option A.**

- **A. New endpoint on `gateway-proxy`, `GET /v1/me/activity`.** Recommended.
  - Gate Connect stores exactly one URL (the gateway base URL) and one auth
    header. No new configuration, no second host, no CORS surface.
  - Sits beside `/v1/me/orgs` and reuses the same `CognitoRequestAuthService`
    pattern already proven for the connect app.
  - `gateway-proxy` already has database access to `gateway_requests` and
    `security_decisions`; it writes both.
- **B. Point Gate Connect at `dashboard-api` directly.** Not recommended.
  - Reuses existing endpoints nearly as-is, but requires a second base URL in
    `account.json`, a second auth header shape, and a new opener/CSP surface.
  - Couples the shipped desktop app to an internal dashboard API that was never
    designed as a public contract.
- **C. `gateway-proxy` fans out to `dashboard-api` server-side.** Fallback.
  - Avoids duplicating SQL, at the cost of a service hop and a new internal
    trust relationship.

**Condition on option A:** lift the aggregation queries into a shared package
under `packages/` so `gateway-proxy` and `dashboard-api` compute identical
numbers. Two independent implementations of "blocked or redacted in the last
24h" is how the popover and the dashboard end up disagreeing, which defeats the
purpose of the screen.

## 4. Endpoint definition

- **Route:** `GET /v1/me/activity`
- **Service:** `apps/gateway-proxy`, registered before the `@All("*")` proxy
  catch-all (same constraint as `OrgSelectionModule`).
- **Auth: both credential types.** Revised 2026-08-11 per Gabriel Claramunt on
  AG-572: API-key accounts are in scope, and the key resolves to an org.
  - OAuth: `X-Gate-Authorization: Bearer <cognito access token>` plus
    `X-Gate-Org-Id`. The custom slot, not standard `Authorization`.
  - API key: `sk-gw-*` through the normal `ApiKeyGuard` path, which already
    resolves the org from the key. No org header needed or accepted.
  - This rules out the `@Public()` cognito-only shape `/v1/me/orgs` uses. The
    route needs a guard that admits either credential and yields one resolved
    org either way.
  - Consequence for the client: an `AuthMode::ApiKey` account holds no `org_id`
    locally, so the response's `org` block is how it learns which org it is
    looking at. That field is load-bearing, not decorative.
- **Org scoping:** required, but sourced from the credential.
  - OAuth: from `X-Gate-Org-Id`, with membership verified against the token and
    never trusted from the header.
  - API key: from the key itself.
- **Installation scoping:** required. See section 4a.
- **Query parameters:**
  - `window` (optional, default `24h`). Only `24h` supported at v1. Reserved so
    a 7d view does not need a new route.
  - No `orgId` query parameter. The header is the single scoping mechanism, so
    there is one path to audit.
- **Caching:** `Cache-Control: no-store`. The response carries a `generatedAt`
  that the client renders as "last refresh".

## 5. Response schema

```jsonc
{
  "generatedAt": "2026-08-11T14:03:00Z",   // drives "last refresh"
  "window": { "from": "2026-08-10T14:00:00Z", "to": "2026-08-11T14:00:00Z" },
  "org": { "orgId": "uuid", "name": "Acme Inc" },

  "counters": {
    "blockedOrRedacted": { "state": "ok", "value": 12 },
    "needsReview":       { "state": "ok", "value": 3 },
    "requestsRouted":    { "state": "ok", "value": 1840 },
    "tokensSaved":       { "state": "ok", "percent": 18.4, "costUsd": 2.71 }
  },

  "requestsByHour": {
    "state": "ok",
    "buckets": [                            // exactly 24, oldest first
      { "hour": "2026-08-10T14:00:00Z", "requests": 74,
        "securityActions": 2, "reviewItems": 0 }
    ]
  },

  "policies": {
    "state": "ok",
    "rows": [
      { "id": "prompt-injection", "label": "Prompt injection", "action": "Block",  "enabled": true },
      { "id": "pii-phi",          "label": "PII / PHI",        "action": "Flag",   "enabled": true },
      { "id": "credentials",      "label": "Credentials",      "action": "Redact", "enabled": false }
    ]
  },

  "tokenSavings": {
    "state": "ok",
    "rows": [
      { "id": "compression", "label": "Compression", "enabled": true },
      { "id": "caching",     "label": "Caching",     "enabled": false }
    ]
  },

  "notices": [
    { "id": "…", "tool": "Codex", "reason": "…", "action": "…", "severity": "warning" }
  ]
}
```

Shape rules:

- **Every section carries its own `state`**, one of `ok | unavailable`. A failed
  metric must not fail the response. AG-572 requires that one broken metric
  never hides the others, so partial failure is modelled in the payload, not in
  the HTTP status.
- The endpoint returns `200` whenever the caller is authorised, even if every
  section is `unavailable`. Non-200 is reserved for auth and scoping failures.
- `state: "ok"` with `value: 0` is the **zero** state. Zero is data, not
  absence, and the client must render it differently from `unavailable`.
- `buckets` is always exactly 24 entries when `state` is `ok`, zero-filled.
  Empty hours are `requests: 0`, never omitted.

## 6. Field derivation

Precise sources, verified against the schema on 2026-08-11.

- **`requestsRouted`**
  - `COUNT(*)` from `gateway_requests` where `org_id = $org` and `created_at`
    inside the window.
  - Served by the index `idx_requests_org_created` on `(org_id, created_at)`.
- **`requestsByHour[].requests`**
  - Same table, bucketed by `date_trunc('hour', created_at)`.
  - Zero-filled via `generate_series`, per `admin.repository.ts:1735`.
- **`blockedOrRedacted`**
  - `security_decisions` where `action IN ('block','redact')`.
  - **Constraint: `security_decisions` has no `org_id` column.** Scoping must
    join to `gateway_requests` on `request_id`. This is already documented in
    `admin.repository.ts` ("correlate through the decision's request").
  - Count distinct `request_id`, not decision rows, or a request with several
    decisions inflates the counter.
- **`requestsByHour[].securityActions`**
  - Same source, bucketed. Note `security_decisions.created_at` and
    `gateway_requests.created_at` can differ; **bucket by the request's
    timestamp** so the two series in one bar reconcile.
- **`needsReview`**
  - `security_event_reviews` where `org_id = $org` and `verdict IS NULL`.
  - This table does carry `org_id`, so no join is needed.
  - Open question: whether "needs review" is window-scoped or a standing
    backlog. See 9.2.
- **`tokensSaved`**
  - Requires confirmation. Candidate sources are the caching savings series
    (`GET /api/v1/orgs/:orgId/caching/stats/savings-series`) and the compression
    module. Neither was verified for this document.
  - `percent` and `costUsd` are both optional. AG-572 only specifies behaviour
    when both are present; see 9.3.
- **`policies[]`**
  - `GET /api/v1/orgs/:orgId/security-policy`, reduced to three fixed rows.
  - Labels and actions are fixed by AG-572 (Block / Flag / Redact). Only
    `enabled` is dynamic.
- **`tokenSavings[]`**
  - `.../compression/config` and `.../caching/config`, reduced to `enabled`.
- **`notices[]`**
  - **Not yet sourced.** See 9.1. Should be empty at v1 rather than fabricated.

## 6a. The installation dimension

Added 2026-08-11 after Gabriel Claramunt resolved open decisions 4, 6 and 7 on
AG-572. His answer: an installation is an id/label per Gate Connect instance, so
two machines signed in with the same OAuth user (or the same API key) can be told
apart, and **scoping is (user token + org, or API key) plus installation id**.

This is the largest piece of work in the ticket, and it is new end to end. His
own caveat is correct: it limits how much of the existing gate services can be
reused, because nothing in them carries an installation dimension.

**Current state, verified 2026-08-11:**

- `crates/core/src/primitives.rs:210` already defines `install_id()`, a stable
  UUID cached at `<app_support_dir>/install-id`. Its doc comment says it is the
  "stable UUID we send to the gateway audit trail for telemetry attribution."
- That comment is stale. **Nothing calls it.** The only other reference in the
  repo is a doc comment in `proxy/control.rs:84` distinguishing itself from it.
- It is `#[cfg(target_os = "macos")]`, so Windows and Linux have no install id
  at all.
- The gateway has **no installation concept**: no column on `gateway_requests`,
  no header, nothing in the entity definitions.

**What making this real requires:**

- **Connect app, identity:** lift `install_id()` out of the macOS cfg so all
  three platforms have one, and add a user-facing label. The AC says
  "installation", which implies something a human recognises, not a raw UUID.
  Decide whether the label is user-editable and where it is persisted.
- **Connect app, data plane:** inject an installation header on **every** request
  the app routes, in both mechanisms. The seam exists: the proxy engine and the
  relay already share header injection in `crates/core/src/proxy/mod.rs` around
  the `x-gate-*` constants. Without this, no request is attributable and the
  counters cannot be scoped at all.
- **Gateway, ingest:** accept the header and persist it on `gateway_requests`,
  which needs a new nullable column plus an index that actually serves the query.
  The existing `idx_requests_org_created` on `(org_id, created_at)` will not
  cover an installation filter; this likely wants
  `(org_id, installation_id, created_at)`.
- **Gateway, discovery:** something must list an org's installations, or the app
  cannot offer the "selected installation" the AC describes.
- **Migration reality:** every historical row has no installation. Decide what
  the overview shows for an org whose traffic predates the column, and whether
  unattributed traffic appears under a synthetic "unknown" installation or is
  excluded outright.

**Scope boundary worth stating plainly:** only traffic that passes through Gate
Connect can carry an installation id. An `sk-gw-*` key used directly from curl or
CI produces rows with no installation. So "requests routed" per installation
means "routed by this copy of the app", which is arguably the honest reading of
the screen, but it will not reconcile with an org-wide total in the dashboard.
That difference should be deliberate and stated in the UI copy rather than
discovered.

## 7. Privacy constraints

AG-572: the overview must not show prompt text, response text, credentials,
matched secrets, or raw security evidence.

- Enforce by **omitting these fields from the DTO entirely**, not by hiding them
  client-side. A field that never crosses the wire cannot leak through a log, a
  crash report, or a future UI change.
- `security_decisions.results` is `jsonb` and contains match evidence. It must
  never be selected into this response. Aggregate counts only.
- The same rule applies to client analytics: no counter breakdown that could
  re-identify a single request.

## 8. Performance and correctness notes

- The org/time index `idx_requests_org_created` covers the primary query. The
  security join uses `idx_security_request_id`.
- Three sections hit different stores. Compute them **concurrently and
  independently**, so one slow or failing source degrades to
  `state: "unavailable"` rather than timing out the whole response.
- Set a short server-side timeout per section. The client calls this from a
  menubar popover on open; a slow response is a blank screen.
- Boundary consistency: `getStats` uses `>= since` and `<= until` deliberately,
  after an incident where mixing `<` and `<=` made KPI cards disagree with row
  lists. Match that convention exactly.
- Bucket count is fixed at 24 regardless of retention. If retention is shorter
  than the window, the missing hours are `unavailable`, not zero. See 9.5.

## 9. Open decisions for stakeholders

Answered items are marked RESOLVED with the source. Updated 2026-08-11.

1. **Routing notices have no source.** OPEN. AG-572 wants notices naming tool,
   reason, and action. Gate Connect already computes per-tool attention state
   locally (`src/lib/groups.ts`: `error | drifted | master-off | needs-trust`).
   Decide: are these client-side facts the app already knows, or a new
   server-side concept? If client-side, they should not be in this contract at
   all.
2. **Is "needs review" window-scoped or a standing backlog?** OPEN. A backlog
   count under a "Last 24 hours" heading is misleading; a windowed count may read
   as zero while a real backlog exists.
3. **Token savings when only one value is available.** OPEN. AG-572 specifies the
   display only when percent and cost are both present. Decide the fallback.
4. **API-key accounts.** RESOLVED, Gabriel Claramunt on AG-572, 2026-08-11:
   "api key accounts should be supported, the api key resolves to an org."
   Endpoint accepts both credential types; see section 4. The response's `org`
   block becomes the only way a key-mode client learns its org.
5. **Retention.** OPEN. Whether `gateway_requests` reliably holds 24 hours for a
   low-traffic org, and what the screen shows before an org has any traffic.
   Now compounded by installation scoping: a per-machine slice of a low-traffic
   org is far sparser than the org total.
6. **Org-wide versus caller-scoped counters.** RESOLVED, same source: scoping is
   "(user token + org / api-key) + installation id". Neither org-wide nor
   per-user; per installation. This dissolves the original permissions concern
   but raises 9.8.
7. **"Installation" is undefined.** RESOLVED in principle, same source: "an
   installation id/label for each gate connect instance, so you can differentiate
   two different machines using gate connect with oauth (or the same api key)."
   Everything this now requires is in section 6a. Note Gabriel's own caveat:
   "This might limit how much we can reuse from the existing gate services."
8. **NEW: can a user select another machine's installation?** The AC says
   "selected installation", which implies a picker over more than one. If the
   picker lists every installation in the org, a member can read a colleague's
   per-machine activity, which is the permissions question from 9.6 returning in
   a new form. If it lists only this machine, "selected" is the wrong word and
   there is no picker to build.
9. **NEW: what is an installation's label, and who sets it?** A raw UUID is not
   something a person recognises in a 380px popover. Machine hostname, a
   user-chosen name, or something derived at first run are all plausible, and
   they differ in privacy: a hostname is often a person's real name.
10. **NEW: what happens to unattributed traffic?** Rows written before the
    column exists, and any `sk-gw-*` traffic that never passed through Gate
    Connect, have no installation. Excluded, or grouped under a synthetic
    "unknown"? This decides whether the popover's totals can ever reconcile with
    the dashboard's.

## 10. Suggested phasing

Revised 2026-08-11. The installation answer moves work from "blocked" into
"required", and it is now the critical path rather than a footnote.

- **Phase 0, new prerequisite:** the installation dimension end to end. A
  cross-platform `install_id`, a header injected on every routed request, a
  gateway column and index, and a discovery endpoint. Nothing below can be
  correctly scoped until this exists. See section 6a.
- **Phase 1, the screen:** `requestsRouted`, `requestsByHour`,
  `blockedOrRedacted`, `policies`, `tokenSavings` toggles. Sourced from verified
  tables and existing config endpoints, but now filtered by installation, which
  is what reduces the reuse of `getStats`.
- **Phase 2, needs decisions:** `needsReview` (9.2), `tokensSaved` values (9.3),
  `notices` (9.1).

**Estimate change.** The pre-answer read was "one controller, one shared query
module, and a bucketing variant." That still holds for the read path, but phase 0
adds a schema migration, a data-plane change in the desktop app on all three
platforms, and a backfill decision. The gateway work roughly doubles and the
connect-app work grows by a piece that has nothing to do with the screen itself.

Phase 1 is a composition of queries that already exist. The realistic cost on
the gateway side is one controller, one shared query module, and the
security-action bucketing variant, not a new metrics pipeline.

## 11. Consumer notes for `gate-connect-app`

- The fetch must live in Rust, not the webview. The popover CSP
  (`src-tauri/tauri.conf.json`) does not allow gateway origins in `connect-src`,
  and widening it would be the wrong fix since the webview also holds no token.
- Model the new client on `crates/core/src/org.rs`: `reqwest::blocking` with
  `.no_proxy()` (mandatory, or the app's own machine-wide `HTTPS_PROXY` export
  captures the call and the injected `X-Gate-Api-Key` produces a spurious 401),
  the `x-gate-authorization` header, and a hermetic endpoint env seam mirroring
  `GATE_CONNECT_TEST_ORGS_ENDPOINT`.
- Take the token from `oauth::live_session()`, which refreshes, **not**
  `oauth::current()`, which is a raw keychain read. `org::list_current()`
  currently uses the latter and can fail on a refreshable token; that is a
  separate bug worth its own ticket.
- Reuse the `SessionProbe` discipline from `org.rs`: only a definite 401 is a
  rejection. A metrics failure must never sign the user out.

## 11a. Installation scoping: decisions

Settled 2026-08-17. These close open decisions 7-10 in section 9.

- **Reuse `gateway_org_machines`.** Do not invent a second installation entity.
  That table is already `(org_id, machine_id)` unique with `first_seen_at`,
  `last_seen_at` and a `trust_status` of `unverified` / `verified`, and it already
  carries a worked-out threat model from the audit-ingest path (first sight
  registers unverified; a scoped key forging a machine id is recorded as SEC-H8).
  What changes is that inference requests start writing to it too, not just
  `POST /v1/audit`.
- **Label is the raw UUID for now.** Hostname would be friendlier and is often a
  person's real name, so it is a privacy decision nobody needs to take yet. A
  display name can land later without moving the identifier.
- **A user sees only their own installation.** Cross-installation visibility,
  probably for Owner and Admin, is explicitly out of scope. Note this is the same
  shape as the `securityEvents` policy, which already answers "no" for everyone,
  so the conservative default costs nothing to keep.
- **Unattributed traffic groups as "unknown", and must not change any existing
  behaviour.** Every historical row has no installation, and so does any
  `sk-gw-*` key used outside Gate Connect. Two consequences follow and are
  requirements rather than preferences: an absent installation must never filter
  a request out of a total the user already sees, and the column must be nullable
  with no backfill, so nothing that works today starts failing.

**The risk to state before anyone writes it.** Stamping an installation id is a
change to the *data plane* of the desktop app: the header rides on every proxied
request, in both the relay and the MITM engine. It has to fail open. A missing,
malformed, or unwritable installation id must degrade to today's behaviour -
unattributed traffic - and never break the request carrying the user's actual
work.

## 12. Deferred follow-ups (not AG-572)

Recorded 2026-08-12 so they are not silently absorbed into this ticket.

**Per-tool tagging of requests.** Asked by joão carvalho on AG-572: "are we able
to also identify and tag which tool generated the message?" Feasibility differs
by mechanism:

- **Relay-routed config tools: feasible.** The relay URL is already
  `http://127.0.0.1:<port>/<slug><client-path>` where `<slug>` is the catalog
  *domain*, not the tool (`crates/core/src/proxy/relay.rs:18`). Adding a tool
  identifier means changing what each integration writes, which makes existing
  configs stale. The app already handles that: `Status::Drifted`,
  `config_is_managed()` and `provider::reconcile_enabled` auto-reapply configs
  Gate Connect itself wrote.
- **Engine-routed apps: not feasible as asked.** For Claude Desktop and ChatGPT
  the engine sees only a TLS CONNECT to a host. Attribution to a specific desktop
  app would need process inspection on the socket. There, "which tool" collapses
  to "which domain", and any per-tool UI must degrade for these.
- **Storage:** `gateway_requests` records no user-agent, client, or tool column,
  only `client_ip`. `request_bodies.request_headers` is stored as jsonb, so the
  tool's own User-Agent may already be captured where header capture is enabled.
  Worth confirming as a heuristic for historical traffic, but too fragile to be
  the design: UA strings drift, and Claude Code and Claude Desktop both talk to
  `api.anthropic.com`.
- **Sequencing note:** tool id and installation id are both new dimensions on the
  same table. If a migration happens for one, doing both together is much cheaper
  than sequentially.

**Latest messages per tool detail.** Same comment: "we need that detail to show
the user latest messages in each tool detail on gate connect." Technically
possible, since `request_bodies` stores `request_body`, `response_body` and
`response_raw`. Deliberately **out of scope for AG-572** and to be tracked
separately, because it is a change of product posture rather than a screen
detail:

- AG-572's own privacy AC forbids prompt and response text on the overview. Tool
  detail is a different screen, so this is not a literal contradiction, but
  `PRODUCT.md` states the gateway is what inspects traffic and that the popover
  does not evidence it yet.
- Open questions it would need answered: body retention per org; whether the
  stored body is pre- or post-redaction (`request_body_pre_compression` suggests
  several stages are kept); who may read whose messages once installation scoping
  exists; how conversation text renders at 380px and 200% text; and whether the
  DTO and CSP discipline in this document survives a payload carrying prose.
