# Security review: `feat/oauth-authentication`

Base `main` (merge-base `581fb6d`). Lens: **security**. Scope: the OAuth/Cognito
sign-in, token refresh, keychain chunking, the new reverse-proxy relay, and org
header injection.

## Summary

The OAuth core is solid: authorization-code + PKCE (S256), a random 256-bit
`state` that is validated on the loopback callback, `.no_proxy()` on every
control-plane call, default TLS roots on the token/orgs/gateway hops, and no
client-side trust placed in the id token (the gateway verifies; the client only
reads `email` for display). No confirmed Critical or High issue. The findings
below are hardening gaps and consistency problems, the two most material being
(1) the new relay injects the signed-in user's Gate credential for **any** local
peer with no UID/peer gating, unlike the MITM path which gates on `owner_uid`,
and (2) the MITM `apply_rewrite` does not strip client-supplied Gate credential
headers the way the relay does, so a local process can smuggle
`x-gate-org-id` / `x-gate-authorization` past it.

Threat-model note: the desktop product is single-user (macOS/Windows menubar,
Linux per-UID), which bounds the loopback findings to same-host. The
`proxy serve` headless mode explicitly targets containers/servers/CI, where a
multi-tenant host raises the impact of the relay finding.

---

## Medium

### M1 — Relay injects the user's credential for any loopback peer (no UID gating)
`crates/core/src/proxy/relay.rs:229` (`accept_loop`), `:284` (`proxy`), `:349` (`inject_credential`)

The relay accepts every connection on `127.0.0.1:<relay_port>` and, for any
request whose `x-gate-upstream-url` is in the catalog, injects the live Cognito
access token (`x-gate-authorization` + `x-gate-org-id`) or the legacy
`x-gate-api-key` and forwards to the gateway. There is **no peer check**. Contrast
the MITM engine, which gates interception on `owner_uid` (`engine.rs` /
`helper.rs:250` sets `owner_uid: Some(geteuid())`) precisely so only the owning
user's traffic is credentialed.

- Risk: any local process/user that can reach the loopback port can spend the
  signed-in user's Gate/OAuth credential (and their paid inference quota) against
  catalog upstreams, acting as that user/org, without ever seeing the secret.
- Attack scenario: on a shared or multi-tenant host (the documented `proxy serve`
  deployment — containers/servers/CI, `relay.rs:127`), user B does
  `curl -H 'x-gate-upstream-url: https://api.anthropic.com' http://127.0.0.1:<port>/v1/messages`
  and rides user A's session.
- Mitigation: gate the relay on the connection's peer credentials
  (`SO_PEERCRED` on Linux, `LOCAL_PEERCRED`/`getsockopt` on macOS) against the
  owning UID, mirroring the MITM `owner_uid` check; or document that `serve` must
  only run on a single-tenant host and bind accordingly. At minimum this
  asymmetry with the MITM path should be a deliberate, documented decision.

### M2 — MITM `apply_rewrite` does not strip client-supplied Gate headers (relay does)
`crates/core/src/proxy/engine.rs:478-495` vs `crates/core/src/proxy/relay.rs:352-356`

The relay defensively removes `x-gate-authorization`, `x-gate-api-key`, and
`x-gate-org-id` from the incoming request before injecting the live values. The
MITM `apply_rewrite` uses only `HeaderValue::insert`, which replaces the header
it writes but leaves the others untouched:

- OAuth branch: overwrites `x-gate-authorization`; writes `x-gate-org-id` **only
  when `org_id` is `Some`** (a client-supplied `x-gate-org-id` survives when no
  org is selected); never removes a client-supplied `x-gate-api-key`.
- API-key branch: overwrites `x-gate-api-key`; never removes a client-supplied
  `x-gate-authorization` or `x-gate-org-id`.

- Risk: a local process routed through the MITM proxy can smuggle its own Gate
  headers to the gateway. The most concrete is **org spoofing**: in OAuth mode
  with no org selected, an attacker-set `x-gate-org-id` rides alongside the
  injected valid token, so requests hit an org of the caller's choosing (any org
  the user belongs to). A smuggled `x-gate-authorization` in API-key mode may also
  win at the gateway, since the relay's own docs state the OAuth header "takes
  precedence over the API key."
- Mitigation: `headers.remove()` all three Gate credential headers at the top of
  `apply_rewrite` before injecting, exactly as `inject_credential` does. This also
  makes the two proxy paths consistent.
