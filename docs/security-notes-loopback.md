# Gate Connect security notes: the loopback trust boundary

Status: accepted posture, written down as part of addressing an internal
whole-app security review (findings H1/H2/M2 there). This is the reference
for what the loopback listeners do and do not defend against, so the next
review starts from the decision instead of rediscovering it.

## What the listeners are

While routing is on, Gate Connect serves plain HTTP on `127.0.0.1`:

- the MITM engine port (system-proxy traffic),
- the reverse-proxy relay port (CLI tool configs bake
  `http://127.0.0.1:<port>/__gate/t/<tool>/<slug>` as their base URL). On
  macOS and Windows the forwarder holds that port and the engine's relay binds
  a second one behind it; see "Accepted: the forwarder's relay listener",
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
  page cannot read the forwarder port (or, when the forwarder would not
  start, the engine port) out of the PAC body.

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
  is a viable shape if this is ever prioritized. **That measurement has since
  shipped as a mechanism**: the tool marker (`/__gate/t/<tool>/`) is a path
  prefix the relay writes into every tool config and peels back off, proving
  the shape end to end for Codex and OpenCode. A token would be the same
  mechanism carrying a secret instead of a name, which also means it inherits
  the same constraint - the value is only as private as the config file it is
  written into, so it closes the other-local-user gap and nothing else.

Blast radius in both cases is spend, not theft: the raw key is not
disclosed, and the catalog-constrained upstream resolution means the relay
cannot be aimed at an arbitrary host.

That bound depends on the `GATE_CONNECT_TEST_*` seams being debug-only, and
for a while five of them were not. `GATE_CONNECT_TEST_{ACTIVITY,TOOL_EVENTS,
INSTALLATIONS,CREDITS,GATE_MODELS}_ENDPOINT` read `std::env::var_os` directly
instead of `env::test_seam`, so a release build obeyed them - and each URL goes
to `gateway_api::call_json`, which attaches the live `x-gate-api-key` or bearer
with no scheme or host check. One line in a shell profile therefore sent the
raw `sk-gw-` key in cleartext to an arbitrary host on every later launch:
theft, persistent, and from a user-writable location rather than for the
lifetime of one process. Found by review, not in the field.

All five now go through the helper, and the rule is a test rather than a
sentence - `env::tests::every_seam_is_read_through_the_helper` scans this
crate's production sources for a bare read of a seam name, so a sixth cannot be
added the same way. If that test is ever relaxed, this paragraph is the reason
it exists.

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
- **It does not read traffic** on this listener. Only the first request head is
  parsed, and only far enough to learn where the connection is going; after
  that the connection is spliced. (The relay listener, in its own section
  below, does read requests.)
- **It does not forward the proxy credential when going direct.**
  `Proxy-Authorization` addresses this hop, so the direct path strips it rather
  than carrying it to a third party. It is passed through untouched when the
  connection goes to the engine, which is where it is meant to be read.
- **It is an open forward proxy to an arbitrary `host:port`**, which is the one
  capability it does have, and it has it for as long as it runs. It applies no
  peer gate of its own.
- **It is the same signed executable as the app**, shipped as a sidecar, and it
  inherits the environment of whichever process spawned it.

