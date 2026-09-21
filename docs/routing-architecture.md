# Routing architecture

How traffic actually reaches Gate, what each tool depends on, and what is
verified. Written 2026-08-06, after the work in PR #112. Supersedes the
mechanism half of `harness-integration-validation.md`, which is kept for its
per-finding detail but predates the proxy rewrites.

Read this before changing an integration or the routing UI.

## 1. Three mechanisms, not one

There are three distinct ways a tool's request ends up at the gateway. They
fail differently, so which one a tool uses determines what the UI can honestly
claim about it.

**The relay** (`proxy::relay`) is a plaintext loopback *reverse* proxy. The tool
config names it as a base URL, and the relay injects the live Gate credential
and the upstream hint per request. Nothing secret is ever written to a tool
config. Routing is per-endpoint: whatever decides which endpoint is live can
route around us.

**The proxy engine** (`proxy::engine`) is a MITM *forward* proxy. A tool points
its own proxy setting at it, or inherits it from the environment. The engine
decides per CONNECT whether the host is intercepted; intercepted hosts are
MITM'd and rewritten, everything else is blind-tunnelled untouched. Routing is
socket-level, so it catches traffic regardless of which config layer won.

**The OS proxy setting** is how config-less GUI apps are reached: a PAC on
macOS/Windows, part of the `environment.d` drop-in on Linux.

The distinction that matters: **a relay integration can be silently bypassed; a
proxy integration cannot.** Every "green pill, no routing" bug in
`harness-integration-validation.md` is an instance of the former.

## 2. The two channels the system proxy wires

Enabling routing wires two channels, because they reach different populations.

|                  | Reaches                        | macOS              | Windows                                 | Linux                   |
|---|---|---|---|---|
| OS proxy setting | GUI apps, platform HTTP stacks | `networksetup` PAC | WinINET `AutoConfigURL`                 | (same drop-in)          |
| Proxy env vars   | CLI tools (Node/Bun/Python)    | `launchctl setenv` | `HKCU\Environment` + `WM_SETTINGCHANGE` | `environment.d` drop-in |

Before PR #112 only Linux wired the env channel, so on macOS and Windows the
command-line tools were never routed by the master switch at all.

`proxy::proxy_env` is the single source of truth for the names and values:

```
http_proxy / https_proxy / HTTP_PROXY / HTTPS_PROXY  -> http://127.0.0.1:<engine-port>
no_proxy / NO_PROXY                                  -> localhost,127.0.0.1,::1
NODE_EXTRA_CA_CERTS                                  -> <app-support>/proxy/ca-cert.pem
```

Four things about this are load-bearing and easy to undo by accident:

- **The proxy points at the engine port, not the PAC port.** Env-var proxies
  have no PAC equivalent, so every request from a tool that honours them goes
  to the engine, which blind-tunnels what it does not intercept.
- **Windows gets upper-case only.** Environment names there are
  case-insensitive, so writing both cases is two writes contending for one
  registry entry.
- **`NO_PROXY` loopback is required, not polite.** OpenCode's TUI talks to its
  own local HTTP server; proxying that forms a loop. Their docs say so.
- **`NODE_EXTRA_CA_CERTS` is additive** to Node/Bun's bundle, so one cert is
  right. Contrast Python, below.

### Prior values are snapshotted on macOS and Windows

The Linux drop-in is a file we own outright, so "off" is a delete that uncovers
whatever else the session set. `launchctl` and `HKCU\Environment` are *shared*
stores: we overwrite in place, and deleting on disable would destroy a
corporate egress proxy. So prior values are recorded and restored, with a
re-entry guard so a second enable does not record our own values as the user's.

### Ordering rule for disable

Revert the env channel **first**, above the PAC restore. A stale PAC fails open
to DIRECT; a stale `HTTPS_PROXY` makes every CLI request fail to connect. On
Windows this also has to run above the early return in `reconcile_on_startup`,
because registry values outlive a reboot where launchd variables do not.

### The variables name the forwarder, not the engine

The ordering rule above is necessary and not sufficient, because `launchctl
unsetenv` cannot reach a process that is already running. Every shell, editor
and CLI keeps the exported `HTTPS_PROXY` for its whole life, so whatever
address it names has to keep answering after the engine goes away - otherwise
turning routing off, quitting the app, or an engine crash all become "no
provider is reachable", for tools that were never switched on and for software
Gate does not manage, since the export is machine-wide.

The two channels failed asymmetrically, and that was the bug. A PAC naming a
dead port **fails open**: the fetch fails and the client goes DIRECT. An
exported `HTTPS_PROXY` naming a dead port **fails closed**: the request dies.