- Confidence: header behavior confirmed in code; exploitability depends on
  gateway-side precedence for duplicate/multiple Gate headers — **worth
  confirming with the gateway team.**

---

## Low

### L1 — Token-endpoint error body is propagated/logged verbatim
`crates/core/src/oauth.rs` `post_token` (`bail!("Cognito token endpoint returned {status}: {body}")`), surfaced at `src-tauri/src/lib.rs:7884` (`eprintln!` of the refresh error)

A non-2xx token/refresh response is bailed with the full response body, which
then reaches stderr on the desktop startup-refresh path. Cognito error bodies do
not echo the access/refresh token, so leakage risk is low, but this is the one
place raw token-endpoint output crosses into logs. Consider truncating or
logging status only. Confirmed low.

### L2 — Loopback redirect uses `localhost`, and ports are fixed and predictable
`crates/core/src/oauth.rs` `REDIRECT_PORTS = &[8977,8978,8979]`, `LoopbackListener::bind`

RFC 8252 prefers the loopback IP literal (`127.0.0.1`) over `localhost` to avoid
DNS/hosts-file interposition; the code notes Cognito only accepts `localhost`
over http, so this is a constrained choice. Because the ports are fixed, a
malicious local process could pre-bind them and receive the redirect
(`code`+`state`). This is **not** a token-compromise path — the PKCE `verifier`
never leaves gate-connect's process, so a stolen code cannot be redeemed — but it
is a login DoS. Accept or document; PKCE is doing the real work here.

### L3 — No `nonce` on the authorize request
`crates/core/src/oauth.rs` `begin_login`

No OIDC `nonce` is sent. Acceptable here because the id token is used only for
non-authoritative email display and is never trusted for auth (the gateway
verifies the access token). Noted for completeness; no action needed unless the
id token later gains a trust role.

### L4 — `state` compared with non-constant-time `!=`
`crates/core/src/oauth.rs` `handle_callback` (`if state != expected_state`)

Plain string comparison of the CSRF `state`. No practical timing oracle exists
(single-shot, attacker gets no repeated measurements), so this is informational.

---

## Informational / confirmed-safe (checked, no change needed)

- **Baked Cognito client config** (`crates/core/build.rs`, `oauth.rs`
  `from_build_env`): only `hosted_domain`, `client_id`, and `scopes` are baked —
  all public client metadata for a native PKCE client, **no client secret**. This
  is the correct pattern for a public client; the runtime-override comment is
  accurate. Expected, safe.
- **Keychain chunking** (`crates/core/src/keychain.rs`): split/reassembly is
  char-boundary safe, the `\u{0}gck-chunks\u{0}` manifest marker cannot collide
  with JSON/PEM/`sk-gw-` values, `set` clears prior state before writing and
  writes the manifest last (torn write reads as absent, not as a dangling
  manifest). Secrets stay in the OS store; no plaintext-to-disk path introduced.
  Solid.
- **Control-plane bypass**: token exchange (`oauth.rs post_token`), org list
  (`org.rs list`), and the relay gateway hop (`relay.rs RelayState::new`) all use
  `.no_proxy()`, matching the documented requirement that Gate's own reqwest
  clients bypass the system proxy. Correct.
- **Relay upstream allow-list** (`relay.rs proxy`): the relay refuses any
  `x-gate-upstream-url` not in the built-in catalog and only ever forwards to the
  configured gateway, so it is not a general open proxy. Good.
- **TLS**: default roots on all hops; the extra-CA path is gated behind
  `GATE_CONNECT_TEST_CA` and `tls_certs_only` is used only in that test seam.
  No verification is disabled in real builds.
- **Disconnect hygiene** (`account.rs clear` / `reconcile`): a full disconnect and
  the URL-orphan reconcile both call `oauth::clear()`, so tokens are not stranded
  in the secret store. Good.
- **id token handling** (`oauth.rs OAuthTokens::email`): decoded without
  signature verification but used only for display, explicitly documented, gateway
  is the verifier. Acceptable.
- **Helper IPC** (`control.rs` / `helper.rs` / `helper_client.rs`): the access
  token and org now cross the Linux daemon socket alongside the already-present
  `api_key`; same trust boundary as before (owner-only Unix socket + token
  handshake), no new exposure. Errors returned over IPC do not embed the token.