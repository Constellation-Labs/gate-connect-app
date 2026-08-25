# Gate Connect - Security Review (whole app)

Defensive review of the team's own product before wider release. Read-only:
no source was modified. Focus lenses, in the order the brief set them: MITM
machinery, credential handling, Tauri attack surface, config-file writers,
OAuth/Cognito, local port trust.

## What I inspected

- Proxy core: `crates/core/src/proxy/` - `engine.rs`, `relay.rs`,
  `cert_authority.rs`, `ca.rs`, `ca_linux.rs`, `ca_windows.rs`,
  `ca_bundle.rs`, `mod.rs`, `control.rs`, `helper.rs`, `helper_client.rs`
  (skim), `system_proxy.rs`, `system_proxy_linux.rs`, `proxy_env.rs`,
  `manager.rs` / `manager_windows.rs` / `manager_linux.rs` (owner_uid only).
- Credentials: `crates/core/src/keychain.rs`, `account.rs`, `oauth.rs`,
  `audit.rs`, `org.rs`, `primitives.rs`, `diagnostics.rs`.
- Tauri shell: `src-tauri/src/lib.rs` (command surface + report_backend_error),
  `src-tauri/tauri.conf.json`, `src-tauri/capabilities/*.json`.
- Config writers: `crates/core/src/integrations/claude_code.rs` in full;
  grepped file modes and header handling across all integration writers.
- Frontend seams: `src/lib/analytics.ts`, `src/lib/openExternal.ts`,
  `src/lib/config.ts` (partial).

## What I did NOT fully inspect

- `manager.rs` / `manager_windows.rs` / `manager_linux.rs` beyond the
  owner_uid wiring and the enable/disable ordering seen from `lib.rs`.
- `system_proxy_windows.rs`, `ca_windows.rs` timeout logic, `autostart_optout.rs`,
  `port_persist.rs`, `flock.rs`, `intent.rs` beyond skimming.
- The full bodies of `codex.rs`, `opencode.rs`, `openclaw.rs`, `hermes.rs`,
  `dotenv.rs`, `env_proxy.rs` integration writers (I checked their file modes,
  header stripping, and the shared `write_file` symlink guard, not every branch).
- The React screens' rendering paths for XSS beyond confirming no
  `dangerouslySetInnerHTML` / `innerHTML` exists in `src/`.
- Runtime behaviour: this is a static read, no build or dynamic testing.

Overall the codebase is unusually security-aware: the CA key never leaves the
OS keychain, leaf minting is correct, the relay derives upstreams from a fixed
catalog so a caller cannot aim it at an arbitrary host, config files are written
0600 with a symlink-escape guard, and the diagnostics report has a structural
no-credential test. The findings below are mostly about the loopback trust
boundary on macOS/Windows.

---

## Critical

None found.

---

## High

### H1. Loopback relay + MITM engine are an unauthenticated credential deputy on macOS/Windows

Files:
- `crates/core/src/proxy/manager.rs:251` (macOS) and
  `crates/core/src/proxy/manager_windows.rs:256` pass `owner_uid: None`.
- `crates/core/src/proxy/relay.rs:249-254` - `peer_allowed` returns `true`
  whenever `owner_uid` is `None`.
- `crates/core/src/proxy/engine.rs:331-344` - the MITM handler's `peer_allowed`
  does the same.
- The relay injects the live Gate credential per request at
  `relay.rs:461-467` / `crates/core/src/proxy/mod.rs:418-451`.

The loopback listeners are plain TCP on `127.0.0.1` reachable by every local
process and (on a multi-user box) every local user. On Linux the engine and
relay gate the peer by UID via `SO_PEERCRED`-equivalent `/proc/net/tcp`
resolution (`engine.rs:354-387`, `helper.rs:319` sets `owner_uid: Some(geteuid)`),
failing closed. On macOS and Windows `owner_uid` is `None`, so any local process
that connects to `http://127.0.0.1:<relay_port>/anthropic/v1/messages` gets the
owner's Gate credential (Cognito bearer or `sk-gw-` key) injected and the request
billed to the owner's Gate account/quota.

This is the confused-deputy that partly undercuts the product's core promise. The
credential is safe *at rest* (keychain, never a config file), but any local
process can *spend* it with no prompt and without ever reading it. The code
comment at `engine.rs:82-88` acknowledges this is deliberately out of scope for
macOS/Windows this release ("out of scope for this release"). Flagging it because
"wider release" is the trigger for this review and the exposure is real.