So the variables name `proxy::forwarder`, a tiny separate process that hands
each connection to the engine when it answers and connects the client straight
to its destination when it does not. That gives the env channel the PAC's
fail-open behaviour. It holds no credential, reads no traffic and makes no
routing decision: the engine already blind-tunnels whatever it does not route,
so "hand it over when it answers" needs no rules, and needs no PAC - which
matters because the PAC is served *by* the engine and so disappears exactly
when the fallback is needed.

It is a separate process because the failure includes the GUI going away. This
is the same shape as the Linux helper daemon and is spawned the same way
(`<current-exe> --env-forwarder`, detached), so there is no second binary to
package, sign or locate. Linux does not run one: its engine is already a daemon
that outlives the GUI, which is why Linux never had this bug.

It ships as a **separate small binary** (`gate-connect-forwarder`), installed
beside the app as a Tauri sidecar. Not the app re-invoked with a flag, which is
how the Linux helper daemon works, because a running process holds a file lock
on its own image: if that image were the app's, the Windows updater could not
replace it. A distinct name in Activity Monitor and Task Manager is the second
reason, and linking none of the app's machinery is the third.

On macOS it is **socket-activated**: a LaunchAgent declares the socket, launchd
binds it at login and starts the forwarder on the first connection. So the
address answers from login onward even with no Gate process running, nothing
sits resident between uses, and the port cannot be squatted. Best-effort - if
the agent does not produce a forwarder that answers, it is removed and the app
spawns one directly, which is what Windows does anyway.

Lifecycle: started on enable when the export is opted in, deliberately **not**
stopped on disable (that is precisely when the processes holding our variables
need it), and retired by removing the marker file it polls (plus the agent on
macOS). The two places that retire it are signing out and untrusting the CA -
both explicit "let go of this machine" actions. Note Reset is a disable plus
`clear_account`, so it is the sign-out half that stops the forwarder, not the
CA half. A forwarder that will not start is not fatal: the enable falls back to
exporting the engine's own port, which is the pre-forwarder behaviour.

One consequence worth knowing: the exported address is no longer the engine's,
so `exported_proxy_identity_url` - not `persisted_engine_proxy_url` - is what a
drift check must compare against. Comparing against the engine's would report
every correctly-exported machine as drifted, permanently.

### Disable parks the engine, it does not stop it

The forwarder above answers the same premise for the *exported variables*: a
process already running keeps whatever address it inherited, so that address
has to keep answering. A **tool config** outlives its writer the same way, so
the three proxy integrations name the forwarder too - Claude Code's
`settings.json`, OpenClaw's `proxy.proxyUrl` and Hermes's `.env` all take
`proxy::tool_proxy_url`, which is the forwarder's address with the engine's as
the fallback when one will not start. Two addresses are therefore ours at once,
and `proxy::tool_proxy_identity_urls` is what a status check compares against:
an install written before that change holds the engine's, and it routes, so
calling it drift would draw a repair over a working file.

What no forwarder fronts is the **relay** port, which every `base_url` names -
Codex and OpenCode. That one is the park's alone.

### The routing toggle keeps tool configs; disconnect reverts them

Master-off used to revert every tool's configuration on its way out, on the
reasoning in `routing::disable`'s own doc: a config naming "the relay we are
about to kill" would strand the tool while the UI reported "not routing". The
park is what retired that reason, and `provider::ToolConfigs` is the split it
left behind.

`snapshot_and_park_everything` is the routing switch: snapshot the enabled
providers, turn the domains off, leave every tool config alone. The addresses
those configs hold still answer, and they forward direct, so the tool reaches
its own provider exactly as it would with Gate not installed. Reverting them
moves no traffic and costs a restart of every running tool, because a tool
reads its configuration once and the file's mtime is what says it missed a
change.

`snapshot_and_disable_everything` is the full sweep, and the quit-and-disconnect
choice still runs it. Signing out and Reset are on the same side of that line,
which is where `forwarder::stop` already sat.

Keeping the configs makes the switch **live**: a `codex` running before the
toggle passes through while parked and routes again when the engine unparks,
without being restarted at either edge. Reverting was what broke that, by
handing the next-started process a different answer from the one the running
process holds.

Three consequences are handled rather than discovered:

- **Drift is the steady state while parked** for the three proxy tools, since
 their `status` asks whether the engine is *routing*. So the reconcile passes
 ask `Integration::requires_engine` before re-asserting a drifted config;
 otherwise they would call a `connect` that refuses by design, on every
 startup and every window focus.
- **"Dead address" became false.** All three said the configured address was
 dead for every not-routing state, which was true when the ports went away.
 `proxy::loopback_proxy_answers` is the measurement that separates a parked
 listener from a released one, and the message now says which it is.
