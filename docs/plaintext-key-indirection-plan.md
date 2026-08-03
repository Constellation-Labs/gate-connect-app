# Keeping the Gate key out of tool config files

Investigation + design options for security finding #1
(`docs/review-main-security.md`): the `sk-gw-...` gateway key is copied
from the keychain into tool config files in plaintext, and
`primitives::write_file` can follow a symlink into a synced folder,
letting the key leave the machine.

Status: investigation complete, no code written yet. This doc exists to
pick a strategy before building.

## Threat, precisely scoped

The plaintext key is **exclusive to the direct config route**. Two ways a
tool reaches Gate:

- **Proxy route** (system proxy on): the tool's traffic goes through the
  loopback MITM engine, which injects `X-Gate-Api-Key` from its in-memory,
  keychain-backed key (`proxy/engine.rs:406`). The tool config holds no
  secret.
- **Config route** (system proxy off): the tool's `base_url` points
  straight at the gateway and the tool sends `X-Gate-Api-Key` itself, so
  `connect()` copies the literal key out of the keychain (via
  `account::load()`) into the config file.

Every config-route integration inlines the key as a custom header, same
shape across all five:

| Tool | On-disk slot | Site |
|---|---|---|
| Claude Code | `env.ANTHROPIC_CUSTOM_HEADERS` (string) | `claude_code.rs:154,293` |
| Codex | `[model_providers.<id>.http_headers]` | `codex.rs:391` |
| opencode | `provider.<id>.options.headers[...]` | `opencode.rs:474` |
| openclaw | provider `headers` map | `openclaw.rs:559` |
| hermes | `model.default_headers[...]` | `hermes.rs:125` |

Amplifier: `write_file` deliberately `canonicalize`s a symlinked target
"to rewrite the real file and leave the link intact"
(`primitives.rs:24-28`). If `~/.claude/settings.json` is a symlink into
iCloud/Dropbox/a dotfiles repo, the `0o600` key (and even the temp
staging file) is written into that synced location and leaves the box.

## Why the review's suggested fix does not apply as-written

The review proposed "indirection like Claude Code's `apiKeyHelper`."
Confirmed against docs: `apiKeyHelper` (and Codex's `[auth] command`)
only feed the **standard** credential (`x-api-key` / `Authorization:
Bearer`), never an arbitrary **custom** header. Gate identifies the
workspace with a custom header (`X-Gate-Api-Key`) precisely so the tool's
own upstream credential can pass through as the standard bearer. So no
tool can source `X-Gate-Api-Key` from `apiKeyHelper`. Real indirection
needs either a different transport (below) or a gateway protocol change
(accept the workspace key as standard bearer auth), which is out of scope
for this app alone.

## Strategy A - loopback header-injection endpoint (recommended target)

Point config-route tools at a local origin endpoint
(`base_url = http://127.0.0.1:<port>`) that injects `X-Gate-Api-Key` from
the keychain-backed key and forwards to the gateway over TLS. The config
file then holds only a loopback URL, never the secret.

Why this is smaller than it sounds: the engine **already** injects the
key into plain-HTTP (non-CONNECT) origin requests and gates injection on
the owner UID (`engine.rs:301-329`, `apply_rewrite` at 389). The
credential path, owner-peer check, and forwarding all exist. What is
missing is a way to map a loopback-addressed request to its intended
upstream, since a tool pointed at `127.0.0.1` sends `Host: 127.0.0.1`,
which today's host-based `decide()` will not match to a rewrite rule.
Options: a dedicated port (or path prefix) per provider, or a
`X-Gate-Upstream-Url` the endpoint reads and strips.

- Upholds the promise fully: key stays in-process (keychain to memory to
  header), never on disk, on every platform.
- No CA install and no admin prompt: the tool-to-loopback hop is plain
  HTTP to localhost; loopback-to-gateway is ordinary TLS. This keeps the
  config route's "lightweight, no sudo" character.
- No shell-profile edits; works for GUI-launched tools too.
- Cost: a small always-available loopback listener (independent of the
  system-proxy master toggle) plus the origin-to-upstream mapping. It is
  a real feature, but it reuses the existing engine machinery.

## Strategy B - env-sourced headers (partial, fragile)

Write a reference to an env var into the config instead of the literal,
and populate that env var elsewhere. Per-tool capability, confirmed
against current docs (July 2026):

| Tool | Header from env? | Syntax | Notes |
|---|---|---|---|
| Codex | Yes | `env_http_headers = { "X-Gate-Api-Key" = "GATE_API_KEY" }` | value is the env var name |
| opencode | Yes | `"headers": { "X-Gate-Api-Key": "{env:GATE_API_KEY}" }` | `{file:}` is a more reliable fallback; open bugs where custom headers / `{env:}` do not reach the wire in some 1.2.x builds |
| Claude Code | No (in config) | n/a | `ANTHROPIC_CUSTOM_HEADERS` is a literal static string; but you can skip writing `settings.json` and rely on the **ambient** env var since Claude Code reads process env |
| openclaw | Unverified | n/a | env-ref documented for the credential, not for a header |
| hermes | No header field | n/a | `${VAR}` works for `api_key`, no documented custom-header field |

The catch that guts most of the benefit: **env-indirection only relocates
the plaintext to wherever the env var is set.** If that is a file
(`~/.hermes/.env`, an opencode `{file:}` target, or a shell profile), the
secret is still on disk, and shell profiles are frequently themselves in
synced dotfiles repos, i.e. potentially worse than a `0o600` config file.
The only variant that actually keeps the literal off disk is populating
the env var from the keychain at runtime (e.g. a shell snippet running
`security find-generic-password` / `secret-tool`). That requires editing
the user's shell profile (a persistent-config change needing consent),
only covers shell-launched tools, and differs on Windows.

Verdict: viable as an opportunistic enhancement for Codex and opencode
only if paired with a keychain-sourced env var; not a general fix and not
recommended as the primary one.

## Strategy C - symlink hardening (do regardless)

Orthogonal to A/B and necessary either way: stop `write_file` from
writing the payload through a symlink that resolves outside the tool's
own config directory. Chosen policy (per product owner): **refuse and
surface a clear error** naming the link and its target, rather than
silently writing into the resolved location or clobbering the link.

- Contained change in `primitives::write_file`.
- Independent of the indirection work, so it can land first.
- Trade-off accepted: a dotfile-manager setup that symlinks a config path
  out of its directory will get an actionable error instead of a silent
  off-box write.

## Recommendation and phasing

1. **Now: Strategy C.** Small, contained, high-value; removes the
   "key leaves the machine" amplifier that makes this High severity.
   Policy already decided (refuse + surface error).
2. **Target: Strategy A.** The only approach that fully upholds the
   "your key is not in a config file" promise across platforms without
   relocating the secret, and it reuses the engine's existing injection
   path. Scope as a follow-up feature.
3. **Opportunistic: Strategy B** for Codex/opencode, only if a
   keychain-sourced env var is arranged. Otherwise it moves plaintext
   around without a real security gain.

Not pursued: gateway protocol change to accept the workspace key as
standard bearer auth (would enable `apiKeyHelper`/`env_key` indirection
everywhere) - out of scope for this app, worth raising with the gateway
team separately.
