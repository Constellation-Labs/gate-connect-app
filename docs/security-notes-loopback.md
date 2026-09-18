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
  capability it does have, and it has it for as long as it runs. It applies no
  peer gate of its own.
- **It is the same signed executable as the app**, shipped as a sidecar, and it
  inherits the environment of whichever process spawned it.

**It fronts tool configurations as well as the variables now.** This section
first described the forwarder as the address of the machine-wide export alone.
`proxy::tool_proxy_url` also writes it into Claude Code's `settings.json`,
OpenClaw's `proxy.proxyUrl` and Hermes's `.env`, so those three explicitly
connected tools now depend on it too. The capability is unchanged - it is the
same listener, holding no credential - and the `Proxy-Authorization` handling
is what keeps it parity rather than escalation: the header is passed verbatim
to the engine and stripped on the direct path, so a local process sending
Claude Code's route selector through the forwarder gets exactly what it would
get by dialing the engine, which is the accepted cross-user gap above and not a
new one.

Decision: accepted. The marginal capability over the status quo is small - any
local process can already open its own outbound socket, so what this adds is
reaching a host *through* Gate's process rather than directly, which matters
only where an egress filter distinguishes the two. Weighed against the
alternative, which is that turning Gate off takes the machine's AI tooling
(and its curl and its npm) offline until every affected process is restarted.

**But do not read "it holds no credential" as containment for the cross-user
case.** While the engine is up, the forwarder hands the connection straight to
it, and the engine on macOS and Windows applies no UID gate either - so a
non-owner peer reaching the forwarder gets exactly what it would get by dialing
the engine directly, credential injection included. The forwarder neither adds
that exposure nor removes it; it is parity with an already-accepted gap, which
is why the decision stands. It does mean the ordering of any future UID work
matters: gating the engine and relay while leaving the forwarder ungated would
launder a non-owner peer into an owner-uid connection and undo the gate. The
forwarder has to be gated first, or at the same time.

Where the forwarder is socket-activated (macOS, via its LaunchAgent) the
squatting case disappears rather than being detected: launchd holds the port
from login, so no other process can be there to adopt. On Windows, and on macOS
when the agent could not be installed, the app instead proves the listener is
ours with a 0600 shared token on a reserved health path before exporting its
port - a plain TCP probe would have let any local process that bound the
remembered port be published as the machine's `HTTPS_PROXY`.

## Accepted: the parked state (macOS and Windows, routing off)

Turning routing off **parks** the engine rather than stopping it: the three
ports stay bound and `set_intercept(false)` drops both listeners to plain
forwarding. This is not a convenience. `launchctl unsetenv` cannot reach a
process that is already running, so releasing the ports strands every shell,
editor and CLI that already inherited `HTTPS_PROXY` - including tools the user
never switched on, and software Gate does not manage, because the export is
machine-wide. Linux has always parked (`helper::set_passthrough`).

The question this section answers is what a parked listener can be used for,
since it outlives the user's "off".

- **It cannot spend the credential.** This is the property the rest of this
  document is about, and the park does not weaken it. The MITM port claims no
  host at all - `engine::effective_rules` returns an empty set while parked -
  so nothing is decrypted, no leaf cert is minted, and `decide` never reaches
  a rewrite. The relay forces `Route::Passthrough` (`relay.rs`, on
  `state.intercept`), strips the Gate headers, and forwards under the tool's
  own credential.
- **The relay is still not an open proxy.** Catalog resolution runs *before*
  the intercept check, so a parked relay can only ever be aimed at a known
  upstream, exactly as while routing is on.
- **The MITM port remains a general forward proxy.** A CONNECT to an
  arbitrary `host:port` is tunnelled, as it is while routing is on. This is
  the one capability the park extends in time - from "while routing is on" to
  "for the rest of the app session".
- **The PAC responder is inert.** `pac_script` is built from the live rules,
  which are empty while parked, so it names the engine for no host.

Decision: accepted. The comparison that matters is parked versus *routing*,
not parked versus nothing bound - the alternative to parking is not a quiet
machine, it is a broken one. A parked engine is strictly less capable than the
engine it replaces: egress only, no spend. Egress is also what any local
process already has by opening its own socket, so the marginal capability is
reaching an arbitrary host *through* Gate's process rather than directly,
which matters only where an egress filter treats the two differently.

The macOS/Windows cross-user gap named above is therefore unchanged in kind
and smaller in consequence while parked: another local user reaching a parked
port gets a forward proxy, not the owner's credential. The same UID-resolution
work (`net.inet.tcp.pcblist`, `GetExtendedTcpTable`) would close it for both
states at once.

The park does not outlive the app: the ports are released on app exit, on a
gateway switch, on a re-enable, and on untrusting the CA. Exit is also the
residual - quitting still strands already-running tools, because on these two
platforms the listeners live in the GUI process.

The section below is the design this paragraph asked to have assessed before
it existed. The forwarder is a listener that outlives the GUI, it carries no
credential, and it is accepted - so the forward-proxy capability is already
permanent rather than session-scoped on any machine that exports the
variables, and a park that ended at app exit was never what bounded it.
What the park still decides on its own is the *engine* and *relay* ports,
which no forwarder currently fronts.

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