- **A bare TCP probe reads a parked relay as routing.** `relay_listening` is
 that probe, so Codex would have reported Connected with routing off - a green
 pill over traffic going direct, which is the one thing its status exists to
 prevent. It asks the routing intent as well now. The known inaccuracy is the
 headless `proxy relay` host, which always intercepts and writes no intent
 file.

So the routing toggle **parks** the engine: the ports stay bound and
`set_intercept(false)` drops both listeners to plain forwarding, which is the
path those tools would have taken with Gate not installed. Linux has always
done this (`helper::set_passthrough`); the desktop managers now do too.

`set_intercept` is what parks, **not** clearing the domain set. `route_rules`
force-enables Claude Code's entry precisely when the live set does not claim
the host, so an empty set makes the selector path fire rather than stop: an
engine parked by clearing domains alone would go on decrypting and billing a
`claude` session started before the toggle.

Four paths still release the ports, because a parked listener would be wrong
there rather than idle: app exit, a gateway switch, a re-enable (which rebinds
the same port), and untrusting the CA. App exit is the residual - the listeners
live in the GUI process on macOS and Windows, so quitting still strands
already-running tools. Closing that needs a listener that outlives the GUI,
which is what the Linux daemon already is.

## 3. Per-tool status

| Tool                                | Mechanism                         | What Gate writes                                                                             | In UI  |
|---|---|---|---|
| Claude Code                         | proxy engine                      | `HTTPS_PROXY` + `NODE_EXTRA_CA_CERTS` in the `settings.json` env block; canonical Anthropic base URL stays untouched | yes    |
| Codex                               | relay                             | `[model_providers.gate]` + pointer                                                           | yes    |
| OpenCode                            | relay                             | `provider.<id>.options.baseURL`                                                              | yes    |
| OpenClaw                            | proxy engine                      | `proxy.proxyUrl` + `NODE_EXTRA_CA_CERTS`                                                     | yes    |
| Hermes                              | proxy engine                      | four vars in `~/.hermes/.env`                                                                | yes    |
| **Terminal tools** (`env-proxy`)     | proxy engine, via the environment | nothing per-tool; the machine-wide export                                                    | yes    |

Claude Code's proxy URL includes a fixed, non-secret route selector. That lets
the engine keep intercepting its canonical Anthropic connection when the user
independently switches off the Claude Desktop domain; without it, the same
connected configuration would silently blind-tunnel around Gate. It is scoped
to that one destination: a selected connection to any other host is decided by
the catalog alone. What it does not do is make a bypass detectable from the
config file - see O1 below.

Claude Code needs the CA in that same env block, not just the proxy. Node and
Bun ignore the OS trust store, so the system-wide anchor install does nothing
for `claude`: routed with no `NODE_EXTRA_CA_CERTS` it rejects the engine's leaf
with `UNABLE_TO_VERIFY_LEAF_SIGNATURE`. The settings.json value is a *fallback*
rather than an override, though. Claude Code returns early when the variable is
already in its environment and only then reads `env.NODE_EXTRA_CA_CERTS` from
settings, so a machine that exports its own CA (a corporate bundle from a shell
rc, or the prior value `env_proxy` restored on disable) keeps that one, still
cannot verify our leaf, and `status()` - which can only read the file - reports
Connected anyway. The environment is the channel that wins, and it carries
Gate's cert alone.

No tool config anywhere holds a credential. Codex is the one documented
disconnect exception: a passthrough stub survives so threads started while
routed can still resume.

### The environment channel is its own entry

`env-proxy` is not a tool. It models the *mechanism* - the variables the system
proxy exports - because some tools cannot be configured at all. OpenCode has no
proxy or CA setting anywhere in its config schema and loads no dotenv, so those
variables are the only way its sockets reach the *engine*; an OpenCode-shaped
proxy integration would be a fiction, since nothing tool-specific happens. The
same export covers anything else that reads `HTTPS_PROXY`.

That qualifier is load-bearing, and this sentence used to drop it and read "the
only way to route it". The variables are not the only way OpenCode routes:
`integrations::opencode` rewrites `provider.<id>.options.baseURL` to the relay,
which needs neither a variable nor the CA, and it predates this section. Without
the qualifier the sentence denies the relay half outright, which is how it came
to contradict both the table above and section 6 below.

**OpenCode's own integration is therefore relay-only**, and stays that way. It
writes `baseURL` and nothing else. The env coverage belongs to `env-proxy`.

Neither mechanism subsumes the other, which is why both are wired. The rewrite
is a snapshot taken at connect time over `KNOWN_PROVIDERS`, so a provider the
user adds afterwards, one outside that allowlist, or one the `looks_local` guard
skips has no rewrite and is the environment's to carry. Going the other way, the
rewrite routes without the CA and without touching the machine, which is the
whole reason it is not simply deleted in favour of the channel.

