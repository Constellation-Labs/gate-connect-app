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

## Open questions to escalate

- **NUL escape in a schema pattern** (`^[^\0]*$`). Muse Spark refuses it with
  a 400. OpenAI models answer 200 with nothing in them. The gateway could
  drop or rewrite that pattern before forwarding, which would fix both.
- **Prompt caching** (`cache_control`). Some upstreams refuse it, so every
  Claude Code request to them fails. The gateway could strip it for those.
- **Catalogue entries with no route**, such as
  `qwen/qwen3-235b-a22b-instruct-2507` (404 on any request).
