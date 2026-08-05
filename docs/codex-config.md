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
through and forwards to OpenAI per the upstream hint the relay injects.

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

## Thread history is listed per provider

Turning routing on or off changes **which Codex conversations the ChatGPT app
lists**. With routing on you see the threads you started while routed; turn it
off and you see the ones from before, and vice versa.

Nothing is deleted. Codex keeps one thread store and tags every thread with the
provider it ran under - `model_provider` in `state_*.sqlite`'s `threads` table,
indexed - and the app's list request filters on it (`ThreadListParams` carries a
`modelProviders` field, which becomes `AND threads.model_provider IN (…)`). Since
Gate Connect flips the top-level `model_provider` between `gate` and your own
value, the same store answers two different queries. Every rollout file under
`~/.codex/sessions/` and every row in the index survives the toggle; flip
routing back and the other set returns.

This is Codex's own behaviour, not something Gate Connect can route around:
native Codex partitions history the same way when you switch providers and back.
The only way to make one continuous list would be to keep `model_provider =
"gate"` in place permanently - which would mean owning your pointer even while
disconnected, and would *still* hide anything you started before using Gate. We
would rather restore your config honestly and tell you where the threads went.

## What the app writes

One key, and no headers. `base_url` points at Gate Connect's loopback relay,
never at the gateway directly, and carries two path segments: the catalog slug
the relay routes on, and the suffix Codex appends `/responses` to. Which pair
you get depends on the auth mode `codex login` left you in, because the two
modes have incompatible upstream URL shapes.

### ChatGPT subscription mode

```toml
model_provider = "gate"

[model_providers.gate]
name = "Constellation Gate"
base_url = "http://127.0.0.1:8977/chatgpt/codex"
wire_api = "responses"
requires_openai_auth = true
```

- Codex sends `/chatgpt/codex/responses`.
- The relay strips `/chatgpt`, looks that slug up in its built-in catalog, and
  forwards `/codex/responses` to the gateway with
  `X-Gate-Upstream-Url: https://chatgpt.com/backend-api` attached.

### API key mode

```toml
model_provider = "gate"

[model_providers.gate]
name = "Constellation Gate"
base_url = "http://127.0.0.1:8977/openai/v1"
wire_api = "responses"
requires_openai_auth = true
```

- Codex sends `/openai/v1/responses`.
- The relay strips `/openai` and forwards `/v1/responses` with
  `X-Gate-Upstream-Url: https://api.openai.com`.

## Notes

- **No credential is in this file.** Your Gate credential (OAuth access token,
  or the legacy `sk-gw-…` key) lives in the OS keychain and is injected by the
  relay per request, so a token refresh or a key rotation touches nothing on
  disk. Earlier versions did write `X-Gate-Api-Key` into an
  `[model_providers.gate.http_headers]` table; disconnect removes any leftover.
- **No upstream hint is in this file either.** The relay derives it from the slug
  in `base_url` and injects it itself, the same way the system-proxy path derives
  it from the TLS host. That keeps Gate out of `http_headers` entirely.
- `8977` above is Gate Connect's relay port. It is persisted, so it survives
  restarts and upgrades; if it ever has to change, the app rewrites this file and
  tells you to restart running `codex` sessions.
- The consequence of pointing at loopback: Codex only routes through Gate while
  Gate Connect is running. That is why connecting a tool turns routing on, and
  why quitting the app offers to restore your configs first.