It is a *choice*, not a side effect, because the variables are machine-wide:
`HTTPS_PROXY` redirects git, curl and npm too. `manager.enable()` consults
`proxy::env_export_opted_in()` before exporting, so a user who disconnects it
does not get it back on the next routing toggle. Defaults to on, which is what
the routing switch always implied.

**Linux is the exception.** There the `environment.d` drop-in *is* the system
proxy - no PAC - so the variables cannot be declined without declining routing.
Disconnect still records the choice (it must not fail: sign-out and the
master-off sweep both call it) but cannot withdraw them, and `status` says so
rather than claiming clean. `proxy::env_export_is_separable()` is the flag.

Status is read back from the OS - `launchctl getenv`, the registry, the drop-in
- never from a record of what we wrote. That is the O1 class, and this entry
must not reintroduce it.

### Hermes needs a full CA bundle, not the single cert

Hermes installs into a venv, so its `httpx`/`requests` clients use a
pip-installed certifi that never sees the OS store, and `ssl_verify.py` feeds
the value to `create_default_context(cafile=...)`, which **replaces** the trust
store. A single cert there would break every non-Gate TLS call it makes. Hence
`proxy::ca_bundle` (platform roots + our CA) for Hermes, and the plain cert for
Node/Bun tools. Measured, not assumed.

## 4. Findings from the validation doc, current state

| Finding                                      | State                                                                                                    |
|---|---|
| H1 Hermes `default_headers` OpenAI-wire only | retired - proxy is transport-agnostic                                                                    |
| H2 OpenClaw auth-profile discovery           | retired - no discovery step remains                                                                      |
| H3 OpenClaw Anthropic beta suppression       | retired - `baseUrl` is no longer redirected                                                              |
| H4 OpenClaw behaviour deltas                 | retired for the same reason; still worth knowing if Gate keys off attribution headers                    |
| H5 Hermes fresh install `model: ""`          | obsolete - `config.yaml` is never read                                                                   |
| H6 Hermes only `model.base_url` covered      | retired - proxy catches every provider entry                                                             |
| H7 OpenClaw stale module doc                 | fixed                                                                                                    |
| O1 OpenCode config precedence                | **impact** neutralised by the env vars; the underlying "status reads our own write" problem is untouched |
| O2 `options.headers` undeclared              | moot - we no longer write headers to any tool config                                                     |
| O3 Zen provider IDs unverified               | open                                                                                                     |

The structural problem O1 names - `status()` verifies our own write rather than
the effective configuration - still applies to every integration, including
Claude Code. It is the main reason a tool can show Connected while traffic goes
elsewhere.

Claude Code's route selector narrows what that costs rather than fixing it. The
selector is what makes the engine route a connected session regardless of the
Desktop switch, so the switch alone can no longer strand it - but `status()`
still only reads `settings.json` and never learns whether the engine actually
received the selector. A Claude Code release that stopped deriving
`Proxy-Authorization` from the proxy URL's userinfo would blind-tunnel behind a
green pill. The one witness is the engine: it emits a single line per run when a
CONNECT to Claude Code's own destination arrives without the selector
(`engine::GateHandler::warn_if_anthropic_is_unselected`).

## 5. What is verified, and how

**Verified by automated test.** `crates/core/tests/proxy_e2e.rs` boots the real
engine against a loopback mock gateway. `exported_proxy_env_routes_an_external_process`
drives a real `curl` using *only* what `proxy::proxy_env_vars()` exports, with
`env_clear()` so an ambient proxy cannot carry it. Mutation-checked against a
wrong port, a `NO_PROXY` that exempts the target, a variable-name typo, and a
CA path pointing elsewhere.

Note for whoever extends it: the CA assertion must write at the production
`ca_cert_path()` and read via the *exported* value. Writing to the exported
path and reading it back is circular and passes even when the export is wrong.
That bug was in the first draft.

**Verified by type-check only.** The macOS `launchctl` block, checked on Linux
against the real module. Cross-compiling the crate is blocked by `zstd-sys`
(via `hudsucker`) needing a platform C toolchain.

**Not compiled at all.** The Windows `HKCU\Environment` path. Reviewed against
the `winreg` 0.52 API (`set_value`/`get_value`/`delete_value`, `REG_SZ` via
`to_reg_value_sz!(String)`) and against the WinINET patterns in the same file,
but never built. This is the highest-risk code in the change.

**Not covered, by design.** The `launchctl` and registry writers themselves -
exercising them would mutate the machine running the suite.

**Not run against a live install.** The OpenClaw and Hermes harnesses.