**It fronts the PAC as well, and so it exists whenever routing is on.**
`engine::pac_script` names the forwarder's port for every Gate host, with a
fallback after it (`; DIRECT`, or the user's prior proxy), so a browser holding
a cached PAC keeps working when the engine is gone instead of failing closed on
exactly the hosts Gate intercepts. Same listener, same capability: browser
traffic reaches the engine through one more loopback hop while it is up, and
goes direct when it is not. Two consequences are new and recorded here:

- The forwarder is started by every enable, independently of the machine-wide
  export choice, and on macOS that installs the socket-activated LaunchAgent,
  which holds the port from every login onward. Declining the export no longer
  means "no Gate process besides the app"; it means only that `HTTPS_PROXY` is
  not set. Signing out and untrusting the CA remain the paths that retire it.
  The quit that disconnects the tools *drains* it instead (`forwarder::drain`):
  tool configs go back to their own settings, and the forwarder keeps serving
  whatever already holds its address, direct, until the login session ends. On
  macOS the agent's plist is deleted and the loaded job left alone, so launchd
  keeps the sockets until logout and loads nothing at the next login; on
  Windows nothing but Gate starts it. So after that choice the forward-proxy
  capability lasts until logout rather than ending at the click - the price of
  not failing every running tool closed.
- The fallback makes a forwarder that stops answering a *silent* fail-open for
  the browser channel, where before the PAC named the engine and failed
  closed. Any same-user process can kill it; any local user can saturate it
  (no peer gate, a 512-connection cap that accepts and drops at capacity), and
  a browser that has once seen the port refuse keeps it on its bad-proxy list
  for minutes after it is back. The manager therefore re-checks it every 30s
  while it hosts an engine (`DesktopManager::forwarder_tick`), restarts one
  that has died, re-exports the variables if it came back on another port, and
  reports the outcome as `ProxyState.forwarder_answering`, which is what lets
  Home's routing line say that routing is on and nothing is being routed. Accepted on the same ground
  as the rest of this section: the exposure is unrouted traffic, not spend.

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

## The relay proves itself too

The forwarder has always answered a challenge before its port is published. The
relay did not: `relay_listening` was a bare TCP connect, so anything that
accepted on the persisted relay port read as a healthy Gate relay, and the tool
statuses built on it reported Connected over a stranger. It answers the same
proof now, on its own reserved path, minted from the same 0600 token.

This identifies without authorising. The endpoint reaches no credential,
resolves no route and returns no traffic, and it sits above the relay's own
loopback guards so a prober can tell a squatted port from a refused one.

It reports one thing about itself as well: whether it is intercepting. That is
not secret - whether Gate is routing is what the app's own window says - and it
replaced the relay tools reading the user's stored routing intent, which is a
preference standing in for a measurement and got the headless `proxy relay`
host backwards, since that host always intercepts and writes no intent file.

The challenge authenticates the *listener*, not the caller, so any process that
can reach the port can read that flag. Nothing credential-bearing may be added
on those terms.

The engine's own MITM port still has no proof and does not need one on this
path: `engine_proxy_url` gates on the system-proxy snapshot, which is Gate's
own file, so a stranger holding that port cannot make a tool read Connected.

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
  which are empty while parked, so it names the forwarder for no host.

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
What the park still decides on its own is the *engine* port. The relay port is
fronted by the forwarder now, in the section below.

## Accepted: the forwarder's relay listener (macOS and Windows)

The forwarder holds the public relay port as well, so a relay tool config
keeps working after the app is gone (rationale in
`docs/routing-architecture.md`). The engine's relay binds
`proxy/relay-engine-port` behind it. Per connection the forwarder splices to
the engine's relay once it proves itself, serves the request itself when
nothing accepts on that port, and refuses with a 502 when something accepts and
does not prove itself.

This is the first listener in the forwarder that reads a request and opens
TLS, so the section above's "it does not read traffic" is no longer true of the
process as a whole. What bounds it:

- **It still cannot spend the Gate credential.** The process holds none, and
  the direct path strips every `x-gate-*` header rather than a list of known
  ones. The only credential on that path is the one the tool sent, going to
  the provider the tool was configured for.
- **It is not an open proxy.** A request must name a slug in
  `gate_connect_paths::RELAY_UPSTREAMS` (or, for a config written before path
  encoding, carry the legacy `x-gate-upstream-url` naming one of those URLs
  exactly). Anything else is refused. The table is a compile-time copy of the
  catalog, held equal to it by a test in core, not a file a local process could
  edit.
- **It refuses browsers the same way the relay does.** `Host` must name
  loopback and any `Origin` must be loopback, from the same function the relay
  and the PAC responder use, now defined in `gate-connect-paths`. Dot segments
  are refused, for the reason the relay refuses them.
- **It does not hand a request to a stranger.** A process that binds the
  engine's relay port while the app is closed would otherwise be spliced every
  tool request, and relay requests are plaintext carrying the tool's own
  provider key. So the forwarder asks the engine side for the token proof
  before every splice, with no cache, on the connection the request then
  travels over - there is no gap between proof and splice in which the port
  could change hands - and refuses the request when it is not answered.
  **The proof binds the path it is asked on** (HMAC-SHA256 keyed by the token,
  over the path and the challenge), and the forwarder asks on
  `RELAY_ENGINE_HEALTH_PATH`, which only an engine's relay answers. That matters
  because the forwarder answers the forwarder and relay health paths for
  anybody: with one proof for every path, a squatter could relay the
  forwarder's challenge to one of those answers and replay it. The same
  engine-only path decides the "another Gate Connect" refusal, so a squatter
  cannot fake that either. A process running as the owner can read the token
  and forge any proof; this defends against other local users and sandboxed
  processes that cannot read Gate's data directory.
  A connect to the engine port that has not completed in 250 ms is read per
  platform: on macOS it is a listener that has not accepted (a busy engine
  with a full backlog) and gets the 502, not the direct path, so a routing Gate
  is not bypassed for being slow; on Windows, which does not refuse a closed
  loopback port at once, it is the engine being gone. A full backlog is
  refused outright on Windows and so also reads as gone there: that is the one
  way a live engine's traffic can go around it, and it is accepted.
- **Probes are bounded as a whole.** Every proof probe has a 750 ms budget for
  the whole exchange, not per read, so a listener trickling bytes cannot hold
  an enable, a quit, or a relay connection. The forwarder's own proof of the
  engine allows 5 s and then refuses.
- **Pay-as-you-go requests are refused, not sent bare.** If `account.json`
  says `billing_mode: payg`, the direct path answers the slugs Gate would bill
  that way with a 503 saying to open Gate Connect: such tools send no
  credential of their own, and a bare request to the provider would only earn
  a 401. Nothing writes that mode in this tree yet.
- **Its framing is strict.** At most one `Content-Length` and one
  `Transfer-Encoding`, lengths of digits only, `chunked` exactly once and last
  in the coding list and never on an HTTP/1.0 request, chunk sizes of hex
  digits only with CRLF line endings; the chunked framing is re-emitted
  canonically and trailers are dropped. `Connection` cannot nominate
  `Content-Length` or `Transfer-Encoding` away in either direction, since the
  body is framed by them before any header is dropped. Interim (1xx) responses
  lose the provider's connection headers as a final one does, and are not sent
  to an HTTP/1.0 client at all. Anything looser is refused rather than passed on for
  the provider to read differently. A present but unreadable `Host` or `Origin`
  is refused, as the engine's relay refuses it.
- **It bounds the slow phases.** The request head has 30 s, the TCP connect
  to the provider 30 s and its TLS handshake another 30 s, the request body
  5 minutes, the provider's first byte 10 minutes (a non-streaming completion
  answers only when it is done). A streaming response body and a spliced
  connection are closed after 10 minutes with no bytes in either direction,
  so neither a stalled provider nor a client that stopped reading holds a
  connection for good.
