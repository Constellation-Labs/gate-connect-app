# Cursor routing spike (Phase 0)

Decision gate for Cursor support in Gate Connect. Fill this in on a real macOS
machine with Cursor installed. **Do not write the `ProxyDomain` / `Provider`
entries until this is green** — the whole feature is contingent on the results
below. See the plan for context (`~/.claude/plans/what-i-need-to-swirling-avalanche.md`).

Why a spike: Cursor can't send the `X-Gate-*` headers itself and has no writable
config, so full routing must go through the built-in MITM proxy (the Cowork
model). That only works if Cursor honors the system HTTPS proxy for *every*
feature, accepts our MITM CA (no cert-pinning), and its streaming survives the
proxy. None of that can be confirmed by reading code.

The Gate server side is already confirmed OK: its BYOK path forwards to any
public host (SSRF-blocklist only) and passes Cursor's own `Authorization` bearer
through intact — no Gate-server change needed.

## How to capture

Pick one:

- **A (recommended, isolates Cursor from the app):** run `mitmproxy` /
  `mitmweb`, set the macOS system HTTPS proxy at it, trust its CA, and watch the
  flow list while exercising Cursor.
- **B (uses the app's own engine):** temporarily add a wildcard-ish Cursor
  `ProxyDomain` with `supported: false, enabled: true` to `default_domains()` in
  `crates/core/src/proxy/mod.rs`, run the app, trust the CA, and read what the
  engine intercepts vs. tunnels. Revert before committing. Starter snippet:

  ```rust
  // TEMPORARY spike domain — delete after Phase 0.
  ProxyDomain {
      slug: "cursor-spike".into(),
      display_name: "Cursor (spike)".into(),
      hosts: vec!["api2.cursor.sh".into()], // add api3/api4/api.cursor.com as found
      upstream_url: "https://api2.cursor.sh".into(),
      rewrite_prefixes: vec!["/".into()],   // capture everything, then narrow
      passthrough_prefixes: vec![],
      enabled: true,
      supported: false,
  },
  ```

## Feature matrix — record per feature

For each Cursor feature, note: hosts contacted, whether the request showed up in
the proxy (system-proxy **honored**) or went direct (**bypassed** — the
Codex-agent failure mode), whether the MITM CA was accepted (no **pinning**), and
whether streaming completed.

| Feature | Host(s) | Proxy honored? | CA accepted? | Streaming OK? | Notes |
|---|---|---|---|---|---|
| Chat / plan panel | | | | | |
| Composer / Agent | | | | | |
| Inline edit / apply | | | | | |
| Tab completion | | | | | |
| Autocomplete | | | | | |
| Login / auth | | n/a | | n/a | must stay tunnelled |
| Auto-update | | n/a | | n/a | must stay tunnelled |

## Host & path taxonomy (feeds Phase 1)

- **Inference host(s)** (each becomes its own `ProxyDomain`, upstream = itself):
  - `______________________`
- **Inference path prefix(es)** → `rewrite_prefixes` (e.g. `/aiserver.`):
  - `______________________`
- **Must-not-touch paths on those hosts** → `passthrough_prefixes`
  (auth/login, telemetry) :
  - `______________________`
- **Hosts to leave fully tunnelled** (login/marketing/updates — do NOT list):
  - `______________________`

## Streaming / protocol

- Protocol observed (Connect / gRPC / plain HTTP+SSE): `__________`
- HTTP version required end-to-end (Gate forwards via HTTP/1.1 `fetch`): `______`
- Did streamed responses complete through the MITM? `______`

## Go / No-Go

- [ ] Every core feature's traffic is **capturable** through the proxy (none bypass).
- [ ] MITM CA accepted on all inference hosts (no pinning).
- [ ] Streaming survives the proxy.

**Decision:** ☐ GO → Phase 1/2 below  ☐ NO-GO → report; full routing not
achievable with this architecture (chat-panel-only was already rejected).

---

## Ready-to-drop implementation (only after GO)

### Phase 1 — `crates/core/src/proxy/mod.rs`, in `default_domains()`

One entry **per inference host** (each forwards to itself; do not collapse
distinct hosts onto one `upstream_url`). `supported: true` only once GO.

```rust
ProxyDomain {
    slug: "cursor-api2".into(),
    display_name: "Cursor".into(),
    hosts: vec!["api2.cursor.sh".into()],          // from taxonomy above
    upstream_url: "https://api2.cursor.sh".into(),
    rewrite_prefixes: vec![/* inference prefix(es) */],
    passthrough_prefixes: vec![/* auth/telemetry paths on this host */],
    enabled: false,
    supported: true,
},
// + cursor-api3 / cursor-api4 / cursor-apicom as the spike found
```

### Phase 2 — `crates/core/src/provider.rs`, in `providers()`

One provider grouping all Cursor host-domain slugs into a single UI toggle
(proxy-only, like `openrouter`):

```rust
Provider {
    slug: "cursor",
    display_name: "Cursor",
    subtitle: "Cursor editor",                     // final copy TBD
    tool_ids: &[],
    proxy_domain_slugs: &["cursor-api2" /*, "cursor-api3", ... */],
},
```

Add matching unit tests (mirror the existing `anthropic`/`openrouter` cases in
each file), then `cargo test -p gate-connect-core`. `ProxyScreen.tsx` renders the
provider automatically — no frontend changes.

> If the spike finds *many* inference hosts, consider (ask first) a small
> `decide()` change so a "self-upstream" domain derives `X-Gate-Upstream-Url`
> from the matched host instead of N near-duplicate domain entries.