## 6. Implications for the UI

These are the decisions the popover has to make; none are implemented yet.

**Routing is a precondition, not a nicety.** OpenClaw and Hermes `connect()`
refuse when the engine is not running, because pointing a tool's whole egress
at a dead port breaks it outright rather than merely un-routing it. The UI
should not offer Connect for a proxy-based tool while the master switch is off;
today the user gets an error string instead.

**"Pointed at us but the engine is down" is Drifted, never Connected.** Both
proxy harnesses compute this explicitly. The UI needs a state for "configured
but not routing" that reads as a problem, and the copy should offer the way
out (turn routing on, or disconnect).

**Relaunch is required and currently unsaid - but the unit is per tool, and
for Codex it is not the process.** This paragraph used to say relaunch was
required, flatly, on every platform. That is right for the environment channel
and wrong for at least one config tool, and the difference decides what the UI
is allowed to say.

Environment variables only reach processes started *after* the change, on every
platform, and nothing can fix that: a user who turns routing on with OpenCode
already open sees no effect and no explanation.

Codex is the measured exception, and it is a conversation rather than a
process. Measured 2026-09-18 on codex-cli 0.146.0-alpha.3.1, driving
`codex app-server` against two loopback listeners and watching which one a turn
reached:

| | picks up an edited `config.toml`? |
| --- | --- |
| a new thread in a running process | yes, immediately, no restart |
| a thread already open | no, it keeps the address it started with |
| that thread resumed after a restart | yes, it re-resolves |

So a routing change reaches every conversation started after it with no restart
at all, and no conversation already open, however often the process is
restarted, unless the user resumes it. Telling a Codex user to reopen the tool
is advice that does nothing for either half. `integrations::codex` carries the
measurement and emits the copy: **"New conversations will go through Gate."**

The other config tools were put through the same probe on 2026-09-18, and the
answer is per tool:

| tool | version | granularity | how |
| --- | --- | --- | --- |
| Codex | 0.146.0-alpha.3.1 | **per conversation** | measured |
| Claude Code | 2.1.276 | per process | measured |
| OpenCode | 1.18.27 | per process | measured |
| OpenClaw | 2026.6.11 | per gateway process | vendor-stated, not measured |
| Hermes | - | per process | measured |

Claude Code and OpenCode behave the way this document always assumed, and
Claude Code's reason is structural: its `env` block becomes process environment
variables, which cannot change under a running process. OpenCode was measured
through a headless `opencode serve` with its provider `baseURL` repointed
underneath it; a second message on the same session still went to the original
address.

OpenClaw is the one to be careful with. `openclaw config set proxy.proxyUrl ...`
answers "Restart the gateway to apply", and a running gateway logged nothing
about a change made underneath it, but the probe could not be completed: a
gateway on a fresh profile makes no outbound request at all, and every way to
force one needs a provider credential. Uncontradicted is not the same as
measured, and the module doc says so too.

Hermes was measured once a working install was available: a `hermes chat`
session against a fake proxy named by `HTTPS_PROXY`, repointed mid-session.
Nine fresh requests followed the repoint and every one still went to the old
proxy, so it is per process like the others, which its `connect` note already
said.

So Codex remains the only tool where "restart it" is the wrong thing to say,
and it is the one whose module doc asserted the opposite hardest.

**The restart hint is invisible.** OpenClaw, Hermes and now Codex emit it via
`eprintln!`, which goes nowhere in a GUI build.

**Degraded routing is silent.** The env export is deliberately best-effort: if
`launchctl` or the registry write fails, routing still succeeds for GUI apps
and silently does not for CLI tools. Today that difference is only an
`eprintln!`. If the UI ever claims "everything is routed", it needs to know
about this state.

**The environment channel has a switch now.** It sits under the master one in
the Routing card, and is absent entirely on Linux (`env_export_separable`),
where those variables *are* the system proxy and a switch could not honour
itself. Turning it off is a real opt-out that survives routing toggles.

It has a **row** now as well, "Terminal tools", in the Tools band beside
OpenCode. (It was "Experimental" when this paragraph was written; `BAND_LABELS`
in `groups.ts` draws Apps and Tools.) The two are there together because the
channel carries what OpenCode's own config cannot: the `baseURL` rewrite covers
the providers it found at connect time, the variables cover whatever else it
sends, and turning OpenCode on turns the channel on with it. (This used to say
the two "share a mechanism" and that the variables are "how it routes" - which
contradicted the mechanism table in section 3, where OpenCode is relay. See the
qualifier there.) `useRouting`'s `opencode-env` prompt says so before either
write, and the row is what makes that promise checkable. Both controls call
`proxy::set_env_export`, so they cannot disagree.