- **It caps connections at 512**, as the forward proxy does. Past that, up to
  64 more at a time are answered with a 503 saying the relay is busy, rather
  than reset; any beyond those are dropped. Any local process can still fill
  the cap and deny the relay to every tool while it holds it, which is the
  local-DoS class accepted everywhere else in this file.
- **It originates TLS and terminates none.** It verifies providers with
  `rustls-platform-verifier`, the verifier reqwest uses for the relay's own
  direct hop, so it trusts what that hop trusts and nothing extra.
- **One request per connection on the direct path**, `Connection: close` both
  ways, so a client pool keyed on the loopback origin cannot carry a request
  for one provider onto a TLS session opened to another, and a request body is
  relayed to its declared length and no further.

What it adds over the state before it: the relay port answers for as long as
the forwarder runs, including with no Gate process up, and on macOS from login
through the launch agent. While the app is closed, any local process - the
listener has no peer gate, so not only the owner's - can use it to reach the
catalog's providers through Gate's process. That is the
capability the parked relay already had for the length of an app session,
extended in time, with the same catalog constraint - and any local process can
reach those providers with its own socket anyway. The cross-user gap above is
unchanged: while the engine is up the forwarder splices a non-owner peer
straight to the engine's relay, which applies no UID gate on these platforms,
so the ordering note in the previous section applies to this listener too.

Windows has no socket activation, so nothing holds the public relay port from
login until the app first starts the forwarder. In that window a process that
binds the port receives relay tool requests, the tool's own key included,
exactly as it could before this change whenever the app was closed; after
that, the forwarder holds it for the rest of the session. Two other transition
windows exist and are accepted: retiring a forwarder left from an older build
frees the forwarder's port for a few seconds, once, and the first enable after
a session whose engine held the public port frees it for up to the
forwarder's one-second retry.

Decision: accepted, on the same ground as the forwarder itself. The alternative
is a quit that rewrites every relay tool's config and a crash that strands them.

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