Exploitability: a malicious npm package, VS Code extension, or any other code
running as the same user on macOS/Windows can drain the owner's Gate inference
quota / rack up spend. It cannot exfiltrate the raw key, and it cannot aim the
gateway at an arbitrary upstream (catalog-constrained, see `relay.rs:613-653`),
so the blast radius is "spend the owner's inference budget," not "steal the key."
On Linux this is limited to the same UID (normal trust boundary). Recommend at
minimum documenting this in the product security notes, and ideally a
per-run loopback bearer token the integrations write into tool configs (the
configs are already 0600) so an arbitrary local process can't hit the relay
blind.

### H2. Relay accepts cross-origin browser requests (DNS-rebinding / drive-by spend)

Files: `crates/core/src/proxy/relay.rs:428-534` - `proxy()` reads no `Origin`,
`Host`, or `Referer` header and applies no same-origin / Host-allowlist check
before injecting the credential and forwarding.

Because the relay is a plain-HTTP loopback responder with no Host/Origin check,
a web page the user visits can reach it two ways: a DNS-rebinding attack that
rebinds an attacker hostname to `127.0.0.1:<relay_port>`, or a plain
cross-origin `fetch` to `http://127.0.0.1:<relay_port>/...`. CORS stops the page
from *reading* the streamed response, but a "simple" request (e.g.
`Content-Type: text/plain`) is delivered and processed without preflight, so the
gateway still runs the inference and bills the owner. The relay port is
guessable/discoverable: it sits in the fixed `47100..47200` band
(`engine.rs:650`) and the PAC endpoint (macOS/Windows) discloses the engine port
to any local fetch (`engine.rs:798-828`, no Host check either).

Exploitability: a remote website open in the user's browser can cause
attacker-chosen inference calls billed to the owner, on macOS/Windows without
even a local-process foothold. On Linux the browser runs as the owner UID so the
UID gate does not stop it. Mitigation: reject relay requests whose `Host` header
is not `127.0.0.1:<port>` / `localhost:<port>` and whose `Origin` is present and
not same-origin; this is the standard local-daemon DNS-rebinding defense and
composes with the H1 token suggestion.

---

## Medium

### M1. `GATE_CONNECT_TEST_CA` disables the point of the upstream trust check if ever set in a shipped env

Files: `crates/core/src/proxy/engine.rs:243-272` and
`crates/core/src/proxy/relay.rs:89-93,222-224`.