**The ledger groups by client, not by vendor.** Every row - a config tool or a
proxy domain - answers `taxonomy::Client`, and `buildGroups` buckets on it. So
OpenCode, OpenClaw and Hermes each head a group, Claude Code and Claude Desktop
are two groups rather than one "Anthropic", and the environment channel sits
with the host entries that also cover whatever happens to be running, under
**Any app on this machine**.

This replaced a vendor grouping plus four hand-added headings. A vendor heading
cannot file a tool that routes whatever providers the user configured in it, so
OpenClaw, Hermes and an "Experimental" pair were named one at a time in a
`LEFTOVER_GROUPS` table, behind an `any-provider` catch-all whose whole job was
to catch what the taxonomy could not place. There is no catch-all now: every row
names its client, so nothing can fall off the ledger. (The shared group was
called "Agent harnesses" until the round-15 design pass, then "Other tools".)

**The machine-wide group holds the `openai` domain.** api.openai.com belongs to no
OpenAI tool: Codex is config-routed through the relay, which resolves routes
against the whole catalog (`relay.rs` builds from `default_domains()`, not the
enabled set), so it routes whatever that switch says; the ChatGPT desktop app
talks to chatgpt.com. What the switch governs is MITM interception of that host
for any system-proxy-honouring client - and the clients that depend on it are
OpenClaw and Hermes, which blind-tunnel anything outside the *enabled* catalog.
So the row sits with them, and `provider.rs` no longer lists the slug: the OpenAI
family switch governs Codex alone, which is what it was doing in effect already. It
is labelled **OpenAI API** - the host's role, with `api.openai.com` itself in the
row's description rather than the label, since the popover already prints it in a
mono identifier slot.

The grouping is what makes the row labels work. Rows are named for the surface
they cover - "API" and "Chat" for the two surfaces of the Claude desktop app,
"CLI" for a terminal tool - and a surface kind is only legible under a heading
that names the program. The sentence explaining each row is UI copy, in
`MEMBER_DESCRIPTIONS`.

**Each row also carries a scope and a credential** (`taxonomy::Scope`,
`taxonomy::Credential`), and both are rendered rather than implied:

- `scope` is the blast radius. Every proxy domain is `host`, which means
  flipping it intercepts those hosts for *every* proxy-honouring client on the
  machine, not only the one the row is named for - `should_intercept_host`
  matches on host alone at CONNECT, before any header exists, and the
  per-request narrowing in `rules_for_client` decides only what is rewritten.
  Config tools are `client`; the environment channel is `machine`. The sentence
  comes from `switchScopeNote` in `groups.ts` for a host section and
  `machineScopeNote` for the environment channel.
- `credential` is whose key rides the request, and it is the single thing that
  decides whether a group switch may flip the row: `provider::cascade_domains`
  filters on `Credential::Brokered`, and `cascadeTargets` does the same on the
  frontend. The session surfaces (`claude-web`, `chatgpt-apps`, `chatgpt`) are
  `Additive`, so they are listed under their client and reachable only from
  their own switch.

Those two used to be one `chat` boolean plus a second `chat_domain_slugs` array
on each provider, which is why a row could be silently dropped from the ledger
by an edit meant to keep it out of a cascade, and why the scope fact had nowhere
to live at all. A support thread in 2026-09 turned on exactly that gap: `Chat`
was labelled "Web" and described as the browser tab, so its owner concluded the
Claude desktop app was uncovered when the row covers it, and covers it more
fully than a browser.

`hidden_in_ui` still exists; nothing uses it now that `env-proxy` is listed.
Hiding is always a UI-boundary decision (`list_tools`), never removal from the
registry, because the master-off sweep and `restore_swept_tools` walk it.

**They were listed ahead of the stated bar.** That bar was one end-to-end run
against a real install, per tool, and none of the three has had one. What
changed is that the failure mode is no longer silent: OpenClaw and Hermes route
through the proxy engine, so a config that loses is no longer a config that
lies. The residual risk is a first-run failure on a real install, not a green
pill over dead routing.

**Flipping one on with routing off is an error, by design.** OpenClaw and Hermes
refuse rather than point a tool's whole egress at a dead port. `classifyError`
has a branch for it so the message names the remedy instead of suggesting a
retry, but there is no pre-emptive guard: the row is clickable and the error is
how the user learns. Worth revisiting if it reads badly in practice.

### Who does what, per event

The two tables below are the answer to "does the user have to restart anything",
per tool and per event, on the tree that parks the engine and keeps tool configs
across a routing toggle. They are what the UI is allowed to claim. Written
2026-09-18, off the measurements in section 7 and the code paths named in the
first table.

**What Gate does:**

