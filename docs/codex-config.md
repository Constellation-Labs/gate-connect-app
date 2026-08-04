# Codex config Gate Connect writes

When you connect the **OpenAI / Codex** provider, Gate Connect edits
`~/.codex/config.toml`: it adds (or updates) a `[model_providers.gate]`
block and flips top-level `model_provider = "gate"`. `toml_edit` keeps the
rest of the file (comments, other providers, profiles) byte-identical.

You never edit this by hand; it is documented here so you know what the app
does to your config.

## How the upstream credential is handled

Codex brings its own upstream credentials. Gate Connect sets
`requires_openai_auth = true` on the provider so Codex attaches its own
`codex login` session (the ChatGPT OAuth token or the API key in
`~/.codex/auth.json`) as the upstream bearer. Gate passes that bearer
through and forwards to OpenAI per the `X-Gate-Upstream-Url` hint.

This is the only provider shape that carries a ChatGPT-subscription login
through a custom `base_url`. A bare `[auth] command` credential-helper
script works for API keys but leaves ChatGPT-mode Codex falling back to its
built-in provider and hitting `chatgpt.com` directly, so Gate Connect does
not write one. Older versions did; disconnect deletes any leftover
`codex-credential-helper.sh` / `.cmd` so an upgrade-then-disconnect leaves
zero residue.

Codex reads `config.toml` at startup, so restart any running `codex`
sessions after connecting or disconnecting.

## What disconnect leaves behind

Disconnecting (or turning routing off, which disconnects for you) restores
your previous `model_provider` and strips every Gate value out of the file -
with one deliberate exception. `[model_providers.gate]` stays, rewritten as a
passthrough that points straight at OpenAI:

```toml
# Left by Constellation Gate Connect when Codex was disconnected.
# …
[model_providers.gate]
name = "OpenAI (direct)"
base_url = "https://chatgpt.com/backend-api/codex"
wire_api = "responses"
requires_openai_auth = true

[_gate_connect]
passthrough_stub = true
```

Codex records the provider *name* in each thread's session metadata
(`"model_provider":"gate"`), and resolves that name against `config.toml`
again when the thread resumes. Deleting the block outright therefore breaks
every thread that was started while routing was on:

> ChatGPT can't load config.toml, so this thread can't resume.
> Fix config.toml: Model provider `gate` not found.

The stub keeps those threads resumable and sends them direct to OpenAI. It
holds no Gate credential, no gateway URL and no `X-Gate-Upstream-Url` header,
so nothing routes through Gate while it sits there; the app reports Codex as
disconnected, and reconnecting overwrites it with the managed block. It is
safe to delete by hand once the old threads are finished with.

## What the app writes

The base URL and the `X-Gate-Upstream-Url` header depend on which auth mode
`codex login` left you in. The two modes have incompatible upstream URL
shapes, so Gate Connect picks the matching pair.

### ChatGPT subscription mode

```toml
model_provider = "gate"

[model_providers.gate]
name = "Constellation Gate"
base_url = "https://gateway.constellationgate.ai/codex"
wire_api = "responses"
requires_openai_auth = true

[model_providers.gate.http_headers]
"X-Gate-Api-Key" = "sk-gw-…your Gate key…"
"X-Gate-Upstream-Url" = "https://chatgpt.com/backend-api"
```

- `base_url` ends in `/codex`, so the client sends `/codex/responses`.
- `X-Gate-Upstream-Url` is the bare `backend-api` host; the `/codex`
  segment lives in the path, not the upstream hint.

### API key mode

```toml
model_provider = "gate"

[model_providers.gate]
name = "Constellation Gate"
base_url = "https://gateway.constellationgate.ai/v1"
wire_api = "responses"
requires_openai_auth = true

[model_providers.gate.http_headers]
"X-Gate-Api-Key" = "sk-gw-…your Gate key…"
"X-Gate-Upstream-Url" = "https://api.openai.com"
```

- `base_url` ends in `/v1`, so the client sends `/v1/responses`.
- `X-Gate-Upstream-Url` is the bare `api.openai.com` host; `/v1` lives in
  the path.

## Notes

- The gateway address (`https://gateway.constellationgate.ai` above) comes
  from your account's `gateway_base_url`; a staging URL substitutes cleanly.
  A trailing slash and any existing `/codex` or `/v1` suffix are handled.
- `X-Gate-Api-Key` is your Gate workspace key (`sk-gw-…`). It lives in the
  OS keychain; the app injects it when writing the file. It is not stored
  anywhere else in the config.
