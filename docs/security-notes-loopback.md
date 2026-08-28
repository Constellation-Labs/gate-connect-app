# Gate Connect security notes: the loopback trust boundary

Status: accepted posture, written down as part of addressing an internal
whole-app security review (findings H1/H2/M2 there). This is the reference
for what the loopback listeners do and do not defend against, so the next
review starts from the decision instead of rediscovering it.

## What the listeners are

While routing is on, Gate Connect serves plain HTTP on `127.0.0.1`:

- the MITM engine port (system-proxy traffic),
- the reverse-proxy relay port (CLI tool configs bake
  `http://127.0.0.1:<port>/<slug>` as their base URL),
- the PAC responder port (macOS/Windows `AutoConfigURL`).

The relay and engine inject the owner's live Gate credential (Cognito bearer
or `sk-gw-` key) into requests they rewrite to the gateway. The credential
itself never leaves the OS keychain at rest and is never written to a config
file; what these listeners expose is the ability to *spend* it.

## Defended: browsers (all platforms)

A web page can reach a loopback listener with no local foothold: a "simple"
cross-origin `fetch` to `http://127.0.0.1:<port>` is delivered without a
preflight (CORS only blocks the read), and DNS rebinding delivers the same
request under an attacker hostname. Both are refused since the whole-app
review:

- the relay rejects any request whose `Host` is not loopback and any request
  carrying a non-loopback `Origin` (`relay.rs::proxy`, helpers and rationale
  in `proxy/mod.rs::authority_is_loopback` / `origin_is_loopback`);
- the PAC responder rejects non-loopback `Host` the same way, so a rebound
  page cannot read the engine port out of the PAC body.

## Defended: other local users (Linux)

The engine and relay resolve the loopback peer's UID and fail closed unless
it matches the daemon owner (`owner_uid`, `engine::peer_uid_for`). A second
account on the machine cannot spend the owner's credential.

## Accepted: local processes on macOS and Windows

On macOS and Windows `owner_uid` is `None`: TCP loopback peers are not
UID-resolved there (no `SO_PEERCRED` equivalent for TCP; the platform routes
exist - `GetExtendedTcpTable`, pcblist sysctls - but are a per-platform
project). Consequences, stated plainly:

- **Same user, any process**: a malicious npm package or editor extension
  running as the owner can drive billed inference through the relay. A
  config-carried token would not change this - the same process can read the
  owner's 0600 tool configs (and the keychain entry gates on the app, not on
  the caller). This is the OS's same-user trust boundary; every local
  credential helper (ssh-agent, cloud CLI token caches) shares it.
- **Different local user on a shared macOS/Windows box**: can connect to the
  loopback port and spend the owner's credential. This is the real gap a
  per-run path token baked into tool configs would close (other users cannot
  read the owner's 0600 configs). Measured earlier: the major CLI tools
  preserve a base-URL path prefix, so `http://127.0.0.1:<port>/<token>/...`
  is a viable shape if this is ever prioritized.

Blast radius in both cases is spend, not theft: the raw key is not
disclosed, and the catalog-constrained upstream resolution means the relay
cannot be aimed at an arbitrary host.

Decision: ship with the browser and cross-user-Linux defenses; treat the
macOS/Windows cross-user token as a tracked follow-up rather than a blocker,
because multi-user desktop machines are rare in the target audience and the
same-user case is not fixable with a token at all.

## Noted: the `claude-web` catalog entry (session cookie)

The opt-in `claude-web` domain MITMs `claude.ai/organizations/*` and forwards
the user's live Claude **session cookie** to the gateway for audit. This is
deliberate, CLI-only, `enabled: false` by default, detached from the provider
cascade, and flagged in the catalog entry as pending validation. It stays an
explicit product decision: enabling it moves a credential strictly more
powerful than an API key off-box. Any move to surface it in the UI must
re-open that decision.

## Related hardening landed with this review

- `GATE_CONNECT_TEST_*` seams (extra trust roots, secrets-dir redirect,
  control-plane endpoint overrides) are honored only in debug builds
  (`env::test_seam`); a release binary ignores them loudly.
- The relay and PAC accept loops pause briefly on accept errors instead of
  spinning a core at 100% on a permanently failed listener.
