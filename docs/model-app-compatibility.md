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
| `status` | One of `ok`, `open`, `mitigated` or `accepted` (a known failure we live with), and nothing else, so it filters cleanly |
| `source` | How it was found |
| `notes` | Anything else: how it was mitigated, who it was escalated to |

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
| Works | anthropic/claude-opus-5, anthropic/claude-sonnet-5, google/gemini-3-1-flash-lite, deepseek/deepseek-v4-1-flash, deepseek/deepseek-v4-flash, moonshotai/kimi-k2-6, z-ai/glm-5-2, x-ai/grok-4-6 | |
| 400 error | meta-llama/muse-spark-1-1, meta-llama/muse-spark-1-2, meta-llama/muse-spark-1-3 | Refuses a schema pattern with a NUL escape (`^[^\0]*$`) in Claude Code's Artifact tool |
| Empty reply, no error | openai/gpt-5.6-terra, openai/gpt-6-1-sol, openai/gpt-6-luna | Same pattern |
| 403 error | deepseek/r1, deepseek/v3, mistralai/devstral-2-123b | Refuses Claude Code's prompt caching |
| 404 error | qwen/qwen3-235b-a22b-instruct-2507 | Listed in the catalogue but never served, even outside Claude Code |

Eight models work, so the failures are a risk we assume rather than a reason
to escalate. Two are still worth raising with the gateway team: the OpenAI
models fail silently (an empty answer, not an error), and one gateway change
would fix both them and Muse Spark (below).

The working test list, rerun with the live replay below. Each model answers
and calls the Bash tool, with the same result direct and through Gate Connect:

| Model | Result |
|---|---|
| anthropic/claude-opus-5-5 | Works |
| openai/gpt-sol-latest | Empty reply, no error (the OpenAI case above) |
| deepseek/deepseek-v4-1-flash | Works |
| qwen/qwen3-8-flash | Works |
| moonshotai/kimi-k3 | Works |

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

Where it would live: in the gateway (the `gate` repo), in the step that
converts Anthropic requests for other providers. It would fix every app that
sends a schema like this, not only Claude Code.

For whoever implements it:

- **The pattern is nested.** The failing one is `file_paths.items.pattern`, so
  the walk has to go into `items`, `properties`, `anyOf` and the rest, not stop
  at the top of each tool's schema.
- **NUL has more than one spelling.** After JSON decoding the same rule can
  arrive as `\0`, `\x00`, `\u0000` or the NUL character itself. Matching "the
  regex can match or excludes NUL", or at least listing those spellings, keeps
  the rule from missing the next tool that writes it differently.
- **Only the keyword.** A `pattern` whose value is a string is the schema
  keyword. A tool input that is itself named `pattern` (Claude Code's Grep)
  sits under `properties` as an object and must be left alone.

**Not in Gate Connect, by decision.** #400 did this on Gate Connect's Gate
models route and was closed in review: Gate Connect forwards requests
faithfully, so any model that works with an app works through Gate Connect,
and it does not patch a model and app incompatibility that exists without it.
These models fail the same way sent straight to the gateway.

The tradeoff: the gateway quietly edits what the app sent, and it fixes one
specific case. If another tool uses another regex these providers dislike, it
needs another rule. That is why it is raised with the gateway team rather
than treated as settled.

## Checking that Gate Connect forwards faithfully

`crates/core/tests/live_tool_schema_replay.rs` (ignored by default) sends a
captured request for each model twice: straight to the gateway, and through
the relay's Gate models route. It fails if Gate Connect changed the outcome. A
model that fails both ways is a row here, not a test failure. Its module doc
says how to run it and how to capture a request.

## Open questions to escalate

- **NUL escape in a schema pattern.** See the NUL-escape section above.
- **Prompt caching** (`cache_control`). Some upstreams refuse it, so every
  Claude Code request to them fails. The gateway could strip it for those.
- **Catalogue entries with no route**, such as
  `qwen/qwen3-235b-a22b-instruct-2507` (404 on any request).
