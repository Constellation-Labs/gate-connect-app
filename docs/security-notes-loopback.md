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

## Accepted: the environment forwarder

The machine-wide variables name `proxy::forwarder` rather than the engine, so
that the env channel fails open the way the PAC channel does (rationale in
`docs/routing-architecture.md`). It is a separate, detached process, so unlike
the listeners above it can still be accepting when no other part of Gate is
running.

What it is: a forward proxy that hands each connection to the engine when the
engine answers, and connects the client straight to its destination when it
does not.

- **It cannot spend the Gate credential**, because it never has one. It holds
  no key, no token and no org; it never terminates TLS, mints no certificate,
  and rewrites nothing to the gateway. The blast radius that the rest of this
  document weighs - the ability to *spend* - does not apply to it at all.
- **It does not read traffic.** Only the first request head is parsed, and only
  far enough to learn where the connection is going; after that the connection
  is spliced.
- **It does not forward the proxy credential when going direct.**
  `Proxy-Authorization` addresses this hop, so the direct path strips it rather
  than carrying it to a third party. It is passed through untouched when the
  connection goes to the engine, which is where it is meant to be read.
- **It is an open forward proxy to an arbitrary `host:port`**, which is the one
  capability it does have, and it has it for as long as it runs.

Decision: accepted. The marginal capability over the status quo is small - any
local process can already open its own outbound socket, so what this adds is
reaching a host *through* Gate's process rather than directly, which matters
only where an egress filter distinguishes the two. Weighed against the
alternative, which is that turning Gate off takes the machine's AI tooling
(and its curl and its npm) offline until every affected process is restarted.

The macOS/Windows cross-user gap named above applies here too and is smaller in
consequence: another local user reaching the forwarder gets egress, not the
owner's credential. The same UID-resolution work
(`net.inet.tcp.pcblist`, `GetExtendedTcpTable`) would close it here and for the
engine and relay at once, and is the thing to do first if this is prioritized.

## Noted: the `claude-web` catalog entry (session cookie)

The opt-in `claude-web` domain MITMs `claude.ai/organizations/*` and forwards
the user's live Claude **session cookie** to the gateway for audit. This is
deliberate, CLI-only, `enabled: false` by default, detached from the provider
cascade, and flagged in the catalog entry as pending validation. It stays an
explicit product decision: enabling it moves a credential strictly more
powerful than an API key off-box. Any move to surface it in the UI must
re-open that decision.

## Noted: the website-shaped user-agent on rewritten chatgpt.com turns

A chatgpt.com app turn routed to the gateway is presented the way the
*website* presents it: the app shell's product token (`CodexBrowser/…`) is
stripped from the `user-agent`, and no captured `cf_clearance` is injected.
Both halves together, or neither - see
`engine::website_shaped_rewritten_turns` for the captures behind it.

Written down here because it is a deliberate decision with a cost, not an
implementation detail. Gate ships a request naming a different client than
the one that sent it to a third party's bot management, which erases the
signal that vendor uses to tell its own clients apart. It buys the chat turn:
with the app's token on it the same request is challenged and the turn fails
outright, cookie or no cookie.

The scope grew when it landed: it applies to every rewritten turn, where it
first applied only to a rewritten turn with no clearance in hand. Its only
control today is `GATE_CF_APP_SHAPED_TURNS`, an environment variable that is
read once per process and turns the behaviour OFF when set. That is the shape
of a measurement switch, not of a user-facing one - a user cannot see the
reshape or turn it off from the app. Promoting it to a real setting (and
saying so in the UI) is an open product decision, and is the thing to revisit
first if this starts costing more than it buys.

## Related hardening landed with this review

- `GATE_CONNECT_TEST_*` seams (extra trust roots, secrets-dir redirect,
  control-plane endpoint overrides) are honored only in debug builds
  (`env::test_seam`); a release binary ignores them loudly.
- The relay and PAC accept loops pause briefly on accept errors instead of
  spinning a core at 100% on a permanently failed listener.
