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

## 3. Per-tool status

| Tool                                | Mechanism                         | What Gate writes                                                                             | In UI  |
|---|---|---|---|
| Claude Code                         | proxy engine                      | `HTTPS_PROXY` in the `settings.json` env block; canonical Anthropic base URL stays untouched | yes    |
| Codex                               | relay                             | `[model_providers.gate]` + pointer                                                           | yes    |
| OpenCode                            | relay                             | `provider.<id>.options.baseURL`                                                              | yes    |
| OpenClaw                            | proxy engine                      | `proxy.proxyUrl` + `NODE_EXTRA_CA_CERTS`                                                     | yes    |
| Hermes                              | proxy engine                      | four vars in `~/.hermes/.env`                                                                | yes    |
| **Environment proxy** (`env-proxy`) | proxy engine, via the environment | nothing per-tool; the machine-wide export                                                    | hidden |

Claude Code's proxy URL includes a fixed, non-secret route selector. That lets
the engine keep intercepting its canonical Anthropic connection when the user
independently switches off the Claude Desktop domain; without it, the same
connected configuration would silently blind-tunnel around Gate. It is scoped
to that one destination: a selected connection to any other host is decided by
the catalog alone. What it does not do is make a bypass detectable from the
config file - see O1 below.

No tool config anywhere holds a credential. Codex is the one documented
disconnect exception: a passthrough stub survives so threads started while
routed can still resume.

### The environment channel is its own entry

`env-proxy` is not a tool. It models the *mechanism* - the variables the system
proxy exports - because some tools cannot be configured at all. OpenCode has no
proxy or CA setting anywhere in its config schema and loads no dotenv, so those
variables are the only way to route it; an OpenCode-shaped proxy integration
would be a fiction, since nothing tool-specific happens. The same export covers
anything else that reads `HTTPS_PROXY`.

**OpenCode's own integration is therefore relay-only**, and stays that way. It
writes `baseURL` and nothing else. The env coverage belongs to `env-proxy`.

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

**Relaunch is required and currently unsaid.** Environment variables only reach
processes started *after* the change - on every platform, and nothing can fix
that. A user who turns routing on with OpenCode already open will see no effect
and no explanation. This is the single most likely support question, and it is
worth a line in the UI at the moment routing is enabled.

**The restart hint is invisible.** OpenClaw and Hermes emit it via `eprintln!`,
which goes nowhere in a GUI build.

**Degraded routing is silent.** The env export is deliberately best-effort: if
`launchctl` or the registry write fails, routing still succeeds for GUI apps
and silently does not for CLI tools. Today that difference is only an
`eprintln!`. If the UI ever claims "everything is routed", it needs to know
about this state.

**The environment channel has a switch now.** It sits under the master one in
the Routing card, and is absent entirely on Linux (`env_export_separable`),
where those variables *are* the system proxy and a switch could not honour
itself. Turning it off is a real opt-out that survives routing toggles.

**The harnesses are listed.** OpenCode, OpenClaw and Hermes now appear in the
ledger, forming the "Other tools" group that was dormant while they were
hidden. (That group was called "Agent harnesses" until the round-15 design pass:
it is the label on a `filter(t => !claimed.has(t.slug))`, and nobody installs a
harness.) `hidden_in_ui` still exists and `env-proxy` still uses it; hiding is
always a UI-boundary decision (`list_tools`), never removal from the registry,
because the master-off sweep and `restore_swept_tools` walk it.

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
   O1 class across relay integrations.
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
crates/core/src/integrations/
  dotenv.rs           shared managed .env edits (never clobbers a user value)
crates/core/tests/proxy_e2e.rs   engine + exported-env end-to-end
```
