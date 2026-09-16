# Spike: attributing a request to the process that sent it

Question, from the #271 review: the engine already looks the peer up for the
owner check, so could peer PID -> process name name every tool on the shared
proxy export, including the ones no config file can reach?

Measured on Linux 7.0.0-30-generic, 2026-09-16. Probe under
`docs/spike-peer-process-attribution/`. **Recommendation: no, not for
attribution.** The reasoning is below, and two of the three findings are
reusable whatever is decided.

## The mechanism works, and is more than an extension

The chain on Linux is: peer address -> the client's row in `/proc/net/tcp` ->
socket inode (field 9) -> scan `/proc/<pid>/fd/*` for `socket:[inode]` -> pid ->
`comm` / `cmdline` / `exe`. The probe resolves a real loopback connection
end to end every time.

But "already does a peer lookup" overstates what is there.
`engine::peer_uid_for` is `#[cfg(target_os = "linux")]` and returns `None`
everywhere else, and it stops at the **uid** (field 7). The inode and the fd
scan are new work, and the other two platforms are new implementations, not
extensions:

| | socket -> pid | shape |
|---|---|---|
| Linux | `/proc/net/tcp` + `/proc/*/fd` walk | O(all fds); no reverse map exists |
| Windows | `GetExtendedTcpTable(TCP_TABLE_OWNER_PID_CONNECTIONS)` | direct, the pid is in the row |
| macOS | `proc_listpids` + `proc_pidfdinfo` per fd | O(all fds), and `libproc` is not a dependency today |

`sysinfo` is already a dependency and would cover *describing* a pid on all
three, but it has no socket-to-pid facility, which is the half that is hard.

## What the process is called is the crux, and the answer is "sometimes"

This is what sinks it. Hermes is Python, so the fields disagree with each other
depending only on how it was started:

| invocation | `comm` | `exe` | `cmdline` |
|---|---|---|---|
| venv console script (`uv venv` + pip, which is what `setup-hermes.sh` does) | `hermes` | `/usr/bin/python3.12` | `…/python …/bin/hermes` |
| `python3 -m hermes_cli` | `python3` | `/usr/bin/python3.12` | `…/python -m hermes_cli` |
| console script with a longer name | `hermes-agent-cl` | `/usr/bin/python3.12` | `…/bin/hermes-agent-cli-longname` |
| bare `python3 -c …` | `python3` | `/usr/bin/python3.12` | `…/python -c …` |

Three things follow:

- **`exe` is useless for an interpreted tool.** It names the interpreter in
  every row. Any design that keys on the executable path fails for Hermes,
  OpenClaw and anything else shipped as a script.
- **`comm` is right for the shape Hermes installs as**, which is the good news,
  and wrong for a module invocation. It is also `TASK_COMM_LEN`-truncated to 15
  characters, which the third row shows.
- **Only `cmdline` carries the name in every case** - and matching a needle
  against `cmdline` is *the same class of heuristic as the User-Agent match this
  was meant to replace*. A substring search over a string we do not control,
  which varies by how the user started the program.

That is the finding. Peer-process attribution does not give structural
evidence the way the relay path marker does, where the tool is named because
Gate wrote the URL. It swaps one substring heuristic for another. The new
string is better in one way - it is local, so it cannot be set by a remote -
and worse in another: the User-Agent at least does not change when you start
the same program a different way.

## Cost: ~32ms per lookup, and it cannot be indexed away

Full `/proc/*/fd` walk on this machine: **median 32.2ms over 5 runs**, 11,082
fds across 572 processes. Filtering to the owner's uid first saved nothing
(31.3ms, 11,080 fds) because a developer's machine is effectively single-user -
the filter would only pay off on a box this product does not target.

Procfs has no reverse inode-to-pid index, so this is inherent rather than a
naive implementation; `ss -p` performs the same walk. Latching once per CONNECT
rather than per request bounds it, which is what the selector does, but 32ms is
still paid on the accept path of every new connection, and it grows with the
machine's total fd count rather than with anything Gate controls.

## The one thing it does better than the selector

It attributes the **process that actually connected**. Review finding 2 was
that a proxy selector living in the process environment is inherited: `codex`,
`curl` or an MCP server spawned from Claude Code's Bash tool sends Claude
Code's selector and mis-files as `claude-code`. Peer-process attribution gets
those right by construction, because it asks the kernel who opened the socket
rather than what the environment says.

If attribution of the shared export is ever revisited, that property is the
reason to come back to this.

## Also worth recording: the selector is inert in the default configuration,
not in every one

The blocking finding is that `proxy_env` exports a bare `HTTPS_PROXY`
machine-wide, and python-dotenv's `load_dotenv()` is `override=False`, so
Hermes' `.env` line never applies. That is true while the export is on -
`export_opted_in()` defaults to true and only an explicit disconnect clears it.

So #271's mechanism is not wrong everywhere; it is defeated by a setting the
user can turn off. That is not a reason to ship it - a feature that works only
for users who disabled a default is worse than none, because its status pill
cannot tell the two populations apart - but it does mean the idea is sound and
the export is what conflicts with it. Any future attempt should start there
rather than at the selector.

## Recommendation

1. **Do not build peer-process attribution for Hermes.** Three platform
   implementations and 32ms per connection to arrive at a `cmdline` substring
   match is a poor trade against the User-Agent substring match it replaces.
2. **Record Hermes as unattributable** in the gate manifest, which is what
   `harnesses.json` already says: detector `null`, tier `works` not `certified`,
   with the exception text already written. Nothing needs changing there.
3. **Keep the peer-process idea attached to the inheritance problem**, not to
   Hermes. If the shared export ever needs honest per-tool attribution, this is
   the only mechanism that survives environment inheritance, and the notes above
   are the starting point.
