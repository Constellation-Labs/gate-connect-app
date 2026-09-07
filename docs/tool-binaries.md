# Finding a tool's executable, and what it unlocks

Written while building `integrations/binaries.rs`. Records the measurements,
because two of them contradict the obvious approach.

## The problem this started from

Gate Connect could not name a tool's version because it does not know where the
tool *is*. Every integration carries a short list of absolute paths
(`/usr/local/bin/x`, `/opt/homebrew/bin/x`) and falls back to "does the config
directory exist". That answers **is it installed** and nothing else.

Measured on one developer machine: Claude Code lives at `~/.local/bin/claude`,
which is on none of the lists. `detect()` still returns true - through the
config-directory fallback - so the tool shows as installed and no path is ever
known. On Windows `CLAUDE_BIN_PATHS` is empty entirely.

## Why the version has to come from the binary

Three cheaper sources exist. Two of them lie. Measured on a machine holding both
an npm and a native install of Claude Code:

| source | said |
| --- | --- |
| npm `package.json` | **2.0.13** |
| the resolved native path (`~/.local/share/claude/versions/2.1.263`) | 2.1.263 |
| the tool's own `.last-update-result.json` | 2.1.263 |
| `claude --version` | **2.1.263** |

The npm copy is stale and is not what runs. Reading `package.json` - the cheap,
obvious approach - would report a version the user is not running, which is
worse than reporting none: it sends a support thread after the wrong release.

The path-encoded and update-record readings happen to be right here, and both are
Claude-Code-specific; the update record is only right if the last update
succeeded.

## The cost is not what it used to be

`claude --version` measured at **80ms cold, under 1ms warm**. These tools ship
native binaries now rather than Node entrypoints - Claude Code has a native
installer, Codex is Rust, OpenCode is Bun-compiled. The Node-startup objection
that would have killed this a year ago no longer applies.

It is still never on a render path. `tool_versions` is its own command, cached on
path + mtime, for the same reason `routing_verdicts` is separate from
`list_tools`: the sidebar repaints far more often than a version changes.

## Why PATH alone is not the answer

Gate Connect is a GUI application. A macOS `.app` launched from Finder, or a
Linux desktop entry, inherits the session's minimal environment and **not** the
user's shell `PATH` - no nvm, no Volta, no `~/.local/bin`. So the search is:
the integration's own absolute paths, then `PATH`, then the directories a login
shell would have added.

`PATH` before the shell directories, deliberately: whatever the user's own shell
resolves is the copy their terminal runs, and it is the copy whose version and
configuration matter.

## What this unlocks, and what it does not

**Detection accuracy**, for free. Any integration that adopts `resolve_binary`
finds installs the hardcoded lists miss. Not done here - `detect()` is untouched,
because it is a render-path read and changing it changes what users see. Worth
its own change.

**Restart - and this is the part to read before planning it.** Knowing the path
is *necessary* for relaunching a tool, and it is not *sufficient*. `can_reopen`
already says why, and it was written before this work:

> Every tool in the registry is a terminal program whose shell session, working
> directory and conversation this process cannot see; a GUI tool is where it
> turns true.

`useRunningApps.ts` agrees: "A tool Gate could relaunch would go to `reopening`
here; none can, so they all wait for the user." The `reopening` stage exists in
`reopen.ts`, with copy, and nothing can reach it.

So spawning `~/.local/bin/claude` after a config change does not restart the
user's session. It starts a **new, detached** process with no terminal, in the
wrong working directory, with none of their conversation - and the one they were
using is still running on the old route, which is the exact problem the reopen
flow exists to solve. Gate would have made it worse and reported success.

Where the path *does* make restart real:

- **a GUI tool** - it owns its window and its state, so relaunching it is what a
  user would have done by hand. This is the case `can_reopen` was written for.
- **a daemon or background service** - same argument.

For the terminal tools, the honest options are the ones that keep the user in
their own shell: tell them the command, or offer to copy it. Neither needs a
spawn, and both need exactly the path this module resolves.
