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

## 5. Plan

In dependency order. Each is separately revertable.

1. **Make the writes idempotent.** Compare before writing; keep the `0o600`
   chmod on the skip path, because `write_file` also fixes mode on overwrite.
   Independent of everything else, and the one item that is pure subtraction.
2. **Point the proxy-tool configs at the forwarder.** `provider.rs`'s four
   sites and the Tauri one. Drift and `status` then have to compare against
   `exported_proxy_identity_url`, not `persisted_engine_proxy_url`; section 2
   of `routing-architecture.md` already warns that comparing against the
   engine's reports every correctly-exported machine as permanently drifted.
3. **Stop tearing down tool configs on a routing toggle.** `routing.rs:79`.
   Disconnect stays what it is: a per-tool action, and the explicit "let go of
   this machine" paths (sign-out, Reset, untrusting the CA) keep sweeping.
   Read section 6 before doing this one.

## 6. Traps, for whoever takes item 3

**`relay_listening` cannot tell parked from routing.** `mod.rs:817` is a bare
TCP probe with no snapshot gate, so a parked relay reads as alive.
`codex::status` (`codex.rs:321`) treats that as "the proxy is running", so once
the teardown stops rewriting Codex's config, Codex would report **Connected
while routing is off**. Today the identity check fails first, which is the only
reason this is invisible. `engine::intercepting()` (`engine.rs:169`) already
computes the distinction and is not reachable from the status path.

`engine_proxy_url` does not have this problem and it is worth knowing why: it
gates on `engine_likely_running()` (`mod.rs:761`), which reads the system-proxy
snapshot, and `disable_inner` clears that snapshot at `manager_core.rs:597`
while parking. So OpenClaw and Hermes still correctly refuse to connect while
parked. Any fix for `relay_listening` should use the same signal rather than a
second one.

**App exit is the residual, and item 2 is what closes it.** On macOS and
Windows the listeners live in the GUI process, so quitting releases the ports
and a config naming them is stranded until the app is next opened. The park
cannot fix this; a listener that outlives the GUI can, and the forwarder is
one. That is the argument for doing item 2 before item 3, not after.

**Leaving Gate's values in a config while parked is not a credential
question.** No tool config anywhere holds a credential, and a parked relay
injects none. The policy this follows is already written for the forwarder at
`manager_core.rs:87-89`: a plain disable is not "let go of this machine", and
the paths that are keep their teardown.

**It is still a product question**, and it is the one to put to design rather
than settle here: after item 3, a user who switches routing off and opens
`~/.codex/config.toml` finds `[model_providers.gate]` still there. It routes
nothing, and the row says so, but the file says Gate is configured. Principle 1
argues both ways - the user should feel where things live, and the file is now
telling the truth about a thing that is parked rather than gone.

## 7. What is verified here, and what is not

**Verified in this tree.** The merge: clippy `-D warnings` clean, and
`cargo test --workspace -- --test-threads=1` green at 382 tests across 31
binaries. Every file:line in this document was read at the merge commit.

**Read, not run.** The park's own behaviour. Its branch carries tests pinning
that a park beats the Claude Code selector and beats an enabled domain; this
review did not drive a live toggle.

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