| event | Gate |
| --- | --- |
| **Start** (after a plain quit) | Rebinds the engine and relay on their persisted ports, re-exports the PAC and the env vars, ensures the forwarder (including when the machine-wide export is declined, since tool configs may name it). Reconnects whatever the previous quit put back on its own settings; every other config is already right, so nothing is written to it. |
| **Routing off** | Parks the engine (ports stay bound, forwarding straight through), reverts the PAC and the env export, records which providers were on. **Touches no tool config** (`provider::snapshot_and_park_everything`). |
| **Routing on** | Unparks (the engine intercepts again), re-exports the PAC and the env, restores the providers. The reconnect writes are byte-identical, so no file is touched (`primitives::write_file`). |
| **Any exit** (tray Quit, macOS Cmd+Q, the crash screen, a logout or shutdown) | Reverts the PAC and the env, stops the engine and the relay; the forwarder keeps running. **Reverts a config if and only if an address it names dies with the process**, decided per configured address (`proxy::address_dies_with_gui`): a base URL under the relay origin, or the engine's own proxy port (a pre-forwarder install, or a forwarder that would not start), is put back on its own settings and recorded for the startup restore (`provider::revert_stranded_configs_for_quit`). A config naming the forwarder, or one the user repointed by hand, is untouched. The quit dialog names the same list before the user chooses, and the revert runs again from `RunEvent::Exit` so the paths that never reach the dialog - Cmd+Q, a logout, a shutdown - are safe by default; the second run is a no-op. Not on an updater relaunch, which is coming straight back. Linux reverts none; its engine is a daemon. |
| **Disconnect and quit** | Restores every config to the tool's own settings, stops the engine, the relay **and the forwarder** (`snapshot_and_disable_everything`, `forwarder::stop`). |

**What the user does:**

| tool | start | routing off | routing on | any exit | disconnect and quit |
| --- | --- | --- | --- | --- | --- |
| **Claude Code** | nothing | nothing | nothing | nothing, works unrouted | restart a running session |
| **Codex** | nothing; a conversation opened while Gate was closed keeps its direct route until resumed | nothing | nothing, open conversations route again | nothing; new conversations go direct, open ones need resuming | resume open conversations |
| **OpenCode** | restart an OpenCode opened while Gate was closed | nothing | nothing | nothing for a new OpenCode; a running one needs a restart | restart |
| **OpenClaw** | nothing | nothing | nothing | nothing, works unrouted | `openclaw gateway restart` |
| **Hermes** | nothing | nothing | nothing | nothing, works unrouted | restart |
| **Terminal tools** (env vars) | nothing | nothing | **new terminal**, for a shell opened while routing was off | nothing, works unrouted | new terminal |

Starting Gate *after* a disconnect-and-quit is the start that costs the most:
`restore_all` rewrites every config, so the last column applies again in
reverse. A start after a plain quit rewrites only the two relay configs, which
is why the Codex and OpenCode start cells are not "nothing".

Three things to read off this:

- **The routing columns are empty but for one cell.** A shell opened while
 routing was off never received the variables, and turning routing back on
 cannot reach it. That is inherent to environment variables and holds on every
 platform; a shell that was already open keeps the forwarder address the whole
 way through and needs nothing.
- **Every exit reverts by one rule: does the address die with the process.**
 Every exit, not only the one that goes through the panel: `quit_app` is
 reached by the tray's Quit and the crash screen's and by nothing else, while
 Cmd+Q comes from Tauri's default menu and a logout comes from the OS. Both
 land in `RunEvent::Exit` having touched none of our own code, so the revert
 runs there too. It deliberately does not *veto* the exit - `ExitRequested`
 can be prevented, but the same event carries a logout, and an app that puts a
 dialog in front of a logout is an app that hangs it. Cmd+Q therefore means
 "quit without disconnecting", the safe half of the panel.
 Decided per *configured address*, not per tool, because which address a
 config holds is per install: Codex and OpenCode name the relay origin, and a
 Claude Code, OpenClaw or Hermes install written before the forwarder repoint
 (or whose forwarder would not start) still names the engine's own port. All
 of those live in the GUI process on macOS and Windows, go back to their own
 settings on the way out, and come back at the next start. A config naming
 the forwarder keeps working, because that process is left running on
 purpose; one the user repointed by hand names nothing of ours and is not
 touched. Before this rule the stranded ones were simply broken until Gate
 ran again, with an error naming a loopback port, which reads as the tool
 being broken rather than Gate being off. The quit dialog names which tools
 will be put back and which keep working, from the same predicate, and a
 notification repeats what was rewritten.
- **Disconnect and quit is deliberately the harsh column.** Stopping the
 forwarder is what makes it "Gate is out of the path", and it means a process
 that inherited the forwarder address fails closed rather than falling back.
 That is the existing design, now visible as the only column with real work.