Both the engine's upstream connector and the relay's gateway client add an extra
root (engine) or switch to a webpki store holding *only* that CA (relay,
`tls_certs_only`) when `GATE_CONNECT_TEST_CA` points at a PEM. This is a genuine
test seam and is documented as never set in a shipped build. The engine variant
is safe-ish (adds to webpki roots). The relay variant is more dangerous: it
drops the built-in roots entirely and trusts only the test CA. If this env var
were ever inherited into a production process (a build/packaging mistake, or an
attacker who can set the app's environment), gateway TLS validation would be
replaced wholesale. Exploitability: requires the attacker to already control the
process environment, so this is defense-in-depth, not a live hole. Recommend a
compile-time `#[cfg(debug_assertions)]` / feature gate on these seams so a
release binary cannot honor the variable at all. Same reasoning applies to the
other `GATE_CONNECT_TEST_*` seams (`GATE_CONNECT_TEST_SECRETS` in
`keychain.rs:45`, `GATE_CONNECT_TEST_UPSTREAM` in `relay.rs:102`, the token/orgs
endpoints in `oauth.rs`/`org.rs`) which redirect secrets storage and control-plane
calls to attacker-chosen locations if set.

### M2. `claude-web` MITM intercepts the user's Claude session cookie

Files: `crates/core/src/proxy/mod.rs:690-750` (the `claude-web` catalog entry),
`engine.rs:565-603` (`apply_rewrite` leaves the app's own headers, incl. cookies,
intact and forwards to the gateway).

When the opt-in `claude-web` domain is enabled, `claude.ai/organizations/*`
traffic is MITM'd and rewritten to the gateway, carrying the user's Claude
*session cookie* to Gate for inspection/audit. This is intentional and gated
behind an explicit CLI-only opt-in (`enabled: false`, not attached to the
provider cascade - see the comment at `mod.rs:736-748`), and the CA's name
constraints do include `claude.ai` (`cert_authority.rs:136-137` calls this out
as widening the mintable set to the cookie-bearing host). The risk is that a
highly sensitive credential (a live web session, not just an API key) now flows
to and is logged by the gateway. This is a product/data-governance decision more
than a code bug; noting it so the "credentials are the product" promise is
weighed against a surface that ships a session cookie off-box when toggled on.
The comment already flags it as pending validation.

### M3. Analytics / PostHog allowed in CSP connect-src; distinct-id in diagnostics

Files: `src-tauri/tauri.conf.json:30` (CSP `connect-src` allows
`https://us.i.posthog.com`), `src/lib/analytics.ts`.

The analytics layer is well-built for a credentials product: manual events only,
an allowlist (`analytics.ts:56-75`) that drops any prop key not on it, no
`identify`, classified error titles instead of raw Tauri strings
(`analytics.ts:167-172`). This is a strong posture. The residual note is that the
CSP opens outbound to PostHog and the anonymous device id is surfaced into the
diagnostics report (`analytics.ts:145-154`). No credential or host rides along
given the allowlist, so this is Low-leaning-Medium; the main ask is to keep the
allowlist as the tripwire it is (a new event prop that carries a gateway host or
key would silently be dropped today, which is the desired behavior - keep it).

---

## Low

### L1. `report_backend_error` ships full anyhow context chains to the frontend buffer

Files: `src-tauri/src/lib.rs:998-1017` (buffer + drain),
call sites at `lib.rs:239,626,637,656` etc., all passing `format!("{e:#}")`.

`report_backend_error` buffers the *full* anyhow chain (`{e:#}`) and hands it to
the webview via `drain_backend_errors`. Anyhow contexts in this codebase include
file paths and gateway URLs (e.g. `account.rs`, `oauth.rs` `with_context` calls),
so the raw string reaching the renderer can carry a host or a path. The frontend
analytics layer re-classifies before sending upstream (`analytics.ts:167`, only
the classified title leaves the machine), so this does not exfiltrate by itself -
it stays in-process unless the renderer is compromised. Confirmed the diagnostics
report itself is scrubbed and has a structural guard against `sk-gw-` / `Bearer `
(`diagnostics.rs:250-258`). Low because the sink is local; worth keeping in mind
that `{e:#}` is the raw channel and a future context that interpolates a secret
would land here unredacted.

### L2. `id_token` email parsed without signature verification

Files: `crates/core/src/oauth.rs:276-282`.

`OAuthTokens::email()` base64-decodes the id token payload and reads `email`
without verifying the JWT signature. This is explicitly documented as display-only
(`oauth.rs:16-17,275`) and the gateway is the party that verifies the bearer, so
a forged id token only mislabels the UI, it grants no access. Correct call for a
public client; noting it only so a future change never starts trusting this claim
for an authz decision.

### L3. macOS install-id uses a hand-rolled UUID from /dev/urandom

Files: `crates/core/src/primitives.rs:271-304`.

The telemetry install-id is a v4 UUID generated inline from `/dev/urandom` and
cached at `<app_support_dir>/install-id` with default (non-0600) perms via
`fs::write`. It is a non-secret attribution id, so world-readability is not a
credential exposure; flagged only for consistency (most other state files here
are written 0600 through `primitives::write_file`).

---

## Notes on things checked and found sound

- **CA private key storage**: never written to disk. macOS login keychain
  (`ca.rs`), Linux Secret Service (`ca_linux.rs`), Windows Credential Manager
  (`ca_windows.rs`), all via `keychain::` with the `ai.constellation.gate-connect`
  prefix. Only the public cert lands on disk (`ca-cert.pem`), correctly treated
  as public.
- **Leaf minting**: correct - separate leaf key from the CA key
  (`cert_authority.rs:187-204`), `serverAuth` EKU, AKI, `CA:FALSE`, 1-year TTL
  with a renew margin. The CA carries X.509 name constraints limited to the
  catalog hosts (`cert_authority.rs:145-169`), so even a CA-key compromise cannot
  mint trusted certs for arbitrary domains. A host-set fingerprint sidecar forces
  regeneration when the catalog changes (`cert_authority.rs:59-119`).
- **Loopback binding**: all listeners bind `127.0.0.1` explicitly
  (`engine.rs:669-675,738-763`, `relay.rs:134-154`, oauth `oauth.rs:537-562`),
  never `0.0.0.0`. `bind_preferred`'s `SO_REUSEADDR` path is carefully guarded
  with a connect-probe so it can't shadow a live wildcard listener
  (`engine.rs:737-758`).
- **Upstream TLS validation**: the engine connector and relay client validate
  gateway/upstream TLS against webpki/platform roots; redirects disabled on the
  relay (`relay.rs:211-213`). No `danger_accept_invalid_certs` anywhere. The only
  override is the test CA seam (M1).
- **Credential injection scoping**: a caller-supplied `x-gate-api-key` is
  respected and nothing is injected over it; otherwise stray
  `x-gate-authorization`/`x-gate-org-id` are stripped before injecting, so a
  caller can't smuggle an org alongside the credential (`mod.rs:418-451`).
  Passthrough hops strip all Gate-internal headers (`relay.rs:688-693`).
- **Linux control channel**: Unix-domain socket in `$XDG_RUNTIME_DIR`, dir 0700 /
  socket 0600, `SO_PEERCRED` UID check, per-run 128-bit token, and
  `validate_domains` rejecting any non-catalog slug/host/upstream even for an
  authenticated caller (`control.rs`, `helper.rs:141-236`). Well layered.
- **Config writers**: all tool configs written 0600 (0644 only for the
  non-secret Linux env drop-in and the CA bundle). `primitives::write_file`
  is atomic (temp + fsync + rename) and refuses to follow a symlink that
  escapes its own directory - the iCloud/Dropbox exfiltration guard
  (`primitives.rs:20-116`), with tests. Claude Code writer no longer writes any
  credential or header into `settings.json`; the credential is injected by the
  relay at request time (`claude_code.rs:159-216`).
- **Privileged execution**: absolute binary paths (`/usr/bin/security`,
  `/bin/launchctl`, certutil with no shell), and every shell-interpolated arg is
  `sh_quote`d (`ca_linux.rs:243-264`, `primitives.rs:253-266`,
  `ca.rs:407-426`). Headless trust path uses `sudo -n` with stdin closed so it
  fails fast rather than hanging (`primitives.rs:205-251`).
- **Tauri surface**: CSP is tight (`default-src 'self'`, `object-src 'none'`,
  `frame-ancestors 'none'`, `script-src 'self'`, no `unsafe-inline` on scripts).
  `opener:allow-open-url` is allowlisted to GitHub releases / amazoncognito /
  constellationgate hosts (`capabilities/default.json:19-26`); frontend
  `openExternal` only ever passes fixed config URLs, never tool-derived strings.
  No exposed command takes a shell string; `save_upstream_api_key` validates the
  key shape and per-integration prefix before persisting (`lib.rs:29-43,177-187`),
  `open_onboarding_window` normalizes `source` before splicing into the webview
  URL (`lib.rs` comment). Updater endpoint is HTTPS GitHub releases with a
  minisign pubkey (`tauri.conf.json:50-57`), so update artifacts are signature-
  verified by the Tauri updater.
- **OAuth/Cognito**: authorization-code + PKCE (S256), high-entropy `state`
  validated on the callback with an explicit CSRF abort (`oauth.rs:648-654`),
  loopback redirect per RFC 8252, `prompt=login` to prevent silent account
  reuse. Tokens stored as one keychain blob, not on disk. Client-id stamped and
  checked so tokens surviving a pool switch are rejected (`oauth.rs:486-504`).
- **Diagnostics report**: deliberately excludes every secret and has a
  serialize-and-scan test asserting no `sk-gw-` / `Bearer ` shape can appear
  (`diagnostics.rs:250-258`).

## Top recommendations

1. Add a Host/Origin check to the relay (H2) - cheapest fix with the widest
   payoff, closes the browser drive-by vector on all platforms.
2. Gate `GATE_CONNECT_TEST_*` seams (especially the relay's `tls_certs_only`
   and the secrets-dir redirect) behind a compile-time flag so a release binary
   cannot honor them (M1).
3. Decide the multi-user / same-user spend posture for macOS/Windows (H1):
   either ship a per-run loopback token the integrations already-0600 configs
   carry, or document the exposure explicitly in the security notes.
