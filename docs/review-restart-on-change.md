# Why a routing change still asks the user to restart their tools

Written 2026-09-17 on `fix/keep-tool-configs-through-routing-toggle`, which is
the engine park (`fix/dormant-passthrough-on-routing-off`) with `main` merged
into it, so the park and the environment forwarder are in one tree for the
first time.

The question it answers: with a shim that outlives the engine and a PAC that
resolves per request, what is left that genuinely requires a running tool to be
restarted? The short answer is **the config teardown, and almost nothing else**
- and the teardown's own stated reason is no longer true in this tree.

Read `routing-architecture.md` first. This file is the restart question only.

## 1. What forces a restart

A tool reads its configuration once, at startup. So a tool has to be restarted
exactly when **the address it loaded stops working, or stops meaning what it
meant**. Three things can do that:

1. The address changes. Retired: the engine, relay and PAC ports are persisted
   (`proxy/port_persist.rs`) and rebind from a stable band
   (`engine.rs:1722`, `47100..47200`). `codex.rs:315` says it in as many words:
   "The persisted relay port survives restarts precisely so configs stay valid".
2. Nothing answers at the address. Retired for the env channel by the forwarder
   (#295), and retired for the engine and relay ports, while the app runs, by
   the park (`manager_core.rs:178`).
3. **Gate rewrites the config out from under the running process.** Not
   retired. This is the whole of what is left.

### The premise itself was a comment, not a measurement

"Reads it once, at startup" is what everything above rests on, and for Codex it
is wrong. Measured 2026-09-18 on codex-cli 0.146.0-alpha.3.1, driving `codex
app-server` against two loopback listeners and watching which one a turn
reached:

| | picks up an edited `config.toml`? |
| --- | --- |
| a new thread in a running process | yes, immediately, no restart |
| a thread already open | no, it keeps the address it started with |
| that thread resumed after a restart | yes, it re-resolves |

The unit is the **conversation**, not the process. A thread pins the provider
*name* and re-resolves it against whatever is on disk when it starts or
resumes, which is also why the passthrough stub has to survive disconnect.

**Nothing in sections 2 to 6 changes.** Every claim there is about an address
that stops answering or stops meaning what it meant, and that holds per
conversation exactly as it held per process. What changes is **what the UI may
say**: "reopen Codex" helps neither half, and the copy is now "New conversations
will go through Gate." Section 7 of `routing-architecture.md` carries the
measurement.

It also settles the `[model_providers.gate]` question section 6 leaves open, in
favour of keeping the block. Conversations already open are pinned to the relay
address, and the parked relay forwards them straight through, so keeping it is
what lets them go on working. Removing it on a routing toggle would be worse
than anything shipped so far: `disconnect` does not remove it either, it leaves
a passthrough stub, because a thread whose provider name stops resolving cannot
resume at all.

**The other four were then put through the same probe**, and Codex stays the
only exception:

| tool | version | granularity | how |
| --- | --- | --- | --- |
| Codex | 0.146.0-alpha.3.1 | **per conversation** | measured |
| Claude Code | 2.1.276 | per process | measured |
| OpenCode | 1.18.27 | per process | measured |
| OpenClaw | 2026.6.11 | per gateway process | vendor-stated, not measured |
| Hermes | - | per process | measured |

Claude Code's answer is also structural, which is why it is safe to lean on: the
`env` block becomes process environment variables, and those cannot change under
a running process. OpenCode was driven through a headless `opencode serve` with
its provider `baseURL` repointed underneath it.

**OpenClaw is uncontradicted rather than measured**, and the difference matters
because that is exactly the footing the Codex claim was on before it turned out
to be wrong. Its CLI says "Restart the gateway to apply" and a running gateway
logged nothing about a change made underneath it, but a gateway on a fresh
profile makes no outbound request at all, and forcing one needs a provider
credential. **Hermes was measured** once a working install existed: a
`hermes chat` session against a fake proxy named by `HTTPS_PROXY`, repointed
mid-session, produced nine fresh requests that all still went to the old proxy.
Per process, as its own connect note always said.

## 2. The teardown, and the premise it rests on

`routing::disable` runs the full config sweep before the proxy comes down:

```
routing.rs:79    provider::snapshot_and_disable_everything()
routing.rs:82    proxy::manager().disable()
```

The sweep (`provider.rs:729`) walks the registry and disconnects every tool
that is Connected or Drifted, which rewrites each tool's config back to its own
upstream. `routing.rs`'s doc gives the reason:

> leaving them pointed at the relay we are about to kill would strand them
> while the UI reports "not routing"

**In this tree the relay is not killed.** `disable_inner` parks instead
(`manager_core.rs:584`): the ports stay bound and `set_intercept(false)` drops
both listeners to plain forwarding. The premise the sweep rests on is false,
and the sweep is the last thing forcing a restart.

It is worse than redundant. It defeats the park's best property. With the
configs left in place:

- routing off parks the relay, so a running `codex` passes through direct;
- routing on unparks it, so the same process routes through Gate again;
- **neither transition needs a restart**, because the address it loaded never
  stopped working and never changed meaning.

With the teardown, a tool started during the off window loads the direct config,
and needs a restart when routing comes back. The switch stops being live.

## 3. What is still wired to the engine rather than the shim

Independent of the teardown, and true on `main` as well.

`ConnectInput.engine_proxy_url` is `proxy::engine_proxy_url()` at all four
construction sites (`provider.rs:329, 499, 549, 838`) plus `src-tauri/src/lib.rs`.
That is the engine's **own** port. So:

| what | address it names | on a stop |
| --- | --- | --- |
| the machine-wide export | the forwarder (`mod.rs:1120`) | fails open |
| Claude Code `settings.json` | the engine (`mod.rs:1331`) | fails closed |
| OpenClaw `proxy.proxyUrl` | the engine | fails closed |
| Hermes `.env` | the engine | fails closed |

The first and the second sit in the same `env` block, under the same variable
name, pointing at two different addresses, one hardened and one not.

Pointing the tool configs at `exported_proxy_identity_url` instead would make
them fail open on every path the export already does, including the one the
park cannot cover (below). The Claude Code route selector rides in the URL's
userinfo and survives a verbatim hand-off, so it keeps working.

## 4. Writes are not idempotent

Every integration rewrites its file whether or not the bytes changed:
`claude_code.rs:341`, `opencode.rs:439`, `codex.rs:581`, `openclaw.rs:310`.
`primitives::write_file` has no compare.

The exception is `dotenv::add_vars`, which skips when every value is already
right (`dotenv.rs:215-218`, early return at `:232`) with the reasoning written
down: "or every unattended re-connect would announce itself as a repair".

On this branch that costs churn and a bumped mtime. It costs more on
`feat/new-app-ui`, where `reopen.rs` decides staleness by comparing a process's
start time against the config file's mtime, so a byte-identical rewrite is
indistinguishable from a real change and raises "Reopen required" on a tool
nothing happened to.

## 5. Plan, and what it became

All three landed on this branch, in this order. Each is a separate commit and
separately revertable.

1. **Make the writes idempotent.** Done in `primitives::write_file`, one place
   rather than five, because it is a property of writing config files and not of
   any one format. The `mode` fix is deliberately not skipped with the write: a
   config that already holds the right bytes under the tool's own umask is
   exactly the file a naive skip would leave at 0o644 forever.
2. **Point the proxy-tool configs at the forwarder.** Done, as two functions
   rather than one, because the old single call was answering two questions:
   `tool_proxy_url` is the write path and may spawn, `tool_proxy_identity_urls`
   is the read path and never does. The second returns a *list*, because two
   addresses are legitimately ours at once - every install written before this
   holds the engine's, and it routes.
3. **Stop tearing down tool configs on a routing toggle.** Done via
   `provider::ToolConfigs`, so the routing switch parks and the
   quit-and-disconnect choice still sweeps. Section 6 was written before this
   was attempted and named one of the three traps; the other two are recorded
   there now as well.

**The residual, stated plainly.** An existing install is not migrated onto the
forwarder's address by being told it is broken - both addresses read Connected,
so nothing drifts and nothing repairs. It moves the next time something writes
its config: connecting the tool, reconnecting it, or a master-on restore. Until
then it keeps the behaviour it already had, which is the pre-forwarder
behaviour rather than a regression. Forcing it would mean reporting a working
config as drifted, and a reconcile pass rewriting a file that is already right.

## 6. Traps, found by following item 3

Section 5's third item turned up three of these. One was predicted here before
the work started; the other two were not, and they are the reason this section
is longer than the plan above.

**`relay_listening` cannot tell parked from routing.** `mod.rs` is a bare TCP
probe with no snapshot gate, so a parked relay reads as alive. `codex::status`
treated that as "the proxy is running", so once the teardown stopped rewriting
Codex's config, Codex reported **Connected while routing is off** - a green pill
over traffic going direct, which is the single thing that status exists to
prevent. Fixed by asking the routing intent as well. The intent rather than
anything measured, because the relay publishes no health endpoint on this branch
and "is it intercepting" is not observable from outside the hosting process. The
known inaccuracy is the headless `proxy relay` host: it always intercepts and
writes no intent file, so on a machine whose last explicit answer was "off" it
under-claims. That is the safe direction.

`engine_proxy_url` does not have this problem and it is worth knowing why: it
gates on `engine_likely_running()`, which reads the system-proxy snapshot, and
`disable_inner` clears that snapshot while parking. So OpenClaw and Hermes still
correctly refuse to connect while parked. The fix for `relay_listening` uses a
separate signal rather than that one on purpose - `relay_listening`'s own doc
explains that it probes the port precisely because a standalone relay host
leaves no snapshot.

**Drift is the steady state while parked**, which no longer had anywhere safe to
land. The three proxy tools' `status` asks whether the engine is *routing*; it
is not, so each reads `Drifted`, and each is `config_is_managed`. Both reconcile
passes act on exactly that pair, so they would have called a `connect` that
refuses by design, on every startup and every window focus, logging a failure
about a machine with nothing wrong with it. `Integration::requires_engine` is
the guard, declared rather than matched off an error string for the reason
`restore_swept_tools` already gives in place.

The first attempt was blunter - skip reconcile entirely whenever routing is off
- and it broke five tests in `reconcile_enabled` that exercise reconcile without
setting the intent. They were right to break.

**"`<addr>` is a dead address" became false.** All three proxy integrations said
it for every not-routing state, which was accurate when the only way to stop
routing was to release the port. `proxy::loopback_proxy_answers` is the
measurement that separates a parked listener from a released one, and the
message now says which one it is.

**App exit was the residual, and it split in two.** On macOS and Windows the
listeners live in the GUI process, so quitting releases the ports. For the three
forwarder-named tools item 2 closes it: the forwarder outlives the GUI and
forwards direct. For the two relay-named tools nothing fronts the port, and the
options were weighed on the branch stacked on this one
(`fix/plain-quit-reverts-relay-configs`): fronting the relay at TCP level only
turns "refused" into a 503, a passthrough forwarder changes what that process
is, and a daemon is a packaging project. What landed is the narrow rule: plain
quit reverts a config if and only if an address it names dies with the process,
decided per configured address (`proxy::address_dies_with_gui`) rather than per
tool, because which address a config holds is per install. It costs Codex a
thread-list flip per quit/launch and a
running OpenCode a restart after relaunch, both already the price of the
quit-and-disconnect choice, now paid by two tools instead of five.

**Leaving Gate's values in a config while parked is not a credential
question.** No tool config anywhere holds a credential, and a parked relay
injects none. The policy this follows is already written for the forwarder at
`manager_core.rs:87-89`: a plain disable is not "let go of this machine", and
the paths that are keep their teardown.

**This was raised as a product question and is now answered by measurement.**
After item 3, a user who switches routing off and opens `~/.codex/config.toml`
finds `[model_providers.gate]` still there. That is not new - `disconnect` has
always left the block behind, as a passthrough stub - and it is load-bearing:
a Codex conversation pins its provider by name, so a conversation open across
the toggle needs the block to keep resolving, and the parked relay is what
carries it. See the measurement under section 1. What is left for design is
narrower: not whether the block stays, but whether a parked tool should read as
a problem in the UI at all.

## 7. What is verified here, and what is not

**Verified in this tree.** clippy `-D warnings` clean and
`cargo test --workspace -- --test-threads=1` green at every commit: 382 tests
at the merge, 390 with this branch's own. Every file:line in this document was
read at the merge commit; the line numbers moved afterwards and the symbol
names are the durable reference.

**Mutation-checked, not just green.** The headline assertion is that a full
master off/on cycle does not move a tool config's mtime
(`master_cycle_preserves_members`). It needs both halves of the branch, and
each was broken separately to confirm the test notices: reverting the park to
`ToolConfigs::Reverted`, and disabling `write_file`'s identical-write skip. An
earlier version of that test used OpenCode and passed under both mutations,
because no provider claims OpenCode so the provider pass never reaches it. It
was replaced rather than kept as decoration.

**Read, not run.** The park's own behaviour, and everything this branch does on
macOS and Windows. The park carries tests pinning that it beats the Claude Code
selector and beats an enabled domain, but no live toggle was driven here, and
the forwarder repoint is the half that most wants one: `tool_proxy_url` falls
back to the engine's address when `ensure_running` fails, and that fallback has
been reasoned about rather than provoked. Note also that `pnpm app:local`
cannot see it - the seam that makes the keychain safe also skips this - so the
check has to be `pnpm app` on each OS.

**Measured here.** Codex's config-reload granularity, under section 1: a new
thread picks up an edit immediately, an open one never does, and a resumed one
re-resolves. Driven against a throwaway `CODEX_HOME` and two loopback listeners,
so no account and no network were involved. It is an alpha build
(0.146.0-alpha.3.1) and worth re-running on a version bump.

**Not measured, and named as a question rather than a finding.** Whether a GUI
app re-resolves the PAC without a restart. `feat/new-app-ui`'s
`useSectionRouting` offers to close the Claude desktop app on a domain toggle,
on the strength of a comment asserting the app "resolves that proxy at its own
launch", and no measurement behind that assertion exists in the repo. It is not
in this branch's scope, but it is the same class of claim, and it is the one
with the worst consequence if wrong: Gate offers to kill a running app for a
change that may need nothing.

**Out of scope.** The reopen flow itself. It does not exist on `main`; the
restart burden there is carried by four `eprintln!` hints that go nowhere in a
GUI build (`env_proxy.rs`, `hermes.rs`, `openclaw.rs`, `codex.rs`), which
`routing-architecture.md` section 6 already files as "Relaunch is required and
currently unsaid". Everything above narrows what that flow would have to say.