Confidence: the two routing columns rest on the park keeping its ports and on
the master-cycle mtime test, verified separately, not on a live toggle with a
tool open. The OpenClaw row is vendor-stated rather than measured. Everything
else is measured or read directly off the code path named.

## 7. Open items

1. **Compile and test the Windows env path.** Highest risk in the change.
2. **Run OpenClaw and Hermes against real installs**, then unhide.
3. **Python CA gap in the Linux drop-in.** Scoped and measured, unimplemented:
   `requests` and `httpx` pass an explicit `cafile=certifi.where()`, which makes
   `create_default_context` skip the system store. Two vars at the distro bundle
   (`SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`) fix it. Only pip/venv certifi is
   affected; stdlib `ssl` already loads the system bundle. This belongs in
   `proxy_env`, and should be extended to macOS/Windows with it.
4. ~~Decide OpenCode's mechanism.~~ Done: OpenCode stays relay-only and the
   environment channel is its own entry, surfaced as a switch under the master
   one in the Routing card (fed by `ProxyState`, not `list_tools`, because it is
   a property of routing rather than a tool). Remaining: it is untested against
   a real macOS or Windows session.
5. **Verify the effective config, not our own write** - the general fix for the
   O1 class. **Mostly done (AG-674), with one hole named below.**

   Two halves. The *path* half is closed: `status` now reads the file the
   harness loads, not the one Gate picked. `CLAUDE_CONFIG_DIR` and `CODEX_HOME`
   were being ignored - Gate Connect would edit a file the CLI never opens and
   report `Connected` off that write - and they join `OPENCLAW_CONFIG_PATH`,
   `HERMES_HOME` and OpenCode's `OPENCODE_CONFIG` / `OPENCODE_CONFIG_DIR` /
   `XDG_CONFIG_HOME`, which were already honoured.

   The *precedence* half has its own state rather than a green pill:
   `Status::Overridden(source)` -> `ConfigState::Overridden` ->
   `Reason::ConfigurationOverridden`, whose next action is
   `ShowConflictingConfig` and not `ApplyGateConfiguration`, because re-writing a
   file that is already correct moves nothing. What each integration checks
   before it says `Connected`:

   | Tool | Layer it can lose to | Seen? |
   |---|---|---|
   | Claude Code | enterprise `managed-settings.json` setting `HTTPS_PROXY` or `ANTHROPIC_BASE_URL` | yes |
   | Claude Code | project `.claude/settings*.json`, CLI flags | **no** |
   | Codex | the selected `profile`'s own `model_provider` | yes |
   | Codex | `--profile` / `-c` on the command line | **no** |
   | OpenCode | managed `/etc/opencode/opencode.json`, `OPENCODE_CONFIG_CONTENT` | yes |
   | OpenCode | project `./opencode.json`, `.opencode/` - **finding O1 itself** | **no** |
   | OpenClaw | nothing above its single file; the path override is honoured | n/a |
   | Hermes | an `HTTPS_PROXY` already in the login environment, which python-dotenv will not replace | yes |

   The hole is one shape, not five: **a layer chosen by the harness's working
   directory**. Gate Connect is a windowed process and does not know which repo
   `codex` was started in, so a per-project config is not reachable from here -
   and O1's own case, a repo-local `opencode.json`, is exactly that. Closing it
   needs per-tool traffic attribution in the relay, or the harness's cwd from the
   process table, both larger than this item. Until then the app under-claims
   rather than over-claims, which is the direction that was wrong before.
6. **The OpenRouter ALB fix is mock-tested only**; it asserts our side of the
   contract, not Gate's reassembly.
7. O3: Zen provider IDs.

## 8. Where things live

```
crates/core/src/proxy/
  proxy_env.rs        names + values + prior-value snapshot (all platforms)
  system_proxy.rs     macOS: networksetup PAC + launchctl setenv
  system_proxy_windows.rs  WinINET PAC + HKCU\Environment
  system_proxy_linux.rs    environment.d drop-in (both channels at once)
  manager*.rs         enable/disable/crash/reconcile orchestration
  ca_bundle.rs        platform roots + our CA, for tools that replace the store
  relay.rs            loopback reverse proxy for base-URL integrations
crates/core/src/
  routing_health.rs   the per-tool verdict: what a tool is *doing*
  verdict_log.rs      what the last sweep concluded, kept across launches
  recovery.rs         what an interrupted restore did, entry by entry
crates/core/src/integrations/
  dotenv.rs           shared managed .env edits (never clobbers a user value)
crates/core/tests/proxy_e2e.rs   engine + exported-env end-to-end
```
