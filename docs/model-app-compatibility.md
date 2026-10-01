# Model and app compatibility

`model-app-compatibility.csv` records what we have seen when an app runs on a
Gate model: which pairs work, which fail, and why. It is a history, not a
filter. The picker does not read it, and nothing is hidden because of it.

One row per app, app version and model, from a test or a report. Add a row
rather than editing an old one, so the history stays.

| Column | Meaning |
|---|---|
| `date` | When it was seen |
| `app`, `app_version` | The app, and its version when known |
| `model` | Gate model id |
| `result` | `works`, `fails`, or `fails silently` (a success status with no answer) |
| `symptom` | What the user sees, with the error text |
| `cause` | Why, once known |
| `status` | `ok`, `open`, `mitigated`, or `open: risk assumed` |
| `source` | How it was found |

## How the 2026-10-01 Claude Code rows were tested

Claude Code's real request was captured once (its system prompt and its 23
built-in tools), then replayed against the staging gateway on the served
route with only the model changed. Two probes per model: a plain reply, and a
request to call the Bash tool. The Artifact tool's `file_paths` schema, which
interactive sessions load and `claude -p` does not, was added to reproduce
the live failure.

## Claude Code on Gate models (2026-10-01)

| Result | Models | Cause |
|---|---|---|
| Works | claude-opus-5, claude-sonnet-5, gemini-3-1-flash-lite, deepseek-v4-1-flash, deepseek-v4-flash, kimi-k2-6, glm-5-2, grok-4-6 | |
| 400 error | muse-spark-1-1, 1-2, 1-3 | Refuses a schema pattern with a NUL escape (`^[^\0]*$`) in Claude Code's Artifact tool |
| Empty reply, no error | gpt-5.6-terra, gpt-6-1-sol, gpt-6-luna | Same pattern |
| 403 error | deepseek/r1, deepseek/v3, devstral-2-123b | Refuses Claude Code's prompt caching |
| 404 error | qwen3-235b-a22b-instruct-2507 | Listed in the catalogue but never served, even outside Claude Code |

Eight models work, so the failures are a risk we assume rather than a reason
to escalate. Two are still worth raising with the gateway team: the OpenAI
models fail silently (an empty answer, not an error), and one change fixes
both them and Muse Spark. That change is now in Gate Connect (below).

## The NUL-escape pattern, and dropping it before forwarding

Claude Code describes each tool's inputs as a JSON Schema. One tool
(Artifact) says file paths must match the regex `^[^\0]*$`, meaning "no NUL
character". Muse Spark refuses the whole request because of that one rule
(400); the OpenAI models accept it and come back empty.

The gateway could walk the tool schemas before forwarding and delete any
`pattern` whose regex contains `\0`:

```json
{"type": "string", "minLength": 1, "maxLength": 1024, "pattern": "^[^\\0]*$"}
```

becomes

```json
{"type": "string", "minLength": 1, "maxLength": 1024}
```

Why it is safe:

- **The rule does almost nothing.** A file path with a NUL character
  essentially never happens, and the model never sends one.
- **Nothing breaks without it.** A `pattern` only guides what the model
  writes; Claude Code still validates the tool's input when it runs the tool.
- **Tested.** With the pattern removed, or replaced by a plain one, Muse
  Spark and the OpenAI models answered and called tools correctly in the
  staging replay.

**Done in Gate Connect.** Every app on Gate models (Codex, Hermes, Claude
Code) sends through the relay's Gate models route, which already rewrites the
body before forwarding (it drops routing overrides). It now drops a NUL-escape
`pattern` under `tools` too (`gate_served::for_gateway`). Only a string
`pattern` is the schema keyword, so a tool input that is itself named
`pattern` (Claude Code's Grep) is left alone. Verified live on staging with
`crates/core/tests/live_tool_schema_replay.rs`: all six failing models answer
and call tools through the route, and claude-sonnet-5 is unchanged.

Still open for the gateway: a request that does not come through Gate
Connect's Gate models route (another client, or an app on its own model under
a PAYG org) is not rewritten, so the same fix in the gateway's Anthropic
conversion would cover everyone.

The tradeoff: the gateway quietly edits what the app sent, and it fixes one
specific case. If another tool uses another regex these providers dislike, it
needs another rule. That is why it is raised with the gateway team rather
than treated as settled.

## Open questions to escalate

- **NUL escape in a schema pattern** (`^[^\0]*$`). Fixed for apps on Gate
  models in Gate Connect (above); the gateway could do the same for everyone.
- **Prompt caching** (`cache_control`). Some upstreams refuse it, so every
  Claude Code request to them fails. The gateway could strip it for those.
- **Catalogue entries with no route**, such as
  `qwen/qwen3-235b-a22b-instruct-2507` (404 on any request).
